//! The two reverb models built on a feedback delay network: Studio, a
//! diffuser cascade into a short network, and Room, a network that
//! scatters.
use super::{
    DiffuserStep, Fdn, Layout, ROOM_SCATTER_PERMUTATION, ROOM_SCATTER_SIGNS, TAP_FADE_SECONDS,
    absorption_shelf, hadamard_sign,
};
use crate::{DelayAllpass, LinearRamp};

pub(super) const LINES: usize = 16;
const PAIRS: usize = LINES / 2;
const CHANNELS: usize = 8;

/// Feedback routing: line `i` is fed from the absorbed output of line
/// `PERMUTATION[i]` with `SIGNS[i]`, then all are mixed by the Hadamard
/// matrix. The permutation keeps a line's own output out of its input's
/// largest term; any choice is lossless.
const PERMUTATION: [usize; LINES] = [7, 12, 1, 10, 4, 15, 2, 9, 0, 13, 6, 11, 3, 14, 5, 8];
const SIGNS: [f32; LINES] = [
    1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0,
];

/// Left feeds the even lines, right the odd ones; scaled so that the energy
/// that enters is the input's.
const INJECTION: [f32; LINES] = {
    let scale = 0.353_553_4;
    let mut gains = [scale; LINES];
    let mut index = 0;
    while index < LINES {
        gains[index] *= hadamard_sign(6, index);
        index += 1;
    }
    gains
};
/// Two rows of the Hadamard matrix: orthogonal, so the channels are
/// decorrelated.
const PICK_UP: [[f32; LINES]; 2] = {
    let mut rows = [[0.25; LINES]; 2];
    let mut index = 0;
    while index < LINES {
        rows[0][index] *= hadamard_sign(5, index);
        rows[1][index] *= hadamard_sign(10, index);
        index += 1;
    }
    rows
};

/// Each diffuser step shuffles within the even and within the odd channels,
/// so that without mixing left stays left.
const STEP_PERMUTATIONS: [[usize; CHANNELS]; 4] = [
    [2, 5, 6, 1, 0, 7, 4, 3],
    [6, 3, 0, 7, 4, 1, 2, 5],
    [4, 7, 2, 5, 6, 3, 0, 1],
    [0, 1, 4, 3, 2, 7, 6, 5],
];
const STEP_SIGNS: [[f32; CHANNELS]; 4] = [
    [1.0, -1.0, -1.0, 1.0, 1.0, 1.0, -1.0, -1.0],
    [-1.0, 1.0, 1.0, 1.0, -1.0, -1.0, 1.0, -1.0],
    [1.0, 1.0, -1.0, -1.0, -1.0, 1.0, 1.0, -1.0],
    [-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0],
];
/// Level of the early output, taken after the third step.
pub(super) const STUDIO_EARLY_GAIN: f32 = 0.6;
/// Swing of the rotating matrix at full Movement, in radians.
const ROTATION_RADIANS: f32 = 0.6;
pub(super) const ROOM_DIFFUSER_SECONDS: [[f32; 2]; 2] =
    [[0.004_77, 0.003_60], [0.005_03, 0.003_83]];
const ROOM_DIFFUSER_GAIN: f32 = 0.7;

pub(super) const STUDIO_LINE_SECONDS: f32 = 0.036;
pub(super) const STUDIO_STEP_SECONDS: f32 = 0.027;
pub(super) const ROOM_LINE_SECONDS: f32 = 0.075;
pub(super) const ROOM_SCATTER_SECONDS: f32 = 0.004;

/// What a model does with a new layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Move {
    /// It is silent: jump.
    Snap,
    /// Recursive lengths slide there.
    Glide,
    /// Another seed: where a slide would swoop, crossfade.
    Fade,
}

/// The slow rotation of a network's channel pairs: Movement without a
/// moving delay.
#[derive(Clone, Debug, Default)]
struct Rotation {
    /// Cycles since the reset.
    turn: [f32; PAIRS],
    phases: [f32; PAIRS],
    rates: [f32; PAIRS],
}

impl Rotation {
    fn configure(&mut self, layout: &Layout) {
        self.phases = layout.phases;
        self.rates = layout.rates;
    }

    fn aim(&self, network: &mut Fdn<LINES>, movement: f32) {
        for pair in 0..PAIRS {
            let cycle = self.turn[pair] + self.phases[pair];
            let swing = (std::f32::consts::TAU * cycle).sin();
            network.set_rotation(pair, movement * ROTATION_RADIANS * swing);
        }
    }

    fn tick(&mut self, network: &mut Fdn<LINES>, movement: f32, sample_rate: f32) {
        for (turn, rate) in self.turn.iter_mut().zip(&self.rates) {
            *turn = (*turn + rate * Fdn::<LINES>::TICK as f32 / sample_rate).fract();
        }
        self.aim(network, movement);
    }

    fn reset(&mut self, network: &mut Fdn<LINES>, movement: f32) {
        self.turn = [0.0; PAIRS];
        self.aim(network, movement);
    }
}

fn absorb(
    network: &mut Fdn<LINES>,
    extra: f32,
    decay: f32,
    high_decay: f32,
    crossover_hz: f32,
    sample_rate: f32,
    ramp: u32,
) -> bool {
    let lengths = *network.current_lengths();
    for (line, length) in lengths.iter().enumerate() {
        let samples = (length + f64::from(extra)).round() as u32;
        let shelf = absorption_shelf(samples, decay, high_decay, crossover_hz, sample_rate);
        network.set_absorption(line, &shelf, ramp);
    }
    network.is_gliding()
}

fn pick_up(outputs: &[f32; LINES]) -> [f32; 2] {
    PICK_UP.map(|row| outputs.iter().zip(&row).map(|(x, gain)| x * gain).sum())
}

/// Small rooms are dense within milliseconds: four feed-forward diffuser
/// steps of eight channels make thousands of echoes in a few of them and
/// cannot ring, then a short network of sixteen lines carries the decay.
#[derive(Clone, Debug)]
pub(super) struct Studio {
    steps: [DiffuserStep<CHANNELS>; 4],
    network: Fdn<LINES>,
    /// Cosine and sine of the steps' mixing angle.
    angle: [LinearRamp; 2],
    rotation: Rotation,
}

impl Studio {
    pub fn new() -> Self {
        Self {
            steps: std::array::from_fn(|step| {
                DiffuserStep::new(STEP_PERMUTATIONS[step], STEP_SIGNS[step])
            }),
            network: Fdn::new(PERMUTATION, SIGNS),
            angle: [LinearRamp::new(1.0), LinearRamp::new(0.0)],
            rotation: Rotation::default(),
        }
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        let samples = |seconds: f32| (seconds * sample_rate).ceil() as usize;
        for step in &mut self.steps {
            step.allocate(
                samples(STUDIO_STEP_SECONDS),
                samples(TAP_FADE_SECONDS) as u32,
            );
        }
        self.network
            .allocate(samples(STUDIO_LINE_SECONDS), sample_rate);
    }

    pub fn configure(&mut self, layout: &Layout, how: Move) {
        for (step, delays) in self.steps.iter_mut().zip(&layout.steps) {
            step.set_delays(delays);
        }
        self.network.set_lengths(&layout.elements);
        self.rotation.configure(layout);
        if how == Move::Snap {
            self.network.snap_lengths();
        }
    }

    fn aim(&mut self, diffusion: f32) {
        let (sin, cos) = (diffusion * std::f32::consts::FRAC_PI_4).sin_cos();
        self.angle[0].set_target(cos, Fdn::<LINES>::TICK);
        self.angle[1].set_target(sin, Fdn::<LINES>::TICK);
    }

    pub fn tick(&mut self, diffusion: f32, movement: f32, sample_rate: f32) {
        self.aim(diffusion);
        self.rotation.tick(&mut self.network, movement, sample_rate);
    }

    /// Clears the audio state and settles on the controls.
    pub fn reset(&mut self, diffusion: f32, movement: f32) {
        self.aim(diffusion);
        self.angle.iter_mut().for_each(LinearRamp::snap);
        self.rotation.reset(&mut self.network, movement);
        self.steps.iter_mut().for_each(DiffuserStep::reset);
        self.network.reset();
    }

    /// New absorption for the lengths as they are now; whether they still
    /// move.
    pub fn absorb(
        &mut self,
        decay: f32,
        high: f32,
        crossover_hz: f32,
        sample_rate: f32,
        ramp: u32,
    ) -> bool {
        absorb(
            &mut self.network,
            0.0,
            decay,
            high,
            crossover_hz,
            sample_rate,
            ramp,
        )
    }

    pub fn lengths(&self) -> &[f64; LINES] {
        self.network.current_lengths()
    }

    /// One sample: the late and the early output, each left and right.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> ([f32; 2], [f32; 2]) {
        let [cos, sin] = self.angle.each_mut().map(LinearRamp::next);
        let mut channels: [f32; CHANNELS] =
            std::array::from_fn(|index| 0.5 * if index % 2 == 0 { left } else { right });
        for step in &mut self.steps[..3] {
            channels = step.process(&channels, cos, sin);
        }
        let mut early = [0.0; 2];
        for (index, value) in channels.iter().enumerate() {
            early[index % 2] += STUDIO_EARLY_GAIN * value;
        }
        channels = self.steps[3].process(&channels, cos, sin);
        // Each channel into two lines, in opposite polarity: the energy
        // that enters is the input's.
        let feed: [f32; LINES] = std::array::from_fn(|line| {
            let gain = if line < CHANNELS {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                -std::f32::consts::FRAC_1_SQRT_2
            };
            gain * channels[line % CHANNELS]
        });
        (pick_up(&self.network.process(&feed)), early)
    }
}

/// A medium room: two allpass diffusers per channel into a sixteen-line
/// network whose feedback matrix scatters, so that the echo density that a
/// plain network needs 150 ms and more for is there within tens.
#[derive(Clone, Debug)]
pub(super) struct Room {
    diffusers: [[DelayAllpass; 2]; 2],
    network: Fdn<LINES>,
    rotation: Rotation,
    /// Mean scattering delay in samples: part of every lap.
    extra: f32,
}

impl Room {
    pub fn new() -> Self {
        Self {
            diffusers: std::array::from_fn(|_| std::array::from_fn(|_| DelayAllpass::default())),
            network: Fdn::new(PERMUTATION, SIGNS),
            rotation: Rotation::default(),
            extra: 0.0,
        }
    }

    pub fn prepare(&mut self, sample_rate: f32) {
        let samples = |seconds: f32| (seconds * sample_rate).ceil() as usize;
        for (diffusers, seconds) in self.diffusers.iter_mut().zip(&ROOM_DIFFUSER_SECONDS) {
            for (diffuser, seconds) in diffusers.iter_mut().zip(seconds) {
                diffuser.allocate(samples(*seconds) + 1);
                diffuser.snap_length((seconds * sample_rate).round() as usize);
            }
        }
        self.network
            .allocate(samples(ROOM_LINE_SECONDS), sample_rate);
        self.network.allocate_scatter(
            samples(ROOM_SCATTER_SECONDS),
            samples(TAP_FADE_SECONDS) as u32,
            ROOM_SCATTER_PERMUTATION,
            ROOM_SCATTER_SIGNS,
        );
    }

    pub fn configure(&mut self, layout: &Layout, how: Move) {
        self.network.set_lengths(&layout.elements);
        self.network.set_scatter_delays(&layout.scatter);
        self.extra = layout.extra;
        self.rotation.configure(layout);
        if how == Move::Snap {
            self.network.snap_lengths();
        }
    }

    pub fn tick(&mut self, diffusion: f32, movement: f32, sample_rate: f32) {
        self.network
            .set_scatter_angle(diffusion * std::f32::consts::FRAC_PI_4);
        self.rotation.tick(&mut self.network, movement, sample_rate);
    }

    /// Clears the audio state and settles on the controls.
    pub fn reset(&mut self, diffusion: f32, movement: f32) {
        self.network
            .set_scatter_angle(diffusion * std::f32::consts::FRAC_PI_4);
        self.rotation.reset(&mut self.network, movement);
        self.diffusers
            .iter_mut()
            .flatten()
            .for_each(DelayAllpass::reset);
        self.network.reset();
    }

    pub fn absorb(
        &mut self,
        decay: f32,
        high: f32,
        crossover_hz: f32,
        sample_rate: f32,
        ramp: u32,
    ) -> bool {
        let extra = self.extra;
        absorb(
            &mut self.network,
            extra,
            decay,
            high,
            crossover_hz,
            sample_rate,
            ramp,
        )
    }

    pub fn lengths(&self) -> &[f64; LINES] {
        self.network.current_lengths()
    }

    /// One sample: the late output, left and right. `diffusion` is 0–1.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32, diffusion: f32) -> [f32; 2] {
        let g = ROOM_DIFFUSER_GAIN * diffusion;
        let mut diffused = [left, right];
        for (signal, diffusers) in diffused.iter_mut().zip(&mut self.diffusers) {
            for diffuser in diffusers {
                *signal = diffuser.process(*signal, g);
            }
        }
        let feed = std::array::from_fn(|line| INJECTION[line] * diffused[line % 2]);
        pick_up(&self.network.process(&feed))
    }
}
