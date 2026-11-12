use std::ops::{Add, Div, Mul, Neg, Sub};

/// The sample type of a generic block: `f32` for instruments and most
/// effects, `f64` where coefficients near z = 1 or very resonant states need
/// the precision (equalizers, matched filters).
pub trait Real:
    Copy
    + Default
    + PartialOrd
    + std::fmt::Debug
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
{
    const ZERO: Self;
    const ONE: Self;
    const TWO: Self;

    fn from_f64(value: f64) -> Self;
    /// Zero below the type's denormal guard, so decays end in exact zeros.
    fn flush(self) -> Self;
    /// See [`crate::tanh_fast`].
    fn tanh_fast(self) -> Self;
}

impl Real for f32 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    const TWO: Self = 2.0;

    #[inline]
    fn from_f64(value: f64) -> Self {
        value as f32
    }

    #[inline]
    fn flush(self) -> Self {
        crate::flush(self)
    }

    #[inline]
    fn tanh_fast(self) -> Self {
        crate::tanh_fast(self)
    }
}

impl Real for f64 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    const TWO: Self = 2.0;

    #[inline]
    fn from_f64(value: f64) -> Self {
        value
    }

    #[inline]
    fn flush(self) -> Self {
        // Far below anything audible after conversion to f32, far above the
        // f64 denormals.
        if self.abs() < 1e-30 { 0.0 } else { self }
    }

    #[inline]
    fn tanh_fast(self) -> Self {
        let x = self.clamp(-3.0, 3.0);
        let x2 = x * x;
        x * (27.0 + x2) / (27.0 + 9.0 * x2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_widths_agree() {
        for step in -40..=40 {
            let x = f64::from(step) * 0.1;
            let wide = x.tanh_fast();
            let narrow = (x as f32).tanh_fast();
            assert!((wide - f64::from(narrow)).abs() < 1e-6, "{x}");
        }
        assert_eq!(1e-25_f32.flush(), 0.0);
        assert_eq!(1e-25_f64.flush(), 1e-25);
        assert_eq!(1e-35_f64.flush(), 0.0);
        assert_eq!(f32::from_f64(0.5), 0.5);
        assert_eq!(<f64 as Real>::TWO, f64::from(<f32 as Real>::TWO));
    }
}
