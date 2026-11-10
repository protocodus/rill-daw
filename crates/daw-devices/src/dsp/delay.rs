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

#[cfg(test)]
mod tests {
    use super::*;

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
