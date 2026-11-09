use super::*;
use crate::stretch::prepare_processed;
use daw_core::AudioData;
use std::{f64::consts::TAU, sync::atomic::Ordering};

fn streamed_tone(path: &std::path::Path, frames: usize, channels: usize) -> AudioFile {
    let mut writer = hound::WavWriter::new(
        BufWriter::new(File::create(path).unwrap()),
        hound::WavSpec {
            channels: channels as u16,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..frames {
        let value = (TAU * 997.0 * frame as f64 / 48000.0).sin() as f32 * 0.25;
        writer.write_sample(value).unwrap();
        if channels == 2 {
            writer.write_sample(value * 0.5).unwrap();
        }
    }
    writer.finalize().unwrap();
    AudioFile::open(path).unwrap()
}
fn read(asset: &Arc<AudioAsset>) -> Vec<f32> {
    let mut output = vec![0.0; asset.frames() * asset.channels()];
    assert_eq!(
        asset
            .file()
            .unwrap()
            .reader()
            .unwrap()
            .read_frames(0, &mut output)
            .unwrap(),
        asset.frames()
    );
    output
}

fn assert_exact(actual: &[f32], expected: &[f32], context: &str) {
    assert_eq!(actual.len(), expected.len(), "{context}: sample count");
    if let Some(index) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        panic!(
            "{context}: sample {index}: {} vs {}",
            actual[index], expected[index]
        );
    }
}

#[test]
fn streamed_stretch_matches_continuous_resident_history_and_exact_tail_frames() {
    let directory = tempfile::tempdir().unwrap();
    for channels in [1, 2] {
        let source = streamed_tone(
            &directory.path().join(format!("{channels}.wav")),
            48013,
            channels,
        );
        let resident =
            AudioData::load_wav(directory.path().join(format!("{channels}.wav"))).unwrap();
        for algorithm in AudioProcessingAlgorithm::ALL {
            for (rate, semitones) in [
                (0.25, -7.25),
                (0.7, 3.01),
                (1.0, 0.0),
                (1.0, 7.25),
                (2.0, -7.25),
                (4.0, 24.0),
            ] {
                let expected =
                    prepare_processed(&resident, rate, semitones, algorithm, None).unwrap();
                let actual = prepare_file(&source, rate, semitones, algorithm, None).unwrap();
                assert!(actual.is_streamed());
                assert_eq!(actual.resident_bytes(), 0);
                assert_eq!(actual.frames(), expected.frames());
                assert_eq!(actual.channels(), expected.channels);
                assert_eq!(actual.sample_rate(), expected.sample_rate);
                assert_exact(
                    &read(&actual),
                    &expected.samples,
                    &format!("{algorithm:?}, {channels}ch, rate{rate}, pitch{semitones}"),
                );
            }
        }
    }
}

#[test]
fn streamed_stretch_preserves_short_boundary_transients_and_silent_channel() {
    let directory = tempfile::tempdir().unwrap();
    for frames in [0, 1, 10, 1024, 5000] {
        let mut resident = AudioData {
            sample_rate: 48000,
            channels: 2,
            samples: vec![0.0; frames * 2],
        };
        if frames > 0 {
            resident.samples[0] = 0.25;
            resident.samples[(frames - 1) * 2] = 0.25;
        }
        let path = directory.path().join(format!("{frames}.wav"));
        resident.write_wav(&path).unwrap();
        let file = AudioFile::open(path).unwrap();
        for algorithm in AudioProcessingAlgorithm::ALL {
            for rate in [0.25, 4.0] {
                let expected = prepare_processed(&resident, rate, 7.0, algorithm, None).unwrap();
                let actual = prepare_file(&file, rate, 7.0, algorithm, None).unwrap();
                assert_exact(
                    &read(&actual),
                    &expected.samples,
                    &format!("{algorithm:?}, {frames}frames, rate{rate}"),
                );
            }
        }
    }
}

#[test]
fn large_source_is_prepared_as_disk_audio_with_correct_pitch_duration_and_stereo() {
    let _cpu = crate::allocation_test_support::heavy_audio_test_guard();
    let directory = tempfile::tempdir().unwrap();
    // Exceeds the 32 MiB resident threshold; constructing it never allocates a
    // source-length PCM vector, and preparation returns only a leased disk file.
    let frames = 32 * 1024 * 1024 / 8 + 37;
    let source = streamed_tone(&directory.path().join("large.wav"), frames, 2);
    for algorithm in AudioProcessingAlgorithm::ALL {
        let asset = prepare_file(&source, 4.0, 7.25, algorithm, None).unwrap();
        assert!(asset.is_streamed());
        assert_eq!(asset.resident_bytes(), 0);
        assert_eq!(asset.frames(), (frames as f64 / 4.0).ceil() as usize);
        let mut middle = vec![0.0; 8192 * 2];
        assert_eq!(
            asset
                .file()
                .unwrap()
                .reader()
                .unwrap()
                .read_frames(asset.frames() / 2, &mut middle)
                .unwrap(),
            8192
        );
        let mono: Vec<_> = middle
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| frame[0])
            .collect();
        let crossings = mono
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        let hz = crossings as f64 * 48000.0 / mono.len() as f64;
        let expected = 997.0 * 2.0_f64.powf(7.25 / 12.0);
        assert!(
            (hz - expected).abs() < 16.0,
            "{algorithm:?}: {hz}Hz vs{expected}"
        );
        let rms = (mono
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            / mono.len() as f64)
            .sqrt();
        assert!(rms > 0.08 && rms < 0.4, "{algorithm:?}: RMS{rms}");
        assert!(
            middle
                .as_chunks::<2>()
                .0
                .iter()
                .all(|frame| (frame[0] * 0.5 - frame[1]).abs() < 0.001)
        );
    }
    let cancel = AtomicBool::new(false);
    for algorithm in AudioProcessingAlgorithm::ALL {
        cancel.store(false, Ordering::Release);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(10));
                cancel.store(true, Ordering::Release);
            });
            let Err(error) = prepare_file(&source, 0.25, 7.0, algorithm, Some(&cancel)) else {
                panic!("Cancellation stops disk preparation");
            };
            assert!(
                error.to_string().contains("cancelled"),
                "{algorithm:?}: {error:#}"
            );
        });
    }
}

#[test]
fn private_prepared_writer_checks_frames_finiteness_cancellation_and_removes_partial_files() {
    let mut writer = StreamWriter::new(48000, 2, 4, None).unwrap();
    let path = writer.lease.path().to_path_buf();
    assert!(writer.write(&[f32::NAN, 0.0]).is_err());
    assert!(writer.write(&[0.0]).is_err());
    assert!(writer.write(&[0.0; 10]).is_err());
    writer.write(&[0.25, 0.5]).unwrap();
    assert!(writer.finish().is_err());
    assert!(!path.exists());
    let cancel = AtomicBool::new(false);
    let mut writer = StreamWriter::new(48000, 1, 4, Some(&cancel)).unwrap();
    let path = writer.lease.path().to_path_buf();
    writer.write(&[0.25; 4]).unwrap();
    cancel.store(true, Ordering::Release);
    assert!(writer.finish().is_err());
    assert!(!path.exists());
    assert!(StreamWriter::new(48000, 2, u32::MAX as usize, None).is_err());
    assert!(StreamWriter::new(48000, 3, 4, None).is_err());
    assert!(StreamWriter::new(0, 1, 4, None).is_err());
}
