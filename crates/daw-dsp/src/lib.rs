//! Rill's shared DSP: the blocks every built-in device is made of (filters,
//! circuit-modelled oscillators, envelopes, modulators), also
//! usable by the app and engine directly (a track high-pass, an EQ band).
//! Each block is deterministic, allocation-free after construction and
//! unit-tested on its own. No other `daw-*` crate, no I/O.
#![forbid(unsafe_code)]
mod allpass;
mod biquad;
mod complex;
mod cutoff;
mod delay;
mod dynamics;
pub mod echo;
mod envelope;
pub mod filter;
mod follower;
mod lfo;
mod limiter;
pub mod matched;
pub mod modulation;
mod nonlinear;
pub mod oscillator;
mod oversample;
pub mod playback;
mod random;
mod real;
pub mod resample;
pub mod reverb;
mod shaper;
mod smooth;
mod svf;

pub use allpass::{AbsorbentAllpass, Allpass1, AllpassChain, DelayAllpass};
pub use biquad::Biquad;
pub use complex::Complex;
pub use cutoff::CutoffGlide;
pub use delay::{AllpassInterp, DelayLine, FadeTap, SincTable, StaticDelay, glide_length};
pub use dynamics::{Ballistics, Compander, CurveKind, GainComputer, ProgramRelease};
pub use echo::{
    BucketFilter, BucketLine, EchoKernel, EchoModel, EchoSettings, EchoStereo, TapPattern,
    TapeTransport,
};
pub use envelope::{Adsr, Stage};
pub use filter::{
    CleanShape, FilterKernel, FilterModel, FilterShape, FilterType, LatencyDelay, sweep_corner,
    sweep_mix,
};
pub use follower::EnvelopeFollower;
pub use lfo::{Lfo, LfoShape};
pub use limiter::{BoxSmoother, SlidingMax, TruePeak4x};
pub use matched::{BUTTERWORTH_Q, Fit, MatchedDesigner, Prototype};
pub use modulation::{ModulationKernel, ModulationMode, ModulationSettings};
pub use nonlinear::{Ladder, SallenKey, soft_limit, tanh_fast};
pub use oscillator::{Drift, Oscillator, OscillatorModel, SubOscillator, Waveform};
pub use oversample::{HalfBand, OversampleFactor, Oversampler, Oversampler2x};
pub use playback::{LoopMode, Markers, Playhead, Region};
pub use random::{Random, hash32};
pub use real::Real;
pub use resample::{MipMap, SincBank, Step};
pub use reverb::{
    DiffuserStep, Fdn, ReverbKernel, ReverbModel, ReverbSettings, absorption_shelf, butterflies,
    hadamard, nearest_prime,
};
pub use shaper::{
    Adaa1, AdaaCentered, AlgebraicSigmoid, AntialiasedShaper, BoundedCubic, BoundedSquare, Drive,
    DriveGains, DroopCorrector, HyperbolicClip, Shape,
};
pub use smooth::{LinearRamp, Smoothed};
pub use svf::{OnePole, Shelf1, ShelfGlide, Svf, SvfMix, SvfMixRamp, SvfOutputs};

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

/// `tan` for arguments up to 0.7 (error below 2e-4), cheap enough for a
/// corner that is swept every sample.
#[inline]
pub fn tan_fast(x: f32) -> f32 {
    let x2 = x * x;
    x * (15.0 - x2) / (15.0 - 6.0 * x2)
}

/// [`prewarp`] through [`tan_fast`]: exact enough up to 0.22 of the sample
/// rate, where the frequency is held.
#[inline]
pub fn prewarp_fast(frequency: f32, sample_rate: f32) -> f32 {
    tan_fast((std::f32::consts::PI * frequency / sample_rate).clamp(1e-6, 0.7))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tan_fast_follows_tan_up_to_its_limit() {
        for step in 0..=700 {
            let x = step as f32 * 0.001;
            assert!((tan_fast(x) - x.tan()).abs() < 2e-4, "{x}");
        }
        // Above the limit the frequency is held, not wrapped.
        assert_eq!(
            prewarp_fast(30_000.0, 48_000.0),
            prewarp_fast(40_000.0, 48_000.0)
        );
        let exact = prewarp(1000.0, 96_000.0);
        assert!((prewarp_fast(1000.0, 96_000.0) - exact).abs() < 1e-6);
    }
}
