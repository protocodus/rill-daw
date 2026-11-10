//! R1 Filter: one filter with nine models, an LFO and an envelope follower on
//! the cutoff. See `docs/R1_DEVICES.md#r1-filter`.
use crate::dsp::{
    Complex, DelayLine, EnvelopeFollower, Ladder, Lfo, LfoShape, Oversampler2x, SallenKey,
    Smoothed, Svf, db_to_gain, prewarp,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, ParamRange, ParamSpec, ProcessContext, StereoIo,
    Telemetry,
};
use std::sync::Arc;

pub const MODEL: u32 = 1;
pub const TYPE: u32 = 2;
pub const SLOPE: u32 = 3;
pub const CUTOFF: u32 = 4;
pub const RESONANCE: u32 = 5;
pub const DRIVE: u32 = 6;
pub const MORPH: u32 = 7;
pub const LFO_RATE: u32 = 10;
pub const LFO_SYNC: u32 = 11;
pub const LFO_SHAPE: u32 = 12;
pub const LFO_DEPTH: u32 = 13;
pub const LFO_STEREO: u32 = 14;
pub const ENV_AMOUNT: u32 = 20;
pub const ENV_ATTACK: u32 = 21;
pub const ENV_RELEASE: u32 = 22;
pub const MIX: u32 = 30;
pub const OUTPUT: u32 = 31;

/// Telemetry value slots.
pub const TELEMETRY_CUTOFF_LEFT: usize = 0;
pub const TELEMETRY_CUTOFF_RIGHT: usize = 1;
pub const TELEMETRY_LFO_PHASE: usize = 2;

/// Circuit-modelled families carry a letter instead of a manufacturer's name.
pub const MODELS: &[&str] = &[
    "Clean",
    "M Ladder",
    "K Sallen-Key",
    "X Pole-Mix",
    "V Digital",
    "SP Diode",
    "State Variable",
    "Comb",
    "Vowel",
];
pub const TYPES: &[&str] = &["Low-pass", "High-pass", "Band-pass", "Notch"];
pub const SLOPES: &[&str] = &["12 dB", "24 dB"];
pub const SYNC: &[&str] = &[
    "Off", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16",
];
const SYNC_BEATS: [f64; 8] = [0.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25];

const fn param(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    unit: &'static str,
    range: ParamRange,
    default: f32,
) -> ParamSpec {
    ParamSpec {
        id,
        key,
        name,
        group,
        unit,
        range,
        default,
        modulatable: !matches!(range, ParamRange::Choice(_) | ParamRange::Toggle),
    }
}

pub static PARAMETERS: [ParamSpec; 17] = [
    param(
        MODEL,
        "model",
        "Model",
        "Filter",
        "",
        ParamRange::Choice(MODELS),
        0.0,
    ),
    param(
        TYPE,
        "type",
        "Type",
        "Filter",
        "",
        ParamRange::Choice(TYPES),
        0.0,
    ),
    param(
        SLOPE,
        "slope",
        "Slope",
        "Filter",
        "",
        ParamRange::Choice(SLOPES),
        0.0,
    ),
    param(
        CUTOFF,
        "cutoff",
        "Cutoff",
        "Filter",
        "Hz",
        ParamRange::Log {
            min: 20.0,
            max: 20_000.0,
        },
        1000.0,
    ),
    param(
        RESONANCE,
        "resonance",
        "Resonance",
        "Filter",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        20.0,
    ),
    param(
        DRIVE,
        "drive",
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
        MORPH,
        "morph",
        "Morph",
        "Filter",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        0.0,
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
        1.0,
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
        LFO_DEPTH,
        "lfo.depth",
        "Depth",
        "LFO",
        "oct",
        ParamRange::Linear {
            min: -5.0,
            max: 5.0,
        },
        0.0,
    ),
    param(
        LFO_STEREO,
        "lfo.stereo",
        "Stereo",
        "LFO",
        "°",
        ParamRange::Linear {
            min: 0.0,
            max: 180.0,
        },
        0.0,
    ),
    param(
        ENV_AMOUNT,
        "env.amount",
        "Amount",
        "Envelope",
        "oct",
        ParamRange::Linear {
            min: -5.0,
            max: 5.0,
        },
        0.0,
    ),
    param(
        ENV_ATTACK,
        "env.attack",
        "Attack",
        "Envelope",
        "ms",
        ParamRange::Log {
            min: 0.1,
            max: 100.0,
        },
        5.0,
    ),
    param(
        ENV_RELEASE,
        "env.release",
        "Release",
        "Envelope",
        "ms",
        ParamRange::Log {
            min: 10.0,
            max: 2000.0,
        },
        150.0,
    ),
    param(
        MIX,
        "mix",
        "Mix",
        "Output",
        "%",
        ParamRange::Linear {
            min: 0.0,
            max: 100.0,
        },
        100.0,
    ),
    param(
        OUTPUT,
        "output",
        "Output",
        "Output",
        "dB",
        ParamRange::Linear {
            min: -24.0,
            max: 24.0,
        },
        0.0,
    ),
];

pub static DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    id: "r1-filter",
    name: "R1 Filter",
    kind: DeviceKind::Effect,
    category: "Filter",
    parameters: &PARAMETERS,
    sample_slots: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-filter/default.rillpreset"),
        include_str!("../../presets/r1-filter/acid-ladder.rillpreset"),
        include_str!("../../presets/r1-filter/auto-wah.rillpreset"),
        include_str!("../../presets/r1-filter/slow-sweep.rillpreset"),
        include_str!("../../presets/r1-filter/talking-vowels.rillpreset"),
        include_str!("../../presets/r1-filter/metal-comb.rillpreset"),
        include_str!("../../presets/r1-filter/screaming-high-pass.rillpreset"),
    ],
    create: || Box::new(Filter::new()),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Clean,
    /// M: transistor ladder.
    Ladder,
    /// K: Sallen-Key with clipped positive feedback.
    SallenKey,
    /// X: OTA four-pole cascade, multimode by mixing the pole outputs.
    PoleMix,
    /// V: digital multimode, resonance on every stage, saturation between.
    Digital,
    /// SP: diode-clipped two-pole multimode.
    Diode,
    StateVariable,
    Comb,
    Vowel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterType {
    LowPass,
    HighPass,
    BandPass,
    Notch,
}

/// Plain parameter values, decoded once per block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub model: Model,
    pub filter_type: FilterType,
    pub steep: bool,
    pub cutoff: f32,
    /// 0–1.
    pub resonance: f32,
    pub drive_db: f32,
    /// 0–1.
    pub morph: f32,
    pub lfo_rate: f32,
    /// Beats per LFO cycle, or `None` when free-running.
    pub lfo_sync_beats: Option<f64>,
    pub lfo_shape: LfoShape,
    pub lfo_depth: f32,
    pub lfo_stereo: f32,
    pub env_amount: f32,
    pub env_attack: f32,
    pub env_release: f32,
    /// 0–1.
    pub mix: f32,
    pub output_db: f32,
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("filter parameter");
            spec.to_plain(value(id))
        };
        let index = |id: u32| plain(id) as usize;
        let model = match index(MODEL) {
            1 => Model::Ladder,
            2 => Model::SallenKey,
            3 => Model::PoleMix,
            4 => Model::Digital,
            5 => Model::Diode,
            6 => Model::StateVariable,
            7 => Model::Comb,
            8 => Model::Vowel,
            _ => Model::Clean,
        };
        // A type the model lacks, left over from another model, falls back
        // to the model's last type.
        let offered = type_options(model).len();
        let filter_type = match index(TYPE).min(offered.saturating_sub(1)) {
            1 => FilterType::HighPass,
            2 => FilterType::BandPass,
            3 => FilterType::Notch,
            _ => FilterType::LowPass,
        };
        let sync = SYNC_BEATS[index(LFO_SYNC).min(SYNC_BEATS.len() - 1)];
        Self {
            model,
            filter_type,
            steep: index(SLOPE) == 1,
            cutoff: plain(CUTOFF),
            resonance: plain(RESONANCE) / 100.0,
            drive_db: plain(DRIVE),
            morph: plain(MORPH) / 100.0,
            lfo_rate: plain(LFO_RATE),
            lfo_sync_beats: (sync > 0.0).then_some(sync),
            lfo_shape: LfoShape::from_index(index(LFO_SHAPE)),
            lfo_depth: plain(LFO_DEPTH),
            lfo_stereo: plain(LFO_STEREO),
            env_amount: plain(ENV_AMOUNT),
            env_attack: plain(ENV_ATTACK),
            env_release: plain(ENV_RELEASE),
            mix: plain(MIX) / 100.0,
            output_db: plain(OUTPUT),
        }
    }
}

/// Whether a parameter does anything in the current model; panels hide the
/// ones that do not.
pub fn applies(id: u32, settings: &Settings) -> bool {
    match id {
        TYPE => !type_options(settings.model).is_empty(),
        SLOPE => matches!(
            settings.model,
            Model::Clean | Model::PoleMix | Model::Digital
        ),
        MORPH => matches!(
            settings.model,
            Model::StateVariable | Model::Comb | Model::Vowel
        ),
        LFO_RATE => settings.lfo_sync_beats.is_none(),
        _ => true,
    }
}

/// Type options that a model offers, for the panel's selector.
pub fn type_options(model: Model) -> &'static [&'static str] {
    match model {
        Model::Clean | Model::PoleMix | Model::Digital => TYPES,
        Model::SallenKey => &TYPES[..2],
        Model::Diode => &TYPES[..3],
        _ => &[],
    }
}

fn clean_q(resonance: f32) -> f32 {
    0.707 * 35.0_f32.powf(resonance)
}

/// Butterworth stage Qs for the 24 dB low- and high-pass, with resonance on
/// the second stage.
fn steep_qs(resonance: f32) -> [f32; 2] {
    [0.541, 1.307 * 19.0_f32.powf(resonance)]
}

fn ladder_k(resonance: f32) -> f32 {
    4.1 * resonance
}

fn sallen_key_k(resonance: f32) -> f32 {
    2.05 * resonance
}

/// Weights of the loop input and the four stage outputs. Each stage output
/// is the loop input over (1 + s)^n, so binomial weights build any 12 or
/// 24 dB response over the same resonant loop.
fn pole_mix(filter_type: FilterType, steep: bool) -> [f32; 5] {
    match (filter_type, steep) {
        (FilterType::LowPass, false) => [0.0, 0.0, 1.0, 0.0, 0.0],
        (FilterType::LowPass, true) => [0.0, 0.0, 0.0, 0.0, 1.0],
        (FilterType::HighPass, false) => [1.0, -2.0, 1.0, 0.0, 0.0],
        (FilterType::HighPass, true) => [1.0, -4.0, 6.0, -4.0, 1.0],
        (FilterType::BandPass, false) => [0.0, 2.0, -2.0, 0.0, 0.0],
        (FilterType::BandPass, true) => [0.0, 0.0, 4.0, -8.0, 4.0],
        (FilterType::Notch, false) => [1.0, -2.0, 2.0, 0.0, 0.0],
        (FilterType::Notch, true) => [1.0, -4.0, 8.0, -8.0, 4.0],
    }
}

/// The low-pass loses bass as resonance rises; this restores half of it in
/// dB, as the M ladder does.
fn pole_mix_gain(filter_type: FilterType, k: f32) -> f32 {
    if filter_type == FilterType::LowPass {
        (1.0 + k).sqrt()
    } else {
        1.0
    }
}

fn digital_q(resonance: f32) -> f32 {
    0.707 * 20.0_f32.powf(resonance)
}

/// SVF damping (1/Q): two real poles at 0, slightly negative at 100 % so the
/// clipped loop self-oscillates.
fn diode_damping(resonance: f32) -> f32 {
    2.0 - 2.05 * resonance
}

/// Band-pass gain that keeps the peak from vanishing as damping reaches 0.
fn diode_band_gain(damping: f32) -> f32 {
    damping.max(0.25)
}

fn comb_feedback(resonance: f32) -> f32 {
    0.97 * resonance
}

/// In-loop low-pass coefficient: Morph darkens the comb.
fn comb_damping(morph: f32) -> f32 {
    1.0 - 0.9 * morph
}

fn vowel_k(resonance: f32) -> f32 {
    1.0 / (4.0 + 16.0 * resonance)
}

const VOWELS: [([f32; 3], [f32; 3]); 5] = [
    ([800.0, 1150.0, 2900.0], [0.0, -6.0, -32.0]),
    ([350.0, 2000.0, 2800.0], [0.0, -20.0, -15.0]),
    ([270.0, 2140.0, 2950.0], [0.0, -12.0, -26.0]),
    ([450.0, 800.0, 2830.0], [0.0, -11.0, -22.0]),
    ([325.0, 700.0, 2530.0], [0.0, -16.0, -35.0]),
];

/// Formant frequencies and linear gains for Morph 0–1 (A-E-I-O-U), shifted by
/// the cutoff relative to 1 kHz.
fn formants(morph: f32, cutoff: f32) -> [(f32, f32); 3] {
    let position = morph.clamp(0.0, 1.0) * (VOWELS.len() - 1) as f32;
    let index = (position as usize).min(VOWELS.len() - 2);
    let t = position - index as f32;
    let (from, to) = (VOWELS[index], VOWELS[index + 1]);
    let shift = cutoff / 1000.0;
    std::array::from_fn(|formant| {
        let frequency = from.0[formant].powf(1.0 - t) * to.0[formant].powf(t) * shift;
        let gain = from.1[formant] * (1.0 - t) + to.1[formant] * t;
        (frequency, db_to_gain(gain))
    })
}

/// Pre-filter saturation of the linear models: identity at 0 dB, blending
/// into a level-compensated `tanh` as Drive rises.
#[inline]
fn soft_drive(input: f32, drive_db: f32) -> f32 {
    if drive_db <= 0.0 {
        return input;
    }
    let gain = db_to_gain(drive_db);
    let amount = drive_db / 24.0;
    input + amount * ((gain * input).tanh() / gain.sqrt() - input)
}

/// Small-signal gain of the drive stage, for the drawn curve.
fn drive_gain(model: Model, drive_db: f32) -> f32 {
    let gain = db_to_gain(drive_db);
    match model {
        Model::Ladder | Model::SallenKey | Model::PoleMix | Model::Diode => gain.sqrt(),
        _ => 1.0 + drive_db / 24.0 * (gain.sqrt() - 1.0),
    }
}

/// Delays a signal by the oversampler latency so every model, and the dry
/// path, line up.
#[derive(Clone, Debug)]
struct LatencyDelay {
    buffer: [f32; Oversampler2x::LATENCY as usize],
    cursor: usize,
}

impl Default for LatencyDelay {
    fn default() -> Self {
        Self {
            buffer: [0.0; Oversampler2x::LATENCY as usize],
            cursor: 0,
        }
    }
}

impl LatencyDelay {
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = std::mem::replace(&mut self.buffer[self.cursor], input);
        self.cursor = (self.cursor + 1) % self.buffer.len();
        output
    }
}

#[derive(Clone, Debug, Default)]
struct Channel {
    svf: [Svf; 3],
    ladder: Ladder,
    sallen_key: SallenKey,
    comb: DelayLine,
    comb_damping: f32,
    oversampler: Oversampler2x,
    wet_delay: LatencyDelay,
    dry_delay: LatencyDelay,
}

impl Channel {
    fn reset_filters(&mut self) {
        for svf in &mut self.svf {
            svf.reset();
        }
        self.ladder.reset();
        self.sallen_key.reset();
        self.comb.reset();
        self.comb_damping = 0.0;
        self.oversampler.reset();
    }

    fn reset(&mut self) {
        self.reset_filters();
        self.wet_delay = LatencyDelay::default();
        self.dry_delay = LatencyDelay::default();
    }

    #[inline]
    fn process(&mut self, input: f32, s: &Settings, state: &Frame, sample_rate: f32) -> f32 {
        let cutoff = state.cutoff;
        let wet = match s.model {
            Model::Ladder | Model::SallenKey | Model::PoleMix | Model::Diode => {
                let gain = db_to_gain(s.drive_db);
                let g = prewarp(cutoff, 2.0 * sample_rate);
                let upsampled = self.oversampler.upsample(input * gain);
                let filtered = upsampled.map(|x| match (s.model, s.filter_type) {
                    (Model::Ladder, _) => {
                        let k = ladder_k(state.resonance);
                        self.ladder.process(x, g, k) * (1.0 + k).sqrt()
                    }
                    (Model::PoleMix, _) => {
                        let k = ladder_k(state.resonance);
                        let taps = self.ladder.process_taps(x, g, k, f32::tanh);
                        let mixed: f32 = pole_mix(s.filter_type, s.steep)
                            .iter()
                            .zip(taps)
                            .map(|(weight, tap)| weight * tap)
                            .sum();
                        mixed * pole_mix_gain(s.filter_type, k)
                    }
                    (Model::Diode, _) => {
                        let k = diode_damping(state.resonance);
                        let out = self.svf[0].process_clipped(x, g, k);
                        pick(out, s.filter_type, diode_band_gain(k))
                    }
                    (_, FilterType::HighPass) => {
                        self.sallen_key
                            .high_pass(x, g, sallen_key_k(state.resonance))
                    }
                    _ => self
                        .sallen_key
                        .low_pass(x, g, sallen_key_k(state.resonance)),
                });
                return self.oversampler.downsample(filtered) / gain.sqrt();
            }
            Model::Clean => {
                let x = soft_drive(input, s.drive_db);
                let g = prewarp(cutoff, sample_rate);
                if s.steep && matches!(s.filter_type, FilterType::LowPass | FilterType::HighPass) {
                    let [q1, q2] = steep_qs(state.resonance);
                    let first = self.svf[0].process(x, g, 1.0 / q1);
                    let first = pick(first, s.filter_type, 1.0 / q1);
                    let second = self.svf[1].process(first, g, 1.0 / q2);
                    pick(second, s.filter_type, 1.0 / q2)
                } else {
                    let k = 1.0 / clean_q(state.resonance);
                    let first = self.svf[0].process(x, g, k);
                    let first = pick(first, s.filter_type, k);
                    if s.steep {
                        let second = self.svf[1].process(first, g, k);
                        pick(second, s.filter_type, k)
                    } else {
                        first
                    }
                }
            }
            Model::Digital => {
                let x = soft_drive(input, s.drive_db);
                let g = prewarp(cutoff, sample_rate);
                let k = 1.0 / digital_q(state.resonance);
                let first = pick(self.svf[0].process(x, g, k), s.filter_type, k);
                if s.steep {
                    let second = self.svf[1].process(first.tanh(), g, k);
                    pick(second, s.filter_type, k)
                } else {
                    first
                }
            }
            Model::StateVariable => {
                let x = soft_drive(input, s.drive_db);
                let k = 1.0 / clean_q(state.resonance);
                let out = self.svf[0].process(x, prewarp(cutoff, sample_rate), k);
                let morph = state.morph;
                if morph < 0.5 {
                    out.low * (1.0 - 2.0 * morph) + out.notch() * 2.0 * morph
                } else {
                    out.notch() * (2.0 - 2.0 * morph) + out.high * (2.0 * morph - 1.0)
                }
            }
            Model::Comb => {
                let x = soft_drive(input, s.drive_db);
                let feedback = comb_feedback(state.resonance);
                let delayed = self.comb.read(sample_rate / cutoff);
                self.comb_damping += comb_damping(state.morph) * (delayed - self.comb_damping);
                let y = x + feedback * self.comb_damping;
                self.comb.write(y);
                y * (1.0 - feedback * feedback).sqrt()
            }
            Model::Vowel => {
                let x = soft_drive(input, s.drive_db);
                let k = vowel_k(state.resonance);
                let mut sum = 0.0;
                for (svf, (frequency, gain)) in
                    self.svf.iter_mut().zip(formants(state.morph, cutoff))
                {
                    sum += gain * k * svf.process(x, prewarp(frequency, sample_rate), k).band;
                }
                sum
            }
        };
        self.wet_delay.process(wet)
    }
}

/// `band_gain` is normally the damping k, for a constant-peak band-pass, so
/// resonance narrows instead of boosting.
#[inline]
fn pick(out: crate::dsp::SvfOutputs, filter_type: FilterType, band_gain: f32) -> f32 {
    match filter_type {
        FilterType::LowPass => out.low,
        FilterType::HighPass => out.high,
        FilterType::BandPass => out.band * band_gain,
        FilterType::Notch => out.notch(),
    }
}

/// Smoothed values for one sample.
struct Frame {
    cutoff: f32,
    resonance: f32,
    morph: f32,
}

pub struct Filter {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    dirty: bool,
    sample_rate: f32,
    channels: [Channel; 2],
    /// log2 of the cutoff, so sweeps move evenly in pitch.
    cutoff: Smoothed,
    resonance: Smoothed,
    morph: Smoothed,
    mix: Smoothed,
    output: Smoothed,
    lfo_depth: Smoothed,
    env_amount: Smoothed,
    lfo: Lfo,
    follower: EnvelopeFollower,
    telemetry: Arc<Telemetry>,
}

impl Default for Filter {
    fn default() -> Self {
        Self::new()
    }
}

impl Filter {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let settings = Settings::from_normalized(|id| {
            values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
        });
        let mut filter = Self {
            values,
            settings,
            dirty: false,
            sample_rate: 48_000.0,
            channels: Default::default(),
            cutoff: Smoothed::new(settings.cutoff.log2()),
            resonance: Smoothed::new(settings.resonance),
            morph: Smoothed::new(settings.morph),
            mix: Smoothed::new(settings.mix),
            output: Smoothed::new(db_to_gain(settings.output_db)),
            lfo_depth: Smoothed::new(settings.lfo_depth),
            env_amount: Smoothed::new(settings.env_amount),
            lfo: Lfo::new(0x005E_EDF1),
            follower: EnvelopeFollower::default(),
            telemetry: Arc::new(Telemetry::default()),
        };
        filter.prepare(48_000.0, 0);
        filter
    }

    fn update_settings(&mut self) {
        let values = self.values;
        let settings = Settings::from_normalized(|id| {
            values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
        });
        if settings.model != self.settings.model {
            for channel in &mut self.channels {
                channel.reset_filters();
            }
        }
        self.settings = settings;
        self.cutoff.set_target(settings.cutoff.log2());
        self.resonance.set_target(settings.resonance);
        self.morph.set_target(settings.morph);
        self.mix.set_target(settings.mix);
        self.output.set_target(db_to_gain(settings.output_db));
        self.lfo_depth.set_target(settings.lfo_depth);
        self.env_amount.set_target(settings.env_amount);
        self.follower
            .set_times(settings.env_attack, settings.env_release, self.sample_rate);
        self.dirty = false;
    }

    fn smoothers(&mut self) -> [&mut Smoothed; 7] {
        [
            &mut self.lfo_depth,
            &mut self.env_amount,
            &mut self.cutoff,
            &mut self.resonance,
            &mut self.morph,
            &mut self.mix,
            &mut self.output,
        ]
    }
}

impl Device for Filter {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        let rate = self.sample_rate;
        for smoothed in self.smoothers() {
            smoothed.set_time(5.0, rate);
        }
        for channel in &mut self.channels {
            channel.comb.allocate((rate / 20.0).ceil() as usize + 4);
        }
        self.update_settings();
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

    fn process(&mut self, context: &ProcessContext, _events: &[Event], io: StereoIo<'_>) {
        if self.dirty {
            self.update_settings();
        }
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        self.telemetry.record_input(left, right);
        let s = self.settings;
        let rate = self.sample_rate;
        let stereo = f64::from(s.lfo_stereo) / 360.0;
        let synced = s.lfo_sync_beats.filter(|_| context.playing);
        let beats_per_sample = context.tempo / 60.0 / f64::from(rate);
        let mut cutoffs = [s.cutoff; 2];
        for index in 0..frames {
            match synced {
                Some(beats) => self
                    .lfo
                    .set_phase((context.beat + index as f64 * beats_per_sample) / beats),
                None => {
                    let rate_hz = s
                        .lfo_sync_beats
                        .map_or(f64::from(s.lfo_rate), |beats| context.tempo / 60.0 / beats);
                    self.lfo.advance(rate_hz / f64::from(rate));
                }
            }
            let envelope = self
                .follower
                .process(left[index].abs().max(right[index].abs()));
            let base = self.cutoff.next_value();
            let resonance = self.resonance.next_value();
            let morph = self.morph.next_value();
            let mix = self.mix.next_value();
            let output = self.output.next_value();
            let lfo_depth = self.lfo_depth.next_value();
            let env_amount = self.env_amount.next_value();
            for (channel, sample) in [&mut *left, &mut *right].into_iter().enumerate() {
                let lfo = self.lfo.value(s.lfo_shape, stereo * channel as f64);
                let octaves = base + lfo_depth * lfo + env_amount * envelope;
                let cutoff = octaves.exp2().clamp(20.0, 20_000.0);
                cutoffs[channel] = cutoff;
                let frame = Frame {
                    cutoff,
                    resonance,
                    morph,
                };
                let input = sample[index];
                let state = &mut self.channels[channel];
                let wet = state.process(input, &s, &frame, rate);
                let dry = state.dry_delay.process(input);
                sample[index] = (dry + mix * (wet - dry)) * output;
            }
        }
        self.telemetry.set_value(TELEMETRY_CUTOFF_LEFT, cutoffs[0]);
        self.telemetry.set_value(TELEMETRY_CUTOFF_RIGHT, cutoffs[1]);
        self.telemetry
            .set_value(TELEMETRY_LFO_PHASE, self.lfo.phase() as f32);
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        if self.dirty {
            self.update_settings();
        }
        for channel in &mut self.channels {
            channel.reset();
        }
        for smoothed in self.smoothers() {
            smoothed.snap();
        }
        self.lfo.reset();
        self.follower.reset();
    }

    fn latency_samples(&self) -> u32 {
        Oversampler2x::LATENCY
    }

    fn tail_samples(&self) -> u32 {
        // Long enough for a resonant comb at 20 Hz to ring down.
        (self.sample_rate * 2.0) as u32
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}

/// Small-signal magnitude response in dB at `frequencies`, for drawing. Uses
/// the same mappings as the audio path; `cutoff` is the effective cutoff
/// (pass `settings.cutoff`, or the live value from telemetry).
pub fn response_db(
    settings: &Settings,
    cutoff: f32,
    sample_rate: f32,
    frequencies: &[f32],
    out: &mut [f32],
) {
    let s = settings;
    let wet_gain = f64::from(drive_gain(s.model, s.drive_db));
    let output = f64::from(db_to_gain(s.output_db));
    for (frequency, out) in frequencies.iter().zip(out) {
        let frequency = frequency.clamp(1.0, sample_rate * 0.499);
        let analog = |rate: f32, corner: f32| {
            Complex::new(
                0.0,
                f64::from(prewarp(frequency, rate)) / f64::from(prewarp(corner, rate)),
            )
        };
        let wet = match s.model {
            Model::Clean => {
                let x = analog(sample_rate, cutoff);
                let stage = |q: f32| svf_response(x, s.filter_type, 1.0 / q);
                if s.steep && matches!(s.filter_type, FilterType::LowPass | FilterType::HighPass) {
                    let [q1, q2] = steep_qs(s.resonance);
                    stage(q1) * stage(q2)
                } else if s.steep {
                    let q = clean_q(s.resonance);
                    stage(q) * stage(q)
                } else {
                    stage(clean_q(s.resonance))
                }
            }
            Model::Ladder => {
                let x = analog(2.0 * sample_rate, cutoff);
                let k = f64::from(ladder_k(s.resonance));
                let one = Complex::ONE + x;
                Complex::real((1.0 + k).sqrt()) / (one * one * one * one + Complex::real(k))
            }
            Model::SallenKey => {
                let x = analog(2.0 * sample_rate, cutoff);
                let k = f64::from(sallen_key_k(s.resonance));
                let one = Complex::ONE + x;
                let denominator = one * one - x.scale(k);
                let numerator = if s.filter_type == FilterType::HighPass {
                    x * x
                } else {
                    Complex::ONE
                };
                numerator / denominator
            }
            Model::PoleMix => {
                let x = analog(2.0 * sample_rate, cutoff);
                let k = ladder_k(s.resonance);
                let t = Complex::ONE + x;
                let powers = [t * t * t * t, t * t * t, t * t, t, Complex::ONE];
                let numerator = pole_mix(s.filter_type, s.steep)
                    .iter()
                    .zip(powers)
                    .fold(Complex::real(0.0), |sum, (weight, power)| {
                        sum + power.scale(f64::from(*weight))
                    });
                (numerator / (powers[0] + Complex::real(f64::from(k))))
                    .scale(f64::from(pole_mix_gain(s.filter_type, k)))
            }
            Model::Digital => {
                let x = analog(sample_rate, cutoff);
                let stage = svf_response(x, s.filter_type, 1.0 / digital_q(s.resonance));
                if s.steep { stage * stage } else { stage }
            }
            Model::Diode => {
                let x = analog(2.0 * sample_rate, cutoff);
                let k = diode_damping(s.resonance);
                if s.filter_type == FilterType::BandPass {
                    // svf_response scales the band by k; use the clamped gain.
                    (x / (x * x + x.scale(f64::from(k)) + Complex::ONE))
                        .scale(f64::from(diode_band_gain(k)))
                } else {
                    svf_response(x, s.filter_type, k)
                }
            }
            Model::StateVariable => {
                let x = analog(sample_rate, cutoff);
                let k = 1.0 / clean_q(s.resonance);
                let low = svf_response(x, FilterType::LowPass, k);
                let notch = svf_response(x, FilterType::Notch, k);
                let high = svf_response(x, FilterType::HighPass, k);
                let morph = f64::from(s.morph);
                if morph < 0.5 {
                    low.scale(1.0 - 2.0 * morph) + notch.scale(2.0 * morph)
                } else {
                    notch.scale(2.0 - 2.0 * morph) + high.scale(2.0 * morph - 1.0)
                }
            }
            Model::Comb => {
                let omega = std::f64::consts::TAU * f64::from(frequency / sample_rate);
                let feedback = f64::from(comb_feedback(s.resonance));
                let a = f64::from(comb_damping(s.morph));
                let delay = f64::from(sample_rate / cutoff);
                let damping =
                    Complex::real(a) / (Complex::ONE - Complex::unit(-omega).scale(1.0 - a));
                let loop_gain = damping * Complex::unit(-omega * delay);
                Complex::real((1.0 - feedback * feedback).sqrt())
                    / (Complex::ONE - loop_gain.scale(feedback))
            }
            Model::Vowel => {
                let k = vowel_k(s.resonance);
                formants(s.morph, cutoff).into_iter().fold(
                    Complex::real(0.0),
                    |sum, (corner, gain)| {
                        let x = analog(sample_rate, corner);
                        sum + svf_response(x, FilterType::BandPass, k).scale(f64::from(gain))
                    },
                )
            }
        };
        let mix = f64::from(s.mix);
        let total = (Complex::real(1.0 - mix) + wet.scale(mix * wet_gain)).scale(output);
        *out = (20.0 * total.norm().max(1e-9).log10()) as f32;
    }
}

/// Analog prototype of [`Svf`] at normalized `s`; band-pass is constant-peak.
fn svf_response(s: Complex, filter_type: FilterType, k: f32) -> Complex {
    let k = f64::from(k);
    let denominator = s * s + s.scale(k) + Complex::ONE;
    let numerator = match filter_type {
        FilterType::LowPass => Complex::ONE,
        FilterType::HighPass => s * s,
        FilterType::BandPass => s.scale(k),
        FilterType::Notch => s * s + Complex::ONE,
    };
    numerator / denominator
}
