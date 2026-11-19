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

/// Safety stage for a loop that may reach unity gain: the exact identity up
/// to ±1, then a curve that leaves the knee with slope 1 and never reaches
/// `ceiling` (above 1).
#[inline]
pub fn soft_limit(x: f32, ceiling: f32) -> f32 {
    let size = x.abs();
    if size <= 1.0 {
        return x;
    }
    let room = ceiling - 1.0;
    let t = (size - 1.0) / room;
    (1.0 + room * t / (1.0 + t)).copysign(x)
}

/// Precomputed coefficients for [`Ladder`] at one `(g, k)` point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderCoeffs {
    pub big_g: f32,
    pub g2: f32,
    pub g3: f32,
    pub beta_g: f32,
    pub loop_denom: f32,
    pub k: f32,
}

impl LadderCoeffs {
    #[inline]
    pub fn new(g: f32, k: f32) -> Self {
        let beta = 1.0 / (1.0 + g);
        let big_g = g * beta;
        let g2 = big_g * big_g;
        let g3 = g2 * big_g;
        let g4 = big_g * big_g * big_g * big_g;
        Self {
            big_g,
            g2,
            g3,
            beta_g: beta,
            loop_denom: 1.0 + k * g4,
            k,
        }
    }
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

    #[inline]
    pub fn process_coeffs(&mut self, input: f32, coeffs: LadderCoeffs) -> [f32; 5] {
        let [s1, s2, s3, s4] = self.stages.each_ref().map(|s| s.state * coeffs.beta_g);
        let sum = coeffs.g3 * s1 + coeffs.g2 * s2 + coeffs.big_g * s3 + s4;
        let u = tanh_fast((input - coeffs.k * sum) / coeffs.loop_denom);
        let mut taps = [u; 5];
        for (index, stage) in self.stages.iter_mut().enumerate() {
            let stage_input = if index == 0 {
                u
            } else {
                tanh_fast(taps[index])
            };
            taps[index + 1] = stage.low(stage_input, coeffs.big_g);
        }
        taps[4] = flush(taps[4]);
        taps
    }

    /// The loop input and the four stage outputs; the last is the low-pass.
    /// Every stage input saturates, as the transistor pairs of a ladder and
    /// the input pairs of an OTA do. The loop is solved linearly, so small
    /// signals stay exact.
    #[inline]
    pub fn process(&mut self, input: f32, g: f32, k: f32) -> [f32; 5] {
        self.process_coeffs(input, LadderCoeffs::new(g, k))
    }
}

/// Stereo pair of [`Ladder`] filters sharing coefficients across channels.
#[derive(Clone, Debug, Default)]
pub struct LadderStereo {
    stages: [[f32; 2]; 4],
}

impl LadderStereo {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[inline]
    pub fn process_coeffs(&mut self, input: [f32; 2], coeffs: LadderCoeffs) -> [[f32; 2]; 5] {
        self.process_asym(input, [coeffs, coeffs])
    }

    #[inline]
    pub fn process_asym(&mut self, input: [f32; 2], coeffs: [LadderCoeffs; 2]) -> [[f32; 2]; 5] {
        let mut s = [[0.0_f32; 2]; 4];
        for (dst, src) in s.iter_mut().zip(self.stages.iter()) {
            *dst = [src[0] * coeffs[0].beta_g, src[1] * coeffs[1].beta_g];
        }
        let u = [
            tanh_fast(
                (input[0]
                    - coeffs[0].k
                        * (coeffs[0].g3 * s[0][0]
                            + coeffs[0].g2 * s[1][0]
                            + coeffs[0].big_g * s[2][0]
                            + s[3][0]))
                    / coeffs[0].loop_denom,
            ),
            tanh_fast(
                (input[1]
                    - coeffs[1].k
                        * (coeffs[1].g3 * s[0][1]
                            + coeffs[1].g2 * s[1][1]
                            + coeffs[1].big_g * s[2][1]
                            + s[3][1]))
                    / coeffs[1].loop_denom,
            ),
        ];
        let mut taps = [u; 5];
        for index in 0..4 {
            let stage_in = if index == 0 {
                u
            } else {
                [tanh_fast(taps[index][0]), tanh_fast(taps[index][1])]
            };
            let st = &mut self.stages[index];
            for c in 0..2 {
                let v = (stage_in[c] - st[c]) * coeffs[c].big_g;
                let low = v + st[c];
                st[c] = flush(low + v);
                taps[index + 1][c] = low;
            }
        }
        taps[4] = [flush(taps[4][0]), flush(taps[4][1])];
        taps
    }
}

/// Precomputed coefficients for [`SallenKey`] at one `(g, k)` point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SallenKeyCoeffs {
    pub beta: f32,
    pub big_g: f32,
    pub one_minus_g: f32,
    pub loop_denom: f32,
    pub k: f32,
}

impl SallenKeyCoeffs {
    #[inline]
    pub fn new(g: f32, k: f32) -> Self {
        let beta = 1.0 / (1.0 + g);
        let big_g = g * beta;
        let one_minus_g = 1.0 - big_g;
        Self {
            beta,
            big_g,
            one_minus_g,
            loop_denom: 1.0 - k * big_g * one_minus_g,
            k,
        }
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

    #[inline]
    pub fn low_pass_coeffs(&mut self, input: f32, coeffs: SallenKeyCoeffs) -> f32 {
        let y1 = self.first.low(input, coeffs.big_g);
        let s2 = self.second.state * coeffs.beta;
        let s3 = self.feedback.state * coeffs.beta;
        let u = (y1 + coeffs.k * (coeffs.one_minus_g * s2 - s3)) / coeffs.loop_denom;
        let output = self.second.low(tanh_fast(u), coeffs.big_g);
        self.feedback.low(output, coeffs.big_g);
        flush(output)
    }

    /// Low-pass: LP → (+) → LP, feedback through a high-pass.
    #[inline]
    pub fn low_pass(&mut self, input: f32, g: f32, k: f32) -> f32 {
        self.low_pass_coeffs(input, SallenKeyCoeffs::new(g, k))
    }

    #[inline]
    pub fn high_pass_coeffs(&mut self, input: f32, coeffs: SallenKeyCoeffs) -> f32 {
        let y1 = input - self.first.low(input, coeffs.big_g);
        let s2 = self.second.state * coeffs.beta;
        let s3 = self.feedback.state * coeffs.beta;
        let u = (y1 + coeffs.k * (s3 - coeffs.big_g * s2)) / coeffs.loop_denom;
        let shaped = tanh_fast(u);
        let output = shaped - self.second.low(shaped, coeffs.big_g);
        self.feedback.low(output, coeffs.big_g);
        flush(output)
    }

    /// High-pass: HP → (+) → HP, feedback through a low-pass.
    #[inline]
    pub fn high_pass(&mut self, input: f32, g: f32, k: f32) -> f32 {
        self.high_pass_coeffs(input, SallenKeyCoeffs::new(g, k))
    }
}

/// Stereo pair of [`SallenKey`] filters sharing coefficients across channels.
#[derive(Clone, Debug, Default)]
pub struct SallenKeyStereo {
    first: [f32; 2],
    second: [f32; 2],
    feedback: [f32; 2],
}

impl SallenKeyStereo {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[inline]
    fn step_one_pole(state: &mut [f32; 2], input: [f32; 2], big_g: [f32; 2]) -> [f32; 2] {
        let mut out = [0.0; 2];
        for c in 0..2 {
            let v = (input[c] - state[c]) * big_g[c];
            let low = v + state[c];
            state[c] = flush(low + v);
            out[c] = low;
        }
        out
    }

    #[inline]
    pub fn low_pass_coeffs(&mut self, input: [f32; 2], coeffs: SallenKeyCoeffs) -> [f32; 2] {
        self.low_pass_asym(input, [coeffs, coeffs])
    }

    #[inline]
    pub fn low_pass_asym(&mut self, input: [f32; 2], coeffs: [SallenKeyCoeffs; 2]) -> [f32; 2] {
        let big_g = [coeffs[0].big_g, coeffs[1].big_g];
        let y1 = Self::step_one_pole(&mut self.first, input, big_g);
        let s2 = [
            self.second[0] * coeffs[0].beta,
            self.second[1] * coeffs[1].beta,
        ];
        let s3 = [
            self.feedback[0] * coeffs[0].beta,
            self.feedback[1] * coeffs[1].beta,
        ];
        let shaped = [
            tanh_fast(
                (y1[0] + coeffs[0].k * (coeffs[0].one_minus_g * s2[0] - s3[0]))
                    / coeffs[0].loop_denom,
            ),
            tanh_fast(
                (y1[1] + coeffs[1].k * (coeffs[1].one_minus_g * s2[1] - s3[1]))
                    / coeffs[1].loop_denom,
            ),
        ];
        let output = Self::step_one_pole(&mut self.second, shaped, big_g);
        Self::step_one_pole(&mut self.feedback, output, big_g);
        [flush(output[0]), flush(output[1])]
    }

    #[inline]
    pub fn high_pass_coeffs(&mut self, input: [f32; 2], coeffs: SallenKeyCoeffs) -> [f32; 2] {
        self.high_pass_asym(input, [coeffs, coeffs])
    }

    #[inline]
    pub fn high_pass_asym(&mut self, input: [f32; 2], coeffs: [SallenKeyCoeffs; 2]) -> [f32; 2] {
        let big_g = [coeffs[0].big_g, coeffs[1].big_g];
        let low1 = Self::step_one_pole(&mut self.first, input, big_g);
        let y1 = [input[0] - low1[0], input[1] - low1[1]];
        let s2 = [
            self.second[0] * coeffs[0].beta,
            self.second[1] * coeffs[1].beta,
        ];
        let s3 = [
            self.feedback[0] * coeffs[0].beta,
            self.feedback[1] * coeffs[1].beta,
        ];
        let shaped = [
            tanh_fast((y1[0] + coeffs[0].k * (s3[0] - big_g[0] * s2[0])) / coeffs[0].loop_denom),
            tanh_fast((y1[1] + coeffs[1].k * (s3[1] - big_g[1] * s2[1])) / coeffs[1].loop_denom),
        ];
        let low2 = Self::step_one_pole(&mut self.second, shaped, big_g);
        let output = [shaped[0] - low2[0], shaped[1] - low2[1]];
        Self::step_one_pole(&mut self.feedback, output, big_g);
        [flush(output[0]), flush(output[1])]
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

    #[test]
    fn soft_limit_is_the_identity_below_one_and_bounded_above() {
        let mut previous = -4.0;
        for step in -20_000..=20_000 {
            let x = step as f32 * 0.001;
            let y = soft_limit(x, 4.0);
            if x.abs() <= 1.0 {
                assert_eq!(y.to_bits(), x.to_bits());
            }
            assert!(y >= previous && y.abs() < 4.0, "{x}: {y}");
            previous = y;
        }
        // Slope 1 on leaving the knee.
        assert!((soft_limit(1.001, 4.0) - 1.001).abs() < 1e-6);
        assert!(soft_limit(1e9, 4.0) <= 4.0 && soft_limit(-1e9, 4.0) >= -4.0);
    }
}
