//! Memoryless shapers and first-order antiderivative anti-aliasing (ADAA):
//! instead of sampling a shaper, average it between consecutive input
//! samples, which removes most of the aliasing a nonlinearity folds back.
//! Run it inside an [`crate::Oversampler2x`] for the rest.

/// A memoryless curve and its integral.
pub trait Shape {
    fn value(&self, x: f64) -> f64;
    /// Any function whose derivative is [`Self::value`].
    fn antiderivative(&self, x: f64) -> f64;
}

/// First-order ADAA around a [`Shape`]. State and arithmetic are `f64`: the
/// output is a difference of nearly equal antiderivatives.
#[derive(Clone, Debug)]
pub struct Adaa1<S: Shape> {
    shape: S,
    previous: f64,
    previous_integral: f64,
}

impl<S: Shape> Adaa1<S> {
    pub fn new(shape: S) -> Self {
        let previous_integral = shape.antiderivative(0.0);
        Self {
            shape,
            previous: 0.0,
            previous_integral,
        }
    }

    pub fn reset(&mut self) {
        self.previous = 0.0;
        self.previous_integral = self.shape.antiderivative(0.0);
    }

    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        let integral = self.shape.antiderivative(input);
        let difference = input - self.previous;
        // The quotient is ill-conditioned for nearly equal samples; the
        // shaper at their midpoint is its limit.
        let output = if difference.abs() >= 1e-6 {
            (integral - self.previous_integral) / difference
        } else {
            self.shape.value(0.5 * (input + self.previous))
        };
        self.previous = input;
        self.previous_integral = integral;
        output
    }
}

/// The colour of a soft clipper without its limiting, as a residual to add
/// to the untouched signal: `r(x) = −(x/12)·v/(1 + v)²` with `v = (x/knee)²`.
///
/// Below the knee it is the cubic term of a `tanh` stage (`−x³/48` for a knee
/// of 2); above, it fades out again, so it never exceeds 1/48 of the signal
/// (at the knee) however hard the stage is driven. A residual that kept
/// growing would have to be time-aligned with the signal it is added to.
#[derive(Clone, Copy, Debug)]
pub struct BoundedCubic {
    pub knee: f64,
}

impl Shape for BoundedCubic {
    #[inline]
    fn value(&self, x: f64) -> f64 {
        let v = (x / self.knee).powi(2);
        -(x / 12.0) * v / ((1.0 + v) * (1.0 + v))
    }

    #[inline]
    fn antiderivative(&self, x: f64) -> f64 {
        let v = (x / self.knee).powi(2);
        // ln(1 + v) + 1/(1 + v) − 1, written without the cancellation.
        -(self.knee * self.knee / 24.0) * (v.ln_1p() - v / (1.0 + v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPE: BoundedCubic = BoundedCubic { knee: 2.0 };

    #[test]
    fn antiderivative_differentiates_to_the_shape() {
        for step in -400..=400 {
            let x = f64::from(step) * 0.05;
            let h = 1e-5;
            let slope = (SHAPE.antiderivative(x + h) - SHAPE.antiderivative(x - h)) / (2.0 * h);
            assert!((slope - SHAPE.value(x)).abs() < 1e-9, "{x}");
        }
    }

    #[test]
    fn residual_is_bounded_at_any_level() {
        let mut largest = 0.0_f64;
        let mut largest_share = 0.0_f64;
        let mut x = 1e-6;
        while x < 1e6 {
            let r = SHAPE.value(x);
            assert_eq!(SHAPE.value(-x), -r, "odd");
            largest = largest.max(r.abs());
            largest_share = largest_share.max((r / x).abs());
            x *= 1.001;
        }
        assert!(largest <= 0.0542 && largest > 0.054, "{largest}");
        assert!(
            largest_share <= 1.0 / 48.0 + 1e-12 && largest_share > 0.0208,
            "{largest_share}"
        );
        // The share peaks at the knee, and small signals see the cubic.
        assert!((SHAPE.value(2.0) / 2.0 + 1.0 / 48.0).abs() < 1e-15);
        assert!((SHAPE.value(0.01) + 0.01_f64.powi(3) / 48.0).abs() < 1e-10);
        assert!(SHAPE.value(1e6).abs() < 1e-6);
    }

    #[test]
    fn constant_input_takes_the_midpoint_branch() {
        for level in [-3.0, -0.2, 0.0, 0.7, 2.0, 50.0] {
            let mut adaa = Adaa1::new(SHAPE);
            adaa.process(level);
            for _ in 0..4 {
                assert_eq!(adaa.process(level), SHAPE.value(level));
            }
        }
    }

    #[test]
    fn slow_ramp_reproduces_the_shape() {
        let mut adaa = Adaa1::new(SHAPE);
        let step = 1e-4;
        adaa.process(-6.0);
        for n in 1..120_000 {
            let x = -6.0 + f64::from(n) * step;
            let y = adaa.process(x);
            // The average over one step, which is the value at its middle
            // up to the curvature.
            assert!((y - SHAPE.value(x - 0.5 * step)).abs() < 1e-9, "{x}");
        }
    }

    #[test]
    fn zero_in_gives_exact_zero_out_and_reset_forgets() {
        let mut adaa = Adaa1::new(SHAPE);
        for _ in 0..8 {
            assert_eq!(adaa.process(0.0), 0.0);
        }
        adaa.process(3.0);
        adaa.reset();
        assert_eq!(adaa.process(0.0), 0.0);
        let mut fresh = Adaa1::new(SHAPE);
        assert_eq!(adaa.process(1.5), fresh.process(1.5));
    }
}
