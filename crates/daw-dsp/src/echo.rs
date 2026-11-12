//! A stereo feedback echo with three characters: a clean digital line, a
//! tape machine and a bucket-brigade line. The kernel gives the wet signal
//! alone, without latency.
use super::{
    AlgebraicSigmoid, AntialiasedShaper, Compander, CutoffGlide, DelayLine, FadeTap, Lfo, LfoShape,
    LinearRamp, Oversampler2x, Shelf1, Smoothed, Svf, SvfMix, flush, prewarp, soft_limit,
};
use crate::reverb::{CUT_OFF_HIGH_HZ, CUT_OFF_LOW_HZ};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoType {
    /// Every repeat is the input again, only quieter and filtered.
    Digital,
    /// Repeats saturate and lose their top; the speed wobbles; a time
    /// change bends the pitch.
    Tape,
    /// Darker repeats the longer the time, behind a compander.
    Analog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoStereo {
    Mono,
    /// Repeats alternate between left and right.
    PingPong,
    /// One line per channel, the right one slightly longer.
    Wide,
}

/// Plain-unit settings of an [`EchoKernel`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EchoSettings {
    pub kind: EchoType,
    /// Spacing of the repeats, up to [`MAX_SECONDS`].
    pub seconds: f32,
    /// 0–1; see [`loop_gain`].
    pub feedback: f32,
    pub stereo: EchoStereo,
    /// In the loop. Off at 20 Hz for Digital; Tape and Analog never pass DC.
    pub low_cut_hz: f32,
    /// In the loop. Off at 20 kHz, except for Analog, whose bandwidth
    /// follows the time.
    pub high_cut_hz: f32,
    /// 0–1: wow (and for Tape flutter and drift) of the delay time.
    pub wobble: f32,
}

impl Default for EchoSettings {
    fn default() -> Self {
        Self {
            kind: EchoType::Digital,
            seconds: 0.375,
            feedback: 0.35,
            stereo: EchoStereo::PingPong,
            low_cut_hz: 120.0,
            high_cut_hz: 8000.0,
            wobble: 0.0,
        }
    }
}

pub const MAX_SECONDS: f32 = 4.0;
/// Samples a signal takes around the loop of Tape and Analog besides the
/// line itself: the read-before-write order and the oversampled saturator.
/// The read position makes up for it. Digital's is 1.
pub const LOOP_DELAY: usize = 1 + Oversampler2x::LATENCY as usize + STAGE_DELAY;
/// The shaper's two samples at the doubled rate.
const STAGE_DELAY: usize = 1;

/// The longest line: the time, Wide's offset, the wobble's reach and the
/// interpolator's taps.
const LINE_SECONDS: f32 = 4.1;
const SHORTEST_SECONDS: f32 = 0.001;
const DIGITAL_CEILING: f32 = 4.0;
/// Loop gain at full feedback of Tape and Analog: they self-oscillate,
/// bounded by their saturation.
const OSCILLATING_GAIN: f32 = 1.06;
const OSCILLATION_FROM: f32 = 0.9;

const TIME_FADE_SECONDS: f32 = 0.04;
const TIME_GLIDE_SECONDS: f32 = 0.2;
/// Samples of delay per sample: the tape never stops or runs backward.
const FASTEST_GLIDE: f64 = 0.9;
const SLOWEST_GLIDE: f64 = 1.0 / 128.0;
const TYPE_DUCK_SECONDS: f32 = 0.01;
const ROUTING_SECONDS: f32 = 0.02;

const WOW_HZ: f32 = 0.5;
const WOW_SHARE: f32 = 0.015;
const FLUTTER_HZ: f32 = 6.5;
const FLUTTER_SHARE: f32 = 0.004;
const DRIFT_HZ: f32 = 1.3;
const DRIFT_SHARE: f32 = 0.0015;
const WOBBLE_TICK: u32 = 16;

const TAPE_EMPHASIS_HZ: (f32, f32) = (1500.0, 6000.0);
const ANALOG_BIAS: f64 = 0.08;
const BUCKETS: f32 = 4096.0;
const SHAPER_LIMIT: f64 = 1e4;
/// With the shaper's own droop this stays at or below unit gain at every
/// frequency, which a loop needs.
const DROOP_CORRECTION: f64 = 0.125;

/// The loop gain for a feedback of 0–1. Digital holds forever at 1; Tape
/// and Analog pass unity at 96 % and oscillate above.
pub fn loop_gain(kind: EchoType, feedback: f32) -> f32 {
    let feedback = feedback.clamp(0.0, 1.0);
    if kind == EchoType::Digital || feedback <= OSCILLATION_FROM {
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

/// The bandwidth of a bucket-brigade line follows its clock: the longer the
/// time, the darker.
fn bucket_bandwidth_hz(seconds: f32) -> f32 {
    (0.35 * BUCKETS / (2.0 * seconds)).clamp(2000.0, 20_000.0)
}

/// Tape loses more of its top at slow speeds.
fn tape_loss_hz(seconds: f32) -> f32 {
    (9000.0 * (0.3 / seconds).sqrt()).clamp(4000.0, 12_000.0)
}

fn high_cut_target(settings: &EchoSettings) -> f32 {
    if settings.kind == EchoType::Analog {
        settings
            .high_cut_hz
            .min(bucket_bandwidth_hz(settings.seconds))
    } else {
        settings.high_cut_hz
    }
}

fn low_cut_is_off(settings: &EchoSettings) -> bool {
    settings.kind == EchoType::Digital && settings.low_cut_hz <= CUT_OFF_LOW_HZ
}

fn high_cut_is_off(settings: &EchoSettings) -> bool {
    settings.kind != EchoType::Analog && settings.high_cut_hz >= CUT_OFF_HIGH_HZ
}

/// The first echoes as `(seconds, left gain, right gain)`, in order of
/// arrival, without the loop's filters. Returns how many of `out` are filled:
/// those above −60 dB.
pub fn taps(settings: &EchoSettings, out: &mut [(f32, f32, f32)]) -> usize {
    let gain = loop_gain(settings.kind, settings.feedback);
    let seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
    let later = seconds + wide_offset(seconds);
    let mut count = 0;
    let mut level = 1.0_f32;
    let mut repeat = 1;
    while count < out.len() && level >= 1e-3 {
        let time = repeat as f32 * seconds;
        match settings.stereo {
            EchoStereo::Mono => out[count] = (time, level, level),
            EchoStereo::PingPong if repeat % 2 == 1 => out[count] = (time, level, 0.0),
            EchoStereo::PingPong => out[count] = (time, 0.0, level),
            EchoStereo::Wide => {
                out[count] = (time, level, 0.0);
                if count + 1 < out.len() {
                    count += 1;
                    out[count] = (repeat as f32 * later, 0.0, level);
                }
            }
        }
        count += 1;
        repeat += 1;
        level *= gain;
    }
    if settings.stereo == EchoStereo::Wide {
        out[..count].sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    count
}

/// Magnitude of one pass through the loop at `hz` for small signals,
/// without the loop gain: what each repeat loses to the filters and the
/// character.
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
    if !high_cut_is_off(settings) {
        gain *= butterworth(high_cut_target(settings), false);
    }
    if settings.kind != EchoType::Digital {
        // The shaper's averaging and its corrector, at the doubled rate.
        let cos = (0.5 * omega).cos();
        gain *= (6.0 + 2.0 * cos) / 8.0 * (1.0 + 2.0 * DROOP_CORRECTION * (1.0 - cos));
    }
    let seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
    match settings.kind {
        EchoType::Digital => {}
        EchoType::Tape => {
            gain *= Shelf1::low_pass(tape_loss_hz(seconds), sample_rate)
                .response(omega)
                .norm();
        }
        EchoType::Analog => gain *= butterworth(bucket_bandwidth_hz(seconds), false),
    }
    gain
}

/// Seconds until the repeats have fallen by 60 dB, at most 30; 30 when they
/// never do.
pub fn tail_seconds(settings: &EchoSettings) -> f32 {
    let gain = loop_gain(settings.kind, settings.feedback);
    let mut seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
    if settings.stereo == EchoStereo::Wide {
        seconds += wide_offset(seconds);
    }
    if gain >= 1.0 {
        30.0
    } else if gain <= 0.0 {
        seconds
    } else {
        (seconds * (3.0 / -gain.log10()).ceil()).min(30.0)
    }
}

fn sigmoid(kind: EchoType) -> AlgebraicSigmoid {
    let bias = if kind == EchoType::Analog {
        ANALOG_BIAS
    } else {
        0.0
    };
    AlgebraicSigmoid::new(1.0, 1.0, bias)
}

#[derive(Clone, Debug)]
struct EchoLine {
    line: DelayLine,
    /// Digital: the time changes by crossfading.
    tap: FadeTap,
    /// Tape and Analog: the delay in samples glides, bending the pitch.
    delay: f64,
    target: f64,
    low_cut: Svf,
    high_cut: Svf,
    /// Analog: the line's reconstruction filter.
    post: Svf,
    emphasis: Shelf1,
    de_emphasis: Shelf1,
    loss: Shelf1,
    over: Oversampler2x,
    shaper: AntialiasedShaper<AlgebraicSigmoid>,
    compander: Compander,
    /// Offset of the read position in samples, and its change per sample.
    wobble: f32,
    wobble_step: f32,
}

impl EchoLine {
    fn new() -> Self {
        let mut shaper = AntialiasedShaper::new(sigmoid(EchoType::Tape));
        shaper.corrector.c = DROOP_CORRECTION;
        Self {
            line: DelayLine::default(),
            tap: FadeTap::default(),
            delay: 0.0,
            target: 0.0,
            low_cut: Svf::default(),
            high_cut: Svf::default(),
            post: Svf::default(),
            emphasis: Shelf1::default(),
            de_emphasis: Shelf1::default(),
            loss: Shelf1::default(),
            over: Oversampler2x::new(),
            shaper,
            compander: Compander::default(),
            wobble: 0.0,
            wobble_step: 0.0,
        }
    }

    /// Clears what a character keeps besides the line.
    fn reset_colour(&mut self, kind: EchoType) {
        self.post.reset();
        self.emphasis.reset();
        self.de_emphasis.reset();
        self.loss.reset();
        self.over.reset();
        self.shaper.set_shape(sigmoid(kind));
        self.shaper.reset();
        self.compander.reset();
        self.delay = self.target;
        self.tap.snap(self.target as usize - 1);
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

    #[inline]
    fn read(&mut self, kind: EchoType, glide: f64, post_g: f32) -> f32 {
        self.wobble += self.wobble_step;
        if kind == EchoType::Digital {
            return if self.wobble == 0.0 && self.wobble_step == 0.0 {
                self.tap.read(&self.line)
            } else {
                let whole = self.wobble.floor();
                self.tap
                    .read_offset(&self.line, whole as isize, self.wobble - whole)
            };
        }
        let distance = self.target - self.delay;
        if distance != 0.0 {
            // An exponential approach that ends in a slow constant slide,
            // so that it arrives.
            let step = (glide * distance).clamp(-FASTEST_GLIDE, FASTEST_GLIDE);
            self.delay = if distance.abs() <= SLOWEST_GLIDE {
                self.target
            } else if step.abs() < SLOWEST_GLIDE {
                self.delay + SLOWEST_GLIDE.copysign(distance)
            } else {
                self.delay + step
            };
        }
        let position = (self.delay - LOOP_DELAY as f64 + f64::from(self.wobble)).max(1.0);
        let whole = position.floor();
        let mut signal = self.line.read_at(whole as usize, (position - whole) as f32);
        if kind == EchoType::Analog {
            signal = self.compander.expand(signal);
            signal = self
                .post
                .process(signal, post_g, std::f32::consts::SQRT_2)
                .low;
        }
        signal
    }

    #[inline]
    fn write(&mut self, kind: EchoType, mut signal: f32, cuts: &Cuts) {
        // The filters run while they are off, so they come in on a signal
        // they already know.
        let cut = self
            .low_cut
            .process(signal, cuts.low_g, std::f32::consts::SQRT_2)
            .high;
        if !cuts.low_off {
            signal = cut;
        }
        let cut = self
            .high_cut
            .process(signal, cuts.high_g, std::f32::consts::SQRT_2)
            .low;
        if !cuts.high_off {
            signal = cut;
        }
        signal = match kind {
            EchoType::Digital => soft_limit(signal, DIGITAL_CEILING),
            EchoType::Tape => {
                // Highs meet the curve harder; the inverse shelf makes the
                // pair flat for small signals.
                let driven = self.emphasis.process(signal);
                let shaped = self.saturate(driven);
                let restored = self.de_emphasis.process(shaped);
                self.loss.process(restored)
            }
            EchoType::Analog => {
                let compressed = self.compander.compress(signal);
                self.saturate(compressed)
            }
        };
        self.line.write(flush(signal));
    }
}

struct Cuts {
    low_g: f32,
    high_g: f32,
    low_off: bool,
    high_off: bool,
}

#[derive(Clone, Debug)]
pub struct EchoKernel {
    sample_rate: f32,
    settings: EchoSettings,
    /// The character that runs; follows the settings at the bottom of a duck.
    kind: EchoType,
    pending: Option<EchoType>,
    duck: LinearRamp,
    lines: [EchoLine; 2],
    wow: Lfo,
    flutter: Lfo,
    drift: Lfo,
    wobble: Smoothed,
    wobble_tick: u32,
    /// Weights of the Mono, Ping-pong and Wide routings.
    routing: [LinearRamp; 3],
    low: CutoffGlide,
    high: CutoffGlide,
    gain: Smoothed,
    post_g: f32,
    glide: f64,
}

impl Default for EchoKernel {
    fn default() -> Self {
        let settings = EchoSettings::default();
        Self {
            sample_rate: 48_000.0,
            settings,
            kind: settings.kind,
            pending: None,
            duck: LinearRamp::new(1.0),
            lines: [EchoLine::new(), EchoLine::new()],
            wow: Lfo::new(0x00EC_0001),
            flutter: Lfo::new(0x00EC_0002),
            drift: Lfo::new(0x00EC_0003),
            wobble: Smoothed::new(settings.wobble),
            wobble_tick: 0,
            routing: [LinearRamp::new(0.0); 3],
            low: CutoffGlide::new(settings.low_cut_hz),
            high: CutoffGlide::new(settings.high_cut_hz),
            gain: Smoothed::new(settings.feedback),
            post_g: 1.0,
            glide: 1.0,
        }
    }
}

impl EchoKernel {
    /// Allocates for a sample rate and resets.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let fade = (TIME_FADE_SECONDS * sample_rate) as u32;
        for line in &mut self.lines {
            line.line.allocate((LINE_SECONDS * sample_rate) as usize);
            line.tap.set_fade(fade);
            line.compander.prepare(sample_rate);
            let (zero, pole) = TAPE_EMPHASIS_HZ;
            line.emphasis = Shelf1::shelf(zero, pole, sample_rate);
            line.de_emphasis = Shelf1::inverse_shelf(zero, pole, sample_rate);
        }
        self.glide = 1.0 - (-1.0 / f64::from(TIME_GLIDE_SECONDS * sample_rate)).exp();
        self.gain.set_time(20.0, sample_rate);
        self.wobble.set_time(50.0, sample_rate);
        self.low.prepare(20.0, sample_rate);
        self.high.prepare(20.0, sample_rate);
        self.reset();
    }

    /// New targets: the time crossfades (Digital) or glides, the routing
    /// crossfades, the character changes under a short duck.
    pub fn set(&mut self, settings: &EchoSettings) {
        self.settings = *settings;
        let rate = self.sample_rate;
        let seconds = settings.seconds.clamp(SHORTEST_SECONDS, MAX_SECONDS);
        let longest = self.lines[0].line.max_delay() - (0.02 * rate).ceil();
        for (index, line) in self.lines.iter_mut().enumerate() {
            let seconds = if index == 1 && settings.stereo == EchoStereo::Wide {
                seconds + wide_offset(seconds)
            } else {
                seconds
            };
            let delay = (seconds * rate).round().clamp(
                (LOOP_DELAY + 1) as f32,
                longest.max((LOOP_DELAY + 1) as f32),
            );
            line.target = f64::from(delay);
            line.tap.set(delay as usize - 1);
            line.loss
                .set(&Shelf1::low_pass(tape_loss_hz(seconds), rate));
        }
        self.post_g = prewarp(bucket_bandwidth_hz(seconds), rate);
        self.gain
            .set_target(loop_gain(settings.kind, settings.feedback));
        self.wobble.set_target(settings.wobble.clamp(0.0, 1.0));
        self.low.set(settings.low_cut_hz.max(CUT_OFF_LOW_HZ));
        self.high.set(high_cut_target(settings));
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
        let aim = if settings.kind == self.kind {
            self.pending = None;
            1.0
        } else {
            self.pending = Some(settings.kind);
            0.0
        };
        if self.duck.target() != aim {
            self.duck.set_target(aim, (TYPE_DUCK_SECONDS * rate) as u32);
        }
    }

    /// Clears the audio state and settles on the settings.
    pub fn reset(&mut self) {
        let settings = self.settings;
        self.set(&settings);
        self.kind = settings.kind;
        self.pending = None;
        self.duck = LinearRamp::new(1.0);
        for line in &mut self.lines {
            line.line.reset();
            line.low_cut.reset();
            line.high_cut.reset();
            line.reset_colour(settings.kind);
            line.wobble = 0.0;
            line.wobble_step = 0.0;
        }
        self.wow.reset();
        self.flutter.reset();
        self.drift.reset();
        self.wobble.snap();
        self.wobble_tick = 0;
        for weight in &mut self.routing {
            weight.snap();
        }
        self.low.snap();
        self.high.snap();
        self.gain.snap();
    }

    /// The repeat time of the left line now, in seconds: on the way to the
    /// set time while it glides.
    pub fn seconds(&self) -> f32 {
        let line = &self.lines[0];
        let samples = if self.kind == EchoType::Digital {
            line.target
        } else {
            line.delay
        };
        (samples / f64::from(self.sample_rate)) as f32
    }

    fn switch(&mut self, kind: EchoType) {
        // What a bucket-brigade line holds is compressed: it means nothing
        // to the other characters, nor theirs to it.
        let clear = kind == EchoType::Analog || self.kind == EchoType::Analog;
        self.kind = kind;
        for line in &mut self.lines {
            if clear {
                line.line.reset();
            }
            line.reset_colour(kind);
        }
    }

    /// The read position's offset in samples for a line: what a transport
    /// whose speed wobbles does to a delay of `seconds`.
    fn wobble_offset(&self, line: usize, seconds: f32, amount: f32) -> f32 {
        let later = 0.25 * line as f64;
        // A speed error of `share·sin` integrates over the time the tape
        // spends between the heads.
        let wander = |lfo: &Lfo, hz: f32, share: f32| {
            let now = 0.25 + later;
            let then = now - f64::from(hz * seconds);
            share / (std::f32::consts::TAU * hz)
                * (lfo.value(LfoShape::Sine, now) - lfo.value(LfoShape::Sine, then))
        };
        let mut offset = wander(&self.wow, WOW_HZ, WOW_SHARE);
        if self.kind == EchoType::Tape {
            offset += wander(&self.flutter, FLUTTER_HZ, FLUTTER_SHARE)
                - seconds * DRIFT_SHARE * self.drift.value(LfoShape::Smooth, later);
        }
        amount * offset * self.sample_rate
    }

    /// One sample in, the echoes out.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let mut duck = self.duck.next();
        if duck == 0.0
            && let Some(kind) = self.pending.take()
        {
            self.switch(kind);
            self.duck
                .set_target(1.0, (TYPE_DUCK_SECONDS * self.sample_rate) as u32);
            duck = 0.0;
        }
        let kind = self.kind;
        let amount = self.wobble.next_value();
        if self.wobble_tick == 0 {
            let step = f64::from(WOBBLE_TICK as f32 / self.sample_rate);
            self.wow.advance(f64::from(WOW_HZ) * step);
            self.flutter.advance(f64::from(FLUTTER_HZ) * step);
            self.drift.advance(f64::from(DRIFT_HZ) * step);
            for index in 0..2 {
                let seconds = (self.lines[index].target / f64::from(self.sample_rate)) as f32;
                let aim = if amount > 0.0 {
                    self.wobble_offset(index, seconds, amount)
                } else {
                    0.0
                };
                let line = &mut self.lines[index];
                line.wobble_step = (aim - line.wobble) / WOBBLE_TICK as f32;
                if amount == 0.0 && line.wobble_step.abs() < 1e-9 {
                    // At rest the read is a whole tap again.
                    line.wobble = 0.0;
                    line.wobble_step = 0.0;
                }
            }
        }
        self.wobble_tick = (self.wobble_tick + 1) % WOBBLE_TICK;

        let gain = self.gain.next_value();
        let [mono, ping, wide] = self.routing.each_mut().map(LinearRamp::next);
        let cuts = Cuts {
            low_g: self.low.next_g(),
            high_g: self.high.next_g(),
            low_off: low_cut_is_off(&self.settings)
                && kind == EchoType::Digital
                && self.low.is_settled(),
            high_off: high_cut_is_off(&self.settings)
                && kind != EchoType::Analog
                && self.high.is_settled(),
        };
        // Ducked where it is read and where it is written: nothing that
        // jumps at a change of character is heard or stored.
        let [a, b] = self
            .lines
            .each_mut()
            .map(|line| duck * line.read(kind, self.glide, self.post_g));
        let center = 0.5 * (left + right);
        let into_a =
            duck * ((mono + ping) * center + wide * left) + gain * ((mono + wide) * a + ping * b);
        let into_b = duck * wide * right + gain * (ping * a + wide * b);
        let [first, second] = &mut self.lines;
        first.write(kind, into_a, &cuts);
        second.write(kind, into_b, &cuts);
        (a, mono * a + (ping + wide) * b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Random;

    const RATE: f32 = 48_000.0;

    fn kernel(settings: &EchoSettings) -> EchoKernel {
        let mut kernel = EchoKernel::default();
        kernel.prepare(RATE);
        kernel.set(settings);
        kernel.reset();
        kernel
    }

    fn open(kind: EchoType, seconds: f32) -> EchoSettings {
        EchoSettings {
            kind,
            seconds,
            low_cut_hz: CUT_OFF_LOW_HZ,
            high_cut_hz: CUT_OFF_HIGH_HZ,
            ..EchoSettings::default()
        }
    }

    fn impulse_response(kernel: &mut EchoKernel, frames: usize) -> Vec<(f32, f32)> {
        (0..frames)
            .map(|n| kernel.process(if n == 0 { 1.0 } else { 0.0 }, 0.0))
            .collect()
    }

    #[test]
    fn digital_repeats_are_exact_and_alternate_in_ping_pong() {
        let settings = EchoSettings {
            feedback: 0.5,
            ..open(EchoType::Digital, 0.01)
        };
        let response = impulse_response(&mut kernel(&settings), 2400);
        for (n, (left, right)) in response.iter().enumerate() {
            // The impulse is on the left only: half of it enters the line.
            let expected = match n {
                480 => (0.5, 0.0),
                960 => (0.0, 0.25),
                1440 => (0.125, 0.0),
                1920 => (0.0, 0.0625),
                _ => (0.0, 0.0),
            };
            assert_eq!((*left, *right), expected, "frame {n}");
        }
        let mono = EchoSettings {
            stereo: EchoStereo::Mono,
            ..settings
        };
        let response = impulse_response(&mut kernel(&mono), 1000);
        assert_eq!(response[480], (0.5, 0.5));
        assert_eq!(response[960], (0.25, 0.25));
        let wide = EchoSettings {
            stereo: EchoStereo::Wide,
            ..settings
        };
        let mut kernel = kernel(&wide);
        let response: Vec<_> = (0..1000)
            .map(|n| kernel.process(0.0, if n == 0 { 1.0 } else { 0.0 }))
            .collect();
        // 3 ms later on the right, and the left stays silent.
        assert_eq!(response[480 + 144], (0.0, 1.0));
        assert!(response.iter().all(|(left, _)| *left == 0.0));
    }

    #[test]
    fn full_feedback_holds_in_digital_and_stays_bounded_under_input() {
        let settings = EchoSettings {
            feedback: 1.0,
            stereo: EchoStereo::Mono,
            ..open(EchoType::Digital, 0.005)
        };
        let mut kernel = kernel(&settings);
        let response = impulse_response(&mut kernel, 48_000);
        assert_eq!(response[240].0, 0.5);
        assert_eq!(response[47_760].0, 0.5);
        let mut random = Random::new(6);
        let mut peak = 0.0_f32;
        for _ in 0..200_000 {
            let x = random.bipolar();
            let (left, right) = kernel.process(x, x);
            peak = peak.max(left.abs()).max(right.abs());
        }
        assert!(peak > 1.0 && peak <= DIGITAL_CEILING, "{peak}");
    }

    #[test]
    fn tape_and_analog_repeat_on_time_despite_the_stage_inside_the_loop() {
        // Analog only at times short enough for its filters to be open: at
        // 350 ms they alone delay the peak by ten samples.
        let cases = [
            (EchoType::Tape, 0.001),
            (EchoType::Tape, 0.35),
            (EchoType::Analog, 0.001),
            (EchoType::Analog, 0.03),
        ];
        {
            for (kind, seconds) in cases {
                let settings = EchoSettings {
                    feedback: 0.0,
                    stereo: EchoStereo::Mono,
                    ..open(kind, seconds)
                };
                // A compander makes little of a lone impulse; noise tells
                // the delay for both.
                let mut kernel = kernel(&settings);
                let mut random = Random::new(31);
                let input: Vec<f32> = (0..40_000).map(|_| 0.05 * random.bipolar()).collect();
                let output: Vec<f32> = input.iter().map(|x| kernel.process(*x, *x).0).collect();
                let correlation = |lag: usize| -> f64 {
                    (20_000..40_000)
                        .map(|n| f64::from(output[n]) * f64::from(input[n - lag]))
                        .sum()
                };
                let expected = (seconds * RATE).round() as usize;
                let peak = (expected - 20..expected + 20)
                    .max_by(|a, b| correlation(*a).total_cmp(&correlation(*b)))
                    .unwrap();
                assert!(peak.abs_diff(expected) <= 1, "{kind:?} {seconds}: {peak}");
            }
        }
    }

    /// Level of the first echo of a sine against the sine, by quadrature.
    fn first_echo_gain(settings: &EchoSettings, hz: f64, amplitude: f64) -> f64 {
        let mut kernel = kernel(&EchoSettings {
            feedback: 0.0,
            stereo: EchoStereo::Mono,
            ..*settings
        });
        const WINDOW: usize = 9600;
        let (mut sine, mut cosine) = (0.0, 0.0);
        for n in 0..48_000 {
            let phase = std::f64::consts::TAU * hz * n as f64 / f64::from(RATE);
            let x = (amplitude * phase.sin()) as f32;
            let (y, _) = kernel.process(x, x);
            if n >= 48_000 - WINDOW {
                sine += f64::from(y) * phase.sin();
                cosine += f64::from(y) * phase.cos();
            }
        }
        2.0 * sine.hypot(cosine) / (WINDOW as f64 * amplitude)
    }

    #[test]
    fn small_signals_see_only_the_loop_filters() {
        let cases = [
            open(EchoType::Digital, 0.05),
            EchoSettings::default(),
            open(EchoType::Tape, 0.05),
            EchoSettings {
                kind: EchoType::Tape,
                ..EchoSettings::default()
            },
            open(EchoType::Analog, 0.05),
            EchoSettings {
                kind: EchoType::Analog,
                seconds: 0.3,
                ..EchoSettings::default()
            },
        ];
        for settings in cases {
            for hz in [50.0, 200.0, 1000.0, 3000.0, 5000.0] {
                let measured = first_echo_gain(&settings, hz, 0.01);
                let expected = repeat_response(&settings, RATE, hz);
                let error_db = 20.0 * (measured / expected).log10();
                assert!(
                    error_db.abs() < 0.5,
                    "{settings:?} at {hz} Hz: {error_db} dB"
                );
            }
        }
        // Passive at every frequency: the loop gain is the feedback.
        for kind in [EchoType::Digital, EchoType::Tape, EchoType::Analog] {
            for step in 1..240 {
                let hz = 100.0 * f64::from(step);
                assert!(
                    repeat_response(&open(kind, 0.02), RATE, hz) <= 1.0,
                    "{kind:?} {hz}"
                );
            }
        }
    }

    #[test]
    fn full_feedback_oscillates_bounded_in_tape_and_analog() {
        for kind in [EchoType::Tape, EchoType::Analog] {
            let settings = EchoSettings {
                kind,
                feedback: 1.0,
                seconds: 0.05,
                ..EchoSettings::default()
            };
            assert!((loop_gain(kind, 1.0) - OSCILLATING_GAIN).abs() < 1e-6);
            let mut kernel = kernel(&settings);
            let mut peak = 0.0_f32;
            let mut late = 0.0_f32;
            for n in 0..480_000 {
                let x = if n < 4800 {
                    0.5 * (std::f32::consts::TAU * 440.0 * n as f32 / RATE).sin()
                } else {
                    0.0
                };
                let (left, right) = kernel.process(x, x);
                peak = peak.max(left.abs()).max(right.abs());
                if n > 432_000 {
                    late = late.max(left.abs()).max(right.abs());
                }
            }
            assert!(peak < 2.0, "{kind:?}: peak {peak}");
            assert!(late > 0.05, "{kind:?}: died to {late}");
        }
    }

    #[test]
    fn taps_list_the_repeats_of_each_routing() {
        let mut out = [(0.0, 0.0, 0.0); 8];
        let settings = EchoSettings {
            feedback: 0.5,
            seconds: 0.1,
            ..EchoSettings::default()
        };
        assert_eq!(taps(&settings, &mut out), 8);
        assert_eq!(out[0], (0.1, 1.0, 0.0));
        assert_eq!(out[1], (0.2, 0.0, 0.5));
        assert_eq!(out[2].1, 0.25);
        let quiet = EchoSettings {
            feedback: 0.01,
            stereo: EchoStereo::Mono,
            ..settings
        };
        assert_eq!(taps(&quiet, &mut out), 2);
        assert_eq!(out[1], (0.2, 0.01, 0.01));
        let wide = EchoSettings {
            stereo: EchoStereo::Wide,
            ..settings
        };
        assert_eq!(taps(&wide, &mut out), 8);
        assert_eq!(out[0], (0.1, 1.0, 0.0));
        assert!((out[1].0 - 0.103).abs() < 1e-6 && out[1].2 == 1.0);
        assert_eq!(wide_offset(0.025), 0.003);
        assert_eq!(wide_offset(4.0), 0.015);
        assert_eq!(
            tail_seconds(&EchoSettings {
                feedback: 1.0,
                ..settings
            }),
            30.0
        );
        assert!((tail_seconds(&settings) - 1.0).abs() < 1e-5);
        assert!((tail_seconds(&wide) - 1.03).abs() < 1e-5);
    }

    #[test]
    fn character_and_routing_changes_do_not_click() {
        let settings = EchoSettings {
            seconds: 0.03,
            feedback: 0.4,
            ..EchoSettings::default()
        };
        let mut kernel = kernel(&settings);
        let mut n = 0;
        let mut largest_step = |kernel: &mut EchoKernel, frames: usize| {
            let mut previous: Option<(f32, f32)> = None;
            let mut largest = 0.0_f32;
            for _ in 0..frames {
                let x = 0.5 * (std::f32::consts::TAU * 220.0 * n as f32 / RATE).sin();
                n += 1;
                let now = kernel.process(x, x);
                if let Some(previous) = previous {
                    largest = largest
                        .max((now.0 - previous.0).abs())
                        .max((now.1 - previous.1).abs());
                }
                previous = Some(now);
            }
            largest
        };
        largest_step(&mut kernel, 24_000);
        let steady = largest_step(&mut kernel, 24_000);
        let changes = [
            EchoSettings {
                kind: EchoType::Tape,
                ..settings
            },
            EchoSettings {
                kind: EchoType::Analog,
                ..settings
            },
            EchoSettings {
                kind: EchoType::Analog,
                stereo: EchoStereo::Wide,
                ..settings
            },
            EchoSettings {
                stereo: EchoStereo::Mono,
                ..settings
            },
            EchoSettings {
                stereo: EchoStereo::Mono,
                seconds: 0.05,
                ..settings
            },
            EchoSettings {
                stereo: EchoStereo::Mono,
                seconds: 0.05,
                wobble: 1.0,
                ..settings
            },
            settings,
        ];
        for change in changes {
            kernel.set(&change);
            let step = largest_step(&mut kernel, 24_000);
            assert!(
                step <= 1.5 * steady + 0.02,
                "{change:?}: {step} against {steady}"
            );
        }
    }

    #[test]
    fn a_tape_time_change_bends_the_pitch_within_bounds() {
        let settings = EchoSettings {
            kind: EchoType::Tape,
            feedback: 0.0,
            stereo: EchoStereo::Mono,
            ..open(EchoType::Tape, 0.1)
        };
        let mut kernel = kernel(&settings);
        kernel.set(&EchoSettings {
            seconds: 0.4,
            ..settings
        });
        let mut previous = kernel.lines[0].delay;
        for _ in 0..96_000 {
            kernel.process(0.0, 0.0);
            let delay = kernel.lines[0].delay;
            // Playback speed is 1 minus the delay's change per sample.
            assert!((delay - previous).abs() <= FASTEST_GLIDE);
            previous = delay;
        }
        assert_eq!(kernel.lines[0].delay, 19_200.0);
        assert_eq!(kernel.seconds(), 0.4);
    }
}
