//! Allpass sections: first-order stages for phasers, alone or as a chain
//! with zero-delay feedback, and the delay-line allpass of reverb diffusers.
use super::{Complex, DelayLine, OnePole, flush, tanh_fast};

/// First-order allpass on a [`OnePole`]: `2·low − input`, the bilinear
/// `(a + z⁻¹)/(1 + a·z⁻¹)` with `a = (g − 1)/(g + 1)`. Unit gain at every
/// frequency; the phase turns from 0 to −180° and passes −90° at the corner.
#[derive(Clone, Debug, Default)]
pub struct Allpass1 {
    pole: OnePole,
}

impl Allpass1 {
    /// `big_g` is `g/(1 + g)` with `g` from [`super::prewarp`].
    #[inline]
    pub fn process(&mut self, input: f32, big_g: f32) -> f32 {
        2.0 * self.pole.low(input, big_g) - input
    }

    /// With the integrator input soft-clipped, as the transconductance stage
    /// of an analog phaser does. `drive` scales the signal into the curve
    /// and back out, so small signals keep unit gain.
    #[inline]
    pub fn process_saturated(&mut self, input: f32, big_g: f32, drive: f32) -> f32 {
        let state = self.pole.state;
        let v = big_g * tanh_fast(drive * (input - state)) / drive;
        let low = v + state;
        self.pole.state = flush(low + v);
        2.0 * low - input
    }

    /// The coefficient `a` of the stage for an integrator gain `g`.
    pub fn coefficient(g: f64) -> f64 {
        (g - 1.0) / (g + 1.0)
    }

    /// Response of a stage with coefficient `a` at `omega` radians per
    /// sample.
    pub fn response(a: f64, omega: f64) -> Complex {
        let z = Complex::unit(-omega);
        (Complex::real(a) + z) / (Complex::ONE + z.scale(a))
    }
}

/// Up to `N` [`Allpass1`] stages in series inside a feedback loop that is
/// solved exactly, without a unit delay: the resonant peaks of a phaser stay
/// where the stages put them, at any frequency.
#[derive(Clone, Debug)]
pub struct AllpassChain<const N: usize> {
    stages: [Allpass1; N],
}

impl<const N: usize> Default for AllpassChain<N> {
    fn default() -> Self {
        Self {
            stages: std::array::from_fn(|_| Allpass1::default()),
        }
    }
}

impl<const N: usize> AllpassChain<N> {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The chain's input for this sample: `x + feedback·y` with `y` the
    /// output the linear chain will give for it.
    #[inline]
    fn loop_input(&self, x: f32, big_g: &[f32; N], count: usize, feedback: f32) -> f32 {
        // Each stage is `c·input + e·state` within a sample, so the chain is
        // `p·input + s`.
        let (mut p, mut s) = (1.0, 0.0);
        for (stage, big_g) in self.stages[..count].iter().zip(big_g) {
            let c = 2.0 * big_g - 1.0;
            p *= c;
            s = c * s + 2.0 * (1.0 - big_g) * stage.pole.state;
        }
        // |p| ≤ 1 and |feedback| < 1 keep the divisor away from zero.
        x + feedback * (p * x + s) / (1.0 - feedback * p)
    }

    /// One sample through the first `count` stages with `feedback` (below 1
    /// in size) from the output to the input. `big_g` holds each stage's
    /// `g/(1 + g)`.
    #[inline]
    pub fn process(&mut self, x: f32, big_g: &[f32; N], count: usize, feedback: f32) -> f32 {
        let count = count.min(N);
        let mut signal = self.loop_input(x, big_g, count, feedback);
        for (stage, big_g) in self.stages[..count].iter_mut().zip(big_g) {
            signal = stage.process(signal, *big_g);
        }
        signal
    }

    /// [`Self::process`] with saturating stages (see
    /// [`Allpass1::process_saturated`]). The loop is solved for the linear
    /// chain, then the stages saturate.
    #[inline]
    pub fn process_saturated(
        &mut self,
        x: f32,
        big_g: &[f32; N],
        count: usize,
        feedback: f32,
        drive: f32,
    ) -> f32 {
        let count = count.min(N);
        let mut signal = self.loop_input(x, big_g, count, feedback);
        for (stage, big_g) in self.stages[..count].iter_mut().zip(big_g) {
            signal = stage.process_saturated(signal, *big_g, drive);
        }
        signal
    }

    /// Response `A/(1 − feedback·A)` of a chain of stages with the
    /// coefficients `a` (see [`Allpass1::coefficient`]).
    pub fn response(a: &[f64], feedback: f64, omega: f64) -> Complex {
        let chain = a.iter().fold(Complex::ONE, |product, a| {
            product * Allpass1::response(*a, omega)
        });
        chain / (Complex::ONE - chain.scale(feedback))
    }
}

/// Schroeder allpass around its own delay line:
/// `(−g + z^−M)/(1 − g·z^−M)`. Smears a transient into a burst of echoes
/// without colouring the spectrum: the diffuser in front of a reverb.
#[derive(Clone, Debug, Default)]
pub struct DelayAllpass {
    line: DelayLine,
    length: usize,
}

impl DelayAllpass {
    /// Room for `max_length` samples. Call from `prepare` only.
    pub fn allocate(&mut self, max_length: usize) {
        self.line.allocate(max_length);
        self.length = self.length.clamp(1, max_length.max(1));
    }

    /// The delay `M` in samples, at least 1.
    pub fn set_length(&mut self, length: usize) {
        self.length = length.clamp(1, (self.line.max_delay() as usize).max(1));
    }

    pub fn reset(&mut self) {
        self.line.reset();
    }

    /// One sample with the gain `g` (below 1 in size).
    #[inline]
    pub fn process(&mut self, input: f32, g: f32) -> f32 {
        let delayed = self.line.tap(self.length - 1);
        let w = input + g * delayed;
        self.line.write(flush(w));
        delayed - g * w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Random, prewarp};

    const RATE: f64 = 96_000.0;

    fn big_g(frequency: f32) -> f32 {
        let g = prewarp(frequency, RATE as f32);
        g / (1.0 + g)
    }

    /// Amplitude of a settled sine at `frequency` through `process`.
    fn gain(frequency: f64, mut process: impl FnMut(f32) -> f32) -> f64 {
        const WINDOW: i32 = 24_000;
        let (mut sine, mut cosine) = (0.0, 0.0);
        for n in 0..96_000 {
            let phase = std::f64::consts::TAU * frequency * f64::from(n) / RATE;
            let y = f64::from(process(phase.sin() as f32));
            if n >= 96_000 - WINDOW {
                sine += y * phase.sin();
                cosine += y * phase.cos();
            }
        }
        2.0 * sine.hypot(cosine) / f64::from(WINDOW)
    }

    #[test]
    fn a_chain_without_feedback_has_unit_gain_at_any_corner() {
        for corner in [50.0, 800.0, 7000.0, 20_000.0] {
            for frequency in [100.0, 1000.0, 9000.0] {
                let mut chain = AllpassChain::<6>::default();
                let coefficients = [big_g(corner); 6];
                let level = gain(frequency, |x| chain.process(x, &coefficients, 6, 0.0));
                assert!(
                    (level - 1.0).abs() < 1e-3,
                    "{corner} Hz at {frequency}: {level}"
                );
            }
        }
    }

    #[test]
    fn notches_of_the_mix_sit_where_the_stages_put_them() {
        let corner = 900.0_f32;
        let g = f64::from(prewarp(corner, RATE as f32));
        for stages in [2, 4, 8] {
            let coefficients = [big_g(corner); 8];
            for k in 0..stages / 2 {
                let angle = (2 * k + 1) as f64 * std::f64::consts::PI / (2 * stages) as f64;
                let frequency = (g * angle.tan()).atan() * RATE / std::f64::consts::PI;
                let mut chain = AllpassChain::<8>::default();
                let level = gain(frequency, |x| {
                    0.5 * (x + chain.process(x, &coefficients, stages, 0.0))
                });
                assert!(
                    level < 2e-3,
                    "{stages} stages, notch {k} at {frequency}: {level}"
                );
                // The response function says the same.
                let a = vec![Allpass1::coefficient(g); stages];
                let omega = std::f64::consts::TAU * frequency / RATE;
                let wet = AllpassChain::<8>::response(&a, 0.0, omega);
                assert!((Complex::ONE + wet).norm() < 1e-9);
            }
        }
    }

    #[test]
    fn the_feedback_loop_is_solved_exactly() {
        let mut random = Random::new(9);
        let coefficients: [f32; 12] =
            std::array::from_fn(|index| big_g(300.0 + 700.0 * index as f32));
        for feedback in [0.7_f32, -0.95, 0.95] {
            let mut chain = AllpassChain::<12>::default();
            let mut reference = AllpassChain::<12>::default();
            let mut peak = 0.0_f32;
            for _ in 0..3000 {
                let x = random.bipolar();
                let y = chain.process(x, &coefficients, 12, feedback);
                // Brute force: iterate the loop on a copy until it settles.
                let mut guess = 0.0;
                for _ in 0..600 {
                    let mut trial = reference.clone();
                    let mut signal = x + feedback * guess;
                    for (stage, big_g) in trial.stages.iter_mut().zip(&coefficients) {
                        signal = stage.process(signal, *big_g);
                    }
                    guess = signal;
                }
                let mut signal = x + feedback * guess;
                for (stage, big_g) in reference.stages.iter_mut().zip(&coefficients) {
                    signal = stage.process(signal, *big_g);
                }
                assert!(
                    (y - signal).abs() < 1e-5 * (1.0 + signal.abs()),
                    "{y} {signal}"
                );
                peak = peak.max(y.abs());
            }
            // Resonant, but bounded: at most 1/(1 − |feedback|).
            assert!(peak < 21.0, "{feedback}: {peak}");
        }
    }

    #[test]
    fn feedback_response_matches_a_measurement() {
        let (corner, feedback, stages) = (1200.0_f32, 0.6_f32, 4);
        let coefficients = [big_g(corner); 4];
        let a = [Allpass1::coefficient(f64::from(prewarp(corner, RATE as f32))); 4];
        for frequency in [300.0, 1200.0, 2500.0, 8000.0] {
            let mut chain = AllpassChain::<4>::default();
            let level = gain(frequency, |x| {
                chain.process(x, &coefficients, stages, feedback)
            });
            let omega = std::f64::consts::TAU * frequency / RATE;
            let expected = AllpassChain::<4>::response(&a, f64::from(feedback), omega).norm();
            assert!(
                (level / expected - 1.0).abs() < 2e-3,
                "{frequency}: {level} {expected}"
            );
        }
    }

    #[test]
    fn saturated_stages_are_linear_for_small_signals_and_bounded_for_large() {
        let coefficients = [big_g(700.0); 4];
        let mut clean = AllpassChain::<4>::default();
        let mut analog = AllpassChain::<4>::default();
        let mut random = Random::new(4);
        for _ in 0..5000 {
            let x = 1e-3 * random.bipolar();
            let a = clean.process(x, &coefficients, 4, 0.5);
            let b = analog.process_saturated(x, &coefficients, 4, 0.5, 0.5);
            assert!((a - b).abs() < 1e-6);
        }
        let mut peak = 0.0_f32;
        for _ in 0..200_000 {
            let y = analog.process_saturated(4.0 * random.bipolar(), &coefficients, 4, 0.95, 0.5);
            peak = peak.max(y.abs());
        }
        assert!(peak.is_finite() && peak < 200.0, "{peak}");
    }

    #[test]
    fn delay_allpass_is_flat_and_rings_as_its_formula_says() {
        let mut allpass = DelayAllpass::default();
        allpass.allocate(600);
        allpass.set_length(7);
        let g = 0.7;
        let response: Vec<f32> = (0..22)
            .map(|n| allpass.process(if n == 0 { 1.0 } else { 0.0 }, g))
            .collect();
        // −g, then (1 − g²)·g^(k−1) every M samples.
        for (n, value) in response.iter().enumerate() {
            let expected = match n {
                0 => -g,
                7 => 1.0 - g * g,
                14 => (1.0 - g * g) * g,
                21 => (1.0 - g * g) * g * g,
                _ => 0.0,
            };
            assert!((value - expected).abs() < 1e-6, "{n}: {value}");
        }
        allpass.reset();
        allpass.set_length(431);
        let mut random = Random::new(2);
        let (mut energy_in, mut energy_out) = (0.0_f64, 0.0_f64);
        for n in 0..400_000 {
            let x = if n < 300_000 { random.bipolar() } else { 0.0 };
            let y = allpass.process(x, g);
            energy_in += f64::from(x).powi(2);
            energy_out += f64::from(y).powi(2);
        }
        let difference = 10.0 * (energy_out / energy_in).log10();
        assert!(difference.abs() < 0.01, "{difference} dB");
    }
}
