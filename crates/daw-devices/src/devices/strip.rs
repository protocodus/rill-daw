//! R1 Strip: a mixer channel as an insert. Trim and polarity, high- and
//! low-pass, gate, a three-band equalizer, compressor, stereo width, pan and
//! output, in that order. See `docs/R1_DEVICES.md#r1-strip`.
//!
//! It runs no DSP of its own: the filters and the equalizer are R1
//! Equalizer, the gate and the compressor are R1 Compressor, and width and
//! pan are the mixer's own `stereo_width` and `balance`. Its parameters are
//! forwarded to those inner devices, so the strip sounds exactly like them.
//!
//! A mixer channel is this device with an insert slot between
//! [`Strip::process_channel`] and [`Strip::process_output`].
use super::{compressor, equalizer, param};
use crate::dsp::{Smoothed, balance, db_to_gain, stereo_width};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, ParamRange, ParamSpec, ProcessContext, StereoIo,
    Telemetry,
};
use std::sync::Arc;

pub const TRIM: u32 = 1;
pub const POLARITY: u32 = 2;
pub const MODE: u32 = 3;
pub const HP_ON: u32 = 10;
pub const HP_FREQUENCY: u32 = 11;
pub const HP_SLOPE: u32 = 12;
pub const LP_ON: u32 = 15;
pub const LP_FREQUENCY: u32 = 16;
pub const LP_SLOPE: u32 = 17;
pub const GATE_ON: u32 = 20;
pub const GATE_THRESHOLD: u32 = 21;
pub const GATE_RATIO: u32 = 22;
pub const GATE_ATTACK: u32 = 23;
pub const GATE_RELEASE: u32 = 24;
pub const GATE_RANGE: u32 = 25;
pub const GATE_HOLD: u32 = 26;
pub const EQ_ON: u32 = 30;
pub const LOW_FREQUENCY: u32 = 31;
pub const LOW_GAIN: u32 = 32;
pub const MID_FREQUENCY: u32 = 33;
pub const MID_GAIN: u32 = 34;
pub const MID_Q: u32 = 35;
pub const HIGH_FREQUENCY: u32 = 36;
pub const HIGH_GAIN: u32 = 37;
pub const COMP_ON: u32 = 40;
pub const COMP_THRESHOLD: u32 = 41;
pub const COMP_RATIO: u32 = 42;
pub const COMP_ATTACK: u32 = 43;
pub const COMP_RELEASE: u32 = 44;
pub const COMP_MAKEUP: u32 = 45;
pub const COMP_UPWARD_THRESHOLD: u32 = 46;
pub const COMP_UPWARD_RATIO: u32 = 47;
pub const WIDTH: u32 = 50;
pub const PAN: u32 = 52;
pub const OUTPUT: u32 = 51;

/// What a mixer channel does with its own controls, around its inserts:
/// its strip never sets these.
pub const CHANNEL_FIELDS: [u32; 5] = [TRIM, POLARITY, WIDTH, PAN, OUTPUT];

const fn toggle(id: u32, key: &'static str, name: &'static str, group: &'static str) -> ParamSpec {
    param(id, key, name, group, "", ParamRange::Toggle, 0.0)
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

const fn slope(id: u32, key: &'static str, group: &'static str) -> ParamSpec {
    param(
        id,
        key,
        "Slope",
        group,
        "",
        ParamRange::Choice(equalizer::SLOPES),
        0.0,
    )
}

// Ranges match the inner devices' where the strip offers the whole range,
// so a value moves the same distance on either panel.
pub static PARAMETERS: [ParamSpec; 35] = [
    linear(TRIM, "trim", "Trim", "Input", "dB", (-24.0, 24.0), 0.0),
    toggle(POLARITY, "polarity", "Invert polarity", "Input"),
    param(
        MODE,
        "mode",
        "Mode",
        "Input",
        "",
        ParamRange::Choice(equalizer::MODES),
        0.0,
    ),
    toggle(HP_ON, "hp.on", "On", "High-pass"),
    log(
        HP_FREQUENCY,
        "hp.frequency",
        "Frequency",
        "High-pass",
        "Hz",
        (20.0, 2000.0),
        80.0,
    ),
    slope(HP_SLOPE, "hp.slope", "High-pass"),
    toggle(LP_ON, "lp.on", "On", "Low-pass"),
    log(
        LP_FREQUENCY,
        "lp.frequency",
        "Frequency",
        "Low-pass",
        "Hz",
        (200.0, 20_000.0),
        18_000.0,
    ),
    slope(LP_SLOPE, "lp.slope", "Low-pass"),
    toggle(GATE_ON, "gate.on", "On", "Gate"),
    linear(
        GATE_THRESHOLD,
        "gate.threshold",
        "Threshold",
        "Gate",
        "dB",
        (-60.0, 0.0),
        -50.0,
    ),
    log(
        GATE_RATIO,
        "gate.ratio",
        "Ratio",
        "Gate",
        ":1",
        (1.0, 20.0),
        10.0,
    ),
    log(
        GATE_ATTACK,
        "gate.attack",
        "Attack",
        "Gate",
        "ms",
        (0.02, 250.0),
        0.5,
    ),
    log(
        GATE_RELEASE,
        "gate.release",
        "Release",
        "Gate",
        "ms",
        (5.0, 2500.0),
        150.0,
    ),
    linear(
        GATE_RANGE,
        "gate.range",
        "Range",
        "Gate",
        "dB",
        (0.0, 80.0),
        80.0,
    ),
    linear(
        GATE_HOLD,
        "gate.hold",
        "Hold",
        "Gate",
        "ms",
        (0.0, 500.0),
        0.0,
    ),
    param(
        EQ_ON,
        "eq.on",
        "On",
        "Equalizer",
        "",
        ParamRange::Toggle,
        1.0,
    ),
    log(
        LOW_FREQUENCY,
        "low.frequency",
        "Low",
        "Equalizer",
        "Hz",
        (20.0, 1000.0),
        100.0,
    ),
    linear(
        LOW_GAIN,
        "low.gain",
        "Low gain",
        "Equalizer",
        "dB",
        (-24.0, 24.0),
        0.0,
    ),
    log(
        MID_FREQUENCY,
        "mid.frequency",
        "Mid",
        "Equalizer",
        "Hz",
        (100.0, 10_000.0),
        1000.0,
    ),
    linear(
        MID_GAIN,
        "mid.gain",
        "Mid gain",
        "Equalizer",
        "dB",
        (-24.0, 24.0),
        0.0,
    ),
    log(MID_Q, "mid.q", "Mid Q", "Equalizer", "", (0.1, 18.0), 1.0),
    log(
        HIGH_FREQUENCY,
        "high.frequency",
        "High",
        "Equalizer",
        "Hz",
        (1000.0, 20_000.0),
        8000.0,
    ),
    linear(
        HIGH_GAIN,
        "high.gain",
        "High gain",
        "Equalizer",
        "dB",
        (-24.0, 24.0),
        0.0,
    ),
    toggle(COMP_ON, "comp.on", "On", "Compressor"),
    linear(
        COMP_THRESHOLD,
        "comp.threshold",
        "Threshold",
        "Compressor",
        "dB",
        (-60.0, 0.0),
        -18.0,
    ),
    log(
        COMP_RATIO,
        "comp.ratio",
        "Ratio",
        "Compressor",
        ":1",
        (1.0, 20.0),
        4.0,
    ),
    log(
        COMP_ATTACK,
        "comp.attack",
        "Attack",
        "Compressor",
        "ms",
        (0.02, 250.0),
        10.0,
    ),
    log(
        COMP_RELEASE,
        "comp.release",
        "Release",
        "Compressor",
        "ms",
        (5.0, 2500.0),
        120.0,
    ),
    linear(
        COMP_MAKEUP,
        "comp.makeup",
        "Makeup",
        "Compressor",
        "dB",
        (-12.0, 24.0),
        0.0,
    ),
    linear(
        COMP_UPWARD_THRESHOLD,
        "comp.upward.threshold",
        "Upward threshold",
        "Compressor",
        "dB",
        (-60.0, 0.0),
        -40.0,
    ),
    log(
        COMP_UPWARD_RATIO,
        "comp.upward.ratio",
        "Upward ratio",
        "Compressor",
        ":1",
        (1.0, 20.0),
        1.0,
    ),
    linear(WIDTH, "width", "Width", "Output", "%", (0.0, 200.0), 100.0),
    linear(PAN, "pan", "Pan", "Output", "%", (-100.0, 100.0), 0.0),
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
    id: "r1-strip",
    name: "R1 Strip",
    kind: DeviceKind::Effect,
    category: "Channel strip",
    parameters: &PARAMETERS,
    sample_slots: 0,
    sample_capacity: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-strip/default.rillpreset"),
        include_str!("../../presets/r1-strip/vocal.rillpreset"),
        include_str!("../../presets/r1-strip/kick.rillpreset"),
        include_str!("../../presets/r1-strip/snare.rillpreset"),
        include_str!("../../presets/r1-strip/bass.rillpreset"),
        include_str!("../../presets/r1-strip/acoustic-guitar.rillpreset"),
        include_str!("../../presets/r1-strip/backing-vocals.rillpreset"),
        include_str!("../../presets/r1-strip/dense-vocal.rillpreset"),
        include_str!("../../presets/r1-strip/voice-over.rillpreset"),
        include_str!("../../presets/r1-strip/toms.rillpreset"),
        include_str!("../../presets/r1-strip/overheads.rillpreset"),
        include_str!("../../presets/r1-strip/hi-hat.rillpreset"),
        include_str!("../../presets/r1-strip/room-crush.rillpreset"),
        include_str!("../../presets/r1-strip/drum-bus.rillpreset"),
        include_str!("../../presets/r1-strip/mix-bus-warmth.rillpreset"),
        include_str!("../../presets/r1-strip/electric-guitar.rillpreset"),
        include_str!("../../presets/r1-strip/piano.rillpreset"),
        include_str!("../../presets/r1-strip/synth-pad.rillpreset"),
        include_str!("../../presets/r1-strip/strings.rillpreset"),
        include_str!("../../presets/r1-strip/mono-check.rillpreset"),
    ],
    legacy_labels: &[],
    // Polarity changes what the strip does to the mix, not how it sounds.
    fixed_on_roll: &["polarity"],
    // Neither inner compressor looks ahead, and R1 Equalizer has no latency.
    latency: |_, _| 0,
    create: || Box::new(Strip::new()),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// High- and low-pass: an R1 Equalizer with its bands off.
    Cut,
    Gate,
    /// The three bands: an R1 Equalizer with its cut filters off.
    Eq,
    Compressor,
}

/// Each forwarded parameter and where it goes. Values are passed as plain
/// values, so the strip's ranges can be narrower than the inner device's.
const ROUTES: &[(u32, Stage, u32)] = {
    use Stage::*;
    use equalizer::{BAND_FREQUENCY, BAND_GAIN, BAND_Q, BANDS};
    &[
        (HP_ON, Cut, equalizer::HP_ON),
        (HP_FREQUENCY, Cut, equalizer::HP_FREQUENCY),
        (HP_SLOPE, Cut, equalizer::HP_SLOPE),
        (LP_ON, Cut, equalizer::LP_ON),
        (LP_FREQUENCY, Cut, equalizer::LP_FREQUENCY),
        (LP_SLOPE, Cut, equalizer::LP_SLOPE),
        (GATE_THRESHOLD, Gate, compressor::THRESHOLD),
        (GATE_RATIO, Gate, compressor::RATIO),
        (GATE_ATTACK, Gate, compressor::ATTACK),
        (GATE_RELEASE, Gate, compressor::RELEASE),
        (GATE_RANGE, Gate, compressor::RANGE),
        (GATE_HOLD, Gate, compressor::HOLD),
        (LOW_FREQUENCY, Eq, BANDS[0] + BAND_FREQUENCY),
        (LOW_GAIN, Eq, BANDS[0] + BAND_GAIN),
        (MID_FREQUENCY, Eq, BANDS[1] + BAND_FREQUENCY),
        (MID_GAIN, Eq, BANDS[1] + BAND_GAIN),
        (MID_Q, Eq, BANDS[1] + BAND_Q),
        (HIGH_FREQUENCY, Eq, BANDS[2] + BAND_FREQUENCY),
        (HIGH_GAIN, Eq, BANDS[2] + BAND_GAIN),
        (COMP_THRESHOLD, Compressor, compressor::THRESHOLD),
        (COMP_RATIO, Compressor, compressor::RATIO),
        (COMP_ATTACK, Compressor, compressor::ATTACK),
        (COMP_RELEASE, Compressor, compressor::RELEASE),
        (COMP_MAKEUP, Compressor, compressor::MAKEUP),
        (
            COMP_UPWARD_THRESHOLD,
            Compressor,
            compressor::UPWARD_THRESHOLD,
        ),
        (COMP_UPWARD_RATIO, Compressor, compressor::UPWARD_RATIO),
    ]
};

/// What the strip fixes in its inner devices, as plain values.
const FIXED: &[(Stage, u32, f32)] = {
    use Stage::*;
    use equalizer::{BAND_ON, BAND_Q, BAND_TYPE, BANDS};
    &[
        (Cut, BANDS[0] + BAND_ON, 0.0),
        (Cut, BANDS[1] + BAND_ON, 0.0),
        (Cut, BANDS[2] + BAND_ON, 0.0),
        (Cut, BANDS[3] + BAND_ON, 0.0),
        (Cut, equalizer::ANALYZER, 0.0),
        (Eq, equalizer::HP_ON, 0.0),
        (Eq, equalizer::LP_ON, 0.0),
        (Eq, equalizer::ANALYZER, 0.0),
        (Eq, BANDS[0] + BAND_TYPE, 1.0),
        (Eq, BANDS[0] + BAND_Q, 0.71),
        (Eq, BANDS[1] + BAND_TYPE, 0.0),
        (Eq, BANDS[2] + BAND_TYPE, 2.0),
        (Eq, BANDS[2] + BAND_Q, 0.71),
        (Eq, BANDS[3] + BAND_ON, 0.0),
        // A gate is a steep expander; a soft knee keeps it from chattering.
        (Gate, compressor::MODE, 2.0),
        (Gate, compressor::KNEE, 3.0),
    ]
};

/// The input transformer of a console channel, which Analog mode switches in
/// on the cut filters' two spare bands: a slight lift in the deep lows and
/// a soft top. Kind, frequency, gain and Q.
const TRANSFORMER: [(equalizer::BandType, f32, f32, f32); 2] = [
    (equalizer::BandType::Bell, 40.0, 0.6, 0.8),
    (equalizer::BandType::HighShelf, 16_000.0, -0.6, 0.71),
];

/// The three bands EQ_ON switches together.
const EQ_BANDS: [u32; 3] = [
    equalizer::BANDS[0] + equalizer::BAND_ON,
    equalizer::BANDS[1] + equalizer::BAND_ON,
    equalizer::BANDS[2] + equalizer::BAND_ON,
];

/// Frames the strip hands its stages at a time, so a stage's dry copy fits
/// on the stack.
const CHUNK: usize = 64;

/// The same bound as the inner devices put on their output (+36 dBFS):
/// only hostile settings and input reach it, but trim, makeup, width and
/// output stacked on top could otherwise run far past it.
const LIMIT: f32 = 64.0;

/// The plain value of a strip parameter.
fn plain(values: &[f32; PARAMETERS.len()], id: u32) -> f32 {
    let index = index(id);
    PARAMETERS[index].to_plain(values[index])
}

fn index(id: u32) -> usize {
    PARAMETERS
        .iter()
        .position(|spec| spec.id == id)
        .expect("strip parameter")
}

/// The equalizer the strip's filters and bands add up to, for drawing its
/// response: the inner devices are two serial equalizers whose sections
/// together are this one.
pub fn equalizer_settings(value: impl Fn(u32) -> f32) -> equalizer::Settings {
    let plain = |id: u32| DESCRIPTOR.parameter(id).unwrap().to_plain(value(id));
    let on = |id: u32| plain(id) >= 0.5;
    let slope = |id: u32| match plain(id) as usize {
        1 => equalizer::Slope::Db24,
        2 => equalizer::Slope::Db48,
        _ => equalizer::Slope::Db12,
    };
    let eq = on(EQ_ON);
    let band = |kind, frequency, gain, q: f32| equalizer::BandSettings {
        on: eq,
        kind,
        frequency: plain(frequency),
        gain_db: plain(gain),
        q,
    };
    equalizer::Settings {
        hp: equalizer::CutSettings {
            on: on(HP_ON),
            frequency: plain(HP_FREQUENCY),
            slope: slope(HP_SLOPE),
        },
        bands: [
            band(equalizer::BandType::LowShelf, LOW_FREQUENCY, LOW_GAIN, 0.71),
            band(
                equalizer::BandType::Bell,
                MID_FREQUENCY,
                MID_GAIN,
                plain(MID_Q),
            ),
            band(
                equalizer::BandType::HighShelf,
                HIGH_FREQUENCY,
                HIGH_GAIN,
                0.71,
            ),
            equalizer::BandSettings {
                on: false,
                kind: equalizer::BandType::Bell,
                frequency: 1000.0,
                gain_db: 0.0,
                q: 1.0,
            },
        ],
        lp: equalizer::CutSettings {
            on: on(LP_ON),
            frequency: plain(LP_FREQUENCY),
            slope: slope(LP_SLOPE),
        },
        mode: if on(MODE) {
            equalizer::Mode::Analog
        } else {
            equalizer::Mode::Digital
        },
        auto_gain: false,
        analyzer: equalizer::Analyzer::Off,
        output_db: 0.0,
    }
}

/// The strip's whole frequency response, for drawing it: the equalizer
/// above plus, in Analog mode, the transformer.
pub fn response_db(
    value: impl Fn(u32) -> f32,
    sample_rate: f32,
    frequencies: &[f32],
    out: &mut [f32],
) {
    let settings = equalizer_settings(&value);
    equalizer::response_db(&settings, sample_rate, frequencies, out);
    if settings.mode == equalizer::Mode::Digital {
        return;
    }
    let transformer = equalizer::Settings {
        hp: equalizer::CutSettings {
            on: false,
            ..settings.hp
        },
        lp: equalizer::CutSettings {
            on: false,
            ..settings.lp
        },
        bands: [0, 1, 2, 3].map(|index| match TRANSFORMER.get(index) {
            Some(&(kind, frequency, gain_db, q)) => equalizer::BandSettings {
                on: true,
                kind,
                frequency,
                gain_db,
                q,
            },
            None => equalizer::BandSettings {
                on: false,
                ..settings.bands[3]
            },
        }),
        // The transformer sits with the cut filters, which stay Digital.
        mode: equalizer::Mode::Digital,
        ..settings
    };
    let mut lift = vec![0.0; out.len()];
    equalizer::response_db(&transformer, sample_rate, frequencies, &mut lift);
    for (out, lift) in out.iter_mut().zip(lift) {
        *out += lift;
    }
}

pub struct Strip {
    values: [f32; PARAMETERS.len()],
    cut: equalizer::Equalizer,
    gate: compressor::Compressor,
    eq: equalizer::Equalizer,
    compressor: compressor::Compressor,
    /// How much of each dynamics stage is heard, so switching one is a
    /// short crossfade, not a step.
    gate_mix: Smoothed,
    compressor_mix: Smoothed,
    /// Trim with the polarity sign, which glides through zero like the
    /// mixer's does.
    trim: Smoothed,
    width: Smoothed,
    pan: Smoothed,
    output: Smoothed,
    telemetry: Arc<Telemetry>,
}

impl Default for Strip {
    fn default() -> Self {
        Self::new()
    }
}

impl Strip {
    pub fn new() -> Self {
        let mut strip = Self {
            values: PARAMETERS.map(|spec| spec.default_normalized()),
            cut: equalizer::Equalizer::new(),
            gate: compressor::Compressor::new(),
            eq: equalizer::Equalizer::new(),
            compressor: compressor::Compressor::new(),
            gate_mix: Smoothed::new(0.0),
            compressor_mix: Smoothed::new(0.0),
            trim: Smoothed::new(1.0),
            width: Smoothed::new(1.0),
            pan: Smoothed::new(0.0),
            output: Smoothed::new(1.0),
            telemetry: Arc::new(Telemetry::meter_only()),
        };
        for &(stage, id, value) in FIXED {
            let spec = strip.inner_descriptor(stage).parameter(id).unwrap();
            strip
                .inner(stage)
                .set_parameter(id, spec.to_normalized(value));
        }
        for (index, &(kind, frequency, gain, q)) in TRANSFORMER.iter().enumerate() {
            let band = equalizer::BANDS[index];
            for (offset, value) in [
                (equalizer::BAND_TYPE, kind as u32 as f32),
                (equalizer::BAND_FREQUENCY, frequency),
                (equalizer::BAND_GAIN, gain),
                (equalizer::BAND_Q, q),
            ] {
                let spec = equalizer::DESCRIPTOR.parameter(band + offset).unwrap();
                strip
                    .cut
                    .set_parameter(band + offset, spec.to_normalized(value));
            }
        }
        for spec in &PARAMETERS {
            strip.forward(spec.id);
        }
        strip.prepare(48_000.0, 0);
        strip
    }

    fn inner(&mut self, stage: Stage) -> &mut dyn Device {
        match stage {
            Stage::Cut => &mut self.cut,
            Stage::Gate => &mut self.gate,
            Stage::Eq => &mut self.eq,
            Stage::Compressor => &mut self.compressor,
        }
    }

    fn inner_descriptor(&self, stage: Stage) -> &'static DeviceDescriptor {
        match stage {
            Stage::Cut | Stage::Eq => &equalizer::DESCRIPTOR,
            Stage::Gate | Stage::Compressor => &compressor::DESCRIPTOR,
        }
    }

    /// Passes a parameter's value on to whatever it drives.
    fn forward(&mut self, id: u32) {
        let value = plain(&self.values, id);
        let on = if value >= 0.5 { 1.0 } else { 0.0 };
        match id {
            TRIM | POLARITY => {
                let sign = if plain(&self.values, POLARITY) >= 0.5 {
                    -1.0
                } else {
                    1.0
                };
                self.trim
                    .set_target(sign * db_to_gain(plain(&self.values, TRIM)));
            }
            // Analog: the equalizer's console curves and soft headroom, and
            // the transformer on the cut filters. Digital costs nothing.
            MODE => {
                let spec = equalizer::DESCRIPTOR.parameter(equalizer::MODE).unwrap();
                self.eq
                    .set_parameter(equalizer::MODE, spec.to_normalized(on));
                for index in 0..TRANSFORMER.len() {
                    self.cut
                        .set_parameter(equalizer::BANDS[index] + equalizer::BAND_ON, on);
                }
            }
            GATE_ON => self.gate_mix.set_target(on),
            COMP_ON => self.compressor_mix.set_target(on),
            EQ_ON => {
                for band in EQ_BANDS {
                    self.eq.set_parameter(band, on);
                }
            }
            WIDTH => self.width.set_target(value / 100.0),
            PAN => self.pan.set_target(value / 100.0),
            OUTPUT => self.output.set_target(db_to_gain(value)),
            _ => {
                if let Some(&(_, stage, inner)) = ROUTES.iter().find(|route| route.0 == id) {
                    let spec = self.inner_descriptor(stage).parameter(inner).unwrap();
                    self.inner(stage)
                        .set_parameter(inner, spec.to_normalized(value));
                }
            }
        }
    }

    /// Trim and polarity through the compressor: everything before the
    /// insert slot of a mixer channel. [`Device::process`] is this, then
    /// [`Self::process_output`]; a mixer channel runs its inserts between.
    pub fn process_channel(
        &mut self,
        context: &ProcessContext,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        for (left, right) in left.chunks_mut(CHUNK).zip(right.chunks_mut(CHUNK)) {
            for index in 0..left.len() {
                let trim = self.trim.next_value();
                left[index] *= trim;
                right[index] *= trim;
            }
            self.cut.process(
                context,
                &[],
                StereoIo {
                    left: &mut *left,
                    right: &mut *right,
                    sidechain: None,
                },
            );
            dynamics(&mut self.gate, &mut self.gate_mix, context, left, right);
            self.eq.process(
                context,
                &[],
                StereoIo {
                    left: &mut *left,
                    right: &mut *right,
                    sidechain: None,
                },
            );
            dynamics(
                &mut self.compressor,
                &mut self.compressor_mix,
                context,
                left,
                right,
            );
        }
    }

    /// Width, pan and output: the volume stage after a channel's inserts.
    pub fn process_output(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (left, right) in left.iter_mut().zip(right.iter_mut()) {
            let width = self.width.next_value();
            let output = self.output.next_value();
            let [l, r] = if width == 1.0 {
                [*left, *right]
            } else {
                stereo_width([*left, *right], width)
            };
            let [pan_left, pan_right] = balance(self.pan.next_value());
            *left = (l * output * pan_left).clamp(-LIMIT, LIMIT);
            *right = (r * output * pan_right).clamp(-LIMIT, LIMIT);
        }
    }

    fn smoothers(&mut self) -> [&mut Smoothed; 6] {
        [
            &mut self.pan,
            &mut self.gate_mix,
            &mut self.compressor_mix,
            &mut self.trim,
            &mut self.width,
            &mut self.output,
        ]
    }
}

/// Runs a dynamics stage on a chunk, crossfaded with its input by `mix`.
/// Skipped while it is fully off; it then starts again from rest.
fn dynamics(
    device: &mut compressor::Compressor,
    mix: &mut Smoothed,
    context: &ProcessContext,
    left: &mut [f32],
    right: &mut [f32],
) {
    if mix.is_settled() && mix.value() == 0.0 {
        return;
    }
    if mix.value() == 0.0 {
        device.reset();
    }
    let frames = left.len();
    let mut dry = [[0.0_f32; CHUNK]; 2];
    dry[0][..frames].copy_from_slice(left);
    dry[1][..frames].copy_from_slice(right);
    device.process(
        context,
        &[],
        StereoIo {
            left: &mut *left,
            right: &mut *right,
            sidechain: None,
        },
    );
    if mix.is_settled() {
        return;
    }
    for index in 0..frames {
        let amount = mix.next_value();
        left[index] = dry[0][index] + amount * (left[index] - dry[0][index]);
        right[index] = dry[1][index] + amount * (right[index] - dry[1][index]);
    }
}

impl Device for Strip {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, max_block: usize) {
        let rate = sample_rate.max(8000.0);
        for smoothed in self.smoothers() {
            smoothed.set_time(5.0, rate);
        }
        let block = max_block.min(CHUNK);
        self.cut.prepare(rate, block);
        self.gate.prepare(rate, block);
        self.eq.prepare(rate, block);
        self.compressor.prepare(rate, block);
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
                self.forward(id);
            }
        }
    }

    fn process(&mut self, context: &ProcessContext, _events: &[Event], io: StereoIo<'_>) {
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        self.telemetry.record_input(left, right);
        self.process_channel(context, left, right);
        self.process_output(left, right);
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        self.cut.reset();
        self.gate.reset();
        self.eq.reset();
        self.compressor.reset();
        for smoothed in self.smoothers() {
            smoothed.snap();
        }
    }

    fn tail_samples(&self) -> u32 {
        self.cut.tail_samples()
            + self.gate.tail_samples()
            + self.eq.tail_samples()
            + self.compressor.tail_samples()
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}
