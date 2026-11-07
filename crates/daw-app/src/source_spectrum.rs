//! Bounded sequential spectral overview. Overlapping chunks retain complete
//! FFT windows; output pools source energy into the same fixed time/frequency grid.
use daw_core::{AudioData, AudioFile, AudioSpectrogram};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn compute(file: &AudioFile, cancel: &AtomicBool) -> anyhow::Result<AudioSpectrogram> {
    anyhow::ensure!(
        !cancel.load(Ordering::Acquire),
        "Spectral preview cancelled"
    );
    let info = file.info();
    anyhow::ensure!(
        (1..=32).contains(&info.channels),
        "Spectral preview supports at most 32 channels"
    );
    let window = ((info.sample_rate / 24).clamp(512, 8192) as usize)
        .next_power_of_two()
        .min(8192);
    let hop = window / 4;
    let columns = info
        .frames
        .div_ceil(hop)
        .min(daw_core::SPECTROGRAM_MAX_COLUMNS);
    let max_frequency = info.sample_rate as f32 * 0.5;
    let mut result = AudioSpectrogram {
        sample_rate: info.sample_rate,
        source_frames: info.frames,
        window_frames: window,
        columns,
        bands: daw_core::SPECTROGRAM_BANDS,
        min_frequency: 40.0_f32.min(max_frequency * 0.5),
        max_frequency,
        levels: vec![0; columns * daw_core::SPECTROGRAM_BANDS],
    };
    if columns == 0 {
        return Ok(result);
    }
    let chunk = (65536 / info.channels).max(window * 2).div_ceil(hop) * hop;
    let mut reader = file.reader()?;
    for first in (0..info.frames).step_by(chunk) {
        anyhow::ensure!(
            !cancel.load(Ordering::Acquire),
            "Spectral preview cancelled"
        );
        let end = first.saturating_add(chunk).min(info.frames);
        let padded = first.saturating_sub(window);
        let padded_end = end.saturating_add(window).min(info.frames);
        let mut samples = vec![0.0; (padded_end - padded) * info.channels];
        let count = reader.read_frames(padded, &mut samples)?;
        anyhow::ensure!(
            count == padded_end - padded,
            "Audio source ended during spectral preview"
        );
        let audio = AudioData {
            sample_rate: info.sample_rate,
            channels: info.channels,
            samples,
        };
        let part = daw_core::compute_audio_spectrogram_cancellable(&audio, cancel)?;
        for local in 0..part.columns {
            let center = padded + local * hop;
            if center < first || center >= end {
                continue;
            }
            let column = ((center as f64 / info.frames as f64) * columns as f64) as usize;
            if column >= columns {
                continue;
            }
            for band in 0..part.bands {
                let value = part.levels[local * part.bands + band];
                result.levels[column * part.bands + band] =
                    result.levels[column * part.bands + band].max(value);
            }
        }
    }
    file.check_unchanged()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunked_spectrum_matches_resident_reference_across_chunk_edges_and_stereo_phase() {
        let directory = tempfile::tempdir().unwrap();
        for (rate, channels) in [(8000, 1), (48000, 2), (192000, 2)] {
            let mut samples = Vec::with_capacity(140001 * channels);
            for frame in 0..140001 {
                let tone =
                    (std::f64::consts::TAU * 997.0 * frame as f64 / rate as f64).sin() as f32 * 0.4;
                for channel in 0..channels {
                    samples.push(if channel == 0 { tone } else { -tone });
                }
            }
            for frame in [0, 16383, 32767, 32768, 65535, 65536, 140000] {
                samples[frame * channels] = 0.95;
            }
            let data = AudioData {
                sample_rate: rate,
                channels,
                samples,
            };
            let path = directory.path().join(format!("spectrum-{rate}.wav"));
            data.write_wav(&path).unwrap();
            let file = AudioFile::open(path).unwrap();
            let actual = compute(&file, &AtomicBool::new(false)).unwrap();
            let expected = daw_core::compute_audio_spectrogram(&data).unwrap();
            assert_eq!(actual.source_frames, expected.source_frames);
            assert_eq!(actual.columns, expected.columns);
            assert_eq!(actual.window_frames, expected.window_frames);
            assert_eq!(
                actual.levels, expected.levels,
                "chunk boundaries must neither drop energy nor alter normalization at {rate}"
            );
            assert!(compute(&file, &AtomicBool::new(true)).is_err());
        }
    }
}
