//! Nonlinear filters. The feedback loop is solved linearly (zero-delay
//! feedback), then the loop input passes through `tanh`. That keeps the
//! filters stable at any setting, including self-oscillation, without an
//! iterative solver.
use super::{OnePole, flush};

/// Soft saturator shaped like `tanh` (within 0.025, same slope and level at
/// 0, smoothly flat at ±1 beyond ±3), for a fraction of the cost: one short
/// division. The circuit models call it in every stage of every
/// oversampled sample, where its latency is the filter's cost.
#[inline]
pub fn tanh_fast(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Four-pole transistor ladder. `k` 0–4+ (self-oscillates near 4); the
/// passband drops as resonance rises, as in the original circuit.
#[derive(Clone, Debug, Default)]
pub struct Ladder {
    stages: [OnePole; 4],
}

impl Ladder {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The loop input and the four stage outputs; the last is the low-pass.
    /// Every stage input saturates, as the transistor pairs of a ladder and
    /// the input pairs of an OTA do. The loop is solved linearly, so small
    /// signals stay exact.
    #[inline]
    pub fn process(&mut self, input: f32, g: f32, k: f32) -> [f32; 5] {
        let beta = 1.0 / (1.0 + g);
        let big_g = g * beta;
        let [s1, s2, s3, s4] = [0, 1, 2, 3].map(|index| self.stages[index].state * beta);
        let sum = big_g * big_g * big_g * s1 + big_g * big_g * s2 + big_g * s3 + s4;
        let g4 = big_g * big_g * big_g * big_g;
        let u = tanh_fast((input - k * sum) / (1.0 + k * g4));
        let mut taps = [u; 5];
        for (index, stage) in self.stages.iter_mut().enumerate() {
            let stage_input = if index == 0 {
                u
            } else {
                tanh_fast(taps[index])
            };
            taps[index + 1] = stage.low(stage_input, big_g);
        }
        taps[4] = flush(taps[4]);
        taps
    }
}

/// Two-pole Sallen-Key: two one-poles with a clipped, positive, high-passed
/// feedback path. `k` 0–2 (self-oscillates near 2).
#[derive(Clone, Debug, Default)]
pub struct SallenKey {
    first: OnePole,
    second: OnePole,
    feedback: OnePole,
}

impl SallenKey {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Low-pass: LP → (+) → LP, feedback through a high-pass.
    #[inline]
    pub fn low_pass(&mut self, input: f32, g: f32, k: f32) -> f32 {
        let beta = 1.0 / (1.0 + g);
        let big_g = g * beta;
        let y1 = self.first.low(input, big_g);
        let s2 = self.second.state * beta;
        let s3 = self.feedback.state * beta;
        // u = y1 + k·HP(LP2(u)), solved for u.
        let u = (y1 + k * ((1.0 - big_g) * s2 - s3)) / (1.0 - k * big_g * (1.0 - big_g));
        let output = self.second.low(tanh_fast(u), big_g);
        self.feedback.low(output, big_g);
        flush(output)
    }

    /// High-pass: HP → (+) → HP, feedback through a low-pass.
    #[inline]
    pub fn high_pass(&mut self, input: f32, g: f32, k: f32) -> f32 {
        let beta = 1.0 / (1.0 + g);
        let big_g = g * beta;
        let y1 = input - self.first.low(input, big_g);
        let s2 = self.second.state * beta;
        let s3 = self.feedback.state * beta;
        // u = y1 + k·LP(HP2(u)), solved for u.
        let u = (y1 + k * (s3 - big_g * s2)) / (1.0 - k * big_g * (1.0 - big_g));
        let shaped = tanh_fast(u);
        let output = shaped - self.second.low(shaped, big_g);
        self.feedback.low(output, big_g);
        flush(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tanh_fast_follows_tanh_and_stays_monotonic() {
        let mut previous = -1.0;
        for step in -10_000..=10_000 {
            let x = step as f32 * 0.001;
            let y = tanh_fast(x);
            assert!((y - x.tanh()).abs() < 0.025, "{x}: {y} vs {}", x.tanh());
            // Flat within rounding next to ±3.
            assert!(y >= previous - 1e-6, "{x}");
            previous = y;
        }
        assert_eq!(tanh_fast(1e6), 1.0);
        assert_eq!(tanh_fast(-1e6), -1.0);
    }
}
