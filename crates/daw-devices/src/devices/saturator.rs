//! R1 Saturator: three circuit-style curves inside an oversampled,
//! antialiased shaper, with a clean low band, static level compensation and
//! the same latency at every quality. See `docs/R1_DEVICES.md#r1-saturator`.
use super::param;
use crate::dsp::{
    AlgebraicSigmoid, AntialiasedShaper, Ballistics, CurveKind, DroopCorrector, GainComputer,
    HyperbolicClip, LinearRamp, OversampleFactor, Oversampler, Shape, Shelf1, Smoothed,
    StaticDelay, Svf, db_to_gain, flush, prewarp,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, ParamRange, ParamSpec, ProcessContext, StereoIo,
    Telemetry,
};
use std::sync::Arc;

pub const MODEL: u32 = 1;
pub const DRIVE: u32 = 2;
pub const CHARACTER: u32 = 3;
pub const TONE: u32 = 4;
pub const FOCUS: u32 = 5;
pub const QUALITY: u32 = 10;
pub const AUTO_GAIN: u32 = 20;
pub const MIX: u32 = 30;
pub const OUTPUT: u32 = 31;

/// Telemetry value slots. The level is the peak going into the shaper, in
/// its units (1.0 is the curve's knee), held with a 300 ms fall.
pub const TELEMETRY_LEVEL: usize = 0;
pub const TELEMETRY_SAMPLE_RATE: usize = 1;

/// Named after the circuit technology, never a manufacturer.
pub const MODELS: &[&str] = &["Transistor", "Tube", "Tape"];
pub const QUALITIES: &[&str] = &["2×", "4×", "8×"];

/// The same at every quality and sample rate: the shorter oversamplers are
/// padded, so changing Quality moves nothing else in the song.
pub const LATENCY: u32 = Oversampler::latency(OversampleFactor::X8);

const fn linear(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    unit: &'static str,
    (min, max): (f32, f32),
    default: f32,
) -> ParamSpec {
    param(
        id,
        key,
        name,
        group,
        unit,
        ParamRange::Linear { min, max },
        default,
    )
}

pub static PARAMETERS: [ParamSpec; 9] = [
    param(
        MODEL,
        "model",
        "Model",
        "Saturation",
        "",
        ParamRange::Choice(MODELS),
        1.0,
    ),
    linear(
        DRIVE,
        "drive",
        "Drive",
        "Saturation",
        "dB",
        (0.0, 36.0),
        6.0,
    ),
    linear(
        CHARACTER,
        "character",
        "Character",
        "Saturation",
        "%",
        (0.0, 100.0),
        50.0,
    ),
    linear(TONE, "tone", "Tone", "Saturation", "dB", (-6.0, 6.0), 0.0),
    param(
        FOCUS,
        "focus",
        "Focus",
        "Saturation",
        "Hz",
        ParamRange::Log {
            min: 20.0,
            max: 1000.0,
        },
        20.0,
    ),
    param(
        QUALITY,
        "quality",
        "Quality",
        "Quality",
        "",
        ParamRange::Choice(QUALITIES),
        1.0,
    ),
    param(
        AUTO_GAIN,
        "auto_gain",
        "Auto gain",
        "Output",
        "",
        ParamRange::Toggle,
        1.0,
    ),
    linear(MIX, "mix", "Mix", "Output", "%", (0.0, 100.0), 100.0),
    linear(
        OUTPUT,
        "output",
        "Output",
        "Output",
        "dB",
        (-24.0, 24.0),
        0.0,
    ),
];

pub static DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    id: "r1-saturator",
    name: "R1 Saturator",
    kind: DeviceKind::Effect,
    category: "Distortion",
    parameters: &PARAMETERS,
    sample_slots: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-saturator/default.rillpreset"),
        include_str!("../../presets/r1-saturator/warm-tube.rillpreset"),
        include_str!("../../presets/r1-saturator/tube-crunch.rillpreset"),
        include_str!("../../presets/r1-saturator/bass-grit.rillpreset"),
        include_str!("../../presets/r1-saturator/tape-glue.rillpreset"),
        include_str!("../../presets/r1-saturator/tape-slam.rillpreset"),
        include_str!("../../presets/r1-saturator/hi-fi-sheen.rillpreset"),
        include_str!("../../presets/r1-saturator/transistor-edge.rillpreset"),
        include_str!("../../presets/r1-saturator/fuzz-box.rillpreset"),
        include_str!("../../presets/r1-saturator/parallel-dirt.rillpreset"),
    ],
    legacy_labels: &[],
    latency: |_, _| LATENCY,
    create: || Box::new(Saturator::new()),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Transistor,
    Tube,
    Tape,
}

/// Plain parameter values, decoded once per change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub model: Model,
    pub drive_db: f32,
    /// 0–1: Transistor hardness, Tube bias, Tape compression.
    pub character: f32,
    pub tone_db: f32,
    pub focus_hz: f32,
    /// Off at the bottom of the Focus range: everything is saturated.
    pub focus_on: bool,
    pub quality: OversampleFactor,
    pub auto_gain: bool,
    /// 0–1.
    pub mix: f32,
    pub output_db: f32,
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("saturator parameter");
            spec.to_plain(value(id))
        };
        Self {
            model: match plain(MODEL) as usize {
                0 => Model::Transistor,
                1 => Model::Tube,
                _ => Model::Tape,
            },
            drive_db: plain(DRIVE),
            character: plain(CHARACTER) / 100.0,
            tone_db: plain(TONE),
            focus_hz: plain(FOCUS),
            focus_on: value(FOCUS) > 0.0,
            quality: match plain(QUALITY) as usize {
                0 => OversampleFactor::X2,
                1 => OversampleFactor::X4,
                _ => OversampleFactor::X8,
            },
            auto_gain: plain(AUTO_GAIN) >= 0.5,
            mix: plain(MIX) / 100.0,
            output_db: plain(OUTPUT),
        }
    }
}

/// A model's static curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Clip(HyperbolicClip),
    Sigmoid(AlgebraicSigmoid),
}

impl Shape for Curve {
    #[inline]
    fn value(&self, x: f64) -> f64 {
        match self {
            Self::Clip(shape) => shape.value(x),
            Self::Sigmoid(shape) => shape.value(x),
        }
    }

    #[inline]
    fn antiderivative(&self, x: f64) -> f64 {
        match self {
            Self::Clip(shape) => shape.antiderivative(x),
            Self::Sigmoid(shape) => shape.antiderivative(x),
        }
    }
}

/// The curve of a model at a Character of 0–1: the one mapping, for the
/// audio path and for drawing. Every curve has unit slope at zero, so the
/// models are level matched for small signals.
pub fn curve(model: Model, character: f32) -> Curve {
    let character = f64::from(character.clamp(0.0, 1.0));
    match model {
        // An asymmetric clipper whose corner hardens with Character: odd
        // harmonics, some even ones from the asymmetry.
        Model::Transistor => Curve::Clip(HyperbolicClip::new(1.0, 1.25, 0.05_f64.powf(character))),
        // More headroom below than above, and a bias that moves the
        // operating point: the second harmonic grows with Character.
        Model::Tube => Curve::Sigmoid(AlgebraicSigmoid::new(1.0, 2.0, 0.8 * character)),
        // Symmetric and soft; Character is the program compression instead.
        Model::Tape => Curve::Sigmoid(AlgebraicSigmoid::new(1.0, 1.0, 0.0)),
    }
}

/// Zero and rise in dB of the emphasis before the shaper; its exact inverse
/// follows, so small signals stay flat while highs meet the curve harder.
fn emphasis(model: Model) -> (f32, f32) {
    match model {
        Model::Transistor => (720.0, 4.0),
        Model::Tube => (1500.0, 3.0),
        Model::Tape => (3183.0, 8.0),
    }
}

fn emphasis_pair(model: Model, sample_rate: f32) -> (Shelf1, Shelf1) {
    let (zero, rise_db) = emphasis(model);
    let pole = zero * db_to_gain(rise_db);
    (
        Shelf1::shelf(zero, pole, sample_rate),
        Shelf1::inverse_shelf(zero, pole, sample_rate),
    )
}

/// What the de-emphasis takes from a fully saturated signal, in dB: its
/// spectrum is flatter than the input's. Tuned per model against pink noise
/// (the auto gain test).
fn emphasis_loss_db(model: Model) -> f64 {
    match model {
        Model::Transistor => 3.8,
        Model::Tube => 1.9,
        Model::Tape => 4.2,
    }
}

/// Tape's program compression ahead of the shaper. In RMS terms the knee
/// starts at −14 dB, where a signal with 12 dB of crest begins to touch the
/// curve, so compression sets in together with saturation; the threshold is
/// higher by what the detector over-reads.
fn tape_curve(character: f32) -> GainComputer {
    GainComputer {
        kind: CurveKind::Compress,
        threshold_db: -9.0 + TAPE_DETECTOR_BIAS_DB,
        ratio: 1.0 + 2.0 * character,
        knee_db: 10.0,
    }
}

/// With its fast rise and slow fall, the power detector reads a noise-like
/// signal this far above its RMS level.
const TAPE_DETECTOR_BIAS_DB: f32 = 3.0;

/// The reference signal of the level compensation: Gaussian at −18 dBFS RMS.
const REFERENCE_RMS: f64 = 0.125_892_541_179_416_73;
const SIMPSON_POINTS: usize = 241;

/// Level gain of the saturated band for the reference signal: what the clean
/// low band is scaled by to keep the balance, and what Auto gain undoes.
/// Static by design; a follower would pump.
fn reference_gain(settings: &Settings) -> f64 {
    let drive = 10.0_f64.powf(f64::from(settings.drive_db) / 20.0);
    let compression = if settings.model == Model::Tape {
        let level_db = 20.0 * (drive * REFERENCE_RMS).log10() as f32 + TAPE_DETECTOR_BIAS_DB;
        let reduction = tape_curve(settings.character).reduction_db(level_db);
        10.0_f64.powf(-f64::from(reduction) / 20.0)
    } else {
        1.0
    };
    let shape = curve(settings.model, settings.character);
    let scale = drive * compression * REFERENCE_RMS;
    // Mean and mean square of the shaped signal: Simpson's rule over ±6
    // standard deviations.
    let step = 12.0 / (SIMPSON_POINTS - 1) as f64;
    let (mut mean, mut mean_square) = (0.0, 0.0);
    for index in 0..SIMPSON_POINTS {
        let z = -6.0 + step * index as f64;
        let weight = if index == 0 || index == SIMPSON_POINTS - 1 {
            1.0
        } else if index % 2 == 1 {
            4.0
        } else {
            2.0
        };
        let density = (-0.5 * z * z).exp() / std::f64::consts::TAU.sqrt();
        let y = shape.value(scale * z);
        mean += weight * density * y;
        mean_square += weight * density * y * y;
    }
    mean *= step / 3.0;
    mean_square *= step / 3.0;
    // The variance: the DC blocker removes the mean.
    let static_gain = (mean_square - mean * mean).max(0.0).sqrt() / REFERENCE_RMS;
    // 1 while the curve is linear, toward 0 as it saturates.
    let linearity = (static_gain / (drive * compression)).min(1.0);
    static_gain * 10.0_f64.powf(-emphasis_loss_db(settings.model) * (1.0 - linearity) / 20.0)
}

/// The compensation Auto gain applies, in dB (negative when the curve adds
/// level), whether or not it is switched on.
pub fn auto_gain_db(settings: &Settings) -> f32 {
    (-20.0 * reference_gain(settings).log10()) as f32
}

/// The static curve as drawn: output for each input of −1–1 at the set
/// Drive, with the Auto gain compensation when it is on. Emphasis, Tape's
/// compression and Tone are not part of it.
pub fn transfer(settings: &Settings, input: &[f32], out: &mut [f32]) {
    let drive = 10.0_f64.powf(f64::from(settings.drive_db) / 20.0);
    let shape = curve(settings.model, settings.character);
    let gain = if settings.auto_gain {
        1.0 / reference_gain(settings)
    } else {
        1.0
    };
    for (x, out) in input.iter().zip(out) {
        *out = (shape.value(drive * f64::from(*x)) * gain) as f32;
    }
}

/// Levels of harmonics 1–10 of a sine that reaches the shaper with a peak
/// of `amplitude`, in dB relative to the fundamental, floored at −100.
pub fn harmonics_db(settings: &Settings, amplitude: f32, out: &mut [f32; 10]) {
    // Enough points for the tenth harmonic of a hard-clipped wave.
    const POINTS: usize = 1024;
    let shape = curve(settings.model, settings.character);
    let mut levels = [0.0_f64; 10];
    for n in 0..POINTS {
        let cos = (std::f64::consts::TAU * (n as f64 + 0.5) / POINTS as f64).cos();
        let y = shape.value(f64::from(amplitude) * cos);
        // cos(kθ) by Chebyshev's recurrence.
        let (mut before, mut current) = (1.0, cos);
        for level in &mut levels {
            *level += y * current;
            (before, current) = (current, 2.0 * cos * current - before);
        }
    }
    let fundamental = levels[0].abs().max(1e-30);
    for (out, level) in out.iter_mut().zip(levels) {
        *out = (20.0 * (level.abs() / fundamental).max(1e-5).log10()) as f32;
    }
}

/// Coefficients that follow a moving parameter are recomputed this often
/// and ramped in between.
const TICK: u32 = 32;
const SLOW_RAMP_SECONDS: f32 = 0.020;
const SPLIT_RAMP_SECONDS: f32 = 0.010;
const SWAP_SECONDS: f32 = 0.005;
/// Every curve is flat to 1e-7 beyond this, and the antiderivatives stay
/// well inside f64.
const SHAPER_LIMIT: f64 = 1e4;
/// Input is held to ±36 dBFS, so the full drive stays inside the shaper's
/// range.
const INPUT_LIMIT: f32 = 64.0;
const TONE_PIVOT_HZ: f32 = 1000.0;
const BLOOM_HZ: f32 = 120.0;
const BLOOM_AMOUNT: f32 = 0.26;
const DC_BLOCKER_HZ: f32 = 5.0;

const PAD: usize = (LATENCY - Oversampler::latency(OversampleFactor::X2)) as usize;

#[derive(Clone, Debug)]
struct Channel {
    focus: Svf,
    low_delay: StaticDelay<{ LATENCY as usize }>,
    dry_delay: StaticDelay<{ LATENCY as usize }>,
    pre: Shelf1,
    post: Shelf1,
    bump: Shelf1,
    bloom: Shelf1,
    dc: Shelf1,
    tone: Shelf1,
    oversampler: Oversampler,
    shaper: AntialiasedShaper<Curve>,
    /// Makes up the latency of the shorter oversamplers.
    pad: StaticDelay<PAD>,
}

impl Channel {
    fn new(model: Model, character: f32, quality: OversampleFactor) -> Self {
        Self {
            focus: Svf::default(),
            low_delay: StaticDelay::default(),
            dry_delay: StaticDelay::default(),
            pre: Shelf1::default(),
            post: Shelf1::default(),
            bump: Shelf1::default(),
            bloom: Shelf1::default(),
            dc: Shelf1::default(),
            tone: Shelf1::default(),
            oversampler: Oversampler::new(quality),
            shaper: AntialiasedShaper::new(curve(model, character)),
            pad: StaticDelay::default(),
        }
    }

    #[inline]
    fn shape(&mut self, input: f32) -> f32 {
        let mut buffer = [0.0; Oversampler::MAX];
        // Flushed on the way in and out, so the FIR histories never hold
        // denormals.
        let count = self.oversampler.upsample(flush(input), &mut buffer);
        for sample in &mut buffer[..count] {
            let x = f64::from(*sample).clamp(-SHAPER_LIMIT, SHAPER_LIMIT);
            *sample = flush(self.shaper.process(x) as f32);
        }
        self.oversampler.downsample(&buffer)
    }
}

pub struct Saturator {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    dirty: bool,
    sample_rate: f32,
    channels: [Channel; 2],
    /// The model and quality the audio path runs now; a change waits for the
    /// swap fade to reach zero.
    active: (Model, OversampleFactor),
    pending: Option<(Model, OversampleFactor)>,
    /// 1 = the saturated path, 0 = the delayed dry signal, while model or
    /// quality are exchanged.
    swap: LinearRamp,
    /// Samples the fade stays at zero after an exchange, until the new
    /// oversampler has filled.
    swap_hold: u32,
    /// Drive, reference gain and compensation ramp in dB, so that their
    /// product keeps its level on the way.
    drive: LinearRamp,
    rho: LinearRamp,
    comp: LinearRamp,
    gains: [f32; 3],
    character: LinearRamp,
    character_applied: f32,
    tone: LinearRamp,
    tone_applied: f32,
    tone_coefficients: [LinearRamp; 3],
    /// log2 of the Focus frequency.
    focus: LinearRamp,
    focus_applied: f32,
    focus_g: LinearRamp,
    /// 1 while Focus keeps the low band clean, 0 when it is off.
    split: LinearRamp,
    mix: Smoothed,
    output: Smoothed,
    tape_power: Ballistics,
    bloom_level: Ballistics,
    /// Droop corrector coefficient per quality at this sample rate.
    correctors: [f64; 3],
    tick: u32,
    level: f32,
    telemetry: Arc<Telemetry>,
}

impl Default for Saturator {
    fn default() -> Self {
        Self::new()
    }
}

fn decode(values: &[f32; PARAMETERS.len()]) -> Settings {
    Settings::from_normalized(|id| {
        values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
    })
}

fn quality_index(quality: OversampleFactor) -> usize {
    match quality {
        OversampleFactor::X2 => 0,
        OversampleFactor::X4 => 1,
        OversampleFactor::X8 => 2,
    }
}

/// Advances a ramp in dB and refreshes its linear gain while it moves.
#[inline]
fn step_db(ramp: &mut LinearRamp, gain: &mut f32) {
    if !ramp.settled() {
        *gain = db_to_gain(ramp.next());
    }
}

impl Saturator {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let settings = decode(&values);
        let channel = Channel::new(settings.model, settings.character, settings.quality);
        let mut saturator = Self {
            values,
            settings,
            dirty: false,
            sample_rate: 48_000.0,
            channels: [channel.clone(), channel],
            active: (settings.model, settings.quality),
            pending: None,
            swap: LinearRamp::new(1.0),
            swap_hold: 0,
            drive: LinearRamp::new(0.0),
            rho: LinearRamp::new(0.0),
            comp: LinearRamp::new(0.0),
            gains: [1.0; 3],
            character: LinearRamp::new(settings.character),
            character_applied: settings.character,
            tone: LinearRamp::new(settings.tone_db),
            tone_applied: settings.tone_db,
            tone_coefficients: [LinearRamp::new(1.0); 3],
            focus: LinearRamp::new(settings.focus_hz.log2()),
            focus_applied: 0.0,
            focus_g: LinearRamp::new(0.0),
            split: LinearRamp::new(0.0),
            mix: Smoothed::new(settings.mix),
            output: Smoothed::new(db_to_gain(settings.output_db)),
            tape_power: Ballistics::default(),
            bloom_level: Ballistics::default(),
            correctors: [0.0; 3],
            tick: 0,
            level: 0.0,
            telemetry: Arc::new(Telemetry::default()),
        };
        saturator.prepare(48_000.0, 0);
        saturator
    }

    fn samples(&self, seconds: f32) -> u32 {
        (seconds * self.sample_rate).ceil() as u32
    }

    fn update_settings(&mut self) {
        let settings = decode(&self.values);
        self.settings = settings;
        self.dirty = false;
        let slow = self.samples(SLOW_RAMP_SECONDS);
        let rho_db = (20.0 * reference_gain(&settings).log10()) as f32;
        self.drive.set_target(settings.drive_db, slow);
        self.rho.set_target(rho_db, slow);
        self.comp
            .set_target(if settings.auto_gain { -rho_db } else { 0.0 }, slow);
        self.character.set_target(settings.character, slow);
        self.tone.set_target(settings.tone_db, slow);
        self.focus.set_target(settings.focus_hz.log2(), slow);
        let split = if settings.focus_on { 1.0 } else { 0.0 };
        if self.split.target() != split {
            self.split
                .set_target(split, self.samples(SPLIT_RAMP_SECONDS));
        }
        self.mix.set_target(settings.mix);
        self.output.set_target(db_to_gain(settings.output_db));
        let wanted = (settings.model, settings.quality);
        let (aim, pending) = if wanted == self.active {
            (1.0, None)
        } else {
            (0.0, Some(wanted))
        };
        self.pending = pending;
        if self.swap.target() != aim && self.swap_hold == 0 {
            let fade = self.samples(SWAP_SECONDS);
            self.swap.set_target(aim, fade);
        }
    }

    /// Exchanges model and quality while the fade is at zero.
    fn apply_swap(&mut self, (model, quality): (Model, OversampleFactor)) {
        self.active = (model, quality);
        let rate = self.sample_rate;
        let (pre, post) = emphasis_pair(model, rate);
        let shape = curve(model, self.character.value());
        let corrector = self.correctors[quality_index(quality)];
        for channel in &mut self.channels {
            channel.oversampler.set_factor(quality);
            channel.shaper.set_shape(shape);
            channel.shaper.reset();
            channel.shaper.corrector.c = corrector;
            channel.pre = pre.clone();
            channel.post = post.clone();
            channel.pad = StaticDelay::default();
            channel.bump.reset();
            channel.bloom.reset();
            channel.dc.reset();
        }
        self.character_applied = self.character.value();
        self.tape_power.reset();
        self.bloom_level.reset();
    }

    fn control_tick(&mut self) {
        let rate = self.sample_rate;
        let tone = self.tone.value();
        if tone != self.tone_applied {
            self.tone_applied = tone;
            let tilt = Shelf1::tilt(tone, TONE_PIVOT_HZ, rate);
            for (ramp, target) in
                self.tone_coefficients
                    .iter_mut()
                    .zip([tilt.big_g, tilt.low_gain, tilt.high_gain])
            {
                ramp.set_target(target, TICK);
            }
        }
        let focus = self.focus.value();
        if focus != self.focus_applied {
            self.focus_applied = focus;
            self.focus_g.set_target(prewarp(focus.exp2(), rate), TICK);
        }
        let character = self.character.value();
        if character != self.character_applied {
            self.character_applied = character;
            let shape = curve(self.active.0, character);
            for channel in &mut self.channels {
                channel.shaper.set_shape(shape);
            }
        }
    }

    /// Settles every ramp on the current settings and clears the audio state.
    fn settle(&mut self) {
        self.update_settings();
        let settings = self.settings;
        for ramp in [
            &mut self.drive,
            &mut self.rho,
            &mut self.comp,
            &mut self.character,
            &mut self.tone,
            &mut self.focus,
            &mut self.split,
        ] {
            ramp.snap();
        }
        self.gains = [self.drive.value(), self.rho.value(), self.comp.value()].map(db_to_gain);
        self.mix.snap();
        self.output.snap();
        let rate = self.sample_rate;
        let tilt = Shelf1::tilt(settings.tone_db, TONE_PIVOT_HZ, rate);
        self.tone_applied = settings.tone_db;
        self.tone_coefficients = [tilt.big_g, tilt.low_gain, tilt.high_gain].map(LinearRamp::new);
        self.focus_applied = self.focus.value();
        self.focus_g = LinearRamp::new(prewarp(settings.focus_hz, rate));
        self.swap = LinearRamp::new(1.0);
        self.swap_hold = 0;
        self.pending = None;
        let mut channel = Channel::new(settings.model, settings.character, settings.quality);
        channel.bump = Shelf1::low_shelf(1.5, 90.0, rate);
        channel.bloom = Shelf1::low_shelf(0.0, BLOOM_HZ, rate);
        channel.dc = Shelf1::high_pass(DC_BLOCKER_HZ, rate);
        channel.tone = tilt;
        self.channels = [channel.clone(), channel];
        self.apply_swap((settings.model, settings.quality));
        self.tick = 0;
        self.level = 0.0;
    }
}

impl Device for Saturator {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        let rate = self.sample_rate;
        self.correctors =
            [2.0, 4.0, 8.0].map(|factor| DroopCorrector::coefficient(factor * f64::from(rate)));
        self.mix.set_time(5.0, rate);
        self.output.set_time(5.0, rate);
        self.tape_power.set_times(10.0, 150.0, rate);
        self.bloom_level.set_times(30.0, 300.0, rate);
        self.telemetry.set_value(TELEMETRY_SAMPLE_RATE, rate);
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

    fn process(&mut self, _context: &ProcessContext, _events: &[Event], io: StereoIo<'_>) {
        if self.dirty {
            self.update_settings();
        }
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        self.telemetry.record_input(left, right);
        let mut peak = 0.0_f32;
        for index in 0..frames {
            if self.tick == 0 {
                self.control_tick();
            }
            self.tick = (self.tick + 1) % TICK;

            let mut wet_share = self.swap.next();
            if self.swap_hold > 0 {
                self.swap_hold -= 1;
                if self.swap_hold == 0 {
                    let aim = if self.pending.is_some() { 0.0 } else { 1.0 };
                    self.swap.set_target(aim, self.samples(SWAP_SECONDS));
                }
            } else if wet_share == 0.0
                && let Some(pending) = self.pending.take()
            {
                self.apply_swap(pending);
                self.swap_hold = LATENCY;
            }
            if self.swap_hold > 0 {
                wet_share = 0.0;
            }

            let [drive, rho, comp] = &mut self.gains;
            step_db(&mut self.drive, drive);
            step_db(&mut self.rho, rho);
            step_db(&mut self.comp, comp);
            let (drive, rho, comp) = (*drive, *rho, *comp);
            let character = self.character.next();
            self.tone.next();
            self.focus.next();
            let split = self.split.next();
            let focus_g = self.focus_g.next();
            let tone = self.tone_coefficients.each_mut().map(LinearRamp::next);
            let mix = self.mix.next_value();
            let output = self.output.next_value();
            let model = self.active.0;
            let pad = (LATENCY - Oversampler::latency(self.active.1)) as usize;

            // One bad sample must not poison the filter states: the squares
            // of a huge finite one overflow in the level detectors.
            let input = [left[index], right[index]].map(|x| {
                if x.is_finite() {
                    x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
                } else {
                    0.0
                }
            });
            let mut dry = [0.0; 2];
            let mut low = [0.0; 2];
            let mut driven = [0.0; 2];
            for (channel, state) in self.channels.iter_mut().enumerate() {
                let x = input[channel];
                dry[channel] = state.dry_delay.process(x);
                // A subtractive split sums back to the input exactly, so
                // with a linear shaper the device is a pure delay.
                let high = state
                    .focus
                    .process(x, focus_g, std::f32::consts::SQRT_2)
                    .high;
                let clean = split * (x - high);
                low[channel] = state.low_delay.process(clean);
                driven[channel] = (x - clean) * drive;
            }
            peak = peak.max(driven[0].abs()).max(driven[1].abs());

            let compression = if model == Model::Tape {
                let power = self
                    .tape_power
                    .process(0.5 * (driven[0] * driven[0] + driven[1] * driven[1]));
                let reduction = tape_curve(character).reduction_db(10.0 * power.max(1e-12).log10());
                if reduction > 0.0 {
                    db_to_gain(-reduction)
                } else {
                    1.0
                }
            } else {
                1.0
            };
            // A level-dependent low shelf as a parallel low-pass: only a
            // scalar moves, so it cannot click. At most +2 dB.
            let bloom = if model == Model::Tube {
                let level = self
                    .bloom_level
                    .process(driven[0].abs().max(driven[1].abs()));
                BLOOM_AMOUNT * level * level / (1.0 + level * level)
            } else {
                0.0
            };

            for (channel, sample) in [&mut *left, &mut *right].into_iter().enumerate() {
                let state = &mut self.channels[channel];
                let mut wet = state.pre.process(driven[channel] * compression);
                wet = state.shape(wet);
                wet = state.pad.process_at(wet, pad);
                wet = state.post.process(wet);
                match model {
                    Model::Transistor => {}
                    Model::Tube => wet += bloom * state.bloom.low(wet),
                    Model::Tape => wet = state.bump.process(wet),
                }
                wet = state.dc.process(wet);
                wet = (wet + rho * low[channel]) * comp;
                [state.tone.big_g, state.tone.low_gain, state.tone.high_gain] = tone;
                wet = state.tone.process(wet);
                let dry = dry[channel];
                wet = dry + wet_share * (wet - dry);
                sample[index] = (dry + mix * (wet - dry)) * output;
            }
        }
        // Held with a 300 ms fall, so the panel's dot does not flicker.
        let fall = (-(frames as f32) / (0.3 * self.sample_rate)).exp();
        self.level = flush((self.level * fall).max(peak));
        self.telemetry.set_value(TELEMETRY_LEVEL, self.level);
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        self.settle();
        self.tape_power.reset();
        self.bloom_level.reset();
    }

    fn latency_samples(&self) -> u32 {
        LATENCY
    }

    fn tail_samples(&self) -> u32 {
        // The DC blocker is the only thing that still sounds after the input
        // stops; by then it is 120 dB down.
        LATENCY + (0.5 * self.sample_rate) as u32
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}
