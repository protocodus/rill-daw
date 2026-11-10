//! R1 Mini: two circuit-modelled oscillators (VCO or DCO), sub and noise into
//! the shared saturating filter, two envelopes and one LFO, up to eight
//! voices. Each voice runs at 2× from the oscillators to the filter, so the
//! oscillator edges, the overdriven mixer and the filter's saturation do not
//! alias. See `docs/R1_DEVICES.md#r1-mini`.
use super::param;
use crate::dsp::filter::{self, FilterKernel, FilterModel, FilterShape, FilterType};
use crate::dsp::oscillator::Corrector;
use crate::dsp::{
    Adsr, Drift, Lfo, LfoShape, OnePole, Oscillator, OscillatorModel, Oversampler2x, Random,
    Smoothed, Stage, SubOscillator, Waveform, db_to_gain, prewarp,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, EventKind, ParamRange, ParamSpec, ProcessContext,
    StereoIo, Telemetry,
};
use std::sync::Arc;

pub const OSC1_MODEL: u32 = 1;
pub const OSC1_WAVE: u32 = 2;
pub const OSC1_OCTAVE: u32 = 3;
pub const OSC1_WIDTH: u32 = 4;
pub const OSC2_MODEL: u32 = 10;
pub const OSC2_WAVE: u32 = 11;
pub const OSC2_OCTAVE: u32 = 12;
pub const OSC2_SEMITONES: u32 = 13;
pub const OSC2_DETUNE: u32 = 14;
pub const OSC2_WIDTH: u32 = 15;
pub const OSC2_SYNC: u32 = 16;
pub const MIX_OSC1: u32 = 20;
pub const MIX_OSC2: u32 = 21;
pub const MIX_SUB: u32 = 22;
pub const MIX_NOISE: u32 = 23;
pub const FILTER_MODEL: u32 = 30;
pub const CUTOFF: u32 = 31;
pub const RESONANCE: u32 = 32;
pub const FILTER_ENV: u32 = 33;
pub const KEY_TRACKING: u32 = 34;
pub const DRIVE: u32 = 35;
pub const FENV_ATTACK: u32 = 40;
pub const FENV_DECAY: u32 = 41;
pub const FENV_SUSTAIN: u32 = 42;
pub const FENV_RELEASE: u32 = 43;
pub const AENV_ATTACK: u32 = 50;
pub const AENV_DECAY: u32 = 51;
pub const AENV_SUSTAIN: u32 = 52;
pub const AENV_RELEASE: u32 = 53;
pub const LFO_RATE: u32 = 60;
pub const LFO_SYNC: u32 = 61;
pub const LFO_SHAPE: u32 = 62;
pub const LFO_PITCH: u32 = 63;
pub const LFO_CUTOFF: u32 = 64;
pub const LFO_WIDTH: u32 = 65;
pub const LFO_WHEEL: u32 = 66;
pub const VELOCITY_AMP: u32 = 70;
pub const VELOCITY_CUTOFF: u32 = 71;
pub const VOICE_MODE: u32 = 80;
pub const VOICES: u32 = 81;
pub const GLIDE: u32 = 82;
pub const BEND_RANGE: u32 = 83;
pub const DRIFT: u32 = 84;
pub const VOLUME: u32 = 85;

/// Telemetry value slots.
pub const TELEMETRY_VOICES: usize = 0;
pub const TELEMETRY_LFO_PHASE: usize = 1;
pub const TELEMETRY_CUTOFF: usize = 2;

pub const MAX_VOICES: usize = 8;
/// Oscillator cores, named after their circuit, never a manufacturer.
pub const MODELS: &[&str] = &["VCO", "DCO"];
/// The saturating models of the shared filter kernel, low-pass.
pub const FILTER_MODELS: &[&str] = &[
    "Transistor Ladder",
    "OTA Cascade",
    "Sallen-Key",
    "OTA Multimode",
    "Diode Pair",
];
pub const MODES: &[&str] = &["Poly", "Mono", "Legato"];
pub const SYNC: &[&str] = super::filter::SYNC;

/// Base-rate samples between modulation updates (pitch, cutoff, pulse width);
/// values ramp linearly in between.
const CONTROL: usize = 8;
/// The mixer feeds the filter at this level, so one oscillator stays clean
/// and a full mix overdrives the filter input, as on the hardware.
const MIX_LEVEL: f32 = 0.5;
/// How far drift moves each voice's cutoff at 100 %, in octaves.
const CUTOFF_DRIFT_OCTAVES: f32 = 0.08;
/// Largest pulse width swing from the LFO, 0–1 of the cycle.
const LFO_WIDTH_RANGE: f32 = 0.45;
const STEAL_FADE_MS: f32 = 5.0;
/// Below this, glide is off.
const GLIDE_OFF_MS: f32 = 1.0;
/// Coupling capacitor between the mixer and the filter.
const COUPLING_HZ: f32 = 8.0;
const HELD_NOTES: usize = 16;

pub static PARAMETERS: [ParamSpec; 44] = [
    param(
        OSC1_MODEL,
        "osc1.model",
        "Model",
        "Osc 1",
        "",
        ParamRange::Choice(MODELS),
        0.0,
    ),
    param(
        OSC1_WAVE,
        "osc1.wave",
        "Wave",
        "Osc 1",
        "",
        ParamRange::Choice(Waveform::LABELS),
        0.0,
    ),
    param(
        OSC1_OCTAVE,
        "osc1.octave",
        "Octave",
        "Osc 1",
        "oct",
        ParamRange::Integer { min: -2, max: 2 },
        0.0,
    ),
    param(
        OSC1_WIDTH,
        "osc1.width",
        "Width",
        "Osc 1",
        "%",
        ParamRange::Linear {
            min: 5.0,
            max: 95.0,
        },
        50.0,
    ),
    param(
        OSC2_MODEL,
        "osc2.model",
        "Model",
        "Osc 2",
        "",
        ParamRange::Choice(MODELS),
        0.0,
    ),
    param(
        OSC2_WAVE,
        "osc2.wave",
        "Wave",
        "Osc 2",
        "",
        ParamRange::Choice(Waveform::LABELS),
        0.0,
    ),
    param(
        OSC2_OCTAVE,
        "osc2.octave",
        "Octave",
        "Osc 2",
        "oct",
        ParamRange::Integer { min: -2, max: 2 },
        0.0,
    ),
    param(
        OSC2_SEMITONES,
        "osc2.semitones",
        "Semitones",
        "Osc 2",
        "st",
        ParamRange::Integer { min: -12, max: 12 },
        0.0,
    ),
    param(
        OSC2_DETUNE,
        "osc2.detune",
        "Detune",
        "Osc 2",
        "ct",
        ParamRange::Linear {
            min: -50.0,
            max: 50.0,
        },
        7.0,
    ),
    param(
        OSC2_WIDTH,
        "osc2.width",
        "Width",
        "Osc 2",
        "%",
        ParamRange::Linear {
            min: 5.0,
            max: 95.0,
        },
        50.0,
    ),
    param(
        OSC2_SYNC,
        "osc2.sync",
        "Sync",
        "Osc 2",
        "",
        ParamRange::Toggle,
        0.0,
    ),
    param(
        MIX_OSC1,
        "mix.osc1",
        "Osc 1",
        "Mixer",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        100.0,
    ),
    param(
        MIX_OSC2,
        "mix.osc2",
        "Osc 2",
        "Mixer",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        70.0,
    ),
    param(
        MIX_SUB,
        "mix.sub",
        "Sub",
        "Mixer",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        0.0,
    ),
    param(
        MIX_NOISE,
        "mix.noise",
        "Noise",
        "Mixer",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        0.0,
    ),
    param(
        FILTER_MODEL,
        "filter.model",
        "Model",
        "Filter",
        "",
        ParamRange::Choice(FILTER_MODELS),
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
        2000.0,
    ),
    param(
        RESONANCE,
        "filter.resonance",
        "Resonance",
        "Filter",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        15.0,
    ),
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
        2.0,
    ),
    param(
        KEY_TRACKING,
        "filter.keytrack",
        "Key track",
        "Filter",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        50.0,
    ),
    param(
        DRIVE,
        "filter.drive",
        "Drive",
        "Filter",
        "dB",
        ParamRange::Linear {
            min: 0.0,
            max: 24.0,
        },
        0.0,
    ),
    param(
        FENV_ATTACK,
        "fenv.attack",
        "Attack",
        "Filter envelope",
        "ms",
        ParamRange::Log {
            min: 0.5,
            max: 10_000.0,
        },
        2.0,
    ),
    param(
        FENV_DECAY,
        "fenv.decay",
        "Decay",
        "Filter envelope",
        "ms",
        ParamRange::Log {
            min: 1.0,
            max: 10_000.0,
        },
        400.0,
    ),
    param(
        FENV_SUSTAIN,
        "fenv.sustain",
        "Sustain",
        "Filter envelope",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        30.0,
    ),
    param(
        FENV_RELEASE,
        "fenv.release",
        "Release",
        "Filter envelope",
        "ms",
        ParamRange::Log {
            min: 1.0,
            max: 10_000.0,
        },
        300.0,
    ),
    param(
        AENV_ATTACK,
        "aenv.attack",
        "Attack",
        "Amp envelope",
        "ms",
        ParamRange::Log {
            min: 0.5,
            max: 10_000.0,
        },
        2.0,
    ),
    param(
        AENV_DECAY,
        "aenv.decay",
        "Decay",
        "Amp envelope",
        "ms",
        ParamRange::Log {
            min: 1.0,
            max: 10_000.0,
        },
        300.0,
    ),
    param(
        AENV_SUSTAIN,
        "aenv.sustain",
        "Sustain",
        "Amp envelope",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        80.0,
    ),
    param(
        AENV_RELEASE,
        "aenv.release",
        "Release",
        "Amp envelope",
        "ms",
        ParamRange::Log {
            min: 1.0,
            max: 10_000.0,
        },
        250.0,
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
        1.0,
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
    param(
        LFO_WIDTH,
        "lfo.width",
        "Width",
        "LFO",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        0.0,
    ),
    param(
        LFO_WHEEL,
        "lfo.wheel",
        "Mod wheel",
        "LFO",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        100.0,
    ),
    param(
        VELOCITY_AMP,
        "velocity.amp",
        "Amp",
        "Velocity",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        60.0,
    ),
    param(
        VELOCITY_CUTOFF,
        "velocity.cutoff",
        "Cutoff",
        "Velocity",
        "oct",
        ParamRange::Linear { min: 0.0, max: 5.0 },
        0.5,
    ),
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
        8.0,
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
    param(
        DRIFT,
        "voice.drift",
        "Drift",
        "Voice",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        25.0,
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
    id: "r1-mini",
    name: "R1 Mini",
    kind: DeviceKind::Instrument,
    category: "Synth",
    parameters: &PARAMETERS,
    sample_slots: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-mini/default.rillpreset"),
        include_str!("../../presets/r1-mini/rubber-bass.rillpreset"),
        include_str!("../../presets/r1-mini/acid-line.rillpreset"),
        include_str!("../../presets/r1-mini/sync-lead.rillpreset"),
        include_str!("../../presets/r1-mini/string-pad.rillpreset"),
        include_str!("../../presets/r1-mini/glass-keys.rillpreset"),
        include_str!("../../presets/r1-mini/brass-stab.rillpreset"),
        include_str!("../../presets/r1-mini/noise-sweep.rillpreset"),
    ],
    create: || Box::new(Mini::new()),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceMode {
    Poly,
    Mono,
    /// Mono without retriggering the envelopes, gliding only between
    /// overlapping notes.
    Legato,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscSettings {
    pub model: OscillatorModel,
    pub waveform: Waveform,
    /// Semitones from the played note: octave, semitones and detune together.
    pub transpose: f32,
    /// 0–1.
    pub width: f32,
}

/// Plain parameter values, decoded once per change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub osc: [OscSettings; 2],
    pub sync: bool,
    /// Osc 1, Osc 2, Sub, Noise; 0–1.
    pub mix: [f32; 4],
    pub filter_model: FilterModel,
    pub cutoff: f32,
    /// 0–1.
    pub resonance: f32,
    pub filter_env: f32,
    /// 0–1.
    pub key_tracking: f32,
    pub drive_db: f32,
    /// Attack, decay, release in ms; sustain 0–1.
    pub filter_envelope: [f32; 4],
    pub amp_envelope: [f32; 4],
    pub lfo_rate: f32,
    pub lfo_sync_beats: Option<f64>,
    pub lfo_shape: LfoShape,
    pub lfo_pitch: f32,
    pub lfo_cutoff: f32,
    /// 0–1.
    pub lfo_width: f32,
    /// How much the mod wheel controls the LFO depth, 0–1.
    pub lfo_wheel: f32,
    /// 0–1.
    pub velocity_amp: f32,
    pub velocity_cutoff: f32,
    pub mode: VoiceMode,
    pub voices: usize,
    pub glide_ms: f32,
    pub bend_range: f32,
    /// 0–1.
    pub drift: f32,
    pub volume_db: f32,
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("mini parameter");
            spec.to_plain(value(id))
        };
        let index = |id: u32| plain(id) as usize;
        let model = |id| {
            if index(id) == 1 {
                OscillatorModel::Dco
            } else {
                OscillatorModel::Vco
            }
        };
        let sync = super::filter::SYNC_BEATS[index(LFO_SYNC).min(SYNC.len() - 1)];
        let envelope = |attack, decay, sustain, release| {
            [
                plain(attack),
                plain(decay),
                plain(sustain) / 100.0,
                plain(release),
            ]
        };
        Self {
            osc: [
                OscSettings {
                    model: model(OSC1_MODEL),
                    waveform: Waveform::from_index(index(OSC1_WAVE)),
                    transpose: 12.0 * plain(OSC1_OCTAVE),
                    width: plain(OSC1_WIDTH) / 100.0,
                },
                OscSettings {
                    model: model(OSC2_MODEL),
                    waveform: Waveform::from_index(index(OSC2_WAVE)),
                    transpose: 12.0 * plain(OSC2_OCTAVE)
                        + plain(OSC2_SEMITONES)
                        + plain(OSC2_DETUNE) / 100.0,
                    width: plain(OSC2_WIDTH) / 100.0,
                },
            ],
            sync: plain(OSC2_SYNC) >= 0.5,
            mix: [MIX_OSC1, MIX_OSC2, MIX_SUB, MIX_NOISE].map(|id| plain(id) / 100.0),
            filter_model: match index(FILTER_MODEL) {
                1 => FilterModel::OtaCascade,
                2 => FilterModel::SallenKey,
                3 => FilterModel::OtaMultimode,
                4 => FilterModel::DiodePair,
                _ => FilterModel::TransistorLadder,
            },
            cutoff: plain(CUTOFF),
            resonance: plain(RESONANCE) / 100.0,
            filter_env: plain(FILTER_ENV),
            key_tracking: plain(KEY_TRACKING) / 100.0,
            drive_db: plain(DRIVE),
            filter_envelope: envelope(FENV_ATTACK, FENV_DECAY, FENV_SUSTAIN, FENV_RELEASE),
            amp_envelope: envelope(AENV_ATTACK, AENV_DECAY, AENV_SUSTAIN, AENV_RELEASE),
            lfo_rate: plain(LFO_RATE),
            lfo_sync_beats: (sync > 0.0).then_some(sync),
            lfo_shape: LfoShape::from_index(index(LFO_SHAPE)),
            lfo_pitch: plain(LFO_PITCH),
            lfo_cutoff: plain(LFO_CUTOFF),
            lfo_width: plain(LFO_WIDTH) / 100.0,
            lfo_wheel: plain(LFO_WHEEL) / 100.0,
            velocity_amp: plain(VELOCITY_AMP) / 100.0,
            velocity_cutoff: plain(VELOCITY_CUTOFF),
            mode: match index(VOICE_MODE) {
                1 => VoiceMode::Mono,
                2 => VoiceMode::Legato,
                _ => VoiceMode::Poly,
            },
            voices: (plain(VOICES) as usize).clamp(1, MAX_VOICES),
            glide_ms: plain(GLIDE),
            bend_range: plain(BEND_RANGE),
            drift: plain(DRIFT) / 100.0,
            volume_db: plain(VOLUME),
        }
    }

    /// The filter as the voices run it. 24 dB where the model has a choice.
    pub fn filter_shape(&self) -> FilterShape {
        FilterShape {
            model: self.filter_model,
            filter_type: FilterType::LowPass,
            steep: true,
            drive_db: self.drive_db,
        }
    }
}

/// Whether a parameter does anything with the current settings; panels hide
/// the ones that do not.
pub fn applies(id: u32, settings: &Settings) -> bool {
    match id {
        OSC1_WIDTH => settings.osc[0].waveform == Waveform::Pulse,
        OSC2_WIDTH => settings.osc[1].waveform == Waveform::Pulse,
        LFO_WIDTH => settings
            .osc
            .iter()
            .any(|osc| osc.waveform == Waveform::Pulse),
        LFO_RATE => settings.lfo_sync_beats.is_none(),
        VOICES => settings.mode == VoiceMode::Poly,
        _ => true,
    }
}

/// Small-signal magnitude response of the voice filter in dB, for drawing:
/// the cutoff and resonance knobs without envelope, LFO or key tracking.
pub fn response_db(settings: &Settings, sample_rate: f32, frequencies: &[f32], out: &mut [f32]) {
    let shape = settings.filter_shape();
    for (frequency, out) in frequencies.iter().zip(out) {
        let response = filter::response(
            &shape,
            settings.cutoff,
            settings.resonance,
            0.0,
            sample_rate,
            *frequency,
        );
        *out = (20.0 * response.norm().max(1e-9).log10()) as f32;
    }
}

/// A value that moves linearly to its next control-rate target, one step per
/// oversampled sample.
#[derive(Clone, Copy, Debug, Default)]
struct Ramp {
    value: f32,
    step: f32,
}

impl Ramp {
    fn to(&mut self, target: f32, steps: usize) {
        self.step = (target - self.value) / steps as f32;
    }

    fn snap(&mut self, target: f32) {
        self.value = target;
        self.step = 0.0;
    }

    #[inline]
    fn at(&self, step: usize) -> f32 {
        self.value + self.step * step as f32
    }

    fn advance(&mut self, steps: usize) {
        self.value += self.step * steps as f32;
    }
}

/// Values shared by every voice for one control period.
#[derive(Clone, Copy, Debug, Default)]
struct Shared {
    /// Osc 1, Osc 2, Sub, Noise, input drive, output compensation, velocity → amp.
    ramps: [Ramp; 7],
}

const RAMP_DRIVE: usize = 4;
const RAMP_MAKEUP: usize = 5;
const RAMP_VELOCITY: usize = 6;

/// Control-rate modulation every voice reads.
#[derive(Clone, Copy, Debug, Default)]
struct Modulation {
    lfo: f32,
    /// LFO depth scale from the mod wheel.
    depth: f32,
    bend: f32,
    cutoff_octaves: f32,
    resonance: f32,
    filter_env: f32,
    key_tracking: f32,
    velocity_cutoff: f32,
    lfo_pitch: f32,
    lfo_cutoff: f32,
    lfo_width: f32,
    widths: [f32; 2],
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
    note: Note,
    /// Cleared when the key is released during the fade.
    held: bool,
    glide_from: f32,
}

#[derive(Clone, Debug)]
struct Voice {
    oscillators: [Oscillator; 2],
    sub: SubOscillator,
    noise: Random,
    coupling: OnePole,
    filter: FilterKernel,
    downsampler: Oversampler2x,
    amp: Adsr,
    filter_envelope: Adsr,
    cutoff_drift: Drift,
    note: Note,
    /// Current pitch while gliding, in semitones.
    pitch: f32,
    /// The key is down.
    held: bool,
    /// Released while the sustain pedal is down.
    sustained: bool,
    age: u64,
    /// A note waiting for this voice's steal fade.
    pending: Option<Pending>,
    fade: f32,
    g: Ramp,
    widths: [Ramp; 2],
    cutoff: f32,
}

impl Voice {
    fn new(index: u32) -> Self {
        let seed = 0x00C0_FFEE ^ (index * 7919);
        Self {
            oscillators: [Oscillator::new(seed + 1), Oscillator::new(seed + 2)],
            sub: SubOscillator::default(),
            noise: Random::new(seed + 3),
            coupling: OnePole::default(),
            filter: FilterKernel::default(),
            downsampler: Oversampler2x::new(),
            amp: Adsr::default(),
            filter_envelope: Adsr::default(),
            cutoff_drift: Drift::new(seed + 4),
            note: Note::default(),
            pitch: 60.0,
            held: false,
            sustained: false,
            age: 0,
            pending: None,
            fade: 1.0,
            g: Ramp::default(),
            widths: [Ramp::default(); 2],
            cutoff: 1000.0,
        }
    }

    fn reset(&mut self) {
        for oscillator in &mut self.oscillators {
            oscillator.reset();
        }
        self.sub.reset();
        self.noise.reset();
        self.coupling = OnePole::default();
        self.filter.reset();
        self.downsampler.reset();
        self.amp.reset();
        self.filter_envelope.reset();
        self.cutoff_drift.reset();
        self.held = false;
        self.sustained = false;
        self.age = 0;
        self.pending = None;
        self.fade = 1.0;
    }

    fn active(&self) -> bool {
        self.amp.is_active() || self.pending.is_some()
    }

    fn releasing(&self) -> bool {
        !self.held && !self.sustained && self.amp.stage() == Stage::Release
    }

    fn release(&mut self) {
        self.amp.gate_off();
        self.filter_envelope.gate_off();
    }
}

pub struct Mini {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    dirty: bool,
    sample_rate: f32,
    voices: [Voice; MAX_VOICES],
    held: [Note; HELD_NOTES],
    held_count: usize,
    sustain: bool,
    bend: Smoothed,
    wheel: Smoothed,
    lfo: Lfo,
    countdown: usize,
    age: u64,
    /// The last played pitch, where a glide starts.
    last_pitch: f32,
    modulation: Modulation,
    shared: Shared,
    cutoff: Smoothed,
    resonance: Smoothed,
    filter_env: Smoothed,
    key_tracking: Smoothed,
    velocity_cutoff: Smoothed,
    lfo_pitch: Smoothed,
    lfo_cutoff: Smoothed,
    lfo_width: Smoothed,
    widths: [Smoothed; 2],
    mix: [Smoothed; 4],
    drive: Smoothed,
    velocity_amp: Smoothed,
    volume: Smoothed,
    telemetry: Arc<Telemetry>,
}

impl Default for Mini {
    fn default() -> Self {
        Self::new()
    }
}

fn frequency(pitch: f32) -> f32 {
    440.0 * ((pitch - 69.0) / 12.0).exp2()
}

impl Mini {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let settings = Settings::from_normalized(|id| {
            values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
        });
        let mut mini = Self {
            values,
            settings,
            dirty: true,
            sample_rate: 48_000.0,
            voices: std::array::from_fn(|index| Voice::new(index as u32)),
            held: [Note::default(); HELD_NOTES],
            held_count: 0,
            sustain: false,
            bend: Smoothed::new(0.0),
            wheel: Smoothed::new(0.0),
            lfo: Lfo::new(0x0011_1F00),
            countdown: 0,
            age: 0,
            last_pitch: 60.0,
            modulation: Modulation::default(),
            shared: Shared::default(),
            cutoff: Smoothed::new(0.0),
            resonance: Smoothed::new(0.0),
            filter_env: Smoothed::new(0.0),
            key_tracking: Smoothed::new(0.0),
            velocity_cutoff: Smoothed::new(0.0),
            lfo_pitch: Smoothed::new(0.0),
            lfo_cutoff: Smoothed::new(0.0),
            lfo_width: Smoothed::new(0.0),
            widths: [Smoothed::new(0.5), Smoothed::new(0.5)],
            mix: std::array::from_fn(|_| Smoothed::new(0.0)),
            drive: Smoothed::new(0.0),
            velocity_amp: Smoothed::new(0.0),
            volume: Smoothed::new(0.0),
            telemetry: Arc::new(Telemetry::default()),
        };
        mini.prepare(48_000.0, 0);
        mini
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn control_smoothers(&mut self) -> [&mut Smoothed; 17] {
        let [w1, w2] = &mut self.widths;
        let [m1, m2, m3, m4] = &mut self.mix;
        [
            &mut self.bend,
            &mut self.wheel,
            &mut self.cutoff,
            &mut self.resonance,
            &mut self.filter_env,
            &mut self.key_tracking,
            &mut self.velocity_cutoff,
            &mut self.lfo_pitch,
            &mut self.lfo_cutoff,
            &mut self.lfo_width,
            w1,
            w2,
            m1,
            m2,
            m3,
            m4,
            &mut self.drive,
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
                voice.release();
            }
            self.held_count = 0;
        }
        if settings.filter_model != self.settings.filter_model {
            for voice in &mut self.voices {
                voice.filter.reset();
            }
        }
        let s = settings;
        self.settings = s;
        self.cutoff.set_target(s.cutoff.log2());
        self.resonance.set_target(s.resonance);
        self.filter_env.set_target(s.filter_env);
        self.key_tracking.set_target(s.key_tracking);
        self.velocity_cutoff.set_target(s.velocity_cutoff);
        self.lfo_pitch.set_target(s.lfo_pitch);
        self.lfo_cutoff.set_target(s.lfo_cutoff);
        self.lfo_width.set_target(s.lfo_width);
        for (smoothed, osc) in self.widths.iter_mut().zip(s.osc) {
            smoothed.set_target(osc.width);
        }
        for (smoothed, level) in self.mix.iter_mut().zip(s.mix) {
            smoothed.set_target(level);
        }
        self.drive.set_target(db_to_gain(s.drive_db));
        self.velocity_amp.set_target(s.velocity_amp);
        self.volume.set_target(db_to_gain(s.volume_db));
        let rate = self.sample_rate;
        for voice in &mut self.voices {
            let [a, d, sustain, r] = s.amp_envelope;
            voice.amp.set(a, d, sustain, r, rate);
            let [a, d, sustain, r] = s.filter_envelope;
            voice.filter_envelope.set(a, d, sustain, r, rate);
            for (oscillator, osc) in voice.oscillators.iter_mut().zip(s.osc) {
                oscillator.set_model(osc.model);
                oscillator.set_drift(s.drift);
            }
        }
        self.dirty = false;
    }

    /// Global modulation and the shared ramps, once per control period.
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
        let wheel = self.wheel.next_value();
        let glide_samples = s.glide_ms * 0.001 * rate / CONTROL as f32;
        self.modulation = Modulation {
            lfo: self.lfo.value(s.lfo_shape, 0.0),
            depth: 1.0 - s.lfo_wheel * (1.0 - wheel),
            bend: self.bend.next_value() * s.bend_range,
            cutoff_octaves: self.cutoff.next_value(),
            resonance: self.resonance.next_value(),
            filter_env: self.filter_env.next_value(),
            key_tracking: self.key_tracking.next_value(),
            velocity_cutoff: self.velocity_cutoff.next_value(),
            lfo_pitch: self.lfo_pitch.next_value(),
            lfo_cutoff: self.lfo_cutoff.next_value(),
            lfo_width: self.lfo_width.next_value(),
            widths: [self.widths[0].next_value(), self.widths[1].next_value()],
            // Exponential like an RC portamento; 99 % of the way in the set time.
            glide: if s.glide_ms <= GLIDE_OFF_MS {
                1.0
            } else {
                1.0 - (-4.6 / glide_samples.max(1.0)).exp()
            },
        };
        let steps = 2 * CONTROL;
        let drive = self.drive.next_value();
        let targets = [
            MIX_LEVEL * self.mix[0].next_value(),
            MIX_LEVEL * self.mix[1].next_value(),
            MIX_LEVEL * self.mix[2].next_value(),
            MIX_LEVEL * self.mix[3].next_value(),
            drive,
            1.0 / drive.sqrt(),
            self.velocity_amp.next_value(),
        ];
        for (ramp, target) in self.shared.ramps.iter_mut().zip(targets) {
            ramp.to(target, steps);
        }
        let modulation = self.modulation;
        for voice in &mut self.voices {
            if voice.active() {
                voice_control(voice, &s, &modulation, rate, false);
            }
        }
        let active = self.voices.iter().filter(|voice| voice.active()).count();
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
        voice.amp.gate_on();
        voice.filter_envelope.gate_on();
        voice_control(voice, &s, &modulation, rate, !was_active);
    }

    fn note_on(&mut self, id: u32, pitch: f32, velocity: f32) {
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
        // The same key again retriggers its voice.
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
        let releasing = self.voices[..count]
            .iter()
            .enumerate()
            .filter(|(_, voice)| voice.pending.is_none() && voice.releasing())
            .min_by(|(_, a), (_, b)| a.amp.level().total_cmp(&b.amp.level()))
            .map(|(index, _)| index);
        let index = releasing.unwrap_or_else(|| {
            self.voices[..count]
                .iter()
                .enumerate()
                .min_by_key(|(_, voice)| voice.age)
                .map_or(0, |(index, _)| index)
        });
        let voice = &mut self.voices[index];
        // A voice already fading takes the newer note instead.
        voice.pending = Some(Pending {
            note,
            held: true,
            glide_from,
        });
        voice.held = false;
        voice.sustained = false;
        self.age += 1;
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
        if !voice.amp.is_active() {
            self.start(0, note, if legato { note.pitch } else { glide_from });
            return;
        }
        // Already sounding: Mono glides and retriggers; Legato glides between
        // overlapping notes and otherwise starts afresh at the new pitch.
        voice.note = note;
        voice.held = true;
        voice.sustained = false;
        if !legato || !overlapping {
            voice.amp.gate_on();
            voice.filter_envelope.gate_on();
        }
        if legato && !overlapping {
            voice.pitch = note.pitch;
        }
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
        let mono = self.settings.mode != VoiceMode::Poly;
        if mono {
            self.remove_held(id);
            let voice = &mut self.voices[0];
            if voice.note.id == id && self.held_count > 0 {
                // Back to the most recent key still held, without retrigger.
                voice.note = self.held[self.held_count - 1];
                return;
            }
        }
        let sustain = self.sustain;
        for voice in &mut self.voices {
            if let Some(pending) = &mut voice.pending
                && pending.note.id == id
            {
                pending.held = false;
            }
            if voice.held && voice.note.id == id {
                voice.held = false;
                if sustain {
                    voice.sustained = true;
                } else {
                    voice.release();
                }
            }
        }
    }

    fn set_sustain(&mut self, on: bool) {
        self.sustain = on;
        if !on {
            for voice in &mut self.voices {
                if voice.sustained {
                    voice.sustained = false;
                    voice.release();
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
            EventKind::ModWheel(value) => self.wheel.set_target(value.clamp(0.0, 1.0)),
            EventKind::Sustain(on) => self.set_sustain(on),
            EventKind::Pressure { .. } => {}
        }
    }

    /// Renders `frames` samples of every voice into `out`, starting at
    /// oversampled step 0 of the shared ramps.
    fn render(&mut self, out: &mut [f32]) {
        let context = TickContext {
            settings: self.settings,
            shared: self.shared,
            shape: self.settings.filter_shape(),
            resonance: self.modulation.resonance,
            fade_step: 1.0 / (STEAL_FADE_MS * 0.001 * self.sample_rate),
            coupling: coupling_g(self.sample_rate),
        };
        let sounding: [bool; MAX_VOICES] = std::array::from_fn(|index| self.voices[index].active());
        let steps = 2 * out.len();
        debug_assert!(out.len() <= CONTROL);
        // In three passes over the span, so the filters, each a serial chain
        // of saturating stages, run for all voices in one tight loop where the
        // processor can overlap the independent chains.
        let mut signal = [[0.0_f32; 2 * CONTROL]; MAX_VOICES];
        for ((voice, signal), _) in self
            .voices
            .iter_mut()
            .zip(&mut signal)
            .zip(&sounding)
            .filter(|(_, sounding)| **sounding)
        {
            for (step, slot) in signal[..steps].iter_mut().enumerate() {
                *slot = voice.source(step, &context);
            }
        }
        for step in 0..steps {
            for ((voice, signal), _) in self
                .voices
                .iter_mut()
                .zip(&mut signal)
                .zip(&sounding)
                .filter(|(_, sounding)| **sounding)
            {
                signal[step] = voice.filter.circuit(
                    signal[step],
                    &context.shape,
                    voice.g.at(step),
                    context.resonance,
                );
            }
        }
        for ((voice, signal), _) in self
            .voices
            .iter_mut()
            .zip(&signal)
            .zip(&sounding)
            .filter(|(_, sounding)| **sounding)
        {
            for (frame, sample) in out.iter_mut().enumerate() {
                *sample +=
                    voice.output(frame, [signal[2 * frame], signal[2 * frame + 1]], &context);
            }
        }
        for index in 0..MAX_VOICES {
            let voice = &mut self.voices[index];
            for ramp in voice.widths.iter_mut().chain([&mut voice.g]) {
                ramp.advance(steps);
            }
            if voice.pending.is_some() && voice.fade == 0.0 {
                // The fade ended inside this span; the stolen voice starts
                // its new note with the next span.
                let pending = voice.pending.take().expect("pending note");
                voice.fade = 1.0;
                voice.amp.reset();
                voice.filter_envelope.reset();
                self.start(index, pending.note, pending.glide_from);
                if !pending.held {
                    self.voices[index].held = false;
                    self.voices[index].release();
                }
            }
        }
    }
}

/// What every voice reads while rendering one span.
struct TickContext {
    settings: Settings,
    shared: Shared,
    shape: FilterShape,
    resonance: f32,
    fade_step: f32,
    coupling: f32,
}

impl Voice {
    /// The filter input at one oversampled step: oscillators, sub and noise
    /// through the mixer, the coupling capacitor and the drive.
    #[inline]
    fn source(&mut self, step: usize, c: &TickContext) -> f32 {
        let s = &c.settings;
        let ramps = &c.shared.ramps;
        let (first, clock) =
            self.oscillators[0].process(s.osc[0].waveform, self.widths[0].at(step), None);
        let (second, _) = self.oscillators[1].process(
            s.osc[1].waveform,
            self.widths[1].at(step),
            clock.filter(|_| s.sync),
        );
        let sub = self.sub.process(clock);
        let noise = self.noise.bipolar();
        let mixed = ramps[0].at(step) * first
            + ramps[1].at(step) * second
            + ramps[2].at(step) * sub
            + ramps[3].at(step) * noise;
        let coupled = mixed - self.coupling.low(mixed, c.coupling);
        coupled * ramps[RAMP_DRIVE].at(step)
    }

    /// One base-rate output sample from two filtered oversampled steps.
    #[inline]
    fn output(&mut self, frame: usize, filtered: [f32; 2], c: &TickContext) -> f32 {
        let ramps = &c.shared.ramps;
        let filtered = self.downsampler.downsample(filtered) * ramps[RAMP_MAKEUP].at(2 * frame);
        self.filter_envelope.next_value();
        let velocity = ramps[RAMP_VELOCITY].at(2 * frame);
        let gain = self.amp.next_value() * (1.0 - velocity * (1.0 - self.note.velocity));
        if self.pending.is_some() {
            self.fade = (self.fade - c.fade_step).max(0.0);
        }
        filtered * gain * self.fade
    }
}

fn coupling_g(sample_rate: f32) -> f32 {
    let g = prewarp(COUPLING_HZ, 2.0 * sample_rate);
    g / (1.0 + g)
}

/// Pitch, cutoff and pulse width of one voice for the next control period.
/// `start` jumps there instead of ramping, for a voice that was silent.
fn voice_control(voice: &mut Voice, s: &Settings, m: &Modulation, rate: f32, start: bool) {
    let steps = 2 * CONTROL;
    voice.pitch += (voice.note.pitch - voice.pitch) * m.glide;
    let lfo = m.lfo * m.depth;
    let pitch = voice.pitch + m.bend + lfo * m.lfo_pitch;
    for (oscillator, osc) in voice.oscillators.iter_mut().zip(s.osc) {
        oscillator.set_frequency(
            frequency(pitch + osc.transpose),
            if start { 1 } else { steps as u32 },
        );
    }
    let drift = voice.cutoff_drift.next_value() * s.drift * CUTOFF_DRIFT_OCTAVES;
    let octaves = m.cutoff_octaves
        + m.filter_env * voice.filter_envelope.level()
        + m.key_tracking * (voice.pitch - 60.0) / 12.0
        + m.velocity_cutoff * (voice.note.velocity - 1.0)
        + lfo * m.lfo_cutoff
        + drift;
    voice.cutoff = octaves.exp2().clamp(20.0, 20_000.0);
    let g = prewarp(voice.cutoff, 2.0 * rate);
    let widths = m
        .widths
        .map(|width| (width + lfo * m.lfo_width * LFO_WIDTH_RANGE).clamp(0.05, 0.95));
    if start {
        voice.g.snap(g);
        for (ramp, width) in voice.widths.iter_mut().zip(widths) {
            ramp.snap(width);
        }
    } else {
        voice.g.to(g, steps);
        for (ramp, width) in voice.widths.iter_mut().zip(widths) {
            ramp.to(width, steps);
        }
    }
}

impl Device for Mini {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        let rate = self.sample_rate;
        let control_rate = rate / CONTROL as f32;
        for smoothed in self.control_smoothers() {
            smoothed.set_time(5.0, control_rate);
        }
        // Controllers arrive as steps; this keeps bends and wheel moves smooth.
        self.bend.set_time(10.0, control_rate);
        self.wheel.set_time(10.0, control_rate);
        self.velocity_amp.set_time(5.0, control_rate);
        self.volume.set_time(5.0, rate);
        for voice in &mut self.voices {
            for oscillator in &mut voice.oscillators {
                oscillator.prepare(2.0 * rate, control_rate);
            }
            voice.cutoff_drift.prepare(control_rate);
        }
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

    fn process(&mut self, context: &ProcessContext, events: &[Event], io: StereoIo<'_>) {
        if self.dirty {
            self.update_settings();
        }
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        left.fill(0.0);
        let mut event = 0;
        let mut frame = 0;
        while frame < frames {
            // Modulation first, so a note starting here uses this period's values.
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
            self.render(&mut left[frame..frame + span]);
            for ramp in &mut self.shared.ramps {
                ramp.advance(2 * span);
            }
            self.countdown -= span;
            frame += span;
        }
        for sample in left.iter_mut() {
            *sample *= self.volume.next_value();
        }
        right.copy_from_slice(left);
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        if self.dirty {
            self.update_settings();
        }
        for voice in &mut self.voices {
            voice.reset();
        }
        self.held_count = 0;
        self.sustain = false;
        self.bend.set_target(0.0);
        self.wheel.set_target(0.0);
        self.lfo.reset();
        self.countdown = 0;
        self.age = 0;
        self.last_pitch = 60.0;
        self.shared = Shared::default();
        self.modulation = Modulation::default();
        for smoothed in self.control_smoothers() {
            smoothed.snap();
        }
        self.velocity_amp.snap();
        self.volume.snap();
    }

    fn latency_samples(&self) -> u32 {
        LATENCY
    }

    fn tail_samples(&self) -> u32 {
        // The release reaches −80 dB in about twice its set time.
        let release = self.settings.amp_envelope[3] * 0.001 * 2.0;
        ((release + 0.05) * self.sample_rate) as u32
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}

/// The edge corrections and the downsampler's half-band filter, in base-rate
/// samples (16.5, rounded down).
pub const LATENCY: u32 = (Corrector::LATENCY + Oversampler2x::LATENCY) / 2;
