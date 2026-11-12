//! R1 Spacer: an echo and a reverb in one device, in series or in parallel,
//! each ducked by the dry input by its own amount. Both sections are shared
//! kernels; this module maps the parameters, routes, gates, ducks and mixes.
//! See `docs/R1_DEVICES.md#r1-spacer`.
use super::param;
use crate::dsp::echo::{self, EchoKernel, EchoSettings};
use crate::dsp::reverb::{self, EARLY_TAPS, ReverbKernel, ReverbSettings};
use crate::dsp::{
    Ballistics, CurveKind, GainComputer, LinearRamp, SlidingMax, Smoothed, db_to_gain,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, ParamRange, ParamSpec, ProcessContext, StereoIo,
    Telemetry,
};
use std::sync::Arc;

pub use crate::dsp::{EchoModel, EchoStereo, ReverbModel, TapPattern};

pub const ECHO_ON: u32 = 1;
pub const ECHO_MODEL: u32 = 2;
pub const ECHO_SYNC: u32 = 3;
pub const ECHO_TIME: u32 = 4;
pub const ECHO_DIVISION: u32 = 5;
pub const ECHO_FEEDBACK: u32 = 6;
pub const ECHO_STEREO: u32 = 7;
pub const ECHO_LOW_CUT: u32 = 8;
pub const ECHO_COLOR: u32 = 9;
pub const ECHO_WOBBLE: u32 = 10;
pub const ECHO_LEVEL: u32 = 11;
pub const ECHO_TAPS: u32 = 12;
pub const ECHO_DIFFUSION: u32 = 13;
pub const ECHO_DUCK: u32 = 14;
pub const ROUTING: u32 = 20;
pub const MIX: u32 = 30;
pub const OUTPUT: u32 = 31;
pub const REVERB_ON: u32 = 40;
pub const REVERB_MODEL: u32 = 41;
pub const PRE_DELAY: u32 = 42;
pub const SIZE: u32 = 43;
pub const DECAY: u32 = 44;
pub const DAMPING: u32 = 45;
pub const REVERB_LOW_CUT: u32 = 46;
pub const REVERB_COLOR: u32 = 47;
pub const DIFFUSION: u32 = 48;
pub const MOVEMENT: u32 = 49;
pub const WIDTH: u32 = 50;
pub const REVERB_LEVEL: u32 = 51;
pub const SEED: u32 = 52;
pub const REVERB_DUCK: u32 = 53;

/// Telemetry value slots: the duck's reduction now in dB at full depth (a
/// section is reduced by this times its Duck), the echo time in effect in
/// seconds (after sync), and the tempo it was derived from.
pub const TELEMETRY_DUCK: usize = 0;
pub const TELEMETRY_TIME: usize = 1;
pub const TELEMETRY_TEMPO: usize = 2;
pub const TELEMETRY_SAMPLE_RATE: usize = 3;

pub const ECHO_MODELS: &[&str] = &["Digital", "BBD", "Tape"];
pub const ECHO_TAP_PATTERNS: &[&str] = &[
    "Single", "Double", "Triple", "Quad", "Dotted", "Swing", "Gallop", "Golden",
];
pub const ECHO_STEREOS: &[&str] = &["Mono", "Ping-pong", "Wide"];
/// Shortest first; D is dotted, T a triplet.
pub const DIVISIONS: &[&str] = &[
    "1/32 T", "1/32", "1/16 T", "1/32 D", "1/16", "1/8 T", "1/16 D", "1/8", "1/4 T", "1/8 D",
    "1/4", "1/2 T", "1/4 D", "1/2", "1/1 T", "1/2 D", "1/1", "1/1 D",
];
pub const ROUTINGS: &[&str] = &["Echo → Reverb", "Parallel"];
pub const REVERB_MODELS: &[&str] = &["Studio", "Room", "Hall"];
pub const SEEDS: (i32, i32) = (1, 32);

/// A level parameter at its minimum is silence, not −60 dB.
pub const SILENT_DB: f32 = -60.0;

/// Length of a division in beats (quarter notes).
pub fn division_beats(index: usize) -> f64 {
    let label = DIVISIONS[index.min(DIVISIONS.len() - 1)];
    let (note, feel) = label.split_at(label.find(' ').unwrap_or(label.len()));
    let denominator: f64 = note[2..].parse().expect("a division is 1/n");
    let scale = match feel.trim() {
        "D" => 1.5,
        "T" => 2.0 / 3.0,
        _ => 1.0,
    };
    4.0 / denominator * scale
}

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

const fn toggle(
    id: u32,
    key: &'static str,
    name: &'static str,
    group: &'static str,
    default: f32,
) -> ParamSpec {
    param(id, key, name, group, "", ParamRange::Toggle, default)
}

const PERCENT: (f32, f32) = (0.0, 100.0);
const BIPOLAR: (f32, f32) = (-100.0, 100.0);
const LEVEL: (f32, f32) = (SILENT_DB, 0.0);

/// In signal order: Echo, Reverb, Output.
pub static PARAMETERS: [ParamSpec; 31] = [
    toggle(ECHO_ON, "echo.on", "Echo on", "Echo", 1.0),
    choice(
        ECHO_MODEL,
        "echo.model",
        "Echo model",
        "Echo",
        ECHO_MODELS,
        0.0,
    ),
    toggle(ECHO_SYNC, "echo.sync", "Echo sync", "Echo", 1.0),
    log(
        ECHO_TIME,
        "echo.time",
        "Echo time",
        "Echo",
        "ms",
        (1.0, 2000.0),
        350.0,
    ),
    choice(
        ECHO_DIVISION,
        "echo.division",
        "Echo division",
        "Echo",
        DIVISIONS,
        9.0,
    ),
    linear(
        ECHO_FEEDBACK,
        "echo.feedback",
        "Echo feedback",
        "Echo",
        "%",
        PERCENT,
        35.0,
    ),
    choice(
        ECHO_TAPS,
        "echo.taps",
        "Echo taps",
        "Echo",
        ECHO_TAP_PATTERNS,
        0.0,
    ),
    choice(
        ECHO_STEREO,
        "echo.stereo",
        "Echo stereo",
        "Echo",
        ECHO_STEREOS,
        1.0,
    ),
    linear(
        ECHO_DIFFUSION,
        "echo.diffusion",
        "Echo diffusion",
        "Echo",
        "%",
        PERCENT,
        0.0,
    ),
    linear(
        ECHO_COLOR,
        "echo.color",
        "Echo color",
        "Echo",
        "%",
        BIPOLAR,
        0.0,
    ),
    linear(
        ECHO_WOBBLE,
        "echo.wobble",
        "Echo wobble",
        "Echo",
        "%",
        PERCENT,
        0.0,
    ),
    log(
        ECHO_LOW_CUT,
        "echo.low_cut",
        "Echo low cut",
        "Echo",
        "Hz",
        (20.0, 2000.0),
        120.0,
    ),
    linear(
        ECHO_DUCK,
        "echo.duck",
        "Echo duck",
        "Echo",
        "%",
        PERCENT,
        0.0,
    ),
    linear(
        ECHO_LEVEL,
        "echo.level",
        "Echo level",
        "Echo",
        "dB",
        LEVEL,
        -6.0,
    ),
    toggle(REVERB_ON, "reverb.on", "Reverb on", "Reverb", 1.0),
    choice(
        REVERB_MODEL,
        "reverb.model",
        "Reverb model",
        "Reverb",
        REVERB_MODELS,
        2.0,
    ),
    linear(
        PRE_DELAY,
        "reverb.pre_delay",
        "Pre-delay",
        "Reverb",
        "ms",
        (0.0, 250.0),
        15.0,
    ),
    linear(SIZE, "reverb.size", "Size", "Reverb", "%", PERCENT, 50.0),
    log(
        DECAY,
        "reverb.decay",
        "Decay",
        "Reverb",
        "s",
        (0.1, 20.0),
        1.8,
    ),
    linear(
        DAMPING,
        "reverb.damping",
        "Damping",
        "Reverb",
        "%",
        PERCENT,
        40.0,
    ),
    linear(
        DIFFUSION,
        "reverb.diffusion",
        "Diffusion",
        "Reverb",
        "%",
        PERCENT,
        80.0,
    ),
    linear(
        MOVEMENT,
        "reverb.movement",
        "Movement",
        "Reverb",
        "%",
        PERCENT,
        25.0,
    ),
    param(
        SEED,
        "reverb.seed",
        "Seed",
        "Reverb",
        "",
        ParamRange::Integer {
            min: SEEDS.0,
            max: SEEDS.1,
        },
        1.0,
    ),
    linear(
        REVERB_COLOR,
        "reverb.color",
        "Reverb color",
        "Reverb",
        "%",
        BIPOLAR,
        0.0,
    ),
    log(
        REVERB_LOW_CUT,
        "reverb.low_cut",
        "Reverb low cut",
        "Reverb",
        "Hz",
        (20.0, 1000.0),
        100.0,
    ),
    linear(
        WIDTH,
        "reverb.width",
        "Width",
        "Reverb",
        "%",
        (0.0, 150.0),
        100.0,
    ),
    linear(
        REVERB_DUCK,
        "reverb.duck",
        "Reverb duck",
        "Reverb",
        "%",
        PERCENT,
        0.0,
    ),
    linear(
        REVERB_LEVEL,
        "reverb.level",
        "Reverb level",
        "Reverb",
        "dB",
        LEVEL,
        -9.0,
    ),
    choice(ROUTING, "routing", "Routing", "Output", ROUTINGS, 0.0),
    linear(MIX, "mix", "Mix", "Output", "%", PERCENT, 30.0),
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
    id: "r1-spacer",
    name: "R1 Spacer",
    kind: DeviceKind::Effect,
    category: "Space",
    parameters: &PARAMETERS,
    sample_slots: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-spacer/default.rillpreset"),
        include_str!("../../presets/r1-spacer/slapback.rillpreset"),
        include_str!("../../presets/r1-spacer/ping-pong-eighths.rillpreset"),
        include_str!("../../presets/r1-spacer/tape-echo.rillpreset"),
        include_str!("../../presets/r1-spacer/multi-head.rillpreset"),
        include_str!("../../presets/r1-spacer/bbd-dub.rillpreset"),
        include_str!("../../presets/r1-spacer/wide-doubler.rillpreset"),
        include_str!("../../presets/r1-spacer/diffused-repeats.rillpreset"),
        include_str!("../../presets/r1-spacer/vocal-booth.rillpreset"),
        include_str!("../../presets/r1-spacer/drum-room.rillpreset"),
        include_str!("../../presets/r1-spacer/live-room.rillpreset"),
        include_str!("../../presets/r1-spacer/dark-chamber.rillpreset"),
        include_str!("../../presets/r1-spacer/large-hall.rillpreset"),
        include_str!("../../presets/r1-spacer/cathedral.rillpreset"),
        include_str!("../../presets/r1-spacer/echo-into-hall.rillpreset"),
        include_str!("../../presets/r1-spacer/ducked-space.rillpreset"),
        include_str!("../../presets/r1-spacer/parallel-wash.rillpreset"),
        include_str!("../../presets/r1-spacer/send-hall.rillpreset"),
        include_str!("../../presets/r1-spacer/send-echo.rillpreset"),
    ],
    legacy_labels: &[],
    fixed_on_roll: &[],
    latency: |_, _| 0,
    create: || Box::new(Spacer::new()),
};

/// Plain parameter values, decoded once per change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub echo_on: bool,
    pub echo_sync: bool,
    /// The Time knob; see [`echo_seconds`] for the time in effect.
    pub echo_time_seconds: f32,
    /// Index into [`DIVISIONS`].
    pub echo_division: usize,
    /// What the echo is given but for its time; see [`Settings::echo`].
    pub echo: EchoSettings,
    /// 0–1: how much of the duck the echo takes.
    pub echo_duck: f32,
    /// Linear gain; 0 at the bottom of the range.
    pub echo_level: f32,
    pub reverb_on: bool,
    pub reverb: ReverbSettings,
    /// 0–1.
    pub reverb_duck: f32,
    /// Linear gain; 0 at the bottom of the range.
    pub reverb_level: f32,
    /// The echo feeds the reverb; otherwise both hear only the input.
    pub series: bool,
    /// 0–1.
    pub mix: f32,
    pub output_db: f32,
}

fn level_gain(db: f32) -> f32 {
    if db <= SILENT_DB { 0.0 } else { db_to_gain(db) }
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("spacer parameter");
            spec.to_plain(value(id))
        };
        let echo_time_seconds = plain(ECHO_TIME) / 1000.0;
        Self {
            echo_on: plain(ECHO_ON) >= 0.5,
            echo_sync: plain(ECHO_SYNC) >= 0.5,
            echo_time_seconds,
            echo_division: (plain(ECHO_DIVISION) as usize).min(DIVISIONS.len() - 1),
            echo: EchoSettings {
                model: match plain(ECHO_MODEL) as usize {
                    0 => EchoModel::Digital,
                    1 => EchoModel::Bbd,
                    _ => EchoModel::Tape,
                },
                seconds: echo_time_seconds,
                feedback: plain(ECHO_FEEDBACK) / 100.0,
                stereo: match plain(ECHO_STEREO) as usize {
                    0 => EchoStereo::Mono,
                    1 => EchoStereo::PingPong,
                    _ => EchoStereo::Wide,
                },
                taps: TapPattern::ALL[(plain(ECHO_TAPS) as usize).min(TapPattern::ALL.len() - 1)],
                diffusion: plain(ECHO_DIFFUSION) / 100.0,
                color: plain(ECHO_COLOR) / 100.0,
                wobble: plain(ECHO_WOBBLE) / 100.0,
                low_cut_hz: plain(ECHO_LOW_CUT),
            },
            echo_duck: plain(ECHO_DUCK) / 100.0,
            echo_level: level_gain(plain(ECHO_LEVEL)),
            reverb_on: plain(REVERB_ON) >= 0.5,
            reverb: ReverbSettings {
                model: ReverbModel::ALL
                    [(plain(REVERB_MODEL) as usize).min(ReverbModel::ALL.len() - 1)],
                size: plain(SIZE) / 100.0,
                decay_seconds: plain(DECAY),
                pre_delay_seconds: plain(PRE_DELAY) / 1000.0,
                damping: plain(DAMPING) / 100.0,
                diffusion: plain(DIFFUSION) / 100.0,
                movement: plain(MOVEMENT) / 100.0,
                seed: plain(SEED).round().clamp(SEEDS.0 as f32, SEEDS.1 as f32) as u32,
                color: plain(REVERB_COLOR) / 100.0,
                low_cut_hz: plain(REVERB_LOW_CUT),
                width: plain(WIDTH) / 100.0,
            },
            reverb_duck: plain(REVERB_DUCK) / 100.0,
            reverb_level: level_gain(plain(REVERB_LEVEL)),
            series: plain(ROUTING) < 0.5,
            mix: plain(MIX) / 100.0,
            output_db: plain(OUTPUT),
        }
    }

    /// What the echo kernel is given at a tempo.
    pub fn echo(&self, tempo: f64) -> EchoSettings {
        EchoSettings {
            seconds: echo_seconds(self, tempo),
            ..self.echo
        }
    }
}

/// The echo time in effect: the Time knob, or the division at `tempo`,
/// halved until it fits the line (a whole note below 60 bpm).
pub fn echo_seconds(settings: &Settings, tempo: f64) -> f32 {
    if !settings.echo_sync {
        return settings.echo_time_seconds;
    }
    let mut seconds = division_beats(settings.echo_division) * 60.0 / tempo.max(1.0);
    while seconds > f64::from(echo::MAX_SECONDS) {
        seconds *= 0.5;
    }
    seconds as f32
}

/// Whether a parameter does anything at these settings.
pub fn applies(id: u32, settings: &Settings) -> bool {
    match id {
        ECHO_ON | REVERB_ON => true,
        ECHO_TIME => settings.echo_on && !settings.echo_sync,
        ECHO_DIVISION => settings.echo_on && settings.echo_sync,
        ECHO_MODEL..=ECHO_DUCK => settings.echo_on,
        REVERB_MODEL..=REVERB_DUCK => settings.reverb_on,
        ROUTING => settings.echo_on && settings.reverb_on,
        _ => true,
    }
}

/// The first echoes at a tempo, as `(seconds, left gain, right gain)` with
/// the Echo level applied; returns how many of `out` are filled.
pub fn echo_taps(settings: &Settings, tempo: f64, out: &mut [(f32, f32, f32)]) -> usize {
    let count = echo::taps(&settings.echo(tempo), out);
    for tap in &mut out[..count] {
        tap.1 *= settings.echo_level;
        tap.2 *= settings.echo_level;
    }
    count
}

/// The reverb's decay time at a frequency: Decay at 1 kHz, shorter above
/// with Damping, somewhat longer below.
pub fn decay_seconds_at(settings: &Settings, hz: f32, sample_rate: f32) -> f32 {
    reverb::decay_seconds_at(&settings.reverb, hz, sample_rate)
}

/// What the reverb gives first, per channel, as
/// `(seconds after the pre-delay, gain)`: it moves with the model, the
/// size and the seed.
pub fn early_taps(settings: &Settings, out: &mut [[(f32, f32); EARLY_TAPS]; 2]) {
    reverb::early_taps(&settings.reverb, out);
}

/// The duck: full reduction from −12 dBFS of input, a soft start at −42.
const DUCK_CURVE: GainComputer = GainComputer {
    kind: CurveKind::Compress,
    threshold_db: -36.0,
    ratio: f32::INFINITY,
    knee_db: 12.0,
};
pub const DUCK_RANGE_DB: f32 = 24.0;
const DUCK_RISE_MS: f32 = 8.0;
const DUCK_FALL_MS: f32 = 300.0;
/// The detector holds its peak this long, so a low note's cycles do not
/// ripple the gain.
const DUCK_HOLD_SECONDS: f32 = 0.005;
const GATE_SECONDS: f32 = 0.05;
const ROUTING_SECONDS: f32 = 0.02;
/// Input is held to ±36 dBFS: a feedback line would keep anything larger,
/// or a NaN, for ever.
const INPUT_LIMIT: f32 = 64.0;
const LONGEST_TAIL_SECONDS: f32 = 30.0;

/// A section's on/off state: its input and output fade, and once silent it
/// is cleared and no longer run.
struct Gate {
    gain: LinearRamp,
    running: bool,
}

impl Gate {
    fn new(on: bool) -> Self {
        Self {
            gain: LinearRamp::new(if on { 1.0 } else { 0.0 }),
            running: on,
        }
    }

    fn set(&mut self, on: bool, fade: u32) {
        let aim = if on { 1.0 } else { 0.0 };
        if self.gain.target() != aim {
            self.gain.set_target(aim, fade);
        }
        self.running |= on;
    }

    /// Whether the section has just faded out and should be cleared.
    fn finished(&mut self) -> bool {
        let done = self.running && self.gain.settled() && self.gain.value() == 0.0;
        if done {
            self.running = false;
        }
        done
    }
}

pub struct Spacer {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    dirty: bool,
    sample_rate: f32,
    /// The tempo the echo time was last derived from.
    tempo: f64,
    echo: EchoKernel,
    reverb: ReverbKernel,
    echo_gate: Gate,
    reverb_gate: Gate,
    echo_level: Smoothed,
    reverb_level: Smoothed,
    /// 1 while the echo feeds the reverb.
    series: LinearRamp,
    hold: SlidingMax,
    duck: Ballistics,
    echo_duck: Smoothed,
    reverb_duck: Smoothed,
    mix: Smoothed,
    /// Dry and wet gain of the equal-power mix.
    mix_gains: (f32, f32),
    output: Smoothed,
    telemetry: Arc<Telemetry>,
}

impl Default for Spacer {
    fn default() -> Self {
        Self::new()
    }
}

fn decode(values: &[f32; PARAMETERS.len()]) -> Settings {
    Settings::from_normalized(|id| {
        values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
    })
}

fn mix_gains(mix: f32) -> (f32, f32) {
    // The wet signal is decorrelated from the dry one, so their powers add.
    if mix >= 1.0 {
        (0.0, 1.0)
    } else {
        let (wet, dry) = (mix * std::f32::consts::FRAC_PI_2).sin_cos();
        (dry, wet)
    }
}

impl Spacer {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let settings = decode(&values);
        let mut spacer = Self {
            values,
            settings,
            dirty: false,
            sample_rate: 48_000.0,
            tempo: 120.0,
            echo: EchoKernel::default(),
            reverb: ReverbKernel::default(),
            echo_gate: Gate::new(settings.echo_on),
            reverb_gate: Gate::new(settings.reverb_on),
            echo_level: Smoothed::new(settings.echo_level),
            reverb_level: Smoothed::new(settings.reverb_level),
            series: LinearRamp::new(1.0),
            hold: SlidingMax::default(),
            duck: Ballistics::default(),
            echo_duck: Smoothed::new(settings.echo_duck),
            reverb_duck: Smoothed::new(settings.reverb_duck),
            mix: Smoothed::new(settings.mix),
            mix_gains: mix_gains(settings.mix),
            output: Smoothed::new(db_to_gain(settings.output_db)),
            telemetry: Arc::new(Telemetry::default()),
        };
        spacer.prepare(48_000.0, 0);
        spacer
    }

    fn samples(&self, seconds: f32) -> u32 {
        (seconds * self.sample_rate).ceil() as u32
    }

    fn update_settings(&mut self, tempo: f64) {
        let settings = decode(&self.values);
        self.settings = settings;
        self.dirty = false;
        self.tempo = tempo;
        self.echo.set(&settings.echo(tempo));
        self.reverb.set(&settings.reverb);
        let fade = self.samples(GATE_SECONDS);
        self.echo_gate.set(settings.echo_on, fade);
        self.reverb_gate.set(settings.reverb_on, fade);
        self.echo_level.set_target(settings.echo_level);
        self.reverb_level.set_target(settings.reverb_level);
        let series = if settings.series { 1.0 } else { 0.0 };
        if self.series.target() != series {
            self.series
                .set_target(series, self.samples(ROUTING_SECONDS));
        }
        self.echo_duck.set_target(settings.echo_duck);
        self.reverb_duck.set_target(settings.reverb_duck);
        self.mix.set_target(settings.mix);
        self.output.set_target(db_to_gain(settings.output_db));
    }
}

impl Device for Spacer {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        let rate = self.sample_rate;
        self.echo.prepare(rate);
        self.reverb.prepare(rate);
        self.echo_level.set_time(20.0, rate);
        self.reverb_level.set_time(20.0, rate);
        self.echo_duck.set_time(20.0, rate);
        self.reverb_duck.set_time(20.0, rate);
        self.hold
            .allocate((DUCK_HOLD_SECONDS * rate).ceil() as usize);
        self.duck.set_times(DUCK_RISE_MS, DUCK_FALL_MS, rate);
        self.mix.set_time(5.0, rate);
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
                self.dirty = true;
            }
        }
    }

    fn process(&mut self, context: &ProcessContext, _events: &[Event], io: StereoIo<'_>) {
        // A synced echo follows the tempo, playing or not.
        if self.dirty || (self.settings.echo_sync && context.tempo != self.tempo) {
            self.update_settings(context.tempo);
        }
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        self.telemetry.record_input(left, right);
        for index in 0..frames {
            let input = [left[index], right[index]].map(|x| {
                if x.is_finite() {
                    x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
                } else {
                    0.0
                }
            });

            // Keyed from the dry input, so tails rise when the source stops.
            let amounts = [self.echo_duck.next_value(), self.reverb_duck.next_value()];
            let mut duck = [1.0; 2];
            if amounts[0] > 0.0 || amounts[1] > 0.0 || self.duck.value() > 0.0 {
                let level = self
                    .hold
                    .process(input[0].abs().max(input[1].abs()))
                    .max(1e-6);
                let reduction = DUCK_CURVE
                    .reduction_db(20.0 * level.log10())
                    .min(DUCK_RANGE_DB);
                let envelope = self.duck.process(reduction);
                for (gain, amount) in duck.iter_mut().zip(amounts) {
                    if amount > 0.0 {
                        *gain = db_to_gain(-envelope * amount);
                    }
                }
            }

            let echo_level = self.echo_level.next_value();
            let mut echo = [0.0; 2];
            if self.echo_gate.running {
                let gate = self.echo_gate.gain.next();
                let (echo_left, echo_right) = self.echo.process(gate * input[0], gate * input[1]);
                echo = [echo_left, echo_right].map(|x| x * gate * echo_level);
                if self.echo_gate.finished() {
                    self.echo.reset();
                }
            }

            // The reverb hears the echo before its duck: the reverb's own
            // duck governs the wash.
            let series = self.series.next();
            let reverb_level = self.reverb_level.next_value();
            let mut reverb = [0.0; 2];
            if self.reverb_gate.running {
                let gate = self.reverb_gate.gain.next();
                let (reverb_left, reverb_right) = self.reverb.process(
                    gate * (input[0] + series * echo[0]),
                    gate * (input[1] + series * echo[1]),
                );
                reverb = [reverb_left, reverb_right].map(|x| x * gate * reverb_level);
                if self.reverb_gate.finished() {
                    self.reverb.reset();
                }
            }

            if !self.mix.is_settled() {
                self.mix_gains = mix_gains(self.mix.next_value());
            }
            let (dry_gain, wet_gain) = self.mix_gains;
            let output = self.output.next_value();
            for (channel, sample) in [&mut *left, &mut *right].into_iter().enumerate() {
                let wet = echo[channel] * duck[0] + reverb[channel] * duck[1];
                sample[index] = (dry_gain * input[channel] + wet_gain * wet) * output;
            }
        }
        self.telemetry.set_value(TELEMETRY_DUCK, self.duck.value());
        self.telemetry
            .set_value(TELEMETRY_TIME, echo_seconds(&self.settings, self.tempo));
        self.telemetry.set_value(TELEMETRY_TEMPO, self.tempo as f32);
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        self.update_settings(self.tempo);
        self.echo.reset();
        self.reverb.reset();
        self.echo_gate = Gate::new(self.settings.echo_on);
        self.reverb_gate = Gate::new(self.settings.reverb_on);
        self.echo_level.snap();
        self.reverb_level.snap();
        self.series.snap();
        self.hold
            .set_window((DUCK_HOLD_SECONDS * self.sample_rate).ceil() as usize, 0.0);
        self.duck.reset();
        self.echo_duck.snap();
        self.reverb_duck.snap();
        self.mix.snap();
        self.mix_gains = mix_gains(self.mix.value());
        self.output.snap();
    }

    fn tail_samples(&self) -> u32 {
        let settings = decode(&self.values);
        let echo = if settings.echo_on {
            // The engine asks before the first block, when the tempo is
            // still unknown: a synced echo answers for its longest time.
            let seconds = if settings.echo_sync {
                echo::MAX_SECONDS
            } else {
                settings.echo_time_seconds
            };
            echo::tail_seconds(&EchoSettings {
                seconds,
                ..settings.echo
            })
        } else {
            0.0
        };
        let reverb = if settings.reverb_on {
            reverb::tail_seconds(&settings.reverb)
        } else {
            0.0
        };
        // In series the last echo still has its whole reverb to come.
        let seconds = if settings.series {
            echo + reverb
        } else {
            echo.max(reverb)
        };
        (seconds.min(LONGEST_TAIL_SECONDS) * self.sample_rate).ceil() as u32
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}
