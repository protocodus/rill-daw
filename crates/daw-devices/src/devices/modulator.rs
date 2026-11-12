//! R1 Modulator: chorus, flanger and phaser with one set of controls. The
//! swept wet signal comes from the shared modulation kernel; this module
//! maps the parameters, mixes it with the aligned dry signal at a constant
//! loudness and publishes what the panel draws. See
//! `docs/R1_DEVICES.md#r1-modulator`.
use super::param;
use crate::dsp::modulation::{self, ModulationKernel, ModulationSettings};
use crate::dsp::{
    Complex, CutoffGlide, LfoShape, Smoothed, StaticDelay, Svf, SvfMix, db_to_gain, prewarp,
};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, ParamRange, ParamSpec, ProcessContext, StereoIo,
    Telemetry,
};
use std::sync::Arc;

pub use crate::dsp::ModulationMode as Mode;

pub const MODE: u32 = 1;
pub const CHARACTER: u32 = 2;
pub const SHAPE: u32 = 10;
pub const SYNC: u32 = 11;
pub const RATE: u32 = 12;
pub const DIVISION: u32 = 13;
pub const DEPTH: u32 = 14;
pub const STEREO: u32 = 15;
pub const FEEDBACK: u32 = 20;
pub const CENTER_CHORUS: u32 = 21;
pub const CENTER_FLANGER: u32 = 22;
pub const CENTER_PHASER: u32 = 23;
pub const VOICES: u32 = 24;
pub const STAGES: u32 = 25;
pub const LOW_CUT: u32 = 26;
pub const MIX: u32 = 30;
pub const OUTPUT: u32 = 31;

/// Telemetry value slots: the LFO phase in cycles at the end of the last
/// block, and from each sweep slot on, where the four voices of that channel
/// are in their sweep, as LFO values (−1–1) after the sweep smoothing.
pub const TELEMETRY_PHASE: usize = 0;
pub const TELEMETRY_SWEEP_LEFT: usize = 4;
pub const TELEMETRY_SWEEP_RIGHT: usize = 8;
pub const TELEMETRY_SAMPLE_RATE: usize = 3;

pub const MODES: &[&str] = &["Chorus", "Flanger", "Phaser"];
pub const CHARACTERS: &[&str] = &["Clean", "Analog"];
pub const SHAPES: &[&str] = &["Sine", "Triangle", "Random"];
pub const DIVISIONS: &[&str] = &[
    "8 bars", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16",
];
/// Length of an LFO cycle in beats for each division.
pub const DIVISION_BEATS: [f64; 8] = [32.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25];
pub const STAGE_OPTIONS: &[&str] = &["2", "4", "6", "8", "12"];
pub const STAGE_COUNTS: [usize; 5] = [2, 4, 6, 8, 12];

/// The same at every setting and sample rate: the wet path runs at twice the
/// rate in both characters, and the dry path waits for it.
pub const LATENCY: u32 = modulation::LATENCY;

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

pub static PARAMETERS: [ParamSpec; 17] = [
    choice(MODE, "mode", "Mode", "Mode", MODES, 0.0),
    choice(CHARACTER, "character", "Character", "Mode", CHARACTERS, 0.0),
    choice(SHAPE, "shape", "Shape", "LFO", SHAPES, 0.0),
    param(SYNC, "sync", "Sync", "LFO", "", ParamRange::Toggle, 0.0),
    log(RATE, "rate", "Rate", "LFO", "Hz", (0.01, 20.0), 0.6),
    choice(DIVISION, "division", "Division", "LFO", DIVISIONS, 3.0),
    linear(DEPTH, "depth", "Depth", "LFO", "%", (0.0, 100.0), 50.0),
    linear(STEREO, "stereo", "Stereo", "LFO", "°", (0.0, 180.0), 90.0),
    linear(
        FEEDBACK,
        "feedback",
        "Feedback",
        "Wet",
        "%",
        (-95.0, 95.0),
        0.0,
    ),
    log(
        CENTER_CHORUS,
        "center.chorus",
        "Chorus center",
        "Wet",
        "ms",
        (5.0, 30.0),
        12.0,
    ),
    log(
        CENTER_FLANGER,
        "center.flanger",
        "Flanger center",
        "Wet",
        "ms",
        (0.1, 10.0),
        1.5,
    ),
    log(
        CENTER_PHASER,
        "center.phaser",
        "Phaser center",
        "Wet",
        "Hz",
        (100.0, 8000.0),
        800.0,
    ),
    param(
        VOICES,
        "voices",
        "Voices",
        "Wet",
        "",
        ParamRange::Integer { min: 1, max: 4 },
        2.0,
    ),
    choice(STAGES, "stages", "Stages", "Wet", STAGE_OPTIONS, 1.0),
    log(
        LOW_CUT,
        "low_cut",
        "Wet low cut",
        "Wet",
        "Hz",
        (20.0, 500.0),
        20.0,
    ),
    linear(MIX, "mix", "Mix", "Output", "%", (0.0, 100.0), 50.0),
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
    id: "r1-modulator",
    name: "R1 Modulator",
    kind: DeviceKind::Effect,
    category: "Modulation",
    parameters: &PARAMETERS,
    sample_slots: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-modulator/default.rillpreset"),
        include_str!("../../presets/r1-modulator/wide-chorus.rillpreset"),
        include_str!("../../presets/r1-modulator/ensemble.rillpreset"),
        include_str!("../../presets/r1-modulator/analog-chorus.rillpreset"),
        include_str!("../../presets/r1-modulator/vibrato.rillpreset"),
        include_str!("../../presets/r1-modulator/drift.rillpreset"),
        include_str!("../../presets/r1-modulator/jet-flanger.rillpreset"),
        include_str!("../../presets/r1-modulator/hollow-flanger.rillpreset"),
        include_str!("../../presets/r1-modulator/tape-flange.rillpreset"),
        include_str!("../../presets/r1-modulator/classic-phaser.rillpreset"),
        include_str!("../../presets/r1-modulator/deep-phaser.rillpreset"),
        include_str!("../../presets/r1-modulator/synced-phaser.rillpreset"),
        include_str!("../../presets/r1-modulator/fast-wobble.rillpreset"),
    ],
    legacy_labels: &[],
    fixed_on_roll: &[],
    latency: |_, _| LATENCY,
    create: || Box::new(Modulator::new()),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sine,
    Triangle,
    /// A smooth random wander, not steps.
    Random,
}

/// Plain parameter values, decoded once per change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    pub analog: bool,
    pub shape: Shape,
    pub sync: bool,
    pub rate_hz: f32,
    /// Index into [`DIVISIONS`].
    pub division: usize,
    /// 0–1.
    pub depth: f32,
    pub stereo_degrees: f32,
    /// −0.95–0.95.
    pub feedback: f32,
    /// The mode's own: seconds of delay for Chorus and Flanger, hertz for
    /// Phaser.
    pub center: f32,
    pub voices: usize,
    pub stages: usize,
    pub low_cut_hz: f32,
    /// Off at the bottom of its range: the wet signal is not filtered.
    pub low_cut_on: bool,
    /// 0–1.
    pub mix: f32,
    pub output_db: f32,
}

impl Settings {
    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("modulator parameter");
            spec.to_plain(value(id))
        };
        let mode = match plain(MODE) as usize {
            0 => Mode::Chorus,
            1 => Mode::Flanger,
            _ => Mode::Phaser,
        };
        Self {
            mode,
            analog: plain(CHARACTER) >= 0.5,
            shape: match plain(SHAPE) as usize {
                0 => Shape::Sine,
                1 => Shape::Triangle,
                _ => Shape::Random,
            },
            sync: plain(SYNC) >= 0.5,
            rate_hz: plain(RATE),
            division: (plain(DIVISION) as usize).min(DIVISIONS.len() - 1),
            depth: plain(DEPTH) / 100.0,
            stereo_degrees: plain(STEREO),
            feedback: plain(FEEDBACK) / 100.0,
            center: match mode {
                Mode::Chorus => plain(CENTER_CHORUS) / 1000.0,
                Mode::Flanger => plain(CENTER_FLANGER) / 1000.0,
                Mode::Phaser => plain(CENTER_PHASER),
            },
            voices: plain(VOICES) as usize,
            stages: STAGE_COUNTS[(plain(STAGES) as usize).min(STAGE_COUNTS.len() - 1)],
            low_cut_hz: plain(LOW_CUT),
            low_cut_on: value(LOW_CUT) > 0.0,
            mix: plain(MIX) / 100.0,
            output_db: plain(OUTPUT),
        }
    }

    /// What the shared kernel is given.
    pub fn kernel(&self) -> ModulationSettings {
        ModulationSettings {
            mode: self.mode,
            analog: self.analog,
            shape: match self.shape {
                Shape::Sine => LfoShape::Sine,
                Shape::Triangle => LfoShape::Triangle,
                Shape::Random => LfoShape::Smooth,
            },
            rate_hz: self.rate_hz,
            sync_beats: self.sync.then(|| DIVISION_BEATS[self.division]),
            depth: self.depth,
            stereo_degrees: self.stereo_degrees,
            feedback: self.feedback,
            center: self.center,
            voices: self.voices,
            stages: self.stages,
        }
    }

    /// Gain that keeps broadband material at its level whatever Mix and
    /// Feedback are: one over the root of the mean power gain across the
    /// audio band.
    pub fn loudness(&self) -> f32 {
        let (dry, wet, feedback) = (
            f64::from(1.0 - self.mix),
            f64::from(self.mix),
            f64::from(self.feedback),
        );
        let power = match self.mode {
            // The wet signal's phase against the dry one runs through every
            // angle many times across the band, so the two add in power,
            // and so does each pass through the loop. A pass carries the
            // average of the voices, which have drifted apart by then and
            // keep one part in their number of the power.
            Mode::Chorus | Mode::Flanger => {
                let voices = if self.mode == Mode::Chorus {
                    self.voices.clamp(1, 4) as f64
                } else {
                    1.0
                };
                dry * dry + wet * wet / (1.0 - feedback * feedback / voices)
            }
            // Over the sweep: these two corners stand for a sine's whole
            // cycle (Gauss-Chebyshev).
            Mode::Phaser => {
                let reach =
                    modulation::PHASER_OCTAVES * self.depth * std::f32::consts::FRAC_1_SQRT_2;
                [-reach, reach]
                    .map(|octaves| {
                        let corner = (self.center * octaves.exp2()).clamp(20.0, 20_000.0);
                        phaser_power(self.stages, f64::from(corner), dry, wet, feedback)
                    })
                    .iter()
                    .sum::<f64>()
                    / 2.0
            }
        };
        (1.0 / power.sqrt()) as f32
    }
}

/// Mean power gain of a phaser from 20 Hz to 20 kHz on a pitch axis,
/// with its stages' corner at `corner_hz`. Its chain turns the phase only a few
/// times, around the corner, and is in phase with the dry signal far
/// from it, so no single figure holds for every Center and stage count.
/// The band is cut into cells; in each the phase `φ = 2N·atan(f/fc)`
/// runs evenly enough that the mean over `φ` stands for the mean over
/// pitch, and that one has a closed form, however sharp the resonances.
fn phaser_power(stages: usize, corner_hz: f64, dry: f64, wet: f64, feedback: f64) -> f64 {
    const CELLS: usize = 40;
    const LOW_HZ: f64 = 20.0;
    const OCTAVES: f64 = 10.0;
    let resonance = 1.0 / (1.0 - feedback * feedback);
    let even = dry * dry + wet * wet * resonance;
    // The integral over `φ` of `|dry + wet·e^jφ/(1 − fb·e^jφ)|²`.
    let integral = |phase: f64| {
        let (sin, cos) = phase.sin_cos();
        let turn = (feedback * sin).atan2(1.0 - feedback * cos);
        let cross = if feedback.abs() < 1e-3 {
            sin
        } else {
            turn / feedback
        };
        even * phase + 2.0 * wet * wet * resonance * turn + 2.0 * dry * wet * cross
    };
    let phase_at = |cell: usize| {
        let hz = LOW_HZ * (OCTAVES * cell as f64 / CELLS as f64).exp2();
        2.0 * stages as f64 * (hz / corner_hz).atan()
    };
    let mut sum = 0.0;
    let mut from = phase_at(0);
    for cell in 1..=CELLS {
        let to = phase_at(cell);
        sum += if to - from > 1e-6 {
            (integral(to) - integral(from)) / (to - from)
        } else {
            // The phase stands still: the gain itself.
            let cos = from.cos();
            let denominator = 1.0 - 2.0 * feedback * cos + feedback * feedback;
            dry * dry + (wet * wet + 2.0 * dry * wet * (cos - feedback)) / denominator
        };
        from = to;
    }
    sum / CELLS as f64
}

/// Whether a parameter does anything at these settings: the panel shows one
/// Center and either Voices or Stages, Rate or Division.
pub fn applies(id: u32, settings: &Settings) -> bool {
    match id {
        CENTER_CHORUS | VOICES => settings.mode == Mode::Chorus,
        CENTER_FLANGER => settings.mode == Mode::Flanger,
        CENTER_PHASER | STAGES => settings.mode == Mode::Phaser,
        RATE => !settings.sync,
        DIVISION => settings.sync,
        _ => true,
    }
}

/// The plain values a panel sets together with Mode, as one edit: each mode
/// starts from its own Feedback, Center and voice or stage count. Presets,
/// automation and Undo never apply them.
pub fn mode_defaults(mode: Mode) -> Vec<(u32, f32)> {
    let default = |id: u32| {
        DESCRIPTOR
            .parameter(id)
            .expect("modulator parameter")
            .default
    };
    match mode {
        Mode::Chorus => vec![
            (FEEDBACK, 0.0),
            (CENTER_CHORUS, default(CENTER_CHORUS)),
            (VOICES, default(VOICES)),
        ],
        // A flanger has neither voices nor stages.
        Mode::Flanger => vec![(FEEDBACK, 50.0), (CENTER_FLANGER, default(CENTER_FLANGER))],
        Mode::Phaser => vec![
            (FEEDBACK, 50.0),
            (CENTER_PHASER, default(CENTER_PHASER)),
            (STAGES, default(STAGES)),
        ],
    }
}

fn low_cut_response(settings: &Settings, rate: f32, hz: f64) -> Complex {
    if !settings.low_cut_on {
        return Complex::ONE;
    }
    let k = std::f64::consts::SQRT_2;
    SvfMix::<f64>::new(
        f64::from(prewarp(settings.low_cut_hz, rate)),
        k,
        1.0,
        -k,
        -1.0,
    )
    .response(std::f64::consts::TAU * hz / f64::from(rate))
}

/// Gain in dB of the whole device at `hz` for small signals, with the LFO
/// standing at `lfo` for each voice: dry and wet mixed, at the loudness
/// compensation. The half-band filters of the wet path, which roll off above
/// 0.41 of the sample rate, are not part of it.
pub fn response_db(settings: &Settings, lfo: &[f32; 4], rate: f32, hz: f32) -> f32 {
    let hz = f64::from(hz);
    let wet = modulation::wet_response(&settings.kernel(), lfo, rate, hz)
        * low_cut_response(settings, rate, hz);
    let sum = Complex::real(f64::from(1.0 - settings.mix)) + wet.scale(f64::from(settings.mix));
    (20.0
        * (sum.norm() * f64::from(settings.loudness()))
            .max(1e-6)
            .log10()) as f32
}

/// [`response_db`] over a grid of rising `frequencies`, as a band: the
/// smallest and largest gain inside each grid cell. A comb's teeth can be
/// far narrower than a cell, where a single point per cell would draw
/// noise; such cells are sampled at 16 points per tooth.
pub fn response_band_db(
    settings: &Settings,
    lfo: [f32; 4],
    rate: f32,
    frequencies: &[f32],
    low: &mut [f32],
    high: &mut [f32],
) {
    // Teeth are 1/delay apart; a phaser's notches are always wide.
    let delay = if settings.mode == Mode::Phaser {
        0.0
    } else {
        lfo[..settings.voices.clamp(1, 4)]
            .iter()
            .map(|value| modulation::swept(&settings.kernel(), *value, rate) / (2.0 * rate))
            .fold(0.0, f32::max)
    };
    for (index, frequency) in frequencies.iter().enumerate() {
        let from = match index.checked_sub(1) {
            Some(before) => (frequencies[before] * frequency).sqrt(),
            None => *frequency,
        };
        let to = match frequencies.get(index + 1) {
            Some(after) => (after * frequency).sqrt(),
            None => *frequency,
        };
        let points = ((16.0 * (to - from) * delay).ceil() as usize).clamp(1, 256);
        let (mut least, mut most) = (f32::MAX, f32::MIN);
        for point in 0..points {
            let hz = if points == 1 {
                *frequency
            } else {
                from + (to - from) * point as f32 / (points - 1) as f32
            };
            let db = response_db(settings, &lfo, rate, hz);
            least = least.min(db);
            most = most.max(db);
        }
        if let (Some(low), Some(high)) = (low.get_mut(index), high.get_mut(index)) {
            *low = least;
            *high = most;
        }
    }
}

/// One cycle of a shape over `out`, −1–1, as the LFO runs it from phase 0.
/// Random is drawn as a fixed example of its curve: four smooth ramps
/// between values that end where they began.
pub fn lfo_cycle(shape: Shape, out: &mut [f32]) {
    const WANDER: [f32; 5] = [-0.2, 0.8, -0.6, 0.5, -0.2];
    let last = out.len().saturating_sub(1).max(1) as f32;
    for (index, out) in out.iter_mut().enumerate() {
        let phase = index as f32 / last;
        *out = match shape {
            Shape::Sine => (std::f32::consts::TAU * phase).sin(),
            Shape::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            Shape::Random => {
                let place = (phase * 4.0).min(3.999);
                let (segment, t) = (place as usize, place.fract());
                let (from, to) = (WANDER[segment], WANDER[segment + 1]);
                from + (to - from) * t * t * (3.0 - 2.0 * t)
            }
        };
    }
}

/// Input is held to ±36 dBFS: a feedback line would keep anything larger,
/// or a NaN, for ever.
const INPUT_LIMIT: f32 = 64.0;
const LONGEST_TAIL_SECONDS: f32 = 10.0;

pub struct Modulator {
    values: [f32; PARAMETERS.len()],
    settings: Settings,
    dirty: bool,
    sample_rate: f32,
    kernel: ModulationKernel,
    dry: [StaticDelay<{ LATENCY as usize }>; 2],
    low_cut: [Svf; 2],
    low_cut_g: CutoffGlide,
    mix: Smoothed,
    output: Smoothed,
    loudness: Smoothed,
    telemetry: Arc<Telemetry>,
}

impl Default for Modulator {
    fn default() -> Self {
        Self::new()
    }
}

fn decode(values: &[f32; PARAMETERS.len()]) -> Settings {
    Settings::from_normalized(|id| {
        values[PARAMETERS.iter().position(|spec| spec.id == id).unwrap()]
    })
}

impl Modulator {
    pub fn new() -> Self {
        let values = PARAMETERS.map(|spec| spec.default_normalized());
        let settings = decode(&values);
        let mut modulator = Self {
            values,
            settings,
            dirty: false,
            sample_rate: 48_000.0,
            kernel: ModulationKernel::default(),
            dry: [StaticDelay::default(), StaticDelay::default()],
            low_cut: [Svf::default(), Svf::default()],
            low_cut_g: CutoffGlide::new(settings.low_cut_hz),
            mix: Smoothed::new(settings.mix),
            output: Smoothed::new(db_to_gain(settings.output_db)),
            loudness: Smoothed::new(settings.loudness()),
            telemetry: Arc::new(Telemetry::default()),
        };
        modulator.prepare(48_000.0, 0);
        modulator
    }

    fn update_settings(&mut self) {
        let settings = decode(&self.values);
        self.settings = settings;
        self.dirty = false;
        self.kernel.set(&settings.kernel());
        self.low_cut_g.set(settings.low_cut_hz);
        self.mix.set_target(settings.mix);
        self.output.set_target(db_to_gain(settings.output_db));
        self.loudness.set_target(settings.loudness());
    }
}

impl Device for Modulator {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        &DESCRIPTOR
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate.max(8000.0);
        let rate = self.sample_rate;
        self.kernel.prepare(rate);
        self.low_cut_g.prepare(20.0, rate);
        self.mix.set_time(5.0, rate);
        self.output.set_time(5.0, rate);
        self.loudness.set_time(20.0, rate);
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
        if self.dirty {
            self.update_settings();
        }
        let StereoIo { left, right, .. } = io;
        let frames = left.len().min(right.len());
        let (left, right) = (&mut left[..frames], &mut right[..frames]);
        self.telemetry.record_input(left, right);
        let beats_per_sample = context.tempo / (60.0 * f64::from(self.sample_rate));
        for index in 0..frames {
            let input = [left[index], right[index]].map(|x| {
                if x.is_finite() {
                    x.clamp(-INPUT_LIMIT, INPUT_LIMIT)
                } else {
                    0.0
                }
            });
            let beat = context.beat + index as f64 * beats_per_sample;
            let (wet_left, wet_right) =
                self.kernel
                    .process(context.tempo, beat, context.playing, input[0], input[1]);
            let low_cut_g = self.low_cut_g.next_g();
            let low_cut_off = !self.settings.low_cut_on && self.low_cut_g.is_settled();
            let mix = self.mix.next_value();
            let gain = self.loudness.next_value() * self.output.next_value();
            for (channel, (sample, wet)) in [&mut *left, &mut *right]
                .into_iter()
                .zip([wet_left, wet_right])
                .enumerate()
            {
                // The filter runs while it is off, so it comes in on a
                // signal it already knows.
                let cut = self.low_cut[channel]
                    .process(wet, low_cut_g, std::f32::consts::SQRT_2)
                    .high;
                let wet = if low_cut_off { wet } else { cut };
                let dry = self.dry[channel].process(input[channel]);
                sample[index] = (dry + mix * (wet - dry)) * gain;
            }
        }
        self.telemetry
            .set_value(TELEMETRY_PHASE, self.kernel.phase() as f32);
        for (slot, channel) in [TELEMETRY_SWEEP_LEFT, TELEMETRY_SWEEP_RIGHT]
            .into_iter()
            .zip(self.kernel.sweep())
        {
            for (voice, value) in channel.iter().enumerate() {
                self.telemetry.set_value(slot + voice, *value);
            }
        }
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        self.update_settings();
        self.kernel.reset();
        self.dry = [StaticDelay::default(), StaticDelay::default()];
        for filter in &mut self.low_cut {
            filter.reset();
        }
        self.low_cut_g.snap();
        self.mix.snap();
        self.output.snap();
        self.loudness.snap();
    }

    fn latency_samples(&self) -> u32 {
        LATENCY
    }

    fn tail_samples(&self) -> u32 {
        let seconds = modulation::tail_seconds(&decode(&self.values).kernel());
        LATENCY + (seconds.min(LONGEST_TAIL_SECONDS) * self.sample_rate).ceil() as u32
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        Arc::clone(&self.telemetry)
    }
}
