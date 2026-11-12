//! R1 Filter: the shared filter kernel with every model, an LFO and an
//! envelope follower on the cutoff. See `docs/R1_DEVICES.md#r1-filter`.
use super::param;
use crate::dsp::filter::{self, FilterKernel, FilterModel, FilterShape, FilterType, LatencyDelay};
use crate::dsp::{Complex, EnvelopeFollower, Lfo, LfoShape, Smoothed, db_to_gain};
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

/// Named after the circuit technology, never a manufacturer.
pub const MODELS: &[&str] = &[
    "Clean",
    "Transistor Ladder",
    "Sallen-Key",
    "OTA Cascade",
    "OTA Multimode",
    "Diode Pair",
    "State Variable",
    "Comb",
    "Vowel",
];
pub const TYPES: &[&str] = &["Low-pass", "High-pass", "Band-pass", "Notch"];
pub const SLOPES: &[&str] = &["12 dB", "24 dB"];
pub const SYNC: &[&str] = &[
    "Off", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16",
];
pub(crate) const SYNC_BEATS: [f64; 8] = [0.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25];

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
    legacy_labels: &[
        ("model", "Ladder", "Transistor Ladder"),
        ("model", "M Ladder", "Transistor Ladder"),
        ("model", "K Sallen-Key", "Sallen-Key"),
        ("model", "X Pole-Mix", "OTA Cascade"),
        ("model", "V Digital", "OTA Multimode"),
        ("model", "Digital Multimode", "OTA Multimode"),
        ("model", "SP Diode", "Diode Pair"),
    ],
    fixed_on_roll: &[],
    latency: |_, _| FilterKernel::LATENCY,
    create: || Box::new(Filter::new()),
};

/// Plain parameter values, decoded once per block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub model: FilterModel,
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
    pub fn shape(&self) -> FilterShape {
        FilterShape {
            model: self.model,
            filter_type: self.filter_type,
            steep: self.steep,
            drive_db: self.drive_db,
        }
    }

    /// `value` returns the normalized value of a parameter ID.
    pub fn from_normalized(value: impl Fn(u32) -> f32) -> Self {
        let plain = |id: u32| {
            let spec = DESCRIPTOR.parameter(id).expect("filter parameter");
            spec.to_plain(value(id))
        };
        let index = |id: u32| plain(id) as usize;
        let model = match index(MODEL) {
            1 => FilterModel::TransistorLadder,
            2 => FilterModel::SallenKey,
            3 => FilterModel::OtaCascade,
            4 => FilterModel::OtaMultimode,
            5 => FilterModel::DiodePair,
            6 => FilterModel::StateVariable,
            7 => FilterModel::Comb,
            8 => FilterModel::Vowel,
            _ => FilterModel::Clean,
        };
        // A type the model lacks, left over from another model, falls back
        // to the model's last type.
        let filter_type = match index(TYPE).min(model.type_count().saturating_sub(1)) {
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
        TYPE => settings.model.type_count() > 0,
        SLOPE => settings.model.has_slope(),
        MORPH => settings.model.has_morph(),
        LFO_RATE => settings.lfo_sync_beats.is_none(),
        _ => true,
    }
}

/// Type options that a model offers, for the panel's selector.
pub fn type_options(model: FilterModel) -> &'static [&'static str] {
    &TYPES[..model.type_count()]
}

/// Per channel: the kernel and its aligned dry path.
#[derive(Clone, Debug, Default)]
struct Channel {
    kernel: FilterKernel,
    dry: LatencyDelay,
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
                channel.kernel.reset();
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
            channel.kernel.prepare(rate);
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
        let shape = s.shape();
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
                let lfo = if lfo_depth == 0.0 {
                    0.0
                } else {
                    self.lfo.value(s.lfo_shape, stereo * channel as f64)
                };
                let octaves = base + lfo_depth * lfo + env_amount * envelope;
                let cutoff = octaves.exp2().clamp(20.0, 20_000.0);
                cutoffs[channel] = cutoff;
                let input = sample[index];
                let state = &mut self.channels[channel];
                let wet = state
                    .kernel
                    .process(input, &shape, cutoff, resonance, morph, rate);
                let dry = state.dry.process(input);
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
            channel.kernel.reset();
            channel.dry = LatencyDelay::default();
        }
        for smoothed in self.smoothers() {
            smoothed.snap();
        }
        self.lfo.reset();
        self.follower.reset();
    }

    fn latency_samples(&self) -> u32 {
        FilterKernel::LATENCY
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
    let mix = f64::from(s.mix);
    let output = f64::from(db_to_gain(s.output_db));
    for (frequency, out) in frequencies.iter().zip(out) {
        let wet = filter::response(
            &s.shape(),
            cutoff,
            s.resonance,
            s.morph,
            sample_rate,
            *frequency,
        );
        let total = (Complex::real(1.0 - mix) + wet.scale(mix)).scale(output);
        *out = (20.0 * total.norm().max(1e-9).log10()) as f32;
    }
}
