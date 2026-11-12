//! R1 Drums: sixteen pads on MIDI notes 36–51, one recording each in sample
//! slots 0–15. A hit reads its pad's recording through `daw_dsp`'s
//! band-limited mip-map at the pitch its tune and pitch envelope ask for,
//! walks the pad's start and end markers with the shared playhead and
//! shapes it with an attack, hold and decay. Each pad then sums its voices
//! into one short chain: a one-knob filter (low-pass left, high-pass right,
//! nothing in the middle), a drive, pan and level. Up to four hits of one
//! pad overlap, so rolls ring on; a choke group or Gate cuts them in 5 ms.
//! See `docs/R1_DEVICES.md#r1-drums`.
use super::param;
use crate::dsp::playback::{Markers, Playhead, Region};
use crate::dsp::{
    Adsr, Drive, DriveGains, LoopMode, SincBank, Smoothed, Stage, Step, Svf, SvfMixRamp,
    db_to_gain, sweep_mix,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, EventKind, ParamRange, ParamSpec, ProcessContext,
    SampleData, StereoIo, Telemetry,
};
use std::sync::Arc;

/// Pads, sample slots and notes.
pub const PADS: usize = 16;
/// The note of pad 1; pad `n` plays on `FIRST_NOTE + n − 1`.
pub const FIRST_NOTE: u8 = 36;
/// Each pad owns this many parameter IDs, so it can gain parameters later
/// without renumbering another.
pub const PAD_BLOCK: u32 = 32;

/// The kit's own parameter, below the first pad's block.
pub const VOLUME: u32 = 1;

/// Offsets of a pad's parameters in its block; [`id`] gives the ID.
pub const START: u32 = 0;
pub const END: u32 = 1;
pub const REVERSE: u32 = 2;
pub const PITCH: u32 = 3;
pub const FINE: u32 = 4;
pub const PITCH_ENV: u32 = 5;
pub const ENV_DECAY: u32 = 6;
pub const ATTACK: u32 = 7;
pub const HOLD: u32 = 8;
pub const DECAY: u32 = 9;
pub const TRIGGER: u32 = 10;
pub const FILTER: u32 = 11;
pub const RESONANCE: u32 = 12;
pub const DRIVE: u32 = 13;
pub const VELOCITY: u32 = 14;
pub const CHOKE: u32 = 15;
pub const LEVEL: u32 = 16;
pub const PAN: u32 = 17;
pub const MUTE: u32 = 18;
/// Parameters of one pad.
pub const PAD_PARAMETERS: u32 = 19;

/// Telemetry value slots. Activity: one per pad from
/// [`TELEMETRY_ACTIVITY`], the loudest envelope of its hits, 0–1.
/// Playheads: one per pad from [`TELEMETRY_PLAYHEADS`], where its latest hit
/// is as a share of the recording, or −1.
pub const TELEMETRY_ACTIVITY: usize = 0;
pub const TELEMETRY_PLAYHEADS: usize = 16;
pub const TELEMETRY_VOICES: usize = 32;

pub const TRIGGERS: &[&str] = &["One-shot", "Gate"];
pub const CHOKES: &[&str] = &["Off", "1", "2", "3", "4"];

/// Hits of one pad that sound together; a fifth fades the oldest out.
pub const VOICES_PER_PAD: usize = 4;
/// Room for hits fading out while new ones start.
const SLOTS_PER_PAD: usize = VOICES_PER_PAD + 2;
/// A Hold at its maximum lasts as long as the recording.
pub const HOLD_FULL_MS: f32 = 2000.0;
/// A pad level at its minimum is silence.
pub const LEVEL_OFF_DB: f32 = -60.0;

/// Output samples between modulation updates; values ramp in between.
const CONTROL: usize = 16;
/// Choked and stolen hits fade over this long.
const CHOKE_FADE_MS: f32 = 5.0;
/// A hit reaching its end marker fades over this long, so it never cuts.
const END_FADE_MS: f32 = 2.0;
/// A pad's filter and drive run on at least this long after its last hit,
/// then until they are silent.
const TAIL_MS: f32 = 100.0;
/// How long a resonant filter may ring after its last hit, for the length
/// of a render.
const RING_MS: f32 = 1000.0;
/// Below this an envelope that only decays is over (−80 dB).
const SILENT: f32 = 1e-4;

/// The ID of parameter `offset` of pad `pad` (0–15).
pub const fn id(pad: usize, offset: u32) -> u32 {
    PAD_BLOCK * (pad as u32 + 1) + offset
}

/// The pad (0–15) and offset of a pad parameter's ID.
pub fn pad_of(id: u32) -> Option<(usize, u32)> {
    let pad = (id / PAD_BLOCK).checked_sub(1)? as usize;
    let offset = id % PAD_BLOCK;
    (pad < PADS && offset < PAD_PARAMETERS).then_some((pad, offset))
}

/// The pad a note plays, if any.
pub fn pad_of_note(pitch: f32) -> Option<usize> {
    let pad = pitch.round() - f32::from(FIRST_NOTE);
    (0.0..PADS as f32).contains(&pad).then_some(pad as usize)
}

/// Where a parameter's value sits in [`PARAMETERS`] and in a device's
/// values.
fn index(id: u32) -> Option<usize> {
    if id == VOLUME {
        return Some(0);
    }
    pad_of(id).map(|(pad, offset)| 1 + pad * PAD_PARAMETERS as usize + offset as usize)
}

/// Parameter `offset` of pad `number` (1–16), with its persisted key.
const fn pad_parameter(
    number: u32,
    offset: u32,
    key: &'static str,
    group: &'static str,
) -> ParamSpec {
    const fn linear(min: f32, max: f32) -> ParamRange {
        ParamRange::Linear { min, max }
    }
    const fn log(min: f32, max: f32) -> ParamRange {
        ParamRange::Log { min, max }
    }
    let (name, unit, range, default) = match offset {
        START => ("Start", "%", linear(0.0, 100.0), 0.0),
        END => ("End", "%", linear(0.0, 100.0), 100.0),
        REVERSE => ("Reverse", "", ParamRange::Toggle, 0.0),
        PITCH => (
            "Pitch",
            "st",
            ParamRange::Integer { min: -24, max: 24 },
            0.0,
        ),
        FINE => ("Fine", "ct", linear(-100.0, 100.0), 0.0),
        PITCH_ENV => ("Pitch env", "st", linear(-48.0, 48.0), 0.0),
        ENV_DECAY => ("Env decay", "ms", log(5.0, 500.0), 50.0),
        ATTACK => ("Attack", "ms", log(0.1, 100.0), 0.1),
        HOLD => ("Hold", "ms", log(1.0, HOLD_FULL_MS), HOLD_FULL_MS),
        DECAY => ("Decay", "ms", log(5.0, 10_000.0), 300.0),
        TRIGGER => ("Trigger", "", ParamRange::Choice(TRIGGERS), 0.0),
        FILTER => ("Filter", "%", linear(-100.0, 100.0), 0.0),
        RESONANCE => ("Resonance", "%", linear(0.0, 100.0), 0.0),
        DRIVE => ("Drive", "dB", linear(0.0, 24.0), 0.0),
        VELOCITY => ("Velocity", "%", linear(0.0, 100.0), 80.0),
        CHOKE => ("Choke", "", ParamRange::Choice(CHOKES), 0.0),
        LEVEL => ("Volume", "dB", linear(LEVEL_OFF_DB, 12.0), 0.0),
        PAN => ("Pan", "%", linear(-100.0, 100.0), 0.0),
        _ => ("Mute", "", ParamRange::Toggle, 0.0),
    };
    param(
        PAD_BLOCK * number + offset,
        key,
        name,
        group,
        unit,
        range,
        default,
    )
}

/// Every parameter: the kit's volume, then the pads in order, each in the
/// order of its offsets.
macro_rules! parameters {
    ($($n:literal)*) => {
        [
            param(
                VOLUME,
                "volume",
                "Volume",
                "Kit",
                "dB",
                ParamRange::Linear { min: -36.0, max: 6.0 },
                -6.0,
            ),
            $(
                pad_parameter($n, START, concat!("pad", $n, ".start"), concat!("Pad ", $n)),
                pad_parameter($n, END, concat!("pad", $n, ".end"), concat!("Pad ", $n)),
                pad_parameter($n, REVERSE, concat!("pad", $n, ".reverse"), concat!("Pad ", $n)),
                pad_parameter($n, PITCH, concat!("pad", $n, ".pitch"), concat!("Pad ", $n)),
                pad_parameter($n, FINE, concat!("pad", $n, ".fine"), concat!("Pad ", $n)),
                pad_parameter($n, PITCH_ENV, concat!("pad", $n, ".pitch_env"), concat!("Pad ", $n)),
                pad_parameter($n, ENV_DECAY, concat!("pad", $n, ".env_decay"), concat!("Pad ", $n)),
                pad_parameter($n, ATTACK, concat!("pad", $n, ".attack"), concat!("Pad ", $n)),
                pad_parameter($n, HOLD, concat!("pad", $n, ".hold"), concat!("Pad ", $n)),
                pad_parameter($n, DECAY, concat!("pad", $n, ".decay"), concat!("Pad ", $n)),
                pad_parameter($n, TRIGGER, concat!("pad", $n, ".trigger"), concat!("Pad ", $n)),
                pad_parameter($n, FILTER, concat!("pad", $n, ".filter"), concat!("Pad ", $n)),
                pad_parameter($n, RESONANCE, concat!("pad", $n, ".resonance"), concat!("Pad ", $n)),
                pad_parameter($n, DRIVE, concat!("pad", $n, ".drive"), concat!("Pad ", $n)),
                pad_parameter($n, VELOCITY, concat!("pad", $n, ".velocity"), concat!("Pad ", $n)),
                pad_parameter($n, CHOKE, concat!("pad", $n, ".choke"), concat!("Pad ", $n)),
                pad_parameter($n, LEVEL, concat!("pad", $n, ".volume"), concat!("Pad ", $n)),
                pad_parameter($n, PAN, concat!("pad", $n, ".pan"), concat!("Pad ", $n)),
                pad_parameter($n, MUTE, concat!("pad", $n, ".mute"), concat!("Pad ", $n)),
            )*
        ]
    };
}

pub static PARAMETERS: [ParamSpec; 1 + PADS * PAD_PARAMETERS as usize] =
    parameters!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16);

pub static DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    id: "r1-drums",
    name: "R1 Drums",
    kind: DeviceKind::Instrument,
    category: "Drums",
    parameters: &PARAMETERS,
    sample_slots: PADS as u32,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-drums/default.rillpreset"),
        include_str!("../../presets/r1-drums/gm-kit.rillpreset"),
        include_str!("../../presets/r1-drums/tight-kit.rillpreset"),
        include_str!("../../presets/r1-drums/boom-bap.rillpreset"),
        include_str!("../../presets/r1-drums/electro.rillpreset"),
        include_str!("../../presets/r1-drums/lo-fi.rillpreset"),
    ],
    legacy_labels: &[],
    // Where a roll lands on these, the kit stops being the kit: what of a
    // recording plays, which way, and which pads sound and cut each other.
    fixed_on_roll: &["start", "end", "reverse", "mute", "choke"],
    latency: |_, _| 0,
    create: || Box::new(Drums::new()),
};

/// One pad's plain values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pad {
    /// Shares of the recording, 0–1.
    pub start: f32,
    pub end: f32,
    pub reverse: bool,
    /// Semitones, fine tune included.
    pub pitch: f32,
    /// Semitones at the hit, falling to none over `env_decay_ms`.
    pub pitch_env: f32,
    pub env_decay_ms: f32,
    pub attack_ms: f32,
    /// `None`: as long as the recording.
    pub hold_ms: Option<f32>,
    pub decay_ms: f32,
    /// A released note fades over Decay; a one-shot ignores note-offs.
    pub gate: bool,
    /// −1 (low-pass) to 1 (high-pass); 0 is no filter.
    pub filter: f32,
    /// 0–1.
    pub resonance: f32,
    pub drive_db: f32,
    /// 0–1: how much a soft hit is quieter than a hard one.
    pub velocity: f32,
    /// 0: none.
    pub choke: u8,
    /// Linear; 0 at the bottom of the range.
    pub gain: f32,
    /// −1–1.
    pub pan: f32,
    pub mute: bool,
}

/// Plain parameter values, decoded once per change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub volume_db: f32,
    pub pads: [Pad; PADS],
}

impl Pad {
    /// Pad `pad` (0–15) from normalized values by ID.
    pub fn from_normalized(value: &impl Fn(u32) -> f32, pad: usize) -> Self {
        let plain = |offset: u32| {
            let id = id(pad, offset);
            PARAMETERS[index(id).expect("drum parameter")].to_plain(value(id))
        };
        let hold = plain(HOLD);
        let level = plain(LEVEL);
        Self {
            start: plain(START) / 100.0,
            end: plain(END) / 100.0,
            reverse: plain(REVERSE) >= 0.5,
            pitch: plain(PITCH) + plain(FINE) / 100.0,
            pitch_env: plain(PITCH_ENV),
            env_decay_ms: plain(ENV_DECAY),
            attack_ms: plain(ATTACK),
            hold_ms: (hold < HOLD_FULL_MS * 0.999).then_some(hold),
            decay_ms: plain(DECAY),
            gate: plain(TRIGGER) >= 0.5,
            filter: plain(FILTER) / 100.0,
            resonance: plain(RESONANCE) / 100.0,
            drive_db: plain(DRIVE),
            velocity: plain(VELOCITY) / 100.0,
            choke: plain(CHOKE) as u8,
            gain: if level <= LEVEL_OFF_DB {
                0.0
            } else {
                db_to_gain(level)
            },
            pan: plain(PAN) / 100.0,
            mute: plain(MUTE) >= 0.5,
        }
    }

    /// The markers of the recording, played in the pad's direction.
    pub fn markers(&self) -> Markers {
        Markers {
            start: self.start,
            end: self.end,
            loop_start: 0.0,
            loop_end: 1.0,
            crossfade: 0.0,
            mode: LoopMode::Off,
            reverse: self.reverse,
        }
    }

    /// Gain of a hit at `velocity` (0–1): full at the hardest, falling with
    /// its square as on the velocity curves of drum modules, by as much as
    /// Velocity says.
    pub fn velocity_gain(&self, velocity: f32) -> f32 {
        let velocity = velocity.clamp(0.0, 1.0);
        1.0 - self.velocity + self.velocity * velocity * velocity
    }

    /// The level of the amplifier `seconds` after a hit whose note is held,
    /// as the envelope plays it, for drawing over the recording.
    pub fn envelope(&self, seconds: f32) -> f32 {
        let attack = self.attack_ms * 0.001;
        if seconds < 0.0 {
            return 0.0;
        }
        if seconds < attack {
            // The RC attack of `Adsr`: charging toward 1.3, stopped at 1.
            let charge = (1.3_f32 / 0.3).ln();
            return (1.3 * (1.0 - (-charge * seconds / attack).exp())).min(1.0);
        }
        let Some(hold) = self.hold_ms else {
            return 1.0;
        };
        let decaying = seconds - attack - hold * 0.001;
        if decaying <= 0.0 {
            return 1.0;
        }
        (-4.6 * decaying / (self.decay_ms * 0.001)).exp()
    }
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        Self {
            volume_db: PARAMETERS[0].to_plain(value(VOLUME)),
            pads: std::array::from_fn(|pad| Pad::from_normalized(&value, pad)),
        }
    }
}

/// Whether a parameter does anything with the current settings; panels show
/// the ones that do not disabled in place.
pub fn applies(id: u32, settings: &Settings) -> bool {
    let Some((pad, offset)) = pad_of(id) else {
        return true;
    };
    let pad = &settings.pads[pad];
    match offset {
        ENV_DECAY => pad.pitch_env != 0.0,
        // Without a hold, decay follows the attack; a gate also decays on
        // release.
        DECAY => pad.hold_ms.is_some() || pad.gate,
        RESONANCE => pad.filter != 0.0,
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

/// One hit.
#[derive(Clone, Debug, Default)]
struct Voice {
    active: bool,
    note: u32,
    age: u64,
    /// Velocity gain.
    gain: f32,
    region: Region,
    playhead: Playhead,
    amp: Adsr,
    /// Pitch envelope, 1 at the hit and falling.
    sweep: f32,
    /// Samples of the recording per output sample.
    step: Ramp,
    reader: Option<Step>,
    /// Falls to 0 over [`CHOKE_FADE_MS`] once `fading`.
    fade: f32,
    fading: bool,
}

impl Voice {
    fn level(&self) -> f32 {
        if self.active {
            self.amp.level() * self.fade
        } else {
            0.0
        }
    }
}

/// One pad's chain after its hits.
#[derive(Clone, Debug)]
struct Chain {
    /// Filter position, resonance and drive, smoothed at the control rate.
    position: Smoothed,
    resonance: Smoothed,
    drive_db: Smoothed,
    filter: SvfMixRamp,
    filters: [Svf; 2],
    filtering: bool,
    drive: [Drive; 2],
    /// Now, per sample on the way, and where the ramp ends.
    drive_gains: DriveGains,
    drive_step: DriveGains,
    drive_target: DriveGains,
    driving: bool,
    gains: [Smoothed; 2],
    /// Samples the chain runs on after the last hit.
    tail: usize,
}

impl Default for Chain {
    fn default() -> Self {
        Self {
            position: Smoothed::new(0.0),
            resonance: Smoothed::new(0.0),
            drive_db: Smoothed::new(0.0),
            filter: SvfMixRamp::new(sweep_mix(0.0, 0.0, 48_000.0)),
            filters: Default::default(),
            filtering: false,
            drive: Default::default(),
            drive_gains: DriveGains::default(),
            drive_step: DriveGains::default(),
            drive_target: DriveGains::default(),
            driving: false,
            gains: [Smoothed::new(0.0), Smoothed::new(0.0)],
            tail: 0,
        }
    }
}

pub struct Drums {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    /// Pads whose settings changed, one bit each; bit 16 the kit.
    dirty: u32,
    sample_rate: f32,
    samples: [Option<Arc<SampleData>>; PADS],
    bank: &'static SincBank,
    voices: [Voice; PADS * SLOTS_PER_PAD],
    chains: [Chain; PADS],
    volume: Smoothed,
    countdown: usize,
    age: u64,
    telemetry: Arc<Telemetry>,
}

impl Default for Drums {
    fn default() -> Self {
        Self::new()
    }
}

const KIT: u32 = 1 << PADS;
const ALL: u32 = (KIT << 1) - 1;

impl Drums {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let mut drums = Self {
            values,
            settings: Settings::from_normalized(|id| values[index(id).unwrap()]),
            dirty: ALL,
            sample_rate: 48_000.0,
            samples: Default::default(),
            bank: SincBank::shared(),
            voices: std::array::from_fn(|_| Voice::default()),
            chains: Default::default(),
            volume: Smoothed::new(0.0),
            countdown: 0,
            age: 0,
            telemetry: Arc::new(Telemetry::default()),
        };
        drums.prepare(48_000.0, 0);
        drums
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn update_settings(&mut self) {
        let values = self.values;
        let value = |id: u32| values[index(id).unwrap()];
        if self.dirty & KIT != 0 {
            self.settings.volume_db = PARAMETERS[0].to_plain(value(VOLUME));
            self.volume.set_target(db_to_gain(self.settings.volume_db));
        }
        let rate = self.sample_rate;
        for pad in 0..PADS {
            if self.dirty & (1 << pad) == 0 {
                continue;
            }
            let p = Pad::from_normalized(&value, pad);
            self.settings.pads[pad] = p;
            for voice in self.pad_voices_mut(pad) {
                set_envelope(&mut voice.amp, &p, rate);
            }
            let gain = if p.mute { 0.0 } else { p.gain };
            let chain = &mut self.chains[pad];
            chain.position.set_target(p.filter);
            chain.resonance.set_target(p.resonance);
            chain.drive_db.set_target(p.drive_db);
            chain.gains[0].set_target(gain * (1.0 - p.pan).min(1.0));
            chain.gains[1].set_target(gain * (1.0 + p.pan).min(1.0));
        }
        self.dirty = 0;
    }

    fn pad_voices_mut(&mut self, pad: usize) -> &mut [Voice] {
        &mut self.voices[pad * SLOTS_PER_PAD..(pad + 1) * SLOTS_PER_PAD]
    }

    /// Every pad's chain and hits for the next control period.
    fn control(&mut self) {
        let rate = self.sample_rate;
        let mut sounding = 0;
        for pad in 0..PADS {
            let p = self.settings.pads[pad];
            let chain = &mut self.chains[pad];
            if !(chain.position.is_settled() && chain.resonance.is_settled()) {
                let target = sweep_mix(
                    chain.position.next_value(),
                    chain.resonance.next_value(),
                    rate,
                );
                chain.filter.aim(target, CONTROL as u32);
            }
            let filtering = !(chain.filter.done() && chain.filter.target().is_identity());
            if chain.filtering && !filtering {
                // Settled on the identity: the next sweep starts from rest.
                chain.filters = Default::default();
            }
            chain.filtering = filtering;
            if chain.drive_db.is_settled() {
                // Landed, or never moved while the pad was silent.
                chain.drive_gains = chain.drive_target;
                chain.drive_step = DriveGains::default();
            } else {
                let target = DriveGains::new(chain.drive_db.next_value());
                let from = chain.drive_gains;
                chain.drive_step = DriveGains {
                    input: (target.input - from.input) / CONTROL as f32,
                    output: (target.output - from.output) / CONTROL as f32,
                    blend: (target.blend - from.blend) / CONTROL as f32,
                };
                chain.drive_target = target;
            }
            let driving = !(chain.drive_gains.is_bypass() && chain.drive_target.is_bypass());
            if chain.driving && !driving {
                for stage in &mut chain.drive {
                    stage.reset();
                }
            }
            chain.driving = driving;

            let Some(sample) = &self.samples[pad] else {
                continue;
            };
            let (frames, source_rate) = (sample.frames(), sample.sample_rate());
            let region = Region::new(frames, &p.markers());
            let sweep = (-4.6 * CONTROL as f32 / (p.env_decay_ms * 0.001 * rate)).exp();
            let mut activity = 0.0_f32;
            let mut hits = 0;
            let mut newest: Option<(u64, f32)> = None;
            for voice in &mut self.voices[pad * SLOTS_PER_PAD..(pad + 1) * SLOTS_PER_PAD] {
                if !voice.active {
                    continue;
                }
                if voice.amp.stage() == Stage::Decay && voice.amp.level() < SILENT
                    || !voice.amp.is_active()
                {
                    voice.active = false;
                    continue;
                }
                hits += 1;
                voice.region = region;
                voice_control(voice, &p, source_rate, rate, sweep, false);
                activity = activity.max(voice.level());
                if newest.is_none_or(|(age, _)| voice.age > age) {
                    let at = voice.playhead.position(&voice.region) / frames.max(1) as f64;
                    newest = Some((voice.age, at as f32));
                }
            }
            sounding += hits;
            if hits > 0 {
                self.chains[pad].tail = (TAIL_MS * 0.001 * rate) as usize;
            }
            self.telemetry.set_value(TELEMETRY_ACTIVITY + pad, activity);
            self.telemetry
                .set_value(TELEMETRY_PLAYHEADS + pad, newest.map_or(-1.0, |(_, at)| at));
        }
        self.telemetry.set_value(TELEMETRY_VOICES, sounding as f32);
    }

    fn note_on(&mut self, note: u32, pitch: f32, velocity: f32) {
        let Some(pad) = pad_of_note(pitch) else {
            return;
        };
        let p = self.settings.pads[pad];
        let Some(sample) = self.samples[pad].clone() else {
            return;
        };
        if p.mute {
            return;
        }
        // The other pads of its choke group stop; so do its own earlier
        // hits in a group or on a gate.
        if p.choke != 0 {
            for other in 0..PADS {
                if other != pad && self.settings.pads[other].choke == p.choke {
                    for voice in self.pad_voices_mut(other) {
                        voice.fading = true;
                    }
                }
            }
        }
        let own = &mut self.voices[pad * SLOTS_PER_PAD..(pad + 1) * SLOTS_PER_PAD];
        if p.choke != 0 || p.gate {
            for voice in own.iter_mut() {
                voice.fading = true;
            }
        }
        let sounding = own
            .iter()
            .filter(|voice| voice.active && !voice.fading)
            .count();
        if sounding >= VOICES_PER_PAD
            && let Some(oldest) = own
                .iter_mut()
                .filter(|voice| voice.active && !voice.fading)
                .min_by_key(|voice| voice.age)
        {
            oldest.fading = true;
        }
        // A free slot, else the quietest of those fading out.
        let slot = own
            .iter()
            .position(|voice| !voice.active)
            .unwrap_or_else(|| {
                own.iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.level().total_cmp(&b.level()))
                    .map_or(0, |(slot, _)| slot)
            });
        self.age += 1;
        let rate = self.sample_rate;
        let voice = &mut own[slot];
        voice.active = true;
        voice.note = note;
        voice.age = self.age;
        voice.gain = p.velocity_gain(velocity);
        voice.fade = 1.0;
        voice.fading = false;
        voice.sweep = 1.0;
        voice.region = Region::new(sample.frames(), &p.markers());
        voice.playhead = Playhead::new(&voice.region);
        voice.amp.reset();
        set_envelope(&mut voice.amp, &p, rate);
        voice.amp.gate_on();
        voice_control(voice, &p, sample.sample_rate(), rate, 1.0, true);
        let tail = (TAIL_MS * 0.001 * rate) as usize;
        self.chains[pad].tail = tail;
    }

    fn note_off(&mut self, note: u32) {
        for pad in 0..PADS {
            if !self.settings.pads[pad].gate {
                continue;
            }
            for voice in self.pad_voices_mut(pad) {
                if voice.active && voice.note == note {
                    voice.amp.gate_off();
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
            EventKind::PitchBend(_)
            | EventKind::ModWheel(_)
            | EventKind::Sustain(_)
            | EventKind::Pressure { .. } => {}
        }
    }

    /// Adds `left.len()` samples (at most one control period) of every pad.
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len();
        let fade_step = 1.0 / (CHOKE_FADE_MS * 0.001 * self.sample_rate);
        let end_fade = f64::from(END_FADE_MS * 0.001 * self.sample_rate);
        let bank = self.bank;
        for pad in 0..PADS {
            let chain = &mut self.chains[pad];
            if chain.tail == 0 {
                continue;
            }
            let mut pad_left = [0.0_f32; CONTROL];
            let mut pad_right = [0.0_f32; CONTROL];
            let (pad_left, pad_right) = (&mut pad_left[..frames], &mut pad_right[..frames]);
            if let Some(sample) = &self.samples[pad] {
                let voices = &mut self.voices[pad * SLOTS_PER_PAD..(pad + 1) * SLOTS_PER_PAD];
                render_voices(
                    voices, sample, bank, pad_left, pad_right, fade_step, end_fade,
                );
            }
            let chain = &mut self.chains[pad];
            let mut peak = 0.0_f32;
            for ((out_left, out_right), (l, r)) in left
                .iter_mut()
                .zip(right.iter_mut())
                .zip(pad_left.iter().zip(pad_right.iter()))
            {
                let (mut l, mut r) = (*l, *r);
                if chain.filtering {
                    let mix = chain.filter.advance();
                    let [a, b] = &mut chain.filters;
                    l = a.tick(l, mix);
                    r = b.tick(r, mix);
                }
                if chain.driving {
                    let step = chain.drive_step;
                    let gains = &mut chain.drive_gains;
                    gains.input += step.input;
                    gains.output += step.output;
                    gains.blend += step.blend;
                    let [a, b] = &mut chain.drive;
                    l = a.process(l, gains);
                    r = b.process(r, gains);
                }
                peak = peak.max(l.abs()).max(r.abs());
                *out_left += l * chain.gains[0].next_value();
                *out_right += r * chain.gains[1].next_value();
            }
            chain.tail = chain.tail.saturating_sub(frames);
            if chain.tail == 0 {
                if peak > SILENT * 0.1 {
                    // A resonant filter still rings: drain it rather than
                    // cut it off and resume it on the next hit.
                    chain.tail = frames;
                } else {
                    chain.filters = Default::default();
                    chain.drive = Default::default();
                }
            }
        }
    }
}

/// Attack, hold and decay; Decay is also the release of a gate.
fn set_envelope(amp: &mut Adsr, pad: &Pad, rate: f32) {
    amp.set(pad.attack_ms, pad.decay_ms, 0.0, pad.decay_ms, rate);
    amp.set_hold(pad.hold_ms.unwrap_or(f32::INFINITY), rate);
}

/// Pitch of one hit for the next control period. `start` jumps there, for
/// a new hit.
fn voice_control(
    voice: &mut Voice,
    pad: &Pad,
    source_rate: f32,
    rate: f32,
    sweep: f32,
    start: bool,
) {
    let semitones = pad.pitch + pad.pitch_env * voice.sweep;
    voice.sweep *= sweep;
    let step = ((semitones / 12.0).exp2() * source_rate / rate).min(crate::dsp::resample::MAX_STEP);
    let from = voice.step.target();
    voice.step.to(step, start);
    // For the fastest step of the period, so the cutoff is never too high.
    voice.reader = Some(Step::new(if start { step } else { step.max(from) }));
}

/// Adds one pad's hits to its buffers.
fn render_voices(
    voices: &mut [Voice],
    sample: &SampleData,
    bank: &SincBank,
    left: &mut [f32],
    right: &mut [f32],
    fade_step: f32,
    end_fade: f64,
) {
    let (first, second) = (sample.channel(0), sample.channel(1));
    let stereo = sample.channel_count() > 1;
    for voice in voices {
        if !voice.active {
            continue;
        }
        let Some(reader) = voice.reader else {
            continue;
        };
        for (out_left, out_right) in left.iter_mut().zip(right.iter_mut()) {
            let step = f64::from(voice.step.next());
            let (position, level) = voice.playhead.reads(&voice.region).first;
            let tap = first.tap(bank, &reader, position);
            let l = first.read(&tap);
            let r = if stereo { second.read(&tap) } else { l };
            voice.playhead.advance(&voice.region, step, end_fade);
            if voice.fading {
                voice.fade = (voice.fade - fade_step).max(0.0);
            }
            let gain = level * voice.amp.next_value() * voice.fade * voice.gain;
            *out_left += l * gain;
            *out_right += r * gain;
            if voice.playhead.is_done() || voice.fade == 0.0 {
                voice.active = false;
                voice.amp.reset();
                break;
            }
        }
    }
}

impl Device for Drums {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        self.volume.set_time(5.0, self.sample_rate);
        let control_rate = self.sample_rate / CONTROL as f32;
        for chain in &mut self.chains {
            for smoothed in [
                &mut chain.position,
                &mut chain.resonance,
                &mut chain.drive_db,
            ] {
                smoothed.set_time(5.0, control_rate);
            }
            for gain in &mut chain.gains {
                gain.set_time(5.0, self.sample_rate);
            }
        }
        self.dirty = ALL;
        self.reset();
    }

    fn set_parameter(&mut self, id: u32, normalized: f32) {
        let Some(index) = index(id) else {
            return;
        };
        let normalized = if normalized.is_finite() {
            normalized.clamp(0.0, 1.0)
        } else {
            PARAMETERS[index].default_normalized()
        };
        if self.values[index] != normalized {
            self.values[index] = normalized;
            self.dirty |= pad_of(id).map_or(KIT, |(pad, _)| 1 << pad);
        }
    }

    fn set_sample(&mut self, slot: u32, sample: Option<Arc<SampleData>>) {
        let pad = slot as usize;
        if pad < PADS {
            self.samples[pad] = sample.filter(|sample| sample.frames() > 0);
            for voice in self.pad_voices_mut(pad) {
                voice.active = false;
                voice.amp.reset();
            }
        }
    }

    fn process(&mut self, _context: &ProcessContext, events: &[Event], io: StereoIo<'_>) {
        if self.dirty != 0 {
            self.update_settings();
        }
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        left.fill(0.0);
        right.fill(0.0);
        let mut event = 0;
        let mut frame = 0;
        while frame < frames {
            if self.countdown == 0 {
                self.control();
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
            self.render(
                &mut left[frame..frame + span],
                &mut right[frame..frame + span],
            );
            self.countdown -= span;
            frame += span;
        }
        for (left, right) in left.iter_mut().zip(right.iter_mut()) {
            let gain = self.volume.next_value();
            *left *= gain;
            *right *= gain;
        }
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        self.dirty = ALL;
        self.update_settings();
        let rate = self.sample_rate;
        for voice in &mut self.voices {
            *voice = Voice::default();
        }
        for (pad, chain) in self.chains.iter_mut().enumerate() {
            let p = &self.settings.pads[pad];
            for smoothed in [
                &mut chain.position,
                &mut chain.resonance,
                &mut chain.drive_db,
            ] {
                smoothed.snap();
            }
            chain.filter.snap(sweep_mix(p.filter, p.resonance, rate));
            chain.filters = Default::default();
            chain.filtering = !chain.filter.mix().is_identity();
            for stage in &mut chain.drive {
                stage.reset();
            }
            chain.drive_gains = DriveGains::new(p.drive_db);
            chain.drive_target = chain.drive_gains;
            chain.drive_step = DriveGains::default();
            chain.driving = !chain.drive_gains.is_bypass();
            for gain in &mut chain.gains {
                gain.snap();
            }
            chain.tail = 0;
        }
        self.volume.snap();
        self.countdown = 0;
        self.age = 0;
        for pad in 0..PADS {
            self.telemetry.set_value(TELEMETRY_ACTIVITY + pad, 0.0);
            self.telemetry.set_value(TELEMETRY_PLAYHEADS + pad, -1.0);
        }
        self.telemetry.set_value(TELEMETRY_VOICES, 0.0);
    }

    fn latency_samples(&self) -> u32 {
        0
    }

    fn tail_samples(&self) -> u32 {
        // The longest hit: its recording at its lowest pitch, or its hold
        // and decay, whichever ends first, and a gate's release.
        let longest = (0..PADS)
            .filter_map(|pad| {
                let sample = self.samples[pad].as_ref()?;
                let p = &self.settings.pads[pad];
                let lowest = p.pitch.min(p.pitch + p.pitch_env);
                let played = (p.end - p.start).abs() * sample.frames() as f32
                    / sample.sample_rate()
                    / (lowest / 12.0).exp2();
                let shaped = p
                    .hold_ms
                    .map(|hold| (p.attack_ms + hold + 2.0 * p.decay_ms) * 0.001);
                Some(shaped.map_or(played, |shaped| shaped.min(played)))
            })
            .fold(0.0_f32, f32::max);
        let ring = if self.settings.pads.iter().any(|p| p.filter != 0.0) {
            RING_MS
        } else {
            TAIL_MS
        };
        ((longest + ring * 0.001 + 0.05) * self.sample_rate) as u32
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}
