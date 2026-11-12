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
        if self.buffer.is_empty() {
            return 0.0;
        }
        let delay = delay.clamp(1.0, self.max_delay());
        let whole = delay.floor();
        let d = delay - whole;
        // Taps at whole-1 … whole+2 put the fraction between the middle two.
        let base = self.cursor.wrapping_sub(whole as usize - 1);
        let at = |offset: usize| self.buffer[base.wrapping_sub(offset) & self.mask];
        let (y0, y1, y2, y3) = (at(0), at(1), at(2), at(3));
        // Lagrange weights for x = 1 + d over nodes 0, 1, 2, 3.
        let x = 1.0 + d;
        let c0 = -(x - 1.0) * (x - 2.0) * (x - 3.0) / 6.0;
        let c1 = x * (x - 2.0) * (x - 3.0) / 2.0;
        let c2 = -x * (x - 1.0) * (x - 3.0) / 2.0;
        let c3 = x * (x - 1.0) * (x - 2.0) / 6.0;
        c0 * y0 + c1 * y1 + c2 * y2 + c3 * y3
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
