//! Source-rate conversion is prepared once per source, before playback starts.
//! High uses Rubato's synchronous FFT low-pass resampler; `process_all` trims
//! its startup delay and flushes the tail. No conversion runs in the device callback.
//! Reference: https://github.com/HEnquist/rubato (v5.0.1, Resampler::process_all).
use anyhow::{Context, Result, ensure};
use daw_core::AudioData;
use rubato::{Fft, FixedSync, Resampler, audioadapter_buffers::direct::InterleavedSlice};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(crate) const MAX_ASSET_BYTES: usize = 512 * 1024 * 1024;
pub(crate) const MAX_CACHE_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
/// Conversion used while preparing source audio at the output device/export rate.
pub enum ResampleQuality {
    /// Filtered FFT conversion with its processing delay removed.
    #[default]
    High,
    /// Linear interpolation; quicker preparation but no anti-alias filter.
    Fast,
}

impl ResampleQuality {
    pub fn name(self) -> &'static str {
        match self {
            Self::High => "High quality",
            Self::Fast => "Fast",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// Shared playback/export preparation options. Buffers stay f32 and mix sums f64.
pub struct RenderOptions {
    pub resample_quality: ResampleQuality,
}

pub(crate) fn load_source(path: &Path) -> Result<AudioData> {
    let reader =
        hound::WavReader::open(path).with_context(|| format!("Open WAV {}", path.display()))?;
    let decoded_bytes = u64::from(reader.len()) * size_of::<f32>() as u64;
    ensure!(
        decoded_bytes <= MAX_ASSET_BYTES as u64,
        "Audio source exceeds the 512 MiB decoded asset limit"
    );
    drop(reader);
    AudioData::load_wav(path)
}

pub(crate) fn prepare_audio(
    mut source: AudioData,
    target_rate: u32,
    quality: ResampleQuality,
) -> Result<AudioData> {
    source.validate()?;
    ensure!(
        (8_000..=384_000).contains(&target_rate),
        "Unsupported output sample rate"
    );
    ensure!(
        (1..=768_000).contains(&source.sample_rate),
        "Unsupported source sample rate"
    );
    // Tracks are stereo; preserve the existing first-two-channel mapping without
    // paying to resample extra channels the graph will never read.
    if source.channels > 2 {
        source.samples = source
            .samples
            .chunks_exact(source.channels)
            .flat_map(|frame| [frame[0], frame[1]])
            .collect();
        source.channels = 2;
    }
    if source.sample_rate == target_rate || source.samples.is_empty() {
        source.sample_rate = target_rate;
        return Ok(source);
    }
    let ratio = f64::from(target_rate) / f64::from(source.sample_rate);
    ensure!(
        (1.0 / 128.0..=128.0).contains(&ratio),
        "Sample-rate ratio exceeds the supported preparation range"
    );
    let frames = (source.frames() as u128 * u128::from(target_rate))
        .div_ceil(u128::from(source.sample_rate));
    let samples = frames * source.channels as u128;
    ensure!(
        samples * size_of::<f32>() as u128 <= MAX_ASSET_BYTES as u128,
        "Resampled audio exceeds the 512 MiB asset limit"
    );
    let frames = frames as usize;
    let samples = samples as usize;
    let mut output = match quality {
        ResampleQuality::High => {
            let mut resampler = Fft::<f32>::new(
                source.sample_rate as usize,
                target_rate as usize,
                1024,
                source.channels,
                FixedSync::Both,
            )?;
            let needed = resampler.process_all_needed_output_len(source.frames());
            ensure!(
                needed
                    .checked_mul(source.channels)
                    .and_then(|n| n.checked_mul(size_of::<f32>()))
                    .is_some_and(|n| n <= MAX_ASSET_BYTES + 32 * 1024 * 1024),
                "Resampler workspace exceeds the asset limit"
            );
            let input = InterleavedSlice::new(&source.samples, source.channels, source.frames())?;
            resampler
                .process_all(&input, source.frames(), None)?
                .take_data()
        }
        ResampleQuality::Fast => {
            let mut output = Vec::new();
            output
                .try_reserve_exact(samples)
                .context("Allocate resampled audio")?;
            let ratio = f64::from(source.sample_rate) / f64::from(target_rate);
            for frame in 0..frames {
                let position = frame as f64 * ratio;
                let index = (position.floor() as usize).min(source.frames() - 1);
                let next = (index + 1).min(source.frames() - 1);
                let fraction = position - index as f64;
                for channel in 0..source.channels {
                    let a = f64::from(source.samples[index * source.channels + channel]);
                    let b = f64::from(source.samples[next * source.channels + channel]);
                    output.push((a + (b - a) * fraction) as f32);
                }
            }
            output
        }
    };
    ensure!(
        output.len() >= samples,
        "Resampler returned too few samples"
    );
    output.truncate(samples);
    let audio = AudioData {
        sample_rate: target_rate,
        channels: source.channels,
        samples: output,
    };
    audio.validate()?;
    Ok(audio)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(rate: u32, hz: f64) -> AudioData {
        AudioData {
            sample_rate: rate,
            channels: 1,
            samples: (0..rate as usize)
                .map(|i| (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin() as f32)
                .collect(),
        }
    }
    fn rms(audio: &AudioData) -> f64 {
        let edge = (audio.sample_rate / 20) as usize;
        let samples = &audio.samples[edge..audio.samples.len() - edge];
        (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
    }
    #[test]
    fn high_quality_rejects_out_of_band_aliases_and_preserves_passband() {
        let source = tone(96_000, 30_000.0);
        let high = prepare_audio(source.clone(), 44_100, ResampleQuality::High).unwrap();
        let fast = prepare_audio(source, 44_100, ResampleQuality::Fast).unwrap();
        assert_eq!(high.frames(), 44_100);
        assert!(rms(&high) < 0.0001, "alias level {}", rms(&high));
        assert!(rms(&fast) > 0.2, "linear comparison must expose aliasing");
        let pass = prepare_audio(tone(96_000, 1_000.0), 44_100, ResampleQuality::High).unwrap();
        assert!((rms(&pass) - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.002);
    }
    #[test]
    fn latency_is_trimmed_and_short_stereo_tails_are_not_lost() {
        for length in [1, 17, 1023, 1031] {
            let mut data = AudioData {
                sample_rate: 44_100,
                channels: 2,
                samples: vec![0.0; length * 2],
            };
            data.samples[(length / 2) * 2] = 1.0;
            data.samples[(length / 2) * 2 + 1] = -0.5;
            let high = prepare_audio(data, 48_000, ResampleQuality::High).unwrap();
            assert_eq!(high.frames(), (length * 48_000).div_ceil(44_100));
            let peak = high
                .samples
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a[0].abs().total_cmp(&b[0].abs()))
                .unwrap()
                .0;
            let expected = (length / 2) as f64 * 48_000.0 / 44_100.0;
            assert!(
                (peak as f64 - expected).abs() <= 1.0,
                "delayed impulse {peak}/{expected}"
            );
            assert!(
                high.samples
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .all(|f| (f[0] * 0.5 + f[1]).abs() < 1e-6)
            );
        }
    }
    #[test]
    fn native_rate_is_bit_exact_and_empty_sources_are_supported() {
        let source = tone(48_000, 997.0);
        let samples = source.samples.clone();
        assert_eq!(
            prepare_audio(source, 48_000, ResampleQuality::High)
                .unwrap()
                .samples,
            samples
        );
        let empty = AudioData {
            sample_rate: 44_100,
            channels: 2,
            samples: vec![],
        };
        assert_eq!(
            prepare_audio(empty, 48_000, ResampleQuality::High)
                .unwrap()
                .frames(),
            0
        );
    }

    #[test]
    fn preparation_rejects_unbounded_ratios_and_output_before_allocation() {
        let unreasonable = AudioData {
            sample_rate: 1,
            channels: 1,
            samples: vec![0.0],
        };
        assert!(
            prepare_audio(unreasonable, 48_000, ResampleQuality::High)
                .unwrap_err()
                .to_string()
                .contains("ratio")
        );
        // A 4 MiB source would expand past 512 MiB. Reject it before FFT setup
        // or output allocation; this test itself only needs the small input.
        let too_large = AudioData {
            sample_rate: 3_000,
            channels: 1,
            samples: vec![0.0; MAX_ASSET_BYTES / 128 / size_of::<f32>() + 1],
        };
        assert!(
            prepare_audio(too_large, 384_000, ResampleQuality::High)
                .unwrap_err()
                .to_string()
                .contains("512 MiB")
        );
    }

    #[test]
    #[ignore = "manual release-mode source preparation throughput measurement"]
    fn preparation_throughput() {
        for (source_rate, target_rate) in [(44_100, 48_000), (96_000, 44_100)] {
            for quality in [ResampleQuality::High, ResampleQuality::Fast] {
                let source = AudioData {
                    sample_rate: source_rate,
                    channels: 2,
                    samples: (0..source_rate * 30)
                        .flat_map(|frame| {
                            let value = (f64::from(frame) * 0.017).sin() as f32;
                            [value, -value]
                        })
                        .collect(),
                };
                let start = std::time::Instant::now();
                let prepared = prepare_audio(source, target_rate, quality).unwrap();
                let elapsed = start.elapsed();
                assert_eq!(prepared.frames(), target_rate as usize * 30);
                eprintln!(
                    "{quality:?} {source_rate}->{target_rate}: 30s stereo in {elapsed:?} ({:.1}x realtime)",
                    30.0 / elapsed.as_secs_f64()
                );
            }
        }
    }
}
