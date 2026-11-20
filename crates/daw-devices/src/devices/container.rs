//! R1 Split and R1 Chain: containers whose lanes hold other effects. The
//! engine runs the lanes; this module owns what a container does around
//! them: dividing the signal between lanes, then gain, mute and solo per
//! lane, dry/wet mix and output. See `docs/R1_DEVICES.md#r1-split-and-r1-chain`.
use super::param;
use crate::dsp::{BandSplitter, Smoothed, SvfCoeffs, db_to_gain};
use crate::{
    Device, DeviceDescriptor, DeviceKind, Event, ParamRange, ParamSpec, ProcessContext, StereoIo,
    Telemetry,
};
use std::sync::Arc;

/// Lanes a Split can hold; parameters exist for each.
pub const MAX_LANES: usize = 8;

pub const RULE: u32 = 1;
/// Crossover `n` (from 0) between bands `n` and `n + 1`.
pub const CROSSOVERS: [u32; 3] = [2, 3, 5];
pub const BANDS: u32 = 4;
/// Lane `n` (from 0) has gain `LANE_GAIN + n`, mute `LANE_MUTE + n` and solo
/// `LANE_SOLO + n`.
pub const LANE_GAIN: u32 = 10;
pub const LANE_MUTE: u32 = 20;
pub const LANE_SOLO: u32 = 30;
pub const MIX: u32 = 40;
pub const OUTPUT: u32 = 41;

pub const RULES: &[&str] = &["All", "Bands", "Mid/Side", "Left/Right"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Every lane gets the whole signal; the lanes are summed.
    All,
    /// Two to four frequency bands, lowest in the first lane.
    Bands,
    /// Mid in the first lane, side in the second.
    MidSide,
    /// Left in the first lane, right in the second.
    LeftRight,
}

impl Rule {
    pub fn from_normalized(normalized: f32) -> Self {
        match SPLIT_PARAMETERS[0].to_plain(normalized) as usize {
            0 => Self::All,
            2 => Self::MidSide,
            3 => Self::LeftRight,
            _ => Self::Bands,
        }
    }

    /// Lanes the rule feeds, given the lanes the container holds and the
    /// band count. Lanes past it stay silent; missing lanes pass their part
    /// through unchanged.
    pub fn lanes(self, present: usize, bands: usize) -> usize {
        match self {
            Self::All => present.clamp(1, MAX_LANES),
            Self::Bands => bands.clamp(2, BandSplitter::MAX_BANDS),
            Self::MidSide | Self::LeftRight => 2,
        }
    }

    /// What each lane is called on the panel; `None` for numbered lanes.
    pub fn lane_name(self, lane: usize, bands: usize) -> Option<&'static str> {
        let names: &[&str] = match (self, bands) {
            (Self::All, _) => &[],
            (Self::Bands, 2) => &["Low", "High"],
            (Self::Bands, 3) => &["Low", "Mid", "High"],
            (Self::Bands, _) => &["Low", "Low mid", "High mid", "High"],
            (Self::MidSide, _) => &["Mid", "Side"],
            (Self::LeftRight, _) => &["Left", "Right"],
        };
        names.get(lane).copied()
    }
}

const fn db(id: u32, key: &'static str, name: &'static str, group: &'static str) -> ParamSpec {
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

const fn toggle(id: u32, key: &'static str, name: &'static str) -> ParamSpec {
    param(id, key, name, "Lanes", "", ParamRange::Toggle, 0.0)
}

const fn crossover(id: u32, key: &'static str, name: &'static str, default: f32) -> ParamSpec {
    param(
        id,
        key,
        name,
        "Split",
        "Hz",
        ParamRange::Log {
            min: 20.0,
            max: 20_000.0,
        },
        default,
    )
}

const MIX_SPEC: ParamSpec = param(
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
);
const OUTPUT_SPEC: ParamSpec = db(OUTPUT, "output", "Output", "Output");

pub static SPLIT_PARAMETERS: [ParamSpec; 5 + 3 * MAX_LANES + 2] = [
    param(
        RULE,
        "rule",
        "Split",
        "Split",
        "",
        ParamRange::Choice(RULES),
        1.0,
    ),
    param(
        BANDS,
        "bands",
        "Bands",
        "Split",
        "",
        ParamRange::Integer { min: 2, max: 4 },
        3.0,
    ),
    crossover(CROSSOVERS[0], "crossover.1", "Crossover 1", 120.0),
    crossover(CROSSOVERS[1], "crossover.2", "Crossover 2", 2500.0),
    crossover(CROSSOVERS[2], "crossover.3", "Crossover 3", 8000.0),
    db(LANE_GAIN, "lane1.gain", "Lane 1 gain", "Lanes"),
    db(LANE_GAIN + 1, "lane2.gain", "Lane 2 gain", "Lanes"),
    db(LANE_GAIN + 2, "lane3.gain", "Lane 3 gain", "Lanes"),
    db(LANE_GAIN + 3, "lane4.gain", "Lane 4 gain", "Lanes"),
    db(LANE_GAIN + 4, "lane5.gain", "Lane 5 gain", "Lanes"),
    db(LANE_GAIN + 5, "lane6.gain", "Lane 6 gain", "Lanes"),
    db(LANE_GAIN + 6, "lane7.gain", "Lane 7 gain", "Lanes"),
    db(LANE_GAIN + 7, "lane8.gain", "Lane 8 gain", "Lanes"),
    toggle(LANE_MUTE, "lane1.mute", "Lane 1 mute"),
    toggle(LANE_MUTE + 1, "lane2.mute", "Lane 2 mute"),
    toggle(LANE_MUTE + 2, "lane3.mute", "Lane 3 mute"),
    toggle(LANE_MUTE + 3, "lane4.mute", "Lane 4 mute"),
    toggle(LANE_MUTE + 4, "lane5.mute", "Lane 5 mute"),
    toggle(LANE_MUTE + 5, "lane6.mute", "Lane 6 mute"),
    toggle(LANE_MUTE + 6, "lane7.mute", "Lane 7 mute"),
    toggle(LANE_MUTE + 7, "lane8.mute", "Lane 8 mute"),
    toggle(LANE_SOLO, "lane1.solo", "Lane 1 solo"),
    toggle(LANE_SOLO + 1, "lane2.solo", "Lane 2 solo"),
    toggle(LANE_SOLO + 2, "lane3.solo", "Lane 3 solo"),
    toggle(LANE_SOLO + 3, "lane4.solo", "Lane 4 solo"),
    toggle(LANE_SOLO + 4, "lane5.solo", "Lane 5 solo"),
    toggle(LANE_SOLO + 5, "lane6.solo", "Lane 6 solo"),
    toggle(LANE_SOLO + 6, "lane7.solo", "Lane 7 solo"),
    toggle(LANE_SOLO + 7, "lane8.solo", "Lane 8 solo"),
    MIX_SPEC,
    OUTPUT_SPEC,
];

pub static CHAIN_PARAMETERS: [ParamSpec; 2] = [MIX_SPEC, OUTPUT_SPEC];

fn no_latency(_: &dyn Fn(u32) -> f32, _: f32) -> u32 {
    0
}

pub static SPLIT: DeviceDescriptor = DeviceDescriptor {
    id: "r1-split",
    name: "R1 Split",
    kind: DeviceKind::Effect,
    category: "Container",
    parameters: &SPLIT_PARAMETERS,
    sample_slots: 0,
    sample_capacity: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-split/default.rillpreset"),
        include_str!("../../presets/r1-split/ott.rillpreset"),
        include_str!("../../presets/r1-split/multiband-drive.rillpreset"),
        include_str!("../../presets/r1-split/wide-lows-mono.rillpreset"),
        include_str!("../../presets/r1-split/parallel-crush.rillpreset"),
    ],
    legacy_labels: &[],
    // A roll changes how a container sounds, never how it divides the
    // signal: that decides what its lanes are for.
    fixed_on_roll: &["rule", "bands", "mute", "solo"],
    latency: no_latency,
    create: || Box::new(Container::new(&SPLIT)),
};

pub static CHAIN: DeviceDescriptor = DeviceDescriptor {
    id: "r1-chain",
    name: "R1 Chain",
    kind: DeviceKind::Effect,
    category: "Container",
    parameters: &CHAIN_PARAMETERS,
    sample_slots: 0,
    sample_capacity: 0,
    sidechain_input: false,
    factory_presets: &[
        include_str!("../../presets/r1-chain/default.rillpreset"),
        include_str!("../../presets/r1-chain/vocal-strip.rillpreset"),
        include_str!("../../presets/r1-chain/master-glue.rillpreset"),
    ],
    legacy_labels: &[],
    fixed_on_roll: &[],
    latency: no_latency,
    create: || Box::new(Container::new(&CHAIN)),
};

/// Whether a device ID is a container: its effect holds lanes.
pub fn is_container(device: &str) -> bool {
    device == SPLIT.id || device == CHAIN.id
}

/// Short fade around a change of rule, so switching how the signal is
/// divided does not click.
const SWITCH_MS: f32 = 5.0;
const SMOOTH_MS: f32 = 5.0;
const CROSSOVER_MS: f32 = 20.0;
/// Frames between crossover coefficient updates while one glides.
const CROSSOVER_STEP: u32 = 16;

/// Divides one frame between lanes and mixes them back. Allocation-free.
pub struct Router {
    descriptor: &'static DeviceDescriptor,
    values: Vec<f32>,
    present: usize,
    sample_rate: f32,
    rule: Rule,
    pending: Option<Rule>,
    switch: Smoothed,
    splitter: BandSplitter,
    /// Crossover frequencies as log2(Hz), so a glide is even in pitch.
    bands: usize,
    pending_bands: Option<usize>,
    crossovers: [Smoothed; 3],
    coefficients: [SvfCoeffs<f64>; 3],
    countdown: u32,
    lane_gains: [Smoothed; MAX_LANES],
    mix: Smoothed,
    output: Smoothed,
}

impl Router {
    pub fn new(descriptor: &'static DeviceDescriptor) -> Self {
        let values = descriptor
            .parameters
            .iter()
            .map(ParamSpec::default_normalized)
            .collect();
        let mut router = Self {
            descriptor,
            values,
            present: 1,
            sample_rate: 48_000.0,
            rule: Rule::All,
            pending: None,
            switch: Smoothed::new(1.0),
            splitter: BandSplitter::default(),
            bands: 3,
            pending_bands: None,
            crossovers: std::array::from_fn(|_| Smoothed::new(0.0)),
            coefficients: BandSplitter::coefficients([120.0, 2500.0, 8000.0], 48_000.0),
            countdown: 0,
            lane_gains: std::array::from_fn(|_| Smoothed::new(1.0)),
            mix: Smoothed::new(1.0),
            output: Smoothed::new(1.0),
        };
        router.rule = router.target_rule();
        router.bands = router.target_bands();
        router.update_targets();
        router.snap();
        router
    }

    pub fn descriptor(&self) -> &'static DeviceDescriptor {
        self.descriptor
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8000.0);
        let rate = self.sample_rate;
        self.switch.set_time(SWITCH_MS, rate);
        for smoother in &mut self.crossovers {
            smoother.set_time(CROSSOVER_MS, rate);
        }
        for smoother in self
            .lane_gains
            .iter_mut()
            .chain([&mut self.mix, &mut self.output])
        {
            smoother.set_time(SMOOTH_MS, rate);
        }
        self.reset();
    }

    pub fn reset(&mut self) {
        self.splitter.reset();
        if let Some(rule) = self.pending.take() {
            self.rule = rule;
        }
        if let Some(bands) = self.pending_bands.take() {
            self.bands = bands;
        }
        self.switch.set_target(1.0);
        self.update_targets();
        self.snap();
    }

    /// Lanes the container holds; the All rule feeds that many.
    pub fn set_lanes_present(&mut self, lanes: usize) {
        self.present = lanes.clamp(1, MAX_LANES);
        self.update_targets();
    }

    pub fn set_parameter(&mut self, id: u32, normalized: f32) {
        let Some(index) = self
            .descriptor
            .parameters
            .iter()
            .position(|spec| spec.id == id)
        else {
            return;
        };
        let normalized = if normalized.is_finite() {
            normalized.clamp(0.0, 1.0)
        } else {
            self.descriptor.parameters[index].default_normalized()
        };
        self.values[index] = normalized;
        if id == RULE || id == BANDS {
            let (rule, bands) = (self.target_rule(), self.target_bands());
            self.pending = (rule != self.rule).then_some(rule);
            self.pending_bands = (bands != self.bands).then_some(bands);
            let switching = self.pending.is_some() || self.pending_bands.is_some();
            self.switch.set_target(if switching { 0.0 } else { 1.0 });
        }
        self.update_targets();
    }

    /// The rule in effect now; a change waits for its fade.
    pub fn rule(&self) -> Rule {
        self.rule
    }

    /// Lanes that receive signal under the current rule.
    pub fn lanes(&self) -> usize {
        self.rule.lanes(self.present, self.bands)
    }

    fn value(&self, id: u32) -> Option<(f32, &'static ParamSpec)> {
        let index = self
            .descriptor
            .parameters
            .iter()
            .position(|spec| spec.id == id)?;
        Some((self.values[index], &self.descriptor.parameters[index]))
    }

    fn plain(&self, id: u32) -> Option<f32> {
        self.value(id).map(|(value, spec)| spec.to_plain(value))
    }

    fn target_rule(&self) -> Rule {
        self.value(RULE)
            .map_or(Rule::All, |(value, _)| Rule::from_normalized(value))
    }

    fn target_bands(&self) -> usize {
        self.plain(BANDS).map_or(3, |bands| bands as usize)
    }

    fn update_targets(&mut self) {
        let rule = self.pending.unwrap_or(self.rule);
        let lanes = rule.lanes(self.present, self.pending_bands.unwrap_or(self.bands));
        let solo = (0..lanes).any(|lane| self.plain(LANE_SOLO + lane as u32) == Some(1.0));
        for lane in 0..MAX_LANES {
            let id = lane as u32;
            let audible = lane < lanes
                && self.plain(LANE_MUTE + id) != Some(1.0)
                && (!solo || self.plain(LANE_SOLO + id) == Some(1.0));
            let gain = self.plain(LANE_GAIN + id).map_or(1.0, db_to_gain);
            self.lane_gains[lane].set_target(if audible { gain } else { 0.0 });
        }
        for (smoother, (id, default)) in CROSSOVERS
            .into_iter()
            .zip([120.0, 2500.0, 8000.0])
            .enumerate()
        {
            let frequency = self.plain(id).unwrap_or(default);
            self.crossovers[smoother].set_target(frequency.log2());
        }
        self.mix
            .set_target(self.plain(MIX).map_or(1.0, |percent| percent / 100.0));
        self.output
            .set_target(self.plain(OUTPUT).map_or(1.0, db_to_gain));
    }

    fn snap(&mut self) {
        for smoother in self
            .lane_gains
            .iter_mut()
            .chain(&mut self.crossovers)
            .chain([&mut self.mix, &mut self.output, &mut self.switch])
        {
            smoother.snap();
        }
        self.update_coefficients();
    }

    fn update_coefficients(&mut self) {
        let frequencies = [0, 1, 2].map(|index| self.crossovers[index].value().exp2());
        self.coefficients = BandSplitter::coefficients(frequencies, self.sample_rate);
    }

    /// One frame into each lane's input. Lanes the rule does not feed get
    /// silence.
    #[inline]
    pub fn split(&mut self, input: [f32; 2]) -> [[f32; 2]; MAX_LANES] {
        let switch = self.switch.next_value();
        if (self.pending.is_some() || self.pending_bands.is_some()) && switch < 1e-4 {
            self.rule = self.pending.take().unwrap_or(self.rule);
            self.bands = self.pending_bands.take().unwrap_or(self.bands);
            self.splitter.reset();
            self.switch.set_target(1.0);
        }
        let mut lanes = [[0.0; 2]; MAX_LANES];
        let [left, right] = input;
        match self.rule {
            Rule::All => {
                for lane in &mut lanes[..self.present] {
                    *lane = input;
                }
            }
            Rule::Bands => {
                if !self.crossovers.iter().all(Smoothed::is_settled) {
                    for smoother in &mut self.crossovers {
                        smoother.next_value();
                    }
                    if self.countdown == 0 {
                        self.update_coefficients();
                        self.countdown = CROSSOVER_STEP;
                    }
                    self.countdown -= 1;
                } else if self.countdown != 0 {
                    self.update_coefficients();
                    self.countdown = 0;
                }
                let bands = self.splitter.process(input, self.bands, &self.coefficients);
                lanes[..BandSplitter::MAX_BANDS].copy_from_slice(&bands);
            }
            Rule::MidSide => {
                let mid = 0.5 * (left + right);
                let side = 0.5 * (left - right);
                lanes[0] = [mid; 2];
                lanes[1] = [side; 2];
            }
            Rule::LeftRight => {
                lanes[0] = [left; 2];
                lanes[1] = [right; 2];
            }
        }
        lanes
    }

    /// Mixes the lanes' outputs back into one frame. `dry` is the input,
    /// delayed by the lanes' latency.
    #[inline]
    pub fn merge(&mut self, lanes: &[[f32; 2]; MAX_LANES], dry: [f32; 2]) -> [f32; 2] {
        let mut gains = [0.0; MAX_LANES];
        for (gain, smoother) in gains.iter_mut().zip(&mut self.lane_gains) {
            *gain = smoother.next_value();
        }
        let mono = |lane: usize| 0.5 * (lanes[lane][0] + lanes[lane][1]) * gains[lane];
        let wet = match self.rule {
            Rule::All | Rule::Bands => {
                let mut sum = [0.0; 2];
                for (lane, gain) in lanes.iter().zip(gains) {
                    sum[0] += lane[0] * gain;
                    sum[1] += lane[1] * gain;
                }
                sum
            }
            Rule::MidSide => {
                let (mid, side) = (mono(0), mono(1));
                [mid + side, mid - side]
            }
            Rule::LeftRight => [mono(0), mono(1)],
        };
        let switch = self.switch.value();
        let mix = self.mix.next_value();
        let output = self.output.next_value();
        let dry_share = 1.0 - mix;
        [0, 1].map(|channel| (wet[channel] * mix * switch + dry[channel] * dry_share) * output)
    }
}

/// What the registry creates: a container whose lanes are empty, so it only
/// divides and remixes. The engine runs real lanes with a [`Router`].
pub struct Container {
    router: Router,
    telemetry: Arc<Telemetry>,
}

impl Container {
    fn new(descriptor: &'static DeviceDescriptor) -> Self {
        Self {
            router: Router::new(descriptor),
            telemetry: Arc::new(Telemetry::meter_only()),
        }
    }
}

impl Device for Container {
    fn descriptor(&self) -> &'static DeviceDescriptor {
        self.router.descriptor()
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.router.prepare(sample_rate);
    }

    fn set_parameter(&mut self, id: u32, normalized: f32) {
        self.router.set_parameter(id, normalized);
    }

    fn process(&mut self, _context: &ProcessContext, _events: &[Event], io: StereoIo<'_>) {
        let StereoIo { left, right, .. } = io;
        self.telemetry.record_input(left, right);
        for (left, right) in left.iter_mut().zip(right.iter_mut()) {
            let input = [*left, *right];
            let lanes = self.router.split(input);
            [*left, *right] = self.router.merge(&lanes, input);
        }
        self.telemetry.record_output(left, right);
    }

    fn reset(&mut self) {
        self.router.reset();
    }

    fn telemetry(&self) -> Arc<Telemetry> {
        self.telemetry.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router(rule: f32) -> Router {
        let mut router = Router::new(&SPLIT);
        router.prepare(48_000.0);
        router.set_parameter(RULE, rule);
        router.reset();
        router
    }

    #[test]
    fn identity_lanes_give_back_the_input() {
        for rule in [1.0 / 3.0, 2.0 / 3.0, 1.0] {
            let mut router = router(rule);
            for index in 0..2000 {
                let input = [(index as f32 * 0.01).sin(), (index as f32 * 0.023).cos()];
                let lanes = router.split(input);
                let output = router.merge(&lanes, input);
                if rule > 0.5 {
                    // Mid/side and left/right rebuild the input exactly.
                    assert!((output[0] - input[0]).abs() < 1e-5, "{rule}");
                    assert!((output[1] - input[1]).abs() < 1e-5, "{rule}");
                }
            }
        }
    }

    #[test]
    fn all_sums_every_lane_and_solo_isolates_one() {
        let mut router = router(0.0);
        router.set_lanes_present(3);
        router.reset();
        let lanes = router.split([0.5, 0.5]);
        assert_eq!(&lanes[..3], &[[0.5, 0.5]; 3]);
        assert_eq!(lanes[3], [0.0, 0.0]);
        assert!((router.merge(&lanes, [0.5; 2])[0] - 1.5).abs() < 1e-6);
        router.set_parameter(LANE_SOLO + 1, 1.0);
        router.reset();
        assert!((router.merge(&lanes, [0.5; 2])[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_rule_change_fades_through_silence() {
        let mut router = router(0.0);
        router.set_parameter(RULE, 1.0);
        assert_eq!(router.rule(), Rule::All);
        let mut quietest = f32::MAX;
        for _ in 0..4800 {
            let lanes = router.split([1.0, 1.0]);
            quietest = quietest.min(router.merge(&lanes, [1.0; 2])[0].abs());
        }
        assert_eq!(router.rule(), Rule::LeftRight);
        assert!(quietest < 1e-3);
    }
}
