//! Allpass sections: first-order stages for phasers, alone or as a chain
//! with zero-delay feedback, and the delay-line allpass of reverb diffusers.
use super::{
    AllpassInterp, Complex, DelayLine, FadeTap, OnePole, Shelf1, ShelfGlide, SincTable, flush,
    glide_length, tanh_fast,
};

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
/// without colouring the spectrum: the diffuser in front of a reverb. With
/// `g = 0` it is a plain delay of `M` samples, exactly.
#[derive(Clone, Debug, Default)]
pub struct DelayAllpass {
    line: DelayLine,
    tap: FadeTap,
}

impl DelayAllpass {
    /// Room for `max_length` samples. Call from `prepare` only.
    pub fn allocate(&mut self, max_length: usize) {
        self.line.allocate(max_length);
        self.snap_length(self.length());
    }

    /// How long a change of length crossfades; none by default.
    pub fn set_fade(&mut self, samples: u32) {
        self.tap.set_fade(samples);
    }

    fn clamped(&self, length: usize) -> usize {
        length.clamp(1, (self.line.max_delay() as usize).max(1))
    }

    /// The delay `M` in samples, at least 1; the read crossfades there.
    pub fn set_length(&mut self, length: usize) {
        self.tap.set(self.clamped(length) - 1);
    }

    pub fn snap_length(&mut self, length: usize) {
        self.tap.snap(self.clamped(length) - 1);
    }

    /// The length it is at or fading to.
    pub fn length(&self) -> usize {
        self.tap.target() + 1
    }

    /// Clears the line and ends a fade.
    pub fn reset(&mut self) {
        self.line.reset();
        self.tap.snap(self.tap.target());
    }

    /// What the line held `samples` writes ago, for taps inside the delay.
    #[inline]
    pub fn tap(&self, samples: usize) -> f32 {
        self.line.tap(samples)
    }

    /// One sample with the gain `g` (below 1 in size).
    #[inline]
    pub fn process(&mut self, input: f32, g: f32) -> f32 {
        let delayed = self.tap.read(&self.line);
        let w = input + g * delayed;
        self.line.write(flush(w));
        delayed - g * w
    }

    /// [`Self::process`] at a `length` in samples that need not be whole
    /// (at least 1): for a length that glides, which bends the pitch of
    /// what the line holds where a crossfade would comb it. A whole length
    /// reads exactly; the fade of [`Self::set_length`] plays no part.
    #[inline]
    pub fn process_at(&mut self, input: f32, g: f32, length: f64, sinc: &SincTable) -> f32 {
        let place = (length - 1.0).clamp(0.0, f64::from(self.line.max_delay()) - 1.0);
        let whole = place as usize;
        let frac = (place - whole as f64) as f32;
        let delayed = if frac == 0.0 {
            self.line.tap(whole)
        } else if whole >= SincTable::REACH {
            self.line.read_sinc(whole, frac, sinc)
        } else {
            // Too short for the sinc's reach.
            let near = self.line.tap(whole);
            near + frac * (self.line.tap(whole + 1) - near)
        };
        let w = input + g * delayed;
        self.line.write(flush(w));
        delayed - g * w
    }
}

/// A Schroeder allpass with an absorption filter on its delayed signal
/// (Dahl and Jot): `(−g + A·z^−M)/(1 − g·A·z^−M)`. Inside a loop it loses
/// what a delay of `M` samples with the same filter loses, whatever `g` is,
/// so a ring of these decays as its filters say. With `g = 0` it is an
/// absorbent delay. The length glides (bending the pitch of what it holds)
/// or crossfades, and the read can be modulated without loss.
#[derive(Clone, Debug, Default)]
pub struct AbsorbentAllpass {
    line: DelayLine,
    absorb: ShelfGlide,
    tap: FadeTap,
    interp: AllpassInterp,
    /// In samples; equals the target when at rest.
    length: f64,
    target: u32,
    glide: f64,
}

impl AbsorbentAllpass {
    const GLIDE_SECONDS: f64 = 0.1;
    /// Samples of length per sample.
    pub const FASTEST_GLIDE: f64 = 0.5;

    /// Room for `max_length` samples. Call from `prepare` only.
    pub fn allocate(&mut self, max_length: usize, sample_rate: f32) {
        self.line.allocate(max_length);
        self.glide = 1.0 - (-1.0 / (Self::GLIDE_SECONDS * f64::from(sample_rate))).exp();
        self.snap_to(self.target);
    }

    pub fn set_fade(&mut self, samples: u32) {
        self.tap.set_fade(samples);
    }

    fn clamped(&self, length: u32) -> u32 {
        length.clamp(2, (0.98 * self.line.max_delay()).max(2.0) as u32)
    }

    pub fn snap_to(&mut self, length: u32) {
        self.target = self.clamped(length);
        self.length = f64::from(self.target);
        self.tap.snap(self.target as usize - 1);
    }

    /// Slides to a new length; what the line holds is repitched on the way.
    pub fn glide_to(&mut self, length: u32) {
        if self.tap.is_fading() {
            // One kind of move at a time.
            self.fade_to(length);
        } else {
            self.target = self.clamped(length);
        }
    }

    /// Crossfades to a new length: no pitch, and safe in a loop, since a
    /// linear blend of two taps never exceeds the larger.
    pub fn fade_to(&mut self, length: u32) {
        let gliding = self.is_gliding();
        self.target = self.clamped(length);
        if !gliding {
            self.length = f64::from(self.target);
            self.tap.set(self.target as usize - 1);
        }
    }

    /// The length it is at or moving to.
    pub fn length(&self) -> u32 {
        self.target
    }

    /// The length now, on the way to [`Self::length`].
    pub fn current_length(&self) -> f64 {
        self.length
    }

    /// The length that is heard now, for an absorption that follows it:
    /// [`Self::current_length`], or what the two taps of a crossfade
    /// amount to.
    pub fn heard_length(&self) -> f64 {
        if self.is_fading() {
            f64::from(self.tap.place()) + 1.0
        } else {
            self.length
        }
    }

    pub fn is_gliding(&self) -> bool {
        self.length != f64::from(self.target)
    }

    pub fn is_fading(&self) -> bool {
        !self.is_gliding() && self.tap.is_fading()
    }

    /// Moves to the filter's coefficients over `samples` (at once with 0)
    /// and keeps its state.
    pub fn set_absorption(&mut self, filter: &Shelf1, samples: u32) {
        self.absorb.set(filter, samples);
    }

    /// Clears the audio state and ends any move.
    pub fn reset(&mut self) {
        self.line.reset();
        self.absorb.reset();
        self.interp.reset();
        self.snap_to(self.target);
    }

    /// The delay line, for taps inside the element.
    pub fn line(&self) -> &DelayLine {
        &self.line
    }

    /// The delayed, absorbed signal for this sample: the first half of
    /// [`Self::process`], for an element whose input depends on its own
    /// output. `offset` moves the read by that many samples (`None` at
    /// rest, where the read is a whole tap).
    #[inline]
    pub fn read(&mut self, offset: Option<f32>) -> f32 {
        let target = f64::from(self.target);
        let moved = f64::from(offset.unwrap_or(0.0));
        let split = |position: f64| {
            let position = position.max(1.0);
            let whole = position.floor();
            (whole as usize, (position - whole) as f32)
        };
        let raw = if self.length != target {
            let (whole, frac) = split(self.length - 1.0 + moved);
            self.length = glide_length(self.length, target, self.glide, Self::FASTEST_GLIDE);
            if self.length == target {
                self.tap.snap(self.target as usize - 1);
            }
            self.line.read_at(whole, frac)
        } else if self.tap.is_fading() {
            let whole = moved.floor();
            self.tap
                .read_offset(&self.line, whole as isize, (moved - whole) as f32)
        } else if offset.is_some() {
            let (whole, frac) = split(target - 1.0 + moved);
            self.interp.read(&self.line, whole, frac)
        } else {
            self.tap.read(&self.line)
        };
        self.interp.follow(raw);
        self.absorb.process(raw)
    }

    /// The second half: stores the input and gives the output, for what
    /// [`Self::read`] returned.
    #[inline]
    pub fn write(&mut self, input: f32, delayed: f32, g: f32) -> f32 {
        let w = input + g * delayed;
        self.line.write(flush(w));
        delayed - g * w
    }

    /// One sample with the gain `g` (below 1 in size).
    #[inline]
    pub fn process(&mut self, input: f32, g: f32, offset: Option<f32>) -> f32 {
        let delayed = self.read(offset);
        self.write(input, delayed, g)
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

    #[test]
    fn delay_allpass_without_gain_is_a_delay_and_its_length_crossfades() {
        let mut allpass = DelayAllpass::default();
        allpass.allocate(64);
        allpass.snap_length(9);
        assert_eq!(allpass.length(), 9);
        let mut random = Random::new(4);
        let input: Vec<f32> = (0..200).map(|_| random.bipolar()).collect();
        for (n, x) in input.iter().enumerate() {
            let y = allpass.process(*x, 0.0);
            assert_eq!(y, if n >= 9 { input[n - 9] } else { 0.0 });
        }
        assert_eq!(allpass.tap(3), input[196]);
        // A constant stays one through a crossfade to another length.
        allpass.reset();
        allpass.set_fade(16);
        for _ in 0..40 {
            allpass.process(1.0, 0.0);
        }
        allpass.set_length(30);
        assert_eq!(allpass.length(), 30);
        for _ in 0..40 {
            assert!((allpass.process(1.0, 0.0) - 1.0).abs() < 1e-6);
        }
    }

    /// Energy out over energy in, in dB, for a burst of noise.
    fn energy_db(mut process: impl FnMut(f32) -> f32) -> f64 {
        let mut random = Random::new(2);
        let (mut energy_in, mut energy_out) = (0.0_f64, 0.0_f64);
        for n in 0..400_000 {
            let x = if n < 300_000 { random.bipolar() } else { 0.0 };
            let y = process(x);
            energy_in += f64::from(x).powi(2);
            energy_out += f64::from(y).powi(2);
        }
        10.0 * (energy_out / energy_in).log10()
    }

    #[test]
    fn absorbent_allpass_is_flat_at_rest_and_modulated() {
        let mut allpass = AbsorbentAllpass::default();
        allpass.allocate(2000, 48_000.0);
        allpass.snap_to(431);
        let still = energy_db(|x| allpass.process(x, 0.7, None));
        assert!(still.abs() < 0.01, "{still} dB");
        allpass.reset();
        // A read that moves by 20 samples either way at 1 Hz.
        let mut n = 0;
        let moving = energy_db(|x| {
            n += 1;
            let offset = 20.0 * (std::f32::consts::TAU * n as f32 / 48_000.0).sin();
            allpass.process(x, 0.7, Some(offset))
        });
        assert!(moving.abs() < 0.05, "{moving} dB");
    }

    #[test]
    fn a_ring_of_absorbent_elements_decays_as_its_filters_say() {
        // An allpass of 100 samples and a delay of 150, each losing 6.02 dB
        // per 100 samples of its length: so does the ring, at any gain.
        for g in [0.0, 0.5, 0.7] {
            let mut allpass = AbsorbentAllpass::default();
            let mut delay = AbsorbentAllpass::default();
            for (element, length) in [(&mut allpass, 100), (&mut delay, 150)] {
                element.allocate(400, 48_000.0);
                element.snap_to(length);
                let gain = 0.5_f32.powf(length as f32 / 100.0);
                element.set_absorption(&Shelf1::with_gains(1.0, gain, gain), 0);
            }
            let mut energy = [0.0_f64; 2];
            for n in 0..5000 {
                let end = delay.read(None);
                let input = end + if n == 0 { 1.0 } else { 0.0 };
                let signal = allpass.process(input, g, None);
                delay.write(signal, end, 0.0);
                if (1000..3000).contains(&n) {
                    energy[0] += f64::from(signal).powi(2);
                } else if n >= 3000 {
                    energy[1] += f64::from(signal).powi(2);
                }
            }
            let fall = 10.0 * (energy[0] / energy[1]).log10();
            // The few modes of so short a ring beat: a window's energy
            // wanders by a decibel or two around the slope.
            assert!((fall - 120.4).abs() < 3.0, "g {g}: {fall} dB per 2000");
        }
    }

    #[test]
    fn absorbent_allpass_glides_at_a_bounded_rate_and_crossfades() {
        let mut allpass = AbsorbentAllpass::default();
        allpass.allocate(8000, 48_000.0);
        allpass.set_fade(480);
        allpass.snap_to(1000);
        let mut n = 0;
        let mut largest_step = |allpass: &mut AbsorbentAllpass, samples: usize| {
            let (mut previous, mut largest) = (0.0_f32, 0.0_f32);
            let mut previous_length = allpass.current_length();
            for index in 0..samples {
                let x = 0.5 * (0.02 * n as f32).sin();
                n += 1;
                let y = allpass.process(x, 0.6, None);
                if index > 0 {
                    largest = largest.max((y - previous).abs());
                }
                previous = y;
                let length = allpass.current_length();
                assert!((length - previous_length).abs() <= AbsorbentAllpass::FASTEST_GLIDE);
                previous_length = length;
            }
            largest
        };
        let steady = largest_step(&mut allpass, 20_000);
        allpass.glide_to(6000);
        assert!(allpass.is_gliding());
        let gliding = largest_step(&mut allpass, 48_000);
        assert!(!allpass.is_gliding());
        assert_eq!(allpass.length(), 6000);
        assert!(gliding <= 1.5 * steady + 0.01, "{gliding} against {steady}");
        allpass.fade_to(4500);
        assert!(!allpass.is_gliding());
        let fading = largest_step(&mut allpass, 2000);
        assert!(fading <= 1.5 * steady + 0.01, "{fading} against {steady}");
        assert_eq!(allpass.length(), 4500);
    }
}
