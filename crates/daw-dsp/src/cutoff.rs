use super::{LinearRamp, Smoothed, prewarp};

/// The integrator gain of a filter whose corner follows a target without
/// zipper noise: the frequency moves through a one-pole, and the gain (a
/// tangent) is recomputed every [`Self::TICK`] samples and ramped between.
#[derive(Clone, Debug)]
pub struct CutoffGlide {
    hz: Smoothed,
    g: LinearRamp,
    /// The frequency `g` is heading for.
    applied: f32,
    sample_rate: f32,
    tick: u32,
}

impl CutoffGlide {
    pub const TICK: u32 = 32;

    pub fn new(hz: f32) -> Self {
        Self {
            hz: Smoothed::new(hz),
            g: LinearRamp::new(prewarp(hz, 48_000.0)),
            applied: hz,
            sample_rate: 48_000.0,
            tick: 0,
        }
    }

    /// Sets the smoothing time and jumps to the target.
    pub fn prepare(&mut self, milliseconds: f32, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.hz.set_time(milliseconds, sample_rate);
        self.snap();
    }

    pub fn set(&mut self, hz: f32) {
        self.hz.set_target(hz);
    }

    pub fn target(&self) -> f32 {
        self.hz.target()
    }

    pub fn snap(&mut self) {
        self.hz.snap();
        self.applied = self.hz.value();
        self.g = LinearRamp::new(prewarp(self.applied, self.sample_rate));
        self.tick = 0;
    }

    /// Whether the gain has arrived at the target's.
    pub fn is_settled(&self) -> bool {
        self.g.settled() && self.applied == self.hz.target()
    }

    /// The gain for the next sample.
    #[inline]
    pub fn next_g(&mut self) -> f32 {
        if self.is_settled() {
            return self.g.value();
        }
        let hz = self.hz.next_value();
        if self.tick == 0 {
            self.applied = hz;
            self.g.set_target(prewarp(hz, self.sample_rate), Self::TICK);
        }
        self.tick = (self.tick + 1) % Self::TICK;
        self.g.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glides_to_the_target_and_lands_on_its_exact_gain() {
        let rate = 48_000.0;
        let mut glide = CutoffGlide::new(100.0);
        glide.prepare(20.0, rate);
        assert!(glide.is_settled());
        assert_eq!(glide.next_g(), prewarp(100.0, rate));
        glide.set(5000.0);
        assert!(!glide.is_settled());
        let mut previous = glide.next_g();
        let mut largest = 0.0_f32;
        let mut steps = 1;
        while !glide.is_settled() {
            let g = glide.next_g();
            // Rising, but for the rounding of a ramp that lands exactly.
            assert!(g >= previous - 1e-6);
            largest = largest.max(g - previous);
            previous = g;
            steps += 1;
            assert!(steps < 48_000);
        }
        assert_eq!(previous, prewarp(5000.0, rate));
        assert_eq!(glide.target(), 5000.0);
        // No step is a jump: the whole move is 0.33.
        assert!(largest < 0.002, "{largest}");
        // About the set time, and a few ticks to land.
        assert!((600..6000).contains(&steps), "{steps}");
    }
}
