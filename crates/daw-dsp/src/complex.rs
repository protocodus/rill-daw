use std::ops::{Add, Div, Mul, Sub};

/// Minimal complex number for drawing frequency responses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const ONE: Self = Self { re: 1.0, im: 0.0 };

    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    pub fn real(re: f64) -> Self {
        Self { re, im: 0.0 }
    }

    /// e^(iθ)
    pub fn unit(theta: f64) -> Self {
        Self::new(theta.cos(), theta.sin())
    }

    pub fn norm(self) -> f64 {
        self.re.hypot(self.im)
    }

    pub fn scale(self, factor: f64) -> Self {
        Self::new(self.re * factor, self.im * factor)
    }
}

impl Add for Complex {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.re + other.re, self.im + other.im)
    }
}

impl Sub for Complex {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.re - other.re, self.im - other.im)
    }
}

impl Mul for Complex {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }
}

impl Div for Complex {
    type Output = Self;
    fn div(self, other: Self) -> Self {
        let denominator = other.re * other.re + other.im * other.im;
        Self::new(
            (self.re * other.re + self.im * other.im) / denominator,
            (self.im * other.re - self.re * other.im) / denominator,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Complex, b: Complex) -> bool {
        (a - b).norm() < 1e-12
    }

    #[test]
    fn arithmetic_follows_the_field_rules() {
        let a = Complex::new(3.0, -2.0);
        let b = Complex::new(-0.5, 4.0);
        assert!(close(a * b, Complex::new(6.5, 13.0)));
        assert!(close((a * b) / b, a));
        assert!(close(a + b - b, a));
        assert!(close(
            Complex::ONE / Complex::new(0.0, 1.0),
            Complex::new(0.0, -1.0)
        ));
        assert_eq!(Complex::new(3.0, 4.0).norm(), 5.0);
        assert!(close(a.scale(2.0), a + a));
    }

    #[test]
    fn unit_is_on_the_circle() {
        for theta in [0.0, 0.3, std::f64::consts::FRAC_PI_2, 2.5, -1.0] {
            let z = Complex::unit(theta);
            assert!((z.norm() - 1.0).abs() < 1e-12);
            assert!(close(z * Complex::unit(-theta), Complex::ONE));
        }
    }
}
