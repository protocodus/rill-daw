//! R1 Equalizer: high-pass, four parametric bands and low-pass as a serial
//! chain of matched second-order sections, each run as a state-variable
//! filter in f64. See `docs/R1_DEVICES.md#r1-equalizer`.
//!
//! The linear path has no latency and no oversampling. Analog mode changes
//! the prototypes (proportional-Q bells, dipped shelves) and adds a bounded
//! colour as an oversampled residual on top of the untouched signal.
use super::param;
use crate::dsp::matched::{decay_rate, impulse_invariant};
use crate::dsp::{
    Adaa1, BUTTERWORTH_Q, Biquad, BoundedCubic, Fit, LinearRamp, MatchedDesigner, Oversampler2x,
    Prototype, Smoothed, Svf, SvfMix, SvfMixRamp, db_to_gain, flush,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, ParamRange, ParamSpec, ProcessContext, StereoIo,
    Telemetry,
};
use std::sync::Arc;

pub const HP_ON: u32 = 1;
pub const HP_FREQUENCY: u32 = 2;
pub const HP_SLOPE: u32 = 3;
/// First ID of each parametric band; add the `BAND_*` offsets.
pub const BANDS: [u32; 4] = [10, 20, 30, 40];
pub const BAND_ON: u32 = 0;
pub const BAND_TYPE: u32 = 1;
pub const BAND_FREQUENCY: u32 = 2;
pub const BAND_GAIN: u32 = 3;
pub const BAND_Q: u32 = 4;
pub const LP_ON: u32 = 50;
pub const LP_FREQUENCY: u32 = 51;
pub const LP_SLOPE: u32 = 52;
pub const MODE: u32 = 60;
pub const AUTO_GAIN: u32 = 61;
pub const ANALYZER: u32 = 62;
pub const OUTPUT: u32 = 63;

/// Telemetry value slot: the sample rate the device runs at, which the panel
/// needs to draw the curve it hears and to place the analyzer's bins.
pub const TELEMETRY_SAMPLE_RATE: usize = 0;

pub const TYPES: &[&str] = &["Bell", "Low shelf", "High shelf", "Notch"];
/// Per octave; the labels match R1 Filter's.
pub const SLOPES: &[&str] = &["12 dB", "24 dB", "48 dB"];
pub const MODES: &[&str] = &["Digital", "Analog"];
pub const ANALYZERS: &[&str] = &["Off", "Input", "Output"];

const fn toggle(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    on: bool,
) -> ParamSpec {
    param(
        id,
        key,
        name,
        group,
        "",
        ParamRange::Toggle,
        if on { 1.0 } else { 0.0 },
    )
}

const fn choice(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    options: &'static [&'static str],
    default: f32,
) -> ParamSpec {
    param(
        id,
        key,
        name,
        group,
        "",
        ParamRange::Choice(options),
        default,
    )
}

const fn frequency(
    id: u32,
    key: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
) -> ParamSpec {
    param(
        id,
        key,
        "Frequency",
        group,
        "Hz",
        ParamRange::Log { min, max },
        default,
    )
}

const fn decibels(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
) -> ParamSpec {
    param(
        id,
        key,
        name,
        group,
        "dB",
        ParamRange::Linear {
            min: -24.0,
            max: 24.0,
        },
        0.0,
    )
}

const fn q(id: u32, key: &'static str, group: &'static str, default: f32) -> ParamSpec {
    param(
        id,
        key,
        "Q",
        group,
        "",
        ParamRange::Log {
            min: 0.1,
            max: 18.0,
        },
        default,
    )
}

pub static PARAMETERS: [ParamSpec; 30] = [
    toggle(HP_ON, "hp.on", "On", "High-pass", false),
    frequency(
        HP_FREQUENCY,
        "hp.frequency",
        "High-pass",
        20.0,
        2000.0,
        30.0,
    ),
    choice(HP_SLOPE, "hp.slope", "Slope", "High-pass", SLOPES, 0.0),
    toggle(10, "band1.on", "On", "Band 1", true),
    choice(11, "band1.type", "Type", "Band 1", TYPES, 1.0),
    frequency(12, "band1.frequency", "Band 1", 20.0, 20_000.0, 100.0),
    decibels(13, "band1.gain", "Gain", "Band 1"),
    q(14, "band1.q", "Band 1", 0.71),
    toggle(20, "band2.on", "On", "Band 2", true),
    choice(21, "band2.type", "Type", "Band 2", TYPES, 0.0),
    frequency(22, "band2.frequency", "Band 2", 20.0, 20_000.0, 400.0),
    decibels(23, "band2.gain", "Gain", "Band 2"),
    q(24, "band2.q", "Band 2", 1.0),
    toggle(30, "band3.on", "On", "Band 3", true),
    choice(31, "band3.type", "Type", "Band 3", TYPES, 0.0),
    frequency(32, "band3.frequency", "Band 3", 20.0, 20_000.0, 2500.0),
    decibels(33, "band3.gain", "Gain", "Band 3"),
    q(34, "band3.q", "Band 3", 1.0),
    toggle(40, "band4.on", "On", "Band 4", true),
    choice(41, "band4.type", "Type", "Band 4", TYPES, 2.0),
    frequency(42, "band4.frequency", "Band 4", 20.0, 20_000.0, 8000.0),
    decibels(43, "band4.gain", "Gain", "Band 4"),
    q(44, "band4.q", "Band 4", 0.71),
    toggle(LP_ON, "lp.on", "On", "Low-pass", false),
    frequency(
        LP_FREQUENCY,
        "lp.frequency",
        "Low-pass",
        200.0,
        20_000.0,
        18_000.0,
    ),
    choice(LP_SLOPE, "lp.slope", "Slope", "Low-pass", SLOPES, 0.0),
    choice(MODE, "mode", "Mode", "Global", MODES, 0.0),
    toggle(AUTO_GAIN, "auto_gain", "Auto gain", "Global", false),
    choice(ANALYZER, "analyzer", "Analyzer", "Global", ANALYZERS, 2.0),
    decibels(OUTPUT, "output", "Output", "Output"),
];

pub static DESCRIPTOR: DeviceDescriptor = DeviceDescriptor {
    id: "r1-equalizer",
    name: "R1 Equalizer",
    kind: DeviceKind::Effect,
    category: "Equalizer",
    parameters: &PARAMETERS,
    sample_slots: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-equalizer/default.rillpreset"),
        include_str!("../../presets/r1-equalizer/low-cut-80.rillpreset"),
        include_str!("../../presets/r1-equalizer/vocal-presence.rillpreset"),
        include_str!("../../presets/r1-equalizer/kick-punch.rillpreset"),
        include_str!("../../presets/r1-equalizer/tight-bass.rillpreset"),
        include_str!("../../presets/r1-equalizer/de-mud.rillpreset"),
        include_str!("../../presets/r1-equalizer/air-shelf.rillpreset"),
        include_str!("../../presets/r1-equalizer/analog-smile.rillpreset"),
        include_str!("../../presets/r1-equalizer/telephone.rillpreset"),
        include_str!("../../presets/r1-equalizer/hum-notch-50-hz.rillpreset"),
    ],
    legacy_labels: &[],
    create: || Box::new(Equalizer::new()),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandType {
    Bell,
    LowShelf,
    HighShelf,
    Notch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slope {
    Db12,
    Db24,
    Db48,
}

impl Slope {
    /// Second-order sections in the cascade.
    pub fn sections(self) -> usize {
        BUTTERWORTH_Q[self as usize].len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Digital,
    Analog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Analyzer {
    Off,
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutSettings {
    pub on: bool,
    pub frequency: f32,
    pub slope: Slope,
}

impl CutSettings {
    /// Sections that are in the signal path.
    fn sections(&self) -> usize {
        if self.on { self.slope.sections() } else { 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandSettings {
    pub on: bool,
    pub kind: BandType,
    pub frequency: f32,
    pub gain_db: f32,
    pub q: f32,
}

impl BandSettings {
    /// Whether the band changes the signal: a bell or shelf at 0 dB does not.
    fn active(&self) -> bool {
        self.on && (self.kind == BandType::Notch || self.gain_db != 0.0)
    }
}

/// Plain parameter values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub hp: CutSettings,
    pub bands: [BandSettings; 4],
    pub lp: CutSettings,
    pub mode: Mode,
    pub auto_gain: bool,
    pub analyzer: Analyzer,
    pub output_db: f32,
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("equalizer parameter");
            spec.to_plain(value(id))
        };
        let on = |id: u32| plain(id) >= 0.5;
        let cut = |on_id: u32, frequency: u32, slope: u32| CutSettings {
            on: on(on_id),
            frequency: plain(frequency),
            slope: match plain(slope) as usize {
                1 => Slope::Db24,
                2 => Slope::Db48,
                _ => Slope::Db12,
            },
        };
        Self {
            hp: cut(HP_ON, HP_FREQUENCY, HP_SLOPE),
            bands: BANDS.map(|base| BandSettings {
                on: on(base + BAND_ON),
                kind: match plain(base + BAND_TYPE) as usize {
                    1 => BandType::LowShelf,
                    2 => BandType::HighShelf,
                    3 => BandType::Notch,
                    _ => BandType::Bell,
                },
                frequency: plain(base + BAND_FREQUENCY),
                gain_db: plain(base + BAND_GAIN),
                q: plain(base + BAND_Q),
            }),
            lp: cut(LP_ON, LP_FREQUENCY, LP_SLOPE),
            mode: if plain(MODE) as usize == 1 {
                Mode::Analog
            } else {
                Mode::Digital
            },
            auto_gain: on(AUTO_GAIN),
            analyzer: match plain(ANALYZER) as usize {
                0 => Analyzer::Off,
                1 => Analyzer::Input,
                _ => Analyzer::Output,
            },
            output_db: plain(OUTPUT),
        }
    }
}

/// Whether a parameter does anything at these settings: a notch has no gain.
/// Panels disable the ones that do not.
pub fn applies(id: u32, settings: &Settings) -> bool {
    !BANDS
        .iter()
        .zip(&settings.bands)
        .any(|(base, band)| id == base + BAND_GAIN && band.kind == BandType::Notch)
}

/// Second-order sections: high-pass 0–3, bands 4–7, low-pass 8–11.
pub const SECTIONS: usize = 12;
const HP_SECTION: usize = 0;
const BAND_SECTION: usize = 4;
const LP_SECTION: usize = 8;

/// The analog transfer function of a band. At 0 dB a bell or shelf is flat,
/// but its denominator still says where the poles of small gains sit.
fn prototype(kind: BandType, gain_db: f64, q: f64, mode: Mode) -> Prototype {
    let amplitude = 10.0_f64.powf(gain_db.abs() / 40.0);
    let boost = match (kind, mode) {
        (BandType::Notch, _) => return Prototype::notch(q),
        (BandType::Bell, Mode::Digital) => Prototype::bell(amplitude, q),
        // Proportional Q: half the Q near 0 dB, the set Q at ±12 dB, double
        // at ±24 dB, so small moves are broad and large ones focused.
        (BandType::Bell, Mode::Analog) => Prototype::bell(
            amplitude,
            (q * 10.0_f64.powf((gain_db.abs() - 12.0) / 40.0)).clamp(0.05, 40.0),
        ),
        (BandType::LowShelf, Mode::Digital) => Prototype::low_shelf(amplitude, q, q),
        (BandType::HighShelf, Mode::Digital) => Prototype::high_shelf(amplitude, q, q),
        // Sharper zeros than poles dip the far side of the transition, as
        // passive shelves do, by about a tenth of the gain.
        (BandType::LowShelf, Mode::Analog) => {
            Prototype::low_shelf(amplitude, q * amplitude.sqrt(), q)
        }
        (BandType::HighShelf, Mode::Analog) => {
            Prototype::high_shelf(amplitude, q * amplitude.sqrt(), q)
        }
    };
    if gain_db < 0.0 {
        boost.reciprocal()
    } else {
        boost
    }
}

/// The analog prototype of a parametric band, or `None` for a bell or shelf
/// at 0 dB, which is flat.
pub fn band_prototype(kind: BandType, gain_db: f64, q: f64, mode: Mode) -> Option<Prototype> {
    (kind == BandType::Notch || gain_db != 0.0).then(|| prototype(kind, gain_db, q, mode))
}

fn band_target(
    designer: &MatchedDesigner,
    kind: BandType,
    frequency: f64,
    gain_db: f64,
    q: f64,
    mode: Mode,
) -> (SvfMix<f64>, Fit) {
    let prototype = prototype(kind, gain_db, q, mode);
    let (mix, fit) = if kind == BandType::Notch {
        let (biquad, fit) = designer.notch(q, frequency);
        (SvfMix::from_biquad(&biquad), fit)
    } else if gain_db == 0.0 {
        // The identity, at the poles a small gain would have: ramping to it
        // and away from it moves only the mix.
        let (a1, a2) = impulse_invariant(prototype.denominator, designer.omega(frequency));
        let poles = Biquad {
            b0: 1.0,
            b1: a1,
            b2: a2,
            a1,
            a2,
        };
        (SvfMix::from_biquad(&poles).bypassed(), Fit::Fitted)
    } else {
        let (biquad, fit) = designer.design(&prototype, frequency);
        (SvfMix::from_biquad(&biquad), fit)
    };
    match kind {
        // These take their output from the band state. Fed through
        // `constant_peak` it does not depend on Q, so sweeping Q or gain at
        // the centre of a tall, narrow bell does not overshoot.
        BandType::Bell | BandType::Notch => (mix.constant_peak(), fit),
        // Shelves lean on the low state, which must not move with Q.
        BandType::LowShelf | BandType::HighShelf => (mix, fit),
    }
}

fn cut_target(
    designer: &MatchedDesigner,
    high_pass: bool,
    frequency: f64,
    slope: Slope,
    section: usize,
) -> (SvfMix<f64>, Fit) {
    let q = BUTTERWORTH_Q[slope as usize][section];
    let (biquad, fit) = if high_pass {
        (designer.high_pass(q, frequency), Fit::Fitted)
    } else {
        designer.low_pass(q, frequency)
    };
    (SvfMix::from_biquad(&biquad), fit)
}

/// Coefficients of one parametric band; the identity at 0 dB.
pub fn band_mix(
    designer: &MatchedDesigner,
    kind: BandType,
    frequency: f64,
    gain_db: f64,
    q: f64,
    mode: Mode,
) -> SvfMix<f64> {
    band_target(designer, kind, frequency, gain_db, q, mode).0
}

/// Coefficients of one section (below `slope.sections()`) of a cut filter.
pub fn cut_mix(
    designer: &MatchedDesigner,
    high_pass: bool,
    frequency: f64,
    slope: Slope,
    section: usize,
) -> SvfMix<f64> {
    cut_target(designer, high_pass, frequency, slope, section).0
}

/// Every section that changes the signal at these settings; `None` for one
/// that is off or flat.
pub fn design(settings: &Settings, designer: &MatchedDesigner) -> [Option<SvfMix<f64>>; SECTIONS] {
    let mut sections = [None; SECTIONS];
    for (cut, first, high_pass) in [
        (&settings.hp, HP_SECTION, true),
        (&settings.lp, LP_SECTION, false),
    ] {
        for section in 0..cut.sections() {
            sections[first + section] = Some(cut_mix(
                designer,
                high_pass,
                f64::from(cut.frequency),
                cut.slope,
                section,
            ));
        }
    }
    for (index, band) in settings.bands.iter().enumerate() {
        if band.active() {
            sections[BAND_SECTION + index] = Some(band_mix(
                designer,
                band.kind,
                f64::from(band.frequency),
                f64::from(band.gain_db),
                f64::from(band.q),
                settings.mode,
            ));
        }
    }
    sections
}

fn magnitudes_db(
    sections: &[Option<SvfMix<f64>>],
    sample_rate: f32,
    frequencies: &[f32],
    out: &mut [f32],
) {
    for (frequency, out) in frequencies.iter().zip(out) {
        let omega = std::f64::consts::TAU * f64::from(*frequency) / f64::from(sample_rate);
        let magnitude: f64 = sections
            .iter()
            .flatten()
            .map(|mix| mix.response(omega).norm())
            .product();
        *out = (20.0 * magnitude.max(1e-9).log10()) as f32;
    }
}

/// Small-signal magnitude response in dB of the whole equalizer, without
/// auto gain and Output: the curve the panel draws. Uses the coefficients
/// the audio path runs.
pub fn response_db(settings: &Settings, sample_rate: f32, frequencies: &[f32], out: &mut [f32]) {
    let sections = design(settings, &MatchedDesigner::new(sample_rate));
    magnitudes_db(&sections, sample_rate, frequencies, out);
}

/// Response of one band alone: 0 the high-pass, 1–4 the parametric bands,
/// 5 the low-pass.
pub fn band_response_db(
    settings: &Settings,
    band: usize,
    sample_rate: f32,
    frequencies: &[f32],
    out: &mut [f32],
) {
    let sections = design(settings, &MatchedDesigner::new(sample_rate));
    let range = match band {
        0 => HP_SECTION..BAND_SECTION,
        5.. => LP_SECTION..SECTIONS,
        _ => BAND_SECTION + band - 1..BAND_SECTION + band,
    };
    magnitudes_db(&sections[range], sample_rate, frequencies, out);
}

const AUTO_GAIN_POINTS: usize = 64;
const AUTO_GAIN_LIMIT_DB: f64 = 12.0;

/// Static loudness compensation: minus the mean power gain of the enabled
/// parametric bands over 40 Hz – 16 kHz with equal weight per octave, which
/// is the level change of a pink spectrum. Derived from the curve, not the
/// signal, so it never pumps. The cut filters are left out: they are
/// corrective, and boosting what a low-pass leaves is not a level match.
fn compensation_db(settings: &Settings, designer: &MatchedDesigner) -> f32 {
    if !settings.auto_gain {
        return 0.0;
    }
    let sections = design(settings, designer);
    let bands = &sections[BAND_SECTION..LP_SECTION];
    let power: f64 = (0..AUTO_GAIN_POINTS)
        .map(|point| {
            let frequency = 40.0 * 400.0_f64.powf(point as f64 / (AUTO_GAIN_POINTS - 1) as f64);
            let omega = std::f64::consts::TAU * frequency / designer.sample_rate();
            bands
                .iter()
                .flatten()
                .map(|mix| {
                    let response = mix.response(omega);
                    response.re * response.re + response.im * response.im
                })
                .product::<f64>()
        })
        .sum();
    let mean = power / AUTO_GAIN_POINTS as f64;
    (-10.0 * mean.max(1e-12).log10()).clamp(-AUTO_GAIN_LIMIT_DB, AUTO_GAIN_LIMIT_DB) as f32
}

/// The gain in dB that Auto gain applies at these settings; 0 when it is off.
pub fn auto_gain_db(settings: &Settings, sample_rate: f32) -> f32 {
    compensation_db(settings, &MatchedDesigner::new(sample_rate))
}

/// Seconds a tail may be reported; the engine caps it there as well.
const MAX_TAIL_SECONDS: f64 = 30.0;

/// Samples until the slowest pole has decayed by 120 dB.
fn tail_samples(settings: &Settings, designer: &MatchedDesigner) -> u32 {
    let mut slowest = f64::INFINITY;
    for cut in [&settings.hp, &settings.lp] {
        let omega = designer.omega(f64::from(cut.frequency));
        for &q in &BUTTERWORTH_Q[cut.slope as usize][..cut.sections()] {
            slowest = slowest.min(decay_rate(Prototype::low_pass(q).denominator, omega));
        }
    }
    for band in settings.bands.iter().filter(|band| band.active()) {
        let prototype = prototype(
            band.kind,
            f64::from(band.gain_db),
            f64::from(band.q),
            settings.mode,
        );
        let omega = designer.omega(f64::from(band.frequency));
        slowest = slowest.min(decay_rate(prototype.denominator, omega));
    }
    let filters = if slowest.is_finite() {
        // ln(10⁶). The margin covers what one pole does not describe: the
        // sections of a steep cut ringing together, and the fitted poles
        // near Nyquist.
        1.06 * 13.8155 / slowest + 8.0
    } else {
        0.0
    };
    // The residual of the Analog stage leaves through the oversampler's FIR.
    let stage = match settings.mode {
        Mode::Analog => 64.0,
        Mode::Digital => 0.0,
    };
    (filters + stage).min(MAX_TAIL_SECONDS * designer.sample_rate()) as u32
}

/// Samples between two looks at the parameters. Band values are smoothed and
/// their filters redesigned at this rate; coefficients ramp in between.
const TICK: u32 = 32;
/// Milliseconds a moving band value takes to arrive.
const BAND_SMOOTHING_MS: f32 = 20.0;
/// Seconds of the fades and crossfades that cover discrete changes.
const FADE_SECONDS: f32 = 0.010;
/// Smoothed band values closer than this to their target (in octaves or dB)
/// jump to it. The one-pole would otherwise creep through rounding for half
/// a second, redesigning the filter all the way.
const ARRIVED: f32 = 1e-4;
/// Output guard, +36 dBFS. Four bands and Output add up to +120 dB of gain
/// and nothing here limits; no usable signal comes near this, but extreme
/// settings must not hand the engine values that overflow downstream.
const LIMIT: f64 = 64.0;
/// Largest value handed to the Analog stage, which runs in `f32`.
const SHAPER_LIMIT: f64 = 1e6;
/// Knee of the Analog colour: it is strongest around +6 dBFS.
const KNEE: f64 = 2.0;

struct Section {
    ramp: SvfMixRamp<f64>,
    /// Left, right.
    state: [Svf<f64>; 2],
    /// 0 out of the path, 1 in.
    wet: LinearRamp<f64>,
    /// Samples left of the crossfade that covers a discrete change.
    long: u32,
    /// The target passes the signal unchanged.
    identity: bool,
    fit: Fit,
    /// Fading in from nothing: the next design is taken at once.
    fresh: bool,
    /// Leaving the path to come back with coefficients that no ramp leads
    /// to: those of another band type.
    swap: bool,
}

impl Section {
    fn new() -> Self {
        Self {
            ramp: SvfMixRamp::new(SvfMix::identity()),
            state: Default::default(),
            wet: LinearRamp::new(0.0),
            long: 0,
            identity: true,
            fit: Fit::Fitted,
            fresh: false,
            swap: false,
        }
    }

    fn out(&self) -> bool {
        self.wet.settled() && self.wet.value() == 0.0
    }

    fn idle(&self) -> bool {
        self.out() || (self.identity && self.ramp.done())
    }

    fn clear(&mut self) {
        for state in &mut self.state {
            state.reset();
        }
    }

    /// Fades the section into or out of the path.
    fn switch(&mut self, on: bool, samples: u32) {
        let target = if on { 1.0 } else { 0.0 };
        if self.wet.target() != target {
            self.fresh |= on && self.out();
            self.wet.set_target(target, samples);
            self.leave();
        }
    }

    /// A transparent section is not processed, so its fade out would never
    /// end, and nothing of it is heard that a fade could cover.
    fn leave(&mut self) {
        if self.identity && self.ramp.done() && self.wet.target() == 0.0 {
            self.wet.snap();
        }
    }

    /// Heads for new coefficients: over one tick when they follow a moving
    /// value, over `fade` samples after a discrete change.
    fn aim(&mut self, (mix, fit): (SvfMix<f64>, Fit), fade: u32) {
        let identity = mix.is_identity();
        if self.fresh {
            self.ramp.snap(mix);
            self.clear();
            self.long = 0;
            self.fresh = false;
        } else {
            // The two design methods disagree slightly at the same setting.
            if fit != self.fit {
                self.long = fade;
            }
            if identity && self.idle() {
                self.ramp.snap(mix);
            } else {
                self.ramp.aim(mix, self.long.max(TICK));
            }
            self.long = self.long.saturating_sub(TICK);
        }
        self.fit = fit;
        self.identity = identity;
    }

    /// Settles at once, for `reset`.
    fn set(&mut self, target: Option<(SvfMix<f64>, Fit)>) {
        let (mix, fit) = target.unwrap_or((SvfMix::identity(), Fit::Fitted));
        *self = Self {
            ramp: SvfMixRamp::new(mix),
            wet: LinearRamp::new(if target.is_some() { 1.0 } else { 0.0 }),
            identity: mix.is_identity(),
            fit,
            ..Self::new()
        };
    }

    fn process(&mut self, left: &mut [f64], right: &mut [f64]) {
        if self.idle() {
            return;
        }
        let [state_left, state_right] = &mut self.state;
        for (left, right) in left.iter_mut().zip(right) {
            let mix = self.ramp.advance();
            let filtered = [state_left.tick(*left, mix), state_right.tick(*right, mix)];
            // A fade can end inside the block, at either end.
            if self.wet.settled() && self.wet.value() == 1.0 {
                [*left, *right] = filtered;
            } else {
                let wet = if self.wet.settled() {
                    self.wet.value()
                } else {
                    self.wet.next()
                };
                *left += wet * (filtered[0] - *left);
                *right += wet * (filtered[1] - *right);
            }
        }
        self.leave();
        if self.idle() {
            self.clear();
        }
    }
}

/// Steps a band value and reports it arrived once it is close enough.
fn step(value: &mut Smoothed) {
    value.next_value();
    if (value.value() - value.target()).abs() < ARRIVED {
        value.snap();
    }
}

/// Smoothed values of a parametric band: log2 Hz, dB, log2 Q.
struct BandSmooth {
    frequency: Smoothed,
    gain: Smoothed,
    q: Smoothed,
    /// A discrete change needs a redesign although no value moves.
    dirty: bool,
}

impl BandSmooth {
    fn new(band: &BandSettings) -> Self {
        Self {
            frequency: Smoothed::new(band.frequency.log2()),
            gain: Smoothed::new(band.gain_db),
            q: Smoothed::new(band.q.log2()),
            dirty: false,
        }
    }

    fn values(&mut self) -> [&mut Smoothed; 3] {
        [&mut self.frequency, &mut self.gain, &mut self.q]
    }

    fn set_targets(&mut self, band: &BandSettings) {
        self.frequency.set_target(band.frequency.log2());
        self.gain.set_target(band.gain_db);
        self.q.set_target(band.q.log2());
    }

    fn settled(&self) -> bool {
        self.frequency.is_settled() && self.gain.is_settled() && self.q.is_settled()
    }
}

/// Smoothed corner of a cut filter, log2 Hz.
struct CutSmooth {
    frequency: Smoothed,
    dirty: bool,
}

pub struct Equalizer {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    changed: bool,
    sample_rate: f32,
    designer: MatchedDesigner,
    /// Samples of [`FADE_SECONDS`].
    fade: u32,
    sections: [Section; SECTIONS],
    bands: [BandSmooth; 4],
    hp: CutSmooth,
    lp: CutSmooth,
    /// Samples since the last control tick, below [`TICK`].
    tick_phase: u32,
    /// Amount of the Analog colour, 0–1.
    saturation: LinearRamp<f64>,
    oversamplers: [Oversampler2x; 2],
    shapers: [Adaa1<BoundedCubic>; 2],
    /// Linear gains.
    auto_gain: Smoothed,
    output: Smoothed,
    telemetry: Arc<Telemetry>,
}

impl Default for Equalizer {
    fn default() -> Self {
        Self::new()
    }
}

fn decode(values: &[f32; PARAMETERS.len()]) -> Settings {
    Settings::from_normalized(|id| {
        values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
    })
}

impl Equalizer {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let settings = decode(&values);
        let cut = |cut: &CutSettings| CutSmooth {
            frequency: Smoothed::new(cut.frequency.log2()),
            dirty: false,
        };
        let shaper = Adaa1::new(BoundedCubic { knee: KNEE });
        let mut equalizer = Self {
            values,
            settings,
            changed: false,
            sample_rate: 48_000.0,
            designer: MatchedDesigner::new(48_000.0),
            fade: 0,
            sections: std::array::from_fn(|_| Section::new()),
            bands: std::array::from_fn(|index| BandSmooth::new(&settings.bands[index])),
            hp: cut(&settings.hp),
            lp: cut(&settings.lp),
            tick_phase: 0,
            saturation: LinearRamp::new(0.0),
            oversamplers: Default::default(),
            shapers: [shaper.clone(), shaper],
            auto_gain: Smoothed::new(1.0),
            output: Smoothed::new(1.0),
            telemetry: Arc::new(Telemetry::default()),
        };
        equalizer.prepare(48_000.0, 0);
        equalizer
    }

    /// Takes changed parameters: new targets for everything that glides,
    /// fades and crossfades for everything that switches.
    fn apply_settings(&mut self) {
        let new = decode(&self.values);
        let old = std::mem::replace(&mut self.settings, new);
        self.changed = false;
        let fade = self.fade;
        for (index, band) in self.bands.iter_mut().enumerate() {
            let (target, previous) = (&new.bands[index], &old.bands[index]);
            let section = &mut self.sections[BAND_SECTION + index];
            band.set_targets(target);
            // A band on its way out for another type comes back by itself.
            if target.on != previous.on && !(target.on && section.swap) {
                band.dirty = true;
                section.switch(target.on, fade);
            }
            if new.mode != old.mode {
                band.dirty = true;
                section.long = fade;
            }
            // Coefficients halfway between two types are a filter of their
            // own, with a peak neither has (a bell's pole under a shelf's
            // mix). So the band fades out, changes and fades back in.
            if target.kind != previous.kind && target.on && !section.fresh {
                section.swap = true;
                section.switch(false, fade);
            }
        }
        for (cut, target, previous, first) in [
            (&mut self.hp, &new.hp, &old.hp, HP_SECTION),
            (&mut self.lp, &new.lp, &old.lp, LP_SECTION),
        ] {
            cut.frequency.set_target(target.frequency.log2());
            if target.sections() != previous.sections() {
                cut.dirty = true;
                // Sections the slope keeps take its Q in a crossfade; the
                // others fade in or out.
                for (index, section) in self.sections[first..first + 4].iter_mut().enumerate() {
                    section.long = fade;
                    section.switch(index < target.sections(), fade);
                }
            }
        }
        if new.mode != old.mode {
            let amount = if new.mode == Mode::Analog { 1.0 } else { 0.0 };
            self.saturation.set_target(amount, fade);
        }
        self.auto_gain
            .set_target(db_to_gain(compensation_db(&new, &self.designer)));
        self.output.set_target(db_to_gain(new.output_db));
    }

    fn update_band(&mut self, index: usize) {
        let band = &mut self.bands[index];
        let section = &mut self.sections[BAND_SECTION + index];
        if section.swap {
            if !section.out() {
                return;
            }
            section.swap = false;
            section.switch(self.settings.bands[index].on, self.fade);
        }
        if section.out() || section.fresh {
            // Nothing to glide from: an unheard band starts at its settings.
            for value in band.values() {
                value.snap();
            }
            if section.out() {
                band.dirty = false;
                return;
            }
        } else if band.dirty || !band.settled() {
            band.values().into_iter().for_each(step);
        } else {
            return;
        }
        band.dirty = false;
        let settings = &self.settings;
        let target = band_target(
            &self.designer,
            settings.bands[index].kind,
            f64::from(band.frequency.value().exp2()),
            f64::from(band.gain.value()),
            f64::from(band.q.value().exp2()),
            settings.mode,
        );
        section.aim(target, self.fade);
    }

    fn update_cut(&mut self, high_pass: bool) {
        let (cut, settings, first) = if high_pass {
            (&mut self.hp, &self.settings.hp, HP_SECTION)
        } else {
            (&mut self.lp, &self.settings.lp, LP_SECTION)
        };
        let sections = &mut self.sections[first..first + 4];
        if sections
            .iter()
            .all(|section| section.out() || section.fresh)
        {
            cut.frequency.snap();
        } else if cut.dirty || !cut.frequency.is_settled() {
            step(&mut cut.frequency);
        } else {
            return;
        }
        cut.dirty = false;
        // Sections on their way out keep the coefficients they have.
        for (index, section) in sections[..settings.sections()].iter_mut().enumerate() {
            let target = cut_target(
                &self.designer,
                high_pass,
                f64::from(cut.frequency.value().exp2()),
                settings.slope,
                index,
            );
            section.aim(target, self.fade);
        }
    }

    fn control_tick(&mut self) {
        if self.changed {
            self.apply_settings();
        }
        for index in 0..self.bands.len() {
            self.update_band(index);
        }
        self.update_cut(true);
        self.update_cut(false);
    }

    /// The Analog colour: `x + amount · down2(ADAA(up2(x)))`. Only the
    /// residual passes the oversampler, so the linear signal keeps its level,
    /// response and timing; the residual is bounded at 2 % of the signal, so
    /// arriving 31 samples late it cannot comb-filter audibly.
    fn saturate(&mut self, left: &mut [f64], right: &mut [f64]) {
        if self.saturation.settled() && self.saturation.value() == 0.0 {
            return;
        }
        for index in 0..left.len() {
            let amount = self.saturation.next();
            for (channel, buffer) in [&mut *left, &mut *right].into_iter().enumerate() {
                let oversampler = &mut self.oversamplers[channel];
                let shaper = &mut self.shapers[channel];
                // Flushed on the way in and out, so the FIR histories never
                // hold denormals.
                // The clamp keeps the conversion finite; the colour is long
                // gone up there.
                let input = buffer[index].clamp(-SHAPER_LIMIT, SHAPER_LIMIT);
                let up = oversampler.upsample(flush(input as f32));
                let shaped = up.map(|sample| flush(shaper.process(f64::from(sample)) as f32));
                buffer[index] += amount * f64::from(oversampler.downsample(shaped));
            }
        }
        if self.saturation.settled() && self.saturation.value() == 0.0 {
            self.clear_saturation();
        }
    }

    fn clear_saturation(&mut self) {
        for oversampler in &mut self.oversamplers {
            oversampler.reset();
        }
        for shaper in &mut self.shapers {
            shaper.reset();
        }
    }
}

impl Device for Equalizer {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        let rate = self.sample_rate;
        self.designer = MatchedDesigner::new(rate);
        self.fade = (FADE_SECONDS * rate).ceil() as u32;
        let tick_rate = rate / TICK as f32;
        for band in &mut self.bands {
            for value in band.values() {
                value.set_time(BAND_SMOOTHING_MS, tick_rate);
            }
        }
        self.hp.frequency.set_time(BAND_SMOOTHING_MS, tick_rate);
        self.lp.frequency.set_time(BAND_SMOOTHING_MS, tick_rate);
        self.auto_gain.set_time(50.0, rate);
        self.output.set_time(5.0, rate);
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
                self.changed = true;
            }
        }
    }

    fn process(&mut self, _context: &ProcessContext, _events: &[Event], io: StereoIo<'_>) {
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        self.telemetry.record_input(left, right);
        let mut offset = 0;
        while offset < frames {
            if self.tick_phase == 0 {
                self.control_tick();
            }
            let count = ((TICK - self.tick_phase) as usize).min(frames - offset);
            let (left, right) = (
                &mut left[offset..offset + count],
                &mut right[offset..offset + count],
            );
            let mut chunk = [[0.0_f64; TICK as usize]; 2];
            for (chunk, input) in chunk.iter_mut().zip([&*left, &*right]) {
                for (sample, input) in chunk.iter_mut().zip(input) {
                    // One bad sample must not poison the filter states.
                    *sample = if input.is_finite() {
                        f64::from(*input)
                    } else {
                        0.0
                    };
                }
            }
            let [chunk_left, chunk_right] = &mut chunk;
            let (chunk_left, chunk_right) = (&mut chunk_left[..count], &mut chunk_right[..count]);
            for section in &mut self.sections {
                section.process(chunk_left, chunk_right);
            }
            self.saturate(chunk_left, chunk_right);
            for index in 0..count {
                let gain =
                    f64::from(self.auto_gain.next_value()) * f64::from(self.output.next_value());
                left[index] = flush((chunk_left[index] * gain).clamp(-LIMIT, LIMIT) as f32);
                right[index] = flush((chunk_right[index] * gain).clamp(-LIMIT, LIMIT) as f32);
            }
            self.tick_phase = (self.tick_phase + count as u32) % TICK;
            offset += count;
        }
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        let settings = decode(&self.values);
        self.settings = settings;
        self.changed = false;
        for (band, target) in self.bands.iter_mut().zip(&settings.bands) {
            band.set_targets(target);
            for value in band.values() {
                value.snap();
            }
            band.dirty = false;
        }
        for (cut, target) in [(&mut self.hp, &settings.hp), (&mut self.lp, &settings.lp)] {
            cut.frequency.set_target(target.frequency.log2());
            cut.frequency.snap();
            cut.dirty = false;
        }
        let designer = &self.designer;
        for (index, section) in self.sections.iter_mut().enumerate() {
            let cut = |cut: &CutSettings, high_pass: bool, section: usize| {
                (section < cut.sections()).then(|| {
                    cut_target(
                        designer,
                        high_pass,
                        f64::from(cut.frequency),
                        cut.slope,
                        section,
                    )
                })
            };
            section.set(match index {
                HP_SECTION..BAND_SECTION => cut(&settings.hp, true, index - HP_SECTION),
                BAND_SECTION..LP_SECTION => {
                    let band = &settings.bands[index - BAND_SECTION];
                    band.on.then(|| {
                        band_target(
                            designer,
                            band.kind,
                            f64::from(band.frequency),
                            f64::from(band.gain_db),
                            f64::from(band.q),
                            settings.mode,
                        )
                    })
                }
                _ => cut(&settings.lp, false, index - LP_SECTION),
            });
        }
        self.saturation = LinearRamp::new(if settings.mode == Mode::Analog {
            1.0
        } else {
            0.0
        });
        self.clear_saturation();
        self.auto_gain
            .set_target(db_to_gain(compensation_db(&settings, &self.designer)));
        self.auto_gain.snap();
        self.output.set_target(db_to_gain(settings.output_db));
        self.output.snap();
        self.tick_phase = 0;
    }

    fn tail_samples(&self) -> u32 {
        // From the parameters as they are set, not as the audio thread last
        // saw them: the engine asks while it builds the graph.
        tail_samples(&decode(&self.values), &self.designer)
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}
