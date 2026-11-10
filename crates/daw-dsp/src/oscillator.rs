//! Analog oscillator cores, modelled on the circuits rather than on ideal
//! waveforms, and band-limited by correcting every discontinuity.
//!
//! Both models are a capacitor ramp that a comparator resets once per cycle;
//! pulse, triangle and sine are shaped from that ramp the way the hardware
//! does it. They differ where the circuits differ:
//!
//! - **VCO** (saw-core voltage-controlled oscillator): an exponential
//!   converter sets the charge current, so pitch wanders with temperature
//!   ([`Oscillator::set_drift`]) and each core has its own small tuning error.
//!   The current source has finite output impedance, so the ramp bends
//!   slightly; that makes the triangle a little asymmetric and pulse width
//!   not quite linear. The reset takes a moment, which flattens the highest
//!   notes slightly (high-frequency tracking).
//! - **DCO** (divider-clocked oscillator): a counter on a crystal clock
//!   resets the ramp, so pitch is exact and never drifts, and the ramp is
//!   straight. The charge current comes from a DAC set in semitone steps, so
//!   the ramp's level varies a little with pitch, most audibly under pitch
//!   bend.
//!
//! Discontinuities are corrected with a four-point B-spline step (BLEP) and
//! ramp (BLAMP) residual, which suppresses aliasing far better than the
//! two-point polyBLEP at a two-sample delay. Run the oscillators oversampled
//! for alias-free audio band output.
use super::Random;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscillatorModel {
    Vco,
    Dco,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    Saw,
    Pulse,
    Triangle,
    Sine,
}

impl Waveform {
    pub const LABELS: &[&str] = &["Saw", "Pulse", "Triangle", "Sine"];

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Pulse,
            2 => Self::Triangle,
            3 => Self::Sine,
            _ => Self::Saw,
        }
    }
}

/// Ramp curvature of the VCO core: the ramp's slope falls by this fraction
/// over the cycle as the capacitor charges against the current source's
/// output impedance.
const VCO_CURVATURE: f32 = 0.12;
/// Reset time of the VCO core. The cycle is this much longer than the charge
/// time, so pitch lags slightly at the top of the range.
const VCO_RESET_SECONDS: f32 = 0.4e-6;
/// Largest wander and static tuning error at full drift, in cents.
const VCO_DRIFT_CENTS: f32 = 6.0;
const VCO_TOLERANCE_CENTS: f32 = 3.0;
/// Seconds between new drift targets; the wander is smoothed over the same
/// time, so it moves slowly like a warming circuit.
const DRIFT_PERIOD_SECONDS: f32 = 0.7;
/// Sine shaper drive: a differential pair turning the triangle into a sine,
/// with the soft residual harmonics of the analog circuit.
const SINE_SHAPER: f32 = 1.6;

/// Corrections for band-limited discontinuities, applied to a short window
/// around each event. Delays the signal by [`Corrector::LATENCY`] samples.
#[derive(Clone, Debug, Default)]
pub struct Corrector {
    /// Samples n−2 … n+1 around the sample being produced.
    pending: [f32; 4],
}

impl Corrector {
    pub const LATENCY: u32 = 2;

    pub fn reset(&mut self) {
        self.pending = [0.0; 4];
    }

    /// A jump of `height` that happened `before` (0–1) samples before the
    /// current sample.
    #[inline]
    pub fn step(&mut self, before: f32, height: f32) {
        for (index, slot) in self.pending.iter_mut().enumerate() {
            *slot += height * blep(index as f32 - 2.0 + before);
        }
    }

    /// A slope change of `change` per sample, `before` samples ago.
    #[inline]
    pub fn ramp(&mut self, before: f32, change: f32) {
        for (index, slot) in self.pending.iter_mut().enumerate() {
            *slot += change * blamp(index as f32 - 2.0 + before);
        }
    }

    /// Adds the current sample's naive value and returns the corrected sample
    /// from [`Self::LATENCY`] samples ago.
    #[inline]
    pub fn push(&mut self, naive: f32) -> f32 {
        let [first, second, third, fourth] = self.pending;
        self.pending = [second, third + naive, fourth, 0.0];
        first
    }
}

/// Integrated cubic B-spline minus the unit step, at `x` samples from the
/// step. Zero outside ±2.
#[inline]
fn blep(x: f32) -> f32 {
    if x < -1.0 {
        let t = 2.0 + x;
        t * t * t * t / 24.0
    } else if x < 0.0 {
        let x2 = x * x;
        0.5 + x * (2.0 / 3.0) - x2 * x / 3.0 - x2 * x2 / 8.0
    } else if x < 1.0 {
        let x2 = x * x;
        -(0.5 - x * (2.0 / 3.0) + x2 * x / 3.0 - x2 * x2 / 8.0)
    } else {
        let t = 2.0 - x;
        -t * t * t * t / 24.0
    }
}

/// Twice-integrated cubic B-spline minus the unit ramp: the residual of a
/// slope change. Even, zero outside ±2.
#[inline]
fn blamp(x: f32) -> f32 {
    let y = x.abs();
    if y < 1.0 {
        let y2 = y * y;
        7.0 / 30.0 - y / 2.0 + y2 / 3.0 - y2 * y2 / 12.0 + y2 * y2 * y / 40.0
    } else {
        let t = (2.0 - y).max(0.0);
        t * t * t * t * t / 120.0
    }
}

/// The sine shaper and its slope, a `tanh`-like differential pair normalized
/// to ±1 at the triangle's peaks.
#[inline]
fn sine_shape(x: f32) -> f32 {
    shaper(SINE_SHAPER * x) / shaper(SINE_SHAPER)
}

#[inline]
fn sine_slope(x: f32) -> f32 {
    let u = SINE_SHAPER * x;
    let denominator = 27.0 + 9.0 * u * u;
    SINE_SHAPER * 9.0 * (9.0 - u * u).powi(2) / (denominator * denominator) / shaper(SINE_SHAPER)
}

/// [`super::tanh_fast`] without the clamp; only called within ±1.6.
#[inline]
fn shaper(x: f32) -> f32 {
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

/// Slowly wandering value in −1–1, for component drift. Call at a steady
/// rate; seeded, so it repeats after `reset`.
#[derive(Clone, Debug)]
pub struct Drift {
    random: Random,
    value: f32,
    target: f32,
    coefficient: f32,
    interval: u32,
    countdown: u32,
}

impl Drift {
    pub fn new(seed: u32) -> Self {
        Self {
            random: Random::new(seed),
            value: 0.0,
            target: 0.0,
            coefficient: 0.0,
            interval: 1,
            countdown: 0,
        }
    }

    /// `rate` is how often [`Self::next_value`] is called per second.
    pub fn prepare(&mut self, rate: f32) {
        let calls = (DRIFT_PERIOD_SECONDS * rate).max(1.0);
        self.interval = calls as u32;
        self.coefficient = 1.0 - (-1.0 / calls).exp();
        self.reset();
    }

    pub fn reset(&mut self) {
        self.random.reset();
        // Start somewhere along the walk, not at the center.
        self.value = self.random.bipolar();
        self.target = self.value;
        self.countdown = 0;
    }

    #[inline]
    pub fn next_value(&mut self) -> f32 {
        if self.countdown == 0 {
            self.countdown = self.interval;
            self.target = self.random.bipolar();
        }
        self.countdown -= 1;
        self.value += (self.target - self.value) * self.coefficient;
        self.value
    }
}

/// One oscillator core. Call [`Self::set_frequency`] at control rate and
/// [`Self::process`] once per sample.
#[derive(Clone, Debug)]
pub struct Oscillator {
    model: OscillatorModel,
    sample_rate: f32,
    phase: f32,
    increment: f32,
    increment_step: f32,
    steps_left: u32,
    /// DCO ramp level from the semitone-stepped charge current, ramped
    /// like the increment so pitch bends do not step it.
    level: f32,
    level_step: f32,
    drift_amount: f32,
    drift: Drift,
    /// This core's static tuning error, −1–1 of the tolerance.
    tolerance: f32,
    random: Random,
    corrector: Corrector,
}

impl Oscillator {
    pub fn new(seed: u32) -> Self {
        let mut oscillator = Self {
            model: OscillatorModel::Vco,
            sample_rate: 96_000.0,
            phase: 0.0,
            increment: 0.0,
            increment_step: 0.0,
            steps_left: 0,
            level: 1.0,
            level_step: 0.0,
            drift_amount: 0.0,
            drift: Drift::new(seed ^ 0x5A5A_1234),
            tolerance: 0.0,
            random: Random::new(seed),
            corrector: Corrector::default(),
        };
        oscillator.reset();
        oscillator
    }

    /// `sample_rate` is the rate [`Self::process`] runs at; `control_rate`
    /// how often [`Self::set_frequency`] is called.
    pub fn prepare(&mut self, sample_rate: f32, control_rate: f32) {
        self.sample_rate = sample_rate;
        self.drift.prepare(control_rate);
        self.reset();
    }

    /// Free-running cores start each run at their own seeded phase.
    pub fn reset(&mut self) {
        self.random.reset();
        self.phase = (self.random.bipolar() + 1.0) * 0.5 % 1.0;
        self.tolerance = self.random.bipolar();
        self.increment = 0.0;
        self.increment_step = 0.0;
        self.steps_left = 0;
        self.level = 1.0;
        self.level_step = 0.0;
        self.drift.reset();
        self.corrector.reset();
    }

    pub fn set_model(&mut self, model: OscillatorModel) {
        self.model = model;
    }

    /// 0–1. Only the VCO drifts.
    pub fn set_drift(&mut self, amount: f32) {
        self.drift_amount = amount.clamp(0.0, 1.0);
    }

    /// Glides to `frequency` (Hz) over `samples` calls of [`Self::process`].
    pub fn set_frequency(&mut self, frequency: f32, samples: u32) {
        let wander = self.drift.next_value();
        let frequency = frequency.max(0.0);
        let mut level = 1.0;
        let target = match self.model {
            OscillatorModel::Vco => {
                let cents = self.drift_amount
                    * (VCO_DRIFT_CENTS * wander + VCO_TOLERANCE_CENTS * self.tolerance);
                let tuned = frequency * (cents / 1200.0).exp2();
                tuned / (1.0 + tuned * VCO_RESET_SECONDS)
            }
            OscillatorModel::Dco => {
                // The DAC holds the current of the nearest semitone; the ramp
                // overshoots or falls short by the remainder.
                let semitones = 12.0 * (frequency.max(1.0) / 440.0).log2();
                level = ((semitones.round() - semitones) / 12.0).exp2();
                frequency
            }
        };
        // Below Nyquist, so at most one reset per sample.
        let target = (target / self.sample_rate).min(0.45);
        let samples = samples.max(1);
        if self.increment == 0.0 {
            self.increment = target;
            self.level = level;
            self.steps_left = 0;
        } else {
            self.increment_step = (target - self.increment) / samples as f32;
            self.level_step = (level - self.level) / samples as f32;
            self.steps_left = samples;
        }
    }

    /// The charge on the timing capacitor, 0–1 over a cycle, and its slope.
    #[inline]
    fn charge(&self, phase: f32) -> (f32, f32) {
        match self.model {
            OscillatorModel::Vco => (
                phase + VCO_CURVATURE * phase * (1.0 - phase),
                1.0 + VCO_CURVATURE * (1.0 - 2.0 * phase),
            ),
            OscillatorModel::Dco => (phase, 1.0),
        }
    }

    /// Phase at which the charge reaches `level` (0–1).
    #[inline]
    fn phase_at(&self, level: f32) -> f32 {
        match self.model {
            OscillatorModel::Vco => {
                let k = VCO_CURVATURE;
                let b = 1.0 + k;
                (b - (b * b - 4.0 * k * level).max(0.0).sqrt()) / (2.0 * k)
            }
            OscillatorModel::Dco => level,
        }
    }

    /// Waveform value and slope (per cycle) at `phase`, away from breakpoints.
    #[inline]
    fn shape(&self, waveform: Waveform, phase: f32, width: f32) -> (f32, f32) {
        let (c, slope) = self.charge(phase);
        match waveform {
            Waveform::Saw => (self.level * (2.0 * c - 1.0), self.level * 2.0 * slope),
            Waveform::Pulse => (if c < width { 1.0 } else { -1.0 }, 0.0),
            Waveform::Triangle => {
                let (value, slope) = triangle(c, slope);
                (self.level * value, self.level * slope)
            }
            Waveform::Sine => {
                let (value, slope) = triangle(c, slope);
                (
                    self.level * sine_shape(value),
                    self.level * sine_slope(value) * slope,
                )
            }
        }
    }

    /// Corrects the breakpoints the phase crosses between `from` and `to`
    /// (unwrapped, `to - from` below one cycle). `at_sample` is the phase the
    /// current sample falls on.
    #[inline]
    fn correct(
        &mut self,
        waveform: Waveform,
        width: f32,
        from: f32,
        to: f32,
        at_sample: f32,
        increment: f32,
    ) {
        let edge = match waveform {
            Waveform::Saw => None,
            Waveform::Pulse => Some(self.phase_at(width)),
            Waveform::Triangle | Waveform::Sine => Some(self.phase_at(0.5)),
        };
        for point in [Some(1.0), edge, edge.map(|e| e + 1.0)]
            .into_iter()
            .flatten()
        {
            if !(from < point && point <= to) {
                continue;
            }
            let before = ((at_sample - point) / increment).clamp(0.0, 1.0);
            let wrapped = point % 1.0;
            let left = self.shape(
                waveform,
                if wrapped == 0.0 { 1.0 } else { wrapped - 1e-6 },
                width,
            );
            let right = self.shape(waveform, wrapped + 1e-6, width);
            let (jump, kink) = match waveform {
                // The edge is exact; the shape evaluated beside it is not.
                Waveform::Pulse => (if wrapped == 0.0 { 2.0 } else { -2.0 }, 0.0),
                Waveform::Saw => (-2.0 * self.level, right.1 - left.1),
                _ => (0.0, right.1 - left.1),
            };
            if jump != 0.0 {
                self.corrector.step(before, jump);
            }
            if kink != 0.0 {
                self.corrector.ramp(before, kink * increment);
            }
        }
    }

    /// One sample. `width` is the pulse width (0–1). `sync` resets the core,
    /// that many samples (0–1) before this one. Returns the sample and, when
    /// this core reset, how far before this sample it did, for syncing
    /// another core or clocking a [`SubOscillator`].
    #[inline]
    pub fn process(
        &mut self,
        waveform: Waveform,
        width: f32,
        sync: Option<f32>,
    ) -> (f32, Option<f32>) {
        if self.steps_left > 0 {
            self.increment += self.increment_step;
            self.level += self.level_step;
            self.steps_left -= 1;
        }
        let increment = self.increment;
        let width = width.clamp(0.02, 0.98);
        let start = self.phase;
        let end = start + increment;
        let mut wrapped = None;
        if let Some(before) = sync.filter(|_| increment > 0.0) {
            let before = before.clamp(0.0, 1.0);
            let reset_at = start + increment * (1.0 - before);
            self.correct(waveform, width, start, reset_at, end, increment);
            let wrapped_at = reset_at % 1.0;
            let left = self.shape(waveform, wrapped_at, width);
            let right = self.shape(waveform, 0.0, width);
            self.corrector.step(before, right.0 - left.0);
            self.corrector.ramp(before, (right.1 - left.1) * increment);
            self.phase = increment * before;
            self.correct(waveform, width, 0.0, self.phase, self.phase, increment);
        } else {
            self.correct(waveform, width, start, end, end, increment);
            if end >= 1.0 {
                wrapped = Some(((end - 1.0) / increment.max(1e-9)).clamp(0.0, 1.0));
            }
            self.phase = end % 1.0;
        }
        let naive = self.shape(waveform, self.phase, width).0;
        (self.corrector.push(naive), wrapped)
    }
}

/// Triangle from the ramp by full-wave rectification, and its slope.
#[inline]
fn triangle(charge: f32, slope: f32) -> (f32, f32) {
    if charge < 0.5 {
        (4.0 * charge - 1.0, 4.0 * slope)
    } else {
        (3.0 - 4.0 * charge, -4.0 * slope)
    }
}

/// Square one octave below its clock: a flip-flop toggled by every reset of
/// the oscillator it follows, so it stays phase-locked to it.
#[derive(Clone, Debug)]
pub struct SubOscillator {
    state: f32,
    corrector: Corrector,
}

impl Default for SubOscillator {
    fn default() -> Self {
        Self {
            state: 1.0,
            corrector: Corrector::default(),
        }
    }
}

impl SubOscillator {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `clock` is the reset position [`Oscillator::process`] returned.
    #[inline]
    pub fn process(&mut self, clock: Option<f32>) -> f32 {
        if let Some(before) = clock {
            self.corrector.step(before, -2.0 * self.state);
            self.state = -self.state;
        }
        self.corrector.push(self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Complex;

    const RATE: f32 = 96_000.0;

    fn render(
        model: OscillatorModel,
        waveform: Waveform,
        frequency: f32,
        frames: usize,
    ) -> Vec<f32> {
        let mut oscillator = Oscillator::new(7);
        oscillator.set_model(model);
        oscillator.prepare(RATE, RATE / 16.0);
        oscillator.set_frequency(frequency, 1);
        (0..frames)
            .map(|_| oscillator.process(waveform, 0.5, None).0)
            .collect()
    }

    /// Magnitude of `samples` at `frequency`, Hann-windowed, relative to a
    /// full-scale sine.
    fn magnitude(samples: &[f32], frequency: f32) -> f64 {
        let n = samples.len() as f64;
        let omega = std::f64::consts::TAU * f64::from(frequency / RATE);
        let mut sum = Complex::real(0.0);
        for (index, sample) in samples.iter().enumerate() {
            let window = 0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / n).cos();
            sum = sum + Complex::unit(-omega * index as f64).scale(window * f64::from(*sample));
        }
        sum.norm() * 4.0 / n
    }

    #[test]
    fn corrections_vanish_away_from_the_event() {
        assert_eq!(blep(-2.0), 0.0);
        assert!(blep(2.0).abs() < 1e-7);
        assert!((blep(-1e-6) - 0.5).abs() < 1e-5 && (blep(0.0) + 0.5).abs() < 1e-6);
        assert!(blamp(2.0).abs() < 1e-7);
        assert!((blamp(1.0) - 1.0 / 120.0).abs() < 1e-6);
        // Continuous where the pieces meet.
        assert!((blep(-1.0 - 1e-5) - blep(-1.0 + 1e-5)).abs() < 1e-4);
        assert!((blep(1.0 - 1e-5) - blep(1.0 + 1e-5)).abs() < 1e-4);
    }

    #[test]
    fn every_waveform_has_full_level_and_no_offset_beyond_the_curvature() {
        for model in [OscillatorModel::Vco, OscillatorModel::Dco] {
            for waveform in [
                Waveform::Saw,
                Waveform::Pulse,
                Waveform::Triangle,
                Waveform::Sine,
            ] {
                let samples = render(model, waveform, 440.0, 9600);
                let peak = samples.iter().fold(0.0_f32, |p, x| p.max(x.abs()));
                let mean = samples.iter().sum::<f32>() / samples.len() as f32;
                assert!(
                    (0.9..1.15).contains(&peak),
                    "{model:?} {waveform:?} peak {peak}"
                );
                assert!(mean.abs() < 0.08, "{model:?} {waveform:?} mean {mean}");
            }
        }
    }

    #[test]
    fn dco_is_exactly_in_tune_and_the_vco_drifts() {
        let count_cycles = |model, drift: f32| {
            let mut oscillator = Oscillator::new(3);
            oscillator.set_model(model);
            oscillator.prepare(RATE, RATE / 16.0);
            oscillator.set_drift(drift);
            let mut resets = 0;
            for frame in 0..RATE as usize {
                if frame % 16 == 0 {
                    oscillator.set_frequency(1000.0, 16);
                }
                resets += usize::from(oscillator.process(Waveform::Saw, 0.5, None).1.is_some());
            }
            resets
        };
        assert_eq!(count_cycles(OscillatorModel::Dco, 1.0), 1000);
        let vco = count_cycles(OscillatorModel::Vco, 1.0);
        assert!(vco != 1000 && vco.abs_diff(1000) < 10, "{vco}");
        // Without drift the VCO only lags by its reset time.
        assert!(count_cycles(OscillatorModel::Vco, 0.0).abs_diff(1000) <= 1);
    }

    #[test]
    fn saw_harmonics_follow_one_over_n() {
        // A semitone from A 440, where the DCO's charge current is exact.
        let fundamental = 880.0;
        let samples = render(OscillatorModel::Dco, Waveform::Saw, fundamental, 19_200);
        for harmonic in 1..=12 {
            let frequency = fundamental * harmonic as f32;
            let level = magnitude(&samples, frequency) * harmonic as f64;
            // The B-spline kernel's gentle top-octave roll-off: −2.5 dB at
            // 20 kHz when run at 96 kHz.
            let x = std::f64::consts::PI * f64::from(frequency / RATE);
            let expected = 2.0 / std::f64::consts::PI * (x.sin() / x).powi(4);
            assert!((level / expected - 1.0).abs() < 0.01, "{harmonic}: {level}");
        }
    }

    #[test]
    fn aliasing_stays_far_below_the_fundamental() {
        // A high note at 2× 48 kHz, as a synth voice runs it: everything
        // that folds back below 20 kHz is alias (the voice's downsampler
        // removes the rest). R1_DEVICES.md asks for 80 dB below the
        // fundamental up to 5 kHz.
        let fundamental = 4_700.0;
        for model in [OscillatorModel::Vco, OscillatorModel::Dco] {
            for waveform in [Waveform::Saw, Waveform::Pulse] {
                let samples = render(model, waveform, fundamental, 19_200);
                let fundamental = match model {
                    OscillatorModel::Vco => fundamental / (1.0 + fundamental * VCO_RESET_SECONDS),
                    OscillatorModel::Dco => fundamental,
                };
                let reference = magnitude(&samples, fundamental);
                let mut worst = 0.0_f64;
                for harmonic in 5..60 {
                    let frequency = fundamental * harmonic as f32;
                    let alias = (frequency % RATE).min(RATE - frequency % RATE);
                    if frequency > RATE / 2.0 && alias < 20_000.0 && alias > 200.0 {
                        worst = worst.max(magnitude(&samples, alias));
                    }
                }
                let db = 20.0 * (worst / reference).log10();
                assert!(db < -80.0, "{model:?} {waveform:?}: {db:.1} dB");
            }
        }
    }

    #[test]
    fn hard_sync_follows_the_master_without_clicks() {
        let mut master = Oscillator::new(1);
        let mut slave = Oscillator::new(2);
        for oscillator in [&mut master, &mut slave] {
            oscillator.prepare(RATE, RATE / 16.0);
            oscillator.set_model(OscillatorModel::Dco);
        }
        // A period of exactly 400 samples.
        master.set_frequency(240.0, 1);
        slave.set_frequency(537.0, 1);
        let mut slave_samples = Vec::new();
        let mut resets = 0;
        for _ in 0..9600 {
            let (_, wrap) = master.process(Waveform::Saw, 0.5, None);
            resets += usize::from(wrap.is_some());
            slave_samples.push(slave.process(Waveform::Triangle, 0.5, wrap).0);
        }
        assert_eq!(resets, 24);
        // The synced slave repeats at the master's period.
        let shift = 400;
        let error = slave_samples[2000..8000]
            .iter()
            .zip(&slave_samples[2000 + shift..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(error < 1e-3, "{error}");
        assert!(slave_samples.iter().all(|x| x.abs() < 1.3));
    }

    #[test]
    fn sub_oscillator_is_one_octave_down() {
        let mut oscillator = Oscillator::new(5);
        oscillator.prepare(RATE, RATE / 16.0);
        oscillator.set_frequency(400.0, 1);
        let mut sub = SubOscillator::default();
        let samples: Vec<f32> = (0..19_200)
            .map(|_| sub.process(oscillator.process(Waveform::Saw, 0.5, None).1))
            .collect();
        assert!(magnitude(&samples, 200.0) > 1.1);
        assert!(magnitude(&samples, 400.0) < 0.01);
    }

    #[test]
    fn drift_wanders_slowly_within_range() {
        let mut drift = Drift::new(9);
        drift.prepare(1000.0);
        let values: Vec<f32> = (0..10_000).map(|_| drift.next_value()).collect();
        assert!(values.iter().all(|v| v.abs() <= 1.0));
        let step = values
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max);
        assert!(step < 0.01, "{step}");
        let spread = values.iter().fold(0.0_f32, |m, v| m.max(*v))
            - values.iter().fold(0.0_f32, |m, v| m.min(*v));
        assert!(spread > 0.5, "{spread}");
    }
}
