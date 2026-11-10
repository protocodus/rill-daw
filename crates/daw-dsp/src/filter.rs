//! Filter kernels shared by every device that filters: R1 Filter, the synth
//! filter slots, EQ bands. One kernel holds the state for every model, so a
//! model switch is a reset, never an allocation. Circuit models run at 2× and
//! every model is delayed to the same [`FilterKernel::LATENCY`], so models and
//! dry paths line up whatever is selected.
use super::{
    Complex, DelayLine, Ladder, Oversampler2x, SallenKey, Svf, SvfOutputs, db_to_gain, prewarp,
    tanh_fast,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterModel {
    /// Linear state-variable filter, transparent at any setting.
    Clean,
    /// Four-pole transistor ladder: saturating stages, bass thins with
    /// resonance, self-oscillates.
    TransistorLadder,
    /// Two-pole Sallen-Key with clipped positive feedback.
    SallenKey,
    /// Four-pole OTA cascade, multimode by mixing the pole outputs.
    OtaCascade,
    /// Two-pole state-variable filter built from saturating OTA integrators;
    /// at 24 dB two of them in series, both resonant.
    OtaMultimode,
    /// Two-pole multimode with a diode pair clipping the resonance path.
    DiodePair,
    /// Linear low-pass → notch → high-pass morph.
    StateVariable,
    Comb,
    Vowel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterType {
    LowPass,
    HighPass,
    BandPass,
    Notch,
}

impl FilterModel {
    /// How many of low-pass, high-pass, band-pass and notch, in that order,
    /// the model offers. 0 for the models that have no type.
    pub fn type_count(self) -> usize {
        match self {
            Self::Clean | Self::OtaCascade | Self::OtaMultimode => 4,
            Self::DiodePair => 3,
            Self::SallenKey => 2,
            _ => 0,
        }
    }

    pub fn has_slope(self) -> bool {
        matches!(self, Self::Clean | Self::OtaCascade | Self::OtaMultimode)
    }

    pub fn has_morph(self) -> bool {
        matches!(self, Self::StateVariable | Self::Comb | Self::Vowel)
    }

    fn oversampled(self) -> bool {
        matches!(
            self,
            Self::TransistorLadder
                | Self::SallenKey
                | Self::OtaCascade
                | Self::OtaMultimode
                | Self::DiodePair
        )
    }
}

/// What a kernel does, changed at block rate. Cutoff, resonance and morph
/// are per sample, so they can be modulated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterShape {
    pub model: FilterModel,
    pub filter_type: FilterType,
    /// 24 instead of 12 dB/oct, where the model has a slope.
    pub steep: bool,
    /// 0–24 dB into the filter, level-compensated.
    pub drive_db: f32,
}

fn clean_q(resonance: f32) -> f32 {
    0.707 * 35.0_f32.powf(resonance)
}

/// Butterworth stage Qs for the 24 dB low- and high-pass, with resonance on
/// the second stage.
fn steep_qs(resonance: f32) -> [f32; 2] {
    [0.541, 1.307 * 19.0_f32.powf(resonance)]
}

fn ladder_k(resonance: f32) -> f32 {
    4.1 * resonance
}

fn sallen_key_k(resonance: f32) -> f32 {
    2.05 * resonance
}

/// Weights of the loop input and the four stage outputs. Each stage output
/// is the loop input over (1 + s)^n, so binomial weights build any 12 or
/// 24 dB response over the same resonant loop.
fn pole_mix(filter_type: FilterType, steep: bool) -> [f32; 5] {
    match (filter_type, steep) {
        (FilterType::LowPass, false) => [0.0, 0.0, 1.0, 0.0, 0.0],
        (FilterType::LowPass, true) => [0.0, 0.0, 0.0, 0.0, 1.0],
        (FilterType::HighPass, false) => [1.0, -2.0, 1.0, 0.0, 0.0],
        (FilterType::HighPass, true) => [1.0, -4.0, 6.0, -4.0, 1.0],
        (FilterType::BandPass, false) => [0.0, 2.0, -2.0, 0.0, 0.0],
        (FilterType::BandPass, true) => [0.0, 0.0, 4.0, -8.0, 4.0],
        (FilterType::Notch, false) => [1.0, -2.0, 2.0, 0.0, 0.0],
        (FilterType::Notch, true) => [1.0, -4.0, 8.0, -8.0, 4.0],
    }
}

/// The four-pole low-pass loses bass as resonance rises; this restores half
/// of it in dB.
fn bass_gain(filter_type: FilterType, k: f32) -> f32 {
    if filter_type == FilterType::LowPass {
        (1.0 + k).sqrt()
    } else {
        1.0
    }
}

/// SVF damping (1/Q) of the OTA multimode: Butterworth at 0, slightly
/// negative at 100 % so the saturating loop self-oscillates.
fn ota_damping(resonance: f32) -> f32 {
    1.414 - 1.464 * resonance
}

/// SVF damping of the diode pair: two real poles at 0 (dull, as the circuit
/// is), slightly negative at 100 %.
fn diode_damping(resonance: f32) -> f32 {
    2.0 - 2.05 * resonance
}

/// Band-pass gain for a self-oscillating SVF: constant-peak while damped,
/// capped so the peak does not vanish as damping reaches 0.
fn resonant_band_gain(damping: f32) -> f32 {
    damping.max(0.25)
}

fn comb_feedback(resonance: f32) -> f32 {
    0.97 * resonance
}

/// In-loop low-pass coefficient: Morph darkens the comb.
fn comb_damping(morph: f32) -> f32 {
    1.0 - 0.9 * morph
}

fn vowel_k(resonance: f32) -> f32 {
    1.0 / (4.0 + 16.0 * resonance)
}

const VOWELS: [([f32; 3], [f32; 3]); 5] = [
    ([800.0, 1150.0, 2900.0], [0.0, -6.0, -32.0]),
    ([350.0, 2000.0, 2800.0], [0.0, -20.0, -15.0]),
    ([270.0, 2140.0, 2950.0], [0.0, -12.0, -26.0]),
    ([450.0, 800.0, 2830.0], [0.0, -11.0, -22.0]),
    ([325.0, 700.0, 2530.0], [0.0, -16.0, -35.0]),
];

/// Formant frequencies and linear gains for Morph 0–1 (A-E-I-O-U), shifted by
/// the cutoff relative to 1 kHz.
fn formants(morph: f32, cutoff: f32) -> [(f32, f32); 3] {
    let position = morph.clamp(0.0, 1.0) * (VOWELS.len() - 1) as f32;
    let index = (position as usize).min(VOWELS.len() - 2);
    let t = position - index as f32;
    let (from, to) = (VOWELS[index], VOWELS[index + 1]);
    let shift = cutoff / 1000.0;
    std::array::from_fn(|formant| {
        let frequency = from.0[formant].powf(1.0 - t) * to.0[formant].powf(t) * shift;
        let gain = from.1[formant] * (1.0 - t) + to.1[formant] * t;
        (frequency, db_to_gain(gain))
    })
}

/// Pre-filter saturation of the linear models: identity at 0 dB, blending
/// into a level-compensated `tanh` as Drive rises.
#[inline]
fn soft_drive(input: f32, drive_db: f32) -> f32 {
    if drive_db <= 0.0 {
        return input;
    }
    let gain = db_to_gain(drive_db);
    let amount = drive_db / 24.0;
    input + amount * (tanh_fast(gain * input) / gain.sqrt() - input)
}

/// Small-signal gain of the drive stage, for the drawn curve.
fn drive_gain(model: FilterModel, drive_db: f32) -> f32 {
    let gain = db_to_gain(drive_db);
    if model.oversampled() {
        gain.sqrt()
    } else {
        1.0 + drive_db / 24.0 * (gain.sqrt() - 1.0)
    }
}

/// `band_gain` is normally the damping k, for a constant-peak band-pass, so
/// resonance narrows instead of boosting.
#[inline]
fn pick(out: SvfOutputs, filter_type: FilterType, band_gain: f32) -> f32 {
    match filter_type {
        FilterType::LowPass => out.low,
        FilterType::HighPass => out.high,
        FilterType::BandPass => out.band * band_gain,
        FilterType::Notch => out.notch(),
    }
}

/// Delays a signal by [`FilterKernel::LATENCY`]; for a dry path beside a
/// kernel.
#[derive(Clone, Debug)]
pub struct LatencyDelay {
    buffer: [f32; Oversampler2x::LATENCY as usize],
    cursor: usize,
}

impl Default for LatencyDelay {
    fn default() -> Self {
        Self {
            buffer: [0.0; Oversampler2x::LATENCY as usize],
            cursor: 0,
        }
    }
}

impl LatencyDelay {
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let output = std::mem::replace(&mut self.buffer[self.cursor], input);
        self.cursor += 1;
        if self.cursor == self.buffer.len() {
            self.cursor = 0;
        }
        output
    }
}

/// One channel of filtering, every model.
#[derive(Clone, Debug, Default)]
pub struct FilterKernel {
    svf: [Svf; 3],
    ladder: Ladder,
    sallen_key: SallenKey,
    comb: DelayLine,
    comb_damping: f32,
    oversampler: Oversampler2x,
    latency: LatencyDelay,
}

impl FilterKernel {
    pub const LATENCY: u32 = Oversampler2x::LATENCY;

    /// Allocates the comb for cutoffs down to 20 Hz. Call before audio runs.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.comb.allocate((sample_rate / 20.0).ceil() as usize + 4);
        self.reset();
    }

    pub fn reset(&mut self) {
        for svf in &mut self.svf {
            svf.reset();
        }
        self.ladder.reset();
        self.sallen_key.reset();
        self.comb.reset();
        self.comb_damping = 0.0;
        self.oversampler.reset();
        self.latency = LatencyDelay::default();
    }

    /// One sample. `cutoff` in Hz, `resonance` and `morph` 0–1.
    #[inline]
    pub fn process(
        &mut self,
        input: f32,
        shape: &FilterShape,
        cutoff: f32,
        resonance: f32,
        morph: f32,
        sample_rate: f32,
    ) -> f32 {
        if shape.model.oversampled() {
            return self.process_circuit(input, shape, cutoff, resonance, sample_rate);
        }
        let s = shape;
        let x = soft_drive(input, s.drive_db);
        let g = prewarp(cutoff, sample_rate);
        let wet = match s.model {
            FilterModel::StateVariable => {
                let k = 1.0 / clean_q(resonance);
                let out = self.svf[0].process(x, g, k);
                if morph < 0.5 {
                    out.low * (1.0 - 2.0 * morph) + out.notch() * 2.0 * morph
                } else {
                    out.notch() * (2.0 - 2.0 * morph) + out.high * (2.0 * morph - 1.0)
                }
            }
            FilterModel::Comb => {
                let feedback = comb_feedback(resonance);
                let delayed = self.comb.read(sample_rate / cutoff);
                self.comb_damping += comb_damping(morph) * (delayed - self.comb_damping);
                let y = x + feedback * self.comb_damping;
                self.comb.write(y);
                y * (1.0 - feedback * feedback).sqrt()
            }
            FilterModel::Vowel => {
                let k = vowel_k(resonance);
                let mut sum = 0.0;
                for (svf, (frequency, gain)) in self.svf.iter_mut().zip(formants(morph, cutoff)) {
                    sum += gain * k * svf.process(x, prewarp(frequency, sample_rate), k).band;
                }
                sum
            }
            _ if s.steep && matches!(s.filter_type, FilterType::LowPass | FilterType::HighPass) => {
                let [q1, q2] = steep_qs(resonance);
                let first = pick(self.svf[0].process(x, g, 1.0 / q1), s.filter_type, 1.0 / q1);
                pick(
                    self.svf[1].process(first, g, 1.0 / q2),
                    s.filter_type,
                    1.0 / q2,
                )
            }
            _ => {
                let k = 1.0 / clean_q(resonance);
                let first = pick(self.svf[0].process(x, g, k), s.filter_type, k);
                if s.steep {
                    pick(self.svf[1].process(first, g, k), s.filter_type, k)
                } else {
                    first
                }
            }
        };
        self.latency.process(wet)
    }

    /// The saturating circuit models, at 2× so their harmonics do not alias.
    #[inline]
    fn process_circuit(
        &mut self,
        input: f32,
        s: &FilterShape,
        cutoff: f32,
        resonance: f32,
        sample_rate: f32,
    ) -> f32 {
        let gain = db_to_gain(s.drive_db);
        let g = prewarp(cutoff, 2.0 * sample_rate);
        let upsampled = self.oversampler.upsample(input * gain);
        let filtered = upsampled.map(|x| match s.model {
            FilterModel::TransistorLadder => {
                let k = ladder_k(resonance);
                self.ladder.process(x, g, k)[4] * bass_gain(FilterType::LowPass, k)
            }
            FilterModel::OtaCascade => {
                let k = ladder_k(resonance);
                let taps = self.ladder.process(x, g, k);
                let mixed: f32 = pole_mix(s.filter_type, s.steep)
                    .iter()
                    .zip(taps)
                    .map(|(weight, tap)| weight * tap)
                    .sum();
                mixed * bass_gain(s.filter_type, k)
            }
            FilterModel::OtaMultimode => {
                let k = ota_damping(resonance);
                let band = resonant_band_gain(k);
                let first = pick(self.svf[0].process_saturated(x, g, k), s.filter_type, band);
                if s.steep {
                    pick(
                        self.svf[1].process_saturated(first, g, k),
                        s.filter_type,
                        band,
                    )
                } else {
                    first
                }
            }
            FilterModel::DiodePair => {
                let k = diode_damping(resonance);
                // The diodes clip the signal on its way in as well as the
                // resonance.
                let out = self.svf[0].process_clipped(tanh_fast(x), g, k);
                pick(out, s.filter_type, resonant_band_gain(k))
            }
            _ if s.filter_type == FilterType::HighPass => {
                self.sallen_key.high_pass(x, g, sallen_key_k(resonance))
            }
            _ => self.sallen_key.low_pass(x, g, sallen_key_k(resonance)),
        });
        self.oversampler.downsample(filtered) / gain.sqrt()
    }
}

/// Small-signal response of [`FilterKernel::process`] at `frequency`, from
/// the same mappings as the audio path, for drawing.
pub fn response(
    shape: &FilterShape,
    cutoff: f32,
    resonance: f32,
    morph: f32,
    sample_rate: f32,
    frequency: f32,
) -> Complex {
    let s = shape;
    let frequency = frequency.clamp(1.0, sample_rate * 0.499);
    let analog = |rate: f32, corner: f32| {
        Complex::new(
            0.0,
            f64::from(prewarp(frequency, rate)) / f64::from(prewarp(corner, rate)),
        )
    };
    let wet = match s.model {
        FilterModel::Clean => {
            let x = analog(sample_rate, cutoff);
            let stage = |q: f32| svf_response(x, s.filter_type, 1.0 / q, 1.0 / q);
            if s.steep && matches!(s.filter_type, FilterType::LowPass | FilterType::HighPass) {
                let [q1, q2] = steep_qs(resonance);
                stage(q1) * stage(q2)
            } else if s.steep {
                let q = clean_q(resonance);
                stage(q) * stage(q)
            } else {
                stage(clean_q(resonance))
            }
        }
        FilterModel::TransistorLadder | FilterModel::OtaCascade => {
            let x = analog(2.0 * sample_rate, cutoff);
            let k = ladder_k(resonance);
            let (filter_type, steep) = if s.model == FilterModel::TransistorLadder {
                (FilterType::LowPass, true)
            } else {
                (s.filter_type, s.steep)
            };
            let t = Complex::ONE + x;
            let powers = [t * t * t * t, t * t * t, t * t, t, Complex::ONE];
            let numerator = pole_mix(filter_type, steep)
                .iter()
                .zip(powers)
                .fold(Complex::real(0.0), |sum, (weight, power)| {
                    sum + power.scale(f64::from(*weight))
                });
            (numerator / (powers[0] + Complex::real(f64::from(k))))
                .scale(f64::from(bass_gain(filter_type, k)))
        }
        FilterModel::SallenKey => {
            let x = analog(2.0 * sample_rate, cutoff);
            let k = f64::from(sallen_key_k(resonance));
            let one = Complex::ONE + x;
            let denominator = one * one - x.scale(k);
            let numerator = if s.filter_type == FilterType::HighPass {
                x * x
            } else {
                Complex::ONE
            };
            numerator / denominator
        }
        FilterModel::OtaMultimode => {
            let x = analog(2.0 * sample_rate, cutoff);
            let k = ota_damping(resonance);
            let stage = svf_response(x, s.filter_type, k, resonant_band_gain(k));
            if s.steep { stage * stage } else { stage }
        }
        FilterModel::DiodePair => {
            let x = analog(2.0 * sample_rate, cutoff);
            let k = diode_damping(resonance);
            svf_response(x, s.filter_type, k, resonant_band_gain(k))
        }
        FilterModel::StateVariable => {
            let x = analog(sample_rate, cutoff);
            let k = 1.0 / clean_q(resonance);
            let low = svf_response(x, FilterType::LowPass, k, k);
            let notch = svf_response(x, FilterType::Notch, k, k);
            let high = svf_response(x, FilterType::HighPass, k, k);
            let morph = f64::from(morph);
            if morph < 0.5 {
                low.scale(1.0 - 2.0 * morph) + notch.scale(2.0 * morph)
            } else {
                notch.scale(2.0 - 2.0 * morph) + high.scale(2.0 * morph - 1.0)
            }
        }
        FilterModel::Comb => {
            let omega = std::f64::consts::TAU * f64::from(frequency / sample_rate);
            let feedback = f64::from(comb_feedback(resonance));
            let a = f64::from(comb_damping(morph));
            let delay = f64::from(sample_rate / cutoff);
            let damping = Complex::real(a) / (Complex::ONE - Complex::unit(-omega).scale(1.0 - a));
            let loop_gain = damping * Complex::unit(-omega * delay);
            Complex::real((1.0 - feedback * feedback).sqrt())
                / (Complex::ONE - loop_gain.scale(feedback))
        }
        FilterModel::Vowel => {
            let k = vowel_k(resonance);
            formants(morph, cutoff)
                .into_iter()
                .fold(Complex::real(0.0), |sum, (corner, gain)| {
                    let x = analog(sample_rate, corner);
                    sum + svf_response(x, FilterType::BandPass, k, k).scale(f64::from(gain))
                })
        }
    };
    wet.scale(f64::from(drive_gain(s.model, s.drive_db)))
}

/// Analog prototype of [`Svf`] at normalized `s`, damping `k`; the band-pass
/// is scaled by `band_gain` as in [`pick`].
fn svf_response(s: Complex, filter_type: FilterType, k: f32, band_gain: f32) -> Complex {
    let denominator = s * s + s.scale(f64::from(k)) + Complex::ONE;
    let numerator = match filter_type {
        FilterType::LowPass => Complex::ONE,
        FilterType::HighPass => s * s,
        FilterType::BandPass => s.scale(f64::from(band_gain)),
        FilterType::Notch => s * s + Complex::ONE,
    };
    numerator / denominator
}
