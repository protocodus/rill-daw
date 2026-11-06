use daw_core::{AudioData, MAX_IMPORTED_AUDIO_BYTES, decode_audio_to_wav};
use std::path::{Path, PathBuf};

fn fixture(extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio-import")
        .join(format!("tone.{extension}"))
}

#[test]
fn real_formats_decode_to_finite_gapless_float_wav_without_changing_sources() {
    for extension in ["wav", "mp3", "flac", "ogg"] {
        let directory = tempfile::tempdir().unwrap();
        let input = fixture(extension);
        let before = std::fs::read(&input).unwrap();
        let output = directory.path().join("decoded.wav");
        let info = decode_audio_to_wav(&input, &output, MAX_IMPORTED_AUDIO_BYTES)
            .unwrap_or_else(|e| panic!("{extension}: {e:#}"));
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.channels, 2);
        assert_eq!(
            info.frames, 5_760,
            "{extension} must trim encoder delay/padding"
        );
        assert_eq!(info.decoded_bytes, 5_760 * 2 * 4);
        assert_eq!(info.source_format, extension.to_uppercase());
        let spec = hound::WavReader::open(&output).unwrap().spec();
        assert_eq!(spec.sample_format, hound::SampleFormat::Float);
        assert_eq!(spec.bits_per_sample, 32);
        let audio = AudioData::load_wav(&output).unwrap();
        for (channel, frequency, expected) in [(0, 440.0, 0.4), (1, 880.0, 0.25)] {
            let mut sine = 0.0;
            let mut cosine = 0.0;
            for (frame, samples) in audio.samples.chunks_exact(2).enumerate() {
                let phase = frame as f64 * std::f64::consts::TAU * frequency / 48_000.0;
                sine += f64::from(samples[channel]) * phase.sin();
                cosine += f64::from(samples[channel]) * phase.cos();
            }
            let amplitude = 2.0 * sine.hypot(cosine) / 5_760.0;
            assert!(
                (amplitude - expected).abs() < 0.05,
                "{extension} channel{channel} amplitude {amplitude}"
            );
        }
        assert_eq!(std::fs::read(input).unwrap(), before);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn lossless_sources_retain_samples_and_decoder_uses_content_not_extension() {
    let original = AudioData::load_wav(fixture("wav")).unwrap();
    for extension in ["wav", "flac"] {
        let directory = tempfile::tempdir().unwrap();
        let disguised = directory.path().join("misleading.mp3");
        std::fs::copy(fixture(extension), &disguised).unwrap();
        let output = directory.path().join("out.wav");
        decode_audio_to_wav(&disguised, &output, MAX_IMPORTED_AUDIO_BYTES).unwrap();
        let decoded = AudioData::load_wav(output).unwrap();
        assert_eq!(decoded.samples.len(), original.samples.len());
        assert!(
            decoded
                .samples
                .iter()
                .zip(&original.samples)
                .all(|(a, b)| (a - b).abs() <= 2.0e-7)
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let disguised = directory.path().join("audio.data");
    std::fs::copy(fixture("mp3"), &disguised).unwrap();
    let info = decode_audio_to_wav(
        &disguised,
        &directory.path().join("out.wav"),
        MAX_IMPORTED_AUDIO_BYTES,
    )
    .unwrap();
    assert_eq!(info.source_format, "MP3");
}

#[test]
fn decoded_budget_is_enforced_and_exact_budget_is_accepted_for_every_format() {
    for extension in ["wav", "mp3", "flac", "ogg"] {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("out.wav");
        assert!(decode_audio_to_wav(&fixture(extension), &output, 8_192).is_err());
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            0,
            "Partial {extension} must be removed"
        );
        let info = decode_audio_to_wav(&fixture(extension), &output, 46_080).unwrap();
        assert_eq!(info.decoded_bytes, 46_080);
    }
}

#[test]
fn untagged_mp3_retains_valid_padding_and_partial_frames_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let source = fixture("mp3").with_file_name("tone-cbr-no-tags.mp3");
    let output = directory.path().join("out.wav");
    let info = decode_audio_to_wav(&source, &output, MAX_IMPORTED_AUDIO_BYTES).unwrap();
    assert!(
        (5_760..=9_000).contains(&info.frames),
        "Without encoder metadata, padding cannot be invented or trimmed"
    );
    let bytes = std::fs::read(&source).unwrap();
    let truncated = directory.path().join("partial.mp3");
    std::fs::write(&truncated, &bytes[..bytes.len() - 1]).unwrap();
    assert!(
        decode_audio_to_wav(
            &truncated,
            &directory.path().join("partial.wav"),
            MAX_IMPORTED_AUDIO_BYTES
        )
        .is_err()
    );
}

#[test]
fn truncated_and_corrupt_sources_do_not_leave_decoded_or_temporary_files() {
    for extension in ["wav", "mp3", "flac", "ogg"] {
        let bytes = std::fs::read(fixture(extension)).unwrap();
        for length in [4, bytes.len() / 2, bytes.len() - 1] {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join(format!("broken.{extension}"));
            let output = directory.path().join("out.wav");
            std::fs::write(&source, &bytes[..length]).unwrap();
            let result = decode_audio_to_wav(&source, &output, MAX_IMPORTED_AUDIO_BYTES);
            assert!(
                result.is_err(),
                "Truncated {extension} ({length}/{}) was accepted",
                bytes.len()
            );
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut bytes = std::fs::read(fixture("ogg")).unwrap();
    let length = bytes.len();
    bytes[length - 20] ^= 0x80;
    let input = directory.path().join("corrupt.ogg");
    std::fs::write(&input, bytes).unwrap();
    assert!(
        decode_audio_to_wav(
            &input,
            &directory.path().join("out.wav"),
            MAX_IMPORTED_AUDIO_BYTES
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn nonfinite_unsupported_and_oversized_audio_is_rejected_without_replacing_files() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.wav");
    for (name, channels, value) in [
        ("nan.wav", 2, f32::NAN),
        ("infinity.wav", 1, f32::INFINITY),
        ("surround.wav", 6, 0.5),
    ] {
        let input = directory.path().join(name);
        let mut writer = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels,
                sample_rate: 48_000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..channels {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
        assert!(decode_audio_to_wav(&input, &output, MAX_IMPORTED_AUDIO_BYTES).is_err());
        assert!(!output.exists());
    }
    let oversized = directory.path().join("oversized.wav");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(MAX_IMPORTED_AUDIO_BYTES as u64 + 1)
        .unwrap();
    assert!(
        decode_audio_to_wav(&oversized, &output, MAX_IMPORTED_AUDIO_BYTES)
            .unwrap_err()
            .to_string()
            .contains("Encoded audio")
    );
    let original = b"never overwrite an existing asset";
    std::fs::write(&output, original).unwrap();
    assert!(decode_audio_to_wav(&fixture("wav"), &output, MAX_IMPORTED_AUDIO_BYTES).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), original);
    let source = fixture("wav");
    let before = std::fs::read(&source).unwrap();
    assert!(decode_audio_to_wav(&source, &source, MAX_IMPORTED_AUDIO_BYTES).is_err());
    assert_eq!(std::fs::read(source).unwrap(), before);
}
