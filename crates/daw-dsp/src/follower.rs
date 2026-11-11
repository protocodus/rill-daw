use super::flush;

/// Peak envelope follower with separate attack and release, 0–1 for
/// full-scale input.
#[derive(Clone, Debug, Default)]
pub struct EnvelopeFollower {
    level: f32,
    attack: f32,
    release: f32,
}

impl EnvelopeFollower {
    pub fn set_times(&mut self, attack_ms: f32, release_ms: f32, sample_rate: f32) {
        let coefficient = |ms: f32| 1.0 - (-1.0 / (ms * 0.001 * sample_rate).max(1.0)).exp();
        self.attack = coefficient(attack_ms);
        self.release = coefficient(release_ms);
    }

    pub fn reset(&mut self) {
        self.level = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let input = input.abs().min(1.0);
        let coefficient = if input > self.level {
            self.attack
        } else {
            self.release
        };
        self.level = flush(self.level + (input - self.level) * coefficient);
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    #[test]
    fn attack_and_release_take_one_time_constant_each() {
        let mut follower = EnvelopeFollower::default();
        follower.set_times(10.0, 100.0, RATE);
        let mut level = 0.0;
        for _ in 0..480 {
            level = follower.process(1.0);
        }
        // One time constant: 1 − 1/e of the way.
        assert!((level - 0.632).abs() < 0.01, "{level}");
        for _ in 0..48_000 {
            follower.process(-1.0);
        }
        let full = follower.process(0.0);
        for _ in 0..4799 {
            level = follower.process(0.0);
        }
        assert!((level / full - 0.368).abs() < 0.01, "{level} of {full}");
    }

    #[test]
    fn level_stays_within_full_scale_and_reset_clears_it() {
        let mut follower = EnvelopeFollower::default();
        follower.set_times(0.0, 0.0, RATE);
        // Zero times clamp to one sample instead of dividing by zero, and
        // input beyond full scale counts as full scale.
        let first = follower.process(-4.0);
        assert!((first - 0.632).abs() < 0.001, "{first}");
        let mut level = first;
        for _ in 0..32 {
            level = follower.process(4.0);
        }
        assert_eq!(level, 1.0);
        follower.reset();
        assert_eq!(follower.process(0.0), 0.0);
    }
}
