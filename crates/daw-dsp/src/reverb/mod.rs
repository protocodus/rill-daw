//! Reverberation: three models for three sizes of space behind one decay
//! law, and the complete stereo reverb around them ([`ReverbKernel`]) with
//! pre-delay, early reflections, tone and width.
//!
//! - **Studio**: a feed-forward diffuser cascade ([`DiffuserStep`]) into a
//!   short feedback delay network ([`Fdn`]): dense within milliseconds.
//! - **Room**: a sixteen-line network whose feedback matrix scatters (after
//!   Schlecht and Habets), with image-source early reflections.
//! - **Hall**: a ring of absorbent allpasses and delays (after Dattorro,
//!   Dahl and Jot), whose density builds over time.
//!
//! Every delay element carries an absorption filter whose attenuation is
//! proportional to its length (Jot), so all three obey the same Decay and
//! Damping. A seed draws the times of each structure ([`layout`]).
mod diffuser;
mod fdn;
pub mod layout;
mod ring;
mod rooms;

pub use diffuser::DiffuserStep;
pub use fdn::Fdn;

use super::{CutoffGlide, DelayLine, FadeTap, LinearRamp, Random, Shelf1, Smoothed, Svf, flush};
use ring::Hall;
use rooms::{Move, Room, Studio};

/// Multiplies by the `N` × `N` Hadamard matrix (`N` a power of two) scaled
/// to be orthogonal and its own inverse: sums and differences, no
/// multiplies until the final scaling.
#[inline]
pub fn hadamard<const N: usize>(values: &mut [f32; N]) {
    let mut half = 1;
    while half < N {
        for start in (0..N).step_by(2 * half) {
            for index in start..start + half {
                let (a, b) = (values[index], values[index + half]);
                values[index] = a + b;
                values[index + half] = a - b;
            }
        }
        half *= 2;
    }
    let scale = 1.0 / (N as f32).sqrt();
    for value in values {
        *value *= scale;
    }
}

/// The layers of [`hadamard`] with each pair turned by an angle instead
/// (`cos` and `sin` of it): the identity at 0, the Hadamard matrix up to
/// signs at π/4, orthogonal throughout. With a `cos` and `sin` of norm
/// below 1 it loses, and never gains.
#[inline]
pub fn butterflies<const N: usize>(values: &mut [f32; N], cos: f32, sin: f32) {
    let mut half = 1;
    while half < N {
        for start in (0..N).step_by(2 * half) {
            for index in start..start + half {
                let (a, b) = (values[index], values[index + half]);
                values[index] = cos * a + sin * b;
                values[index + half] = cos * b - sin * a;
            }
        }
        half *= 2;
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

/// The signed permutation between the Hadamard matrix and the rotation of a
/// scattering network of sixteen lines: output `i` is input
/// `ROOM_SCATTER_PERMUTATION[i]` times `ROOM_SCATTER_SIGNS[i]`. Without it
/// the rotation at π/4 would undo the Hadamard matrix and leave sixteen
/// separate combs; with it no entry of the product exceeds 0.375.
pub(crate) const ROOM_SCATTER_PERMUTATION: [usize; 16] =
    [7, 4, 8, 1, 3, 10, 2, 5, 15, 12, 14, 13, 11, 9, 0, 6];
pub(crate) const ROOM_SCATTER_SIGNS: [f32; 16] = [
    1.0, 1.0, -1.0, 1.0, 1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, 1.0, -1.0, -1.0, -1.0,
];

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
pub enum ReverbModel {
    /// A small, tight space: dense at once.
    Studio,
    /// A medium room with audible walls.
    Room,
    /// A large space whose tail builds and moves.
    Hall,
}

impl ReverbModel {
    pub const ALL: [Self; 3] = [Self::Studio, Self::Room, Self::Hall];

    /// Where the absorption turns from the mid decay to the high one, at
    /// Color 0.
    fn crossover_hz(self) -> f32 {
        match self {
            Self::Studio => 6000.0,
            Self::Room => 5000.0,
            Self::Hall => 4000.0,
        }
    }

    /// Pivot of the Color tilt.
    fn pivot_hz(self) -> f32 {
        match self {
            Self::Studio => 1500.0,
            Self::Room => 1000.0,
            Self::Hall => 700.0,
        }
    }

    fn movement_hz(self) -> f32 {
        match self {
            Self::Studio => 1.1,
            Self::Room => 0.8,
            Self::Hall => 0.6,
        }
    }

    /// Level of the late output: set once so that the three are equally
    /// loud at the default settings.
    fn gain(self) -> f32 {
        match self {
            Self::Studio => STUDIO_GAIN,
            Self::Room => ROOM_GAIN,
            Self::Hall => HALL_GAIN,
        }
    }
}

const STUDIO_GAIN: f32 = 1.2;
const ROOM_GAIN: f32 = 1.41;
const HALL_GAIN: f32 = 0.585;

/// Plain-unit settings of a [`ReverbKernel`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbSettings {
    pub model: ReverbModel,
    /// 0–1.
    pub size: f32,
    /// Decay time at 1 kHz.
    pub decay_seconds: f32,
    pub pre_delay_seconds: f32,
    /// 0–1: how much faster the top decays.
    pub damping: f32,
    /// 0–1.
    pub diffusion: f32,
    /// 0–1.
    pub movement: f32,
    /// Which room of this size: every seed draws its own delay times.
    pub seed: u32,
    /// −1 (dark) – 1 (bright): a tilt, a high cut on the dark side, and
    /// where the damping sets in.
    pub color: f32,
    /// Off at [`CUT_OFF_LOW_HZ`] and below.
    pub low_cut_hz: f32,
    /// 0 (mono) – 1 (as the network gives it) – 1.5.
    pub width: f32,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            model: ReverbModel::Hall,
            size: 0.5,
            decay_seconds: 1.8,
            pre_delay_seconds: 0.015,
            damping: 0.4,
            diffusion: 0.8,
            movement: 0.25,
            seed: 1,
            color: 0.0,
            low_cut_hz: 100.0,
            width: 1.0,
        }
    }
}

/// A low cut at or below this, and a high cut at or above the other, is
/// switched off: the signal passes untouched.
pub const CUT_OFF_LOW_HZ: f32 = 20.0;
pub const CUT_OFF_HIGH_HZ: f32 = 20_000.0;
const CUT_FADE_SECONDS: f32 = 0.02;

/// `dry` at a share of 0 and `wet` at 1, exactly; their mix between.
#[inline]
pub(crate) fn blend(dry: f32, wet: f32, share: f32) -> f32 {
    if share == 0.0 {
        dry
    } else if share == 1.0 {
        wet
    } else {
        dry + share * (wet - dry)
    }
}

/// The next share of a cut filter in the signal, on its way to 0 while
/// the cut is `off` and to 1 while it is on, in `fade` samples.
#[inline]
pub(crate) fn cut_share(share: &mut LinearRamp, off: bool, fade: u32) -> f32 {
    let aim = if off { 0.0 } else { 1.0 };
    if share.target() != aim {
        share.set_target(aim, fade);
    }
    share.next()
}

pub const MAX_PRE_DELAY_SECONDS: f32 = 0.25;
pub const EARLY_TAPS: usize = 8;
/// Output taps per channel of the Hall's ring.
pub(crate) const HALL_TAPS: usize = 11;
const MAX_EARLY_SECONDS: f32 = 0.2;
/// Mean loop delay at which a model's level needs no correction.
const REFERENCE_MEAN_SECONDS: f32 = 0.024;
const SHORTEST_HIGH_DECAY_SECONDS: f32 = 0.05;
const SPEED_OF_SOUND: f32 = 343.0;
const WALL_REFLECTION: f32 = 0.8;
pub(crate) const TAP_FADE_SECONDS: f32 = 0.03;
const LEVEL_SECONDS: f32 = 0.1;
/// A model that is switched away from rings out, but not for ever.
const LONGEST_RELEASE_SECONDS: f32 = 10.0;
const TILT_DB: f32 = 4.5;
const TICK: u32 = 32;

/// Streams of a seed: what each draws.
const STREAM_LENGTHS: u32 = 0;
const STREAM_SCATTER: u32 = 1;
const STREAM_STEPS: u32 = 2;
const STREAM_EARLY: u32 = 6;
const STREAM_TAPS: u32 = 7;
const STREAM_PHASES: u32 = 8;
const STREAM_RATES: u32 = 9;

/// Delay ranges of the Studio's four diffuser steps at Size 50 %, seconds.
const STUDIO_STEP_RANGES: [(f32, f32); 4] = [
    (0.0003, 0.0015),
    (0.0006, 0.003),
    (0.0012, 0.006),
    (0.0024, 0.012),
];
/// Where in its element each of the Hall's output taps sits, as a share
/// of the element's length (Dattorro's), before the seed moves it.
const HALL_TAP_PLACES: [[f32; HALL_TAPS]; 2] = [
    [
        0.063, 0.705, 0.720, 0.631, 0.447, 0.104, 0.287, 0.243, 0.885, 0.370, 0.540,
    ],
    [
        0.079, 0.815, 0.682, 0.719, 0.501, 0.126, 0.038, 0.262, 0.930, 0.900, 0.300,
    ],
];

fn high_decay_seconds(decay: f32, damping: f32) -> f32 {
    (decay * 0.1_f32.powf(damping))
        .max(SHORTEST_HIGH_DECAY_SECONDS)
        .min(decay)
}

fn crossover_hz(model: ReverbModel, color: f32) -> f32 {
    model.crossover_hz() * 2.0_f32.powf(0.75 * color.clamp(-1.0, 1.0))
}

/// The first images of a source in a shoebox room of the sides `room`
/// (metres), heard at two points 20 cm apart: per channel
/// `(seconds after the direct sound, gain)`, earliest first.
fn shoebox_taps(room: [f32; 3], gain: f32, out: &mut [[(f32, f32); EARLY_TAPS]; 2]) {
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
                (path - direct) / SPEED_OF_SOUND,
                gain * direct / path * WALL_REFLECTION.powi(order),
            );
            count += 1;
        }
        images.sort_by(|a, b| a.0.total_cmp(&b.0));
        taps.copy_from_slice(&images[..EARLY_TAPS]);
    }
}

/// Every time of a model's structure for a size and a seed, in samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layout {
    /// The recursive elements: a network's lines or the ring's eight
    /// (branch by branch).
    pub elements: [u32; 16],
    pub count: usize,
    /// Samples every lap spends outside the elements (the Room's mean
    /// scattering delay).
    pub extra: f32,
    /// Mean loop delay.
    pub mean_seconds: f32,
    /// Studio: the diffuser steps' delays.
    pub steps: [[u32; 8]; 4],
    /// Room: the scattering delays.
    pub scatter: [u32; 16],
    /// Hall: where each output tap sits in its element.
    pub taps: [[u32; HALL_TAPS]; 2],
    /// Modulation phases in cycles and rates in hertz.
    pub phases: [f32; 8],
    pub rates: [f32; 8],
    /// Early reflections as `(seconds after the pre-delay, gain)`.
    pub early: [[(f32, f32); EARLY_TAPS]; 2],
}

impl Layout {
    pub fn new(settings: &ReverbSettings, sample_rate: f32) -> Self {
        let model = settings.model;
        let size = settings.size.clamp(0.0, 1.0);
        let stream = |b: u32| Random::stream(settings.seed, model as u32, b);
        let samples = |seconds: f32| (seconds * sample_rate).round() as u32;
        let units = |b: u32| -> [f32; 8] {
            let mut random = stream(b);
            std::array::from_fn(|_| random.unit())
        };
        let mut layout = Self {
            elements: [0; 16],
            count: 16,
            extra: 0.0,
            mean_seconds: 0.0,
            steps: [[0; 8]; 4],
            scatter: [0; 16],
            taps: [[0; HALL_TAPS]; 2],
            phases: units(STREAM_PHASES),
            rates: units(STREAM_RATES).map(|unit| model.movement_hz() * (0.8 + 0.4 * unit)),
            early: [[(0.0, 0.0); EARLY_TAPS]; 2],
        };
        let network = |mean: f32, longest: f32| {
            let seconds = layout::line_seconds::<16>(&mut stream(STREAM_LENGTHS), mean);
            layout::prime_lengths(&seconds, sample_rate, (0.98 * longest * sample_rate) as u32)
        };
        match model {
            ReverbModel::Studio => {
                layout.mean_seconds = 0.004 * 4.5_f32.powf(size);
                layout.elements = network(layout.mean_seconds, rooms::STUDIO_LINE_SECONDS);
                let scale = 4.5_f32.powf(size - 0.5);
                for (step, (low, high)) in STUDIO_STEP_RANGES.into_iter().enumerate() {
                    let mut random = stream(STREAM_STEPS + step as u32);
                    layout.steps[step] =
                        layout::stratified::<8>(&mut random, low * scale, high * scale)
                            .map(samples);
                }
                // What is heard first: the first step's delays, four per
                // side.
                for channel in 0..2 {
                    for tap in 0..4 {
                        let delay = layout.steps[0][2 * tap + channel];
                        layout.early[channel][tap] =
                            (delay as f32 / sample_rate, 0.5 * rooms::STUDIO_EARLY_GAIN);
                    }
                }
            }
            ReverbModel::Room => {
                layout.mean_seconds = 0.007 * (40.0_f32 / 7.0).powf(size);
                layout.elements = network(layout.mean_seconds, rooms::ROOM_LINE_SECONDS);
                let longest = (0.25 * layout.mean_seconds).min(0.003);
                layout.scatter =
                    layout::stratified::<16>(&mut stream(STREAM_SCATTER), 0.0, longest)
                        .map(samples);
                layout.extra = layout.scatter.iter().sum::<u32>() as f32 / 16.0;
                let scale = (40.0_f32 / 7.0).powf(size - 0.5);
                shoebox_taps(
                    [7.0, 5.0, 3.0].map(|side| side * scale),
                    0.5,
                    &mut layout.early,
                );
            }
            ReverbModel::Hall => {
                let scale = 0.5 * 4.0_f32.powf(size);
                let mut random = stream(STREAM_LENGTHS);
                let nominal: [f32; 8] = std::array::from_fn(|index| {
                    scale * ring::ELEMENT_SECONDS[index / 4][index % 4]
                });
                let drawn =
                    nominal.map(|seconds| seconds * (1.0 + 0.12 * (2.0 * random.unit() - 1.0)));
                let total: f32 = nominal.iter().sum();
                let fit = total / drawn.iter().sum::<f32>();
                let lengths = layout::prime_lengths(
                    &drawn.map(|seconds| seconds * fit),
                    sample_rate,
                    u32::MAX,
                );
                layout.elements[..8].copy_from_slice(&lengths);
                layout.count = 8;
                layout.mean_seconds = total / 8.0;
                let mut random = stream(STREAM_TAPS);
                for (channel, places) in HALL_TAP_PLACES.iter().enumerate() {
                    for (tap, (branch, element, _)) in ring::TAPS[channel].iter().enumerate() {
                        let place =
                            (places[tap] + 0.08 * (2.0 * random.unit() - 1.0)).clamp(0.02, 0.98);
                        let length = lengths[4 * branch + element];
                        layout.taps[channel][tap] =
                            ((place * length as f32).round() as u32).clamp(1, length - 1);
                    }
                }
                shoebox_taps(
                    [30.0, 20.0, 12.0].map(|side| side * scale),
                    0.355,
                    &mut layout.early,
                );
            }
        }
        if model != ReverbModel::Studio {
            // No two rooms have their walls in the same places.
            let mut random = stream(STREAM_EARLY);
            for taps in &mut layout.early {
                for (seconds, _) in taps.iter_mut() {
                    *seconds = (*seconds * (1.0 + 0.1 * (2.0 * random.unit() - 1.0)))
                        .min(MAX_EARLY_SECONDS);
                }
                taps.sort_by(|a, b| a.0.total_cmp(&b.0));
                // Walls that absorb a tail soon reflect less: the
                // reflections lie on the slope of the decay.
                for (seconds, gain) in taps.iter_mut() {
                    *gain *= 10.0_f32.powf(-3.0 * *seconds / settings.decay_seconds.max(0.01));
                }
            }
        }
        layout
    }

    /// Each recursive element's length with its share of the lap outside
    /// the elements.
    fn laps(&self) -> impl Iterator<Item = u32> + '_ {
        self.elements[..self.count]
            .iter()
            .map(|length| (*length as f32 + self.extra).round() as u32)
    }
}

/// The decay time a model really has at a frequency: what the absorption
/// filters of all its delay elements add up to. `decay_seconds` at 1 kHz,
/// shorter above with Damping, somewhat longer below.
pub fn decay_seconds_at(settings: &ReverbSettings, hz: f32, sample_rate: f32) -> f32 {
    let omega = std::f64::consts::TAU * f64::from(hz / sample_rate).min(0.5);
    let high = high_decay_seconds(settings.decay_seconds, settings.damping);
    let crossover = crossover_hz(settings.model, settings.color);
    // A signal spends time in each element in proportion to its length, so
    // the losses of all are spread over the sum of the lengths.
    let (mut loss_db, mut samples) = (0.0, 0.0);
    for length in Layout::new(settings, sample_rate).laps() {
        let shelf = absorption_shelf(length, settings.decay_seconds, high, crossover, sample_rate);
        loss_db -= 20.0 * shelf.response(omega).norm().log10();
        samples += f64::from(length);
    }
    (60.0 * samples / (loss_db.max(1e-9) * f64::from(sample_rate))) as f32
}

/// What is heard first, per channel, as
/// `(seconds after the pre-delay, gain)`, earliest first: the reflections
/// of a Room's or a Hall's walls, which move with the size and the seed;
/// for the Studio the first four delays of its diffuser, the rest empty.
pub fn early_taps(settings: &ReverbSettings, out: &mut [[(f32, f32); EARLY_TAPS]; 2]) {
    // The times do not depend on the rate but for their rounding.
    *out = Layout::new(settings, 48_000.0).early;
}

/// Seconds a model's structure rings before the decay takes over: its
/// early reflections and one lap.
pub fn structure_seconds(settings: &ReverbSettings) -> f32 {
    match settings.model {
        ReverbModel::Studio => 0.06,
        ReverbModel::Room => 0.15,
        ReverbModel::Hall => {
            let total: f32 = ring::ELEMENT_SECONDS.iter().flatten().sum();
            total * 0.5 * 4.0_f32.powf(settings.size.clamp(0.0, 1.0))
        }
    }
}

/// Seconds until the reverb has fallen by 60 dB at any frequency: the bass
/// may ring for up to twice Decay.
pub fn tail_seconds(settings: &ReverbSettings) -> f32 {
    settings.pre_delay_seconds + structure_seconds(settings) + 2.0 * settings.decay_seconds
}

/// A model's place in the kernel: it runs while it is fed or rings out.
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// 1 while the model is the selected one.
    feed: LinearRamp,
    /// Output fade at the end of the ring-out.
    fade: LinearRamp,
    /// Samples of ring-out left once the feed has closed.
    release: u32,
    running: bool,
}

/// A complete stereo reverb, wet signal only: pre-delay, the three models
/// (the selected one is fed; one switched away from rings out beside it),
/// early reflections, low cut, Color and width.
#[derive(Clone, Debug)]
pub struct ReverbKernel {
    sample_rate: f32,
    settings: ReverbSettings,
    pre: [DelayLine; 2],
    pre_tap: [FadeTap; 2],
    early: [[(FadeTap, LinearRamp); EARLY_TAPS]; 2],
    studio: Studio,
    room: Room,
    hall: Hall,
    slots: [Slot; 3],
    level: [LinearRamp; 3],
    low_cut: [Svf; 2],
    high_cut: [Svf; 2],
    tilt: [Shelf1; 2],
    low_glide: CutoffGlide,
    high_glide: CutoffGlide,
    /// Share of each cut in the signal: 0 while it is off.
    low_share: LinearRamp,
    high_share: LinearRamp,
    /// One over the decay time: the loss in decibels per second moves
    /// evenly, where the time itself would open a short decay in a jump.
    decay_rate: Smoothed,
    damping: Smoothed,
    diffusion: Smoothed,
    movement: Smoothed,
    color: Smoothed,
    width: Smoothed,
    /// Decay, damping and color the absorption filters were last made for.
    absorbed: (f32, f32, f32),
    absorb_dirty: bool,
    /// Color and model the tilt was last made for.
    tilted: (f32, ReverbModel),
    tick: u32,
}

impl Default for ReverbKernel {
    fn default() -> Self {
        let settings = ReverbSettings::default();
        let idle = Slot {
            feed: LinearRamp::new(0.0),
            fade: LinearRamp::new(1.0),
            release: 0,
            running: false,
        };
        Self {
            sample_rate: 48_000.0,
            settings,
            pre: std::array::from_fn(|_| DelayLine::default()),
            pre_tap: [FadeTap::default(); 2],
            early: [[(FadeTap::default(), LinearRamp::new(0.0)); EARLY_TAPS]; 2],
            studio: Studio::new(),
            room: Room::new(),
            hall: Hall::new(),
            slots: [idle; 3],
            level: [LinearRamp::new(1.0); 3],
            low_cut: [Svf::default(), Svf::default()],
            high_cut: [Svf::default(), Svf::default()],
            tilt: [Shelf1::default(), Shelf1::default()],
            low_glide: CutoffGlide::new(settings.low_cut_hz),
            high_glide: CutoffGlide::new(CUT_OFF_HIGH_HZ),
            low_share: LinearRamp::new(1.0),
            high_share: LinearRamp::new(0.0),
            decay_rate: Smoothed::new(1.0 / settings.decay_seconds),
            damping: Smoothed::new(settings.damping),
            diffusion: Smoothed::new(settings.diffusion),
            movement: Smoothed::new(settings.movement),
            color: Smoothed::new(settings.color),
            width: Smoothed::new(settings.width),
            absorbed: (0.0, 0.0, 0.0),
            absorb_dirty: true,
            tilted: (f32::NAN, settings.model),
            tick: 0,
        }
    }
}

/// The corner of Color's high cut: none from the centre up.
fn color_cut_hz(color: f32, sample_rate: f32) -> f32 {
    if color >= 0.0 {
        CUT_OFF_HIGH_HZ
    } else {
        (CUT_OFF_HIGH_HZ * 10.0_f32.powf(color.max(-1.0))).min(0.45 * sample_rate)
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
        let fade = samples(TAP_FADE_SECONDS) as u32;
        for tap in self
            .pre_tap
            .iter_mut()
            .chain(self.early.iter_mut().flatten().map(|(tap, _)| tap))
        {
            tap.set_fade(fade);
        }
        self.studio.prepare(sample_rate);
        self.room.prepare(sample_rate);
        self.hall.prepare(sample_rate);
        self.decay_rate.set_time(50.0, sample_rate);
        self.damping.set_time(50.0, sample_rate);
        for value in [
            &mut self.diffusion,
            &mut self.movement,
            &mut self.color,
            &mut self.width,
        ] {
            value.set_time(20.0, sample_rate);
        }
        self.low_glide.prepare(20.0, sample_rate);
        self.high_glide.prepare(20.0, sample_rate);
        self.reset();
    }

    fn configure(&mut self, layout: &Layout, how: Move) {
        match self.settings.model {
            ReverbModel::Studio => self.studio.configure(layout, how),
            ReverbModel::Room => self.room.configure(layout, how),
            ReverbModel::Hall => self.hall.configure(layout, how),
        }
    }

    /// New targets; everything glides, fades or ramps there. Another model
    /// takes over the input while the one before rings out.
    pub fn set(&mut self, settings: &ReverbSettings) {
        let previous = self.settings;
        self.settings = *settings;
        let rate = self.sample_rate;
        let fade = (TAP_FADE_SECONDS * rate) as u32;
        let layout = Layout::new(settings, rate);
        let selected = settings.model as usize;
        let level = settings.model.gain() * (layout.mean_seconds / REFERENCE_MEAN_SECONDS).sqrt();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let aim = if index == selected { 1.0 } else { 0.0 };
            if slot.feed.target() != aim {
                slot.feed.set_target(aim, fade);
            }
        }
        let slot = &mut self.slots[selected];
        slot.release = 0;
        if slot.fade.target() != 1.0 {
            slot.fade.set_target(1.0, fade);
        }
        if !slot.running {
            // It is silent and cleared: nothing has to glide.
            slot.running = true;
            slot.fade = LinearRamp::new(1.0);
            self.level[selected] = LinearRamp::new(level);
            self.configure(&layout, Move::Snap);
            self.reset_model(settings.model);
        } else {
            let another_room = previous.model == settings.model
                && previous.seed != settings.seed
                && previous.size == settings.size;
            self.configure(
                &layout,
                if another_room {
                    Move::Fade
                } else {
                    Move::Glide
                },
            );
            if self.level[selected].target() != level {
                self.level[selected].set_target(level, (LEVEL_SECONDS * rate) as u32);
            }
        }
        let pre_delay = settings.pre_delay_seconds.clamp(0.0, MAX_PRE_DELAY_SECONDS);
        for (channel, taps) in layout.early.iter().enumerate() {
            self.pre_tap[channel].set((pre_delay * rate).round() as usize);
            for ((tap, gain), (seconds, target)) in self.early[channel].iter_mut().zip(*taps) {
                // The Studio's first sound is its diffuser's, not a tap's.
                let target = if settings.model == ReverbModel::Studio {
                    0.0
                } else {
                    target
                };
                tap.set(((pre_delay + seconds) * rate).round() as usize);
                if gain.target() != target {
                    gain.set_target(target, fade);
                }
            }
        }
        self.decay_rate
            .set_target(1.0 / settings.decay_seconds.max(0.01));
        self.damping.set_target(settings.damping.clamp(0.0, 1.0));
        self.diffusion
            .set_target(settings.diffusion.clamp(0.0, 1.0));
        self.movement.set_target(settings.movement.clamp(0.0, 1.0));
        self.color.set_target(settings.color.clamp(-1.0, 1.0));
        self.width.set_target(settings.width.clamp(0.0, 1.5));
        self.low_glide.set(settings.low_cut_hz);
        self.high_glide.set(color_cut_hz(settings.color, rate));
        self.absorb_dirty = true;
    }

    fn reset_model(&mut self, model: ReverbModel) {
        let (diffusion, movement) = (self.diffusion.value(), self.movement.value());
        match model {
            ReverbModel::Studio => self.studio.reset(diffusion, movement),
            ReverbModel::Room => self.room.reset(diffusion, movement),
            ReverbModel::Hall => self.hall.reset(movement, self.sample_rate),
        }
    }

    /// Clears the audio state and settles on the settings.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.running = false;
            slot.feed = LinearRamp::new(0.0);
        }
        let settings = self.settings;
        self.decay_rate
            .set_target(1.0 / settings.decay_seconds.max(0.01));
        self.diffusion
            .set_target(settings.diffusion.clamp(0.0, 1.0));
        self.movement.set_target(settings.movement.clamp(0.0, 1.0));
        for value in [
            &mut self.decay_rate,
            &mut self.diffusion,
            &mut self.movement,
        ] {
            value.snap();
        }
        self.set(&settings);
        self.slots[settings.model as usize].feed = LinearRamp::new(1.0);
        for model in ReverbModel::ALL {
            self.reset_model(model);
        }
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
            self.low_cut[channel].reset();
            self.high_cut[channel].reset();
            self.tilt[channel].reset();
        }
        for value in [&mut self.damping, &mut self.color, &mut self.width] {
            value.snap();
        }
        self.level.iter_mut().for_each(LinearRamp::snap);
        self.low_glide.snap();
        self.high_glide.snap();
        let (low_off, high_off) = self.cuts_off();
        self.low_share = LinearRamp::new(if low_off { 0.0 } else { 1.0 });
        self.high_share = LinearRamp::new(if high_off { 0.0 } else { 1.0 });
        self.tick = 0;
        self.update_absorption(0);
        self.update_tilt();
    }

    /// Whether the low cut and Color's high cut are off: set so, and the
    /// corner has arrived where it does least.
    fn cuts_off(&self) -> (bool, bool) {
        (
            self.settings.low_cut_hz <= CUT_OFF_LOW_HZ && self.low_glide.is_settled(),
            self.settings.color >= 0.0 && self.high_glide.is_settled(),
        )
    }

    /// The absorption filters for the controls as they are now, reached in
    /// `ramp` samples.
    fn update_absorption(&mut self, ramp: u32) {
        let (rate, damping, color) = (
            self.decay_rate.value(),
            self.damping.value(),
            self.color.value(),
        );
        let decay = 1.0 / rate;
        let high = high_decay_seconds(decay, damping);
        let rate = self.sample_rate;
        let mut gliding = false;
        for model in ReverbModel::ALL {
            if !self.slots[model as usize].running {
                continue;
            }
            let crossover = crossover_hz(model, color);
            gliding |= match model {
                ReverbModel::Studio => self.studio.absorb(decay, high, crossover, rate, ramp),
                ReverbModel::Room => self.room.absorb(decay, high, crossover, rate, ramp),
                ReverbModel::Hall => self.hall.absorb(decay, high, crossover, rate, ramp),
            };
        }
        self.absorbed = (self.decay_rate.value(), damping, color);
        // One more pass after a glide has ended, on the final lengths.
        self.absorb_dirty = gliding;
    }

    fn update_tilt(&mut self) {
        let (color, model) = (self.color.value(), self.settings.model);
        let tilt = Shelf1::tilt(TILT_DB * color, model.pivot_hz(), self.sample_rate);
        for filter in &mut self.tilt {
            filter.set(&tilt);
        }
        self.tilted = (color, model);
    }

    /// The lengths of the selected model's recursive elements now, in
    /// samples: on the way to their targets while they glide. Returns how
    /// many of `out` are filled.
    pub fn lengths(&self, out: &mut [f64; 16]) -> usize {
        match self.settings.model {
            ReverbModel::Studio => *out = *self.studio.lengths(),
            ReverbModel::Room => *out = *self.room.lengths(),
            ReverbModel::Hall => {
                out[..8].copy_from_slice(&self.hall.lengths());
                return 8;
            }
        }
        16
    }

    /// One sample in, the wet signal out.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let (decay_rate, damping) = (self.decay_rate.next_value(), self.damping.next_value());
        let (diffusion, movement) = (self.diffusion.next_value(), self.movement.next_value());
        let color = self.color.next_value();
        if self.tick == 0 {
            if self.absorb_dirty || self.absorbed != (decay_rate, damping, color) {
                self.update_absorption(TICK);
            }
            if self.tilted != (color, self.settings.model) {
                self.update_tilt();
            }
            let rate = self.sample_rate;
            if self.slots[0].running {
                self.studio.tick(diffusion, movement, rate);
            }
            if self.slots[1].running {
                self.room.tick(diffusion, movement, rate);
            }
            if self.slots[2].running {
                self.hall.tick(movement, rate);
            }
        }
        self.tick = (self.tick + 1) % TICK;

        let input = [left, right];
        let mut delayed = [0.0; 2];
        let mut wet = [0.0; 2];
        for channel in 0..2 {
            let line = &mut self.pre[channel];
            line.write(flush(input[channel]));
            delayed[channel] = self.pre_tap[channel].read(line);
            for (tap, gain) in &mut self.early[channel] {
                wet[channel] += gain.next() * tap.read(line);
            }
        }
        for model in ReverbModel::ALL {
            let index = model as usize;
            if !self.slots[index].running {
                continue;
            }
            let feed = self.slots[index].feed.next();
            let (left, right) = (feed * delayed[0], feed * delayed[1]);
            let (late, early) = match model {
                ReverbModel::Studio => self.studio.process(left, right),
                ReverbModel::Room => (self.room.process(left, right, diffusion), [0.0; 2]),
                ReverbModel::Hall => (self.hall.process(left, right, diffusion), [0.0; 2]),
            };
            let slot = &mut self.slots[index];
            let gain = slot.fade.next() * self.level[index].next();
            for channel in 0..2 {
                wet[channel] += gain * late[channel] + slot.fade.value() * early[channel];
            }
            if feed == 0.0 && slot.feed.target() == 0.0 {
                if slot.release > 0 {
                    slot.release -= 1;
                    if slot.release == 0 {
                        slot.fade
                            .set_target(0.0, (TAP_FADE_SECONDS * self.sample_rate) as u32);
                    }
                } else if slot.fade.target() != 0.0 {
                    // The feed has just closed: the tail rings out.
                    let seconds = (2.0 / decay_rate + structure_seconds(&self.settings))
                        .min(LONGEST_RELEASE_SECONDS);
                    slot.release = (seconds * self.sample_rate) as u32 + 1;
                } else if slot.fade.value() == 0.0 {
                    slot.running = false;
                    self.reset_model(model);
                }
            }
        }

        let (low_g, high_g) = (self.low_glide.next_g(), self.high_glide.next_g());
        // A cut that goes off or comes on still turns the phase of what
        // is heard: it leaves and enters by a crossfade.
        let (low_off, high_off) = self.cuts_off();
        let fade = (CUT_FADE_SECONDS * self.sample_rate) as u32;
        let low_share = cut_share(&mut self.low_share, low_off, fade);
        let high_share = cut_share(&mut self.high_share, high_off, fade);
        let flat = color == 0.0;
        for (channel, wet) in wet.iter_mut().enumerate() {
            let mut signal = *wet;
            // The filters run while they are off, so they come in on a
            // signal they already know.
            let cut = self.low_cut[channel]
                .process(signal, low_g, std::f32::consts::SQRT_2)
                .high;
            signal = blend(signal, cut, low_share);
            let tilted = self.tilt[channel].process(signal);
            if !flat {
                signal = tilted;
            }
            let cut = self.high_cut[channel]
                .process(signal, high_g, std::f32::consts::SQRT_2)
                .low;
            *wet = blend(signal, cut, high_share);
        }
        let width = self.width.next_value();
        let (mid, side) = (0.5 * (wet[0] + wet[1]), 0.5 * (wet[0] - wet[1]) * width);
        (mid + side, mid - side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hadamard_is_orthogonal_and_its_own_inverse() {
        let mut random = Random::new(3);
        let original: [f32; 16] = std::array::from_fn(|_| random.bipolar());
        let mut values = original;
        hadamard(&mut values);
        let energy = |values: &[f32; 16]| values.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        assert!((energy(&values) / energy(&original) - 1.0).abs() < 1e-6);
        assert!(
            values
                .iter()
                .zip(&original)
                .any(|(a, b)| (a - b).abs() > 0.1)
        );
        // A unit vector spreads evenly: every entry is ±1/4.
        let mut unit = [0.0; 16];
        unit[5] = 1.0;
        hadamard(&mut unit);
        for (column, value) in unit.iter().enumerate() {
            assert_eq!(*value, 0.25 * hadamard_sign(5, column));
        }
        hadamard(&mut values);
        for (a, b) in values.iter().zip(&original) {
            assert!((a - b).abs() < 1e-6);
        }
        let mut eight = [1.0_f32; 8];
        hadamard(&mut eight);
        assert!((eight[0] - 8.0_f32.sqrt()).abs() < 1e-6 && eight[1..] == [0.0; 7]);
    }

    #[test]
    fn butterflies_turn_from_the_identity_to_a_full_mix() {
        let mut random = Random::new(5);
        let original: [f32; 16] = std::array::from_fn(|_| random.bipolar());
        let mut values = original;
        butterflies(&mut values, 1.0, 0.0);
        assert_eq!(values, original);
        let energy = |values: &[f32; 16]| values.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        for angle in [0.2_f32, 0.5, std::f32::consts::FRAC_PI_4] {
            let (sin, cos) = angle.sin_cos();
            let mut values = original;
            butterflies(&mut values, cos, sin);
            assert!((energy(&values) / energy(&original) - 1.0).abs() < 1e-5);
        }
        // At π/4 a unit vector reaches every channel at the same level.
        let (sin, cos) = std::f32::consts::FRAC_PI_4.sin_cos();
        let mut unit = [0.0; 16];
        unit[9] = 1.0;
        butterflies(&mut unit, cos, sin);
        assert!(unit.iter().all(|value| (value.abs() - 0.25).abs() < 1e-6));
    }

    #[test]
    fn the_scattering_matrix_keeps_the_lines_coupled() {
        // Column by column: B(π/4)·Q·H of each unit vector.
        let product = |shuffle: bool| {
            let (sin, cos) = std::f32::consts::FRAC_PI_4.sin_cos();
            let mut largest = 0.0_f32;
            for column in 0..16 {
                let mut values = [0.0; 16];
                values[column] = 1.0;
                hadamard(&mut values);
                if shuffle {
                    values = std::array::from_fn(|index| {
                        ROOM_SCATTER_SIGNS[index] * values[ROOM_SCATTER_PERMUTATION[index]]
                    });
                }
                butterflies(&mut values, cos, sin);
                largest = values.iter().fold(largest, |a, b| a.max(b.abs()));
            }
            largest
        };
        // Without the shuffle the rotation undoes the Hadamard matrix.
        assert!((product(false) - 1.0).abs() < 1e-5);
        assert!(product(true) <= 0.5, "{}", product(true));
        let mut seen = [false; 16];
        for source in ROOM_SCATTER_PERMUTATION {
            seen[source] = true;
        }
        assert_eq!(seen, [true; 16]);
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

    fn settings(model: ReverbModel) -> ReverbSettings {
        ReverbSettings {
            model,
            ..ReverbSettings::default()
        }
    }

    #[test]
    fn decay_follows_the_knobs_at_each_frequency() {
        let rate = 48_000.0;
        for model in ReverbModel::ALL {
            let settings = settings(model);
            let mid = decay_seconds_at(&settings, 1000.0, rate);
            assert!((mid / 1.8 - 1.0).abs() < 0.01, "{model:?}: {mid}");
            let low = decay_seconds_at(&settings, 60.0, rate);
            let high = decay_seconds_at(&settings, 16_000.0, rate);
            assert!(
                low > mid && low < 3.6 && high < 0.6 * mid,
                "{model:?}: {low} {mid} {high}"
            );
            // Without damping the decay is the same everywhere, whatever
            // moves.
            let flat = ReverbSettings {
                damping: 0.0,
                movement: 1.0,
                ..settings
            };
            for hz in [60.0, 1000.0, 16_000.0] {
                let seconds = decay_seconds_at(&flat, hz, rate);
                assert!(
                    (seconds / 1.8 - 1.0).abs() < 0.01,
                    "{model:?} {hz}: {seconds}"
                );
            }
            // A bright Color keeps the mids ringing higher up.
            let at =
                |color: f32| decay_seconds_at(&ReverbSettings { color, ..settings }, 4000.0, rate);
            assert!(at(-1.0) < at(0.0) && at(0.0) < at(1.0), "{model:?}");
        }
    }

    #[test]
    fn layouts_scale_with_size_keep_their_mean_and_fit_their_buffers() {
        for rate in [44_100.0_f32, 192_000.0] {
            for model in ReverbModel::ALL {
                let mut seen = Vec::new();
                for seed in 1..=32 {
                    for size in [0.0, 0.5, 1.0] {
                        let settings = ReverbSettings {
                            size,
                            seed,
                            ..settings(model)
                        };
                        let layout = Layout::new(&settings, rate);
                        let elements = &layout.elements[..layout.count];
                        let label = format!("{model:?} seed {seed} size {size} at {rate}");
                        let mut unique = elements.to_vec();
                        unique.sort_unstable();
                        unique.dedup();
                        assert_eq!(unique.len(), layout.count, "{label}");
                        assert!(elements.iter().all(|n| nearest_prime(*n) == *n), "{label}");
                        let mean = elements.iter().sum::<u32>() as f32 / layout.count as f32 / rate;
                        assert!(
                            (mean / layout.mean_seconds - 1.0).abs() < 0.01,
                            "{label}: {mean} against {}",
                            layout.mean_seconds
                        );
                        let longest = *elements.iter().max().unwrap() as f32 / rate;
                        let room = match model {
                            ReverbModel::Studio => rooms::STUDIO_LINE_SECONDS,
                            ReverbModel::Room => rooms::ROOM_LINE_SECONDS,
                            ReverbModel::Hall => f32::MAX,
                        };
                        assert!(longest < 0.98 * room, "{label}: {longest}");
                        if model == ReverbModel::Hall {
                            for (index, length) in elements.iter().enumerate() {
                                let nominal = ring::ELEMENT_SECONDS[index / 4][index % 4];
                                assert!(
                                    (*length as f32 / rate) < 0.98 * ring::ELEMENT_ROOM * nominal,
                                    "{label}"
                                );
                            }
                            for channel in 0..2 {
                                for (tap, (branch, element, _)) in
                                    ring::TAPS[channel].iter().enumerate()
                                {
                                    assert!(
                                        layout.taps[channel][tap] < elements[4 * branch + element]
                                    );
                                }
                            }
                        }
                        assert!(
                            layout
                                .steps
                                .iter()
                                .flatten()
                                .all(|delay| (*delay as f32 / rate) < rooms::STUDIO_STEP_SECONDS)
                        );
                        assert!(
                            layout
                                .scatter
                                .iter()
                                .all(|delay| (*delay as f32 / rate) < rooms::ROOM_SCATTER_SECONDS)
                        );
                        if size == 0.5 {
                            seen.push(layout);
                        }
                    }
                }
                // Every seed is another structure.
                for (index, layout) in seen.iter().enumerate() {
                    assert!(
                        seen[..index]
                            .iter()
                            .all(|other| other.elements != layout.elements)
                    );
                }
            }
        }
        let small = Layout::new(
            &ReverbSettings {
                size: 0.0,
                ..settings(ReverbModel::Room)
            },
            48_000.0,
        );
        let large = Layout::new(
            &ReverbSettings {
                size: 1.0,
                ..settings(ReverbModel::Room)
            },
            48_000.0,
        );
        assert!((large.mean_seconds / small.mean_seconds - 40.0 / 7.0).abs() < 1e-3);
    }

    #[test]
    fn early_reflections_come_from_the_room_and_move_with_the_seed() {
        let mut taps = [[(0.0, 0.0); EARLY_TAPS]; 2];
        early_taps(&settings(ReverbModel::Hall), &mut taps);
        let mut small = taps;
        early_taps(&settings(ReverbModel::Room), &mut small);
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
        // The first reflection in a 30 m hall is the floor: about 15 ms.
        assert!((0.005..0.03).contains(&taps[0][0].0), "{}", taps[0][0].0);
        let mut other = taps;
        early_taps(
            &ReverbSettings {
                seed: 2,
                ..settings(ReverbModel::Hall)
            },
            &mut other,
        );
        assert!(other != taps);
        for (a, b) in taps[0].iter().zip(&other[0]) {
            assert!((a.0 / b.0 - 1.0).abs() < 0.35, "{a:?} {b:?}");
        }
        // The Studio shows its diffuser's first delays.
        early_taps(&settings(ReverbModel::Studio), &mut taps);
        assert!(
            taps[0][..4]
                .iter()
                .all(|(seconds, gain)| *seconds < 0.002 && *gain == 0.3)
        );
        assert!(taps[0][4..].iter().all(|tap| *tap == (0.0, 0.0)));
    }

    fn kernel(settings: &ReverbSettings, rate: f32) -> ReverbKernel {
        let mut kernel = ReverbKernel::default();
        kernel.prepare(rate);
        kernel.set(settings);
        kernel.reset();
        kernel
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
    fn every_model_waits_for_the_pre_delay_and_decays_to_exact_silence() {
        let rate = 48_000.0;
        for model in ReverbModel::ALL {
            let settings = ReverbSettings {
                pre_delay_seconds: 0.05,
                decay_seconds: 0.3,
                ..settings(model)
            };
            let mut kernel = kernel(&settings, rate);
            let response = impulse_response(&mut kernel, 6 * 48_000);
            let first = response
                .iter()
                .position(|(l, r)| *l != 0.0 || *r != 0.0)
                .unwrap();
            assert!((2400..2400 + 1000).contains(&first), "{model:?}: {first}");
            let last = response
                .iter()
                .rposition(|(l, r)| *l != 0.0 || *r != 0.0)
                .unwrap();
            assert!(last < 5 * 48_000, "{model:?} still sounding at {last}");
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
            assert!(
                (0.5..2.0).contains(&(left / right)),
                "{model:?}: {left} {right}"
            );
            assert!(
                cross.abs() < 0.4 * (left * right).sqrt(),
                "{model:?}: correlation {}",
                cross / (left * right).sqrt()
            );
            // Reset replays it.
            kernel.reset();
            assert_eq!(impulse_response(&mut kernel, 20_000), response[..20_000]);
        }
    }

    #[test]
    fn width_zero_is_mono_and_color_and_cut_change_the_sound() {
        let rate = 48_000.0;
        let open = ReverbSettings {
            low_cut_hz: CUT_OFF_LOW_HZ,
            ..ReverbSettings::default()
        };
        let mut narrow = kernel(&ReverbSettings { width: 0.0, ..open }, rate);
        assert!(
            impulse_response(&mut narrow, 20_000)
                .iter()
                .all(|(l, r)| l == r)
        );
        let plain = impulse_response(&mut kernel(&open, rate), 20_000);
        let energy = |response: &[(f32, f32)]| -> f64 {
            response.iter().map(|(l, _)| f64::from(*l).powi(2)).sum()
        };
        let cut = impulse_response(
            &mut kernel(
                &ReverbSettings {
                    low_cut_hz: 300.0,
                    ..open
                },
                rate,
            ),
            20_000,
        );
        assert!(plain != cut);
        // An impulse is mostly top: dark loses it, bright adds to it.
        let dark = impulse_response(
            &mut kernel(
                &ReverbSettings {
                    color: -1.0,
                    ..open
                },
                rate,
            ),
            20_000,
        );
        let bright = impulse_response(
            &mut kernel(&ReverbSettings { color: 1.0, ..open }, rate),
            20_000,
        );
        assert!(energy(&dark) < 0.5 * energy(&plain));
        assert!(energy(&bright) > 1.5 * energy(&plain));
    }

    #[test]
    fn another_model_takes_over_while_the_old_tail_rings_out() {
        let rate = 48_000.0;
        let hall = ReverbSettings {
            decay_seconds: 1.0,
            ..ReverbSettings::default()
        };
        let mut kernel = kernel(&hall, rate);
        let mut random = Random::new(12);
        for _ in 0..48_000 {
            let x = 0.25 * random.bipolar();
            kernel.process(x, x);
        }
        let level = |kernel: &mut ReverbKernel, samples: usize| -> f64 {
            let energy: f64 = (0..samples)
                .map(|_| f64::from(kernel.process(0.0, 0.0).0).powi(2))
                .sum();
            (energy / samples as f64).sqrt()
        };
        let mut reference = kernel.clone();
        kernel.set(&ReverbSettings {
            model: ReverbModel::Room,
            ..hall
        });
        // The input has stopped: what sounds is the hall's tail, as if
        // nothing had been switched.
        for _ in 0..4 {
            let (switched, kept) = (level(&mut kernel, 4800), level(&mut reference, 4800));
            let difference = 20.0 * (switched / kept).log10();
            assert!(difference.abs() < 1.0, "{difference} dB");
        }
        // After the ring-out the hall is cleared and stops running.
        for _ in 0..4 * 48_000 {
            kernel.process(0.0, 0.0);
        }
        assert!(!kernel.slots[ReverbModel::Hall as usize].running);
        assert!(kernel.slots[ReverbModel::Room as usize].running);
    }
}
