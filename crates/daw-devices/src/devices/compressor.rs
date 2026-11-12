//! R1 Compressor: compressor, brickwall limiter and downward expander with
//! four detector characters, lookahead and an external sidechain. See
//! `docs/R1_DEVICES.md#r1-compressor`.
use super::param;
use crate::dsp::{
    Ballistics, BoundedCubic, BoundedSquare, BoxSmoother, CurveKind, DelayLine, GainComputer,
    LinearRamp, Oversampler2x, ProgramRelease, Shape, Shelf1, SlidingMax, Smoothed, Svf,
    TruePeak4x, db_to_gain, flush, prewarp,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, HistoryMeter, ParamRange, ParamSpec,
    ProcessContext, StereoIo, Telemetry,
};
use std::sync::Arc;

pub const MODE: u32 = 1;
pub const CHARACTER: u32 = 2;
pub const THRESHOLD: u32 = 3;
pub const RATIO: u32 = 4;
pub const KNEE: u32 = 5;
pub const ATTACK: u32 = 6;
pub const RELEASE: u32 = 7;
pub const AUTO_RELEASE: u32 = 8;
pub const LOOKAHEAD: u32 = 9;
pub const CEILING: u32 = 10;
pub const SIDECHAIN: u32 = 20;
pub const SIDECHAIN_HIGH_PASS: u32 = 21;
pub const STEREO_LINK: u32 = 22;
pub const MAKEUP: u32 = 25;
pub const AUTO_MAKEUP: u32 = 26;
pub const MIX: u32 = 30;
pub const OUTPUT: u32 = 31;

/// Telemetry value slots.
/// Largest gain reduction of the last block, in dB.
pub const TELEMETRY_REDUCTION: usize = 0;
/// Detector level in dB, held with a 300 ms fall: the operating point.
pub const TELEMETRY_LEVEL: usize = 1;
/// 1 when External is selected and the last block carried a sidechain.
pub const TELEMETRY_SIDECHAIN: usize = 2;
pub const TELEMETRY_SAMPLE_RATE: usize = 3;

pub const MODES: &[&str] = &["Compressor", "Limiter", "Expander"];
/// Named after the circuit technology, never a manufacturer.
pub const CHARACTERS: &[&str] = &["Digital", "Tube", "Opto", "FET"];
pub const SIDECHAINS: &[&str] = &["Internal", "External"];

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

const fn log(
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
        ParamRange::Log { min, max },
        default,
    )
}

const fn choice(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    options: &'static [&'static str],
) -> ParamSpec {
    param(id, key, name, group, "", ParamRange::Choice(options), 0.0)
}

const fn toggle(id: u32, key: &'static str, name: &'static str, group: &'static str) -> ParamSpec {
    param(id, key, name, group, "", ParamRange::Toggle, 0.0)
}

pub static PARAMETERS: [ParamSpec; 17] = [
    choice(MODE, "mode", "Mode", "Mode", MODES),
    choice(CHARACTER, "character", "Character", "Mode", CHARACTERS),
    linear(
        THRESHOLD,
        "threshold",
        "Threshold",
        "Dynamics",
        "dB",
        (-60.0, 0.0),
        -18.0,
    ),
    log(RATIO, "ratio", "Ratio", "Dynamics", ":1", (1.0, 20.0), 4.0),
    linear(KNEE, "knee", "Knee", "Dynamics", "dB", (0.0, 24.0), 6.0),
    log(
        ATTACK,
        "attack",
        "Attack",
        "Dynamics",
        "ms",
        (0.02, 250.0),
        10.0,
    ),
    log(
        RELEASE,
        "release",
        "Release",
        "Dynamics",
        "ms",
        (5.0, 2500.0),
        120.0,
    ),
    toggle(AUTO_RELEASE, "auto_release", "Auto release", "Dynamics"),
    linear(
        LOOKAHEAD,
        "lookahead",
        "Lookahead",
        "Dynamics",
        "ms",
        (0.0, 10.0),
        0.0,
    ),
    linear(
        CEILING,
        "ceiling",
        "Ceiling",
        "Dynamics",
        "dB",
        (-12.0, 0.0),
        -0.3,
    ),
    choice(SIDECHAIN, "sidechain", "Sidechain", "Sidechain", SIDECHAINS),
    log(
        SIDECHAIN_HIGH_PASS,
        "sidechain.high_pass",
        "Sidechain high-pass",
        "Sidechain",
        "Hz",
        (20.0, 500.0),
        20.0,
    ),
    linear(
        STEREO_LINK,
        "stereo_link",
        "Stereo link",
        "Sidechain",
        "%",
        (0.0, 100.0),
        100.0,
    ),
    linear(
        MAKEUP,
        "makeup",
        "Makeup",
        "Output",
        "dB",
        (-12.0, 24.0),
        0.0,
    ),
    toggle(AUTO_MAKEUP, "auto_makeup", "Auto makeup", "Output"),
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
    id: "r1-compressor",
    name: "R1 Compressor",
    kind: DeviceKind::Effect,
    category: "Dynamics",
    parameters: &PARAMETERS,
    sample_slots: 0,
    sidechain_input: true,
    factory_presets: &[
        include_str!("../../presets/r1-compressor/default.rillpreset"),
        include_str!("../../presets/r1-compressor/master-bus.rillpreset"),
        include_str!("../../presets/r1-compressor/drum-bus-glue.rillpreset"),
        include_str!("../../presets/r1-compressor/vocal-leveler.rillpreset"),
        include_str!("../../presets/r1-compressor/bass-control.rillpreset"),
        include_str!("../../presets/r1-compressor/snare-snap.rillpreset"),
        include_str!("../../presets/r1-compressor/fet-smash.rillpreset"),
        include_str!("../../presets/r1-compressor/parallel-crush.rillpreset"),
        include_str!("../../presets/r1-compressor/sidechain-duck.rillpreset"),
        include_str!("../../presets/r1-compressor/brickwall-limiter.rillpreset"),
        include_str!("../../presets/r1-compressor/loud-limiter.rillpreset"),
        include_str!("../../presets/r1-compressor/noise-gate.rillpreset"),
        include_str!("../../presets/r1-compressor/gentle-expander.rillpreset"),
    ],
    legacy_labels: &[],
    fixed_on_roll: &[],
    latency: |values, sample_rate| latency(&Settings::from_normalized(values), sample_rate),
    create: || Box::new(Compressor::new()),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Compressor,
    Limiter,
    Expander,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Character {
    Digital,
    Tube,
    Opto,
    Fet,
}

/// Plain parameter values, decoded once per change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    pub character: Character,
    pub threshold_db: f32,
    pub ratio: f32,
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub auto_release: bool,
    pub lookahead_ms: f32,
    pub ceiling_db: f32,
    pub external: bool,
    pub high_pass_hz: f32,
    /// Off at the bottom of the range: the detector hears everything.
    pub high_pass_on: bool,
    /// 0–1.
    pub link: f32,
    pub makeup_db: f32,
    pub auto_makeup: bool,
    /// 0–1.
    pub mix: f32,
    pub output_db: f32,
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("compressor parameter");
            spec.to_plain(value(id))
        };
        let on = |id: u32| plain(id) >= 0.5;
        Self {
            mode: match plain(MODE) as usize {
                0 => Mode::Compressor,
                1 => Mode::Limiter,
                _ => Mode::Expander,
            },
            character: match plain(CHARACTER) as usize {
                0 => Character::Digital,
                1 => Character::Tube,
                2 => Character::Opto,
                _ => Character::Fet,
            },
            threshold_db: plain(THRESHOLD),
            ratio: plain(RATIO),
            knee_db: plain(KNEE),
            attack_ms: plain(ATTACK),
            release_ms: plain(RELEASE),
            auto_release: on(AUTO_RELEASE),
            lookahead_ms: plain(LOOKAHEAD),
            ceiling_db: plain(CEILING),
            external: on(SIDECHAIN),
            high_pass_hz: plain(SIDECHAIN_HIGH_PASS),
            high_pass_on: value(SIDECHAIN_HIGH_PASS) > 0.0,
            link: plain(STEREO_LINK) / 100.0,
            makeup_db: plain(MAKEUP),
            auto_makeup: on(AUTO_MAKEUP),
            mix: plain(MIX) / 100.0,
            output_db: plain(OUTPUT),
        }
    }
}

/// The detector and gain path a setting runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// Feed-forward, exact times.
    Digital,
    /// The level is taken after the gain element; Tube, Opto or FET.
    Feedback(Character),
    Limiter,
    Expander,
}

/// What the device actually uses for a setting, for readouts and drawing:
/// characters hold some values up, the limiter replaces others.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effective {
    pub path: Path,
    pub curve: GainComputer,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub lookahead_ms: f32,
    /// Compressor and Expander: gain after the gain element, with Auto
    /// makeup. Limiter: the gain into it.
    pub makeup_db: f32,
}

/// Fast release under Auto release; a slow cell does the rest.
const AUTO_RELEASE_MS: f32 = 60.0;
/// The limiter's lookahead when the parameter is at 0.
const LIMITER_LOOKAHEAD_MS: f32 = 5.0;
/// Two equal one-poles in series reach 63 % at this many of their time
/// constants: `(1 + x)·e^(−x) = e^(−1)`.
const TWO_STAGE: f32 = 2.146;

pub fn effective(settings: &Settings) -> Effective {
    let s = settings;
    let release_ms = if s.auto_release {
        AUTO_RELEASE_MS
    } else {
        s.release_ms
    };
    match s.mode {
        Mode::Limiter => Effective {
            path: Path::Limiter,
            curve: GainComputer {
                kind: CurveKind::Compress,
                threshold_db: s.ceiling_db,
                ratio: f32::INFINITY,
                knee_db: s.knee_db,
            },
            attack_ms: 0.0,
            release_ms,
            lookahead_ms: if s.lookahead_ms > 0.0 {
                s.lookahead_ms
            } else {
                LIMITER_LOOKAHEAD_MS
            },
            makeup_db: s.makeup_db,
        },
        Mode::Expander => Effective {
            path: Path::Expander,
            curve: GainComputer {
                kind: CurveKind::Expand,
                threshold_db: s.threshold_db,
                ratio: s.ratio,
                knee_db: s.knee_db,
            },
            attack_ms: s.attack_ms,
            release_ms: s.release_ms,
            lookahead_ms: s.lookahead_ms,
            makeup_db: s.makeup_db,
        },
        Mode::Compressor => {
            let curve = GainComputer {
                kind: CurveKind::Compress,
                threshold_db: s.threshold_db,
                ratio: s.ratio,
                // A variable-mu stage has no hard knee.
                knee_db: if s.character == Character::Tube {
                    s.knee_db.max(12.0)
                } else {
                    s.knee_db
                },
            };
            Effective {
                path: match s.character {
                    Character::Digital => Path::Digital,
                    character => Path::Feedback(character),
                },
                curve,
                attack_ms: match s.character {
                    Character::Tube => s.attack_ms.max(0.5),
                    // A photocell cannot follow faster.
                    Character::Opto => s.attack_ms.max(10.0),
                    Character::Digital | Character::Fet => s.attack_ms,
                },
                release_ms,
                // A feedback detector cannot look ahead of its own output.
                lookahead_ms: if s.character == Character::Digital {
                    s.lookahead_ms
                } else {
                    0.0
                },
                makeup_db: s.makeup_db
                    + if s.auto_makeup {
                        curve.auto_makeup_db()
                    } else {
                        0.0
                    },
            }
        }
    }
}

/// Weight, charge and release (ms) of the slow release cell.
fn slow_cell(settings: &Settings, effective: &Effective) -> (f32, f32, f32) {
    let release = effective.release_ms;
    match effective.path {
        Path::Feedback(Character::Opto) => (0.5, 1000.0, (10.0 * release).clamp(500.0, 5000.0)),
        Path::Feedback(Character::Fet) => (0.3, 400.0, (4.0 * release).clamp(200.0, 5000.0)),
        Path::Expander => (0.0, 0.0, 0.0),
        _ if settings.auto_release => (0.5, 300.0, 1500.0),
        _ => (0.0, 0.0, 0.0),
    }
}

fn lookahead_samples(effective: &Effective, sample_rate: f32) -> u32 {
    (effective.lookahead_ms * 0.001 * sample_rate.max(8000.0)).round() as u32
}

/// Latency in samples: the lookahead, plus the true-peak detector's delay in
/// Limiter mode. Zero for a compressor without lookahead.
pub fn latency(settings: &Settings, sample_rate: f32) -> u32 {
    let effective = effective(settings);
    let lookahead = lookahead_samples(&effective, sample_rate);
    match effective.path {
        Path::Limiter => lookahead + TruePeak4x::DELAY,
        _ => lookahead,
    }
}

/// Whether a parameter does anything in the current mode; panels disable the
/// ones that do not.
pub fn applies(id: u32, settings: &Settings) -> bool {
    let s = settings;
    let compressor = s.mode == Mode::Compressor;
    let limiter = s.mode == Mode::Limiter;
    match id {
        CHARACTER | AUTO_MAKEUP => compressor,
        THRESHOLD | RATIO | ATTACK => !limiter,
        RELEASE => s.mode == Mode::Expander || !s.auto_release,
        AUTO_RELEASE => s.mode != Mode::Expander,
        LOOKAHEAD => !compressor || s.character == Character::Digital,
        CEILING => limiter,
        SIDECHAIN | SIDECHAIN_HIGH_PASS => !limiter,
        _ => true,
    }
}

/// The static curve as drawn: output level in dB for each input level,
/// without makeup, mix or output. The same curve the detector uses.
pub fn transfer_db(settings: &Settings, input_db: &[f32], out: &mut [f32]) {
    let curve = effective(settings).curve;
    for (input, out) in input_db.iter().zip(out) {
        *out = curve.output_db(*input);
    }
}

/// What Auto makeup adds in Compressor mode, whether or not it is on.
pub fn auto_makeup_db(settings: &Settings) -> f32 {
    match settings.mode {
        Mode::Compressor => effective(settings).curve.auto_makeup_db(),
        _ => 0.0,
    }
}

const TICK: u32 = 32;
const SWITCH_SECONDS: f32 = 0.010;
const SLOW_RAMP_SECONDS: f32 = 0.020;
const HIGH_PASS_RAMP_SECONDS: f32 = 0.010;
/// Levels are floored here before the logarithm: −120 dB.
const FLOOR: f32 = 1e-6;
/// Output is held to ±36 dBFS: makeup and output gain together reach
/// +48 dB.
const LIMIT: f32 = 64.0;
/// Reduction at which the Tube and FET colour is fully there.
const COLOUR_FULL_DB: f32 = 6.0;
/// Samples without reduction after which the colour stage is cleared and
/// skipped: more than its oversampler remembers.
const COLOUR_IDLE: u32 = 64;

#[inline]
fn to_db(level: f32) -> f32 {
    20.0 * level.max(FLOOR).log10()
}

#[inline]
fn reduction_gain(reduction_db: f32) -> f32 {
    if reduction_db > 0.0 {
        db_to_gain(-reduction_db)
    } else {
        1.0
    }
}

/// What must be faded when it changes: the path, the detector source and
/// the delay tap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Structure {
    path: Path,
    external: bool,
    /// Delay of the audio and dry paths in samples: the latency.
    tap: usize,
}

impl Structure {
    fn new(settings: &Settings, effective: &Effective, sample_rate: f32) -> Self {
        Self {
            path: effective.path,
            external: settings.external && effective.path != Path::Limiter,
            tap: latency(settings, sample_rate) as usize,
        }
    }

    /// The limiter's lookahead without its detector delay.
    fn lookahead(&self) -> usize {
        self.tap.saturating_sub(TruePeak4x::DELAY as usize)
    }
}

/// Detector state of one channel.
#[derive(Clone, Debug, Default)]
struct Detector {
    high_pass: Svf,
    true_peak: TruePeak4x,
    /// The expander's peak follower, in linear amplitude: instant rise,
    /// Release fall.
    level: Ballistics,
    /// Reduction in dB after attack and release.
    fast: Ballistics,
    /// Tube's second stage.
    second: Ballistics,
    slow: ProgramRelease,
    hold: SlidingMax,
    box1: BoxSmoother,
    box2: BoxSmoother,
}

impl Detector {
    /// Starts a path from `reduction`, so the gain continues from where the
    /// previous path left it.
    fn enter(&mut self, structure: &Structure, reduction: f32) {
        self.fast.set(reduction);
        self.second.set(reduction);
        self.slow.reset();
        if structure.path == Path::Limiter {
            let lookahead = structure.lookahead();
            // Two boxes spanning the lookahead together: an S-shaped attack
            // that ends exactly at the delayed peak.
            let first = 2 * lookahead / 3;
            self.true_peak.reset();
            self.hold.set_window(lookahead + 3, reduction);
            self.box1.set_length(first + 1, reduction);
            self.box2.set_length(lookahead - first + 1, reduction);
        }
    }

    /// Gain reduction in dB for the linked detector magnitude.
    #[inline]
    fn process(
        &mut self,
        structure: &Structure,
        curve: &GainComputer,
        magnitude: f32,
        feedback_rise: f32,
    ) -> f32 {
        // Always running, so entering Expander mode finds the level.
        let level = self.level.process(magnitude);
        match structure.path {
            // The reduction of an expander swings to the floor at every
            // zero crossing, so the curve reads a held peak: a steady tone
            // sits on the drawn curve whatever the times. Attack then slows
            // only the opening, in linear gain: 63 % of the amplitude after
            // the set time, from however deep the gate was.
            Path::Expander => {
                let target = curve.reduction_db(to_db(level));
                let previous = self.fast.value();
                let reduction = if target < previous {
                    let closed = reduction_gain(previous);
                    let gain = closed + self.fast.fall * (reduction_gain(target) - closed);
                    flush(-to_db(gain))
                } else {
                    target
                };
                self.fast.set(reduction);
                reduction
            }
            Path::Digital => {
                let fast = self.fast.process(curve.reduction_db(to_db(magnitude)));
                self.slow.process(fast)
            }
            Path::Feedback(character) => {
                let level_db = to_db(magnitude);
                let fast = if structure.external {
                    // No loop to close on another signal.
                    self.fast.process(curve.reduction_db(level_db))
                } else {
                    let previous = self.fast.value();
                    // The solve's residual is negative at the previous
                    // value exactly when the reduction is about to rise.
                    let k = if curve.feedback_reduction_db(level_db - previous) > previous {
                        feedback_rise
                    } else {
                        self.fast.fall
                    };
                    let solved = flush(curve.solve_feedback(level_db, (1.0 - k) * previous, k));
                    self.fast.set(solved);
                    solved
                };
                let reduction = self.slow.process(fast);
                if character == Character::Tube {
                    self.second.process(reduction)
                } else {
                    reduction
                }
            }
            Path::Limiter => {
                let target = curve.reduction_db(to_db(magnitude));
                let released = self.slow.process(self.fast.process(target));
                let held = self.hold.process(released);
                self.box2.process(self.box1.process(held))
            }
        }
    }
}

/// The oversampled residual of the Tube and FET characters.
#[derive(Clone, Debug)]
struct Colour {
    oversampler: Oversampler2x,
    dc: Shelf1,
}

impl Colour {
    #[inline]
    fn residual(&mut self, input: f32, character: Character) -> f32 {
        // A square or a cube of a band-limited signal folds nothing below
        // the base Nyquist at 2×, so the shapes need no antialiasing.
        let shaped = self.oversampler.upsample(flush(input)).map(|sample| {
            let x = f64::from(sample);
            let y = if character == Character::Tube {
                BoundedSquare { knee: 1.0 }.value(x) / 16.0
            } else {
                BoundedCubic { knee: 1.0 }.value(x)
            };
            flush(y as f32)
        });
        let residual = self.oversampler.downsample(shaped);
        if character == Character::Tube {
            // A square has a mean.
            self.dc.process(residual)
        } else {
            residual
        }
    }

    fn reset(&mut self) {
        self.oversampler.reset();
        self.dc.reset();
    }
}

pub struct Compressor {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    effective: Effective,
    dirty: bool,
    sample_rate: f32,
    detectors: [Detector; 2],
    colours: [Colour; 2],
    colour_idle: u32,
    /// The signal the gain is applied to, and the untouched input.
    audio: [DelayLine; 2],
    dry: [DelayLine; 2],
    active: Structure,
    old_tap: usize,
    /// 0 → 1 while a change of path or delay is faded in.
    switch: LinearRamp,
    /// Gains when the switch began.
    frozen: [f32; 2],
    gain: [f32; 2],
    reduction: [f32; 2],
    threshold: Smoothed,
    ratio: Smoothed,
    knee: Smoothed,
    /// The linear ceiling for the threshold it was computed from.
    ceiling: (f32, f32),
    feedback_rise: f32,
    /// log2 of the sidechain high-pass frequency.
    hp_cutoff: LinearRamp,
    hp_applied: f32,
    hp_g: LinearRamp,
    /// 1 while the sidechain high-pass is in, 0 when it is off.
    hp_mix: LinearRamp,
    link: Smoothed,
    /// Gain into the limiter.
    drive: Smoothed,
    makeup: Smoothed,
    mix: Smoothed,
    output: Smoothed,
    tick: u32,
    history: HistoryMeter,
    held_level: f32,
    telemetry: Arc<Telemetry>,
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

fn decode(values: &[f32; PARAMETERS.len()]) -> Settings {
    Settings::from_normalized(|id| {
        values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
    })
}

impl Compressor {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let settings = decode(&values);
        let effective = effective(&settings);
        let colour = Colour {
            oversampler: Oversampler2x::new(),
            dc: Shelf1::default(),
        };
        let mut compressor = Self {
            values,
            settings,
            effective,
            dirty: false,
            sample_rate: 48_000.0,
            detectors: Default::default(),
            colours: [colour.clone(), colour],
            colour_idle: COLOUR_IDLE,
            audio: Default::default(),
            dry: Default::default(),
            active: Structure::new(&settings, &effective, 48_000.0),
            old_tap: 0,
            switch: LinearRamp::new(1.0),
            frozen: [1.0; 2],
            gain: [1.0; 2],
            reduction: [0.0; 2],
            threshold: Smoothed::new(effective.curve.threshold_db),
            ratio: Smoothed::new(settings.ratio),
            knee: Smoothed::new(effective.curve.knee_db),
            ceiling: (0.0, 1.0),
            feedback_rise: 1.0,
            hp_cutoff: LinearRamp::new(settings.high_pass_hz.log2()),
            hp_applied: 0.0,
            hp_g: LinearRamp::new(0.0),
            hp_mix: LinearRamp::new(0.0),
            link: Smoothed::new(settings.link),
            drive: Smoothed::new(1.0),
            makeup: Smoothed::new(1.0),
            mix: Smoothed::new(settings.mix),
            output: Smoothed::new(db_to_gain(settings.output_db)),
            tick: 0,
            history: HistoryMeter::default(),
            held_level: 0.0,
            telemetry: Arc::new(Telemetry::default()),
        };
        compressor.prepare(48_000.0, 0);
        compressor
    }

    fn samples(&self, seconds: f32) -> u32 {
        (seconds * self.sample_rate).ceil() as u32
    }

    /// Takes the parameters as they are set. Only called while no switch is
    /// fading, so a change that arrives during one waits for it to end.
    fn update_settings(&mut self) {
        let settings = decode(&self.values);
        let effective = effective(&settings);
        self.settings = settings;
        self.effective = effective;
        self.dirty = false;
        let rate = self.sample_rate;
        let limiter = effective.path == Path::Limiter;

        let (attack, release) = match effective.path {
            Path::Feedback(Character::Tube) => (
                effective.attack_ms / TWO_STAGE,
                effective.release_ms / TWO_STAGE,
            ),
            _ => (effective.attack_ms, effective.release_ms),
        };
        let (weight, charge, slow_release) = slow_cell(&settings, &effective);
        for detector in &mut self.detectors {
            detector.level.set_times(0.0, effective.release_ms, rate);
            if effective.path == Path::Expander {
                // Closing follows the falling level; opening takes Attack.
                detector.fast.set_times(0.0, attack, rate);
            } else {
                detector.fast.set_times(attack, release, rate);
            }
            detector.second.set_times(attack, release, rate);
            detector.slow.configure(weight, charge, slow_release, rate);
        }
        // A closed loop multiplies the detector's coefficient by its loop
        // gain, about the ratio above the knee. Dividing the rise by it
        // makes the attack above the knee the knob's time; inside the knee
        // the loop is slower, and it still grabs harder the further the
        // signal overshoots.
        let rise = self.detectors[0].fast.rise;
        self.feedback_rise = rise / (settings.ratio - rise * (settings.ratio - 1.0));

        self.threshold.set_target(effective.curve.threshold_db);
        self.ratio.set_target(settings.ratio);
        self.knee.set_target(effective.curve.knee_db);
        self.hp_cutoff.set_target(
            settings.high_pass_hz.log2(),
            self.samples(SLOW_RAMP_SECONDS),
        );
        let hp_mix = if settings.high_pass_on { 1.0 } else { 0.0 };
        if self.hp_mix.target() != hp_mix {
            self.hp_mix
                .set_target(hp_mix, self.samples(HIGH_PASS_RAMP_SECONDS));
        }
        self.link.set_target(settings.link);
        let makeup = db_to_gain(effective.makeup_db);
        // The limiter is driven by the makeup gain; after it, the gain
        // would break the ceiling.
        self.drive.set_target(if limiter { makeup } else { 1.0 });
        self.makeup.set_target(if limiter { 1.0 } else { makeup });
        self.mix.set_target(settings.mix);
        self.output.set_target(db_to_gain(settings.output_db));

        let structure = Structure::new(&settings, &effective, rate);
        if structure != self.active {
            self.start_switch(structure);
        }
    }

    fn start_switch(&mut self, structure: Structure) {
        let rebuild = structure.path != self.active.path
            || structure.external != self.active.external
            || structure.path == Path::Limiter;
        self.frozen = self.gain;
        self.old_tap = self.active.tap;
        self.active = structure;
        if rebuild {
            // The curve changes its meaning with the path (a threshold
            // becomes a ceiling); the fade carries the gain across.
            self.threshold.snap();
            self.ratio.snap();
            self.knee.snap();
            for (detector, reduction) in self.detectors.iter_mut().zip(self.reduction) {
                detector.enter(&structure, reduction);
            }
            for colour in &mut self.colours {
                colour.reset();
            }
            self.colour_idle = COLOUR_IDLE;
        }
        self.switch = LinearRamp::new(0.0);
        self.switch.set_target(1.0, self.samples(SWITCH_SECONDS));
    }

    fn control_tick(&mut self) {
        let cutoff = self.hp_cutoff.value();
        if cutoff != self.hp_applied {
            self.hp_applied = cutoff;
            self.hp_g
                .set_target(prewarp(cutoff.exp2(), self.sample_rate), TICK);
        }
    }
}

impl Device for Compressor {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        let rate = self.sample_rate;
        // The longest lookahead, the limiter's detector delay and a margin.
        let longest = (0.010 * rate).ceil() as usize + 24;
        for line in self.audio.iter_mut().chain(&mut self.dry) {
            line.allocate(longest);
        }
        for detector in &mut self.detectors {
            detector.hold.allocate(longest);
            detector.box1.allocate(longest);
            detector.box2.allocate(longest);
        }
        for colour in &mut self.colours {
            colour.dc = Shelf1::high_pass(5.0, rate);
        }
        for smoothed in [
            &mut self.threshold,
            &mut self.ratio,
            &mut self.knee,
            &mut self.link,
            &mut self.drive,
            &mut self.makeup,
            &mut self.mix,
            &mut self.output,
        ] {
            smoothed.set_time(5.0, rate);
        }
        self.history.prepare(rate);
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
        let StereoIo {
            left,
            right,
            sidechain,
        } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        self.telemetry.record_input(left, right);
        // One bad sample must not poison the detector or the delay lines:
        // squares and gains of a huge finite one overflow.
        let clean = |x: f32| {
            if x.is_finite() {
                x.clamp(-LIMIT, LIMIT)
            } else {
                0.0
            }
        };
        let mut block_reduction = 0.0_f32;
        let mut block_level = 0.0_f32;
        for index in 0..frames {
            if self.dirty && self.switch.settled() {
                self.update_settings();
            }
            if self.tick == 0 {
                self.control_tick();
            }
            self.tick = (self.tick + 1) % TICK;

            let structure = self.active;
            let limiter = structure.path == Path::Limiter;
            let input = [clean(left[index]), clean(right[index])];
            let drive = self.drive.next_value();
            let audio = input.map(|x| x * drive);
            for channel in 0..2 {
                self.dry[channel].write(input[channel]);
                self.audio[channel].write(audio[channel]);
            }

            self.hp_cutoff.next();
            let hp_g = self.hp_g.next();
            let hp_mix = self.hp_mix.next();
            let source = if structure.external {
                // A sidechain shorter than the block is silence beyond it.
                let at = |channel: &[f32]| channel.get(index).copied().map_or(0.0, clean);
                sidechain.map_or([0.0; 2], |(left, right)| [at(left), at(right)])
            } else {
                audio
            };
            let mut magnitude = [0.0_f32; 2];
            for (channel, detector) in self.detectors.iter_mut().enumerate() {
                magnitude[channel] = if limiter {
                    // Always the signal itself, unfiltered: anything else
                    // could not guarantee the ceiling.
                    detector.true_peak.process(audio[channel])
                } else {
                    let mut x = source[channel];
                    if hp_mix > 0.0 {
                        let high = detector
                            .high_pass
                            .process(x, hp_g, std::f32::consts::SQRT_2)
                            .high;
                        x -= hp_mix * (x - high);
                    }
                    x.abs()
                };
            }
            // The linked value is never below the channel's own, so the
            // limiter's guarantee survives a partial link.
            let link = self.link.next_value();
            let link_to = |own: [f32; 2]| {
                let loudest = own[0].max(own[1]);
                own.map(|own| {
                    if link == 1.0 {
                        loudest
                    } else {
                        own + link * (loudest - own)
                    }
                })
            };
            let linked = link_to(magnitude);
            block_level = block_level.max(linked[0]).max(linked[1]);

            let curve = GainComputer {
                kind: self.effective.curve.kind,
                threshold_db: self.threshold.next_value(),
                ratio: if limiter {
                    f32::INFINITY
                } else {
                    self.ratio.next_value()
                },
                knee_db: self.knee.next_value(),
            };
            let mut reduction = [0.0_f32; 2];
            for (channel, detector) in self.detectors.iter_mut().enumerate() {
                reduction[channel] =
                    detector.process(&structure, &curve, linked[channel], self.feedback_rise);
            }

            let fade = self.switch.next();
            let fading = fade < 1.0;
            let tap = |line: &DelayLine| {
                let new = line.tap(structure.tap);
                if fading {
                    let old = line.tap(self.old_tap);
                    old + fade * (new - old)
                } else {
                    new
                }
            };
            let delayed = [tap(&self.audio[0]), tap(&self.audio[1])];
            let dry = [tap(&self.dry[0]), tap(&self.dry[1])];

            if limiter {
                // The reduction the delayed sample itself needs: the
                // sample-peak bound does not depend on rounding in the
                // smoothers.
                let needed = link_to(delayed.map(f32::abs));
                for channel in 0..2 {
                    reduction[channel] =
                        reduction[channel].max(curve.reduction_db(to_db(needed[channel])));
                }
                if self.ceiling.0 != curve.threshold_db {
                    self.ceiling = (curve.threshold_db, db_to_gain(curve.threshold_db));
                }
            }

            let coloured = match structure.path {
                Path::Feedback(character @ (Character::Tube | Character::Fet)) => Some(character),
                _ => None,
            };
            if coloured.is_some() {
                if reduction[0].max(reduction[1]) > 0.0 {
                    self.colour_idle = 0;
                } else if self.colour_idle < COLOUR_IDLE {
                    self.colour_idle += 1;
                    if self.colour_idle == COLOUR_IDLE {
                        for colour in &mut self.colours {
                            colour.reset();
                        }
                    }
                }
            }

            let makeup = self.makeup.next_value();
            let mix = self.mix.next_value();
            let output = self.output.next_value();
            let mut peaks = [0.0_f32; 2];
            for (channel, sample) in [&mut *left, &mut *right].into_iter().enumerate() {
                let mut gain = reduction_gain(reduction[channel]);
                if fading {
                    gain = self.frozen[channel] + fade * (gain - self.frozen[channel]);
                }
                self.gain[channel] = gain;
                let mut wet = delayed[channel] * gain;
                if let Some(character) = coloured
                    && self.colour_idle < COLOUR_IDLE
                {
                    // Grows with the reduction and is absent at rest. The
                    // stage is driven by the level into the gain element, so
                    // deeper reduction does not thin the colour out again.
                    // Only the residual passes the oversampler; bounded at a
                    // few percent, arriving 31 samples late it cannot comb.
                    // A character that was just switched in fades in.
                    let amount = (reduction[channel] / COLOUR_FULL_DB).min(1.0) * fade;
                    wet +=
                        amount * gain * self.colours[channel].residual(delayed[channel], character);
                }
                wet *= makeup;
                if limiter {
                    // Rounding, and the gain of a fade, only.
                    wet = wet.clamp(-self.ceiling.1, self.ceiling.1);
                }
                let dry = dry[channel];
                let out = ((dry + mix * (wet - dry)) * output).clamp(-LIMIT, LIMIT);
                peaks[0] = peaks[0].max(input[channel].abs());
                peaks[1] = peaks[1].max(out.abs());
                sample[index] = out;
            }
            self.reduction = reduction;
            let deepest = reduction[0].max(reduction[1]);
            block_reduction = block_reduction.max(deepest);
            self.history
                .add(peaks[0], peaks[1], deepest, &self.telemetry);
        }
        // Held with a 300 ms fall, so the panel's dot does not flicker.
        let fall = (-(frames as f32) / (0.3 * self.sample_rate)).exp();
        self.held_level = flush((self.held_level * fall).max(block_level));
        self.telemetry
            .set_value(TELEMETRY_REDUCTION, block_reduction);
        self.telemetry
            .set_value(TELEMETRY_LEVEL, to_db(self.held_level));
        let carried = self.active.external && sidechain.is_some();
        self.telemetry
            .set_value(TELEMETRY_SIDECHAIN, if carried { 1.0 } else { 0.0 });
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        self.switch = LinearRamp::new(1.0);
        self.update_settings();
        // Whatever switch the update began: start settled instead.
        self.switch = LinearRamp::new(1.0);
        self.old_tap = self.active.tap;
        self.frozen = [1.0; 2];
        self.gain = [1.0; 2];
        self.reduction = [0.0; 2];
        for smoothed in [
            &mut self.threshold,
            &mut self.ratio,
            &mut self.knee,
            &mut self.link,
            &mut self.drive,
            &mut self.makeup,
            &mut self.mix,
            &mut self.output,
        ] {
            smoothed.snap();
        }
        self.hp_cutoff.snap();
        self.hp_mix.snap();
        self.hp_applied = self.hp_cutoff.value();
        self.hp_g = LinearRamp::new(prewarp(self.settings.high_pass_hz, self.sample_rate));
        let structure = self.active;
        for detector in &mut self.detectors {
            detector.high_pass.reset();
            detector.level.reset();
            detector.enter(&structure, 0.0);
            detector.true_peak.reset();
        }
        for colour in &mut self.colours {
            colour.reset();
        }
        self.colour_idle = COLOUR_IDLE;
        for line in self.audio.iter_mut().chain(&mut self.dry) {
            line.reset();
        }
        self.history.reset();
        self.tick = 0;
        self.held_level = 0.0;
    }

    fn latency_samples(&self) -> u32 {
        // From the parameters as they are set, not as the audio thread last
        // saw them: the engine asks while it builds the graph.
        latency(&decode(&self.values), self.sample_rate)
    }

    fn tail_samples(&self) -> u32 {
        // The lookahead, and the DC blocker of Tube's colour.
        self.latency_samples() + (0.5 * self.sample_rate) as u32
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}
