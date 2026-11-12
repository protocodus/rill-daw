use super::butterflies;
use crate::{DelayLine, FadeTap};

/// One step of a feed-forward multichannel diffuser: every channel is
/// delayed by its own time, then the channels are shuffled, some inverted,
/// and mixed by a rotation. A cascade of steps multiplies the number of
/// echoes by `N` per step and, having no feedback, cannot ring.
#[derive(Clone, Debug)]
pub struct DiffuserStep<const N: usize> {
    lines: [DelayLine; N],
    taps: [FadeTap; N],
    /// Channel `i` of the output comes from channel `permutation[i]`.
    permutation: [usize; N],
    signs: [f32; N],
}

impl<const N: usize> DiffuserStep<N> {
    pub fn new(permutation: [usize; N], signs: [f32; N]) -> Self {
        Self {
            lines: std::array::from_fn(|_| DelayLine::default()),
            taps: [FadeTap::default(); N],
            permutation,
            signs,
        }
    }

    /// Room for delays of `max_delay` samples; a change of delay crossfades
    /// over `fade` samples. Call from `prepare` only.
    pub fn allocate(&mut self, max_delay: usize, fade: u32) {
        for (line, tap) in self.lines.iter_mut().zip(&mut self.taps) {
            line.allocate(max_delay);
            tap.set_fade(fade);
        }
    }

    /// Each channel's delay in samples (0 is none); the reads crossfade
    /// there.
    pub fn set_delays(&mut self, samples: &[u32; N]) {
        for ((tap, line), samples) in self.taps.iter_mut().zip(&self.lines).zip(samples) {
            tap.set((*samples as usize).min(line.max_delay() as usize));
        }
    }

    /// Clears the lines and ends the crossfades.
    pub fn reset(&mut self) {
        for (line, tap) in self.lines.iter_mut().zip(&mut self.taps) {
            line.reset();
            tap.snap(tap.target());
        }
    }

    /// One sample. `cos` and `sin` are of the mixing angle: 0 leaves the
    /// channels apart, π/4 mixes each into all.
    #[inline]
    pub fn process(&mut self, input: &[f32; N], cos: f32, sin: f32) -> [f32; N] {
        let mut delayed = [0.0; N];
        for index in 0..N {
            self.lines[index].write(input[index]);
            delayed[index] = self.taps[index].read(&self.lines[index]);
        }
        let mut mixed: [f32; N] =
            std::array::from_fn(|index| self.signs[index] * delayed[self.permutation[index]]);
        butterflies(&mut mixed, cos, sin);
        mixed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Random;

    fn step() -> DiffuserStep<8> {
        let mut step = DiffuserStep::new(
            [3, 6, 1, 4, 7, 2, 5, 0],
            [1.0, -1.0, -1.0, 1.0, 1.0, 1.0, -1.0, -1.0],
        );
        step.allocate(64, 8);
        step.set_delays(&[0, 5, 9, 14, 20, 27, 33, 40]);
        step.reset();
        step
    }

    #[test]
    fn a_step_keeps_the_energy_and_spreads_an_impulse_over_all_channels() {
        let (sin, cos) = std::f32::consts::FRAC_PI_4.sin_cos();
        let mut step = step();
        let mut random = Random::new(21);
        let (mut energy_in, mut energy_out) = (0.0_f64, 0.0_f64);
        for n in 0..2000 {
            let input: [f32; 8] =
                std::array::from_fn(|_| if n < 1900 { random.bipolar() } else { 0.0 });
            let output = step.process(&input, cos, sin);
            energy_in += input.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
            energy_out += output.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        }
        assert!((energy_out / energy_in - 1.0).abs() < 1e-5);

        let mut step = self::step();
        let mut arrivals = [0; 8];
        for n in 0..64 {
            let mut input = [0.0; 8];
            if n == 0 {
                input[2] = 1.0;
            }
            for (count, value) in arrivals.iter_mut().zip(step.process(&input, cos, sin)) {
                if value != 0.0 {
                    assert_eq!(n, 9);
                    assert!((value.abs() - 8.0_f32.sqrt().recip()).abs() < 1e-6);
                    *count += 1;
                }
            }
        }
        assert_eq!(arrivals, [1; 8]);
    }

    #[test]
    fn without_mixing_the_channels_stay_apart_and_delays_crossfade() {
        let mut step = step();
        let mut input = [0.0; 8];
        input[5] = 1.0;
        let mut heard = Vec::new();
        for n in 0..64 {
            let output = step.process(&input, 1.0, 0.0);
            input = [0.0; 8];
            for (channel, value) in output.iter().enumerate() {
                if *value != 0.0 {
                    heard.push((n, channel, *value));
                }
            }
        }
        // Channel 5 is delayed by 27 samples and leaves as channel 6.
        assert_eq!(heard, [(27, 6, -1.0)]);
        // A constant passes a change of delay unchanged.
        for _ in 0..64 {
            step.process(&[1.0; 8], 1.0, 0.0);
        }
        step.set_delays(&[40, 33, 27, 20, 14, 9, 5, 0]);
        for _ in 0..20 {
            let output = step.process(&[1.0; 8], 1.0, 0.0);
            assert!(output.iter().all(|value| (value.abs() - 1.0).abs() < 1e-6));
        }
    }
}
