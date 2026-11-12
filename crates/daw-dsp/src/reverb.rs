//! Reverberation: a 16-line feedback delay network after Jot with a
//! Hadamard matrix and per-line absorption ([`Fdn16`]), and the complete
//! stereo reverb built around it ([`ReverbKernel`]) with pre-delay, allpass
//! diffusion, image-source early reflections and output filters.
use super::{
    CutoffGlide, DelayAllpass, DelayLine, FadeTap, LinearRamp, Shelf1, Smoothed, Svf, flush,
};

pub const LINES: usize = 16;

/// Multiplies by the 16 × 16 Hadamard matrix scaled to be orthogonal (and
/// its own inverse): four passes of sums and differences, no multiplies
/// until the final exact scaling.
#[inline]
pub fn hadamard16(values: &mut [f32; LINES]) {
    let mut half = 1;
    while half < LINES {
        for start in (0..LINES).step_by(2 * half) {
            for index in start..start + half {
                let (a, b) = (values[index], values[index + half]);
                values[index] = a + b;
                values[index + half] = a - b;
            }
        }
        half *= 2;
    }
    for value in values {
        *value *= 0.25;
    }
}

/// Sign of an entry of the Hadamard matrix.
const fn hadamard_sign(row: usize, column: usize) -> f32 {
    if (row & column).count_ones().is_multiple_of(2) {
        1.0
    } else {
        -1.0
    }
}

/// The prime closest to `n` (the smaller one on a tie), at least 2. Trial
/// division: for delay lengths at control rate.
pub fn nearest_prime(n: u32) -> u32 {
    let is_prime = |n: u32| {
        n >= 2
            && (2..)
                .take_while(|d| d * d <= n)
                .all(|d| !n.is_multiple_of(d))
    };
    (0..)
        .flat_map(|distance| [n.saturating_sub(distance), n.saturating_add(distance)])
        .find(|candidate| is_prime(*candidate))
        .unwrap_or(2)
}

/// Feedback routing: line `i` is fed from the absorbed output of line
/// `PERMUTATION[i]` with `SIGNS[i]`, then all are mixed by the Hadamard
/// matrix. The permutation keeps a line's own output out of its input's
/// largest term; any choice is lossless.
const PERMUTATION: [usize; LINES] = [7, 12, 1, 10, 4, 15, 2, 9, 0, 13, 6, 11, 3, 14, 5, 8];
const SIGNS: [f32; LINES] = [
    1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0,
];
/// The odd lines are the modulated ones.
const MODULATED: usize = LINES / 2;
const GLIDE_SECONDS: f32 = 0.1;
/// Samples of length per sample.
const SLOWEST_GLIDE: f32 = 1.0 / 256.0;

/// The core of a reverb: sixteen delay lines whose outputs pass a
/// first-order absorption filter each and return through an orthogonal
/// matrix. With identity absorption it neither gains nor loses energy; the
/// absorption alone sets the decay, per frequency (see
/// [`absorption_shelf`]). Lengths are primes, so the lines' echoes never
/// coincide; a length change glides, and half of the lines can be modulated
/// slowly to blur the modes.
#[derive(Clone, Debug)]
pub struct Fdn16 {
    lines: [DelayLine; LINES],
    absorb: [Shelf1; LINES],
    /// In samples; equals the target when at rest.
    length: [f32; LINES],
    target: [u32; LINES],
    max_length: usize,
    glide: f32,
    /// Sine and cosine of each modulated line's oscillator.
    wobble: [(f32, f32); MODULATED],
    wobble_step: [f32; MODULATED],
    /// In samples.
    wobble_depth: Smoothed,
}

impl Default for Fdn16 {
    fn default() -> Self {
        let mut network = Self {
            lines: std::array::from_fn(|_| DelayLine::default()),
            absorb: std::array::from_fn(|_| Shelf1::default()),
            length: [2.0; LINES],
            target: [2; LINES],
            max_length: 0,
            glide: 1.0,
            wobble: [(0.0, 1.0); MODULATED],
            wobble_step: [0.0; MODULATED],
            wobble_depth: Smoothed::new(0.0),
        };
        network.reset();
        network
    }
}

impl Fdn16 {
    /// Room for lines of `max_length` samples, modulation included. Call
    /// from `prepare` only.
    pub fn allocate(&mut self, max_length: usize, sample_rate: f32) {
        for line in &mut self.lines {
            line.allocate(max_length);
        }
        self.max_length = max_length;
        self.glide = 1.0 - (-1.0 / (GLIDE_SECONDS * sample_rate)).exp();
        self.wobble_depth.set_time(50.0, sample_rate);
    }

    /// Clears the audio state; lengths and absorption stay.
    pub fn reset(&mut self) {
        for (line, absorb) in self.lines.iter_mut().zip(&mut self.absorb) {
            line.reset();
            absorb.reset();
        }
        for (index, wobble) in self.wobble.iter_mut().enumerate() {
            let (sin, cos) = (std::f32::consts::TAU * index as f32 / MODULATED as f32).sin_cos();
            *wobble = (sin, cos);
        }
        self.wobble_depth.snap();
    }

    /// New line lengths in samples, each rounded to the nearest prime. The
    /// lines glide there, which bends the pitch of what they hold instead of
    /// cutting it.
    pub fn set_lengths(&mut self, samples: &[f32; LINES]) {
        // Room above the longest line for the modulation and the
        // interpolator's taps.
        let longest = (0.98 * self.max_length as f32).max(2.0);
        for (target, samples) in self.target.iter_mut().zip(samples) {
            *target = nearest_prime(samples.round().clamp(2.0, longest) as u32);
        }
    }

    /// Jumps to the set lengths.
    pub fn snap_lengths(&mut self) {
        self.length = self.target.map(|length| length as f32);
    }

    pub fn is_gliding(&self) -> bool {
        self.length
            .iter()
            .zip(&self.target)
            .any(|(length, target)| *length != *target as f32)
    }

    /// The lengths the lines are at or gliding to.
    pub fn lengths(&self) -> &[u32; LINES] {
        &self.target
    }

    /// The lengths now, on the way to [`Self::lengths`].
    pub fn current_lengths(&self) -> &[f32; LINES] {
        &self.length
    }

    /// Takes the filter's coefficients for a line and keeps its state.
    pub fn set_absorption(&mut self, line: usize, filter: &Shelf1) {
        self.absorb[line].set(filter);
    }

    /// Sine modulation of the odd lines' lengths by `depth_samples`, each at
    /// its own rate.
    pub fn set_modulation(
        &mut self,
        depth_samples: f32,
        rates_hz: &[f32; MODULATED],
        sample_rate: f32,
    ) {
        self.wobble_depth.set_target(depth_samples.max(0.0));
        for (step, rate) in self.wobble_step.iter_mut().zip(rates_hz) {
            *step = 2.0 * (std::f32::consts::PI * rate / sample_rate).sin();
        }
    }

    /// One sample: `input` is added to what each line stores; the lines'
    /// outputs are returned, for the caller to pick up.
    #[inline]
    pub fn process(&mut self, input: &[f32; LINES]) -> [f32; LINES] {
        let depth = self.wobble_depth.next_value();
        let mut outputs = [0.0; LINES];
        for (index, output) in outputs.iter_mut().enumerate() {
            let target = self.target[index] as f32;
            let length = self.length[index];
            let modulated = index % 2 == 1 && depth > 0.0;
            *output = if length == target && !modulated {
                // At rest a line is read at a whole tap: nothing is lost to
                // interpolation in the loop.
                self.lines[index].tap(self.target[index] as usize - 1)
            } else {
                let mut position = length - 1.0;
                if modulated {
                    position += depth * self.wobble[index / 2].0;
                }
                let position = position.max(1.0);
                let whole = position.floor();
                self.lines[index].read_at(whole as usize, position - whole)
            };
            if length != target {
                // The exponential approach ends in a slow constant slide: its
                // steps would fall below what an `f32` length can resolve.
                let distance = target - length;
                let step = self.glide * distance;
                self.length[index] = if distance.abs() <= SLOWEST_GLIDE {
                    target
                } else if step.abs() < SLOWEST_GLIDE {
                    length + SLOWEST_GLIDE.copysign(distance)
                } else {
                    length + step
                };
            }
        }
        for ((sin, cos), step) in self.wobble.iter_mut().zip(&self.wobble_step) {
            // A rotation that keeps its amplitude for any step.
            *sin += step * *cos;
            *cos -= step * *sin;
        }
        let mut mixed = [0.0; LINES];
        for index in 0..LINES {
            let source = PERMUTATION[index];
            mixed[index] = SIGNS[index] * self.absorb[source].process(outputs[source]);
        }
        hadamard16(&mut mixed);
        for (line, (mixed, input)) in self.lines.iter_mut().zip(mixed.iter().zip(input)) {
            line.write(flush(mixed + input));
        }
        outputs
    }
}

/// The absorption filter of a delay line of `length` samples inside a
/// feedback delay network that should decay by 60 dB in `t60_mid` seconds at
/// 1 kHz and in `t60_high` seconds (not longer) at the top of the band: a
/// shelf centred on `crossover_hz` whose attenuation is proportional to the
/// line's length. Exact at 1 kHz and at the top; below 1 kHz the decay is
/// longer, but never more than twice `t60_mid`, and that bound takes
/// precedence where a first-order section cannot have both (1 kHz then
/// decays faster than asked). Never above unit gain at any frequency.
pub fn absorption_shelf(
    length: u32,
    t60_mid: f32,
    t60_high: f32,
    crossover_hz: f32,
    sample_rate: f32,
) -> Shelf1 {
    let rate = f64::from(sample_rate);
    let t60_mid = f64::from(t60_mid).max(1e-3);
    let t60_high = f64::from(t60_high).clamp(1e-3, t60_mid);
    let gain = |t60: f64| 10.0_f64.powf(-3.0 * f64::from(length) / (rate * t60));
    let (mid, high) = (gain(t60_mid), gain(t60_high));
    let warp = |hz: f64| (std::f64::consts::PI * (hz / rate).clamp(1e-6, 0.45)).tan();
    let crossover = warp(f64::from(crossover_hz));
    let q = (warp(1000.0) / crossover).powi(2);
    // With pole and zero centred on the crossover, the magnitude at 1 kHz is
    // `low·high·(low + q·high)/(high + q·low)`, squared; solved for `low`.
    let b = q * (high * high - mid * mid);
    let low = (-b + (b * b + 4.0 * high * high * mid * mid).sqrt()) / (2.0 * high);
    let low = low.min(gain(2.0 * t60_mid));
    Shelf1::with_gains(
        (crossover * (high / low).sqrt()) as f32,
        low as f32,
        high as f32,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReverbType {
    Room,
    Chamber,
    Hall,
    Plate,
}

/// What a type is: no knob moves with it.
struct Character {
    /// Scales the mean line length.
    length: f32,
    crossover_hz: f32,
    movement_hz: f32,
    diffusion: f32,
    early_gain: f32,
    /// Width, depth and height in metres at Size 50 %; a plate has no room.
    room: Option<[f32; 3]>,
    /// Offsets of the lines' places on the length scale, frozen from
    /// `Random` seeds 11–14.
    jitter: [f32; LINES],
}

impl ReverbType {
    fn character(self) -> &'static Character {
        match self {
            Self::Room => &Character {
                length: 0.8,
                crossover_hz: 5000.0,
                movement_hz: 0.8,
                diffusion: 0.85,
                early_gain: 0.708,
                room: Some([5.0, 4.0, 2.7]),
                jitter: [
                    -0.3, -0.197, -0.257, 0.298, -0.061, -0.134, -0.175, 0.059, 0.297, 0.037,
                    0.175, 0.127, -0.012, -0.073, 0.299, 0.212,
                ],
            },
            Self::Chamber => &Character {
                length: 1.0,
                crossover_hz: 6000.0,
                movement_hz: 0.6,
                diffusion: 1.0,
                early_gain: 0.501,
                room: Some([8.0, 6.0, 4.0]),
                jitter: [
                    -0.3, -0.187, 0.275, -0.127, 0.179, -0.16, -0.164, 0.127, 0.076, 0.064, -0.188,
                    0.046, -0.245, 0.249, 0.074, -0.272,
                ],
            },
            Self::Hall => &Character {
                length: 1.2,
                crossover_hz: 4000.0,
                movement_hz: 0.4,
                diffusion: 1.0,
                early_gain: 0.355,
                room: Some([28.0, 18.0, 12.0]),
                jitter: [
                    -0.3, -0.178, -0.055, -0.085, -0.143, -0.244, -0.226, 0.112, 0.204, -0.172,
                    0.116, 0.174, 0.084, 0.26, 0.062, 0.155,
                ],
            },
            Self::Plate => &Character {
                length: 0.6,
                crossover_hz: 8000.0,
                movement_hz: 1.0,
                diffusion: 1.1,
                early_gain: 0.0,
                room: None,
                jitter: [
                    -0.299, -0.169, 0.217, -0.201, -0.189, -0.274, 0.193, 0.239, 0.244, 0.173,
                    -0.059, -0.047, -0.275, -0.097, 0.066, 0.004,
                ],
            },
        }
    }
}

/// Plain-unit settings of a [`ReverbKernel`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbSettings {
    pub kind: ReverbType,
    /// 0–1.
    pub size: f32,
    /// Decay time at 1 kHz.
    pub decay_seconds: f32,
    pub pre_delay_seconds: f32,
    /// 0–1: how much faster the top decays.
    pub damping: f32,
    /// Off at [`CUT_OFF_LOW_HZ`] and below.
    pub low_cut_hz: f32,
    /// Off at [`CUT_OFF_HIGH_HZ`] and above.
    pub high_cut_hz: f32,
    /// 0–1.
    pub diffusion: f32,
    /// 0–1.
    pub movement: f32,
    /// 0 (mono) – 1 (as the network gives it) – 1.5.
    pub width: f32,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            kind: ReverbType::Hall,
            size: 0.5,
            decay_seconds: 1.8,
            pre_delay_seconds: 0.015,
            damping: 0.4,
            low_cut_hz: 100.0,
            high_cut_hz: 12_000.0,
            diffusion: 0.8,
            movement: 0.25,
            width: 1.0,
        }
    }
}

/// A low cut at or below this, and a high cut at or above the other, is
/// switched off: the signal passes untouched.
pub const CUT_OFF_LOW_HZ: f32 = 20.0;
pub const CUT_OFF_HIGH_HZ: f32 = 20_000.0;

pub const MAX_PRE_DELAY_SECONDS: f32 = 0.25;
pub const EARLY_TAPS: usize = 8;
const MAX_EARLY_SECONDS: f32 = 0.12;
const MAX_LINE_SECONDS: f32 = 0.15;
const SHORTEST_MEAN_SECONDS: f32 = 0.008;
/// The mean line length grows by this factor over the Size range.
const SIZE_RANGE: f32 = 70.0 / 8.0;
/// Mean line length at which the network's level needs no correction.
const REFERENCE_MEAN_SECONDS: f32 = 0.024;
const MOVEMENT_SECONDS: f32 = 0.001;
const SHORTEST_HIGH_DECAY_SECONDS: f32 = 0.05;
const SPEED_OF_SOUND: f32 = 343.0;
const WALL_REFLECTION: f32 = 0.8;
const TAP_FADE_SECONDS: f32 = 0.03;
const TICK: u32 = 32;

const DIFFUSER_SECONDS: [[f32; 4]; 2] = [
    [0.004_77, 0.003_60, 0.012_73, 0.009_31],
    [0.005_03, 0.003_83, 0.013_37, 0.009_87],
];
const DIFFUSER_GAINS: [f32; 4] = [0.75, 0.75, 0.625, 0.625];
const MAX_DIFFUSER_GAIN: f32 = 0.8;

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

/// The network's line lengths in seconds for a type and size, and their
/// mean: sixteen lengths spread geometrically over a ratio of about 1:3 and
/// jittered, so that no two are simply related.
fn line_seconds(kind: ReverbType, size: f32) -> ([f32; LINES], f32) {
    let character = kind.character();
    let mean = character.length * SHORTEST_MEAN_SECONDS * SIZE_RANGE.powf(size.clamp(0.0, 1.0));
    let mut ratios: [f32; LINES] = std::array::from_fn(|index| {
        let place = (index as f32 + character.jitter[index]) / (LINES - 1) as f32;
        0.55 * (1.6_f32 / 0.55).powf(place)
    });
    let scale = mean * LINES as f32 / ratios.iter().sum::<f32>();
    for ratio in &mut ratios {
        *ratio *= scale;
    }
    (ratios, mean)
}

fn line_lengths(kind: ReverbType, size: f32, sample_rate: f32) -> [u32; LINES] {
    let longest = 0.98 * MAX_LINE_SECONDS * sample_rate;
    line_seconds(kind, size)
        .0
        .map(|seconds| nearest_prime((seconds * sample_rate).round().clamp(2.0, longest) as u32))
}

fn high_decay_seconds(settings: &ReverbSettings) -> f32 {
    (settings.decay_seconds * 0.1_f32.powf(settings.damping))
        .max(SHORTEST_HIGH_DECAY_SECONDS)
        .min(settings.decay_seconds)
}

/// What a modulated line loses per pass to its interpolated read, in
/// decibels, averaged over the sine that moves it by `depth_samples` either
/// way. Nothing at rest; from half a sample on the read passes every
/// fraction and the loss no longer grows.
fn movement_loss_db(depth_samples: f32, omega: f64) -> f64 {
    const STEPS: usize = 32;
    if depth_samples <= 0.0 {
        return 0.0;
    }
    let sum: f64 = (0..STEPS)
        .map(|step| {
            let share = (step as f32 + 0.5) / STEPS as f32;
            let fraction = if depth_samples < 0.5 {
                let position = depth_samples * (std::f32::consts::PI * (share - 0.5)).sin();
                position - position.floor()
            } else {
                share
            };
            -20.0 * DelayLine::lagrange_response(fraction, omega).norm().log10()
        })
        .sum();
    sum / STEPS as f64
}

/// The decay time the network really has at a frequency: what the lines'
/// absorption filters and, with Movement, the interpolated reads of the
/// modulated lines add up to. `decay_seconds` at 1 kHz, shorter above,
/// somewhat longer below.
pub fn decay_seconds_at(settings: &ReverbSettings, hz: f32, sample_rate: f32) -> f32 {
    let character = settings.kind.character();
    let omega = std::f64::consts::TAU * f64::from(hz / sample_rate).min(0.5);
    let lengths = line_lengths(settings.kind, settings.size, sample_rate);
    // A signal spends time in each line in proportion to its length, so the
    // losses of all lines are spread over the sum of the lengths.
    let (mut loss_db, mut samples) = (0.0, 0.0);
    let read_loss_db = movement_loss_db(settings.movement * MOVEMENT_SECONDS * sample_rate, omega);
    for (line, length) in lengths.into_iter().enumerate() {
        if line % 2 == 1 {
            loss_db += read_loss_db;
        }
        let shelf = absorption_shelf(
            length,
            settings.decay_seconds,
            high_decay_seconds(settings),
            character.crossover_hz,
            sample_rate,
        );
        loss_db -= 20.0 * shelf.response(omega).norm().log10();
        samples += f64::from(length);
    }
    (60.0 * samples / (loss_db.max(1e-9) * f64::from(sample_rate))) as f32
}

/// The early reflections of a type at a size, per channel, as
/// `(seconds after the direct sound, gain)`, earliest first: the eight
/// first images of a source in a shoebox room, heard at two points 20 cm
/// apart. All zero for a plate.
pub fn early_taps(kind: ReverbType, size: f32, out: &mut [[(f32, f32); EARLY_TAPS]; 2]) {
    *out = [[(0.0, 0.0); EARLY_TAPS]; 2];
    let character = kind.character();
    let Some(room) = character.room else {
        return;
    };
    let scale = SIZE_RANGE.powf(size.clamp(0.0, 1.0) - 0.5);
    let room = room.map(|side| side * scale);
    let source = [0.30 * room[0], 0.40 * room[1], 0.35 * room[2]];
    for (channel, taps) in out.iter_mut().enumerate() {
        let side = if channel == 0 { -0.1 } else { 0.1 };
        let receiver = [0.60 * room[0] + side, 0.55 * room[1], 0.40 * room[2]];
        let distance = |image: [f32; 3]| {
            (0..3)
                .map(|axis| (image[axis] - receiver[axis]).powi(2))
                .sum::<f32>()
                .sqrt()
        };
        let direct = distance(source);
        let mut images = [(f32::MAX, 0.0); 26];
        let mut count = 0;
        for code in 0..27 {
            // One reflection or none per axis: −1 mirrors in the wall at 0,
            // +1 in the opposite one.
            let index = [code % 3, code / 3 % 3, code / 9].map(|digit| digit - 1);
            let order: i32 = index.iter().map(|digit: &i32| digit.abs()).sum();
            if order == 0 {
                continue;
            }
            let image: [f32; 3] = std::array::from_fn(|axis| match index[axis] {
                -1 => -source[axis],
                1 => 2.0 * room[axis] - source[axis],
                _ => source[axis],
            });
            let path = distance(image);
            images[count] = (
                ((path - direct) / SPEED_OF_SOUND).min(MAX_EARLY_SECONDS),
                character.early_gain * direct / path * WALL_REFLECTION.powi(order),
            );
            count += 1;
        }
        images.sort_by(|a, b| a.0.total_cmp(&b.0));
        taps.copy_from_slice(&images[..EARLY_TAPS]);
    }
}

/// A complete stereo reverb, wet signal only: pre-delay, four allpass
/// diffusers per channel, the [`Fdn16`] with decay and damping, early
/// reflections beside it, low and high cut, and width.
#[derive(Clone, Debug)]
pub struct ReverbKernel {
    sample_rate: f32,
    settings: ReverbSettings,
    pre: [DelayLine; 2],
    pre_tap: [FadeTap; 2],
    early: [[(FadeTap, LinearRamp); EARLY_TAPS]; 2],
    diffusers: [[DelayAllpass; 4]; 2],
    diffusion: Smoothed,
    network: Fdn16,
    low_cut: [Svf; 2],
    high_cut: [Svf; 2],
    low_glide: CutoffGlide,
    high_glide: CutoffGlide,
    decay: Smoothed,
    damping: Smoothed,
    /// Decay and damping the absorption filters were last made for.
    absorbed: (f32, f32),
    absorb_dirty: bool,
    level: LinearRamp,
    width: Smoothed,
    tick: u32,
}

impl Default for ReverbKernel {
    fn default() -> Self {
        let settings = ReverbSettings::default();
        Self {
            sample_rate: 48_000.0,
            settings,
            pre: std::array::from_fn(|_| DelayLine::default()),
            pre_tap: [FadeTap::default(); 2],
            early: [[(FadeTap::default(), LinearRamp::new(0.0)); EARLY_TAPS]; 2],
            diffusers: std::array::from_fn(|_| std::array::from_fn(|_| DelayAllpass::default())),
            diffusion: Smoothed::new(0.0),
            network: Fdn16::default(),
            low_cut: [Svf::default(), Svf::default()],
            high_cut: [Svf::default(), Svf::default()],
            low_glide: CutoffGlide::new(settings.low_cut_hz),
            high_glide: CutoffGlide::new(settings.high_cut_hz),
            decay: Smoothed::new(settings.decay_seconds),
            damping: Smoothed::new(settings.damping),
            absorbed: (0.0, 0.0),
            absorb_dirty: true,
            level: LinearRamp::new(1.0),
            width: Smoothed::new(1.0),
            tick: 0,
        }
    }
}

impl ReverbKernel {
    /// Allocates for a sample rate and resets.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let samples = |seconds: f32| (seconds * sample_rate).ceil() as usize;
        for line in &mut self.pre {
            line.allocate(samples(
                MAX_PRE_DELAY_SECONDS + MAX_EARLY_SECONDS + TAP_FADE_SECONDS,
            ));
        }
        for (diffusers, seconds) in self.diffusers.iter_mut().zip(&DIFFUSER_SECONDS) {
            for (diffuser, seconds) in diffusers.iter_mut().zip(seconds) {
                diffuser.allocate(samples(*seconds) + 1);
                diffuser.set_length((seconds * sample_rate).round() as usize);
            }
        }
        self.network
            .allocate(samples(MAX_LINE_SECONDS), sample_rate);
        let fade = samples(TAP_FADE_SECONDS) as u32;
        for tap in self
            .pre_tap
            .iter_mut()
            .chain(self.early.iter_mut().flatten().map(|(tap, _)| tap))
        {
            tap.set_fade(fade);
        }
        self.diffusion.set_time(20.0, sample_rate);
        self.decay.set_time(50.0, sample_rate);
        self.damping.set_time(50.0, sample_rate);
        self.width.set_time(20.0, sample_rate);
        self.low_glide.prepare(20.0, sample_rate);
        self.high_glide.prepare(20.0, sample_rate);
        self.reset();
    }

    /// New targets; everything glides, fades or ramps there.
    pub fn set(&mut self, settings: &ReverbSettings) {
        self.settings = *settings;
        let rate = self.sample_rate;
        let character = settings.kind.character();
        let (seconds, mean) = line_seconds(settings.kind, settings.size);
        let before = *self.network.lengths();
        self.network
            .set_lengths(&seconds.map(|seconds| seconds * rate));
        self.absorb_dirty |= before != *self.network.lengths();
        let rates = std::array::from_fn(|line| {
            character.movement_hz * (0.95 + 0.1 * line as f32 / (MODULATED - 1) as f32)
        });
        self.network.set_modulation(
            settings.movement.clamp(0.0, 1.0) * MOVEMENT_SECONDS * rate,
            &rates,
            rate,
        );
        // The network is louder the shorter its lines are.
        let level = (mean / REFERENCE_MEAN_SECONDS).sqrt();
        let ramp = (GLIDE_SECONDS * rate) as u32;
        if self.level.target() != level {
            self.level.set_target(level, ramp);
        }
        let pre_delay = settings.pre_delay_seconds.clamp(0.0, MAX_PRE_DELAY_SECONDS);
        let mut taps = [[(0.0, 0.0); EARLY_TAPS]; 2];
        early_taps(settings.kind, settings.size, &mut taps);
        let fade = (TAP_FADE_SECONDS * rate) as u32;
        for (channel, taps) in taps.iter().enumerate() {
            self.pre_tap[channel].set((pre_delay * rate).round() as usize);
            for ((tap, gain), (seconds, target)) in self.early[channel].iter_mut().zip(*taps) {
                tap.set(((pre_delay + seconds) * rate).round() as usize);
                if gain.target() != target {
                    gain.set_target(target, fade);
                }
            }
        }
        self.diffusion
            .set_target(settings.diffusion.clamp(0.0, 1.0) * character.diffusion);
        self.decay.set_target(settings.decay_seconds.max(0.01));
        self.damping.set_target(settings.damping.clamp(0.0, 1.0));
        self.width.set_target(settings.width.clamp(0.0, 1.5));
        self.low_glide.set(settings.low_cut_hz);
        self.high_glide.set(settings.high_cut_hz);
    }

    /// Clears the audio state and settles on the settings.
    pub fn reset(&mut self) {
        let settings = self.settings;
        self.set(&settings);
        for line in &mut self.pre {
            line.reset();
        }
        for channel in 0..2 {
            let target = self.pre_tap[channel].target();
            self.pre_tap[channel].snap(target);
            for (tap, gain) in &mut self.early[channel] {
                let target = tap.target();
                tap.snap(target);
                gain.snap();
            }
            for diffuser in &mut self.diffusers[channel] {
                diffuser.reset();
            }
            self.low_cut[channel].reset();
            self.high_cut[channel].reset();
        }
        self.network.snap_lengths();
        self.network.reset();
        self.diffusion.snap();
        self.decay.snap();
        self.damping.snap();
        self.width.snap();
        self.level.snap();
        self.low_glide.snap();
        self.high_glide.snap();
        self.tick = 0;
        self.update_absorption();
    }

    fn update_absorption(&mut self) {
        let (decay, damping) = (self.decay.value(), self.damping.value());
        let high = (decay * 0.1_f32.powf(damping))
            .max(SHORTEST_HIGH_DECAY_SECONDS)
            .min(decay);
        let crossover = self.settings.kind.character().crossover_hz;
        let lengths = *self.network.current_lengths();
        for (line, length) in lengths.iter().enumerate() {
            let shelf = absorption_shelf(
                length.round() as u32,
                decay,
                high,
                crossover,
                self.sample_rate,
            );
            self.network.set_absorption(line, &shelf);
        }
        self.absorbed = (decay, damping);
        // One more pass after a glide has ended, on the final lengths.
        self.absorb_dirty = self.network.is_gliding();
    }

    /// One sample in, the wet signal out.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let (decay, damping) = (self.decay.next_value(), self.damping.next_value());
        if self.tick == 0 && (self.absorb_dirty || self.absorbed != (decay, damping)) {
            self.update_absorption();
        }
        self.tick = (self.tick + 1) % TICK;

        let diffusion = self.diffusion.next_value();
        let input = [left, right];
        let mut diffused = [0.0; 2];
        let mut early = [0.0; 2];
        for channel in 0..2 {
            let line = &mut self.pre[channel];
            line.write(flush(input[channel]));
            let mut signal = self.pre_tap[channel].read(line);
            for (diffuser, gain) in self.diffusers[channel].iter_mut().zip(DIFFUSER_GAINS) {
                signal = diffuser.process(signal, (diffusion * gain).min(MAX_DIFFUSER_GAIN));
            }
            diffused[channel] = signal;
            for (tap, gain) in &mut self.early[channel] {
                early[channel] += gain.next() * tap.read(line);
            }
        }
        let feed = std::array::from_fn(|line| INJECTION[line] * diffused[line % 2]);
        let outputs = self.network.process(&feed);
        let level = self.level.next();
        let (low_g, high_g) = (self.low_glide.next_g(), self.high_glide.next_g());
        let low_off = self.settings.low_cut_hz <= CUT_OFF_LOW_HZ && self.low_glide.is_settled();
        let high_off = self.settings.high_cut_hz >= CUT_OFF_HIGH_HZ && self.high_glide.is_settled();
        let mut wet = [0.0; 2];
        for channel in 0..2 {
            let late: f32 = outputs
                .iter()
                .zip(&PICK_UP[channel])
                .map(|(output, gain)| output * gain)
                .sum();
            let mut signal = level * late + early[channel];
            // The filters run while they are off, so they come in on a
            // signal they already know.
            let cut = self.low_cut[channel]
                .process(signal, low_g, std::f32::consts::SQRT_2)
                .high;
            if !low_off {
                signal = cut;
            }
            let cut = self.high_cut[channel]
                .process(signal, high_g, std::f32::consts::SQRT_2)
                .low;
            if !high_off {
                signal = cut;
            }
            wet[channel] = signal;
        }
        let width = self.width.next_value();
        let (mid, side) = (0.5 * (wet[0] + wet[1]), 0.5 * (wet[0] - wet[1]) * width);
        (mid + side, mid - side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Random;

    #[test]
    fn hadamard_is_orthogonal_and_its_own_inverse() {
        let mut random = Random::new(3);
        let original: [f32; LINES] = std::array::from_fn(|_| random.bipolar());
        let mut values = original;
        hadamard16(&mut values);
        let energy =
            |values: &[f32; LINES]| values.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        assert!((energy(&values) / energy(&original) - 1.0).abs() < 1e-6);
        assert!(
            values
                .iter()
                .zip(&original)
                .any(|(a, b)| (a - b).abs() > 0.1)
        );
        // A unit vector spreads evenly: every entry is ±1/4.
        let mut unit = [0.0; LINES];
        unit[5] = 1.0;
        hadamard16(&mut unit);
        for (column, value) in unit.iter().enumerate() {
            assert_eq!(*value, 0.25 * hadamard_sign(5, column));
        }
        hadamard16(&mut values);
        for (a, b) in values.iter().zip(&original) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn nearest_prime_searches_both_ways() {
        let cases = [
            (0, 2),
            (1, 2),
            (2, 2),
            (9, 7),
            (14, 13),
            (15, 13),
            (24, 23),
            (90, 89),
        ];
        for (n, prime) in cases {
            assert_eq!(nearest_prime(n), prime, "{n}");
        }
        assert_eq!(nearest_prime(7919), 7919);
        assert_eq!(nearest_prime(28_800), 28_793);
    }

    fn network(lengths: &[f32; LINES]) -> Fdn16 {
        let mut network = Fdn16::default();
        network.allocate(4096, 48_000.0);
        network.set_lengths(lengths);
        network.snap_lengths();
        network
    }

    const LENGTHS: [f32; LINES] = [
        601.0, 683.0, 743.0, 811.0, 877.0, 947.0, 1019.0, 1087.0, 1163.0, 1229.0, 1301.0, 1381.0,
        1453.0, 1531.0, 1607.0, 1693.0,
    ];

    /// The energy the lines hold now: each stored sample leaves its line
    /// exactly once, within the line's length.
    fn stored_energy(network: &mut Fdn16) -> f64 {
        let lengths = *network.lengths();
        let mut energy = 0.0;
        for n in 0..*lengths.iter().max().unwrap() {
            let outputs = network.process(&[0.0; LINES]);
            for (output, length) in outputs.iter().zip(&lengths) {
                if n < *length {
                    energy += f64::from(*output).powi(2);
                }
            }
        }
        energy
    }

    #[test]
    fn network_without_absorption_keeps_its_energy() {
        let mut network = network(&LENGTHS);
        assert!(network.lengths().iter().all(|n| nearest_prime(*n) == *n));
        let mut random = Random::new(8);
        let impulse: [f32; LINES] = std::array::from_fn(|_| random.bipolar());
        let energy_in: f64 = impulse.iter().map(|x| f64::from(*x).powi(2)).sum();
        network.process(&impulse);
        for _ in 0..480_000 {
            network.process(&[0.0; LINES]);
        }
        let difference = 10.0 * (stored_energy(&mut network) / energy_in).log10();
        assert!(difference.abs() < 0.01, "{difference} dB after 10 s");
    }

    #[test]
    fn absorption_sets_the_decay() {
        let mut network = network(&LENGTHS);
        // Half the amplitude per pass through a line of 1147 samples on
        // average: 6.02 dB per 23.9 ms.
        for line in 0..LINES {
            network.set_absorption(line, &Shelf1::with_gains(1.0, 0.5, 0.5));
        }
        let mut impulse = [0.0; LINES];
        impulse[0] = 1.0;
        network.process(&impulse);
        let before = stored_energy(&mut network);
        for _ in 0..4800 {
            network.process(&[0.0; LINES]);
        }
        let after = stored_energy(&mut network);
        let passes = (4800.0 + 1693.0) / 1147.0;
        let decay_db = 10.0 * (before / after).log10();
        assert!(
            (decay_db / (6.02 * passes) - 1.0).abs() < 0.15,
            "{decay_db} dB for {passes} passes"
        );
    }

    #[test]
    fn length_changes_glide_and_modulation_stays_smooth() {
        let mut network = network(&LENGTHS);
        for line in 0..LINES {
            network.set_absorption(line, &Shelf1::with_gains(1.0, 0.6, 0.6));
        }
        let mut n = 0;
        let mut largest_step = |network: &mut Fdn16, samples: usize| {
            let mut previous = [0.0; LINES];
            let mut largest = 0.0_f32;
            for index in 0..samples {
                let x = 0.1 * (0.03 * n as f32).sin();
                n += 1;
                let outputs = network.process(&[x; LINES]);
                if index > 0 {
                    for (output, previous) in outputs.iter().zip(&previous) {
                        largest = largest.max((output - previous).abs());
                    }
                }
                previous = outputs;
            }
            largest
        };
        let steady = largest_step(&mut network, 48_000);
        network.set_lengths(&LENGTHS.map(|length| 0.5 * length));
        assert!(network.is_gliding());
        let gliding = largest_step(&mut network, 48_000);
        assert!(!network.is_gliding());
        assert!(
            network
                .lengths()
                .iter()
                .zip(&LENGTHS)
                .all(|(a, b)| (*a as f32) < 0.6 * b)
        );
        assert!(gliding <= 1.5 * steady + 0.01, "{gliding} against {steady}");
        network.set_modulation(40.0, &[0.5; MODULATED], 48_000.0);
        let modulated = largest_step(&mut network, 96_000);
        assert!(
            modulated <= 1.5 * steady + 0.01,
            "{modulated} against {steady}"
        );
    }

    #[test]
    fn absorption_shelf_hits_the_decay_at_1_khz_and_stays_passive() {
        for rate in [44_100.0_f32, 48_000.0, 96_000.0] {
            let omega = |hz: f64| std::f64::consts::TAU * hz / f64::from(rate);
            for length in [100, 733, 4001, 20_000] {
                for decay in [0.1_f32, 0.5, 1.8, 6.0, 20.0] {
                    for damping in [0.0_f32, 0.25, 0.4, 0.7, 1.0] {
                        for crossover in [4000.0, 5000.0, 6000.0, 8000.0] {
                            let high = (decay * 0.1_f32.powf(damping)).max(0.05).min(decay);
                            let shelf = absorption_shelf(length, decay, high, crossover, rate);
                            let label =
                                format!("{rate} Hz, {length}, {decay} s, {damping}, {crossover}");
                            for step in 0..=64 {
                                let gain = shelf
                                    .response(std::f64::consts::PI * f64::from(step) / 64.0)
                                    .norm();
                                assert!(gain <= 1.0, "{label}: gain {gain}");
                            }
                            let per_pass =
                                |t60: f64| -60.0 * f64::from(length) / (f64::from(rate) * t60);
                            // The bass never rings longer than twice Decay.
                            let low_db = 20.0 * f64::from(shelf.low_gain).log10();
                            assert!(low_db <= per_pass(2.0 * f64::from(decay)) + 1e-4, "{label}");
                            let capped = (low_db - per_pass(2.0 * f64::from(decay))).abs() < 1e-4;
                            let mid_db = 20.0 * shelf.response(omega(1000.0)).norm().log10();
                            let wanted = per_pass(f64::from(decay));
                            if capped {
                                assert!(mid_db <= wanted + 1e-3, "{label}");
                            } else {
                                let tolerance = 0.001_f64.max(1e-4 * wanted.abs());
                                assert!(
                                    (mid_db - wanted).abs() < tolerance,
                                    "{label}: {mid_db} dB, wanted {wanted}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn decay_follows_the_knobs_at_each_frequency() {
        let settings = ReverbSettings::default();
        let rate = 48_000.0;
        let mid = decay_seconds_at(&settings, 1000.0, rate);
        assert!((mid / 1.8 - 1.0).abs() < 0.01, "{mid}");
        let low = decay_seconds_at(&settings, 60.0, rate);
        let high = decay_seconds_at(&settings, 16_000.0, rate);
        assert!(
            low > mid && low < 3.6 && high < 0.6 * mid,
            "{low} {mid} {high}"
        );
        let flat = ReverbSettings {
            damping: 0.0,
            ..settings
        };
        let still = ReverbSettings {
            movement: 0.0,
            ..flat
        };
        for hz in [60.0, 1000.0, 16_000.0] {
            let seconds = decay_seconds_at(&still, hz, rate);
            assert!((seconds / 1.8 - 1.0).abs() < 0.01, "{hz}: {seconds}");
        }
        // Movement costs the top, by the same amount from half a sample on.
        let moving = decay_seconds_at(&flat, 8000.0, rate);
        assert!(moving > 1.5 && moving < 1.75, "{moving}");
        let full = ReverbSettings {
            movement: 1.0,
            ..flat
        };
        assert!((decay_seconds_at(&full, 8000.0, rate) / moving - 1.0).abs() < 0.02);
        assert!((decay_seconds_at(&flat, 1000.0, rate) / 1.8 - 1.0).abs() < 0.01);
    }

    #[test]
    fn line_lengths_scale_with_size_and_type_and_fit_their_buffers() {
        for kind in [
            ReverbType::Room,
            ReverbType::Chamber,
            ReverbType::Hall,
            ReverbType::Plate,
        ] {
            let (small, small_mean) = line_seconds(kind, 0.0);
            let (large, large_mean) = line_seconds(kind, 1.0);
            assert!((large_mean / small_mean - SIZE_RANGE).abs() < 1e-3);
            let mean = small.iter().sum::<f32>() / LINES as f32;
            assert!((mean / small_mean - 1.0).abs() < 1e-4);
            assert!(small.windows(2).all(|pair| pair[0] < pair[1]), "{kind:?}");
            let longest = large.iter().fold(0.0_f32, |a, b| a.max(*b));
            assert!(
                longest + MOVEMENT_SECONDS < 0.98 * MAX_LINE_SECONDS,
                "{kind:?}: {longest}"
            );
            let lengths = line_lengths(kind, 0.0, 44_100.0);
            let mut unique = lengths.to_vec();
            unique.dedup();
            assert_eq!(unique.len(), LINES, "{kind:?}: {lengths:?}");
        }
    }

    #[test]
    fn early_reflections_come_from_the_room() {
        let mut taps = [[(0.0, 0.0); EARLY_TAPS]; 2];
        early_taps(ReverbType::Plate, 0.5, &mut taps);
        assert!(taps.iter().flatten().all(|tap| *tap == (0.0, 0.0)));
        early_taps(ReverbType::Hall, 0.5, &mut taps);
        let mut small = taps;
        early_taps(ReverbType::Room, 0.5, &mut small);
        for channel in 0..2 {
            assert!(taps[channel].windows(2).all(|pair| pair[0].0 <= pair[1].0));
            assert!(taps[channel].iter().all(|(seconds, gain)| {
                *seconds > 0.0 && *seconds <= MAX_EARLY_SECONDS && *gain > 0.0 && *gain < 0.36
            }));
            // A hall's walls are further away than a room's.
            assert!(taps[channel][0].0 > 2.0 * small[channel][0].0);
        }
        // The two ears hear the walls at slightly different times.
        assert!(taps[0] != taps[1]);
        // The first reflection in a 28 m hall is the floor: about 15 ms.
        assert!((0.005..0.03).contains(&taps[0][0].0), "{}", taps[0][0].0);
    }

    fn impulse_response(kernel: &mut ReverbKernel, samples: usize) -> Vec<(f32, f32)> {
        (0..samples)
            .map(|n| {
                let x = if n == 0 { 1.0 } else { 0.0 };
                kernel.process(x, x)
            })
            .collect()
    }

    #[test]
    fn kernel_waits_for_the_pre_delay_and_decays_to_exact_silence() {
        let rate = 48_000.0;
        let mut kernel = ReverbKernel::default();
        kernel.prepare(rate);
        let settings = ReverbSettings {
            kind: ReverbType::Plate,
            pre_delay_seconds: 0.05,
            decay_seconds: 0.3,
            ..ReverbSettings::default()
        };
        kernel.set(&settings);
        kernel.reset();
        let response = impulse_response(&mut kernel, 6 * 48_000);
        let first = response
            .iter()
            .position(|(l, r)| *l != 0.0 || *r != 0.0)
            .unwrap();
        // A plate has no early taps: the first sound has passed the
        // pre-delay, the diffusers' direct path and the shortest line.
        assert!(first >= 2400, "{first}");
        assert!(first < 2400 + 400, "{first}");
        let last = response
            .iter()
            .rposition(|(l, r)| *l != 0.0 || *r != 0.0)
            .unwrap();
        assert!(last < 5 * 48_000, "still sounding at {last}");
        assert!(response.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
        // Left and right are different signals of similar level.
        let energy = |pick: fn(&(f32, f32)) -> f32| {
            response
                .iter()
                .map(|frame| f64::from(pick(frame)).powi(2))
                .sum::<f64>()
        };
        let (left, right) = (energy(|frame| frame.0), energy(|frame| frame.1));
        let cross: f64 = response
            .iter()
            .map(|(l, r)| f64::from(*l) * f64::from(*r))
            .sum();
        assert!((0.5..2.0).contains(&(left / right)), "{left} {right}");
        assert!(
            cross.abs() < 0.3 * (left * right).sqrt(),
            "correlation {cross}"
        );
        // Reset replays it.
        kernel.reset();
        assert_eq!(impulse_response(&mut kernel, 20_000), response[..20_000]);
    }

    #[test]
    fn width_zero_is_mono_and_cuts_off_pass_everything() {
        let rate = 48_000.0;
        let mut kernel = ReverbKernel::default();
        kernel.prepare(rate);
        let open = ReverbSettings {
            low_cut_hz: CUT_OFF_LOW_HZ,
            high_cut_hz: CUT_OFF_HIGH_HZ,
            ..ReverbSettings::default()
        };
        kernel.set(&ReverbSettings { width: 0.0, ..open });
        kernel.reset();
        assert!(
            impulse_response(&mut kernel, 20_000)
                .iter()
                .all(|(l, r)| l == r)
        );
        kernel.set(&open);
        kernel.reset();
        let plain = impulse_response(&mut kernel, 20_000);
        kernel.set(&ReverbSettings {
            low_cut_hz: 300.0,
            ..open
        });
        kernel.reset();
        let cut = impulse_response(&mut kernel, 20_000);
        assert!(plain != cut);
    }
}
