//! Sample playback at any pitch. A recording is kept as a mip-map, each
//! level low-passed and decimated 2:1 from the one before, and read through
//! a 48-tap Kaiser-windowed sinc whose cutoff follows the playback step. A
//! step of `r` source samples per output sample reads level ⌊log2 r⌋, where
//! the remaining step `r'` is 1 to 2, with the cutoff at 0.9/`r'` of that
//! level's band. So a sample transposed up keeps its aliases above 0.88 of
//! the output band (above 21 kHz at 48 kHz) and at least 80 dB down below
//! that (about 90 dB measured), and one transposed down keeps everything up to 0.79 of its own band.
//! Every level is padded with silence, so a read needs no bounds checks;
//! outside the recording a read is silence.
use super::oversample::bessel_i0;
use std::sync::OnceLock;

/// Taps of one read.
pub const TAPS: usize = 48;
/// Running sums of a read, side by side.
const LANES: usize = 8;
/// Taps before the sample a read lands on; the rest are at or after it.
const BEFORE: usize = TAPS / 2 - 1;
/// Fractions of a sample tabulated per kernel; reads blend the two nearest.
const PHASES: usize = 256;
/// Kernels per octave of step, each for the fastest step of its range.
const STEPS: usize = 8;
const BETA: f64 = 8.0;
/// The kernel's −6 dB point at a step of 1, as a share of the band: its
/// transition band then ends at the band's edge.
const CUTOFF: f64 = 0.9;
/// Silence around each level: more than a read reaches.
const PAD: usize = TAPS;
/// Levels of a mip-map: the recording and six halvings.
pub const LEVELS: usize = 7;
/// The fastest step a read is band-limited for, in source samples per output
/// sample: six octaves above unity and most of a seventh.
pub const MAX_STEP: f32 = 120.0;
/// The decimation filter between levels: flat to 0.395 of the finer level's
/// band (0.79 of the coarser one's), 87 dB down from 0.496, so what folds
/// into the coarser level lies above what the kernels read from it.
const DECIMATION_TAPS: usize = 111;
const DECIMATION_BETA: f64 = 8.63;
const DECIMATION_CUTOFF: f64 = 0.4455;

/// The read kernels of every step, shared by every sample and voice.
pub struct SincBank {
    /// `(STEPS + 1) × (PHASES + 1)` rows of weights.
    rows: Box<[[f32; TAPS]]>,
}

/// Windowed sinc with its −6 dB point at `cutoff` of the band, at `x`
/// samples from its centre, over `half` samples either side.
fn windowed_sinc(x: f64, cutoff: f64, half: f64, beta: f64) -> f64 {
    let angle = std::f64::consts::PI * cutoff * x;
    let sinc = if angle.abs() < 1e-9 {
        1.0
    } else {
        angle.sin() / angle
    };
    let ratio = x / half;
    cutoff * sinc * bessel_i0(beta * (1.0 - ratio * ratio).max(0.0).sqrt()) / bessel_i0(beta)
}

impl SincBank {
    fn new() -> Self {
        let mut rows = vec![[0.0_f32; TAPS]; (STEPS + 1) * (PHASES + 1)];
        for table in 0..=STEPS {
            let cutoff = CUTOFF / (table as f64 / STEPS as f64).exp2();
            for phase in 0..=PHASES {
                let fraction = phase as f64 / PHASES as f64;
                let weights: [f64; TAPS] = std::array::from_fn(|tap| {
                    let x = tap as f64 - BEFORE as f64 - fraction;
                    windowed_sinc(x, cutoff, (TAPS / 2) as f64, BETA)
                });
                // Unit gain at DC at every fraction, so a still sample
                // does not ripple as it moves.
                let sum: f64 = weights.iter().sum();
                for (slot, weight) in rows[table * (PHASES + 1) + phase].iter_mut().zip(weights) {
                    *slot = (weight / sum) as f32;
                }
            }
        }
        Self {
            rows: rows.into_boxed_slice(),
        }
    }

    /// Built on first use (a few hundred kilobytes), then shared.
    pub fn shared() -> &'static Self {
        static BANK: OnceLock<SincBank> = OnceLock::new();
        BANK.get_or_init(Self::new)
    }

    /// The weights of a read `frac` (0 up to 1) past a sample, blended
    /// between the two nearest tabulated fractions.
    #[inline]
    fn weights(&self, table: usize, frac: f32, out: &mut [f32; TAPS]) {
        let place = frac.clamp(0.0, 1.0) * PHASES as f32;
        let phase = (place as usize).min(PHASES - 1);
        let t = place - phase as f32;
        let row = table * (PHASES + 1) + phase;
        let (near, far) = (&self.rows[row], &self.rows[row + 1]);
        for ((out, near), far) in out.iter_mut().zip(near).zip(far) {
            *out = near + t * (far - near);
        }
    }
}

/// Which level and kernel a playback step reads with. Worked out once per
/// control period, since it costs a logarithm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub level: usize,
    pub table: usize,
    /// Source positions per position of the level.
    scale: f64,
}

impl Step {
    /// `step` in source samples per output sample, either direction. Choose
    /// it for the fastest step of the period it is used for, so the cutoff
    /// is never too high.
    pub fn new(step: f32) -> Self {
        let step = step.abs().clamp(1e-6, MAX_STEP);
        let level = (step.log2().floor().max(0.0) as usize).min(LEVELS - 1);
        let rest = step / (1_u32 << level) as f32;
        let table = if rest <= 1.0 {
            0
        } else {
            ((rest.log2() * STEPS as f32).ceil() as usize).min(STEPS)
        };
        Self {
            level,
            table,
            scale: 1.0 / f64::from(1_u32 << level),
        }
    }
}

/// Where one read lands and its weights; the same for every channel of a
/// recording, so a stereo read works them out once.
pub struct Tap {
    level: usize,
    base: usize,
    weights: [f32; TAPS],
}

/// One channel of a recording, ready to read at any step.
#[derive(Clone, Debug)]
pub struct MipMap {
    /// Each level with [`PAD`] zeros on both sides.
    levels: Vec<Vec<f32>>,
    frames: usize,
}

impl MipMap {
    /// Allocates about twice the recording and filters it: call outside the
    /// audio thread.
    pub fn new(samples: &[f32]) -> Self {
        let decimation: Vec<f64> = (0..DECIMATION_TAPS)
            .map(|tap| {
                let x = tap as f64 - (DECIMATION_TAPS / 2) as f64;
                windowed_sinc(
                    x,
                    DECIMATION_CUTOFF,
                    (DECIMATION_TAPS / 2 + 1) as f64,
                    DECIMATION_BETA,
                )
            })
            .collect();
        let sum: f64 = decimation.iter().sum();
        let padded = |frames: usize, sample: &dyn Fn(usize) -> f32| {
            let mut level = vec![0.0; frames + 2 * PAD];
            for (index, slot) in level[PAD..PAD + frames].iter_mut().enumerate() {
                *slot = sample(index);
            }
            level
        };
        let mut levels = Vec::with_capacity(LEVELS);
        levels.push(padded(samples.len(), &|index| samples[index]));
        let mut frames = samples.len();
        for _ in 1..LEVELS {
            let finer: &[f32] = &levels[levels.len() - 1];
            let at = |index: isize| {
                usize::try_from(index)
                    .ok()
                    .filter(|index| *index < frames)
                    .map_or(0.0, |index| f64::from(finer[PAD + index]))
            };
            // Sample k of the coarser level is centred on sample 2k of the
            // finer one, so positions halve exactly from level to level.
            let coarser = frames.div_ceil(2);
            let level = padded(coarser, &|index| {
                let centre = 2 * index as isize - (DECIMATION_TAPS / 2) as isize;
                let filtered: f64 = decimation
                    .iter()
                    .enumerate()
                    .map(|(tap, weight)| weight * at(centre + tap as isize))
                    .sum();
                (filtered / sum) as f32
            });
            levels.push(level);
            frames = coarser;
        }
        Self {
            levels,
            frames: samples.len(),
        }
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    /// The recording itself.
    pub fn samples(&self) -> &[f32] {
        &self.levels[0][PAD..PAD + self.frames]
    }

    /// Where a read at `position` (in samples of the recording) lands, and
    /// its weights. Beyond the ends, the read is of silence.
    #[inline]
    pub fn tap(&self, bank: &SincBank, step: &Step, position: f64) -> Tap {
        let level = step.level.min(self.levels.len() - 1);
        let frames = self.levels[level].len() - 2 * PAD;
        // Far enough out that every tap reads padding, and no further.
        let place = (position * step.scale).clamp(-(BEFORE as f64 + 2.0), (frames + BEFORE) as f64);
        let whole = place.floor();
        let mut weights = [0.0; TAPS];
        bank.weights(step.table, (place - whole) as f32, &mut weights);
        Tap {
            level,
            base: (PAD as isize + whole as isize - BEFORE as isize) as usize,
            weights,
        }
    }

    /// The read a [`Self::tap`] of this or a recording of the same length
    /// describes.
    #[inline]
    pub fn read(&self, tap: &Tap) -> f32 {
        let data: &[f32; TAPS] = self.levels[tap.level][tap.base..tap.base + TAPS]
            .try_into()
            .expect("a read of TAPS samples");
        dot(&tap.weights, data)
    }
}

/// Σ weight·sample in eight running sums, so the products add up side by
/// side (and in vector registers) instead of one long chain of dependent
/// adds.
#[inline]
fn dot(weights: &[f32; TAPS], samples: &[f32; TAPS]) -> f32 {
    let mut sums = [0.0_f32; LANES];
    for (weights, samples) in weights.chunks_exact(LANES).zip(samples.chunks_exact(LANES)) {
        for lane in 0..LANES {
            sums[lane] += weights[lane] * samples[lane];
        }
    }
    let [a, b, c, d, e, f, g, h] = sums;
    ((a + b) + (c + d)) + ((e + f) + (g + h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Plays `samples` at a constant step and returns the output.
    fn play(map: &MipMap, step: f32, start: f64, frames: usize) -> Vec<f32> {
        let bank = SincBank::shared();
        let reader = Step::new(step);
        (0..frames)
            .map(|n| map.read(&map.tap(bank, &reader, start + n as f64 * f64::from(step))))
            .collect()
    }

    fn sine(frequency: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (2.0 * PI * frequency * n as f64).sin() as f32)
            .collect()
    }

    /// Amplitude of the component at `frequency` (cycles per sample) and the
    /// RMS of what is left once it is removed, by least squares.
    fn fit(signal: &[f32], frequency: f64) -> (f64, f64) {
        let (mut ss, mut sc, mut cc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (n, y) in signal.iter().enumerate() {
            let (s, c) = (2.0 * PI * frequency * n as f64).sin_cos();
            let y = f64::from(*y);
            ss += s * s;
            sc += s * c;
            cc += c * c;
            ys += y * s;
            yc += y * c;
        }
        let det = ss * cc - sc * sc;
        let a = (ys * cc - yc * sc) / det;
        let b = (yc * ss - ys * sc) / det;
        let residual = signal
            .iter()
            .enumerate()
            .map(|(n, y)| {
                let (s, c) = (2.0 * PI * frequency * n as f64).sin_cos();
                (f64::from(*y) - a * s - b * c).powi(2)
            })
            .sum::<f64>()
            / signal.len() as f64;
        ((a * a + b * b).sqrt(), residual.sqrt())
    }

    fn db(gain: f64) -> f64 {
        20.0 * gain.max(1e-12).log10()
    }

    #[test]
    fn steps_choose_the_level_and_a_cutoff_low_enough() {
        assert_eq!(Step::new(1.0), Step::new(0.25));
        assert_eq!((Step::new(1.0).level, Step::new(1.0).table), (0, 0));
        assert_eq!((Step::new(1.5).level, Step::new(1.5).table), (0, 5));
        assert_eq!((Step::new(2.0).level, Step::new(2.0).table), (1, 0));
        assert_eq!((Step::new(-3.9).level, Step::new(-3.9).table), (1, 8));
        assert_eq!(Step::new(1000.0).level, LEVELS - 1);
        // The kernel's cutoff is at or below the one the step needs.
        for step in [1.01_f32, 1.2, 1.44, 1.7, 1.99] {
            let chosen = (Step::new(step).table as f32 / STEPS as f32).exp2();
            assert!(chosen >= step && chosen < step * 1.1, "{step}: {chosen}");
        }
    }

    #[test]
    fn a_still_or_whole_step_read_returns_the_recording() {
        let samples = sine(0.123, 400);
        let map = MipMap::new(&samples);
        assert_eq!(map.frames(), 400);
        assert_eq!(map.samples(), &samples[..]);
        let played = play(&map, 1.0, 100.0, 200);
        for (played, original) in played.iter().zip(&samples[100..]) {
            assert!((played - original).abs() < 2e-4, "{played} {original}");
        }
        // Outside the recording a read is silence, far outside too.
        let bank = SincBank::shared();
        for position in [-1000.0, 1e9] {
            assert_eq!(map.read(&map.tap(bank, &Step::new(1.0), position)), 0.0);
        }
    }

    #[test]
    fn transposing_keeps_the_passband_flat() {
        let frames = 1 << 15;
        // A tone at 0.3 of the band, played from a step of a half up to just
        // below two (one level), then from the next level.
        for (tone, step) in [
            (0.15, 0.5_f32),
            (0.15, 1.0),
            (0.15, 1.37),
            (0.12, 1.9),
            (0.1, 3.3),
            (0.04, 7.0),
            (0.35, 0.77),
        ] {
            let map = MipMap::new(&sine(tone, frames));
            let played = play(&map, step, 300.0, 2000);
            let (amplitude, residual) = fit(&played, tone * f64::from(step));
            assert!(db(amplitude).abs() < 0.01, "{tone}×{step}: {amplitude}");
            assert!(
                db(residual) < -90.0,
                "{tone}×{step}: residual {}",
                db(residual)
            );
        }
    }

    #[test]
    fn transposing_up_does_not_alias_into_the_band() {
        let frames = 1 << 15;
        // Each tone lands above the output band, where it would fold back
        // to 0.84–0.88 of it: everything left of it is aliasing.
        for (tone, step) in [
            (0.30, 1.9_f32),
            (0.44, 1.25),
            (0.15, 3.8),
            (0.08, 7.1),
            (0.04, 14.2),
        ] {
            let map = MipMap::new(&sine(tone, frames));
            let played = play(&map, step, 600.0, 2000);
            let rms = (played.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>()
                / played.len() as f64)
                .sqrt();
            // Against the tone's own RMS of −3 dB.
            assert!(
                db(rms) - db(0.5_f64.sqrt()) < -85.0,
                "{tone}×{step}: {}",
                db(rms)
            );
        }
    }

    #[test]
    fn a_moving_step_stays_on_the_waveform_across_levels() {
        // A glide over four octaves through every level change: the read
        // follows the recording without a step where the level changes.
        let tone = 0.01;
        let map = MipMap::new(&sine(tone, 1 << 16));
        let bank = SincBank::shared();
        let mut position = 100.0;
        let mut largest = 0.0_f64;
        for n in 0..4000 {
            let step = (n as f32 / 1000.0).exp2();
            let value = map.read(&map.tap(bank, &Step::new(step), position));
            let expected = (2.0 * PI * tone * position).sin();
            largest = largest.max((f64::from(value) - expected).abs());
            position += f64::from(step);
        }
        assert!(db(largest) < -70.0, "{}", db(largest));
    }
}
