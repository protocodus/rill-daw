use super::{Complex, flush, oversample::bessel_i0};

/// Power-of-two ring buffer with third-order Lagrange interpolation, for
/// modulated delays.
#[derive(Clone, Debug, Default)]
pub struct DelayLine {
    buffer: Vec<f32>,
    mask: usize,
    cursor: usize,
}

impl DelayLine {
    /// Allocates room for `max_delay` samples. Call from `prepare` only.
    pub fn allocate(&mut self, max_delay: usize) {
        let size = (max_delay + 4).next_power_of_two();
        self.buffer = vec![0.0; size];
        self.mask = size - 1;
        self.cursor = 0;
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
    }

    pub fn max_delay(&self) -> f32 {
        self.buffer.len().saturating_sub(4) as f32
    }

    #[inline]
    pub fn write(&mut self, sample: f32) {
        if self.buffer.is_empty() {
            return;
        }
        self.cursor = (self.cursor + 1) & self.mask;
        self.buffer[self.cursor] = sample;
    }

    /// The sample written `samples` writes ago, exactly; 0 is the last one.
    /// For lookahead and aligned dry paths.
    #[inline]
    pub fn tap(&self, samples: usize) -> f32 {
        if self.buffer.is_empty() {
            return 0.0;
        }
        self.buffer[self.cursor.wrapping_sub(samples.min(self.mask)) & self.mask]
    }

    /// Reads `delay` samples behind the last write (at least 1).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let delay = delay.clamp(1.0, self.max_delay().max(1.0));
        let whole = delay.floor();
        self.read_at(whole as usize, delay - whole)
    }

    /// [`Self::read`] with the whole (at least 1) and fractional (0 up to 1)
    /// part of the delay apart: an `f32` cannot hold both for a long line.
    /// Without a fraction it is [`Self::tap`], exactly.
    #[inline]
    pub fn read_at(&self, whole: usize, frac: f32) -> f32 {
        if self.buffer.is_empty() {
            return 0.0;
        }
        let whole = whole.clamp(1, self.buffer.len() - 4);
        if frac == 0.0 {
            return self.buffer[self.cursor.wrapping_sub(whole) & self.mask];
        }
        // Taps at whole-1 … whole+2 put the fraction between the middle two.
        let base = self.cursor.wrapping_sub(whole - 1);
        let at = |offset: usize| self.buffer[base.wrapping_sub(offset) & self.mask];
        let [c0, c1, c2, c3] = lagrange_weights(frac);
        c0 * at(0) + c1 * at(1) + c2 * at(2) + c3 * at(3)
    }

    /// Reads `whole + frac` samples behind the last write (`whole` at least
    /// [`SincTable::REACH`], `frac` 0 up to 1) through a windowed sinc: flat
    /// far higher up than [`Self::read_at`], for a read that moves all the
    /// time.
    #[inline]
    pub fn read_sinc(&self, whole: usize, frac: f32, table: &SincTable) -> f32 {
        if self.buffer.len() < 2 * SincTable::TAPS {
            return 0.0;
        }
        let whole = whole.clamp(SincTable::REACH, self.buffer.len() - SincTable::TAPS);
        let place = frac.clamp(0.0, 1.0) * SincTable::PHASES as f32;
        let phase = (place as usize).min(SincTable::PHASES - 1);
        let t = place - phase as f32;
        let (near, far) = (&table.rows[phase], &table.rows[phase + 1]);
        let base = self.cursor.wrapping_sub(whole - SincTable::REACH);
        let mut sum = 0.0;
        for tap in 0..SincTable::TAPS {
            let weight = near[tap] + t * (far[tap] - near[tap]);
            sum += weight * self.buffer[base.wrapping_sub(tap) & self.mask];
        }
        sum
    }

    /// Response of the interpolation at a fraction, relative to the whole
    /// part of the delay, at `omega` radians per sample: what
    /// [`Self::read_at`] does to a signal beyond delaying it.
    pub fn lagrange_response(frac: f32, omega: f64) -> Complex {
        lagrange_weights(frac).iter().enumerate().fold(
            Complex::new(0.0, 0.0),
            |sum, (tap, weight)| {
                sum + Complex::unit(-omega * (tap as f64 - 1.0)).scale(f64::from(*weight))
            },
        )
    }
}

/// Third-order Lagrange weights of the taps one before the whole delay to
/// two after it.
#[inline]
fn lagrange_weights(frac: f32) -> [f32; 4] {
    let x = 1.0 + frac;
    [
        -(x - 1.0) * (x - 2.0) * (x - 3.0) / 6.0,
        x * (x - 2.0) * (x - 3.0) / 2.0,
        -x * (x - 1.0) * (x - 3.0) / 2.0,
        x * (x - 1.0) * (x - 2.0) / 6.0,
    ]
}

/// The weights of [`DelayLine::read_sinc`]: a Kaiser-windowed sinc of
/// sixteen taps at 64 fractions of a sample, blended linearly between.
#[derive(Clone, Debug)]
pub struct SincTable {
    rows: Box<[[f32; Self::TAPS]; Self::PHASES + 1]>,
}

impl SincTable {
    pub const TAPS: usize = 16;
    /// Taps before the whole part of the delay: the shortest delay a read
    /// can have.
    pub const REACH: usize = 7;
    const PHASES: usize = 64;
    const BETA: f64 = 6.0;
    /// What [`Self::passive`] gives up to stay at or below unit gain.
    pub const PASSIVE_GAIN: f64 = 0.998;

    /// `cutoff` is the share of the band that passes (1 is all of it; less
    /// for a read that runs faster than the line was written). Every
    /// fraction has unit gain at DC.
    pub fn new(cutoff: f64) -> Self {
        Self::with_gain(cutoff, 1.0)
    }

    /// [`Self::new`] turned down by a sixtieth of a decibel: the window's
    /// ripple then never exceeds unit gain at any frequency or fraction,
    /// which a read inside a feedback loop needs.
    pub fn passive(cutoff: f64) -> Self {
        Self::with_gain(cutoff, Self::PASSIVE_GAIN)
    }

    fn with_gain(cutoff: f64, gain: f64) -> Self {
        let mut rows = Box::new([[0.0; Self::TAPS]; Self::PHASES + 1]);
        let half = (Self::TAPS / 2) as f64;
        for (phase, row) in rows.iter_mut().enumerate() {
            let fraction = phase as f64 / Self::PHASES as f64;
            let mut weights = [0.0_f64; Self::TAPS];
            for (tap, weight) in weights.iter_mut().enumerate() {
                let x = tap as f64 - Self::REACH as f64 - fraction;
                let angle = std::f64::consts::PI * cutoff * x;
                let sinc = if angle.abs() < 1e-9 {
                    1.0
                } else {
                    angle.sin() / angle
                };
                let ratio = x / half;
                let window = bessel_i0(Self::BETA * (1.0 - ratio * ratio).max(0.0).sqrt())
                    / bessel_i0(Self::BETA);
                *weight = sinc * window;
            }
            let sum: f64 = weights.iter().sum();
            for (slot, weight) in row.iter_mut().zip(weights) {
                *slot = (weight * gain / sum) as f32;
            }
        }
        Self { rows }
    }

    /// Response of a read at a fraction, relative to the whole part of the
    /// delay, at `omega` radians per sample.
    pub fn response(&self, frac: f32, omega: f64) -> Complex {
        let place = frac.clamp(0.0, 1.0) * Self::PHASES as f32;
        let phase = (place as usize).min(Self::PHASES - 1);
        let t = place - phase as f32;
        (0..Self::TAPS).fold(Complex::new(0.0, 0.0), |sum, tap| {
            let weight =
                self.rows[phase][tap] + t * (self.rows[phase + 1][tap] - self.rows[phase][tap]);
            sum + Complex::unit(-omega * (tap as f64 - Self::REACH as f64)).scale(f64::from(weight))
        })
    }
}

/// A fractional read through a first-order allpass (Thiran): unit gain at
/// every frequency, so a slowly modulated delay inside a feedback loop
/// costs no decay, which an interpolating read does. Keeps one sample of
/// state; for delays that move by much less than a sample per sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllpassInterp {
    last: f32,
}

impl AllpassInterp {
    pub fn reset(&mut self) {
        self.last = 0.0;
    }

    /// Tells the filter what was read by other means, so that it takes
    /// over without a step.
    #[inline]
    pub fn follow(&mut self, output: f32) {
        self.last = output;
    }

    /// The line `tap + frac` samples behind its last write (`tap` at least
    /// 1, `frac` 0 up to 1).
    #[inline]
    pub fn read(&mut self, line: &DelayLine, tap: usize, frac: f32) -> f32 {
        // The filter's own delay is held between half a sample and one and
        // a half, where its coefficient is small and its phase most linear.
        let (whole, delay) = if frac >= 0.5 {
            (tap, frac)
        } else {
            (tap.max(1) - 1, frac + 1.0)
        };
        let a = (1.0 - delay) / (1.0 + delay);
        let output = a * (line.tap(whole) - self.last) + line.tap(whole + 1);
        self.last = flush(output);
        output
    }
}

/// One step of a delay length on its way to `target`, in samples: an
/// exponential approach with the coefficient `glide`, never faster than
/// `fastest` samples per sample (a delay that shrinks by one sample per
/// sample stands still; faster, it runs backward) and ending in a slow
/// constant slide, so that it arrives.
#[inline]
pub fn glide_length(length: f64, target: f64, glide: f64, fastest: f64) -> f64 {
    const SLOWEST: f64 = 1.0 / 256.0;
    let distance = target - length;
    let step = (glide * distance).clamp(-fastest, fastest);
    if distance.abs() <= SLOWEST {
        target
    } else if step.abs() < SLOWEST {
        length + SLOWEST.copysign(distance)
    } else {
        length + step
    }
}

/// A read position on a [`DelayLine`] that moves by crossfading two whole
/// taps: a delay time that changes without pitch and without a click. The
/// fade is linear, which never exceeds the larger tap, so it is safe inside
/// a feedback loop.
#[derive(Clone, Copy, Debug, Default)]
pub struct FadeTap {
    from: usize,
    to: usize,
    queued: Option<usize>,
    step: u32,
    length: u32,
}

impl FadeTap {
    pub fn set_fade(&mut self, samples: u32) {
        self.length = samples;
    }

    /// The delay the tap is at or fading to.
    pub fn target(&self) -> usize {
        self.queued.unwrap_or(self.to)
    }

    /// Moves to `delay`; a move that arrives during a fade waits for its
    /// end, and the latest one wins.
    pub fn set(&mut self, delay: usize) {
        if delay == self.to {
            self.queued = None;
        } else if self.from == self.to {
            self.to = delay;
            self.step = 0;
        } else {
            self.queued = Some(delay);
        }
    }

    pub fn snap(&mut self, delay: usize) {
        *self = Self {
            from: delay,
            to: delay,
            queued: None,
            step: 0,
            length: self.length,
        };
    }

    pub fn is_fading(&self) -> bool {
        self.from != self.to
    }

    /// Where the read is in samples: on the way between the two taps of a
    /// fade, by the share each has of it.
    pub fn place(&self) -> f32 {
        if self.from == self.to {
            return self.to as f32;
        }
        let t = self.step as f32 / self.length.max(1) as f32;
        self.from as f32 + t * (self.to as f32 - self.from as f32)
    }

    /// Share of the new tap for this sample; ends and chains the fade.
    #[inline]
    fn advance(&mut self) -> f32 {
        if self.from == self.to {
            return 1.0;
        }
        self.step += 1;
        if self.step >= self.length {
            self.from = self.to;
            self.step = 0;
            if let Some(next) = self.queued.take() {
                self.to = next;
            }
            return 1.0;
        }
        self.step as f32 / self.length as f32
    }

    #[inline]
    pub fn read(&mut self, line: &DelayLine) -> f32 {
        self.read_with(|delay| line.tap(delay))
    }

    /// [`Self::read`] through any way of reading the line at a delay.
    #[inline]
    pub fn read_with(&mut self, at: impl Fn(usize) -> f32) -> f32 {
        let (from, to) = (self.from, self.to);
        let t = self.advance();
        if t == 1.0 {
            at(to)
        } else {
            let old = at(from);
            old + t * (at(to) - old)
        }
    }

    /// [`Self::read`] with both heads moved by the same modulation, in whole
    /// samples and a fraction (0 up to 1), interpolated.
    #[inline]
    pub fn read_offset(&mut self, line: &DelayLine, whole: isize, frac: f32) -> f32 {
        self.read_with(|delay| line.read_at((delay as isize + whole).max(1) as usize, frac))
    }
}

/// Fixed delay of up to `N` samples without a heap: a dry path beside a
/// process with a known latency.
#[derive(Clone, Debug)]
pub struct StaticDelay<const N: usize> {
    buffer: [f32; N],
    cursor: usize,
}

impl<const N: usize> Default for StaticDelay<N> {
    fn default() -> Self {
        Self {
            buffer: [0.0; N],
            cursor: 0,
        }
    }
}

impl<const N: usize> StaticDelay<N> {
    /// The input of `N` calls ago.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.process_at(input, N)
    }

    /// The input of `delay` calls ago, at most `N`; 0 is `input` itself.
    #[inline]
    pub fn process_at(&mut self, input: f32, delay: usize) -> f32 {
        if N == 0 {
            return input;
        }
        let output = if delay == 0 {
            input
        } else {
            self.buffer[(self.cursor + N - delay.min(N)) % N]
        };
        self.buffer[self.cursor] = input;
        self.cursor += 1;
        if self.cursor == N {
            self.cursor = 0;
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taps_are_exact_from_zero() {
        let mut line = DelayLine::default();
        assert_eq!(line.tap(3), 0.0);
        line.allocate(100);
        for n in 0..300 {
            line.write(n as f32);
        }
        assert_eq!(line.tap(0), 299.0);
        assert_eq!(line.tap(1), 298.0);
        assert_eq!(line.tap(100), 199.0);
    }

    #[test]
    fn whole_reads_are_taps_and_long_delays_keep_their_fraction() {
        let mut line = DelayLine::default();
        line.allocate(500_000);
        let mut random = crate::Random::new(5);
        for _ in 0..2000 {
            line.write(random.bipolar());
        }
        for n in [1, 2, 77, 1500] {
            assert_eq!(line.read_at(n, 0.0).to_bits(), line.tap(n).to_bits());
            assert_eq!(line.read(n as f32).to_bits(), line.tap(n).to_bits());
        }
        // A ramp of slope 1e-3 per sample, read 400 000.37 samples back.
        for n in 0..600_000 {
            line.write(n as f32 * 1e-3);
        }
        let expected = (599_999.0 - 400_000.37) * 1e-3;
        assert!((line.read_at(400_000, 0.37) - expected).abs() < 1e-3);
    }

    #[test]
    fn interpolation_is_passive_and_matches_its_closed_form() {
        for step in 0..512 {
            let omega = std::f64::consts::PI * step as f64 / 511.0;
            let half = DelayLine::lagrange_response(0.5, omega).norm();
            let expected = 1.125 * (0.5 * omega).cos() - 0.125 * (1.5 * omega).cos();
            assert!((half - expected.abs()).abs() < 1e-6, "{omega}");
            // Feedback loops around a read rely on this bound.
            for fraction in 0..=64 {
                let gain = DelayLine::lagrange_response(fraction as f32 / 64.0, omega).norm();
                assert!(gain <= 1.0 + 1e-6, "{fraction}/64 at {omega}: {gain}");
            }
        }
        // The response is what the read does to a sine.
        let (omega, frac) = (0.9_f64, 0.3_f32);
        let mut line = DelayLine::default();
        line.allocate(64);
        for n in 0..200 {
            line.write((omega * f64::from(n)).sin() as f32);
        }
        let response = DelayLine::lagrange_response(frac, omega);
        let phase = omega * (199.0 - 10.0) + response.im.atan2(response.re);
        let expected = response.norm() * phase.sin();
        assert!((f64::from(line.read_at(10, frac)) - expected).abs() < 1e-5);
    }

    #[test]
    fn fade_tap_crossfades_linearly_and_queues_the_latest_move() {
        let mut line = DelayLine::default();
        line.allocate(64);
        // Constant content per tap makes the blend readable: tap n holds n.
        let fill = |line: &mut DelayLine| {
            for n in (0..=40).rev() {
                line.write(n as f32);
            }
        };
        fill(&mut line);
        let mut tap = FadeTap::default();
        tap.set_fade(4);
        tap.snap(10);
        assert_eq!(tap.read(&line), 10.0);
        tap.set(20);
        // A move during the fade waits; the latest wins.
        tap.set(30);
        tap.set(34);
        assert_eq!(tap.target(), 34);
        let values: Vec<f32> = (0..4).map(|_| tap.read(&line)).collect();
        assert_eq!(values, [12.5, 15.0, 17.5, 20.0]);
        let values: Vec<f32> = (0..5).map(|_| tap.read(&line)).collect();
        assert_eq!(values, [23.5, 27.0, 30.5, 34.0, 34.0]);
        // Setting the current target again cancels a queued move.
        tap.set(10);
        tap.set(12);
        tap.set(10);
        assert_eq!(tap.target(), 10);
        // Both heads move with the modulation.
        tap.snap(10);
        assert_eq!(tap.read_offset(&line, 3, 0.0), 13.0);
        assert!((tap.read_offset(&line, -2, 0.5) - 8.5).abs() < 1e-5);
        tap.set(20);
        assert!((tap.read_offset(&line, 1, 0.0) - 13.5).abs() < 1e-5);
    }

    #[test]
    fn sinc_reads_are_flat_and_a_passive_table_never_gains() {
        let (plain, passive) = (SincTable::new(1.0), SincTable::passive(1.0));
        let (mut highest, mut lowest) = (0.0_f64, f64::MAX);
        // Every tabulated fraction and every midpoint between two.
        for half_phase in 0..=128 {
            let frac = half_phase as f32 / 128.0;
            for step in 0..1024 {
                let omega = std::f64::consts::PI * f64::from(step) / 1024.0;
                let gain = passive.response(frac, omega).norm();
                highest = highest.max(gain);
                if omega <= 0.7 * std::f64::consts::PI {
                    lowest = lowest.min(gain);
                }
            }
        }
        assert!(highest <= 1.0, "gain {highest}");
        // Within 0.05 dB up to 0.35 of the sample rate.
        assert!(lowest >= 10.0_f64.powf(-0.05 / 20.0), "gain {lowest}");
        assert!((plain.response(0.37, 0.0).norm() - 1.0).abs() < 1e-6);

        // A whole delay is the tap, and a fraction delays a sine by it.
        let mut line = DelayLine::default();
        assert_eq!(line.read_sinc(20, 0.5, &plain), 0.0);
        line.allocate(256);
        let omega = 1.9_f64;
        for n in 0..400 {
            line.write((omega * f64::from(n)).sin() as f32);
        }
        assert!((line.read_sinc(40, 0.0, &plain) - line.tap(40)).abs() < 1e-6);
        for frac in [0.25_f32, 0.5, 0.731] {
            let expected = (omega * (399.0 - 40.0 - f64::from(frac))).sin();
            let read = f64::from(line.read_sinc(40, frac, &plain));
            assert!((read - expected).abs() < 2e-3, "{frac}: {read} {expected}");
            let response = plain.response(frac, omega);
            let phase = omega * (399.0 - 40.0) + response.im.atan2(response.re);
            assert!((read - response.norm() * phase.sin()).abs() < 1e-5);
        }
    }

    #[test]
    fn allpass_read_delays_by_the_fraction_at_unit_gain() {
        let mut line = DelayLine::default();
        line.allocate(256);
        // Whole cycles in the window that is measured.
        let (low, high) = (
            16.0 * std::f64::consts::TAU / 2000.0,
            796.0 * std::f64::consts::TAU / 2000.0,
        );
        for (omega, frac) in [(low, 0.2_f32), (low, 0.8), (high, 0.2), (high, 0.8)] {
            let mut read = AllpassInterp::default();
            let (mut sine, mut cosine) = (0.0, 0.0);
            for n in 0..4000 {
                let phase = omega * f64::from(n);
                line.write(phase.sin() as f32);
                let y = f64::from(read.read(&line, 30, frac));
                if n >= 2000 {
                    sine += y * phase.sin();
                    cosine += y * phase.cos();
                }
            }
            let gain = 2.0 * sine.hypot(cosine) / 2000.0;
            assert!((gain - 1.0).abs() < 2e-3, "{omega} {frac}: gain {gain}");
            if omega < 0.1 {
                // The line is read after the write: a tap of 30 is 30 back.
                let delay = -cosine.atan2(sine) / omega;
                assert!(
                    (delay - 30.0 - f64::from(frac)).abs() < 0.01,
                    "{frac}: {delay}"
                );
            }
        }
    }

    #[test]
    fn a_glide_is_rate_limited_and_arrives() {
        let mut length = 100.0;
        let mut steps = 0;
        while length != 5000.0 {
            let next = glide_length(length, 5000.0, 0.001, 0.5);
            assert!(next > length && next - length <= 0.5);
            length = next;
            steps += 1;
            assert!(steps < 200_000);
        }
        assert!(steps >= 9800);
        assert_eq!(glide_length(7.0, 7.001, 0.5, 0.5), 7.001);
        assert_eq!(glide_length(7.0, 6.0, 1.0, 0.5), 6.5);
    }

    #[test]
    fn static_delay_reads_any_tap_up_to_its_length() {
        let mut full = StaticDelay::<9>::default();
        let mut short = StaticDelay::<9>::default();
        let mut none = StaticDelay::<9>::default();
        let mut empty = StaticDelay::<0>::default();
        for n in 0..40 {
            let x = n as f32;
            assert_eq!(full.process(x), (x - 9.0).max(0.0));
            assert_eq!(short.process_at(x, 2), (x - 2.0).max(0.0));
            assert_eq!(none.process_at(x, 0), x);
            assert_eq!(empty.process(x), x);
        }
    }

    #[test]
    fn integer_delays_are_exact_and_fractions_interpolate() {
        let mut line = DelayLine::default();
        line.allocate(64);
        for n in 0..32 {
            line.write(n as f32);
        }
        assert_eq!(line.read(1.0), 30.0);
        assert_eq!(line.read(5.0), 26.0);
        assert!((line.read(5.25) - 25.75).abs() < 1e-4);
    }
}
