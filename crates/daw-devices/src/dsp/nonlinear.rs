//! Nonlinear filters. The feedback loop is solved linearly (zero-delay
//! feedback), then the loop input passes through `tanh`. That keeps the
//! filters stable at any setting, including self-oscillation, without an
//! iterative solver.
use super::{OnePole, flush};

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

    #[inline]
    pub fn process(&mut self, input: f32, g: f32, k: f32) -> f32 {
        let big_g = g / (1.0 + g);
        let beta = 1.0 / (1.0 + g);
        let [s1, s2, s3, s4] = [0, 1, 2, 3].map(|index| self.stages[index].state * beta);
        let sum = big_g * big_g * big_g * s1 + big_g * big_g * s2 + big_g * s3 + s4;
        let g4 = big_g * big_g * big_g * big_g;
        let u = ((input - k * sum) / (1.0 + k * g4)).tanh();
        let mut signal = u;
        for stage in &mut self.stages {
            signal = stage.low(signal, big_g);
        }
        flush(signal)
    }
}

/// Two-pole Sallen-Key (Korg-35 topology): two one-poles with a clipped,
/// positive, high-passed feedback path. `k` 0–2 (self-oscillates near 2).
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
        let big_g = g / (1.0 + g);
        let beta = 1.0 / (1.0 + g);
        let y1 = self.first.low(input, big_g);
        let s2 = self.second.state * beta;
        let s3 = self.feedback.state * beta;
        // u = y1 + k·HP(LP2(u)), solved for u.
        let u = (y1 + k * ((1.0 - big_g) * s2 - s3)) / (1.0 - k * big_g * (1.0 - big_g));
        let output = self.second.low(u.tanh(), big_g);
        self.feedback.low(output, big_g);
        flush(output)
    }

    /// High-pass: HP → (+) → HP, feedback through a low-pass.
    #[inline]
    pub fn high_pass(&mut self, input: f32, g: f32, k: f32) -> f32 {
        let big_g = g / (1.0 + g);
        let beta = 1.0 / (1.0 + g);
        let y1 = input - self.first.low(input, big_g);
        let s2 = self.second.state * beta;
        let s3 = self.feedback.state * beta;
        // u = y1 + k·LP(HP2(u)), solved for u.
        let u = (y1 + k * (s3 - big_g * s2)) / (1.0 - k * big_g * (1.0 - big_g));
        let shaped = u.tanh();
        let output = shaped - self.second.low(shaped, big_g);
        self.feedback.low(output, big_g);
        flush(output)
    }
}
