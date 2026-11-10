//! Shared DSP blocks used by every device. Each block is deterministic,
//! allocation-free after construction and unit-tested on its own.
mod complex;
mod delay;
mod follower;
mod lfo;
mod nonlinear;
mod oversample;
mod random;
mod smooth;
mod svf;

pub use complex::Complex;
pub use delay::DelayLine;
pub use follower::EnvelopeFollower;
pub use lfo::{Lfo, LfoShape};
pub use nonlinear::{Ladder, SallenKey};
pub use oversample::Oversampler2x;
pub use random::Random;
pub use smooth::Smoothed;
pub use svf::{OnePole, Svf, SvfOutputs};

/// Below this, filter states are flushed to zero so decays never reach
/// denormal numbers.
pub const DENORMAL: f32 = 1e-20;

#[inline]
pub fn flush(value: f32) -> f32 {
    if value.abs() < DENORMAL { 0.0 } else { value }
}

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Prewarped integrator gain of a zero-delay-feedback filter. Frequencies are
/// held below Nyquist so the tangent stays finite.
#[inline]
pub fn prewarp(frequency: f32, sample_rate: f32) -> f32 {
    (std::f32::consts::PI * (frequency / sample_rate).clamp(1e-6, 0.49)).tan()
}
