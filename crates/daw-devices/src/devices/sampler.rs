//! R1 Sampler: one recording played across the keyboard. Each voice reads
//! the recording through `daw_dsp`'s band-limited mip-map at the step its
//! pitch asks for, walks the start, end and loop markers with the shared
//! playhead (equal-power seam crossfade), then runs the Clean filter with
//! its own envelope and an AHDSR amplifier. One LFO for every voice; up to
//! 32 voices. See `docs/R1_DEVICES.md#r1-sampler`.
use super::mini::{MODES, VoiceMode};
use super::param;
use crate::dsp::playback::{LoopMode, Markers, Playhead, Region};
use crate::dsp::{
    Adsr, CleanShape, FilterType, Lfo, LfoShape, SincBank, Smoothed, Stage, Step, Svf, db_to_gain,
    prewarp,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, EventKind, ParamRange, ParamSpec, ProcessContext,
    SampleData, StereoIo, Telemetry,
};
use std::sync::Arc;

pub const ROOT: u32 = 1;
pub const FINE: u32 = 2;
pub const START: u32 = 3;
pub const END: u32 = 4;
pub const REVERSE: u32 = 5;
pub const GAIN: u32 = 6;
pub const LOOP_MODE: u32 = 10;
pub const LOOP_START: u32 = 11;
pub const LOOP_END: u32 = 12;
pub const CROSSFADE: u32 = 13;
pub const KEY_TRACKING: u32 = 20;
pub const TRANSPOSE: u32 = 21;
pub const FILTER_ON: u32 = 30;
pub const FILTER_TYPE: u32 = 31;
pub const FILTER_SLOPE: u32 = 32;
pub const CUTOFF: u32 = 33;
pub const RESONANCE: u32 = 34;
pub const FILTER_ENV: u32 = 35;
pub const FILTER_KEY_TRACKING: u32 = 36;
pub const VELOCITY_CUTOFF: u32 = 37;
pub const FENV_ATTACK: u32 = 40;
pub const FENV_DECAY: u32 = 41;
pub const FENV_SUSTAIN: u32 = 42;
pub const FENV_RELEASE: u32 = 43;
pub const AENV_ATTACK: u32 = 50;
pub const AENV_HOLD: u32 = 51;
pub const AENV_DECAY: u32 = 52;
pub const AENV_SUSTAIN: u32 = 53;
pub const AENV_RELEASE: u32 = 54;
pub const LFO_RATE: u32 = 60;
pub const LFO_SYNC: u32 = 61;
pub const LFO_SHAPE: u32 = 62;
pub const LFO_PITCH: u32 = 63;
pub const LFO_CUTOFF: u32 = 64;
pub const LFO_LEVEL: u32 = 65;
pub const LFO_PAN: u32 = 66;
pub const VOICE_MODE: u32 = 70;
pub const VOICES: u32 = 71;
pub const GLIDE: u32 = 72;
pub const BEND_RANGE: u32 = 73;
pub const VELOCITY_LEVEL: u32 = 74;
pub const PAN: u32 = 75;
pub const VOLUME: u32 = 76;

/// Telemetry value slots. Playheads: one per voice from
/// [`TELEMETRY_PLAYHEADS`], the position as a share of the recording, or
/// −1 for a silent voice.
pub const TELEMETRY_VOICES: usize = 0;
pub const TELEMETRY_LFO_PHASE: usize = 1;
pub const TELEMETRY_CUTOFF: usize = 2;
pub const TELEMETRY_PLAYHEADS: usize = 8;

pub const MAX_VOICES: usize = 32;
pub const TYPES: &[&str] = &["Low-pass", "High-pass", "Band-pass"];
pub const SLOPES: &[&str] = super::filter::SLOPES;
pub const SYNC: &[&str] = super::filter::SYNC;

/// Output samples between modulation updates; values ramp in between.
const CONTROL: usize = 16;
const STEAL_FADE_MS: f32 = 5.0;
/// A voice reaching its end marker fades over this long, so it never cuts.
const END_FADE_MS: f32 = 2.0;
const GLIDE_OFF_MS: f32 = 1.0;
/// A hold this short is none.
const HOLD_OFF_MS: f32 = 1.0;
const HELD_NOTES: usize = 16;

const fn percent(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    default: f32,
) -> ParamSpec {
    param(
        id,
        key,
        name,
        group,
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        default,
    )
}

const fn time(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    min: f32,
    default: f32,
) -> ParamSpec {
    param(
        id,
        key,
        name,
        group,
        "ms",
        ParamRange::Log { min, max: 10_000.0 },
        default,
    )
}

pub static PARAMETERS: [ParamSpec; 43] = [
    param(
        ROOT,
        "sample.root",
        "Root",
        "Sample",
        crate::NOTE,
        ParamRange::Integer { min: 0, max: 127 },
        60.0,
    ),
    param(
        FINE,
        "sample.fine",
        "Fine",
        "Sample",
        "ct",
        ParamRange::Linear {
            min: -100.0,
            max: 100.0,
        },
        0.0,
    ),
    percent(START, "sample.start", "Start", "Sample", 0.0),
    percent(END, "sample.end", "End", "Sample", 100.0),
    param(
        REVERSE,
        "sample.reverse",
        "Reverse",
        "Sample",
        "",
        ParamRange::Toggle,
        0.0,
    ),
    param(
        GAIN,
        "sample.gain",
        "Gain",
        "Sample",
        "dB",
        ParamRange::Linear {
            min: -24.0,
            max: 24.0,
        },
        0.0,
    ),
    param(
        LOOP_MODE,
        "loop.mode",
        "Loop",
        "Loop",
        "",
        ParamRange::Choice(LoopMode::LABELS),
        0.0,
    ),
    percent(LOOP_START, "loop.start", "Loop start", "Loop", 40.0),
    percent(LOOP_END, "loop.end", "Loop end", "Loop", 90.0),
    param(
        CROSSFADE,
        "loop.crossfade",
        "Crossfade",
        "Loop",
        "ms",
        ParamRange::Linear {
            min: 0.0,
            max: 500.0,
        },
        20.0,
    ),
    param(
        KEY_TRACKING,
        "pitch.keytrack",
        "Key track",
        "Pitch",
        "",
        ParamRange::Toggle,
        1.0,
    ),
    param(
        TRANSPOSE,
        "pitch.transpose",
        "Transpose",
        "Pitch",
        "st",
        ParamRange::Integer { min: -24, max: 24 },
        0.0,
    ),
    param(
        FILTER_ON,
        "filter.on",
        "Filter",
        "Filter",
        "",
        ParamRange::Toggle,
        0.0,
    ),
    param(
        FILTER_TYPE,
        "filter.type",
        "Type",
        "Filter",
        "",
        ParamRange::Choice(TYPES),
        0.0,
    ),
    param(
        FILTER_SLOPE,
        "filter.slope",
        "Slope",
        "Filter",
        "",
        ParamRange::Choice(SLOPES),
        0.0,
    ),
    param(
        CUTOFF,
        "filter.cutoff",
        "Cutoff",
        "Filter",
        "Hz",
        ParamRange::Log {
            min: 20.0,
            max: 20_000.0,
        },
        4000.0,
    ),
    percent(RESONANCE, "filter.resonance", "Resonance", "Filter", 10.0),
    param(
        FILTER_ENV,
        "filter.env",
        "Envelope",
        "Filter",
        "oct",
        ParamRange::Linear {
            min: -5.0,
            max: 5.0,
        },
        0.0,
    ),
    percent(
        FILTER_KEY_TRACKING,
        "filter.keytrack",
        "Key track",
        "Filter",
        0.0,
    ),
    param(
        VELOCITY_CUTOFF,
        "filter.velocity",
        "Velocity",
        "Filter",
        "oct",
        ParamRange::Linear { min: 0.0, max: 5.0 },
        0.0,
    ),
    time(
        FENV_ATTACK,
        "fenv.attack",
        "Attack",
        "Filter envelope",
        0.5,
        2.0,
    ),
    time(
        FENV_DECAY,
        "fenv.decay",
        "Decay",
        "Filter envelope",
        1.0,
        400.0,
    ),
    percent(
        FENV_SUSTAIN,
        "fenv.sustain",
        "Sustain",
        "Filter envelope",
        30.0,
    ),
    time(
        FENV_RELEASE,
        "fenv.release",
        "Release",
        "Filter envelope",
        1.0,
        300.0,
    ),
    time(
        AENV_ATTACK,
        "aenv.attack",
        "Attack",
        "Amp envelope",
        0.5,
        1.0,
    ),
    time(
        AENV_HOLD,
        "aenv.hold",
        "Hold",
        "Amp envelope",
        HOLD_OFF_MS,
        HOLD_OFF_MS,
    ),
    time(
        AENV_DECAY,
        "aenv.decay",
        "Decay",
        "Amp envelope",
        1.0,
        600.0,
    ),
    percent(
        AENV_SUSTAIN,
        "aenv.sustain",
        "Sustain",
        "Amp envelope",
        100.0,
    ),
    time(
        AENV_RELEASE,
        "aenv.release",
        "Release",
        "Amp envelope",
        1.0,
        200.0,
    ),
    param(
        LFO_RATE,
        "lfo.rate",
        "Rate",
        "LFO",
        "Hz",
        ParamRange::Log {
            min: 0.01,
            max: 20.0,
        },
        5.0,
    ),
    param(
        LFO_SYNC,
        "lfo.sync",
        "Sync",
        "LFO",
        "",
        ParamRange::Choice(SYNC),
        0.0,
    ),
    param(
        LFO_SHAPE,
        "lfo.shape",
        "Shape",
        "LFO",
        "",
        ParamRange::Choice(LfoShape::LABELS),
        0.0,
    ),
    param(
        LFO_PITCH,
        "lfo.pitch",
        "Pitch",
        "LFO",
        "st",
        ParamRange::Linear {
            min: 0.0,
            max: 12.0,
        },
        0.0,
    ),
    param(
        LFO_CUTOFF,
        "lfo.cutoff",
        "Cutoff",
        "LFO",
        "oct",
        ParamRange::Linear { min: 0.0, max: 5.0 },
        0.0,
    ),
    percent(LFO_LEVEL, "lfo.level", "Level", "LFO", 0.0),
    percent(LFO_PAN, "lfo.pan", "Pan", "LFO", 0.0),
    param(
        VOICE_MODE,
        "voice.mode",
        "Mode",
        "Voice",
        "",
        ParamRange::Choice(MODES),
        0.0,
    ),
    param(
        VOICES,
        "voice.count",
        "Voices",
        "Voice",
        "",
        ParamRange::Integer {
            min: 1,
            max: MAX_VOICES as i32,
        },
        16.0,
    ),
    param(
        GLIDE,
        "voice.glide",
        "Glide",
        "Voice",
        "ms",
        ParamRange::Log {
            min: GLIDE_OFF_MS,
            max: 2000.0,
        },
        GLIDE_OFF_MS,
    ),
    param(
        BEND_RANGE,
        "voice.bend",
        "Bend",
        "Voice",
        "st",
        ParamRange::Integer { min: 0, max: 24 },
        2.0,
    ),
    percent(VELOCITY_LEVEL, "voice.velocity", "Velocity", "Voice", 60.0),
    param(
        PAN,
        "voice.pan",
        "Pan",
        "Voice",
        "%",
        ParamRange::Linear {
            min: -100.0,
            max: 100.0,
        },
        0.0,
    ),
    param(
        VOLUME,
        "voice.volume",
        "Volume",
        "Voice",
        "dB",
        ParamRange::Linear {
            min: -36.0,
            max: 6.0,
        },
        -6.0,
    ),
];

pub static DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    id: "r1-sampler",
    name: "R1 Sampler",
    kind: DeviceKind::Instrument,
    category: "Sampler",
    parameters: &PARAMETERS,
    sample_slots: 1,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-sampler/default.rillpreset"),
        include_str!("../../presets/r1-sampler/sustained-loop.rillpreset"),
        include_str!("../../presets/r1-sampler/one-shot-hit.rillpreset"),
        include_str!("../../presets/r1-sampler/soft-pad.rillpreset"),
        include_str!("../../presets/r1-sampler/plucked-keys.rillpreset"),
        include_str!("../../presets/r1-sampler/tape-reverse.rillpreset"),
        include_str!("../../presets/r1-sampler/lo-fi-chop.rillpreset"),
        include_str!("../../presets/r1-sampler/vibrato-lead.rillpreset"),
    ],
    legacy_labels: &[],
    latency: |_, _| 0,
    create: || Box::new(Sampler::new()),
};

/// Plain parameter values, decoded once per change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub root: f32,
    /// Semitones.
    pub fine: f32,
    /// Shares of the recording, 0–1.
    pub start: f32,
    pub end: f32,
    pub reverse: bool,
    pub gain_db: f32,
    pub loop_mode: LoopMode,
    pub loop_start: f32,
    pub loop_end: f32,
    pub crossfade_ms: f32,
    pub key_tracking: bool,
    pub transpose: f32,
    pub filter_on: bool,
    pub filter_type: FilterType,
    pub steep: bool,
    pub cutoff: f32,
    /// 0–1.
    pub resonance: f32,
    pub filter_env: f32,
    /// 0–1.
    pub filter_key_tracking: f32,
    pub velocity_cutoff: f32,
    /// Attack, decay, release in ms; sustain 0–1.
    pub filter_envelope: [f32; 4],
    /// Attack, hold, decay, release in ms; sustain 0–1.
    pub amp_envelope: [f32; 5],
    pub lfo_rate: f32,
    pub lfo_sync_beats: Option<f64>,
    pub lfo_shape: LfoShape,
    pub lfo_pitch: f32,
    pub lfo_cutoff: f32,
    /// 0–1.
    pub lfo_level: f32,
    pub lfo_pan: f32,
    pub mode: VoiceMode,
    pub voices: usize,
    pub glide_ms: f32,
    pub bend_range: f32,
    /// 0–1.
    pub velocity_level: f32,
    /// −1–1.
    pub pan: f32,
    pub volume_db: f32,
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("sampler parameter");
            spec.to_plain(value(id))
        };
        let index = |id: u32| plain(id) as usize;
        let share = |id: u32| plain(id) / 100.0;
        let sync = super::filter::SYNC_BEATS[index(LFO_SYNC).min(SYNC.len() - 1)];
        Self {
            root: plain(ROOT),
            fine: plain(FINE) / 100.0,
            start: share(START),
            end: share(END),
            reverse: plain(REVERSE) >= 0.5,
            gain_db: plain(GAIN),
            loop_mode: LoopMode::from_index(index(LOOP_MODE)),
            loop_start: share(LOOP_START),
            loop_end: share(LOOP_END),
            crossfade_ms: plain(CROSSFADE),
            key_tracking: plain(KEY_TRACKING) >= 0.5,
            transpose: plain(TRANSPOSE),
            filter_on: plain(FILTER_ON) >= 0.5,
            filter_type: match index(FILTER_TYPE) {
                1 => FilterType::HighPass,
                2 => FilterType::BandPass,
                _ => FilterType::LowPass,
            },
            steep: index(FILTER_SLOPE) == 1,
            cutoff: plain(CUTOFF),
            resonance: share(RESONANCE),
            filter_env: plain(FILTER_ENV),
            filter_key_tracking: share(FILTER_KEY_TRACKING),
            velocity_cutoff: plain(VELOCITY_CUTOFF),
            filter_envelope: [
                plain(FENV_ATTACK),
                plain(FENV_DECAY),
                share(FENV_SUSTAIN),
                plain(FENV_RELEASE),
            ],
            amp_envelope: [
                plain(AENV_ATTACK),
                if plain(AENV_HOLD) <= HOLD_OFF_MS {
                    0.0
                } else {
                    plain(AENV_HOLD)
                },
                plain(AENV_DECAY),
                share(AENV_SUSTAIN),
                plain(AENV_RELEASE),
            ],
            lfo_rate: plain(LFO_RATE),
            lfo_sync_beats: (sync > 0.0).then_some(sync),
            lfo_shape: LfoShape::from_index(index(LFO_SHAPE)),
            lfo_pitch: plain(LFO_PITCH),
            lfo_cutoff: plain(LFO_CUTOFF),
            lfo_level: share(LFO_LEVEL),
            lfo_pan: share(LFO_PAN),
            mode: match index(VOICE_MODE) {
                1 => VoiceMode::Mono,
                2 => VoiceMode::Legato,
                _ => VoiceMode::Poly,
            },
            voices: (plain(VOICES) as usize).clamp(1, MAX_VOICES),
            glide_ms: plain(GLIDE),
            bend_range: plain(BEND_RANGE),
            velocity_level: share(VELOCITY_LEVEL),
            pan: plain(PAN) / 100.0,
            volume_db: plain(VOLUME),
        }
    }

    /// The markers of a recording at `sample_rate`, played in the voice's
    /// direction.
    pub fn markers(&self, sample_rate: f32, reverse: bool) -> Markers {
        Markers {
            start: self.start,
            end: self.end,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            crossfade: f64::from(self.crossfade_ms * 0.001 * sample_rate),
            mode: self.loop_mode,
            reverse,
        }
    }

    /// The note a key plays relative to the root, in semitones before bend
    /// and LFO.
    pub fn transposition(&self, pitch: f32) -> f32 {
        let key = if self.key_tracking {
            pitch - self.root
        } else {
            0.0
        };
        key + self.transpose + self.fine
    }
}

/// Whether a parameter does anything with the current settings; panels show
/// the ones that do not disabled in place.
pub fn applies(id: u32, settings: &Settings) -> bool {
    match id {
        ROOT => settings.key_tracking,
        LOOP_START | LOOP_END => settings.loop_mode.loops(),
        CROSSFADE => settings.loop_mode == LoopMode::Forward,
        FILTER_TYPE | FILTER_SLOPE | CUTOFF | RESONANCE | FILTER_ENV | FILTER_KEY_TRACKING
        | VELOCITY_CUTOFF | FENV_ATTACK | FENV_DECAY | FENV_SUSTAIN | FENV_RELEASE | LFO_CUTOFF => {
            settings.filter_on
        }
        AENV_RELEASE => settings.loop_mode != LoopMode::OneShot,
        LFO_RATE => settings.lfo_sync_beats.is_none(),
        VOICES => settings.mode == VoiceMode::Poly,
        _ => true,
    }
}

/// A value that moves linearly to its next control-rate target.
#[derive(Clone, Copy, Debug, Default)]
struct Ramp {
    value: f32,
    step: f32,
}

impl Ramp {
    fn to(&mut self, target: f32, start: bool) {
        if start {
            self.value = target;
            self.step = 0.0;
        } else {
            self.step = (target - self.value) / CONTROL as f32;
        }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        let value = self.value;
        self.value += self.step;
        value
    }

    fn target(&self) -> f32 {
        self.value + self.step * CONTROL as f32
    }
}

/// Control-rate modulation every voice reads.
#[derive(Clone, Copy, Debug, Default)]
struct Modulation {
    lfo: f32,
    resonance: f32,
    bend: f32,
    cutoff_octaves: f32,
    filter_env: f32,
    key_tracking: f32,
    velocity_cutoff: f32,
    lfo_pitch: f32,
    lfo_cutoff: f32,
    lfo_level: f32,
    lfo_pan: f32,
    pan: f32,
    velocity_level: f32,
    glide: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Note {
    id: u32,
    pitch: f32,
    velocity: f32,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    /// `None` when the note was dropped during the fade: the voice goes idle.
    note: Option<Note>,
    /// Cleared when the key is released during the fade.
    held: bool,
    glide_from: f32,
}

#[derive(Clone, Debug, Default)]
struct Voice {
    note: Note,
    /// Current pitch while gliding.
    pitch: f32,
    held: bool,
    sustained: bool,
    age: u64,
    pending: Option<Pending>,
    fade: f32,
    reverse: bool,
    region: Region,
    playhead: Playhead,
    amp: Adsr,
    filter_envelope: Adsr,
    /// Two stages for each channel.
    filters: [Svf; 4],
    /// Samples of the recording per output sample.
    step: Ramp,
    reader: Option<Step>,
    g: Ramp,
    /// Left and right gain: velocity, LFO level and pan.
    gains: [Ramp; 2],
    cutoff: f32,
}

impl Voice {
    fn reset(&mut self) {
        *self = Self {
            fade: 1.0,
            ..Self::default()
        };
    }

    fn active(&self) -> bool {
        self.amp.is_active() || self.pending.is_some()
    }

    fn releasing(&self) -> bool {
        !self.held && !self.sustained && self.amp.stage() == Stage::Release
    }

    fn release(&mut self, mode: LoopMode) {
        // A one-shot plays to its end whatever the key does.
        if mode != LoopMode::OneShot {
            self.amp.gate_off();
            self.filter_envelope.gate_off();
        }
    }
}

pub struct Sampler {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    dirty: bool,
    sample_rate: f32,
    sample: Option<Arc<SampleData>>,
    bank: &'static SincBank,
    voices: [Voice; MAX_VOICES],
    held: [Note; HELD_NOTES],
    held_count: usize,
    sustain: bool,
    bend: Smoothed,
    lfo: Lfo,
    countdown: usize,
    age: u64,
    last_pitch: f32,
    modulation: Modulation,
    cutoff: Smoothed,
    resonance: Smoothed,
    filter_env: Smoothed,
    key_tracking: Smoothed,
    velocity_cutoff: Smoothed,
    lfo_pitch: Smoothed,
    lfo_cutoff: Smoothed,
    lfo_level: Smoothed,
    lfo_pan: Smoothed,
    pan: Smoothed,
    velocity_level: Smoothed,
    /// Sample gain and volume, per output sample.
    gain: Smoothed,
    telemetry: Arc<Telemetry>,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let mut sampler = Self {
            values,
            settings: Settings::from_normalized(|id| {
                values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
            }),
            dirty: true,
            sample_rate: 48_000.0,
            sample: None,
            bank: SincBank::shared(),
            voices: std::array::from_fn(|_| Voice::default()),
            held: [Note::default(); HELD_NOTES],
            held_count: 0,
            sustain: false,
            bend: Smoothed::new(0.0),
            lfo: Lfo::new(0x5A3B_1E00),
            countdown: 0,
            age: 0,
            last_pitch: 60.0,
            modulation: Modulation::default(),
            cutoff: Smoothed::new(0.0),
            resonance: Smoothed::new(0.0),
            filter_env: Smoothed::new(0.0),
            key_tracking: Smoothed::new(0.0),
            velocity_cutoff: Smoothed::new(0.0),
            lfo_pitch: Smoothed::new(0.0),
            lfo_cutoff: Smoothed::new(0.0),
            lfo_level: Smoothed::new(0.0),
            lfo_pan: Smoothed::new(0.0),
            pan: Smoothed::new(0.0),
            velocity_level: Smoothed::new(0.0),
            gain: Smoothed::new(0.0),
            telemetry: Arc::new(Telemetry::default()),
        };
        sampler.prepare(48_000.0, 0);
        sampler
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn control_smoothers(&mut self) -> [&mut Smoothed; 12] {
        [
            &mut self.bend,
            &mut self.cutoff,
            &mut self.resonance,
            &mut self.filter_env,
            &mut self.key_tracking,
            &mut self.velocity_cutoff,
            &mut self.lfo_pitch,
            &mut self.lfo_cutoff,
            &mut self.lfo_level,
            &mut self.lfo_pan,
            &mut self.pan,
            &mut self.velocity_level,
        ]
    }

    fn update_settings(&mut self) {
        let values = self.values;
        let settings = Settings::from_normalized(|id| {
            values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
        });
        if settings.mode != self.settings.mode {
            // Voices held under the old mode would never see their note-off.
            for voice in &mut self.voices {
                voice.held = false;
                voice.sustained = false;
                voice.amp.gate_off();
                voice.filter_envelope.gate_off();
                if let Some(pending) = &mut voice.pending {
                    pending.note = None;
                }
            }
            self.held_count = 0;
        }
        for voice in &mut self.voices[settings.voices..] {
            voice.held = false;
            voice.sustained = false;
            voice.amp.gate_off();
            voice.filter_envelope.gate_off();
        }
        if (settings.filter_type, settings.steep)
            != (self.settings.filter_type, self.settings.steep)
        {
            for voice in &mut self.voices {
                voice.filters = Default::default();
            }
        }
        let s = settings;
        self.settings = s;
        self.cutoff.set_target(s.cutoff.log2());
        self.resonance.set_target(s.resonance);
        self.filter_env.set_target(s.filter_env);
        self.key_tracking.set_target(s.filter_key_tracking);
        self.velocity_cutoff.set_target(s.velocity_cutoff);
        self.lfo_pitch.set_target(s.lfo_pitch);
        self.lfo_cutoff.set_target(s.lfo_cutoff);
        self.lfo_level.set_target(s.lfo_level);
        self.lfo_pan.set_target(s.lfo_pan);
        self.pan.set_target(s.pan);
        self.velocity_level.set_target(s.velocity_level);
        self.gain.set_target(db_to_gain(s.gain_db + s.volume_db));
        let (rate, control_rate) = (self.sample_rate, self.sample_rate / CONTROL as f32);
        for voice in &mut self.voices {
            let [a, h, d, sustain, r] = s.amp_envelope;
            voice.amp.set(a, d, sustain, r, rate);
            voice.amp.set_hold(h, rate);
            let [a, d, sustain, r] = s.filter_envelope;
            voice.filter_envelope.set(a, d, sustain, r, control_rate);
        }
        self.dirty = false;
    }

    /// Global modulation, then each sounding voice's targets for the next
    /// control period.
    fn control(&mut self, context: &ProcessContext, frame: usize) {
        let s = self.settings;
        let rate = self.sample_rate;
        match s.lfo_sync_beats.filter(|_| context.playing) {
            Some(beats) => {
                let beats_per_sample = context.tempo / 60.0 / f64::from(rate);
                self.lfo
                    .set_phase((context.beat + frame as f64 * beats_per_sample) / beats);
            }
            None => {
                let hz = s
                    .lfo_sync_beats
                    .map_or(f64::from(s.lfo_rate), |beats| context.tempo / 60.0 / beats);
                self.lfo.advance(hz * CONTROL as f64 / f64::from(rate));
            }
        }
        let glide_periods = s.glide_ms * 0.001 * rate / CONTROL as f32;
        self.modulation = Modulation {
            lfo: self.lfo.value(s.lfo_shape, 0.0),
            resonance: self.resonance.next_value(),
            bend: self.bend.next_value() * s.bend_range,
            cutoff_octaves: self.cutoff.next_value(),
            filter_env: self.filter_env.next_value(),
            key_tracking: self.key_tracking.next_value(),
            velocity_cutoff: self.velocity_cutoff.next_value(),
            lfo_pitch: self.lfo_pitch.next_value(),
            lfo_cutoff: self.lfo_cutoff.next_value(),
            lfo_level: self.lfo_level.next_value(),
            lfo_pan: self.lfo_pan.next_value(),
            pan: self.pan.next_value(),
            velocity_level: self.velocity_level.next_value(),
            glide: if s.glide_ms <= GLIDE_OFF_MS {
                1.0
            } else {
                1.0 - (-4.6 / glide_periods.max(1.0)).exp()
            },
        };
        let Some(sample) = &self.sample else {
            return;
        };
        let (sample, modulation) = (Arc::clone(sample), self.modulation);
        let mut active = 0;
        for (index, voice) in self.voices.iter_mut().enumerate() {
            let playhead = if voice.active() {
                active += 1;
                voice_control(voice, &s, &modulation, &sample, rate, false);
                (voice.playhead.position(&voice.region) / sample.frames().max(1) as f64) as f32
            } else {
                -1.0
            };
            self.telemetry
                .set_value(TELEMETRY_PLAYHEADS + index, playhead);
        }
        self.telemetry.set_value(TELEMETRY_VOICES, active as f32);
        self.telemetry
            .set_value(TELEMETRY_LFO_PHASE, self.lfo.phase() as f32);
        if let Some(voice) = self
            .voices
            .iter()
            .filter(|voice| voice.active())
            .max_by_key(|voice| voice.age)
        {
            self.telemetry.set_value(TELEMETRY_CUTOFF, voice.cutoff);
        }
    }

    fn start(&mut self, index: usize, note: Note, glide_from: f32) {
        let Some(sample) = self.sample.clone() else {
            return;
        };
        let s = self.settings;
        let modulation = self.modulation;
        let rate = self.sample_rate;
        self.age += 1;
        let voice = &mut self.voices[index];
        let was_active = voice.amp.is_active();
        voice.note = note;
        voice.held = true;
        voice.sustained = false;
        voice.age = self.age;
        voice.pitch = if modulation.glide < 1.0 {
            glide_from
        } else {
            note.pitch
        };
        if !was_active {
            voice.filters = Default::default();
        }
        // A retriggered voice starts its recording afresh; the envelope
        // continues from its level, so it does not click.
        voice.reverse = s.reverse;
        voice.region = Region::new(
            sample.frames(),
            &s.markers(sample.sample_rate(), voice.reverse),
        );
        voice.playhead = Playhead::new(&voice.region);
        voice.amp.gate_on();
        voice.filter_envelope.gate_on();
        voice_control(voice, &s, &modulation, &sample, rate, !was_active);
    }

    fn note_on(&mut self, id: u32, pitch: f32, velocity: f32) {
        if self.sample.is_none() {
            return;
        }
        let note = Note {
            id,
            pitch,
            velocity: velocity.clamp(0.0, 1.0),
        };
        let glide_from = self.last_pitch;
        self.last_pitch = pitch;
        if self.settings.mode != VoiceMode::Poly {
            self.mono_note_on(note, glide_from);
            return;
        }
        let count = self.settings.voices;
        if let Some(index) = self.voices[..count].iter().position(|voice| {
            voice.amp.is_active() && voice.note.id == id && voice.pending.is_none()
        }) {
            self.start(index, note, glide_from);
            return;
        }
        if let Some(index) = self.voices[..count]
            .iter()
            .position(|voice| !voice.active())
        {
            self.start(index, note, glide_from);
            return;
        }
        // Steal the quietest releasing voice, else the oldest, after a short fade.
        let index = self.voices[..count]
            .iter()
            .enumerate()
            .filter(|(_, voice)| voice.pending.is_none() && voice.releasing())
            .min_by(|(_, a), (_, b)| a.amp.level().total_cmp(&b.amp.level()))
            .or_else(|| {
                self.voices[..count]
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.age)
            })
            .map_or(0, |(index, _)| index);
        self.age += 1;
        let voice = &mut self.voices[index];
        voice.pending = Some(Pending {
            note: Some(note),
            held: true,
            glide_from,
        });
        voice.held = false;
        voice.sustained = false;
        voice.age = self.age;
    }

    fn mono_note_on(&mut self, note: Note, glide_from: f32) {
        let overlapping = self.held_count > 0;
        self.remove_held(note.id);
        if self.held_count == HELD_NOTES {
            self.held.copy_within(1.., 0);
            self.held_count -= 1;
        }
        self.held[self.held_count] = note;
        self.held_count += 1;
        let legato = self.settings.mode == VoiceMode::Legato;
        let voice = &mut self.voices[0];
        if let Some(pending) = &mut voice.pending {
            *pending = Pending {
                note: Some(note),
                held: true,
                glide_from: if legato { note.pitch } else { glide_from },
            };
            return;
        }
        if !voice.amp.is_active() || !(legato && overlapping) {
            // Mono retriggers the recording; Legato does only for a new phrase.
            self.start(0, note, if legato { note.pitch } else { glide_from });
            return;
        }
        // Legato between overlapping notes: the same voice glides on.
        voice.note = note;
        voice.held = true;
        voice.sustained = false;
    }

    fn remove_held(&mut self, id: u32) {
        if let Some(position) = self.held[..self.held_count]
            .iter()
            .position(|note| note.id == id)
        {
            self.held
                .copy_within(position + 1..self.held_count, position);
            self.held_count -= 1;
        }
    }

    fn note_off(&mut self, id: u32) {
        let mode = self.settings.loop_mode;
        if self.settings.mode != VoiceMode::Poly {
            self.remove_held(id);
            if self.held_count > 0 {
                // Back to the most recent key still held, without retrigger.
                let back = self.held[self.held_count - 1];
                let voice = &mut self.voices[0];
                if let Some(pending) = &mut voice.pending
                    && pending.note.is_some_and(|note| note.id == id)
                {
                    pending.note = Some(back);
                    return;
                }
                if voice.note.id == id {
                    voice.note = back;
                    return;
                }
            }
        }
        let sustain = self.sustain;
        for voice in &mut self.voices {
            if let Some(pending) = &mut voice.pending
                && pending.note.is_some_and(|note| note.id == id)
            {
                pending.held = false;
            }
            if voice.held && voice.note.id == id {
                voice.held = false;
                if sustain {
                    voice.sustained = true;
                } else {
                    voice.release(mode);
                }
            }
        }
    }

    fn set_sustain(&mut self, on: bool) {
        self.sustain = on;
        if !on {
            let mode = self.settings.loop_mode;
            for voice in &mut self.voices {
                if voice.sustained {
                    voice.sustained = false;
                    voice.release(mode);
                }
            }
        }
    }

    fn handle(&mut self, kind: EventKind) {
        match kind {
            EventKind::NoteOn {
                note,
                pitch,
                velocity,
            } => {
                if velocity > 0.0 {
                    self.note_on(note, pitch, velocity);
                } else {
                    self.note_off(note);
                }
            }
            EventKind::NoteOff { note } => self.note_off(note),
            EventKind::PitchBend(value) => self.bend.set_target(value.clamp(-1.0, 1.0)),
            EventKind::Sustain(on) => self.set_sustain(on),
            EventKind::ModWheel(_) | EventKind::Pressure { .. } => {}
        }
    }

    /// Adds `left.len()` samples of every voice, within one control period.
    fn render(&mut self, sample: &SampleData, left: &mut [f32], right: &mut [f32]) {
        let s = self.settings;
        let shape = CleanShape::new(s.filter_type, s.steep, self.modulation.resonance);
        let fade_step = 1.0 / (STEAL_FADE_MS * 0.001 * self.sample_rate);
        let end_fade = f64::from(END_FADE_MS * 0.001 * self.sample_rate);
        let (first, second) = (sample.channel(0), sample.channel(1));
        let stereo = sample.channel_count() > 1;
        let bank = self.bank;
        for index in 0..MAX_VOICES {
            let voice = &mut self.voices[index];
            if !voice.active() {
                continue;
            }
            let Some(reader) = voice.reader else {
                continue;
            };
            for (out_left, out_right) in left.iter_mut().zip(right.iter_mut()) {
                let step = f64::from(voice.step.next());
                let reads = voice.playhead.reads(&voice.region);
                let tap = first.tap(bank, &reader, reads.first.0);
                let (mut l, mut r) = (first.read(&tap), 0.0);
                if stereo {
                    r = second.read(&tap);
                }
                l *= reads.first.1;
                r *= reads.first.1;
                if reads.second.1 > 0.0 {
                    let tap = first.tap(bank, &reader, reads.second.0);
                    l += reads.second.1 * first.read(&tap);
                    if stereo {
                        r += reads.second.1 * second.read(&tap);
                    }
                }
                if !stereo {
                    r = l;
                }
                voice.playhead.advance(&voice.region, step, end_fade);
                let g = voice.g.next();
                if s.filter_on {
                    let [a, b, c, d] = &mut voice.filters;
                    l = shape.process(a, b, l, g);
                    r = shape.process(c, d, r, g);
                }
                let mut amp = voice.amp.next_value();
                if voice.pending.is_some() {
                    voice.fade = (voice.fade - fade_step).max(0.0);
                    amp *= voice.fade;
                }
                *out_left += l * amp * voice.gains[0].next();
                *out_right += r * amp * voice.gains[1].next();
                if voice.playhead.is_done() {
                    voice.amp.reset();
                    voice.filter_envelope.reset();
                    if voice.pending.is_some() {
                        voice.fade = 0.0;
                    }
                    break;
                }
            }
            if voice.pending.is_some() && voice.fade == 0.0 {
                // The fade ended in this span; the new note starts with the next.
                let pending = voice.pending.take().expect("pending note");
                voice.fade = 1.0;
                voice.amp.reset();
                voice.filter_envelope.reset();
                let Some(note) = pending.note.filter(|_| index < self.settings.voices) else {
                    continue;
                };
                self.start(index, note, pending.glide_from);
                if !pending.held {
                    let mode = self.settings.loop_mode;
                    let voice = &mut self.voices[index];
                    voice.held = false;
                    if self.sustain {
                        voice.sustained = true;
                    } else {
                        voice.release(mode);
                    }
                }
            }
        }
    }
}

/// Pitch, cutoff and gains of one voice for the next control period.
/// `start` jumps there, for a voice that was silent.
fn voice_control(
    voice: &mut Voice,
    s: &Settings,
    m: &Modulation,
    sample: &SampleData,
    rate: f32,
    start: bool,
) {
    voice.pitch += (voice.note.pitch - voice.pitch) * m.glide;
    // The markers follow their knobs while a voice plays.
    voice.region = Region::new(
        sample.frames(),
        &s.markers(sample.sample_rate(), voice.reverse),
    );
    let semitones = s.transposition(voice.pitch) + m.bend + m.lfo * m.lfo_pitch;
    let step = ((semitones / 12.0).exp2() * sample.sample_rate() / rate)
        .min(crate::dsp::resample::MAX_STEP);
    let from = voice.step.target();
    voice.step.to(step, start);
    // For the fastest step of the period, so the cutoff is never too high.
    voice.reader = Some(Step::new(if start { step } else { step.max(from) }));
    let fenv = voice.filter_envelope.next_value();
    let octaves = m.cutoff_octaves
        + m.filter_env * fenv
        + m.key_tracking * (voice.pitch - 60.0) / 12.0
        + m.velocity_cutoff * (voice.note.velocity - 1.0)
        + m.lfo * m.lfo_cutoff;
    voice.cutoff = octaves.exp2().clamp(20.0, 20_000.0);
    voice.g.to(prewarp(voice.cutoff, rate), start);
    // A dip from full level, so the LFO never raises a note above its velocity.
    let level = (1.0 - m.velocity_level * (1.0 - voice.note.velocity))
        * (1.0 - m.lfo_level * (0.5 - 0.5 * m.lfo));
    let pan = (m.pan + m.lfo * m.lfo_pan).clamp(-1.0, 1.0);
    voice.gains[0].to(level * (1.0 - pan).min(1.0), start);
    voice.gains[1].to(level * (1.0 + pan).min(1.0), start);
}

impl Device for Sampler {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        let control_rate = self.sample_rate / CONTROL as f32;
        for smoothed in self.control_smoothers() {
            smoothed.set_time(5.0, control_rate);
        }
        self.bend.set_time(10.0, control_rate);
        self.gain.set_time(5.0, self.sample_rate);
        self.dirty = true;
        self.reset();
    }

    fn set_parameter(&mut self, id: u32, normalized: f32) {
        if let Some(index) = PARAMETERS.iter().position(|spec| spec.id == id) {
            let normalized = if normalized.is_finite() {
                normalized.clamp(0.0, 1.0)
            } else {
                PARAMETERS[index].default_normalized()
            };
            if self.values[index] != normalized {
                self.values[index] = normalized;
                self.dirty = true;
            }
        }
    }

    fn set_sample(&mut self, slot: u32, sample: Option<Arc<SampleData>>) {
        if slot == 0 {
            self.sample = sample.filter(|sample| sample.frames() > 0);
            for voice in &mut self.voices {
                voice.reset();
            }
            // The reset voices take their envelope times again.
            self.dirty = true;
        }
    }

    fn process(&mut self, context: &ProcessContext, events: &[Event], io: StereoIo<'_>) {
        if self.dirty {
            self.update_settings();
        }
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        left.fill(0.0);
        right.fill(0.0);
        let sample = self.sample.clone();
        let mut event = 0;
        let mut frame = 0;
        while frame < frames {
            if self.countdown == 0 {
                self.control(context, frame);
                self.countdown = CONTROL;
            }
            while let Some(next) = events.get(event)
                && (next.offset as usize).min(frames - 1) <= frame
            {
                self.handle(next.kind);
                event += 1;
            }
            let next_event = events
                .get(event)
                .map_or(frames, |next| (next.offset as usize).min(frames - 1));
            let span = self
                .countdown
                .min(frames - frame)
                .min(next_event.saturating_sub(frame).max(1));
            if let Some(sample) = &sample {
                self.render(
                    sample,
                    &mut left[frame..frame + span],
                    &mut right[frame..frame + span],
                );
            }
            self.countdown -= span;
            frame += span;
        }
        for (left, right) in left.iter_mut().zip(right.iter_mut()) {
            let gain = self.gain.next_value();
            *left *= gain;
            *right *= gain;
        }
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        if self.dirty {
            self.update_settings();
        }
        for voice in &mut self.voices {
            voice.reset();
        }
        self.dirty = true;
        self.update_settings();
        self.held_count = 0;
        self.sustain = false;
        self.bend.set_target(0.0);
        self.lfo.reset();
        self.countdown = 0;
        self.age = 0;
        self.last_pitch = 60.0;
        self.modulation = Modulation::default();
        for smoothed in self.control_smoothers() {
            smoothed.snap();
        }
        self.gain.snap();
        for index in 0..MAX_VOICES {
            self.telemetry.set_value(TELEMETRY_PLAYHEADS + index, -1.0);
        }
    }

    fn tail_samples(&self) -> u32 {
        // The release reaches −80 dB in about twice its set time; a one-shot
        // plays its recording out first.
        let release = self.settings.amp_envelope[4] * 0.001 * 2.0;
        let recording = match (&self.sample, self.settings.loop_mode) {
            (Some(sample), LoopMode::OneShot) => sample.frames() as f32 / sample.sample_rate(),
            _ => 0.0,
        };
        ((release + recording + 0.05) * self.sample_rate) as u32
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}
