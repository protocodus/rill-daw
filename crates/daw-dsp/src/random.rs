/// Avalanching hash of a word (the finalizer of MurmurHash3): seeds that
/// differ in one bit give unrelated results.
pub fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x
}

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

    /// An independent sequence for each `(seed, a, b)`: adjacent seeds
    /// give unrelated streams, which plain xorshift seeds do not.
    pub fn stream(seed: u32, a: u32, b: u32) -> Self {
        const GOLDEN: u32 = 0x9E37_79B9;
        let state = hash32(
            hash32(seed.wrapping_add(GOLDEN.wrapping_mul(a))).wrapping_add(GOLDEN.wrapping_mul(b)),
        );
        let mut random = Self::new(if state == 0 { 0x6D2B_79F5 } else { state });
        random.next_u32();
        random.next_u32();
        Self::new(random.state)
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

    /// Uniform in 0–1, without 1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1 << 24) as f32
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
    fn streams_of_adjacent_seeds_are_unrelated_and_repeat() {
        assert_eq!(hash32(0), 0);
        assert_ne!(hash32(1), hash32(2));
        let draw = |seed, a, b| -> Vec<f32> {
            let mut random = Random::stream(seed, a, b);
            (0..4096).map(|_| random.unit()).collect()
        };
        let first = draw(1, 0, 0);
        assert_eq!(first, draw(1, 0, 0));
        assert!(first.iter().all(|value| (0.0..1.0).contains(value)));
        let mean = first.iter().sum::<f32>() / first.len() as f32;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
        // Neighbours in any argument share nothing.
        for other in [draw(2, 0, 0), draw(1, 1, 0), draw(1, 0, 1), draw(0, 0, 0)] {
            let correlation: f32 = first
                .iter()
                .zip(&other)
                .map(|(a, b)| (a - 0.5) * (b - 0.5))
                .sum::<f32>()
                / (first.len() as f32 / 12.0);
            assert!(correlation.abs() < 0.06, "{correlation}");
        }
        let mut random = Random::stream(5, 2, 3);
        let value = random.next_u32();
        random.next_u32();
        random.reset();
        assert_eq!(random.next_u32(), value);
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
