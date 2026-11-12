use super::Complex;

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
        let (from, to) = (self.from, self.to);
        let t = self.advance();
        if t == 1.0 {
            line.tap(to)
        } else {
            line.tap(from) + t * (line.tap(to) - line.tap(from))
        }
    }

    /// [`Self::read`] with both heads moved by the same modulation, in whole
    /// samples and a fraction (0 up to 1), interpolated.
    #[inline]
    pub fn read_offset(&mut self, line: &DelayLine, whole: isize, frac: f32) -> f32 {
        let (from, to) = (self.from, self.to);
        let t = self.advance();
        let at = |delay: usize| line.read_at((delay as isize + whole).max(1) as usize, frac);
        if t == 1.0 {
            at(to)
        } else {
            let old = at(from);
            old + t * (at(to) - old)
        }
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
