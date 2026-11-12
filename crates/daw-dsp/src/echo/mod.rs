//! A stereo feedback echo on three delay media: a clean digital line, a
//! bucket-brigade line ([`BucketLine`]) and a tape machine
//! ([`TapeTransport`]), each with up to four playback heads and an allpass
//! diffuser in its loop. The kernel gives the wet signal alone, without
//! latency.
mod bucket;
mod tape;

pub use bucket::{BucketFilter, BucketLine};
pub use tape::TapeTransport;

use super::{
    AlgebraicSigmoid, AntialiasedShaper, Compander, CutoffGlide, DelayAllpass, DelayLine, FadeTap,
    Lfo, LfoShape, LinearRamp, Oversampler2x, Random, Shelf1, SincTable, Smoothed, Svf, SvfMix,
    flush, glide_length, prewarp, soft_limit,
};
use crate::reverb::{CUT_OFF_HIGH_HZ, CUT_OFF_LOW_HZ, blend, cut_share};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoModel {
    /// Every repeat is the input again, only quieter and filtered.
    Digital,
    /// A bucket-brigade line: its clock sets the bandwidth, a compander
    /// surrounds it, a time change moves the clock and bends the pitch.
    Bbd,
    /// Repeats saturate and lose their top; the transport wobbles; a time
    /// change bends the pitch while the motor catches up.
    Tape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoStereo {
    Mono,
    /// Repeats alternate between left and right.
    PingPong,
    /// One line per channel, the right one slightly longer.
    Wide,
}

/// Playback heads along the line. The last one sits at the full time and is
/// the only one that feeds back; the others play at fractions of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapPattern {
    Single,
    Double,
    Triple,
    Quad,
    Dotted,
    Swing,
    Gallop,
    Golden,
}

/// Heads of a line, the main one last.
pub const HEADS: usize = 4;
const MAIN: usize = HEADS - 1;

impl TapPattern {
    pub const ALL: [Self; 8] = [
        Self::Single,
        Self::Double,
        Self::Triple,
        Self::Quad,
        Self::Dotted,
        Self::Swing,
        Self::Gallop,
        Self::Golden,
    ];

    /// Where the heads before the main one sit, as fractions of the time.
    pub fn heads(self) -> [Option<f32>; MAIN] {
        match self {
            Self::Single => [None, None, None],
            Self::Double => [Some(0.5), None, None],
            Self::Triple => [Some(1.0 / 3.0), Some(2.0 / 3.0), None],
            Self::Quad => [Some(0.25), Some(0.5), Some(0.75)],
            Self::Dotted => [Some(0.75), None, None],
            Self::Swing => [Some(2.0 / 3.0), None, None],
            Self::Gallop => [Some(0.5), Some(0.75), None],
            Self::Golden => [Some(0.382), Some(0.618), None],
        }
    }
}

/// Plain-unit settings of an [`EchoKernel`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EchoSettings {
    pub model: EchoModel,
    /// Spacing of the repeats, up to [`MAX_SECONDS`].
    pub seconds: f32,
    /// 0–1; see [`loop_gain`].
    pub feedback: f32,
    pub stereo: EchoStereo,
    pub taps: TapPattern,
    /// 0–1: allpass smear inside the loop.
    pub diffusion: f32,
    /// −1 – 1, each model's own tone: Digital's high cut ([`digital_cut_hz`]),
    /// BBD's clock ([`bbd_clock_hz`]), Tape's condition ([`tape_loss_hz`]
    /// and drive).
    pub color: f32,
    /// 0–1: drift of the time (Digital), of the clock (BBD); wow, flutter
    /// and drift (Tape).
    pub wobble: f32,
    /// In the loop. Off at 20 Hz for Digital; BBD and Tape never pass DC.
    pub low_cut_hz: f32,
}

impl Default for EchoSettings {
    fn default() -> Self {
        Self {
            model: EchoModel::Digital,
            seconds: 0.375,
            feedback: 0.35,
            stereo: EchoStereo::PingPong,
            taps: TapPattern::Single,
            diffusion: 0.0,
            color: 0.0,
            wobble: 0.0,
            low_cut_hz: 120.0,
        }
    }
}

pub const MAX_SECONDS: f32 = 4.0;
/// Samples a signal takes around the loop of Tape besides the line itself
/// and the diffusers: the read-before-write order and the oversampled
/// saturator. The read position makes up for it. Digital's is 1.
pub const LOOP_DELAY: usize = 1 + Oversampler2x::LATENCY as usize + STAGE_DELAY;
/// The shaper's two samples at the doubled rate.
const STAGE_DELAY: usize = 1;

/// The longest line: the time, Wide's offset, the wobble's reach and the
/// interpolator's taps.
const LINE_SECONDS: f32 = 4.3;
const SHORTEST_SECONDS: f32 = 0.001;
const DIGITAL_CEILING: f32 = 4.0;
/// The rails of a bucket-brigade line alone would let a loop above unity
/// settle some 20 dB over full scale (the compander squares what they
/// bound); the loop's input is held here instead.
const BBD_CEILING: f32 = 1.6;
/// Loop gain at full feedback of BBD and Tape: they self-oscillate,
/// bounded by their saturation.
const OSCILLATING_GAIN: f32 = 1.06;
const OSCILLATION_FROM: f32 = 0.9;

const TIME_FADE_SECONDS: f32 = 0.04;
const TIME_GLIDE_SECONDS: f32 = 0.2;
/// Samples of delay per sample: a line never stops or runs backward.
const FASTEST_GLIDE: f64 = 0.9;
const MODEL_DUCK_SECONDS: f32 = 0.01;
const ROUTING_SECONDS: f32 = 0.02;

const WOW_HZ: f32 = 0.5;
const WOW_SHARE: f32 = 0.015;
const FLUTTER_HZ: f32 = 6.5;
const FLUTTER_SHARE: f32 = 0.004;
const DRIFT_HZ: f32 = 1.3;
const DRIFT_SHARE: f32 = 0.0015;
const TICK: u32 = TapeTransport::TICK;

const DIFFUSERS: usize = 4;
/// How the diffusers of each line share their total length.
const DIFFUSER_SPLIT: [[f64; DIFFUSERS]; 2] = [[0.37, 0.27, 0.21, 0.15], [0.35, 0.29, 0.19, 0.17]];
const DIFFUSER_GAIN: f32 = 0.7;
const LONGEST_SMEAR_SECONDS: f64 = 0.04;
/// How the diffusers of a bending line follow their share of its time: the
/// coefficient per sample, and the most samples of length per sample.
const DIFFUSER_GLIDE: f64 = 1.0 / 64.0;
const FASTEST_DIFFUSER_GLIDE: f64 = 0.5;
/// Samples of length per sample on the way to the whole lengths at the
/// end: a rounding of half a sample, taken too slowly to hear as pitch.
const DIFFUSER_LANDING: f64 = 1.0 / 4096.0;
/// Seconds the tail grows by at full Diffusion.
const SMEAR_TAIL_SECONDS: f32 = 0.5;

const DIGITAL_CUT_HZ: f32 = 8000.0;
const TAPE_EMPHASIS_HZ: (f32, f32) = (1500.0, 4200.0);
const TAPE_DRIVE_DB: f32 = 6.0;
const SHAPER_LIMIT: f64 = 1e4;
/// With the shaper's own droop this stays at or below unit gain at every
/// frequency, which a loop needs.
const DROOP_CORRECTION: f64 = 0.125;

const BBD_SLOWEST_CLOCK_HZ: f64 = 2000.0;
const BBD_FASTEST_CLOCK_HZ: f64 = 96_000.0;
/// The filters' corner as a share of the clock.
const BBD_CORNER_SHARE: f64 = 0.35;
const BBD_NOISE_DB: f32 = -60.0;
const BBD_DRAIN_FADE_SECONDS: f32 = 0.02;
const BBD_HANDOVER_SECONDS: f32 = 0.005;
const BBD_DC_HZ: f32 = 5.0;
const BBD_SILENCE: f32 = 1e-9;
/// Stages a head is at least behind the input.
const BBD_FLOOR_STAGES: f64 = 4.0;

/// The loop gain for a feedback of 0–1. Digital holds forever at 1; BBD and
/// Tape pass unity at 96 % and oscillate above.
pub fn loop_gain(model: EchoModel, feedback: f32) -> f32 {
    let feedback = feedback.clamp(0.0, 1.0);
    if model == EchoModel::Digital || feedback <= OSCILLATION_FROM {
        feedback
    } else {
        OSCILLATION_FROM
            + (feedback - OSCILLATION_FROM) * (OSCILLATING_GAIN - OSCILLATION_FROM)
                / (1.0 - OSCILLATION_FROM)
    }
}

/// How much later the right line of [`EchoStereo::Wide`] repeats.
pub fn wide_offset(seconds: f32) -> f32 {
    (0.03 * seconds).clamp(0.003, 0.015)
}

/// Digital's Color: the corner of the high cut in its loop. 8 kHz at the
/// centre, down to 1 kHz at −1, up to 20 kHz toward +1, and none at +1,
/// where the repeats are exact.
pub fn digital_cut_hz(color: f32, sample_rate: f32) -> Option<f32> {
    let color = color.clamp(-1.0, 1.0);
    if color >= 1.0 {
        None
    } else if color <= 0.0 {
        Some(DIGITAL_CUT_HZ * 8.0_f32.powf(color))
    } else {
        Some((DIGITAL_CUT_HZ * 2.5_f32.powf(color)).min(0.45 * sample_rate))
    }
}

/// Tape's Color and time: where the heads have lost 3 dB. A slower tape
/// (a longer time) loses more; a worn one (Color to the left) more still.
pub fn tape_loss_hz(seconds: f32, color: f32) -> f32 {
    let speed = (9000.0 * 0.3 / seconds.max(SHORTEST_SECONDS)).clamp(2500.0, 16_000.0);
    (speed * 2.0_f32.powf(1.5 * color.clamp(-1.0, 1.0))).clamp(1000.0, 20_000.0)
}

fn bbd_clock_limits(sample_rate: f64) -> (f64, f64) {
    (
        BBD_SLOWEST_CLOCK_HZ,
        BBD_FASTEST_CLOCK_HZ.min(2.0 * sample_rate),
    )
}

/// The clock a designer would give a line of this time: longer lines run
/// slower, but not in proportion (they get more stages), and Color moves
/// it an octave either way.
fn bbd_law_hz(seconds: f64, color: f64, sample_rate: f64) -> f64 {
    let (slowest, fastest) = bbd_clock_limits(sample_rate);
    (10_000.0 * (0.3 / seconds).sqrt() * 2.0_f64.powf(color.clamp(-1.0, 1.0)))
        .clamp(slowest, fastest)
}

/// The corner of a bucket-brigade line's filters for a clock. Held at a
/// fifth of the sample rate, above which the model's own image of the
/// sample rate would leak through.
pub fn bbd_corner_hz(clock_hz: f64, sample_rate: f64) -> f64 {
    (BBD_CORNER_SHARE * clock_hz)
        .min(18_000.0)
        .min(0.2 * sample_rate)
}

/// Seconds both filters of a line delay low frequencies by.
fn bbd_filter_seconds(clock_hz: f64, sample_rate: f64) -> f64 {
    2.0 * BucketFilter::group_delay(bbd_corner_hz(clock_hz, sample_rate))
}

/// The stage count that puts a line of `seconds` (of which
/// `bucket_seconds` are left to the buckets and their filters) on its
/// law's clock.
fn bbd_stages(seconds: f64, bucket_seconds: f64, color: f64, sample_rate: f64) -> u32 {
    let law = bbd_law_hz(seconds, color, sample_rate);
    let pairs = ((bucket_seconds - bbd_filter_seconds(law, sample_rate)) * law).round();
    (2.0 * pairs.clamp(4.0, (BucketLine::QUEUE - 1) as f64)) as u32
}

/// The clock at which `stages` and the filters that follow it take
/// `bucket_seconds` together.
fn bbd_solve_clock(stages: u32, bucket_seconds: f64, sample_rate: f64) -> f64 {
    let pairs = f64::from(stages) / 2.0;
    let bucket_seconds = bucket_seconds.max(1e-5);
    let cap = 18_000.0_f64.min(0.2 * sample_rate);
    // While the corner follows the clock, the filters' delay is a fixed
    // number of clock periods.
    let periods = 2.0 * BucketFilter::group_delay(BBD_CORNER_SHARE);
    let open = (pairs + periods) / bucket_seconds;
    if BBD_CORNER_SHARE * open <= cap {
        open
    } else {
        pairs / (bucket_seconds - 2.0 * BucketFilter::group_delay(cap)).max(1e-5)
    }
}

/// A line's loop time in whole samples: the time, Wide's offset on the
/// second line, and not less than the model's own loop takes.
fn line_period(settings: &EchoSettings, line: usize, sample_rate: f32) -> f64 {
    let mut seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
    if line == 1 && settings.stereo == EchoStereo::Wide {
        seconds += wide_offset(seconds);
    }
    let period = f64::from(seconds * sample_rate).round();
    period.max(overhead(settings, sample_rate).ceil() + DIFFUSERS as f64)
}

/// Samples of a model's loop that neither the medium nor the diffusers
/// can give up: its own delay and the shortest read.
fn overhead(settings: &EchoSettings, sample_rate: f32) -> f64 {
    match settings.model {
        EchoModel::Digital => 2.0,
        EchoModel::Tape => (LOOP_DELAY + SincTable::REACH) as f64,
        EchoModel::Bbd => {
            let rate = f64::from(sample_rate);
            let seconds = f64::from(settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS));
            let law = bbd_law_hz(seconds, f64::from(settings.color), rate);
            1.0 + (bbd_filter_seconds(law, rate) + 0.5 * BBD_FLOOR_STAGES / law) * rate
        }
    }
}

/// The lengths of a line's four diffusers in samples: an eighth of the
/// time between them (the first head of four sits at a quarter and must
/// stay behind them), at most 40 ms, and what the loop can spare.
fn diffuser_lengths(settings: &EchoSettings, line: usize, sample_rate: f32) -> [usize; DIFFUSERS] {
    let period = line_period(settings, line, sample_rate);
    let total = smear_samples(period, overhead(settings, sample_rate), sample_rate);
    DIFFUSER_SPLIT[line].map(|share| ((total * share).round() as usize).max(1))
}

/// The diffusers' total length for a loop time, both in samples.
fn smear_samples(period: f64, overhead: f64, sample_rate: f32) -> f64 {
    (period / 8.0)
        .min(LONGEST_SMEAR_SECONDS * f64::from(sample_rate))
        .min(period - overhead)
}

/// Samples of the loop that are neither diffusers nor the medium.
fn loop_constant(settings: &EchoSettings, clock_hz: f64, sample_rate: f32) -> f64 {
    match settings.model {
        EchoModel::Digital => 1.0,
        EchoModel::Tape => LOOP_DELAY as f64,
        EchoModel::Bbd => {
            let rate = f64::from(sample_rate);
            1.0 + bbd_filter_seconds(clock_hz, rate) * rate
        }
    }
}

/// The clock a settled bucket-brigade line of this time and Color runs at.
pub fn bbd_clock_hz(settings: &EchoSettings, sample_rate: f32) -> f64 {
    let rate = f64::from(sample_rate);
    let period = line_period(settings, 0, sample_rate);
    let smear: usize = diffuser_lengths(settings, 0, sample_rate).iter().sum();
    let bucket_seconds = (period - 1.0 - smear as f64) / rate;
    let stages = bbd_stages(
        period / rate,
        bucket_seconds,
        f64::from(settings.color),
        rate,
    );
    let (slowest, fastest) = bbd_clock_limits(rate);
    bbd_solve_clock(stages, bucket_seconds, rate).clamp(slowest, fastest)
}

fn low_cut_is_off(settings: &EchoSettings) -> bool {
    settings.model == EchoModel::Digital && settings.low_cut_hz <= CUT_OFF_LOW_HZ
}

/// The heads of the first echoes as `(seconds, left gain, right gain)`, in
/// order of arrival, without the loop's filters. Returns how many of `out`
/// are filled: the heads of the repeats above −60 dB.
pub fn taps(settings: &EchoSettings, out: &mut [(f32, f32, f32)]) -> usize {
    let gain = loop_gain(settings.model, settings.feedback);
    let seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
    let later = seconds + wide_offset(seconds);
    let pattern = settings.taps.heads();
    let fractions = pattern.iter().flatten().copied().chain([1.0]);
    let mut count = 0;
    let mut level = 1.0_f32;
    let mut repeat = 0;
    while count < out.len() && level >= 1e-3 {
        for fraction in fractions.clone() {
            let time = |seconds: f32| (repeat as f32 + fraction) * seconds;
            let heads: &[(f32, f32, f32)] = match settings.stereo {
                EchoStereo::Mono => &[(time(seconds), level, level)],
                EchoStereo::PingPong if repeat % 2 == 0 => &[(time(seconds), level, 0.0)],
                EchoStereo::PingPong => &[(time(seconds), 0.0, level)],
                EchoStereo::Wide => &[(time(seconds), level, 0.0), (time(later), 0.0, level)],
            };
            for head in heads {
                if count < out.len() {
                    out[count] = *head;
                    count += 1;
                }
            }
        }
        repeat += 1;
        level *= gain;
    }
    if settings.stereo == EchoStereo::Wide {
        out[..count].sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    count
}

/// Seconds a repeat is smeared over by the diffusers: their lengths times
/// Diffusion. The repeat's energy stays centred on its time.
pub fn smear_seconds(settings: &EchoSettings) -> f32 {
    // The lengths do not depend on the rate but for their rounding.
    let rate = 48_000.0;
    let samples: usize = diffuser_lengths(settings, 0, rate).iter().sum();
    settings.diffusion.clamp(0.0, 1.0) * samples as f32 / rate
}

/// Magnitude of one pass through the loop at `hz` for small signals,
/// without the loop gain: what each repeat loses to the filters and the
/// medium.
pub fn repeat_response(settings: &EchoSettings, sample_rate: f32, hz: f64) -> f64 {
    let omega = std::f64::consts::TAU * hz / f64::from(sample_rate);
    let butterworth = |corner: f32, high_pass: bool| {
        let g = f64::from(prewarp(corner, sample_rate));
        let k = std::f64::consts::SQRT_2;
        let mix = if high_pass {
            SvfMix::<f64>::new(g, k, 1.0, -k, -1.0)
        } else {
            SvfMix::<f64>::new(g, k, 0.0, 0.0, 1.0)
        };
        mix.response(omega).norm()
    };
    let mut gain = 1.0;
    if !low_cut_is_off(settings) {
        gain *= butterworth(settings.low_cut_hz.max(CUT_OFF_LOW_HZ), true);
    }
    match settings.model {
        EchoModel::Digital => {
            if let Some(corner) = digital_cut_hz(settings.color, sample_rate) {
                gain *= butterworth(corner, false);
            }
        }
        EchoModel::Bbd => {
            let clock = bbd_clock_hz(settings, sample_rate);
            let corner = bbd_corner_hz(clock, f64::from(sample_rate));
            // Each bucket's value is held for a clock period.
            let x = std::f64::consts::PI * hz / clock;
            let hold = if x.abs() < 1e-9 {
                1.0
            } else {
                (x.sin() / x).abs()
            };
            gain *= hold * BucketFilter::magnitude(corner, hz).powi(2);
            gain *= Shelf1::high_pass(BBD_DC_HZ, sample_rate)
                .response(omega)
                .norm();
        }
        EchoModel::Tape => {
            let seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
            let loss = Shelf1::low_pass(
                TAPE_LOSS_SPLIT * tape_loss_hz(seconds, settings.color),
                sample_rate,
            );
            gain *= loss.response(omega).norm().powi(2);
            // The shaper's averaging and its corrector, at the doubled
            // rate, and the read's interpolator.
            let cos = (0.5 * omega).cos();
            gain *= (6.0 + 2.0 * cos) / 8.0 * (1.0 + 2.0 * DROOP_CORRECTION * (1.0 - cos));
            gain *= SincTable::PASSIVE_GAIN;
        }
    }
    gain
}

/// Two one-poles in a row are 3 dB down at the loss frequency when each
/// has its corner this much higher.
const TAPE_LOSS_SPLIT: f32 = 1.554;

/// Seconds until the repeats have fallen by 60 dB, at most 30; 30 when they
/// never do.
pub fn tail_seconds(settings: &EchoSettings) -> f32 {
    let gain = loop_gain(settings.model, settings.feedback);
    let mut seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
    if settings.stereo == EchoStereo::Wide {
        seconds += wide_offset(seconds);
    }
    let smear = SMEAR_TAIL_SECONDS * settings.diffusion.clamp(0.0, 1.0);
    if gain >= 1.0 {
        30.0
    } else if gain <= 0.0 {
        seconds + smear
    } else {
        (seconds * (3.0 / -gain.log10()).ceil() + smear).min(30.0)
    }
}

/// What the kernel hands a line for one sample.
struct Shared<'a> {
    model: EchoModel,
    sinc: &'a SincTable,
    /// Of the heads before the main one.
    head_gains: [f32; MAIN],
    wobbling: bool,
    low_g: f32,
    /// Share of the low cut, and of Digital's Color cut, in the signal:
    /// 0 while it is off.
    low_share: f32,
    color_g: f32,
    color_share: f32,
    /// Gain of the diffusers.
    diffusion: f32,
    /// Tape: into the saturator, and its inverse out of it.
    drive: f32,
    /// BBD: the line's noise at an envelope of 1.
    noise: [f32; 2],
    glide: f64,
    /// The dip around a change of model.
    duck: f32,
}

#[derive(Clone, Debug)]
struct EchoLine {
    line: DelayLine,
    /// The loop time in samples.
    period: f64,
    low_cut: Svf,
    diffusers: [DelayAllpass; DIFFUSERS],
    /// Sum of the diffusers' set lengths.
    smear: f64,
    /// Tape and BBD: the diffusers' lengths now, which glide with the
    /// time so that they repitch with the medium, and their sum.
    lengths: [f64; DIFFUSERS],
    smear_now: f64,
    split: [f64; DIFFUSERS],
    /// What the loop takes besides the medium and the diffusers, and the
    /// sample rate: what [`smear_samples`] needs.
    overhead: f64,
    sample_rate: f32,
    /// Digital: the heads move by crossfading.
    heads: [FadeTap; HEADS],
    color_cut: Svf,
    /// Digital: offset of each head's read in samples, and its change per
    /// sample.
    wobble: [f32; HEADS],
    wobble_step: [f32; HEADS],
    /// Tape: each head's delay in samples, and its change per sample.
    head_delay: [f64; HEADS],
    head_step: [f64; HEADS],
    loss: [Shelf1; 2],
    emphasis: Shelf1,
    de_emphasis: Shelf1,
    over: Oversampler2x,
    shaper: AntialiasedShaper<AlgebraicSigmoid>,
    /// BBD: the loop time in samples glides, which moves the clock.
    delay: f64,
    compander: Compander,
    /// The expander of the heads before the main one.
    expand_taps: Compander,
    dc: [Shelf1; 2],
    /// Two lines: one takes the input, the other drains after a change of
    /// the stage count.
    buckets: [BucketLine; 2],
    filters: [BucketFilter; 2],
    stages: [u32; 2],
    offsets: [[usize; HEADS]; 2],
    active: usize,
    /// Half the active line's clock period in samples.
    half_period: LinearRamp<f64>,
    /// The draining line's.
    old_half_period: f64,
    /// Samples the other line still drains for.
    drain: u32,
    drain_gain: LinearRamp,
    /// The share of the input the active line takes: it rises from 0 after
    /// a handover while the other line's falls, so that neither line's
    /// filters meet a step.
    handover: LinearRamp,
    /// The main head and the sum of the others, for the next sample.
    bucket_out: [f32; 2],
}

impl EchoLine {
    fn new(index: usize) -> Self {
        let mut shaper = AntialiasedShaper::new(AlgebraicSigmoid::new(1.0, 1.0, 0.0));
        shaper.corrector.c = DROOP_CORRECTION;
        Self {
            line: DelayLine::default(),
            period: 0.0,
            low_cut: Svf::default(),
            diffusers: std::array::from_fn(|_| DelayAllpass::default()),
            smear: 0.0,
            lengths: [1.0; DIFFUSERS],
            smear_now: 0.0,
            split: DIFFUSER_SPLIT[index],
            overhead: 0.0,
            sample_rate: 48_000.0,
            heads: [FadeTap::default(); HEADS],
            color_cut: Svf::default(),
            wobble: [0.0; HEADS],
            wobble_step: [0.0; HEADS],
            head_delay: [0.0; HEADS],
            head_step: [0.0; HEADS],
            loss: [Shelf1::default(), Shelf1::default()],
            emphasis: Shelf1::default(),
            de_emphasis: Shelf1::default(),
            over: Oversampler2x::new(),
            shaper,
            delay: 0.0,
            compander: Compander::default(),
            expand_taps: Compander::default(),
            dc: [Shelf1::default(), Shelf1::default()],
            buckets: [BucketLine::default(), BucketLine::default()],
            filters: [BucketFilter::default(), BucketFilter::default()],
            stages: [8; 2],
            offsets: [[0; HEADS]; 2],
            active: 0,
            half_period: LinearRamp::new(1.0),
            old_half_period: 1.0,
            drain: 0,
            drain_gain: LinearRamp::new(1.0),
            handover: LinearRamp::new(1.0),
            bucket_out: [0.0; 2],
        }
    }

    /// Moves the diffusers of a line that bends with its loop time of
    /// `delay` samples, on its way to the set time: their share of it, and
    /// the whole lengths once it has arrived. A crossfade to the new
    /// lengths would comb what is written for its duration.
    #[inline]
    fn glide_diffusers(&mut self, delay: f64) {
        let arrived = (delay - self.period).abs() < 0.5;
        let share = smear_samples(delay, self.overhead, self.sample_rate);
        self.smear_now = 0.0;
        for index in 0..DIFFUSERS {
            let wanted = if arrived {
                self.diffusers[index].length() as f64
            } else {
                (self.split[index] * share).max(1.0)
            };
            let length = &mut self.lengths[index];
            *length = if arrived {
                *length + (wanted - *length).clamp(-DIFFUSER_LANDING, DIFFUSER_LANDING)
            } else {
                glide_length(*length, wanted, DIFFUSER_GLIDE, FASTEST_DIFFUSER_GLIDE)
            };
            self.smear_now += *length;
        }
    }

    /// Where a Digital head reads: its share of the time, less the
    /// diffusers before it and the sample the loop takes.
    fn digital_tap(&self, fraction: f32) -> usize {
        ((f64::from(fraction) * self.period).round() - self.smear - 1.0).max(0.0) as usize
    }

    /// The input ticks a head of the bucket line `index` reads behind the
    /// newest bucket: its share of the stages, as far as the diffusers and
    /// filters, which delay every head alike, leave it.
    fn bucket_offset(&self, index: usize, fraction: f32, constant: f64) -> usize {
        let pairs = f64::from(self.stages[index]) / 2.0;
        let fixed = self.smear + constant;
        let medium = (self.period - fixed).max(1.0);
        let share = (f64::from(fraction) * self.period - fixed) / medium;
        let ticks = (share * pairs).round().max(0.5 * BBD_FLOOR_STAGES);
        (ticks as usize - 1).min(BucketLine::QUEUE - 1)
    }

    /// Clears what a model keeps besides the line and settles it.
    fn reset_model(&mut self, fractions: &[f32; HEADS]) {
        for filter in [&mut self.emphasis, &mut self.de_emphasis]
            .into_iter()
            .chain(&mut self.dc)
        {
            filter.reset();
        }
        self.over.reset();
        self.shaper.reset();
        self.compander.reset();
        self.expand_taps.reset();
        self.delay = self.period;
        for (length, diffuser) in self.lengths.iter_mut().zip(&mut self.diffusers) {
            diffuser.snap_length(diffuser.length());
            *length = diffuser.length() as f64;
        }
        self.smear_now = self.smear;
        self.buckets.iter_mut().for_each(BucketLine::reset);
        self.active = 0;
        self.drain = 0;
        self.handover = LinearRamp::new(1.0);
        self.bucket_out = [0.0; 2];
        self.wobble = [0.0; HEADS];
        self.wobble_step = [0.0; HEADS];
        self.head_step = [0.0; HEADS];
        for (head, fraction) in fractions.iter().enumerate() {
            self.heads[head].snap(self.digital_tap(*fraction));
            self.head_delay[head] = f64::from(*fraction) * self.period;
        }
    }

    /// The saturator, antialiased at twice the rate.
    #[inline]
    fn saturate(&mut self, input: f32) -> f32 {
        let mut pair = self.over.upsample(flush(input));
        for sample in &mut pair {
            let x = f64::from(*sample).clamp(-SHAPER_LIMIT, SHAPER_LIMIT);
            *sample = flush(self.shaper.process(x) as f32);
        }
        self.over.downsample(pair)
    }

    /// The main head and the sum of all heads.
    #[inline]
    fn read(&mut self, shared: &Shared) -> (f32, f32) {
        if shared.model == EchoModel::Tape {
            self.glide_diffusers(self.head_delay[MAIN]);
        }
        let smear = self.smear_now;
        if shared.model == EchoModel::Bbd {
            self.delay = glide_length(self.delay, self.period, shared.glide, FASTEST_GLIDE);
            self.glide_diffusers(self.delay);
            let main = self
                .compander
                .expand(self.dc[0].process(self.bucket_out[0]));
            let others = self
                .expand_taps
                .expand(self.dc[1].process(self.bucket_out[1]));
            return (main, main + others);
        }
        let mut main = 0.0;
        let mut sum = 0.0;
        for head in 0..HEADS {
            let gain = if head == MAIN {
                1.0
            } else {
                shared.head_gains[head]
            };
            if gain == 0.0 {
                continue;
            }
            let line = &self.line;
            let sinc = |position: f64| {
                let position = position.max(SincTable::REACH as f64);
                let whole = position.floor();
                line.read_sinc(whole as usize, (position - whole) as f32, shared.sinc)
            };
            let value = if shared.model == EchoModel::Tape {
                self.head_delay[head] += self.head_step[head];
                sinc(self.head_delay[head] - smear - LOOP_DELAY as f64)
            } else if shared.wobbling {
                self.wobble[head] += self.wobble_step[head];
                let offset = f64::from(self.wobble[head]);
                self.heads[head].read_with(|tap| sinc(tap as f64 + offset))
            } else {
                self.heads[head].read(line)
            };
            if head == MAIN {
                main = value;
            }
            sum += gain * value;
        }
        (main, sum)
    }

    #[inline]
    fn write(&mut self, mut signal: f32, shared: &Shared, line: usize) {
        // The filters run while they are off, so they come in on a signal
        // they already know.
        let cut = self
            .low_cut
            .process(signal, shared.low_g, std::f32::consts::SQRT_2)
            .high;
        signal = blend(signal, cut, shared.low_share);
        let cut = self
            .color_cut
            .process(signal, shared.color_g, std::f32::consts::SQRT_2)
            .low;
        let lost = self.loss[0].process(signal);
        let lost = self.loss[1].process(lost);
        match shared.model {
            EchoModel::Digital => signal = blend(signal, cut, shared.color_share),
            // What the heads lose comes before the emphasis, so less of
            // the top reaches the saturator.
            EchoModel::Tape => signal = lost,
            EchoModel::Bbd => {}
        }
        if shared.model == EchoModel::Digital {
            for diffuser in &mut self.diffusers {
                signal = diffuser.process(signal, shared.diffusion);
            }
        } else {
            for (diffuser, length) in self.diffusers.iter_mut().zip(self.lengths) {
                signal = diffuser.process_at(signal, shared.diffusion, length, shared.sinc);
            }
        }
        // The diffusers still hold sound when a model changes: the medium
        // takes none of it at the bottom of the duck.
        signal *= shared.duck;
        match shared.model {
            EchoModel::Digital => self.line.write(flush(soft_limit(signal, DIGITAL_CEILING))),
            EchoModel::Tape => {
                // Highs meet the curve harder; the inverse shelf makes the
                // pair flat for small signals.
                let driven = self.emphasis.process(shared.drive * signal);
                let shaped = self.saturate(driven);
                let restored = self.de_emphasis.process(shaped) / shared.drive;
                self.line.write(flush(restored));
            }
            EchoModel::Bbd => {
                // The compressor's gain would hold up what is left of a
                // filter ringing down, far below hearing, for minutes.
                if signal.abs() < BBD_SILENCE {
                    signal = 0.0;
                }
                let mut compressed = self.compander.compress(soft_limit(signal, BBD_CEILING));
                // The line's noise breathes with the signal through the
                // expander, and silence stays silence.
                let envelope = self.compander.envelope();
                if envelope > 1e-6 {
                    compressed += shared.noise[line] * envelope;
                }
                let active = self.active;
                let share = self.handover.next();
                let mut out = self.buckets[active].process(
                    share * compressed,
                    &self.filters[active],
                    self.half_period.next(),
                    &self.offsets[active],
                    &shared.head_gains,
                );
                if self.drain > 0 {
                    let old = 1 - active;
                    let gain = self.drain_gain.next();
                    let drained = self.buckets[old].process(
                        (1.0 - share) * compressed,
                        &self.filters[old],
                        self.old_half_period,
                        &self.offsets[old],
                        &shared.head_gains,
                    );
                    out[0] += gain * drained[0];
                    out[1] += gain * drained[1];
                    self.drain -= 1;
                    if self.drain == 0 {
                        self.buckets[old].reset();
                    }
                }
                self.bucket_out = out;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct EchoKernel {
    sample_rate: f32,
    settings: EchoSettings,
    /// The model that runs; follows the settings at the bottom of a duck.
    model: EchoModel,
    pending: Option<EchoModel>,
    duck: LinearRamp,
    lines: [EchoLine; 2],
    /// Where each head sits as a share of the time; the main one at 1.
    fractions: [f32; HEADS],
    head_gain: [LinearRamp; MAIN],
    transport: TapeTransport,
    sinc: SincTable,
    wow: Lfo,
    flutter: Lfo,
    drift: Lfo,
    noise: Random,
    wobble: Smoothed,
    gain: Smoothed,
    diffusion: Smoothed,
    color: Smoothed,
    /// Tape: the saturator's drive and the heads' loss frequency.
    drive: Smoothed,
    loss_hz: Smoothed,
    /// Weights of the Mono, Ping-pong and Wide routings.
    routing: [LinearRamp; 3],
    low: CutoffGlide,
    color_cut: CutoffGlide,
    low_share: LinearRamp,
    color_share: LinearRamp,
    wobbling: bool,
    noise_level: f32,
    glide: f64,
    tick: u32,
}

impl Default for EchoKernel {
    fn default() -> Self {
        let settings = EchoSettings::default();
        Self {
            sample_rate: 48_000.0,
            settings,
            model: settings.model,
            pending: None,
            duck: LinearRamp::new(1.0),
            lines: [EchoLine::new(0), EchoLine::new(1)],
            fractions: [0.5, 0.5, 0.5, 1.0],
            head_gain: [LinearRamp::new(0.0); MAIN],
            transport: TapeTransport::default(),
            sinc: SincTable::passive(1.0),
            wow: Lfo::new(0x00EC_0001),
            flutter: Lfo::new(0x00EC_0002),
            drift: Lfo::new(0x00EC_0003),
            noise: Random::new(0x00EC_0004),
            wobble: Smoothed::new(settings.wobble),
            gain: Smoothed::new(settings.feedback),
            diffusion: Smoothed::new(0.0),
            color: Smoothed::new(settings.color),
            drive: Smoothed::new(1.0),
            loss_hz: Smoothed::new(9000.0),
            routing: [LinearRamp::new(0.0); 3],
            low: CutoffGlide::new(settings.low_cut_hz),
            color_cut: CutoffGlide::new(DIGITAL_CUT_HZ),
            low_share: LinearRamp::new(1.0),
            color_share: LinearRamp::new(1.0),
            wobbling: false,
            noise_level: 0.0,
            glide: 1.0,
            tick: 0,
        }
    }
}

impl EchoKernel {
    /// Allocates for a sample rate and resets.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let fade = (TIME_FADE_SECONDS * sample_rate) as u32;
        let smear = (LONGEST_SMEAR_SECONDS * f64::from(sample_rate)).ceil() as usize;
        for line in &mut self.lines {
            line.line.allocate((LINE_SECONDS * sample_rate) as usize);
            for head in &mut line.heads {
                head.set_fade(fade);
            }
            for diffuser in &mut line.diffusers {
                diffuser.allocate(smear / 2);
                diffuser.set_fade(fade);
            }
            line.compander.prepare(sample_rate);
            line.expand_taps.prepare(sample_rate);
            for filter in &mut line.dc {
                *filter = Shelf1::high_pass(BBD_DC_HZ, sample_rate);
            }
            line.buckets.iter_mut().for_each(BucketLine::allocate);
            let (zero, pole) = TAPE_EMPHASIS_HZ;
            line.emphasis = Shelf1::shelf(zero, pole, sample_rate);
            line.de_emphasis = Shelf1::inverse_shelf(zero, pole, sample_rate);
        }
        self.transport.prepare(sample_rate, LINE_SECONDS - 0.05);
        self.glide = 1.0 - (-1.0 / f64::from(TIME_GLIDE_SECONDS * sample_rate)).exp();
        for value in [
            &mut self.gain,
            &mut self.diffusion,
            &mut self.color,
            &mut self.drive,
            &mut self.loss_hz,
        ] {
            value.set_time(20.0, sample_rate);
        }
        self.wobble.set_time(50.0, sample_rate);
        self.low.prepare(20.0, sample_rate);
        self.color_cut.prepare(20.0, sample_rate);
        self.reset();
    }

    /// New targets: the time crossfades (Digital), moves the clock (BBD) or
    /// the motor (Tape); the routing and the heads crossfade; the model
    /// changes under a short duck.
    pub fn set(&mut self, settings: &EchoSettings) {
        self.settings = *settings;
        let rate = self.sample_rate;
        for (index, line) in self.lines.iter_mut().enumerate() {
            line.period = line_period(settings, index, rate);
            line.overhead = overhead(settings, rate);
            line.sample_rate = rate;
            let lengths = diffuser_lengths(settings, index, rate);
            for (diffuser, length) in line.diffusers.iter_mut().zip(lengths) {
                diffuser.set_length(length);
            }
            line.smear = lengths.iter().sum::<usize>() as f64;
            for head in 0..HEADS {
                let tap = line.digital_tap(self.fractions[head]);
                let silent = head != MAIN && self.head_gain[head].value() == 0.0;
                if silent {
                    line.heads[head].snap(tap);
                } else {
                    line.heads[head].set(tap);
                }
            }
        }
        let seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
        let color = settings.color.clamp(-1.0, 1.0);
        self.gain
            .set_target(loop_gain(settings.model, settings.feedback));
        self.wobble.set_target(settings.wobble.clamp(0.0, 1.0));
        self.diffusion
            .set_target(DIFFUSER_GAIN * settings.diffusion.clamp(0.0, 1.0));
        self.color.set_target(color);
        self.drive
            .set_target(crate::db_to_gain(-TAPE_DRIVE_DB * color));
        self.loss_hz.set_target(tape_loss_hz(seconds, color));
        self.low.set(settings.low_cut_hz.max(CUT_OFF_LOW_HZ));
        self.color_cut
            .set(digital_cut_hz(color, rate).unwrap_or(CUT_OFF_HIGH_HZ));
        let fade = (ROUTING_SECONDS * rate) as u32;
        for (index, weight) in self.routing.iter_mut().enumerate() {
            let aim = if index == settings.stereo as usize {
                1.0
            } else {
                0.0
            };
            if weight.target() != aim {
                weight.set_target(aim, fade);
            }
        }
        let aim = if settings.model == self.model {
            self.pending = None;
            1.0
        } else {
            self.pending = Some(settings.model);
            0.0
        };
        if self.duck.target() != aim {
            self.duck
                .set_target(aim, (MODEL_DUCK_SECONDS * rate) as u32);
        }
    }

    /// Clears the audio state and settles on the settings.
    pub fn reset(&mut self) {
        let settings = self.settings;
        let pattern = settings.taps.heads();
        for (head, place) in pattern.into_iter().enumerate() {
            if let Some(fraction) = place {
                self.fractions[head] = fraction;
            }
            self.head_gain[head] = LinearRamp::new(if place.is_some() { 1.0 } else { 0.0 });
        }
        self.set(&settings);
        self.model = settings.model;
        self.pending = None;
        self.duck = LinearRamp::new(1.0);
        for line in &mut self.lines {
            line.line.reset();
            line.low_cut.reset();
            line.color_cut.reset();
            line.loss.iter_mut().for_each(Shelf1::reset);
            line.diffusers.iter_mut().for_each(DelayAllpass::reset);
        }
        self.wow.reset();
        self.flutter.reset();
        self.drift.reset();
        self.noise.reset();
        for value in [
            &mut self.wobble,
            &mut self.gain,
            &mut self.diffusion,
            &mut self.color,
            &mut self.drive,
            &mut self.loss_hz,
        ] {
            value.snap();
        }
        for weight in &mut self.routing {
            weight.snap();
        }
        self.low.snap();
        self.color_cut.snap();
        let (low_off, color_off) = self.cuts_off();
        self.low_share = LinearRamp::new(if low_off { 0.0 } else { 1.0 });
        self.color_share = LinearRamp::new(if color_off { 0.0 } else { 1.0 });
        self.tick = 0;
        self.settle_model();
    }

    /// Whether the low cut and Digital's Color cut are off: set so, and
    /// the corner has arrived where it does least.
    fn cuts_off(&self) -> (bool, bool) {
        (
            low_cut_is_off(&self.settings)
                && self.model == EchoModel::Digital
                && self.low.is_settled(),
            self.settings.color >= 1.0 && self.color_cut.is_settled(),
        )
    }

    /// Clears what the model keeps besides the lines and settles it on the
    /// time as it is set.
    fn settle_model(&mut self) {
        let rate = self.sample_rate;
        for line in &mut self.lines {
            line.reset_model(&self.fractions);
        }
        self.transport.reset(f64::from(rate) / self.lines[0].period);
        self.wobbling = false;
        for index in 0..2 {
            self.start_buckets(index);
            self.lines[index].half_period.snap();
        }
        self.update_tape_filters();
    }

    /// Puts the idle bucket line of a line on the stage count its time
    /// asks for and hands it the input.
    fn start_buckets(&mut self, index: usize) {
        let rate = f64::from(self.sample_rate);
        let settings = self.settings;
        let line = &mut self.lines[index];
        let bucket_seconds = (line.delay - 1.0 - line.smear_now) / rate;
        let stages = bbd_stages(
            line.delay / rate,
            bucket_seconds,
            f64::from(settings.color),
            rate,
        );
        let (slowest, fastest) = bbd_clock_limits(rate);
        let clock = bbd_solve_clock(stages, bucket_seconds, rate).clamp(slowest, fastest);
        let active = line.active;
        line.stages[active] = stages;
        line.buckets[active].reset();
        line.filters[active].set(bbd_corner_hz(clock, rate), rate);
        line.half_period = LinearRamp::new(rate / (2.0 * clock));
        self.place_bucket_heads(index, active);
    }

    fn place_bucket_heads(&mut self, index: usize, buckets: usize) {
        let rate = f64::from(self.sample_rate);
        let line = &mut self.lines[index];
        let clock = rate / (2.0 * line.half_period.target());
        let constant = loop_constant(&self.settings, clock, self.sample_rate);
        for head in 0..HEADS {
            line.offsets[buckets][head] = if head == MAIN {
                (line.stages[buckets] / 2) as usize - 1
            } else {
                line.bucket_offset(buckets, self.fractions[head], constant)
            };
        }
    }

    /// The repeat time of the left line now, in seconds: on the way to the
    /// set time while the clock or the motor moves.
    pub fn seconds(&self) -> f32 {
        let line = &self.lines[0];
        match self.model {
            EchoModel::Digital => (line.period / f64::from(self.sample_rate)) as f32,
            EchoModel::Bbd => (line.delay / f64::from(self.sample_rate)) as f32,
            EchoModel::Tape => (1.0 / self.transport.speed()) as f32,
        }
    }

    /// The clock of the left bucket-brigade line now.
    pub fn clock_hz(&self) -> f64 {
        f64::from(self.sample_rate) / (2.0 * self.lines[0].half_period.value())
    }

    fn switch(&mut self, model: EchoModel) {
        // What a bucket-brigade line holds is compressed: it means nothing
        // to the other models, nor theirs to it.
        let clear = model == EchoModel::Bbd || self.model == EchoModel::Bbd;
        self.model = model;
        if clear {
            for line in &mut self.lines {
                line.line.reset();
            }
        }
        self.settle_model();
        // The transport's history is laid out for a whole tick to come.
        self.tick = 0;
    }

    fn update_tape_filters(&mut self) {
        let loss = Shelf1::low_pass(TAPE_LOSS_SPLIT * self.loss_hz.value(), self.sample_rate);
        for filter in self.lines.iter_mut().flat_map(|line| &mut line.loss) {
            filter.set(&loss);
        }
    }

    /// A head's change of pattern: it fades out, moves, and fades in.
    fn move_heads(&mut self) {
        let pattern = self.settings.taps.heads();
        let fade = (ROUTING_SECONDS * self.sample_rate) as u32;
        for (head, place) in pattern.into_iter().enumerate() {
            let gain = &mut self.head_gain[head];
            let silent = gain.settled() && gain.value() == 0.0;
            match place {
                Some(fraction) if fraction == self.fractions[head] || silent => {
                    if silent {
                        self.fractions[head] = fraction;
                        for index in 0..2 {
                            let line = &mut self.lines[index];
                            let tap = line.digital_tap(fraction);
                            line.heads[head].snap(tap);
                            line.head_delay[head] = f64::from(fraction) * line.head_delay[MAIN];
                            line.head_step[head] = 0.0;
                            line.wobble[head] = 0.0;
                            line.wobble_step[head] = 0.0;
                            self.place_bucket_heads(index, 0);
                            self.place_bucket_heads(index, 1);
                        }
                    }
                    let gain = &mut self.head_gain[head];
                    if gain.target() != 1.0 {
                        gain.set_target(1.0, fade);
                    }
                }
                _ => {
                    if gain.target() != 0.0 {
                        gain.set_target(0.0, fade);
                    }
                }
            }
        }
    }

    /// What a transport whose speed errs by `share·sin` does to a delay of
    /// `seconds`, in seconds: the error integrated over the time the sound
    /// spends in the line.
    fn wow_seconds(&self, line: usize, seconds: f32) -> f32 {
        let now = 0.25 + 0.25 * line as f64;
        let then = now - f64::from(WOW_HZ * seconds);
        WOW_SHARE / (std::f32::consts::TAU * WOW_HZ)
            * (self.wow.value(LfoShape::Sine, now) - self.wow.value(LfoShape::Sine, then))
    }

    /// The work of every [`TICK`] samples.
    fn control(&mut self, amount: f32) {
        let rate = self.sample_rate;
        let step = f64::from(TICK as f32 / rate);
        self.move_heads();
        let seconds = (self.lines[0].period / f64::from(rate)) as f32;
        // The transport's faults turn with the tape.
        let turn = if self.model == EchoModel::Tape {
            f64::from((0.3 / seconds).clamp(0.5, 2.0))
        } else {
            1.0
        };
        self.wow.advance(f64::from(WOW_HZ) * turn * step);
        self.flutter.advance(f64::from(FLUTTER_HZ) * turn * step);
        self.drift.advance(f64::from(DRIFT_HZ) * turn * step);
        if !self.loss_hz.is_settled() {
            self.update_tape_filters();
        }
        match self.model {
            EchoModel::Digital => {
                let mut moving = amount > 0.0;
                for index in 0..2 {
                    let period = self.lines[index].period as f32 / rate;
                    for head in 0..HEADS {
                        let aim = if amount > 0.0 {
                            amount * rate * self.wow_seconds(index, self.fractions[head] * period)
                        } else {
                            0.0
                        };
                        let line = &mut self.lines[index];
                        line.wobble_step[head] = (aim - line.wobble[head]) / TICK as f32;
                        if amount == 0.0 && line.wobble_step[head].abs() < 1e-9 {
                            // At rest the read is a whole tap again.
                            line.wobble[head] = 0.0;
                            line.wobble_step[head] = 0.0;
                        }
                        moving |= line.wobble[head] != 0.0 || line.wobble_step[head] != 0.0;
                    }
                }
                self.wobbling = moving;
            }
            EchoModel::Tape => {
                let deviation = f64::from(amount)
                    * f64::from(
                        WOW_SHARE * self.wow.value(LfoShape::Sine, 0.25)
                            + FLUTTER_SHARE * self.flutter.value(LfoShape::Sine, 0.25)
                            + DRIFT_SHARE * self.drift.value(LfoShape::Smooth, 0.0),
                    );
                let main = self.lines[0].period;
                self.transport.advance(f64::from(rate) / main, deviation);
                let limit = FASTEST_GLIDE * f64::from(TICK);
                for line in &mut self.lines {
                    let scale = line.period / main;
                    for head in 0..HEADS {
                        let now = line.head_delay[head];
                        let distance = f64::from(self.fractions[head]) * scale;
                        let solved =
                            self.transport.solve(now / f64::from(rate), distance) * f64::from(rate);
                        let aim = solved.clamp(now - limit, now + limit);
                        line.head_step[head] = (aim - now) / f64::from(TICK);
                    }
                }
            }
            EchoModel::Bbd => {
                let wow = f64::from(amount * WOW_SHARE * self.wow.value(LfoShape::Sine, 0.25));
                for index in 0..2 {
                    self.clock_buckets(index, wow);
                }
                self.noise_level = crate::db_to_gain(BBD_NOISE_DB - 9.0 * self.color.value());
            }
        }
    }

    /// Follows the gliding time with a bucket line's clock, and hands over
    /// to another stage count when the clock would leave its range or the
    /// time has settled on a count that is not its own.
    fn clock_buckets(&mut self, index: usize, wow: f64) {
        let rate = f64::from(self.sample_rate);
        let color = f64::from(self.settings.color);
        let (slowest, fastest) = bbd_clock_limits(rate);
        let line = &mut self.lines[index];
        let bucket_seconds = (line.delay - 1.0 - line.smear_now) / rate;
        let mut clock = bbd_solve_clock(line.stages[line.active], bucket_seconds, rate);
        let wanted = bbd_stages(line.delay / rate, bucket_seconds, color, rate);
        let outside = clock < 0.98 * slowest || clock > 1.02 * fastest;
        let settled = line.delay == line.period;
        let stale =
            settled && (f64::from(wanted) / f64::from(line.stages[line.active]) - 1.0).abs() > 0.01;
        if outside && line.drain > 0 {
            // The last handover still drains: it has to make way first.
            if line.drain_gain.target() != 0.0 {
                let fade = (BBD_DRAIN_FADE_SECONDS * self.sample_rate) as u32;
                line.drain_gain.set_target(0.0, fade);
                line.drain = fade.max(1);
            }
        } else if outside || (stale && line.drain == 0) {
            line.old_half_period = line.half_period.value();
            line.drain = (line.delay + f64::from(BBD_DRAIN_FADE_SECONDS) * rate) as u32;
            line.drain_gain = LinearRamp::new(1.0);
            line.handover = LinearRamp::new(0.0);
            line.handover
                .set_target(1.0, (BBD_HANDOVER_SECONDS * self.sample_rate) as u32);
            line.active = 1 - line.active;
            self.start_buckets(index);
            return;
        }
        clock = clock.clamp(slowest, fastest);
        let active = line.active;
        let corner = bbd_corner_hz(clock, rate);
        // The table is rebuilt for a move of a quarter of a per cent: a
        // smaller one is below hearing.
        if (corner / line.filters[active].corner_hz() - 1.0).abs() > 0.0025 {
            line.filters[active].set(corner, rate);
        }
        line.half_period
            .set_target(rate / (2.0 * clock) * (1.0 - wow), TICK);
    }

    /// One sample in, the echoes out.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let mut duck = self.duck.next();
        if duck == 0.0
            && let Some(model) = self.pending.take()
        {
            self.switch(model);
            self.duck
                .set_target(1.0, (MODEL_DUCK_SECONDS * self.sample_rate) as u32);
            duck = 0.0;
        }
        let model = self.model;
        let amount = self.wobble.next_value();
        self.color.next_value();
        self.loss_hz.next_value();
        if self.tick == 0 {
            self.control(amount);
        }
        self.tick = (self.tick + 1) % TICK;

        let gain = self.gain.next_value();
        let [mono, ping, wide] = self.routing.each_mut().map(LinearRamp::next);
        let noise = if model == EchoModel::Bbd {
            [0; 2].map(|_| self.noise_level * self.noise.bipolar())
        } else {
            [0.0; 2]
        };
        // A cut that goes off or comes on still turns the phase of what
        // is heard: it leaves and enters by a crossfade.
        let (low_off, color_off) = self.cuts_off();
        let fade = (ROUTING_SECONDS * self.sample_rate) as u32;
        let shared = Shared {
            model,
            sinc: &self.sinc,
            head_gains: self.head_gain.each_mut().map(LinearRamp::next),
            wobbling: self.wobbling,
            low_g: self.low.next_g(),
            low_share: cut_share(&mut self.low_share, low_off, fade),
            color_g: self.color_cut.next_g(),
            color_share: cut_share(&mut self.color_share, color_off, fade),
            diffusion: self.diffusion.next_value(),
            drive: self.drive.next_value(),
            noise,
            glide: self.glide,
            duck,
        };
        // Ducked where it is read and where it is written: nothing that
        // jumps at a change of model is heard or stored.
        let [(a_main, a), (b_main, b)] = self.lines.each_mut().map(|line| {
            let (main, sum) = line.read(&shared);
            (duck * main, duck * sum)
        });
        let center = 0.5 * (left + right);
        // Only the main head feeds back.
        let into_a = duck * ((mono + ping) * center + wide * left)
            + gain * ((mono + wide) * a_main + ping * b_main);
        let into_b = duck * wide * right + gain * (ping * a_main + wide * b_main);
        let [first, second] = &mut self.lines;
        first.write(into_a, &shared, 0);
        second.write(into_b, &shared, 1);
        (a, mono * a + (ping + wide) * b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// Digital with nothing in its loop: every repeat is the input.
    fn exact() -> EchoSettings {
        EchoSettings {
            seconds: 0.01,
            feedback: 0.5,
            stereo: EchoStereo::Mono,
            color: 1.0,
            low_cut_hz: 20.0,
            ..EchoSettings::default()
        }
    }

    fn prepared(settings: &EchoSettings) -> EchoKernel {
        let mut kernel = EchoKernel::default();
        kernel.prepare(RATE);
        kernel.set(settings);
        kernel.reset();
        kernel
    }

    fn impulse(settings: &EchoSettings, left: f32, right: f32, frames: usize) -> Vec<(f32, f32)> {
        let mut kernel = prepared(settings);
        (0..frames)
            .map(|n| {
                if n == 0 {
                    kernel.process(left, right)
                } else {
                    kernel.process(0.0, 0.0)
                }
            })
            .collect()
    }

    fn sine(n: usize, hz: f32) -> f32 {
        0.25 * (std::f32::consts::TAU * hz * n as f32 / RATE).sin()
    }

    fn rms(samples: &[f32]) -> f64 {
        (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
    }

    /// The largest second difference: a click stands out of a smooth
    /// signal there.
    fn roughness(samples: &[f32]) -> f32 {
        samples
            .windows(3)
            .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn digital_repeats_are_the_input_at_the_time_and_its_multiples() {
        for seconds in [0.001, 0.005, 0.05, 0.5] {
            let settings = EchoSettings { seconds, ..exact() };
            let period = (seconds * RATE).round() as usize;
            let out = impulse(&settings, 1.0, 1.0, 4 * period + 8);
            for (n, (left, right)) in out.iter().enumerate() {
                let expected = if n > 0 && n % period == 0 {
                    0.5_f32.powi((n / period) as i32 - 1)
                } else {
                    0.0
                };
                assert_eq!(
                    (*left, *right),
                    (expected, expected),
                    "{seconds} s, sample {n}"
                );
            }
        }
    }

    #[test]
    fn ping_pong_alternates_and_wide_keeps_the_sides_apart() {
        let period = 480;
        let settings = EchoSettings {
            stereo: EchoStereo::PingPong,
            ..exact()
        };
        let out = impulse(&settings, 1.0, 0.0, 4 * period + 1);
        // The centre of the input goes left first.
        assert_eq!(out[period], (0.5, 0.0));
        assert_eq!(out[2 * period], (0.0, 0.25));
        assert_eq!(out[3 * period], (0.125, 0.0));

        let settings = EchoSettings {
            stereo: EchoStereo::Wide,
            ..exact()
        };
        let later = period + (wide_offset(0.01) * RATE).round() as usize;
        let out = impulse(&settings, 1.0, 0.5, 2 * later + 1);
        assert_eq!(out[period], (1.0, 0.0));
        assert_eq!(out[later], (0.0, 0.5));
        assert_eq!(out[2 * period], (0.5, 0.0));
        assert_eq!(out[2 * later], (0.0, 0.25));
    }

    #[test]
    fn every_pattern_plays_its_heads_and_only_the_main_one_feeds_back() {
        for model in [EchoModel::Digital, EchoModel::Bbd, EchoModel::Tape] {
            for pattern in TapPattern::ALL {
                for diffusion in [0.0, 1.0] {
                    let settings = EchoSettings {
                        model,
                        seconds: 0.2,
                        feedback: 0.0,
                        taps: pattern,
                        diffusion,
                        color: if model == EchoModel::Digital {
                            1.0
                        } else {
                            0.5
                        },
                        ..exact()
                    };
                    let mut kernel = prepared(&settings);
                    // A burst a model's filters pass, loud enough for the
                    // compander.
                    let frames = (0.45 * RATE) as usize;
                    let out: Vec<f32> = (0..frames)
                        .map(|n| {
                            let burst = if n < 96 {
                                (std::f32::consts::PI * n as f32 / 96.0).sin().powi(2)
                                    * (std::f32::consts::TAU * n as f32 / 24.0).sin()
                            } else {
                                0.0
                            };
                            kernel.process(burst, burst).0
                        })
                        .collect();
                    let total: f64 = out.iter().map(|s| f64::from(*s).powi(2)).sum();
                    let mut fractions: Vec<f32> =
                        pattern.heads().iter().flatten().copied().collect();
                    fractions.push(1.0);
                    let centroid = |from: usize, to: usize| {
                        let energy: f64 = out[from..to].iter().map(|s| f64::from(*s).powi(2)).sum();
                        let moment: f64 = out[from..to]
                            .iter()
                            .enumerate()
                            .map(|(n, s)| (from + n) as f64 * f64::from(*s).powi(2))
                            .sum();
                        (energy, moment / energy)
                    };
                    // The burst's middle is 48 samples in.
                    let centres = fractions
                        .iter()
                        .map(|fraction| fraction * 0.2 * RATE + 48.0);
                    if diffusion == 0.0 {
                        let mut found = 0.0;
                        for centre in centres {
                            let reach = 0.004 * RATE;
                            let (energy, at) =
                                centroid((centre - reach) as usize, (centre + reach) as usize);
                            assert!(energy > 0.0, "{model:?} {pattern:?}: no head at {centre}");
                            assert!(
                                (at - f64::from(centre)).abs() < 0.001 * f64::from(RATE),
                                "{model:?} {pattern:?}: head at {at}, expected {centre}"
                            );
                            found += energy;
                        }
                        assert!(
                            found / total > 0.97,
                            "{model:?} {pattern:?}: {found} of {total} at the heads"
                        );
                    } else {
                        // Smeared heads run into each other, but each stays
                        // centred on its time, so their sum does.
                        let expected = centres.sum::<f32>() / fractions.len() as f32;
                        let (_, at) = centroid(0, out.len());
                        assert!(
                            (at - f64::from(expected)).abs() < 0.004 * f64::from(RATE),
                            "{model:?} {pattern:?} smeared: centred at {at}, expected {expected}"
                        );
                    }
                    // Nothing after the main head: no head fed back.
                    let late = (0.26 * RATE) as usize;
                    let after: f64 = out[late..].iter().map(|s| f64::from(*s).powi(2)).sum();
                    let floor = if model == EchoModel::Bbd { 2e-3 } else { 1e-4 };
                    assert!(
                        after / total < floor || diffusion > 0.0,
                        "{model:?} {pattern:?}: {after} of {total} after the main head"
                    );
                }
            }
        }
    }

    #[test]
    fn diffusion_smears_a_repeat_without_moving_or_colouring_it() {
        let period = (0.25 * RATE) as usize;
        let energy = |diffusion: f32| {
            let settings = EchoSettings {
                seconds: 0.25,
                feedback: 0.0,
                diffusion,
                ..exact()
            };
            let out = impulse(&settings, 1.0, 1.0, 2 * period);
            let energy: f64 = out.iter().map(|s| f64::from(s.0).powi(2)).sum();
            let centroid = out
                .iter()
                .enumerate()
                .map(|(n, s)| n as f64 * f64::from(s.0).powi(2))
                .sum::<f64>()
                / energy;
            let spread = (out
                .iter()
                .enumerate()
                .map(|(n, s)| (n as f64 - centroid).powi(2) * f64::from(s.0).powi(2))
                .sum::<f64>()
                / energy)
                .sqrt();
            (energy, centroid, spread)
        };
        let (plain, at, none) = energy(0.0);
        assert_eq!((plain, at, none), (1.0, period as f64, 0.0));
        let mut last = 0.0;
        for diffusion in [0.25, 0.5, 1.0] {
            let (energy, centroid, spread) = energy(diffusion);
            assert!((energy - 1.0).abs() < 1e-3, "energy {energy}");
            assert!(
                (centroid - period as f64).abs() < 0.001 * f64::from(RATE),
                "{centroid}"
            );
            assert!(spread > last, "spread {spread} after {last}");
            last = spread;
        }
        // At full Diffusion the repeat is spread over about the time
        // `smear_seconds` names.
        let settings = EchoSettings {
            seconds: 0.25,
            diffusion: 1.0,
            ..exact()
        };
        let named = f64::from(smear_seconds(&settings) * RATE);
        assert!(
            (0.3..3.0).contains(&(last / named)),
            "{last} against {named}"
        );
        assert_eq!(smear_seconds(&exact()), 0.0);
    }

    #[test]
    fn full_feedback_holds_in_digital_and_sings_bounded_in_the_others() {
        // Digital: what is in the loop stays, to a rounding.
        let settings = EchoSettings {
            seconds: 0.1,
            feedback: 1.0,
            ..exact()
        };
        let mut kernel = prepared(&settings);
        let period = 4800;
        let mut levels = Vec::new();
        let mut block = Vec::with_capacity(period);
        for n in 0..100 * period {
            let input = if n < period { sine(n, 440.0) } else { 0.0 };
            block.push(kernel.process(input, input).0);
            if block.len() == period {
                levels.push(rms(&block));
                block.clear();
            }
        }
        assert!((levels[99] / levels[2] - 1.0).abs() < 1e-3, "{levels:?}");

        for model in [EchoModel::Bbd, EchoModel::Tape] {
            for color in [-1.0, 0.0, 1.0] {
                let settings = EchoSettings {
                    model,
                    seconds: 0.1,
                    feedback: 1.0,
                    color,
                    diffusion: 0.5,
                    low_cut_hz: 60.0,
                    ..exact()
                };
                let mut kernel = prepared(&settings);
                let frames = 20 * 48_000;
                let mut peak = 0.0_f32;
                let mut tail = Vec::new();
                for n in 0..frames {
                    let input = if n < 4800 { sine(n, 330.0) } else { 0.0 };
                    let (left, right) = kernel.process(input, input);
                    assert!(left.is_finite() && right.is_finite());
                    peak = peak.max(left.abs());
                    if n >= frames - 48_000 {
                        tail.push(left);
                    }
                }
                let level = rms(&tail);
                // It neither dies away nor runs off.
                assert!(peak < 4.0, "{model:?} {color}: peak {peak}");
                assert!(level > 0.02, "{model:?} {color}: sank to {level}");
            }
        }
    }

    #[test]
    fn below_unity_every_model_decays_as_its_loop_gain_says() {
        for model in [EchoModel::Digital, EchoModel::Bbd, EchoModel::Tape] {
            let settings = EchoSettings {
                model,
                seconds: 0.1,
                feedback: 0.5,
                color: if model == EchoModel::Digital {
                    1.0
                } else {
                    0.5
                },
                low_cut_hz: 20.0,
                ..exact()
            };
            let mut kernel = prepared(&settings);
            let period = 4800;
            let mut levels = Vec::new();
            let mut block = Vec::with_capacity(period);
            for n in 0..8 * period {
                // A burst shorter than the time, so the repeats stay apart.
                let input = if n < 2400 {
                    (std::f32::consts::PI * n as f32 / 2400.0).sin().powi(2) * sine(n, 400.0)
                } else {
                    0.0
                };
                block.push(kernel.process(input, input).0);
                if block.len() == period {
                    levels.push(rms(&block));
                    block.clear();
                }
            }
            let response = repeat_response(&settings, RATE, 400.0);
            for pair in levels[1..6].windows(2) {
                let ratio = pair[1] / pair[0];
                let expected = 0.5 * response;
                // The compander and the tape's curve move a repeat's level
                // by a little.
                let tolerance = if model == EchoModel::Digital {
                    1e-3
                } else {
                    0.12
                };
                assert!(
                    (ratio / expected - 1.0).abs() < tolerance,
                    "{model:?}: {ratio} per repeat, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn changes_of_model_routing_pattern_and_time_do_not_click() {
        let base = EchoSettings {
            seconds: 0.05,
            feedback: 0.4,
            stereo: EchoStereo::Mono,
            color: 0.0,
            ..EchoSettings::default()
        };
        let changes: [(&str, EchoSettings, EchoSettings); 9] = [
            (
                "to BBD",
                base,
                EchoSettings {
                    model: EchoModel::Bbd,
                    ..base
                },
            ),
            (
                "to Tape",
                base,
                EchoSettings {
                    model: EchoModel::Tape,
                    ..base
                },
            ),
            (
                "BBD to Tape",
                EchoSettings {
                    model: EchoModel::Bbd,
                    ..base
                },
                EchoSettings {
                    model: EchoModel::Tape,
                    ..base
                },
            ),
            (
                "to Wide",
                base,
                EchoSettings {
                    stereo: EchoStereo::Wide,
                    ..base
                },
            ),
            (
                "to Ping-pong",
                base,
                EchoSettings {
                    stereo: EchoStereo::PingPong,
                    ..base
                },
            ),
            (
                "to Quad",
                base,
                EchoSettings {
                    taps: TapPattern::Quad,
                    ..base
                },
            ),
            (
                "Quad to Golden",
                EchoSettings {
                    taps: TapPattern::Quad,
                    ..base
                },
                EchoSettings {
                    taps: TapPattern::Golden,
                    ..base
                },
            ),
            (
                "time",
                base,
                EchoSettings {
                    seconds: 0.08,
                    ..base
                },
            ),
            (
                "diffused time",
                EchoSettings {
                    diffusion: 1.0,
                    ..base
                },
                EchoSettings {
                    diffusion: 1.0,
                    seconds: 0.021,
                    ..base
                },
            ),
        ];
        for (name, from, to) in changes {
            let mut kernel = prepared(&from);
            let mut out = Vec::new();
            for n in 0..48_000 {
                if n == 24_000 {
                    kernel.set(&to);
                }
                let input = sine(n, 110.0);
                let (left, right) = kernel.process(input, input);
                out.push(left + right);
            }
            let before = roughness(&out[12_000..24_000]);
            let after = roughness(&out[24_000..]);
            // A 110 Hz sine of 0.25 bends by 5e-5 a sample; a click of a
            // hundredth of full scale would be 200 times that.
            assert!(
                after < before + 2.5e-3,
                "{name}: roughness {after} after the change, {before} before"
            );
        }
    }

    #[test]
    fn the_bucket_clock_follows_its_law() {
        let clock = |seconds: f32, color: f32| {
            let settings = EchoSettings {
                model: EchoModel::Bbd,
                seconds,
                color,
                ..exact()
            };
            let named = bbd_clock_hz(&settings, RATE);
            let kernel = prepared(&settings);
            assert!((kernel.clock_hz() / named - 1.0).abs() < 1e-9);
            named
        };
        // 10 kHz at 300 ms, an octave either way with Color, slower for
        // longer lines but not in proportion.
        assert!((clock(0.3, 0.0) / 10_000.0 - 1.0).abs() < 0.01);
        assert!((clock(0.3, 1.0) / 20_000.0 - 1.0).abs() < 0.01);
        assert!((clock(0.3, -1.0) / 5000.0 - 1.0).abs() < 0.01);
        assert!((clock(1.2, 0.0) / 5000.0 - 1.0).abs() < 0.01);
        assert!((clock(4.0, -1.0) / 2000.0 - 1.0).abs() < 0.01);
        assert!(clock(0.001, 1.0) <= 96_000.0);
        // The repeats lose their top to the clock.
        let dark = EchoSettings {
            model: EchoModel::Bbd,
            seconds: 0.3,
            color: -1.0,
            ..exact()
        };
        let bright = EchoSettings { color: 1.0, ..dark };
        assert!(repeat_response(&dark, RATE, 4000.0) < 0.1);
        assert!(repeat_response(&bright, RATE, 4000.0) > 0.7);
    }

    #[test]
    fn a_bucket_line_keeps_its_time_bends_on_a_change_and_ends_in_silence() {
        let settings = EchoSettings {
            model: EchoModel::Bbd,
            seconds: 0.1,
            feedback: 0.0,
            color: 1.0,
            low_cut_hz: 20.0,
            ..exact()
        };
        let mut kernel = prepared(&settings);
        let mut out = Vec::new();
        for n in 0..120_000 {
            let input = if n < 480 {
                (std::f32::consts::PI * n as f32 / 480.0).sin().powi(2) * sine(n, 1000.0)
            } else {
                0.0
            };
            out.push(kernel.process(input, input).0);
        }
        let energy: f64 = out.iter().map(|s| f64::from(*s).powi(2)).sum();
        let centroid = out
            .iter()
            .enumerate()
            .map(|(n, s)| n as f64 * f64::from(*s).powi(2))
            .sum::<f64>()
            / energy;
        // The burst's middle is 240 samples in.
        assert!(
            (centroid - 240.0 - 4800.0).abs() < 24.0,
            "repeat at {centroid}"
        );
        // Silence in, exact silence out, once the line has emptied and its
        // 5 Hz blocker has rung down.
        assert!(out[96_000..].iter().all(|s| *s == 0.0));

        // Fast automation of the time, through handovers: finite, bounded,
        // and settled on the new clock in the end.
        let mut kernel = prepared(&EchoSettings {
            feedback: 0.6,
            ..settings
        });
        let mut peak = 0.0_f32;
        for n in 0..96_000 {
            if n % 480 == 0 && n < 72_000 {
                let seconds = 0.002 + 1.5 * (0.5 + 0.5 * (n as f32 * 1e-4).sin()).powi(3);
                kernel.set(&EchoSettings {
                    seconds,
                    feedback: 0.6,
                    ..settings
                });
            }
            let input = sine(n, 220.0);
            let (left, right) = kernel.process(input, input);
            assert!(left.is_finite() && right.is_finite());
            peak = peak.max(left.abs());
        }
        assert!(peak < 3.0, "peak {peak}");
        let last = 0.002 + 1.5 * (0.5 + 0.5 * (71_520.0_f32 * 1e-4).sin()).powi(3);
        for _ in 0..192_000 {
            kernel.process(0.0, 0.0);
        }
        let named = bbd_clock_hz(
            &EchoSettings {
                seconds: last,
                ..settings
            },
            RATE,
        );
        assert!(
            (kernel.clock_hz() / named - 1.0).abs() < 0.02,
            "{} against {named}",
            kernel.clock_hz()
        );
        assert!((kernel.seconds() / last - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_tape_time_change_bends_the_pitch_and_arrives() {
        let settings = EchoSettings {
            model: EchoModel::Tape,
            seconds: 0.2,
            feedback: 0.0,
            color: 1.0,
            ..exact()
        };
        let mut kernel = prepared(&settings);
        let mut out = Vec::new();
        for n in 0..96_000 {
            if n == 24_000 {
                kernel.set(&EchoSettings {
                    seconds: 0.1,
                    ..settings
                });
            }
            let input = sine(n, 1000.0);
            out.push(kernel.process(input, input).0);
        }
        // Rising zero crossings per 50 ms.
        let hz = |from: usize| {
            let window = &out[from..from + 2400];
            window
                .windows(2)
                .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
                .count() as f32
                * 20.0
        };
        assert!((hz(20_000) - 1000.0).abs() <= 20.0);
        // The delay shrinks while the motor speeds up: the pitch rises.
        let bent = hz(25_000);
        assert!(
            bent > 1100.0 && bent < 2000.0,
            "{bent} Hz while the motor catches up"
        );
        assert!((hz(90_000) - 1000.0).abs() <= 20.0);
        assert!((kernel.seconds() - 0.1).abs() < 1e-3);
        assert!(roughness(&out[24_000..]) < 0.05);
    }

    #[test]
    fn a_short_time_bends_without_a_dropout_or_a_click() {
        // Below 320 ms the diffusers' lengths follow the time.
        for model in [EchoModel::Tape, EchoModel::Bbd] {
            let settings = |seconds: f32| EchoSettings {
                model,
                seconds,
                feedback: 0.0,
                color: 1.0,
                ..exact()
            };
            for (from, to) in [(0.3, 0.33), (0.25, 0.24), (0.03, 0.036)] {
                let mut kernel = prepared(&settings(from));
                let mut out = Vec::new();
                for n in 0..144_000 {
                    if n == 48_000 {
                        kernel.set(&settings(to));
                    }
                    let input = sine(n, 1000.0);
                    out.push(kernel.process(input, input).0);
                }
                let before = rms(&out[38_400..48_000]);
                for (index, window) in out[48_000..].chunks(480).enumerate() {
                    let db = 20.0 * (rms(window) / before).log10();
                    assert!(
                        db.abs() < 1.0,
                        "{model:?} {from} to {to} s: {db:.1} dB {} ms after the step",
                        index * 10
                    );
                }
            }
            // A slow ramp, as a tempo change gives a synced time.
            for (from, to) in [(0.2, 0.26), (0.3, 0.36), (0.03, 0.036)] {
                let mut kernel = prepared(&settings(from));
                let mut out = Vec::new();
                for n in 0..240_000 {
                    if n >= 24_000 && n % 64 == 0 {
                        let t = ((n - 24_000) as f32 / 192_000.0).min(1.0);
                        kernel.set(&settings(from + t * (to - from)));
                    }
                    let input = sine(n, 1000.0);
                    out.push(kernel.process(input, input).0);
                }
                let steady = roughness(&out[19_200..24_000]);
                let moving = roughness(&out[24_000..]);
                assert!(
                    moving < 1.5 * steady,
                    "{model:?} {from} to {to} s: roughness {moving} against {steady} at rest"
                );
            }
        }
    }

    #[test]
    fn wobble_moves_the_pitch_of_every_model_and_rests_at_zero() {
        for model in [EchoModel::Digital, EchoModel::Bbd, EchoModel::Tape] {
            let settings = EchoSettings {
                model,
                seconds: 0.3,
                feedback: 0.0,
                wobble: 1.0,
                color: if model == EchoModel::Digital {
                    1.0
                } else {
                    0.5
                },
                ..exact()
            };
            let deviation = |settings: &EchoSettings| {
                let mut kernel = prepared(settings);
                let out: Vec<f32> = (0..192_000)
                    .map(|n| {
                        let input = sine(n, 2000.0);
                        kernel.process(input, input).0
                    })
                    .collect();
                // Periods between rising crossings, interpolated.
                let mut crossings = Vec::new();
                for n in 48_000..out.len() - 1 {
                    if out[n] < 0.0 && out[n + 1] >= 0.0 {
                        crossings.push(n as f32 + out[n] / (out[n] - out[n + 1]));
                    }
                }
                let periods: Vec<f32> = crossings.windows(9).map(|w| (w[8] - w[0]) / 8.0).collect();
                let (low, high) = periods.iter().fold((f32::MAX, f32::MIN), |(low, high), p| {
                    (low.min(*p), high.max(*p))
                });
                (high - low) / 24.0
            };
            let moved = deviation(&settings);
            let still = deviation(&EchoSettings {
                wobble: 0.0,
                ..settings
            });
            // Wow of 1.5 % peak: some per cent between the extremes.
            assert!((0.004..0.06).contains(&moved), "{model:?}: {moved}");
            assert!(still < 0.002, "{model:?}: {still} at rest");
        }
    }

    #[test]
    fn the_named_taps_and_tail_describe_the_echoes() {
        let settings = EchoSettings {
            seconds: 0.3,
            feedback: 0.5,
            stereo: EchoStereo::PingPong,
            taps: TapPattern::Dotted,
            ..exact()
        };
        let mut out = [(0.0, 0.0, 0.0); 64];
        let count = taps(&settings, &mut out);
        // Ten repeats above −60 dB, two heads each.
        assert_eq!(count, 20);
        assert_eq!(out[0], (0.75 * 0.3, 1.0, 0.0));
        assert_eq!(out[1], (0.3, 1.0, 0.0));
        assert_eq!(out[2], (1.75 * 0.3, 0.0, 0.5));
        let wide = EchoSettings {
            stereo: EchoStereo::Wide,
            taps: TapPattern::Single,
            ..settings
        };
        let count = taps(&wide, &mut out);
        assert!(out[..count].windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert_eq!(out[1], (0.3 + wide_offset(0.3), 0.0, 1.0));

        assert_eq!(tail_seconds(&settings), 3.0);
        assert_eq!(
            tail_seconds(&EchoSettings {
                feedback: 1.0,
                ..settings
            }),
            30.0
        );
        assert_eq!(
            tail_seconds(&EchoSettings {
                feedback: 0.0,
                ..settings
            }),
            0.3
        );
        assert!(
            tail_seconds(&EchoSettings {
                diffusion: 1.0,
                ..settings
            }) > 3.0
        );
        assert_eq!(loop_gain(EchoModel::Digital, 1.0), 1.0);
        assert!(loop_gain(EchoModel::Tape, 1.0) > 1.0);
        assert!((loop_gain(EchoModel::Bbd, 0.96) - 1.0).abs() < 0.01);
        assert_eq!(digital_cut_hz(1.0, RATE), None);
        assert_eq!(digital_cut_hz(0.0, RATE), Some(8000.0));
        assert_eq!(digital_cut_hz(-1.0, RATE), Some(1000.0));
        assert!(tape_loss_hz(0.3, 0.0) > tape_loss_hz(1.0, 0.0));
        assert!(tape_loss_hz(0.3, -1.0) < tape_loss_hz(0.3, 0.0));
    }
}
