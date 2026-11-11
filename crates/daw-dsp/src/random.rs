/// Seeded xorshift generator: "analog" randomness that is still bit-identical
/// from run to run.
#[derive(Clone, Debug)]
pub struct Random {
    seed: u32,
    state: u32,
}

impl Random {
    pub fn new(seed: u32) -> Self {
        let seed = seed.max(1);
        Self { seed, state: seed }
    }

    pub fn reset(&mut self) {
        self.state = self.seed;
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Uniform in −1–1.
    pub fn bipolar(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1 << 23) as f32 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequences_repeat_after_reset_and_differ_by_seed() {
        let mut first = Random::new(7);
        let sequence: Vec<u32> = (0..64).map(|_| first.next_u32()).collect();
        first.reset();
        assert!(sequence.iter().all(|&value| value == first.next_u32()));
        let mut other = Random::new(8);
        assert_ne!(other.next_u32(), sequence[0]);
        // Zero would make xorshift stick at zero forever.
        let mut zero = Random::new(0);
        assert_eq!(zero.next_u32(), Random::new(1).next_u32());
    }

    #[test]
    fn bipolar_values_cover_the_range_evenly() {
        let mut random = Random::new(12345);
        let values: Vec<f32> = (0..100_000).map(|_| random.bipolar()).collect();
        assert!(values.iter().all(|value| (-1.0..1.0).contains(value)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!(mean.abs() < 0.01, "{mean}");
        let (min, max) = values.iter().fold((1.0_f32, -1.0_f32), |(min, max), &v| {
            (min.min(v), max.max(v))
        });
        assert!(min < -0.999 && max > 0.999, "{min} – {max}");
    }
}
