//! Filter kernels shared by every device that filters: R1 Filter, the synth
//! filter slots, EQ bands. One kernel holds the state for every model, so a
//! model switch is a reset, never an allocation. Circuit models run at 2× and
//! every model is delayed to the same [`FilterKernel::LATENCY`], so models and
//! dry paths line up whatever is selected.
use super::{
    Complex, DelayLine, Ladder, LadderCoeffs, LadderStereo, Oversampler2x, Oversampler2xStereo,
    SallenKey, SallenKeyCoeffs, SallenKeyStereo, StaticDelay, Svf, SvfCoeffs, SvfOutputs,
    SvfOutputsStereo, SvfStereo, db_to_gain, prewarp, tanh_fast,
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

#[inline]
fn pick_stereo(out: SvfOutputsStereo, filter_type: FilterType, band_gain: f32) -> [f32; 2] {
    match filter_type {
        FilterType::LowPass => out.low,
        FilterType::HighPass => out.high,
        FilterType::BandPass => [out.band[0] * band_gain, out.band[1] * band_gain],
        FilterType::Notch => out.notch(),
    }
}

/// The Clean model's response, its stage dampings worked out once: for a
/// caller that runs many of them with the corner moving every sample (the
/// Sampler's voices), without [`FilterKernel`]'s drive and latency.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CleanShape {
    filter_type: FilterType,
    steep: bool,
    /// Damping (1/Q) of each stage; also the band-pass gain, for a constant peak.
    k: [f32; 2],
}

impl CleanShape {
    /// `resonance` 0–1.
    pub fn new(filter_type: FilterType, steep: bool, resonance: f32) -> Self {
        let k = if steep && matches!(filter_type, FilterType::LowPass | FilterType::HighPass) {
            steep_qs(resonance).map(|q| 1.0 / q)
        } else {
            [1.0 / clean_q(resonance); 2]
        };
        Self {
            filter_type,
            steep,
            k,
        }
    }

    /// One sample through one or two stages; `g` from [`prewarp`].
    #[inline]
    pub fn process(&self, first: &mut Svf, second: &mut Svf, x: f32, g: f32) -> f32 {
        let [k1, k2] = self.k;
        let c1 = SvfCoeffs::new(g, k1);
        let y = pick(first.process_coeffs(x, c1), self.filter_type, k1);
        if self.steep {
            let c2 = if k2 == k1 { c1 } else { SvfCoeffs::new(g, k2) };
            pick(second.process_coeffs(y, c2), self.filter_type, k2)
        } else {
            y
        }
    }

    #[inline]
    pub fn coeffs(&self, g: f32) -> [SvfCoeffs; 2] {
        let [k1, k2] = self.k;
        let c1 = SvfCoeffs::new(g, k1);
        let c2 = if k2 == k1 { c1 } else { SvfCoeffs::new(g, k2) };
        [c1, c2]
    }

    #[inline]
    pub fn process_planar_pair(
        &self,
        stages: &mut [Svf; 4],
        x: [f32; 2],
        coeffs: [SvfCoeffs; 2],
    ) -> [f32; 2] {
        let [s0, s1, s2, s3] = stages;
        let [k1, k2] = self.k;
        let yl = pick(s0.process_coeffs(x[0], coeffs[0]), self.filter_type, k1);
        let yr = pick(s2.process_coeffs(x[1], coeffs[0]), self.filter_type, k1);
        if self.steep {
            [
                pick(s1.process_coeffs(yl, coeffs[1]), self.filter_type, k2),
                pick(s3.process_coeffs(yr, coeffs[1]), self.filter_type, k2),
            ]
        } else {
            [yl, yr]
        }
    }

    #[inline]
    pub fn process_stereo(
        &self,
        first: &mut SvfStereo,
        second: &mut SvfStereo,
        x: [f32; 2],
        g: f32,
    ) -> [f32; 2] {
        let [k1, k2] = self.k;
        let c1 = SvfCoeffs::new(g, k1);
        let y = pick_stereo(first.process_coeffs(x, c1), self.filter_type, k1);
        if self.steep {
            let c2 = if k2 == k1 { c1 } else { SvfCoeffs::new(g, k2) };
            pick_stereo(second.process_coeffs(y, c2), self.filter_type, k2)
        } else {
            y
        }
    }
}

/// The Clean model's latency; identical to [`FilterKernel::LATENCY`].
pub type LatencyDelay = StaticDelay<{ Oversampler2x::LATENCY as usize }>;
/// Stereo latency alignment delay matching [`FilterKernelStereo::LATENCY`].
pub type LatencyDelayStereo = super::StaticDelayStereo<{ Oversampler2x::LATENCY as usize }>;

/// The value last worked out and what it was worked out from: a setting
/// that costs a library call per sample and seldom moves is then free while
/// it rests, and exactly what it was while it moves.
#[derive(Clone, Copy, Debug)]
pub struct Memo<K, V> {
    held: Option<(K, V)>,
}

impl<K, V> Default for Memo<K, V> {
    fn default() -> Self {
        Self { held: None }
    }
}

impl<K: Copy + PartialEq, V: Copy> Memo<K, V> {
    #[inline]
    pub fn get(&mut self, key: K, value: impl FnOnce(K) -> V) -> V {
        match self.held {
            Some((held, value)) if held == key => value,
            _ => {
                let value = value(key);
                self.held = Some((key, value));
                value
            }
        }
    }
}

/// Control-rate precomputed shape/resonance constants for [`FilterKernel::circuit_prepared`].
#[derive(Clone, Copy, Debug)]
pub struct CircuitPrepared {
    model: FilterModel,
    filter_type: FilterType,
    steep: bool,
    k: f32,
    bass_gain: f32,
    weights: [f32; 5],
}

/// One channel of filtering, every model.
#[derive(Clone, Debug, Default)]
pub struct FilterKernel {
    /// The integrator gain of a cutoff and rate.
    g: Memo<(f32, f32), f32>,
    /// A drive's gain and its square root.
    drive: Memo<f32, (f32, f32)>,
    clean: Memo<(FilterType, bool, f32), CleanShape>,
    /// Each formant's integrator gain and level, of a morph, cutoff and rate.
    formants: Memo<(f32, f32, f32), [(f32, f32); 3]>,
    comb: Memo<(f32, f32), (f32, f32, f32)>,
    svf: [Svf; 3],
    ladder: Ladder,
    sallen_key: SallenKey,
    comb_line: DelayLine,
    comb_damping: f32,
    oversampler: Oversampler2x,
    latency: LatencyDelay,
}

impl FilterKernel {
    pub const LATENCY: u32 = Oversampler2x::LATENCY;

    /// Allocates the comb for cutoffs down to 20 Hz. Call before audio runs.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.comb_line
            .allocate((sample_rate / 20.0).ceil() as usize + 4);
        self.reset();
    }

    pub fn reset(&mut self) {
        for svf in &mut self.svf {
            svf.reset();
        }
        self.ladder.reset();
        self.sallen_key.reset();
        self.comb_line.reset();
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
        // Saturation before the linear models: the identity at 0 dB,
        // blending into a level-compensated `tanh` as Drive rises.
        let x = if s.drive_db <= 0.0 {
            input
        } else {
            let (gain, root) = self.drive_gains(s.drive_db);
            let amount = s.drive_db / 24.0;
            input + amount * (tanh_fast(gain * input) / root - input)
        };
        let wet = match s.model {
            FilterModel::StateVariable => {
                let g = self.g.get((cutoff, sample_rate), |(cutoff, rate)| {
                    prewarp(cutoff, rate)
                });
                let [k, _] = self
                    .clean
                    .get(
                        (FilterType::LowPass, false, resonance),
                        |(kind, steep, resonance)| CleanShape::new(kind, steep, resonance),
                    )
                    .k;
                let out = self.svf[0].process(x, g, k);
                if morph < 0.5 {
                    out.low * (1.0 - 2.0 * morph) + out.notch() * 2.0 * morph
                } else {
                    out.notch() * (2.0 - 2.0 * morph) + out.high * (2.0 * morph - 1.0)
                }
            }
            FilterModel::Comb => {
                let (feedback, damp, norm) = self.comb.get((resonance, morph), |(res, morph)| {
                    let fb = comb_feedback(res);
                    (fb, comb_damping(morph), (1.0 - fb * fb).sqrt())
                });
                // The read comes before this sample's write, so the line is
                // already one sample longer than the delay asked of it.
                let delayed = self.comb_line.read(sample_rate / cutoff - 1.0);
                self.comb_damping += damp * (delayed - self.comb_damping);
                let y = x + feedback * self.comb_damping;
                self.comb_line.write(y);
                y * norm
            }
            FilterModel::Vowel => {
                let k = vowel_k(resonance);
                let formants =
                    self.formants
                        .get((morph, cutoff, sample_rate), |(morph, cutoff, rate)| {
                            formants(morph, cutoff)
                                .map(|(frequency, gain)| (prewarp(frequency, rate), gain))
                        });
                let mut sum = 0.0;
                for (svf, (g, gain)) in self.svf.iter_mut().zip(formants) {
                    sum += gain * k * svf.process(x, g, k).band;
                }
                sum
            }
            _ => {
                let g = self.g.get((cutoff, sample_rate), |(cutoff, rate)| {
                    prewarp(cutoff, rate)
                });
                let clean = self.clean.get(
                    (s.filter_type, s.steep, resonance),
                    |(filter_type, steep, resonance)| {
                        CleanShape::new(filter_type, steep, resonance)
                    },
                );
                let (first, rest) = self.svf.split_at_mut(1);
                clean.process(&mut first[0], &mut rest[0], x, g)
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
        let (gain, root) = self.drive_gains(s.drive_db);
        let g = self.g.get((cutoff, 2.0 * sample_rate), |(cutoff, rate)| {
            prewarp(cutoff, rate)
        });
        let upsampled = self.oversampler.upsample(input * gain);
        let filtered = self.circuit_pair(upsampled, s, g, resonance);
        self.oversampler.downsample(filtered) / root
    }

    /// Processes a 2× oversampled pair sharing circuit coefficients.
    #[inline]
    fn circuit_pair(
        &mut self,
        pair: [f32; 2],
        s: &FilterShape,
        g: f32,
        resonance: f32,
    ) -> [f32; 2] {
        match s.model {
            FilterModel::TransistorLadder => {
                let k = ladder_k(resonance);
                let coeffs = LadderCoeffs::new(g, k);
                let gain = bass_gain(FilterType::LowPass, k);
                [
                    self.ladder.process_coeffs(pair[0], coeffs)[4] * gain,
                    self.ladder.process_coeffs(pair[1], coeffs)[4] * gain,
                ]
            }
            FilterModel::OtaCascade => {
                let k = ladder_k(resonance);
                let coeffs = LadderCoeffs::new(g, k);
                let weights = pole_mix(s.filter_type, s.steep);
                let gain = bass_gain(s.filter_type, k);
                pair.map(|x| {
                    let taps = self.ladder.process_coeffs(x, coeffs);
                    let mixed: f32 = weights.iter().zip(taps).map(|(w, tap)| w * tap).sum();
                    mixed * gain
                })
            }
            FilterModel::OtaMultimode => {
                let k = ota_damping(resonance);
                let coeffs = SvfCoeffs::new(g, k);
                let band = resonant_band_gain(k);
                pair.map(|x| {
                    let first = pick(
                        self.svf[0].process_saturated_coeffs(x, coeffs),
                        s.filter_type,
                        band,
                    );
                    if s.steep {
                        pick(
                            self.svf[1].process_saturated_coeffs(first, coeffs),
                            s.filter_type,
                            band,
                        )
                    } else {
                        first
                    }
                })
            }
            FilterModel::DiodePair => {
                let k = diode_damping(resonance);
                let coeffs = SvfCoeffs::new(g, k);
                let band = resonant_band_gain(k);
                pair.map(|x| {
                    let out = self.svf[0].process_clipped_coeffs(tanh_fast(x), coeffs);
                    pick(out, s.filter_type, band)
                })
            }
            FilterModel::SallenKey if s.filter_type == FilterType::HighPass => {
                let coeffs = SallenKeyCoeffs::new(g, sallen_key_k(resonance));
                [
                    self.sallen_key.high_pass_coeffs(pair[0], coeffs),
                    self.sallen_key.high_pass_coeffs(pair[1], coeffs),
                ]
            }
            FilterModel::SallenKey => {
                let coeffs = SallenKeyCoeffs::new(g, sallen_key_k(resonance));
                [
                    self.sallen_key.low_pass_coeffs(pair[0], coeffs),
                    self.sallen_key.low_pass_coeffs(pair[1], coeffs),
                ]
            }
            _ => pair,
        }
    }

    /// The gain of a drive in dB and its square root, for the level
    /// compensation.
    #[inline]
    fn drive_gains(&mut self, drive_db: f32) -> (f32, f32) {
        self.drive.get(drive_db, |db| {
            let gain = db_to_gain(db);
            (gain, gain.sqrt())
        })
    }

    /// One sample of a saturating circuit model at the oversampled rate, for
    /// callers that already run oversampled (a synth voice). `g` is
    /// [`prewarp`] at that rate; drive and its level compensation are the
    /// caller's. Linear models pass the input through.
    #[inline]
    pub fn circuit(&mut self, x: f32, s: &FilterShape, g: f32, resonance: f32) -> f32 {
        match s.model {
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
            FilterModel::SallenKey if s.filter_type == FilterType::HighPass => {
                self.sallen_key.high_pass(x, g, sallen_key_k(resonance))
            }
            FilterModel::SallenKey => self.sallen_key.low_pass(x, g, sallen_key_k(resonance)),
            _ => x,
        }
    }

    /// Per-voice prepared constants for [`Self::circuit_prepared`], hoisted
    /// once per block across multiple voices.
    #[inline]
    pub fn prepare_circuit(s: &FilterShape, resonance: f32) -> CircuitPrepared {
        CircuitPrepared {
            model: s.model,
            filter_type: s.filter_type,
            steep: s.steep,
            k: match s.model {
                FilterModel::TransistorLadder | FilterModel::OtaCascade => ladder_k(resonance),
                FilterModel::OtaMultimode => ota_damping(resonance),
                FilterModel::DiodePair => diode_damping(resonance),
                FilterModel::SallenKey => sallen_key_k(resonance),
                _ => 0.0,
            },
            bass_gain: match s.model {
                FilterModel::TransistorLadder => {
                    bass_gain(FilterType::LowPass, ladder_k(resonance))
                }
                FilterModel::OtaCascade => bass_gain(s.filter_type, ladder_k(resonance)),
                FilterModel::OtaMultimode => resonant_band_gain(ota_damping(resonance)),
                FilterModel::DiodePair => resonant_band_gain(diode_damping(resonance)),
                _ => 1.0,
            },
            weights: if s.model == FilterModel::OtaCascade {
                pole_mix(s.filter_type, s.steep)
            } else {
                [0.0; 5]
            },
        }
    }

    /// Evaluates one sample using precomputed shape/resonance constants while
    /// preserving outer-step ILP across polyphonic voices.
    #[inline]
    pub fn circuit_prepared(&mut self, x: f32, p: &CircuitPrepared, g: f32) -> f32 {
        match p.model {
            FilterModel::TransistorLadder => {
                let coeffs = LadderCoeffs::new(g, p.k);
                self.ladder.process_coeffs(x, coeffs)[4] * p.bass_gain
            }
            FilterModel::OtaCascade => {
                let coeffs = LadderCoeffs::new(g, p.k);
                let taps = self.ladder.process_coeffs(x, coeffs);
                let mixed: f32 = p.weights.iter().zip(taps).map(|(w, tap)| w * tap).sum();
                mixed * p.bass_gain
            }
            FilterModel::OtaMultimode => {
                let coeffs = SvfCoeffs::new(g, p.k);
                let first = pick(
                    self.svf[0].process_saturated_coeffs(x, coeffs),
                    p.filter_type,
                    p.bass_gain,
                );
                if p.steep {
                    pick(
                        self.svf[1].process_saturated_coeffs(first, coeffs),
                        p.filter_type,
                        p.bass_gain,
                    )
                } else {
                    first
                }
            }
            FilterModel::DiodePair => {
                let coeffs = SvfCoeffs::new(g, p.k);
                let out = self.svf[0].process_clipped_coeffs(tanh_fast(x), coeffs);
                pick(out, p.filter_type, p.bass_gain)
            }
            FilterModel::SallenKey if p.filter_type == FilterType::HighPass => {
                let coeffs = SallenKeyCoeffs::new(g, p.k);
                self.sallen_key.high_pass_coeffs(x, coeffs)
            }
            FilterModel::SallenKey => {
                let coeffs = SallenKeyCoeffs::new(g, p.k);
                self.sallen_key.low_pass_coeffs(x, coeffs)
            }
            _ => x,
        }
    }

    /// Processes a block of oversampled samples with a linearly ramped `g`
    /// (`g_start + g_step * step`) and constant `shape` / `resonance`.
    #[inline]
    pub fn circuit_block(
        &mut self,
        samples: &mut [f32],
        s: &FilterShape,
        g_start: f32,
        g_step: f32,
        resonance: f32,
    ) {
        if samples.is_empty() {
            return;
        }
        let steady = g_step == 0.0;
        match s.model {
            FilterModel::TransistorLadder => {
                let k = ladder_k(resonance);
                let gain = bass_gain(FilterType::LowPass, k);
                if steady {
                    let coeffs = LadderCoeffs::new(g_start, k);
                    for sample in samples {
                        *sample = self.ladder.process_coeffs(*sample, coeffs)[4] * gain;
                    }
                } else {
                    for (step, sample) in samples.iter_mut().enumerate() {
                        let coeffs = LadderCoeffs::new(g_start + g_step * step as f32, k);
                        *sample = self.ladder.process_coeffs(*sample, coeffs)[4] * gain;
                    }
                }
            }
            FilterModel::OtaCascade => {
                let k = ladder_k(resonance);
                let gain = bass_gain(s.filter_type, k);
                let weights = pole_mix(s.filter_type, s.steep);
                if steady {
                    let coeffs = LadderCoeffs::new(g_start, k);
                    for sample in samples {
                        let taps = self.ladder.process_coeffs(*sample, coeffs);
                        let mixed: f32 = weights.iter().zip(taps).map(|(w, tap)| w * tap).sum();
                        *sample = mixed * gain;
                    }
                } else {
                    for (step, sample) in samples.iter_mut().enumerate() {
                        let coeffs = LadderCoeffs::new(g_start + g_step * step as f32, k);
                        let taps = self.ladder.process_coeffs(*sample, coeffs);
                        let mixed: f32 = weights.iter().zip(taps).map(|(w, tap)| w * tap).sum();
                        *sample = mixed * gain;
                    }
                }
            }
            FilterModel::OtaMultimode => {
                let k = ota_damping(resonance);
                let band = resonant_band_gain(k);
                if steady {
                    let coeffs = SvfCoeffs::new(g_start, k);
                    for sample in samples {
                        let first = pick(
                            self.svf[0].process_saturated_coeffs(*sample, coeffs),
                            s.filter_type,
                            band,
                        );
                        *sample = if s.steep {
                            pick(
                                self.svf[1].process_saturated_coeffs(first, coeffs),
                                s.filter_type,
                                band,
                            )
                        } else {
                            first
                        };
                    }
                } else {
                    for (step, sample) in samples.iter_mut().enumerate() {
                        let coeffs = SvfCoeffs::new(g_start + g_step * step as f32, k);
                        let first = pick(
                            self.svf[0].process_saturated_coeffs(*sample, coeffs),
                            s.filter_type,
                            band,
                        );
                        *sample = if s.steep {
                            pick(
                                self.svf[1].process_saturated_coeffs(first, coeffs),
                                s.filter_type,
                                band,
                            )
                        } else {
                            first
                        };
                    }
                }
            }
            FilterModel::DiodePair => {
                let k = diode_damping(resonance);
                let band = resonant_band_gain(k);
                if steady {
                    let coeffs = SvfCoeffs::new(g_start, k);
                    for sample in samples {
                        let out = self.svf[0].process_clipped_coeffs(tanh_fast(*sample), coeffs);
                        *sample = pick(out, s.filter_type, band);
                    }
                } else {
                    for (step, sample) in samples.iter_mut().enumerate() {
                        let coeffs = SvfCoeffs::new(g_start + g_step * step as f32, k);
                        let out = self.svf[0].process_clipped_coeffs(tanh_fast(*sample), coeffs);
                        *sample = pick(out, s.filter_type, band);
                    }
                }
            }
            FilterModel::SallenKey if s.filter_type == FilterType::HighPass => {
                let k = sallen_key_k(resonance);
                if steady {
                    let coeffs = SallenKeyCoeffs::new(g_start, k);
                    for sample in samples {
                        *sample = self.sallen_key.high_pass_coeffs(*sample, coeffs);
                    }
                } else {
                    for (step, sample) in samples.iter_mut().enumerate() {
                        let coeffs = SallenKeyCoeffs::new(g_start + g_step * step as f32, k);
                        *sample = self.sallen_key.high_pass_coeffs(*sample, coeffs);
                    }
                }
            }
            FilterModel::SallenKey => {
                let k = sallen_key_k(resonance);
                if steady {
                    let coeffs = SallenKeyCoeffs::new(g_start, k);
                    for sample in samples {
                        *sample = self.sallen_key.low_pass_coeffs(*sample, coeffs);
                    }
                } else {
                    for (step, sample) in samples.iter_mut().enumerate() {
                        let coeffs = SallenKeyCoeffs::new(g_start + g_step * step as f32, k);
                        *sample = self.sallen_key.low_pass_coeffs(*sample, coeffs);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Stereo filter kernel sharing memoized control setup, coefficients, and 2× half-band FIR taps.
#[derive(Clone, Debug, Default)]
pub struct FilterKernelStereo {
    g: Memo<(f32, f32), f32>,
    drive: Memo<f32, (f32, f32)>,
    clean: Memo<(FilterType, bool, f32), CleanShape>,
    formants: Memo<(f32, f32, f32), [(f32, f32); 3]>,
    comb: Memo<(f32, f32), (f32, f32, f32)>,
    svf: [SvfStereo; 3],
    ladder: LadderStereo,
    sallen_key: SallenKeyStereo,
    comb_line: [DelayLine; 2],
    comb_damping: [f32; 2],
    oversampler: Oversampler2xStereo,
    latency: LatencyDelayStereo,
}

impl FilterKernelStereo {
    pub const LATENCY: u32 = FilterKernel::LATENCY;

    pub fn prepare(&mut self, sample_rate: f32) {
        let len = (sample_rate / 20.0).ceil() as usize + 4;
        self.comb_line[0].allocate(len);
        self.comb_line[1].allocate(len);
        self.reset();
    }

    pub fn reset(&mut self) {
        for svf in &mut self.svf {
            svf.reset();
        }
        self.ladder.reset();
        self.sallen_key.reset();
        self.comb_line[0].reset();
        self.comb_line[1].reset();
        self.comb_damping = [0.0; 2];
        self.oversampler.reset();
        self.latency = LatencyDelayStereo::default();
    }

    #[inline]
    pub fn process_asym(
        &mut self,
        input: [f32; 2],
        shape: &FilterShape,
        cutoffs: [f32; 2],
        resonance: f32,
        morph: f32,
        sample_rate: f32,
    ) -> [f32; 2] {
        if cutoffs[0] == cutoffs[1] {
            return self.process(input, shape, cutoffs[0], resonance, morph, sample_rate);
        }
        if shape.model.oversampled() {
            let (gain, root) = self.drive_gains(shape.drive_db);
            let gs = [
                prewarp(cutoffs[0], 2.0 * sample_rate),
                prewarp(cutoffs[1], 2.0 * sample_rate),
            ];
            let upsampled = self
                .oversampler
                .upsample([input[0] * gain, input[1] * gain]);
            let filtered = upsampled.map(|pair| match shape.model {
                FilterModel::TransistorLadder => {
                    let k = ladder_k(resonance);
                    let c0 = LadderCoeffs::new(gs[0], k);
                    let c1 = LadderCoeffs::new(gs[1], k);
                    let bass = bass_gain(FilterType::LowPass, k);
                    let out0 = self.ladder.process_asym(pair, [c0, c1])[4];
                    [out0[0] * bass, out0[1] * bass]
                }
                FilterModel::OtaCascade => {
                    let k = ladder_k(resonance);
                    let c0 = LadderCoeffs::new(gs[0], k);
                    let c1 = LadderCoeffs::new(gs[1], k);
                    let weights = pole_mix(shape.filter_type, shape.steep);
                    let bass = bass_gain(shape.filter_type, k);
                    let taps = self.ladder.process_asym(pair, [c0, c1]);
                    let mut mixed = [0.0; 2];
                    for (w, tap) in weights.iter().zip(taps) {
                        mixed[0] += w * tap[0];
                        mixed[1] += w * tap[1];
                    }
                    [mixed[0] * bass, mixed[1] * bass]
                }
                FilterModel::OtaMultimode => {
                    let k = ota_damping(resonance);
                    let coeffs = [SvfCoeffs::new(gs[0], k), SvfCoeffs::new(gs[1], k)];
                    let band = resonant_band_gain(k);
                    let first = pick_stereo(
                        self.svf[0].process_saturated_stereo_coeffs(pair, coeffs),
                        shape.filter_type,
                        band,
                    );
                    if shape.steep {
                        pick_stereo(
                            self.svf[1].process_saturated_stereo_coeffs(first, coeffs),
                            shape.filter_type,
                            band,
                        )
                    } else {
                        first
                    }
                }
                FilterModel::DiodePair => {
                    let k = diode_damping(resonance);
                    let coeffs = [SvfCoeffs::new(gs[0], k), SvfCoeffs::new(gs[1], k)];
                    let band = resonant_band_gain(k);
                    let shaped = [tanh_fast(pair[0]), tanh_fast(pair[1])];
                    let out = self.svf[0].process_clipped_stereo_coeffs(shaped, coeffs);
                    pick_stereo(out, shape.filter_type, band)
                }
                FilterModel::SallenKey if shape.filter_type == FilterType::HighPass => {
                    let k = sallen_key_k(resonance);
                    self.sallen_key.high_pass_asym(
                        pair,
                        [
                            SallenKeyCoeffs::new(gs[0], k),
                            SallenKeyCoeffs::new(gs[1], k),
                        ],
                    )
                }
                FilterModel::SallenKey => {
                    let k = sallen_key_k(resonance);
                    self.sallen_key.low_pass_asym(
                        pair,
                        [
                            SallenKeyCoeffs::new(gs[0], k),
                            SallenKeyCoeffs::new(gs[1], k),
                        ],
                    )
                }
                _ => pair,
            });
            let down = self.oversampler.downsample(filtered);
            return [down[0] / root, down[1] / root];
        }
        let s = shape;
        let x = if s.drive_db <= 0.0 {
            input
        } else {
            let (gain, root) = self.drive_gains(s.drive_db);
            let amount = s.drive_db / 24.0;
            [
                input[0] + amount * (tanh_fast(gain * input[0]) / root - input[0]),
                input[1] + amount * (tanh_fast(gain * input[1]) / root - input[1]),
            ]
        };
        let wet = match s.model {
            FilterModel::StateVariable => {
                let g0 = prewarp(cutoffs[0], sample_rate);
                let g1 = prewarp(cutoffs[1], sample_rate);
                let [k, _] = self
                    .clean
                    .get(
                        (FilterType::LowPass, false, resonance),
                        |(kind, steep, resonance)| CleanShape::new(kind, steep, resonance),
                    )
                    .k;
                let out = self.svf[0]
                    .process_stereo_coeffs(x, [SvfCoeffs::new(g0, k), SvfCoeffs::new(g1, k)]);
                let notch = out.notch();
                if morph < 0.5 {
                    let a = 1.0 - 2.0 * morph;
                    let b = 2.0 * morph;
                    [out.low[0] * a + notch[0] * b, out.low[1] * a + notch[1] * b]
                } else {
                    let a = 2.0 - 2.0 * morph;
                    let b = 2.0 * morph - 1.0;
                    [
                        notch[0] * a + out.high[0] * b,
                        notch[1] * a + out.high[1] * b,
                    ]
                }
            }
            FilterModel::Comb => {
                let (feedback, damp, norm) = self.comb.get((resonance, morph), |(res, morph)| {
                    let fb = comb_feedback(res);
                    (fb, comb_damping(morph), (1.0 - fb * fb).sqrt())
                });
                let delayed = [
                    self.comb_line[0].read(sample_rate / cutoffs[0] - 1.0),
                    self.comb_line[1].read(sample_rate / cutoffs[1] - 1.0),
                ];
                let mut out = [0.0; 2];
                for c in 0..2 {
                    self.comb_damping[c] += damp * (delayed[c] - self.comb_damping[c]);
                    let y = x[c] + feedback * self.comb_damping[c];
                    self.comb_line[c].write(y);
                    out[c] = y * norm;
                }
                out
            }
            FilterModel::Vowel => {
                let k = vowel_k(resonance);
                let f0 = formants(morph, cutoffs[0]).map(|(f, g)| (prewarp(f, sample_rate), g));
                let f1 = formants(morph, cutoffs[1]).map(|(f, g)| (prewarp(f, sample_rate), g));
                let mut sum = [0.0; 2];
                for (i, svf) in self.svf.iter_mut().enumerate() {
                    let out = svf.process_stereo_coeffs(
                        x,
                        [SvfCoeffs::new(f0[i].0, k), SvfCoeffs::new(f1[i].0, k)],
                    );
                    sum[0] += f0[i].1 * k * out.band[0];
                    sum[1] += f1[i].1 * k * out.band[1];
                }
                sum
            }
            _ => {
                let g0 = prewarp(cutoffs[0], sample_rate);
                let g1 = prewarp(cutoffs[1], sample_rate);
                let clean = self.clean.get(
                    (s.filter_type, s.steep, resonance),
                    |(filter_type, steep, resonance)| {
                        CleanShape::new(filter_type, steep, resonance)
                    },
                );
                let (first, rest) = self.svf.split_at_mut(1);
                let c1 = [
                    SvfCoeffs::new(g0, clean.k[0]),
                    SvfCoeffs::new(g1, clean.k[0]),
                ];
                let y = pick_stereo(
                    first[0].process_stereo_coeffs(x, c1),
                    clean.filter_type,
                    clean.k[0],
                );
                if clean.steep {
                    let c2 = [
                        SvfCoeffs::new(g0, clean.k[1]),
                        SvfCoeffs::new(g1, clean.k[1]),
                    ];
                    pick_stereo(
                        rest[0].process_stereo_coeffs(y, c2),
                        clean.filter_type,
                        clean.k[1],
                    )
                } else {
                    y
                }
            }
        };
        self.latency.process(wet)
    }

    #[inline]
    pub fn process(
        &mut self,
        input: [f32; 2],
        shape: &FilterShape,
        cutoff: f32,
        resonance: f32,
        morph: f32,
        sample_rate: f32,
    ) -> [f32; 2] {
        if shape.model.oversampled() {
            return self.process_circuit(input, shape, cutoff, resonance, sample_rate);
        }
        let s = shape;
        let x = if s.drive_db <= 0.0 {
            input
        } else {
            let (gain, root) = self.drive_gains(s.drive_db);
            let amount = s.drive_db / 24.0;
            [
                input[0] + amount * (tanh_fast(gain * input[0]) / root - input[0]),
                input[1] + amount * (tanh_fast(gain * input[1]) / root - input[1]),
            ]
        };
        let wet = match s.model {
            FilterModel::StateVariable => {
                let g = self.g.get((cutoff, sample_rate), |(cutoff, rate)| {
                    prewarp(cutoff, rate)
                });
                let [k, _] = self
                    .clean
                    .get(
                        (FilterType::LowPass, false, resonance),
                        |(kind, steep, resonance)| CleanShape::new(kind, steep, resonance),
                    )
                    .k;
                let coeffs = SvfCoeffs::new(g, k);
                let out = self.svf[0].process_coeffs(x, coeffs);
                let notch = out.notch();
                if morph < 0.5 {
                    let a = 1.0 - 2.0 * morph;
                    let b = 2.0 * morph;
                    [out.low[0] * a + notch[0] * b, out.low[1] * a + notch[1] * b]
                } else {
                    let a = 2.0 - 2.0 * morph;
                    let b = 2.0 * morph - 1.0;
                    [
                        notch[0] * a + out.high[0] * b,
                        notch[1] * a + out.high[1] * b,
                    ]
                }
            }
            FilterModel::Comb => {
                let (feedback, damp, norm) = self.comb.get((resonance, morph), |(res, morph)| {
                    let fb = comb_feedback(res);
                    (fb, comb_damping(morph), (1.0 - fb * fb).sqrt())
                });
                let delay = sample_rate / cutoff - 1.0;
                let delayed = [self.comb_line[0].read(delay), self.comb_line[1].read(delay)];
                let mut out = [0.0; 2];
                for c in 0..2 {
                    self.comb_damping[c] += damp * (delayed[c] - self.comb_damping[c]);
                    let y = x[c] + feedback * self.comb_damping[c];
                    self.comb_line[c].write(y);
                    out[c] = y * norm;
                }
                out
            }
            FilterModel::Vowel => {
                let k = vowel_k(resonance);
                let formants =
                    self.formants
                        .get((morph, cutoff, sample_rate), |(morph, cutoff, rate)| {
                            formants(morph, cutoff)
                                .map(|(frequency, gain)| (prewarp(frequency, rate), gain))
                        });
                let mut sum = [0.0; 2];
                for (svf, (g, gain)) in self.svf.iter_mut().zip(formants) {
                    let out = svf.process_coeffs(x, SvfCoeffs::new(g, k));
                    sum[0] += gain * k * out.band[0];
                    sum[1] += gain * k * out.band[1];
                }
                sum
            }
            _ => {
                let g = self.g.get((cutoff, sample_rate), |(cutoff, rate)| {
                    prewarp(cutoff, rate)
                });
                let clean = self.clean.get(
                    (s.filter_type, s.steep, resonance),
                    |(filter_type, steep, resonance)| {
                        CleanShape::new(filter_type, steep, resonance)
                    },
                );
                let (first, rest) = self.svf.split_at_mut(1);
                clean.process_stereo(&mut first[0], &mut rest[0], x, g)
            }
        };
        self.latency.process(wet)
    }

    #[inline]
    fn process_circuit(
        &mut self,
        input: [f32; 2],
        s: &FilterShape,
        cutoff: f32,
        resonance: f32,
        sample_rate: f32,
    ) -> [f32; 2] {
        let (gain, root) = self.drive_gains(s.drive_db);
        let g = self.g.get((cutoff, 2.0 * sample_rate), |(cutoff, rate)| {
            prewarp(cutoff, rate)
        });
        let upsampled = self
            .oversampler
            .upsample([input[0] * gain, input[1] * gain]);
        let filtered = self.circuit_pair(upsampled, s, g, resonance);
        let down = self.oversampler.downsample(filtered);
        [down[0] / root, down[1] / root]
    }

    #[inline]
    fn circuit_pair(
        &mut self,
        pair: [[f32; 2]; 2],
        s: &FilterShape,
        g: f32,
        resonance: f32,
    ) -> [[f32; 2]; 2] {
        match s.model {
            FilterModel::TransistorLadder => {
                let k = ladder_k(resonance);
                let coeffs = LadderCoeffs::new(g, k);
                let gain = bass_gain(FilterType::LowPass, k);
                let out0 = self.ladder.process_coeffs(pair[0], coeffs)[4];
                let out1 = self.ladder.process_coeffs(pair[1], coeffs)[4];
                [
                    [out0[0] * gain, out0[1] * gain],
                    [out1[0] * gain, out1[1] * gain],
                ]
            }
            FilterModel::OtaCascade => {
                let k = ladder_k(resonance);
                let coeffs = LadderCoeffs::new(g, k);
                let weights = pole_mix(s.filter_type, s.steep);
                let gain = bass_gain(s.filter_type, k);
                pair.map(|x| {
                    let taps = self.ladder.process_coeffs(x, coeffs);
                    let mut mixed = [0.0; 2];
                    for (w, tap) in weights.iter().zip(taps) {
                        mixed[0] += w * tap[0];
                        mixed[1] += w * tap[1];
                    }
                    [mixed[0] * gain, mixed[1] * gain]
                })
            }
            FilterModel::OtaMultimode => {
                let k = ota_damping(resonance);
                let coeffs = SvfCoeffs::new(g, k);
                let band = resonant_band_gain(k);
                pair.map(|x| {
                    let first = pick_stereo(
                        self.svf[0].process_saturated_coeffs(x, coeffs),
                        s.filter_type,
                        band,
                    );
                    if s.steep {
                        pick_stereo(
                            self.svf[1].process_saturated_coeffs(first, coeffs),
                            s.filter_type,
                            band,
                        )
                    } else {
                        first
                    }
                })
            }
            FilterModel::DiodePair => {
                let k = diode_damping(resonance);
                let coeffs = SvfCoeffs::new(g, k);
                let band = resonant_band_gain(k);
                pair.map(|x| {
                    let shaped = [tanh_fast(x[0]), tanh_fast(x[1])];
                    let out = self.svf[0].process_clipped_coeffs(shaped, coeffs);
                    pick_stereo(out, s.filter_type, band)
                })
            }
            FilterModel::SallenKey if s.filter_type == FilterType::HighPass => {
                let coeffs = SallenKeyCoeffs::new(g, sallen_key_k(resonance));
                [
                    self.sallen_key.high_pass_coeffs(pair[0], coeffs),
                    self.sallen_key.high_pass_coeffs(pair[1], coeffs),
                ]
            }
            FilterModel::SallenKey => {
                let coeffs = SallenKeyCoeffs::new(g, sallen_key_k(resonance));
                [
                    self.sallen_key.low_pass_coeffs(pair[0], coeffs),
                    self.sallen_key.low_pass_coeffs(pair[1], coeffs),
                ]
            }
            _ => pair,
        }
    }

    #[inline]
    fn drive_gains(&mut self, drive_db: f32) -> (f32, f32) {
        self.drive.get(drive_db, |db| {
            let gain = db_to_gain(db);
            (gain, gain.sqrt())
        })
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

/// The lowest corner a [`sweep_mix`] low-pass reaches, at the left end.
pub const SWEEP_LOW_HZ: f32 = 40.0;
/// The highest corner a [`sweep_mix`] high-pass reaches, at the right end.
pub const SWEEP_HIGH_HZ: f32 = 10_000.0;
/// How far from the centre a sweep fades in from no filtering at all, as a
/// share of either half: the corner starts at the edge of the audio band
/// and the filter blends in over this travel, so the centre is exactly
/// bypassed and a sweep through it never steps.
const SWEEP_BLEND: f32 = 0.08;

/// The corner of a one-knob filter sweep at `position` (−1 to 1): a
/// low-pass from 20 kHz down to [`SWEEP_LOW_HZ`] left of the centre, a
/// high-pass from 20 Hz up to [`SWEEP_HIGH_HZ`] right of it, nothing at the
/// centre. `Some((high_pass, hertz))`.
pub fn sweep_corner(position: f32) -> Option<(bool, f32)> {
    if position == 0.0 || !position.is_finite() {
        return None;
    }
    let position = position.clamp(-1.0, 1.0);
    let travel = position.abs();
    Some(if position < 0.0 {
        (false, 20_000.0 * (SWEEP_LOW_HZ / 20_000.0).powf(travel))
    } else {
        (true, 20.0 * (SWEEP_HIGH_HZ / 20.0).powf(travel))
    })
}

/// Damping of a sweep at `resonance` 0–1: Butterworth (Q 0.71) up to Q 8,
/// a peak of about 18 dB.
fn sweep_damping(resonance: f32) -> f32 {
    1.0 / (std::f32::consts::FRAC_1_SQRT_2 * 11.3_f32.powf(resonance.clamp(0.0, 1.0)))
}

/// One-knob filter, as on a DJ mixer or a drum machine's pad: the
/// coefficients of a 12 dB/oct [`Svf`] at [`sweep_corner`]`(position)`,
/// blended in from the identity near the centre. Run it with [`Svf::tick`]
/// and ramp between control periods with [`crate::SvfMixRamp`].
pub fn sweep_mix(position: f32, resonance: f32, sample_rate: f32) -> crate::SvfMix<f32> {
    let k = sweep_damping(resonance);
    let Some((high, hertz)) = sweep_corner(position) else {
        // The poles where the sweep leaves them for either side, so a ramp
        // from here moves the mix only.
        return crate::SvfMix::new(prewarp(20_000.0, sample_rate), k, 1.0, 0.0, 0.0);
    };
    let g = prewarp(hertz, sample_rate);
    let t = (position.abs() / SWEEP_BLEND).min(1.0);
    // Smoothstep: no corner in the blend either.
    let t = t * t * (3.0 - 2.0 * t);
    if high {
        // high = input − k·band − low
        crate::SvfMix::new(g, k, 1.0, -t * k, -t)
    } else {
        crate::SvfMix::new(g, k, 1.0 - t, 0.0, t)
    }
}

/// Magnitude of [`sweep_mix`] at `frequency`, from the same coefficients,
/// for drawing and tests.
pub fn sweep_response(position: f32, resonance: f32, sample_rate: f32, frequency: f32) -> f32 {
    let mix = sweep_mix(position, resonance, sample_rate);
    let frequency = frequency.clamp(1.0, sample_rate * 0.499);
    let s = Complex::new(
        0.0,
        f64::from(prewarp(frequency, sample_rate)) / f64::from(mix.g),
    );
    let denominator = s * s + s.scale(f64::from(mix.k)) + Complex::ONE;
    // m0·input + m1·band + m2·low, with band = s/D and low = 1/D.
    let response = Complex::real(f64::from(mix.m0))
        + (s.scale(f64::from(mix.m1)) + Complex::real(f64::from(mix.m2))) / denominator;
    response.norm() as f32
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

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// Peak gain of [`sweep_mix`] for a sine at `hz`, through an [`Svf`].
    fn sweep_gain(position: f32, hz: f32) -> f32 {
        let mix = sweep_mix(position, 0.0, RATE);
        let mut svf = Svf::default();
        let mut peak = 0.0_f32;
        for n in 0..24_000 {
            let x = (std::f32::consts::TAU * hz * n as f32 / RATE).sin();
            let y = svf.tick(x, &mix);
            if n > 12_000 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn a_sweep_bypasses_at_its_centre_and_cuts_on_either_side() {
        assert!(sweep_mix(0.0, 0.3, RATE).is_identity());
        assert_eq!(sweep_corner(0.0), None);
        assert_eq!(sweep_corner(-1.0), Some((false, SWEEP_LOW_HZ)));
        assert_eq!(sweep_corner(1.0), Some((true, SWEEP_HIGH_HZ)));
        for hz in [50.0, 1000.0, 10_000.0] {
            assert!((sweep_gain(0.0, hz) - 1.0).abs() < 1e-3, "{hz}");
        }
        // Half way left a low-pass near 900 Hz, half way right a high-pass
        // near 450 Hz.
        assert!(sweep_gain(-0.5, 100.0) > 0.95 && sweep_gain(-0.5, 8000.0) < 0.05);
        assert!(sweep_gain(0.5, 5000.0) > 0.95 && sweep_gain(0.5, 50.0) < 0.05);
        // The drawn response is what plays.
        for (position, hz) in [(-0.5, 2000.0), (0.6, 300.0), (0.03, 30.0)] {
            let drawn = sweep_response(position, 0.0, RATE, hz);
            let played = sweep_gain(position, hz);
            assert!(
                (drawn - played).abs() < 0.02,
                "{position} {hz}: {drawn} {played}"
            );
        }
    }

    #[test]
    fn a_sweep_moves_without_steps_through_its_centre() {
        let mut previous = sweep_response(-0.2, 0.5, RATE, 3000.0);
        for step in -199..=200 {
            let position = step as f32 * 0.001;
            let gain = sweep_response(position, 0.5, RATE, 3000.0);
            assert!(
                (gain - previous).abs() < 0.05,
                "{position}: {previous} → {gain}"
            );
            previous = gain;
        }
    }

    const MODELS: [FilterModel; 9] = [
        FilterModel::Clean,
        FilterModel::TransistorLadder,
        FilterModel::SallenKey,
        FilterModel::OtaCascade,
        FilterModel::OtaMultimode,
        FilterModel::DiodePair,
        FilterModel::StateVariable,
        FilterModel::Comb,
        FilterModel::Vowel,
    ];

    fn shape(model: FilterModel) -> FilterShape {
        FilterShape {
            model,
            filter_type: FilterType::LowPass,
            steep: false,
            drive_db: 0.0,
        }
    }

    /// Measured gain in dB of a settled small sine through a fresh kernel.
    fn measured_db(shape: &FilterShape, cutoff: f32, resonance: f32, morph: f32, f: f32) -> f64 {
        let mut kernel = FilterKernel::default();
        kernel.prepare(RATE);
        let amplitude = 0.05;
        let total = RATE as usize;
        let (mut i, mut q) = (0.0_f64, 0.0_f64);
        for n in 0..total {
            let phase = std::f64::consts::TAU * f64::from(f) * n as f64 / f64::from(RATE);
            let x = phase.sin() as f32 * amplitude;
            let y = kernel.process(x, shape, cutoff, resonance, morph, RATE);
            if n >= total / 2 {
                i += f64::from(y) * phase.sin();
                q += f64::from(y) * phase.cos();
            }
        }
        let gain = 2.0 * i.hypot(q) / (total / 2) as f64 / f64::from(amplitude);
        20.0 * gain.log10()
    }

    // The R1 Filter tests compare the drawn curve of every other model with
    // its sound; these two are the morphing models they leave out.
    #[test]
    fn comb_and_vowel_sound_like_their_drawn_response() {
        for (model, cutoff, resonance, morph) in [
            (FilterModel::Comb, 500.0, 0.5, 0.0),
            (FilterModel::Comb, 1200.0, 0.8, 0.6),
            (FilterModel::Vowel, 1000.0, 0.3, 0.0),
            (FilterModel::Vowel, 1500.0, 0.7, 0.55),
        ] {
            let shape = shape(model);
            for f in [100.0, 330.0, 500.0, 750.0, 1000.0, 2400.0, 5000.0] {
                let drawn = 20.0
                    * response(&shape, cutoff, resonance, morph, RATE, f)
                        .norm()
                        .log10();
                if drawn < -50.0 {
                    continue;
                }
                let measured = measured_db(&shape, cutoff, resonance, morph, f);
                assert!(
                    (measured - drawn).abs() < 0.5,
                    "{model:?} {cutoff} Hz res {resonance} morph {morph} at {f} Hz: \
                     measured {measured:.2}, drawn {drawn:.2}"
                );
            }
        }
    }

    #[test]
    fn comb_echoes_exactly_one_period_apart() {
        let shape = shape(FilterModel::Comb);
        let mut kernel = FilterKernel::default();
        kernel.prepare(RATE);
        let echoes: Vec<usize> = (0..300)
            .filter(|&n| {
                let x = if n == 0 { 1.0 } else { 0.0 };
                kernel.process(x, &shape, 500.0, 0.5, 0.0, RATE).abs() > 1e-3
            })
            .collect();
        let latency = FilterKernel::LATENCY as usize;
        assert_eq!(echoes, [latency, latency + 96, latency + 192]);
    }

    #[test]
    fn comb_peaks_on_the_harmonics_of_its_cutoff() {
        let shape = shape(FilterModel::Comb);
        let at = |f| measured_db(&shape, 400.0, 0.9, 0.0, f);
        for harmonic in [400.0, 800.0, 1200.0] {
            assert!(at(harmonic) > at(harmonic + 200.0) + 20.0, "{harmonic}");
        }
    }

    #[test]
    fn every_model_has_the_same_latency_as_the_dry_path() {
        for model in MODELS {
            let mut kernel = FilterKernel::default();
            kernel.prepare(RATE);
            let mut dry = LatencyDelay::default();
            // Wide open, so the kernel is close to a pure delay. Comb and
            // Vowel have no open setting; the comb's first echo is checked
            // in its own test.
            if matches!(model, FilterModel::Comb | FilterModel::Vowel) {
                continue;
            }
            let shape = shape(model);
            let mut wet_peak = (0, 0.0_f32);
            for n in 0..64 {
                let x = if n == 0 { 0.1 } else { 0.0 };
                let y = kernel.process(x, &shape, 20_000.0, 0.0, 0.0, RATE);
                if y.abs() > wet_peak.1 {
                    wet_peak = (n, y.abs());
                }
                if dry.process(x) != 0.0 {
                    assert_eq!(n, FilterKernel::LATENCY as usize);
                }
            }
            assert!(
                wet_peak.0.abs_diff(FilterKernel::LATENCY as usize) <= 1,
                "{model:?} peaks at {}",
                wet_peak.0
            );
        }
    }

    #[test]
    fn extreme_settings_stay_finite_and_reset_silences_every_model() {
        let mut kernel = FilterKernel::default();
        kernel.prepare(RATE);
        for model in MODELS {
            for filter_type in [
                FilterType::LowPass,
                FilterType::HighPass,
                FilterType::BandPass,
                FilterType::Notch,
            ] {
                for steep in [false, true] {
                    let shape = FilterShape {
                        model,
                        filter_type,
                        steep,
                        drive_db: 24.0,
                    };
                    kernel.reset();
                    for n in 0..4800 {
                        // Cutoff, resonance and morph swept per sample, past
                        // their ranges, into a full-scale square.
                        let t = n as f32 / 4800.0;
                        let cutoff = 10.0 + 30_000.0 * t;
                        let x = if n % 37 < 18 { 1.0 } else { -1.0 };
                        let y = kernel.process(x, &shape, cutoff, 1.0, 1.2 * t - 0.1, RATE);
                        assert!(y.is_finite() && y.abs() < 20.0, "{shape:?} at {n}: {y}");
                    }
                    kernel.reset();
                    let after = kernel.process(0.0, &shape, 1000.0, 0.5, 0.5, RATE);
                    assert_eq!(after, 0.0, "{shape:?} keeps state after reset");
                }
            }
        }
    }

    #[test]
    fn type_slope_and_morph_capabilities_match_the_kernel() {
        for model in MODELS {
            // A model without types ignores the type; one with types reacts.
            let low = shape(model);
            let high = FilterShape {
                filter_type: FilterType::HighPass,
                ..low
            };
            let a = measured_db(&low, 1000.0, 0.2, 0.3, 100.0);
            let b = measured_db(&high, 1000.0, 0.2, 0.3, 100.0);
            assert_eq!(
                model.type_count() >= 2,
                (a - b).abs() > 6.0,
                "{model:?}: low-pass {a:.1} dB, high-pass {b:.1} dB"
            );
        }
    }
}
