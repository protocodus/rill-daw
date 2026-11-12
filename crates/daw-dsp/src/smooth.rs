use super::Real;

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

    /// The value [`Self::next_value`] returned last.
    pub fn value(&self) -> f32 {
        self.current
    }

    /// Whether the value has arrived, so work that follows it can stop.
    pub fn is_settled(&self) -> bool {
        self.current == self.target
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

/// Straight-line ramp that reaches its target in a set number of samples and
/// then holds it exactly: fades, crossfades and mixes.
#[derive(Clone, Copy, Debug)]
pub struct LinearRamp<T: Real = f32> {
    current: T,
    target: T,
    step: T,
    remaining: u32,
}

impl<T: Real> LinearRamp<T> {
    pub fn new(value: T) -> Self {
        Self {
            current: value,
            target: value,
            step: T::ZERO,
            remaining: 0,
        }
    }

    /// Reach `target` in `samples` calls of [`Self::next`], from wherever the
    /// ramp is now.
    pub fn set_target(&mut self, target: T, samples: u32) {
        self.target = target;
        self.remaining = samples;
        if samples == 0 {
            self.current = target;
        } else {
            self.step = (target - self.current) / T::from_f64(f64::from(samples));
        }
    }

    pub fn target(&self) -> T {
        self.target
    }

    pub fn value(&self) -> T {
        self.current
    }

    /// Jump to the target.
    pub fn snap(&mut self) {
        self.current = self.target;
        self.remaining = 0;
    }

    pub fn settled(&self) -> bool {
        self.remaining == 0
    }

    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> T {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current = if self.remaining == 0 {
                self.target
            } else {
                self.current + self.step
            };
        }
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

    #[test]
    fn reports_its_value_and_when_it_has_arrived() {
        let mut smoothed = Smoothed::new(2.0);
        assert!(smoothed.is_settled());
        smoothed.set_time(1.0, 1000.0);
        smoothed.set_target(3.0);
        assert!(!smoothed.is_settled());
        let value = smoothed.next_value();
        assert_eq!(smoothed.value(), value);
        for _ in 0..200 {
            smoothed.next_value();
        }
        assert!(smoothed.is_settled());
        assert_eq!(smoothed.value(), 3.0);
    }

    #[test]
    fn linear_ramp_lands_exactly_after_its_length() {
        let mut ramp = LinearRamp::new(0.0_f64);
        assert!(ramp.settled());
        ramp.set_target(1.0, 480);
        for step in 1..=480 {
            let value = ramp.next();
            assert_eq!(ramp.settled(), step == 480);
            if step < 480 {
                assert!((value - f64::from(step) / 480.0).abs() < 1e-12);
            }
        }
        assert_eq!(ramp.value(), 1.0);
        assert_eq!(ramp.next(), 1.0);
        // Retargeting midway continues from the current value.
        ramp.set_target(0.0, 10);
        for _ in 0..5 {
            ramp.next();
        }
        ramp.set_target(1.0, 4);
        assert!((ramp.value() - 0.5).abs() < 1e-12);
        assert_eq!(ramp.target(), 1.0);
        ramp.snap();
        assert!(ramp.settled());
        assert_eq!(ramp.value(), 1.0);
        let mut narrow = LinearRamp::new(1.0_f32);
        narrow.set_target(0.25, 0);
        assert_eq!(narrow.next(), 0.25);
    }
}
