use super::{flush, tanh_fast};

/// Linear zero-delay-feedback (TPT) state-variable filter after Simper. Stable
/// under per-sample coefficient changes, so cutoff can be modulated freely.
#[derive(Clone, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SvfOutputs {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

impl SvfOutputs {
    pub fn notch(self) -> f32 {
        self.low + self.high
    }
}

impl Svf {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `g` from [`super::prewarp`]; `k` = 1/Q.
    #[inline]
    pub fn process(&mut self, input: f32, g: f32, k: f32) -> SvfOutputs {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        SvfOutputs {
            low: v2,
            band: v1,
            high: input - k * v1 - v2,
        }
    }

    /// [`Self::process`] with the band state soft-clipped, like a diode pair
    /// in the resonance path: bounded at any damping, including slightly
    /// negative damping, which self-oscillates.
    #[inline]
    pub fn process_clipped(&mut self, input: f32, g: f32, k: f32) -> SvfOutputs {
        let outputs = self.process(input, g, k);
        self.ic1 = tanh_fast(self.ic1);
        outputs
    }

    /// [`Self::process`] with both integrator states saturating, like the
    /// OTAs of an analog state-variable filter: bounded at any damping.
    #[inline]
    pub fn process_saturated(&mut self, input: f32, g: f32, k: f32) -> SvfOutputs {
        let outputs = self.process(input, g, k);
        self.ic1 = tanh_fast(self.ic1);
        self.ic2 = tanh_fast(self.ic2);
        outputs
    }
}

/// Zero-delay-feedback one-pole low-pass; high-pass is `input - low`.
/// `low = G·input + β·state` with `G = g/(1+g)`, `β = 1/(1+g)`.
#[derive(Clone, Debug, Default)]
pub struct OnePole {
    pub state: f32,
}

impl OnePole {
    #[inline]
    pub fn low(&mut self, input: f32, big_g: f32) -> f32 {
        let v = (input - self.state) * big_g;
        let low = v + self.state;
        self.state = flush(low + v);
        low
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prewarp;

    const RATE: f32 = 48_000.0;

    /// Amplitude of each output after a settled sine at `frequency`.
    fn gains(frequency: f32, cutoff: f32, k: f32) -> [f32; 3] {
        let mut svf = Svf::default();
        let g = prewarp(cutoff, RATE);
        let mut peaks = [0.0_f32; 3];
        for n in 0..48_000 {
            let x = (std::f32::consts::TAU * frequency * n as f32 / RATE).sin();
            let out = svf.process(x, g, k);
            if n >= 24_000 {
                for (peak, y) in peaks.iter_mut().zip([out.low, out.band, out.high]) {
                    *peak = peak.max(y.abs());
                }
            }
        }
        peaks
    }

    #[test]
    fn outputs_split_the_spectrum_at_the_cutoff() {
        let k = std::f32::consts::SQRT_2;
        let [low, band, high] = gains(50.0, 2000.0, k);
        assert!((low - 1.0).abs() < 0.01 && high < 0.01 && band < 0.05);
        let [low, band, high] = gains(15_000.0, 1000.0, k);
        assert!(low < 0.01 && (high - 1.0).abs() < 0.01 && band < 0.1);
        // Butterworth: both −3 dB at the cutoff, band-pass peaks at 1/k.
        let [low, band, high] = gains(1000.0, 1000.0, k);
        for gain in [low, high] {
            assert!(
                (gain - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01,
                "{gain}"
            );
        }
        assert!((band * k - 1.0).abs() < 0.01, "{band}");
    }

    #[test]
    fn saturating_variants_self_oscillate_without_blowing_up() {
        let g = prewarp(1000.0, RATE);
        for clipped in [false, true] {
            let mut svf = Svf::default();
            let mut peak = 0.0_f32;
            for n in 0..96_000 {
                let input = if n == 0 { 0.1 } else { 0.0 };
                let out = if clipped {
                    svf.process_clipped(input, g, -0.05)
                } else {
                    svf.process_saturated(input, g, -0.05)
                };
                assert!(out.band.is_finite());
                if n > 90_000 {
                    peak = peak.max(out.band.abs());
                }
            }
            assert!((0.1..4.0).contains(&peak), "clipped {clipped}: {peak}");
        }
    }

    #[test]
    fn decays_flush_to_exact_zero() {
        let mut svf = Svf::default();
        let g = prewarp(100.0, RATE);
        svf.process(1.0, g, 1.0);
        for _ in 0..RATE as usize * 10 {
            svf.process(0.0, g, 1.0);
        }
        assert_eq!((svf.ic1, svf.ic2), (0.0, 0.0));
    }

    #[test]
    fn one_pole_passes_dc_and_is_3_db_down_at_its_corner() {
        let g = prewarp(1000.0, RATE);
        let big_g = g / (1.0 + g);
        let mut pole = OnePole::default();
        let mut last = 0.0;
        for _ in 0..10_000 {
            last = pole.low(1.0, big_g);
        }
        assert!((last - 1.0).abs() < 1e-5, "{last}");
        let mut pole = OnePole::default();
        let mut peak = 0.0_f32;
        for n in 0..48_000 {
            let x = (std::f32::consts::TAU * 1000.0 * n as f32 / RATE).sin();
            let y = pole.low(x, big_g);
            if n > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        assert!(
            (peak - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01,
            "{peak}"
        );
    }
}
