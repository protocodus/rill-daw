/// One-pole parameter smoother. Reaches about 99 % of a jump in the set time,
/// so block-rate automation does not zipper.
#[derive(Clone, Debug)]
pub struct Smoothed {
    current: f32,
    target: f32,
    coefficient: f32,
}

impl Smoothed {
    pub fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            coefficient: 1.0,
        }
    }

    pub fn set_time(&mut self, milliseconds: f32, sample_rate: f32) {
        let samples = (milliseconds * 0.001 * sample_rate).max(1.0);
        // exp(-4.6) ≈ 1 %: the remaining distance after `samples` steps.
        self.coefficient = 1.0 - (-4.6 / samples).exp();
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Jump to the target, for `reset` and the first block after `prepare`.
    pub fn snap(&mut self) {
        self.current = self.target;
    }

    #[inline]
    pub fn next_value(&mut self) -> f32 {
        let next = self.current + (self.target - self.current) * self.coefficient;
        // Once a step no longer moves the value in f32, finish exactly.
        self.current = if next == self.current {
            self.target
        } else {
            next
        };
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaches_target_in_set_time() {
        let mut smoothed = Smoothed::new(0.0);
        smoothed.set_time(5.0, 48_000.0);
        smoothed.set_target(1.0);
        let mut value = 0.0;
        for _ in 0..240 {
            value = smoothed.next_value();
        }
        assert!((0.985..=1.0).contains(&value), "{value}");
        for _ in 0..10_000 {
            value = smoothed.next_value();
        }
        assert_eq!(value, 1.0);
    }
}
