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
