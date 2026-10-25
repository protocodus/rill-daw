//! Bounded source-spectrum overviews for asynchronous visual analysis only.
//! Overlapping windows cover the entire source; their peaks are pooled into
//! uniformly spaced time cells. No playback state or processed audio changes.
use crate::AudioData;
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex};
use std::sync::atomic::{AtomicBool, Ordering};

pub const SPECTROGRAM_BANDS: usize = 48;
pub const SPECTROGRAM_MAX_COLUMNS: usize = 1024;
pub const SPECTROGRAM_FLOOR_DB: f32 = -90.0;
pub const SPECTROGRAM_CEILING_DB: f32 = -6.0;

#[derive(Clone, Debug)]
pub struct AudioSpectrogram {
    pub sample_rate: u32,
    pub source_frames: usize,
    pub window_frames: usize,
    pub columns: usize,
    pub bands: usize,
    pub min_frequency: f32,
    pub max_frequency: f32,
    /// Time-major [column * bands + band], with band zero at the lowest
    /// frequency. Each value uses the same absolute -90..-6 dBFS scale.
    pub levels: Vec<u8>,
}
impl AudioSpectrogram {
    pub fn band_frequency(&self, band: usize) -> Option<f32> {
        (band < self.bands).then(|| {
            self.min_frequency
                * (self.max_frequency / self.min_frequency)
                    .powf((band as f32 + 0.5) / self.bands as f32)
        })
    }
}

/// Hann-window FFTs, with log-frequency peak bands and mean channel power.
/// Anti-phase stereo cannot cancel. Absolute normalization keeps a quiet clip
/// quiet instead of boosting each time column to full brightness. Planning and
/// scratch allocation occur once; every window reuses bounded buffers.
pub fn compute_audio_spectrogram(data: &AudioData) -> Result<AudioSpectrogram> {
    compute_audio_spectrogram_cancellable(data, &AtomicBool::new(false))
}

/// Cancellation is checked before validation and at each fixed-hop window so
/// a discarded preview does not hold the shared background worker indefinitely.
pub fn compute_audio_spectrogram_cancellable(
    data: &AudioData,
    cancelled: &AtomicBool,
) -> Result<AudioSpectrogram> {
    ensure!(
        !cancelled.load(Ordering::Relaxed),
        "Spectral preview cancelled"
    );
    ensure!(
        data.samples.len() <= 512 * 1024 * 1024 / size_of::<f32>(),
        "Spectral preview exceeds the 512 MiB source limit"
    );
    data.validate()?;
    ensure!(
        data.channels <= 32,
        "Spectral previews support up to 32 audio channels"
    );
    let source_frames = data.frames();
    // About 24 Hz bins at ordinary sample rates, bounded at both extremes.
    let window_frames = ((data.sample_rate / 24).clamp(512, 8192) as usize)
        .next_power_of_two()
        .min(8192);
    let columns = source_frames
        .div_ceil(window_frames / 4)
        .min(SPECTROGRAM_MAX_COLUMNS);
    let max_frequency = data.sample_rate as f32 * 0.5;
    let min_frequency = 40.0_f32.min(max_frequency * 0.5);
    let mut result = AudioSpectrogram {
        sample_rate: data.sample_rate,
        source_frames,
        window_frames,
        columns,
        bands: SPECTROGRAM_BANDS,
        min_frequency,
        max_frequency,
        levels: vec![0; columns * SPECTROGRAM_BANDS],
    };
    if columns == 0 {
        return Ok(result);
    }
    let window: Vec<_> = (0..window_frames)
        .map(|index| {
            0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / window_frames as f32).cos()
        })
        .collect();
    let amplitude_scale = 2.0 / window.iter().sum::<f32>();
    let power_scale = amplitude_scale * amplitude_scale / data.channels as f32;
    let log_range = (max_frequency / min_frequency).ln();
    let bin_bands: Vec<_> = (0..=window_frames / 2)
        .map(|bin| {
            let frequency = bin as f32 * data.sample_rate as f32 / window_frames as f32;
            (frequency >= min_frequency).then(|| {
                (((frequency / min_frequency).ln() / log_range * SPECTROGRAM_BANDS as f32) as usize)
                    .min(SPECTROGRAM_BANDS - 1)
            })
        })
        .collect();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(window_frames);
    let mut input = vec![Complex::new(0.0, 0.0); window_frames];
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    let mut powers = vec![0.0_f32; window_frames / 2 + 1];
    let hop = window_frames / 4;
    for step in 0..source_frames.div_ceil(hop) {
        ensure!(
            !cancelled.load(Ordering::Relaxed),
            "Spectral preview cancelled"
        );
        powers.fill(0.0);
        let center = (step * hop) as isize;
        let column = ((center as f64 / source_frames as f64) * columns as f64) as usize;
        let start = center - window_frames as isize / 2;
        for channel in 0..data.channels {
            let mut has_signal = false;
            for (index, value) in input.iter_mut().enumerate() {
                let frame = start + index as isize;
                let sample = if frame >= 0 && (frame as usize) < source_frames {
                    // Display headroom is already saturated above -6 dBFS.
                    // Bound extreme finite assets to keep FFT arithmetic finite.
                    data.samples[frame as usize * data.channels + channel].clamp(-1e12, 1e12)
                } else {
                    0.0
                };
                has_signal |= sample != 0.0;
                *value = Complex::new(sample * window[index], 0.0);
            }
            if !has_signal {
                continue;
            }
            fft.process_with_scratch(&mut input, &mut scratch);
            for (bin, power) in powers.iter_mut().enumerate().skip(1) {
                let normalization = if bin == window_frames / 2 { 0.25 } else { 1.0 };
                *power += input[bin].norm_sqr() * power_scale * normalization;
            }
        }
        let mut bands = [0.0_f32; SPECTROGRAM_BANDS];
        for (power, band) in powers.iter().zip(&bin_bands) {
            if let Some(band) = band {
                bands[*band] = bands[*band].max(*power);
            }
        }
        for (band, power) in bands.into_iter().enumerate() {
            let db = 10.0 * power.max(1e-20).log10();
            let normalized = ((db - SPECTROGRAM_FLOOR_DB)
                / (SPECTROGRAM_CEILING_DB - SPECTROGRAM_FLOOR_DB))
                .clamp(0.0, 1.0);
            let target = &mut result.levels[column * SPECTROGRAM_BANDS + band];
            *target = (*target).max((normalized * 255.0).round() as u8);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signal(frames: usize, channels: usize, amplitude: f32, reverse_phase: bool) -> AudioData {
        AudioData {
            sample_rate: 48000,
            channels,
            samples: (0..frames)
                .flat_map(|frame| {
                    let frequency = if frame < frames / 2 { 440.0 } else { 4000.0 };
                    let value = amplitude
                        * (std::f32::consts::TAU * frequency * frame as f32 / 48000.0).sin();
                    (0..channels).map(move |channel| {
                        if channel % 2 == 1 && reverse_phase {
                            -value
                        } else {
                            value
                        }
                    })
                })
                .collect(),
        }
    }
    fn dominant(spectrum: &AudioSpectrogram, column: usize) -> (f32, u8) {
        let (band, level) = spectrum.levels[column * spectrum.bands..(column + 1) * spectrum.bands]
            .iter()
            .enumerate()
            .max_by_key(|(_, value)| **value)
            .unwrap();
        (spectrum.band_frequency(band).unwrap(), *level)
    }
    #[test]
    fn tones_stay_in_their_frequency_and_time_regions() {
        let spectrum = compute_audio_spectrogram(&signal(96000, 1, 0.5, false)).unwrap();
        let low = dominant(&spectrum, spectrum.columns / 4);
        let high = dominant(&spectrum, spectrum.columns * 3 / 4);
        assert!((380.0..510.0).contains(&low.0), "{low:?}");
        assert!((3500.0..4500.0).contains(&high.0), "{high:?}");
        assert!(low.1 > 230 && high.1 > 230);
        assert_eq!(spectrum.levels.len(), spectrum.columns * SPECTROGRAM_BANDS);
    }
    #[test]
    fn antiphase_channels_combine_power_without_cancelling() {
        let mono = compute_audio_spectrogram(&signal(48000, 1, 0.25, false)).unwrap();
        let stereo = compute_audio_spectrogram(&signal(48000, 2, 0.25, true)).unwrap();
        assert_eq!(mono.levels, stereo.levels);
    }
    #[test]
    fn quiet_signal_is_not_normalized_to_full_brightness() {
        let loud = compute_audio_spectrogram(&signal(48000, 1, 0.5, false)).unwrap();
        let quiet = compute_audio_spectrogram(&signal(48000, 1, 0.005, false)).unwrap();
        let a = dominant(&loud, loud.columns / 4).1;
        let b = dominant(&quiet, quiet.columns / 4).1;
        assert!(a > b + 110, "loud={a}, quiet={b}");
    }
    #[test]
    fn silence_empty_and_short_sources_are_bounded_and_finite() {
        for frames in [0, 1, 7, 48000, 600000] {
            let audio = AudioData {
                sample_rate: 48000,
                channels: 1,
                samples: vec![0.0; frames],
            };
            let spectrum = compute_audio_spectrogram(&audio).unwrap();
            assert_eq!(spectrum.source_frames, frames);
            assert!(spectrum.columns <= SPECTROGRAM_MAX_COLUMNS);
            assert!(spectrum.levels.iter().all(|value| *value == 0));
            assert!(spectrum.window_frames <= 8192);
            assert!(spectrum.min_frequency > 0.0 && spectrum.max_frequency.is_finite());
        }
        let impulse = AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: vec![1.0],
        };
        assert!(
            compute_audio_spectrogram(&impulse)
                .unwrap()
                .levels
                .iter()
                .any(|value| *value > 0)
        );
    }
    #[test]
    fn localized_burst_does_not_light_the_rest_of_the_file() {
        let mut audio = signal(96000, 1, 0.5, false);
        for (frame, sample) in audio.samples.iter_mut().enumerate() {
            if !(36000..48000).contains(&frame) {
                *sample = 0.0;
            }
        }
        let spectrum = compute_audio_spectrogram(&audio).unwrap();
        assert_eq!(dominant(&spectrum, spectrum.columns / 8).1, 0);
        assert!(dominant(&spectrum, spectrum.columns * 7 / 16).1 > 200);
        assert_eq!(dominant(&spectrum, spectrum.columns * 3 / 4).1, 0);
    }
    #[test]
    fn invalid_assets_fail_without_allocating_unbounded_buffers() {
        let mut audio = signal(48, 1, 0.5, false);
        audio.samples[0] = f32::NAN;
        assert!(compute_audio_spectrogram(&audio).is_err());
        audio.samples.clear();
        audio.sample_rate = 0;
        assert!(compute_audio_spectrogram(&audio).is_err());
    }

    #[test]
    fn long_source_keeps_a_burst_between_coarse_column_centers() {
        // Each pooled column covers 8192 frames, much wider than one FFT.
        // A center-only overview would completely miss this short burst.
        let mut audio = AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: vec![0.0; 8_388_608],
        };
        let start = 400 * 8192 + 512;
        for frame in start..start + 256 {
            audio.samples[frame] =
                0.5 * (std::f32::consts::TAU * 440.0 * (frame - start) as f32 / 48000.0).sin();
        }
        let spectrum = compute_audio_spectrogram(&audio).unwrap();
        assert_eq!(spectrum.columns, SPECTROGRAM_MAX_COLUMNS);
        assert!(dominant(&spectrum, 400).1 > 150);
        assert_eq!(dominant(&spectrum, 390).1, 0);
        assert_eq!(dominant(&spectrum, 410).1, 0);
    }

    #[test]
    fn cancelled_work_returns_without_a_partial_success() {
        let cancelled = AtomicBool::new(true);
        assert!(
            compute_audio_spectrogram_cancellable(&signal(480, 1, 0.5, false), &cancelled)
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }
}
