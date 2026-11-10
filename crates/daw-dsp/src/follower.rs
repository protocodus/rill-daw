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
