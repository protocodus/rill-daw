//! Shimmer: the reverb's output an octave up, fed back into its input, so
//! every pass climbs another octave until Damping and Color take it.
use super::super::{DelayLine, Svf, flush};

/// The grain: long enough for a low note's cycles, short enough not to
/// smear an attack.
const WINDOW_SECONDS: f32 = 0.04;
/// Below this nothing climbs (it would turn to mud an octave up), and above
/// the other the octave would fold back over half the sample rate.
const LOW_HZ: f32 = 180.0;
const HIGH_HZ: f32 = 5000.0;
/// Loop gain at full Shimmer: each pass a little quieter than the last, so
/// the climb stays a tail and does not run away.
pub(crate) const FULL_GAIN: f32 = 0.6;

/// An octave-up pitch shifter on two grains per channel, with its band
/// limits. Reading twice as fast as writing moves the read one whole sample
/// nearer to the write each sample, so it needs no interpolation.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shimmer {
    lines: [DelayLine; 2],
    window: usize,
    /// Position in the window, per channel: the right runs half a window
    /// apart from the left, so the grain seams do not coincide.
    phase: [usize; 2],
    /// `sin²` over the window: the two grains' gains add to 1.
    gains: Vec<f32>,
    low: [Svf; 2],
    high: [[Svf; 2]; 2],
    low_g: f32,
    high_g: f32,
}

impl Shimmer {
    pub fn prepare(&mut self, sample_rate: f32) {
        self.window = ((WINDOW_SECONDS * sample_rate) as usize).max(16);
        for line in &mut self.lines {
            line.allocate(self.window);
        }
        let window = self.window as f32;
        self.gains = (0..self.window)
            .map(|index| (std::f32::consts::PI * index as f32 / window).sin().powi(2))
            .collect();
        let warp = |hz: f32| (std::f32::consts::PI * (hz / sample_rate).min(0.45)).tan();
        self.low_g = warp(LOW_HZ);
        self.high_g = warp(HIGH_HZ.min(0.2 * sample_rate));
        self.reset();
    }

    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.reset();
        }
        self.phase = [0, self.window / 2];
        for filter in self.low.iter_mut().chain(self.high.iter_mut().flatten()) {
            filter.reset();
        }
    }

    /// One sample of the reverb's output in, its octave above out.
    #[inline]
    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        let half = self.window / 2;
        std::array::from_fn(|channel| {
            let k = std::f32::consts::SQRT_2;
            let mut band = self.low[channel]
                .process(input[channel], self.low_g, k)
                .high;
            for filter in &mut self.high[channel] {
                band = filter.process(band, self.high_g, k).low;
            }
            let line = &mut self.lines[channel];
            line.write(flush(band));
            let first = self.phase[channel];
            let second = (first + half) % self.window;
            self.phase[channel] = (first + 1) % self.window;
            // A grain starts at the far end of the window and meets the
            // write at its end, where its gain has fallen to 0.
            let grain = |at: usize| self.gains[at] * line.tap(self.window - 1 - at);
            grain(first) + grain(second)
        })
    }
}
