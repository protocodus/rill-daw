use super::Complex;

/// Coefficients of `(b0 + b1·z⁻¹ + b2·z⁻²) / (1 + a1·z⁻¹ + a2·z⁻²)`.
///
/// Only a description of a filter: designers produce it, drawings evaluate
/// it. Audio runs it through [`crate::SvfMix::from_biquad`], because a direct
/// form loses precision near z = 1 and is not safe under modulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Biquad {
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Response at `omega` radians per sample.
    pub fn response(&self, omega: f64) -> Complex {
        let z1 = Complex::unit(-omega);
        let z2 = z1 * z1;
        (Complex::real(self.b0) + z1.scale(self.b1) + z2.scale(self.b2))
            / (Complex::ONE + z1.scale(self.a1) + z2.scale(self.a2))
    }

    /// The filter that undoes this one: zeros and poles exchanged. Stable
    /// only if this one is minimum phase.
    pub fn inverse(&self) -> Self {
        Self {
            b0: 1.0 / self.b0,
            b1: self.a1 / self.b0,
            b2: self.a2 / self.b0,
            a1: self.b1 / self.b0,
            a2: self.b2 / self.b0,
        }
    }

    pub fn is_finite(&self) -> bool {
        [self.b0, self.b1, self.b2, self.a1, self.a2]
            .iter()
            .all(|value| value.is_finite())
    }

    /// Both poles strictly inside the unit circle.
    pub fn is_stable(&self) -> bool {
        self.a2.abs() < 1.0 && self.a1.abs() < 1.0 + self.a2
    }

    /// Both zeros strictly inside the unit circle.
    pub fn is_minimum_phase(&self) -> bool {
        self.b0 > self.b2.abs() && self.b0 + self.b2 > self.b1.abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BELL: Biquad = Biquad {
        b0: 1.2,
        b1: -1.5,
        b2: 0.5,
        a1: -1.6,
        a2: 0.7,
    };

    #[test]
    fn response_follows_the_coefficients() {
        // DC and Nyquist are plain sums.
        let dc = BELL.response(0.0);
        assert!(
            (dc.re - (1.2 - 1.5 + 0.5) / (1.0 - 1.6 + 0.7)).abs() < 1e-12 && dc.im.abs() < 1e-12
        );
        let nyquist = BELL.response(std::f64::consts::PI);
        assert!((nyquist.re - (1.2 + 1.5 + 0.5) / (1.0 + 1.6 + 0.7)).abs() < 1e-12);
        assert_eq!(Biquad::IDENTITY.response(1.0), Complex::ONE);
    }

    #[test]
    fn inverse_cancels() {
        assert!(BELL.is_stable() && BELL.is_minimum_phase());
        let inverse = BELL.inverse();
        assert!(inverse.is_stable() && inverse.is_minimum_phase());
        for step in 0..=20 {
            let omega = f64::from(step) * std::f64::consts::PI / 20.0;
            let product = BELL.response(omega) * inverse.response(omega);
            assert!((product - Complex::ONE).norm() < 1e-12, "{omega}");
        }
    }

    #[test]
    fn stability_and_phase_checks_find_the_unit_circle() {
        let pole_outside = Biquad { a2: 1.01, ..BELL };
        assert!(!pole_outside.is_stable());
        let pole_on_axis = Biquad {
            a1: -2.0,
            a2: 0.99,
            ..BELL
        };
        assert!(!pole_on_axis.is_stable());
        let zero_outside = Biquad { b2: 1.3, ..BELL };
        assert!(!zero_outside.is_minimum_phase());
        assert!(
            !Biquad {
                b0: f64::NAN,
                ..BELL
            }
            .is_finite()
        );
    }
}
