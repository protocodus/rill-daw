//! Memoryless shapers and first-order antiderivative anti-aliasing (ADAA):
//! instead of sampling a shaper, average it between consecutive input
//! samples, which removes most of the aliasing a nonlinearity folds back.
//! Run it inside an [`crate::Oversampler`] for the rest.

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

/// First-order ADAA centred on the previous sample (after Vicanek): the
/// average of the shape over the half samples before and after it, along the
/// linearly interpolated input. Its delay is exactly one sample, where
/// [`Adaa1`] has half of one, so an oversampled path keeps a whole latency.
/// A linear shape comes out as `(x[n] + 6·x[n−1] + x[n−2])/8`; see
/// [`DroopCorrector`].
#[derive(Clone, Debug)]
pub struct AdaaCentered<S: Shape> {
    shape: S,
    x1: f64,
    x2: f64,
    /// Antiderivative at `x1` and at the midpoint of `x1` and `x2`.
    integral_x1: f64,
    integral_mid: f64,
}

impl<S: Shape> AdaaCentered<S> {
    pub fn new(shape: S) -> Self {
        let at_zero = shape.antiderivative(0.0);
        Self {
            shape,
            x1: 0.0,
            x2: 0.0,
            integral_x1: at_zero,
            integral_mid: at_zero,
        }
    }

    pub fn shape(&self) -> &S {
        &self.shape
    }

    /// Replaces the curve. The stored antiderivatives are taken again with
    /// the new one, so the next quotient never divides a difference of two
    /// curves by a small step.
    pub fn set_shape(&mut self, shape: S) {
        self.shape = shape;
        self.integral_x1 = self.shape.antiderivative(self.x1);
        self.integral_mid = self.shape.antiderivative(0.5 * (self.x1 + self.x2));
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.integral_x1 = self.shape.antiderivative(0.0);
        self.integral_mid = self.integral_x1;
    }

    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        let (x1, x2) = (self.x1, self.x2);
        let mid = 0.5 * (input + x1);
        let integral_mid = self.shape.antiderivative(mid);
        // The antiderivative grows with the input, so the bound below which
        // the quotient loses to rounding is relative. The midpoint rule it
        // falls back to is exact to the second order of the step.
        let bound = 1e-5 * (1.0 + x1.abs());
        let first = if (x1 - x2).abs() >= bound {
            (self.integral_x1 - self.integral_mid) / (x1 - x2)
        } else {
            0.5 * self.shape.value(0.5 * (0.5 * (x1 + x2) + x1))
        };
        let second = if (input - x1).abs() >= bound {
            (integral_mid - self.integral_x1) / (input - x1)
        } else {
            0.5 * self.shape.value(0.5 * (mid + x1))
        };
        self.x2 = x1;
        self.x1 = input;
        self.integral_x1 = self.shape.antiderivative(input);
        self.integral_mid = integral_mid;
        first + second
    }
}

/// Three-tap symmetric FIR `[−c, 1 + 2c, −c]`, one sample of delay, that
/// undoes the `(6 + 2·cos ω)/8` droop of [`AdaaCentered`] in the audio band.
#[derive(Clone, Copy, Debug, Default)]
pub struct DroopCorrector {
    pub c: f64,
    x1: f64,
    x2: f64,
}

impl DroopCorrector {
    /// The `c` that makes shaper and corrector together exactly flat at
    /// 16 kHz when they run at `rate` (the oversampled one).
    pub fn coefficient(rate: f64) -> f64 {
        // Held below Nyquist for very low rates.
        let omega = (std::f64::consts::TAU * 16_000.0 / rate).min(2.0);
        let cos = omega.cos();
        (8.0 / (6.0 + 2.0 * cos) - 1.0) / (2.0 * (1.0 - cos))
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
    }

    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        let output = (1.0 + 2.0 * self.c) * self.x1 - self.c * (input + self.x2);
        self.x2 = self.x1;
        self.x1 = input;
        output
    }
}

/// [`AdaaCentered`] and its [`DroopCorrector`]: a shaper for the inside of
/// an [`crate::Oversampler`], with the delay that keeps its latency whole.
#[derive(Clone, Debug)]
pub struct AntialiasedShaper<S: Shape> {
    adaa: AdaaCentered<S>,
    pub corrector: DroopCorrector,
}

impl<S: Shape> AntialiasedShaper<S> {
    /// In samples at the rate it runs at.
    pub const DELAY: u32 = 2;

    pub fn new(shape: S) -> Self {
        Self {
            adaa: AdaaCentered::new(shape),
            corrector: DroopCorrector::default(),
        }
    }

    pub fn shape(&self) -> &S {
        self.adaa.shape()
    }

    pub fn set_shape(&mut self, shape: S) {
        self.adaa.set_shape(shape);
    }

    pub fn reset(&mut self) {
        self.adaa.reset();
        self.corrector.reset();
    }

    #[inline]
    pub fn process(&mut self, input: f64) -> f64 {
        self.corrector.process(self.adaa.process(input))
    }
}

/// Soft saturation `g(u) = u/√(1 + (u/L)²)` with its own limit `L` on each
/// side and a bias that moves the operating point along the curve, scaled
/// and shifted so that `f(0) = 0` and `f′(0) = 1`: models built on it are
/// level matched for small signals. With a bias the curve is asymmetric and
/// makes even harmonics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlgebraicSigmoid {
    up: f64,
    down: f64,
    bias: f64,
    /// `g(bias)` and `1/g′(bias)`.
    offset: f64,
    scale: f64,
    integral_offset: f64,
}

impl AlgebraicSigmoid {
    /// `up` and `down` are the limits of `g` above and below zero (both
    /// positive); the limits of the shape itself also depend on `bias`.
    pub fn new(up: f64, down: f64, bias: f64) -> Self {
        let mut shape = Self {
            up,
            down,
            bias,
            offset: 0.0,
            scale: 1.0,
            integral_offset: 0.0,
        };
        let limit = shape.limit(bias);
        shape.offset = shape.g(bias);
        shape.scale = (1.0 + (bias / limit).powi(2)).powf(1.5);
        shape.integral_offset = shape.big_g(bias);
        shape
    }

    #[inline]
    fn limit(&self, u: f64) -> f64 {
        if u >= 0.0 { self.up } else { self.down }
    }

    #[inline]
    fn root(&self, u: f64) -> f64 {
        (1.0 + (u / self.limit(u)).powi(2)).sqrt()
    }

    #[inline]
    fn g(&self, u: f64) -> f64 {
        u / self.root(u)
    }

    /// `L²·(√(1 + (u/L)²) − 1)`, written without the cancellation; zero at
    /// zero for either limit, so it is continuous there.
    #[inline]
    fn big_g(&self, u: f64) -> f64 {
        u * u / (1.0 + self.root(u))
    }
}

impl Shape for AlgebraicSigmoid {
    #[inline]
    fn value(&self, x: f64) -> f64 {
        (self.g(x + self.bias) - self.offset) * self.scale
    }

    #[inline]
    fn antiderivative(&self, x: f64) -> f64 {
        (self.big_g(x + self.bias) - self.integral_offset - self.offset * x) * self.scale
    }
}

/// Clipper with separate limits above and below zero and an adjustable
/// corner: `h(x) = ½·[√((x + down)² + s²) − √((x − up)² + s²)]`, scaled and
/// shifted so that `f(0) = 0` and `f′(0) = 1`. As `soft` goes to zero it
/// becomes a hard clip at `+up` and `−down`; it stays analytic above that.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HyperbolicClip {
    up: f64,
    down: f64,
    soft: f64,
    /// `h(0)` and `1/h′(0)`.
    offset: f64,
    scale: f64,
}

impl HyperbolicClip {
    pub fn new(up: f64, down: f64, soft: f64) -> Self {
        let mut shape = Self {
            up,
            down,
            soft,
            offset: 0.0,
            scale: 1.0,
        };
        shape.offset = shape.h(0.0);
        shape.scale = 2.0 / (down / down.hypot(soft) + up / up.hypot(soft));
        shape
    }

    #[inline]
    fn h(&self, x: f64) -> f64 {
        let s2 = self.soft * self.soft;
        let (a, b) = (x + self.down, x - self.up);
        0.5 * ((a * a + s2).sqrt() - (b * b + s2).sqrt())
    }

    /// Antiderivative of `√(u² + s²)`.
    #[inline]
    fn p(&self, u: f64) -> f64 {
        let s = self.soft;
        0.5 * (u * (u * u + s * s).sqrt() + s * s * (u / s).asinh())
    }
}

impl Shape for HyperbolicClip {
    #[inline]
    fn value(&self, x: f64) -> f64 {
        (self.h(x) - self.offset) * self.scale
    }

    #[inline]
    fn antiderivative(&self, x: f64) -> f64 {
        (0.5 * (self.p(x + self.down) - self.p(x - self.up)) - self.offset * x) * self.scale
    }
}

/// The colour of an even-order stage as a residual to add to the untouched
/// signal: `x²/(1 + (x/knee)²)`, a square for small signals that never
/// exceeds `knee²`. It has a mean; follow it with a DC blocker.
#[derive(Clone, Copy, Debug)]
pub struct BoundedSquare {
    pub knee: f64,
}

impl Shape for BoundedSquare {
    #[inline]
    fn value(&self, x: f64) -> f64 {
        x * x / (1.0 + (x / self.knee).powi(2))
    }

    #[inline]
    fn antiderivative(&self, x: f64) -> f64 {
        self.knee * self.knee * (x - self.knee * (x / self.knee).atan())
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

    fn assert_derivative(shape: &impl Shape, range: f64, tolerance: f64) {
        for step in -400..=400 {
            let x = f64::from(step) * range / 400.0;
            let h = 1e-5 * (1.0 + x.abs());
            let slope = (shape.antiderivative(x + h) - shape.antiderivative(x - h)) / (2.0 * h);
            let value = shape.value(x);
            assert!(
                (slope - value).abs() < tolerance * (1.0 + value.abs()),
                "{x}: {slope} against {value}"
            );
        }
    }

    fn slope_at_zero(shape: &impl Shape) -> f64 {
        (shape.value(1e-6) - shape.value(-1e-6)) / 2e-6
    }

    #[test]
    fn algebraic_sigmoid_is_level_matched_and_keeps_its_limits() {
        for (up, down, bias) in [
            (1.0, 1.0, 0.0),
            (1.0, 2.0, 0.0),
            (1.0, 2.0, 0.4),
            (1.0, 2.0, 0.8),
            (1.5, 0.7, -0.3),
        ] {
            let shape = AlgebraicSigmoid::new(up, down, bias);
            assert_derivative(&shape, 30.0, 1e-7);
            assert_eq!(shape.value(0.0), 0.0);
            assert!((slope_at_zero(&shape) - 1.0).abs() < 1e-6, "{bias}");
            // The limits are those of g, moved by the bias and scaled.
            let top = (up - shape.offset) * shape.scale;
            let bottom = (-down - shape.offset) * shape.scale;
            assert!((shape.value(1e7) - top).abs() < 1e-6 * top.abs());
            assert!((shape.value(-1e7) - bottom).abs() < 1e-6 * bottom.abs());
            let mut previous = shape.value(-50.0);
            for step in -499..=500 {
                let value = shape.value(f64::from(step) * 0.1);
                assert!(value > previous, "monotone");
                previous = value;
            }
        }
        let symmetric = AlgebraicSigmoid::new(1.0, 1.0, 0.0);
        assert_eq!(symmetric.value(-0.7), -symmetric.value(0.7));
    }

    #[test]
    fn hyperbolic_clip_runs_from_soft_to_hard() {
        for soft in [1.0, 0.224, 0.05] {
            let shape = HyperbolicClip::new(1.0, 1.25, soft);
            assert_derivative(&shape, 30.0, 1e-7);
            // At ±1e4, the device's input clamp, P is of the order of 1e8.
            assert_derivative(&shape, 1e4, 1e-4);
            assert_eq!(shape.value(0.0), 0.0);
            assert!((slope_at_zero(&shape) - 1.0).abs() < 1e-6, "{soft}");
        }
        // Nearly hard: unit slope inside, flat at +up and −down outside.
        let hard = HyperbolicClip::new(1.0, 1.25, 1e-4);
        assert!((hard.value(0.5) - 0.5).abs() < 1e-6);
        assert!((hard.value(40.0) - 1.0).abs() < 1e-6);
        assert!((hard.value(-40.0) + 1.25).abs() < 1e-6);
    }

    #[test]
    fn bounded_square_is_even_and_bounded() {
        let shape = BoundedSquare { knee: 1.5 };
        assert_derivative(&shape, 30.0, 1e-7);
        assert_eq!(shape.value(0.0), 0.0);
        assert_eq!(shape.value(-0.3), shape.value(0.3));
        assert!((shape.value(1e-3) - 1e-6).abs() < 1e-11);
        let mut x = 1e-3;
        while x < 1e6 {
            assert!(shape.value(x) < 2.25);
            x *= 1.5;
        }
        assert!(shape.value(1e6) > 2.2499);
    }

    #[test]
    fn centred_adaa_delays_by_one_sample() {
        struct Linear;
        impl Shape for Linear {
            fn value(&self, x: f64) -> f64 {
                x
            }
            fn antiderivative(&self, x: f64) -> f64 {
                0.5 * x * x
            }
        }
        let mut adaa = AdaaCentered::new(Linear);
        let input = [0.0, 1.0, 1.0, -2.0, 0.5, 0.5, 0.5, 3.0];
        let mut history = [0.0; 2];
        for x in input {
            let expected = (x + 6.0 * history[0] + history[1]) / 8.0;
            assert!((adaa.process(x) - expected).abs() < 1e-12);
            history = [x, history[0]];
        }
    }

    #[test]
    fn centred_adaa_follows_a_slow_ramp_and_holds_a_constant() {
        let shape = HyperbolicClip::new(1.0, 1.25, 0.2);
        let mut adaa = AdaaCentered::new(shape);
        let step = 1e-3;
        adaa.process(-6.0);
        adaa.process(-6.0 + step);
        for n in 2..12_000 {
            let x = -6.0 + f64::from(n) * step;
            let y = adaa.process(x);
            assert!((y - shape.value(x - step)).abs() < 1e-6, "{x}");
        }
        for level in [-300.0, -0.2, 0.0, 0.7, 2.0, 9000.0] {
            let mut adaa = AdaaCentered::new(shape);
            adaa.process(level);
            adaa.process(level);
            for _ in 0..4 {
                assert_eq!(adaa.process(level), shape.value(level));
            }
        }
    }

    #[test]
    fn centred_adaa_stays_accurate_for_small_steps_at_large_inputs() {
        // Steps around the bound between quotient and midpoint rule.
        let shape = HyperbolicClip::new(1.0, 1.25, 0.05);
        for level in [0.3, 40.0, 400.0, 9000.0] {
            for step in [1e-9, 1e-6, 3e-5, 1e-3] {
                let step = step * (1.0 + level);
                let mut adaa = AdaaCentered::new(shape);
                adaa.process(level);
                adaa.process(level + step);
                let y = adaa.process(level + 2.0 * step);
                let exact = shape.value(level + step);
                assert!(
                    (y - exact).abs() < 1e-6,
                    "{level} {step}: {y} against {exact}"
                );
            }
        }
    }

    #[test]
    fn replacing_the_shape_does_not_glitch() {
        let shape = |n: i32| AlgebraicSigmoid::new(1.0, 2.0, 0.1 + f64::from(n) * 1e-4);
        let mut adaa = AdaaCentered::new(shape(0));
        let mut previous = 0.0;
        for n in 0..4000 {
            if n % 32 == 0 {
                adaa.set_shape(shape(n));
            }
            let x = 40.0 * (f64::from(n) * 0.01).sin();
            let y = adaa.process(x);
            // The average around the previous sample is close to the curve
            // there. A quotient of the old curve's antiderivative and the
            // new one's would be off by about 0.6 at these levels.
            let expected = adaa.shape().value(previous);
            assert!((y - expected).abs() < 0.05, "{n}: {y} against {expected}");
            previous = x;
        }
        adaa.reset();
        assert_eq!(adaa.process(0.0), 0.0);
        assert_eq!(adaa.process(0.0), 0.0);
    }

    #[test]
    fn corrector_flattens_the_droop() {
        for (rate, residual) in [(96_000.0, 0.12), (192_000.0, 0.03), (384_000.0, 0.03)] {
            let c = DroopCorrector::coefficient(rate);
            let gain = |frequency: f64| {
                let omega = std::f64::consts::TAU * frequency / rate;
                let droop = (6.0 + 2.0 * omega.cos()) / 8.0;
                20.0 * (droop * (1.0 + 2.0 * c * (1.0 - omega.cos()))).log10()
            };
            assert!(gain(16_000.0).abs() < 1e-9);
            for frequency in [20.0, 1000.0, 8000.0, 12_000.0, 20_000.0] {
                assert!(gain(frequency).abs() < residual, "{rate} {frequency}");
            }
        }
        assert!((DroopCorrector::coefficient(96_000.0) - 0.1429).abs() < 1e-3);
        // An impulse comes out as the three taps, one sample late.
        let mut corrector = DroopCorrector {
            c: 0.125,
            ..Default::default()
        };
        let output = [1.0, 0.0, 0.0, 0.0].map(|x| corrector.process(x));
        assert_eq!(output, [-0.125, 1.25, -0.125, 0.0]);
        // Very low rates keep a finite coefficient.
        assert!(DroopCorrector::coefficient(16_000.0).is_finite());
    }

    #[test]
    fn antialiased_shaper_is_flat_and_two_samples_late() {
        let rate = 192_000.0;
        let mut shaper = AntialiasedShaper::new(AlgebraicSigmoid::new(1.0, 1.0, 0.0));
        shaper.corrector.c = DroopCorrector::coefficient(rate);
        let frequency = 10_000.0;
        let amplitude = 1e-4;
        let mut error = 0.0_f64;
        for n in 0..2000 {
            let phase = |n: f64| std::f64::consts::TAU * frequency * n / rate;
            let y = shaper.process(amplitude * phase(f64::from(n)).sin());
            if n > 10 {
                error = error.max((y - amplitude * phase(f64::from(n) - 2.0).sin()).abs());
            }
        }
        assert!(error < 0.003 * amplitude, "{error}");
        assert_eq!(AntialiasedShaper::<BoundedSquare>::DELAY, 2);
        shaper.reset();
        assert_eq!(shaper.process(0.0), 0.0);
    }
}
