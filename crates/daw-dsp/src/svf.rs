use super::{Biquad, Complex, Real};

/// Linear zero-delay-feedback (TPT) state-variable filter after Simper. Stable
/// under per-sample coefficient changes, so cutoff can be modulated freely.
/// `f32` unless a caller asks for `f64` state.
#[derive(Clone, Debug, Default)]
pub struct Svf<T: Real = f32> {
    ic1: T,
    ic2: T,
}

#[derive(Clone, Copy, Debug)]
pub struct SvfOutputs<T: Real = f32> {
    pub low: T,
    pub band: T,
    pub high: T,
}

impl<T: Real> SvfOutputs<T> {
    pub fn notch(self) -> T {
        self.low + self.high
    }
}

impl<T: Real> Svf<T> {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `g` from [`super::prewarp`]; `k` = 1/Q.
    #[inline]
    pub fn process(&mut self, input: T, g: T, k: T) -> SvfOutputs<T> {
        let a1 = T::ONE / (T::ONE + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = input - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = (T::TWO * v1 - self.ic1).flush();
        self.ic2 = (T::TWO * v2 - self.ic2).flush();
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
    pub fn process_clipped(&mut self, input: T, g: T, k: T) -> SvfOutputs<T> {
        let outputs = self.process(input, g, k);
        self.ic1 = self.ic1.tanh_fast();
        outputs
    }

    /// [`Self::process`] with both integrator states saturating, like the
    /// OTAs of an analog state-variable filter: bounded at any damping.
    #[inline]
    pub fn process_saturated(&mut self, input: T, g: T, k: T) -> SvfOutputs<T> {
        let outputs = self.process(input, g, k);
        self.ic1 = self.ic1.tanh_fast();
        self.ic2 = self.ic2.tanh_fast();
        outputs
    }

    /// One sample through precomputed coefficients and an output mix: any
    /// second-order response, see [`SvfMix`].
    #[inline]
    pub fn tick(&mut self, input: T, mix: &SvfMix<T>) -> T {
        let v3 = mix.drive * input - self.ic2;
        let v1 = mix.a1 * self.ic1 + mix.a2 * v3;
        let v2 = self.ic2 + mix.a2 * self.ic1 + mix.a3 * v3;
        self.ic1 = (T::TWO * v1 - self.ic1).flush();
        self.ic2 = (T::TWO * v2 - self.ic2).flush();
        mix.m0 * input + mix.m1 * v1 + mix.m2 * v2
    }
}

/// Coefficients of an [`Svf`] whose output is `m0·input + m1·band + m2·low`,
/// with the filter fed `drive·input`. With [`Self::from_biquad`] the filter
/// realizes any stable biquad exactly, in a structure that keeps its
/// precision at low frequencies and stays stable while `g` and `k` move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvfMix<T: Real = f32> {
    pub g: T,
    pub k: T,
    /// Gain into the filter; `m1` and `m2` make up for it.
    pub drive: T,
    pub m0: T,
    pub m1: T,
    pub m2: T,
    // 1/(1 + g(g + k)), g·a1, g·a2: cached so a settled filter does not divide.
    a1: T,
    a2: T,
    a3: T,
}

impl<T: Real> SvfMix<T> {
    pub fn new(g: T, k: T, m0: T, m1: T, m2: T) -> Self {
        let a1 = T::ONE / (T::ONE + g * (g + k));
        let a2 = g * a1;
        Self {
            g,
            k,
            drive: T::ONE,
            m0,
            m1,
            m2,
            a1,
            a2,
            a3: g * a2,
        }
    }

    /// Passes its input unchanged.
    pub fn identity() -> Self {
        Self::new(T::ONE, T::TWO, T::ONE, T::ZERO, T::ZERO)
    }

    /// The same poles with the identity mix: what a band at 0 dB ramps to, so
    /// that it can be skipped without its `g` and `k` jumping.
    pub fn bypassed(&self) -> Self {
        Self {
            m0: T::ONE,
            m1: T::ZERO,
            m2: T::ZERO,
            ..*self
        }
    }

    /// The same response with `drive` moved to the input, which scales the
    /// filter's states by it.
    pub fn driven(&self, drive: T) -> Self {
        let ratio = self.drive / drive;
        Self {
            drive,
            m1: self.m1 * ratio,
            m2: self.m2 * ratio,
            ..*self
        }
    }

    /// The same response with the input scaled by `k`, so that the band
    /// state has unit gain at resonance whatever the Q. A bell or notch that
    /// takes its output from the band state then stays settled when its Q or
    /// gain moves: with a drive of 1 that state is Q times the input, and a
    /// falling Q releases it into the output as an overshoot of up to the
    /// ratio of the two Qs. The price is a low state whose size follows `k`,
    /// so responses that lean on it (shelves, low-passes) keep a drive of 1.
    pub fn constant_peak(&self) -> Self {
        self.driven(self.k)
    }

    pub fn is_identity(&self) -> bool {
        self.m0 == T::ONE && self.m1 == T::ZERO && self.m2 == T::ZERO
    }
}

impl SvfMix<f64> {
    /// The mix that gives this filter the biquad's response. `g` and `k` are
    /// positive for every stable biquad.
    pub fn from_biquad(biquad: &Biquad) -> Self {
        let Biquad { b0, b1, b2, a1, a2 } = *biquad;
        let at_dc = 1.0 + a1 + a2;
        let at_nyquist = 1.0 - a1 + a2;
        let g = (at_dc / at_nyquist).sqrt();
        let k = 2.0 * (1.0 - a2) / (at_dc * at_nyquist).sqrt();
        let m0 = (b0 - b1 + b2) / at_nyquist;
        let m1 = 2.0 * (b0 - b2) / (g * at_nyquist) - m0 * k;
        let m2 = (b0 + b1 + b2) / at_dc - m0;
        Self::new(g, k, m0, m1, m2)
    }

    /// Exact response at `omega` radians per sample.
    pub fn response(&self, omega: f64) -> Complex {
        let omega = omega.clamp(0.0, 0.9999 * std::f64::consts::PI);
        let x = Complex::new(0.0, (0.5 * omega).tan() / self.g);
        let x2 = x * x;
        let (m1, m2) = (self.drive * self.m1, self.drive * self.m2);
        (x2.scale(self.m0) + x.scale(self.m0 * self.k + m1) + Complex::real(self.m0 + m2))
            / (x2 + x.scale(self.k) + Complex::ONE)
    }
}

/// Moves an [`SvfMix`] to a target in a straight line, one step per sample.
#[derive(Clone, Copy, Debug)]
pub struct SvfMixRamp<T: Real = f32> {
    current: SvfMix<T>,
    target: SvfMix<T>,
    /// Per-sample steps of g, k, drive, m0, m1, m2.
    step: [T; 6],
    remaining: u32,
}

impl<T: Real> SvfMixRamp<T> {
    pub fn new(mix: SvfMix<T>) -> Self {
        Self {
            current: mix,
            target: mix,
            step: [T::ZERO; 6],
            remaining: 0,
        }
    }

    /// Reach `target` in `samples` calls of [`Self::advance`].
    pub fn aim(&mut self, target: SvfMix<T>, samples: u32) {
        self.target = target;
        self.remaining = samples;
        if samples == 0 {
            self.current = target;
            return;
        }
        let scale = T::ONE / T::from_f64(f64::from(samples));
        let c = &self.current;
        self.step = [
            (target.g - c.g) * scale,
            (target.k - c.k) * scale,
            (target.drive - c.drive) * scale,
            (target.m0 - c.m0) * scale,
            (target.m1 - c.m1) * scale,
            (target.m2 - c.m2) * scale,
        ];
    }

    /// Jump to `target`.
    pub fn snap(&mut self, target: SvfMix<T>) {
        self.aim(target, 0);
    }

    /// The coefficients for the next sample.
    #[inline]
    pub fn advance(&mut self) -> &SvfMix<T> {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current = if self.remaining == 0 {
                // Land exactly, whatever the steps accumulated.
                self.target
            } else {
                let c = &self.current;
                SvfMix {
                    drive: c.drive + self.step[2],
                    ..SvfMix::new(
                        c.g + self.step[0],
                        c.k + self.step[1],
                        c.m0 + self.step[3],
                        c.m1 + self.step[4],
                        c.m2 + self.step[5],
                    )
                }
            };
        }
        &self.current
    }

    pub fn mix(&self) -> &SvfMix<T> {
        &self.current
    }

    pub fn target(&self) -> &SvfMix<T> {
        &self.target
    }

    pub fn done(&self) -> bool {
        self.remaining == 0
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
        self.state = super::flush(low + v);
        low
    }
}

/// First-order section on a [`OnePole`]:
/// `out = low_gain·lp + high_gain·(x − lp)`. One block for shelves, exact
/// emphasis pairs, tilt, DC blocking and plain one-pole low-passes.
#[derive(Clone, Debug)]
pub struct Shelf1 {
    /// Integrator gain of the pole, `k/(1 + k)` with `k = tan(π·f/fs)`.
    pub big_g: f32,
    pub low_gain: f32,
    pub high_gain: f32,
    pole: OnePole,
}

impl Default for Shelf1 {
    /// Passes its input unchanged.
    fn default() -> Self {
        Self::with_gains(1.0, 1.0, 1.0)
    }
}

/// Bilinear frequency variable, held below 0.45 of the sample rate.
fn warp(hz: f32, sample_rate: f32) -> f32 {
    (std::f32::consts::PI * (hz / sample_rate).clamp(1e-6, 0.45)).tan()
}

impl Shelf1 {
    /// From the bilinear pole variable `k = tan(π·f/fs)` and the gains at DC
    /// and at the top.
    pub fn with_gains(k: f32, low_gain: f32, high_gain: f32) -> Self {
        Self {
            big_g: k / (1.0 + k),
            low_gain,
            high_gain,
            pole: OnePole::default(),
        }
    }

    /// High shelf with unit gain at DC: zero at `zero_hz`, pole at `pole_hz`,
    /// so the top is raised by about their ratio (exactly that of the
    /// warped frequencies).
    pub fn shelf(zero_hz: f32, pole_hz: f32, sample_rate: f32) -> Self {
        let (zero, pole) = (warp(zero_hz, sample_rate), warp(pole_hz, sample_rate));
        Self::with_gains(pole, 1.0, pole / zero)
    }

    /// The exact digital inverse of [`Self::shelf`] with the same arguments:
    /// pole and zero exchanged. Both are the same bilinear function, so the
    /// pair is the identity at any sample rate.
    pub fn inverse_shelf(zero_hz: f32, pole_hz: f32, sample_rate: f32) -> Self {
        let (zero, pole) = (warp(zero_hz, sample_rate), warp(pole_hz, sample_rate));
        Self::with_gains(zero, 1.0, zero / pole)
    }

    /// Tilt around `pivot_hz`: `−gain_db` at DC, `+gain_db` at the top, 0 dB
    /// at the pivot.
    pub fn tilt(gain_db: f32, pivot_hz: f32, sample_rate: f32) -> Self {
        let a = crate::db_to_gain(gain_db);
        Self::with_gains(warp(a * pivot_hz, sample_rate), 1.0 / a, a)
    }

    /// First-order high-pass; at a few hertz, a DC blocker.
    pub fn high_pass(hz: f32, sample_rate: f32) -> Self {
        Self::with_gains(warp(hz, sample_rate), 0.0, 1.0)
    }

    /// First-order low-pass.
    pub fn low_pass(hz: f32, sample_rate: f32) -> Self {
        Self::with_gains(warp(hz, sample_rate), 1.0, 0.0)
    }

    pub fn low_shelf(gain_db: f32, hz: f32, sample_rate: f32) -> Self {
        Self::with_gains(warp(hz, sample_rate), crate::db_to_gain(gain_db), 1.0)
    }

    /// Takes the other section's coefficients and keeps the state.
    pub fn set(&mut self, other: &Shelf1) {
        self.big_g = other.big_g;
        self.low_gain = other.low_gain;
        self.high_gain = other.high_gain;
    }

    pub fn reset(&mut self) {
        self.pole.state = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let low = self.pole.low(input, self.big_g);
        self.low_gain * low + self.high_gain * (input - low)
    }

    /// The pole's low-pass alone, whatever the gains.
    #[inline]
    pub fn low(&mut self, input: f32) -> f32 {
        self.pole.low(input, self.big_g)
    }

    /// Exact response at `omega` radians per sample.
    pub fn response(&self, omega: f64) -> Complex {
        let omega = omega.clamp(0.0, 0.9999 * std::f64::consts::PI);
        let big_g = f64::from(self.big_g);
        let x = Complex::new(0.0, (0.5 * omega).tan() * (1.0 - big_g) / big_g);
        (Complex::real(f64::from(self.low_gain)) + x.scale(f64::from(self.high_gain)))
            / (Complex::ONE + x)
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

    fn test_biquads() -> Vec<Biquad> {
        // Resonant bell, shelf-like, low-pass-like and a notch.
        vec![
            Biquad {
                b0: 1.3,
                b1: -1.7,
                b2: 0.55,
                a1: -1.75,
                a2: 0.82,
            },
            Biquad {
                b0: 2.0,
                b1: -0.4,
                b2: 0.1,
                a1: 0.3,
                a2: 0.2,
            },
            Biquad {
                b0: 0.02,
                b1: 0.04,
                b2: 0.02,
                a1: -1.6,
                a2: 0.68,
            },
            Biquad {
                b0: 0.9,
                b1: -1.2,
                b2: 0.9,
                a1: -1.2,
                a2: 0.8,
            },
        ]
    }

    #[test]
    fn mix_from_a_biquad_has_the_biquads_response() {
        for biquad in test_biquads() {
            let mix = SvfMix::from_biquad(&biquad);
            assert!(mix.g > 0.0 && mix.k > 0.0);
            for step in 1..200 {
                let omega = f64::from(step) * 0.0157;
                let expected = biquad.response(omega);
                let error = (mix.response(omega) - expected).norm() / expected.norm().max(1.0);
                assert!(error < 1e-12, "{biquad:?} at {omega}: {error}");
            }
        }
    }

    #[test]
    fn tick_matches_the_direct_recursion() {
        for biquad in test_biquads() {
            let mix = SvfMix::from_biquad(&biquad);
            let mut svf = Svf::<f64>::default();
            let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
            for n in 0..400 {
                let x = if n == 0 { 1.0 } else { 0.0 };
                let direct = biquad.b0 * x + biquad.b1 * x1 + biquad.b2 * x2
                    - biquad.a1 * y1
                    - biquad.a2 * y2;
                (x2, x1, y2, y1) = (x1, x, y1, direct);
                let y = svf.tick(x, &mix);
                // −200 dB.
                assert!((y - direct).abs() < 1e-10, "{biquad:?} sample {n}");
            }
        }
    }

    #[test]
    fn drive_changes_the_states_but_not_the_response() {
        for biquad in test_biquads() {
            let plain = SvfMix::from_biquad(&biquad);
            for driven in [
                plain.driven(0.01),
                plain.constant_peak(),
                plain.driven(7.0).driven(0.3),
            ] {
                for step in 1..100 {
                    let omega = f64::from(step) * 0.031;
                    let error = (driven.response(omega) - plain.response(omega)).norm();
                    assert!(error < 1e-12 * plain.response(omega).norm().max(1.0));
                }
                let (mut a, mut b) = (Svf::<f64>::default(), Svf::<f64>::default());
                for n in 0..300 {
                    let x = (f64::from(n) * 0.9).sin() + if n == 0 { 1.0 } else { 0.0 };
                    let (y, expected) = (a.tick(x, &driven), b.tick(x, &plain));
                    assert!((y - expected).abs() < 1e-11, "sample {n}");
                }
                assert!((a.ic1 - driven.drive * b.ic1).abs() < 1e-11);
            }
        }
    }

    /// Peak output of a +24 dB bell on a sine at its centre while its Q
    /// falls from 18 to 0.1 within 10 ms, as a multiple of the settled peak.
    fn overshoot_while_q_falls(constant_peak: bool) -> f64 {
        let designer = crate::MatchedDesigner::new(48_000.0);
        let mix = |q: f64| {
            let prototype = crate::Prototype::bell(10.0_f64.powf(24.0 / 40.0), q);
            let mix = SvfMix::from_biquad(&designer.design(&prototype, 1000.0).0);
            if constant_peak {
                mix.constant_peak()
            } else {
                mix
            }
        };
        let mut ramp = SvfMixRamp::new(mix(18.0));
        let mut svf = Svf::<f64>::default();
        let (mut settled, mut peak) = (0.0_f64, 0.0_f64);
        for n in 0..96_000 {
            if (48_000..48_480).contains(&n) && n % 32 == 0 {
                // Fifteen steps, even in log Q.
                let q = 18.0 * (0.1_f64 / 18.0).powf(f64::from(n - 48_000 + 32) / 480.0);
                ramp.aim(mix(q), 32);
            }
            let x = (std::f64::consts::TAU * 1000.0 * f64::from(n) / 48_000.0).sin();
            let y = svf.tick(x, ramp.advance()).abs();
            if (24_000..48_000).contains(&n) {
                settled = settled.max(y);
            }
            peak = peak.max(y);
        }
        peak / settled
    }

    #[test]
    fn constant_peak_keeps_a_bell_settled_while_its_q_moves() {
        // The gain at the centre is 24 dB at any Q, so nothing should happen.
        let plain = overshoot_while_q_falls(false);
        assert!(plain > 4.0, "{plain}");
        let constant_peak = overshoot_while_q_falls(true);
        assert!(constant_peak < 1.05, "{constant_peak}");
    }

    #[test]
    fn identity_mix_passes_the_input_bit_for_bit() {
        let mut svf = Svf::<f64>::default();
        let identity = SvfMix::identity();
        let bypassed = SvfMix::from_biquad(&test_biquads()[0]).bypassed();
        assert!(identity.is_identity() && bypassed.is_identity());
        for n in 0..100 {
            let x = (f64::from(n) * 0.37).sin();
            assert_eq!(svf.tick(x, &identity), x);
            assert_eq!(svf.tick(x, &bypassed), x);
        }
    }

    /// The worst case of an equalizer: a narrow, tall bell far below the
    /// sample rate, where a direct form in f32 loses the signal in rounding.
    #[test]
    fn a_low_resonant_bell_at_192_khz_stays_clean() {
        let rate = 192_000.0;
        let designer = crate::MatchedDesigner::new(rate);
        let prototype = crate::Prototype::bell(10.0_f64.powf(24.0 / 40.0), 18.0);
        let (biquad, _) = designer.design(&prototype, 20.0);
        let mix = SvfMix::from_biquad(&biquad);
        let mut svf = Svf::<f64>::default();
        let period = 9600;
        let omega = std::f64::consts::TAU / f64::from(period);
        // Twenty seconds to settle, then ten periods to measure.
        let settle = 20 * 192_000;
        let (mut sine, mut cosine) = (0.0, 0.0);
        let mut output = Vec::with_capacity(10 * period as usize);
        for n in 0..settle + 10 * period {
            let phase = omega * f64::from(n % period);
            let y = svf.tick(phase.sin(), &mix);
            if n >= settle {
                sine += y * phase.sin();
                cosine += y * phase.cos();
                output.push(y);
            }
        }
        let scale = 2.0 / output.len() as f64;
        let (sine, cosine) = (sine * scale, cosine * scale);
        let amplitude = sine.hypot(cosine);
        let gain_db = 20.0 * amplitude.log10();
        assert!((gain_db - 24.0).abs() < 0.01, "{gain_db}");
        let residual = output
            .iter()
            .enumerate()
            .map(|(n, y)| {
                let phase = omega * n as f64;
                (y - sine * phase.sin() - cosine * phase.cos()).abs()
            })
            .fold(0.0, f64::max);
        let dbc = 20.0 * (residual / amplitude).log10();
        assert!(dbc < -120.0, "{dbc} dBc");
    }

    #[test]
    fn ramp_lands_exactly_and_keeps_its_cache_consistent() {
        let start = SvfMix::<f64>::new(0.1, 1.4, 1.0, 0.0, 0.0);
        let target = SvfMix::<f64>::new(0.7, 0.05, 0.3, -1.0, 2.5).driven(0.5);
        let mut ramp = SvfMixRamp::new(start);
        assert!(ramp.done());
        ramp.aim(target, 32);
        let mut previous = start.g;
        for step in 1..=32 {
            let mix = *ramp.advance();
            assert_eq!(ramp.done(), step == 32);
            assert!(mix.g > previous, "moves every sample");
            previous = mix.g;
            // The cached terms are those of the interpolated g and k.
            let rebuilt = SvfMix {
                drive: mix.drive,
                ..SvfMix::new(mix.g, mix.k, mix.m0, mix.m1, mix.m2)
            };
            assert_eq!(mix, rebuilt);
            if step == 16 {
                assert!((mix.g - 0.4).abs() < 1e-12 && (mix.m2 - 2.5).abs() < 1e-12);
                assert!((mix.drive - 0.75).abs() < 1e-12);
            }
        }
        assert_eq!(*ramp.mix(), target);
        assert_eq!(*ramp.advance(), target);
        ramp.snap(start);
        assert!(ramp.done());
        assert_eq!(*ramp.mix(), start);
        assert_eq!(*ramp.target(), start);
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

    /// Gain of a settled sine through `section`, by correlation.
    fn shelf_gain(section: &mut Shelf1, frequency: f64, rate: f64) -> f64 {
        let frames = (rate * (0.2 + 20.0 / frequency)) as usize;
        let skip = frames / 2;
        let (mut s, mut c, mut reference) = (0.0, 0.0, 0.0);
        for n in 0..frames {
            let phase = std::f64::consts::TAU * frequency * n as f64 / rate;
            let y = f64::from(section.process(phase.sin() as f32));
            if n >= skip {
                s += y * phase.sin();
                c += y * phase.cos();
                reference += phase.sin() * phase.sin();
            }
        }
        s.hypot(c) / reference
    }

    #[test]
    fn shelf_and_its_inverse_are_the_identity() {
        for rate in [44_100.0_f32, 48_000.0, 96_000.0] {
            for (zero, ratio) in [(720.0_f32, 1.585), (1500.0, 1.413), (3183.0, 2.512)] {
                let shelf = Shelf1::shelf(zero, zero * ratio, rate);
                let inverse = Shelf1::inverse_shelf(zero, zero * ratio, rate);
                for step in 0..=200 {
                    let omega = std::f64::consts::PI * f64::from(step) / 200.0;
                    let product = shelf.response(omega) * inverse.response(omega);
                    assert!(
                        (product - Complex::ONE).norm() < 1e-6,
                        "{rate} {zero} {omega}"
                    );
                }
                // DC stays; the top rises by the ratio of the warped
                // frequencies.
                assert!((shelf.response(0.0).norm() - 1.0).abs() < 1e-6);
                let warped = |hz: f32| (std::f64::consts::PI * f64::from(hz / rate)).tan();
                let rise = warped(zero * ratio) / warped(zero);
                let top = shelf.response(std::f64::consts::PI).norm();
                assert!((top / rise - 1.0).abs() < 1e-3, "{top} against {rise}");
                // And in the time domain, on noise.
                let (mut shelf, mut inverse) = (shelf, inverse);
                let mut state = 1u32;
                for _ in 0..4000 {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let x = (state >> 8) as f32 / (1 << 23) as f32 - 1.0;
                    let y = inverse.process(shelf.process(x));
                    assert!((y - x).abs() < 1e-4, "{y} against {x}");
                }
            }
        }
    }

    #[test]
    fn tilt_pivots_at_its_frequency() {
        let rate = 48_000.0;
        for gain_db in [-6.0_f32, -2.0, 3.0, 6.0] {
            let tilt = Shelf1::tilt(gain_db, 1000.0, rate);
            let db = |frequency: f64| {
                let omega = std::f64::consts::TAU * frequency / f64::from(rate);
                20.0 * tilt.response(omega).norm().log10()
            };
            assert!(db(1000.0).abs() < 0.05, "{gain_db}: {}", db(1000.0));
            assert!((db(1.0) + f64::from(gain_db)).abs() < 0.05);
            // Bilinear warping pulls the top up to full gain early; at the
            // analog response 20 kHz would still be a little short of it.
            let analog = {
                let a = 10.0_f64.powf(f64::from(gain_db) / 20.0);
                let x = 20.0;
                20.0 * ((1.0 / a).hypot(x) / (1.0_f64).hypot(x / a)).log10()
            };
            assert!((db(20_000.0) - analog).abs() < 0.05, "{gain_db}");
            assert!(
                (db(20_000.0) - f64::from(gain_db)).abs() < 0.05,
                "{gain_db}"
            );
            // The measured gain is the drawn one.
            let mut section = tilt.clone();
            let measured = 20.0 * shelf_gain(&mut section, 5000.0, f64::from(rate)).log10();
            assert!((measured - db(5000.0)).abs() < 0.01);
        }
    }

    #[test]
    fn high_pass_blocks_dc_and_settles_to_exact_zero() {
        let rate = 48_000.0;
        let mut blocker = Shelf1::high_pass(5.0, rate);
        let gain = 20.0 * shelf_gain(&mut blocker, 20.0, f64::from(rate)).log10();
        assert!((-0.3..0.0).contains(&gain), "{gain}");
        blocker.reset();
        let mut last = 1.0;
        for _ in 0..48_000 {
            last = blocker.process(1.0);
        }
        // An f32 pole this low stops moving 80 dB below the step.
        assert!(last.abs() < 2e-4, "{last}");
        for _ in 0..(2.0 * rate) as usize {
            last = blocker.process(0.0);
        }
        assert_eq!(last, 0.0);
    }

    #[test]
    fn low_shelf_and_low_pass_share_the_pole() {
        let rate = 48_000.0;
        let shelf = Shelf1::low_shelf(1.5, 90.0, rate);
        let db = |omega: f64| 20.0 * shelf.response(omega).norm().log10();
        assert!((db(0.0) - 1.5).abs() < 1e-4);
        assert!(db(3.0).abs() < 1e-3);
        let mut section = Shelf1::low_shelf(0.0, 120.0, rate);
        let mut low = 0.0;
        for _ in 0..48_000 {
            low = section.low(1.0);
        }
        assert!((low - 1.0).abs() < 1e-5);
        // `set` changes the coefficients and keeps the state.
        section.set(&Shelf1::high_pass(120.0, rate));
        assert!(section.process(1.0).abs() < 1e-4);
        let identity = Shelf1::default();
        assert!((identity.response(1.0) - Complex::ONE).norm() < 1e-12);
    }
}
