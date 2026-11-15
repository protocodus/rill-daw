//! Header metadata and additional audio containers use checked-in original tones.
//! No external codec process or FFmpeg installation is required by these tests.
use daw_core::{
    AudioData, MAX_IMPORTED_AUDIO_BYTES, decode_audio_to_wav, is_supported_audio_path,
    probe_audio_metadata,
};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio-import")
        .join(name)
}

const ADDITIONAL: &[&str] = &[
    "tone.aiff",
    "tone.aifc",
    "tone.aac",
    "tone-aac.m4a",
    "tone-alac.m4a",
    "tone.caf",
    "tone-alac.caf",
];

#[test]
fn additional_containers_decode_source_pitch_and_preserve_original_files() {
    for name in ADDITIONAL {
        let directory = tempfile::tempdir().unwrap();
        let input = fixture(name);
        let before = std::fs::read(&input).unwrap();
        let output = directory.path().join("decoded.wav");
        let info = decode_audio_to_wav(&input, &output, MAX_IMPORTED_AUDIO_BYTES)
            .unwrap_or_else(|error| panic!("{name}: {error:#}"));
        assert_eq!(info.sample_rate, 48_000, "{name}");
        assert_eq!(info.channels, 2, "{name}");
        assert!(
            (5_760..=8_192).contains(&info.frames),
            "{name}: {} frames",
            info.frames
        );
        assert_eq!(info.decoded_bytes, info.frames * 2 * 4);
        let decoded = AudioData::load_wav(&output).unwrap();
        assert!(decoded.samples.iter().all(|sample| sample.is_finite()));
        // Lossy codec priming is allowed where gapless data is unavailable.
        // Frequency is independent of that phase/delay and the channel levels
        // distinguish an actual stereo decode from duplicated or swapped audio.
        let first = 1_024;
        let last = 5_760;
        for (channel, frequency, expected) in [(0, 440.0, 0.4), (1, 880.0, 0.25)] {
            let (mut sine, mut cosine) = (0.0, 0.0);
            for frame in first..last {
                let value = f64::from(decoded.samples[frame * 2 + channel]);
                let phase = frame as f64 * std::f64::consts::TAU * frequency / 48_000.0;
                sine += value * phase.sin();
                cosine += value * phase.cos();
            }
            let amplitude = 2.0 * sine.hypot(cosine) / (last - first) as f64;
            assert!(
                (amplitude - expected).abs() < 0.06,
                "{name} channel{channel}: {amplitude}"
            );
        }
        assert_eq!(std::fs::read(&input).unwrap(), before, "{name}");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn lossless_aiff_aifc_alac_and_caf_retain_pcm_and_content_based_recognition() {
    let reference = AudioData::load_wav(fixture("tone.wav")).unwrap();
    for name in [
        "tone.aiff",
        "tone.aifc",
        "tone-alac.m4a",
        "tone.caf",
        "tone-alac.caf",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let disguised = directory.path().join("recording.data");
        std::fs::copy(fixture(name), &disguised).unwrap();
        let output = directory.path().join("decoded.wav");
        decode_audio_to_wav(&disguised, &output, MAX_IMPORTED_AUDIO_BYTES)
            .unwrap_or_else(|error| panic!("{name}: {error:#}"));
        let decoded = AudioData::load_wav(&output).unwrap();
        assert!(decoded.samples.len() >= reference.samples.len(), "{name}");
        assert!(
            decoded
                .samples
                .iter()
                .zip(&reference.samples)
                .all(|(actual, expected)| { (actual - expected).abs() <= 2.0e-7 }),
            "{name}: lossless audio changed"
        );
        assert!(
            decoded.samples[reference.samples.len()..]
                .iter()
                .all(|sample| sample.abs() <= 2.0e-7),
            "{name}: any container padding must be silence"
        );
    }
}

#[test]
fn metadata_reports_container_codec_dimensions_depth_and_encoded_bytes() {
    for (name, depth) in [
        ("tone.wav", Some(32)),
        ("tone.flac", Some(24)),
        ("tone.mp3", None),
        ("tone.ogg", None),
        ("tone.aiff", Some(24)),
        ("tone.aifc", Some(32)),
        ("tone.aac", None),
        ("tone-aac.m4a", None),
        ("tone-alac.m4a", Some(24)),
        ("tone.caf", Some(24)),
        ("tone-alac.caf", Some(24)),
    ] {
        let path = fixture(name);
        let before = std::fs::read(&path).unwrap();
        let metadata =
            probe_audio_metadata(&path).unwrap_or_else(|error| panic!("{name}: {error:#}"));
        assert_eq!(metadata.sample_rate, Some(48_000), "{name}: {metadata:?}");
        assert_eq!(metadata.channels, Some(2), "{name}: {metadata:?}");
        assert_eq!(metadata.bits_per_sample, depth, "{name}: {metadata:?}");
        assert_eq!(metadata.file_bytes, before.len() as u64, "{name}");
        assert!(
            !metadata.format.is_empty() && !metadata.codec.is_empty(),
            "{name}"
        );
        if let Some(duration) = metadata.duration_seconds {
            assert!(
                duration.is_finite() && (0.119..=0.2).contains(&duration),
                "{name}: {duration}"
            );
        } else {
            assert!(
                name.ends_with(".aac"),
                "{name}: container should provide duration"
            );
        }
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}

#[test]
fn probing_is_header_only_and_extension_aliases_are_case_insensitive() {
    for extension in [
        "wav", "wave", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga", "aac", "m4a", "mp4",
        "caf",
    ] {
        assert!(
            is_supported_audio_path(Path::new(&format!("sample.{}", extension.to_uppercase()))),
            "{extension}"
        );
    }
    assert!(!is_supported_audio_path(Path::new("notes.txt")));
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("misleading.bin");
    let mut writer = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for _ in 0..441 {
        writer.write_sample(f32::NAN).unwrap();
    }
    writer.finalize().unwrap();
    // Browsing reads the valid header; decoding must separately reject PCM.
    let metadata = probe_audio_metadata(&source).unwrap();
    assert_eq!(metadata.sample_rate, Some(44_100));
    assert_eq!(metadata.channels, Some(1));
    assert!((metadata.duration_seconds.unwrap() - 0.01).abs() < 1e-8);
    assert!(
        decode_audio_to_wav(
            &source,
            &directory.path().join("decoded.wav"),
            MAX_IMPORTED_AUDIO_BYTES
        )
        .is_err()
    );
}

#[test]
fn new_decoders_enforce_the_same_budget_and_discard_partial_outputs() {
    for name in ADDITIONAL {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("decoded.wav");
        assert!(
            decode_audio_to_wav(&fixture(name), &output, 8_192).is_err(),
            "{name}"
        );
        assert!(!output.exists(), "{name}");
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            0,
            "{name}"
        );
    }
}

#[test]
fn metadata_rejects_missing_non_audio_and_incomplete_container_headers() {
    let directory = tempfile::tempdir().unwrap();
    assert!(probe_audio_metadata(&directory.path().join("missing.wav")).is_err());
    for (name, contents) in [
        ("empty.wav", &b""[..]),
        ("document.mp3", &b"This is not an audio file."[..]),
        ("incomplete.wav", &b"RIFF\x24\x00\x00\x00WAVE"[..]),
        ("incomplete.aiff", &b"FORM\x00\x00\x00\x24AIFF"[..]),
        ("incomplete.aifc", &b"FORM\x00\x00\x00\x24AIFC"[..]),
        ("incomplete.flac", &b"fLaC"[..]),
        ("incomplete.ogg", &b"OggS\x00\x00"[..]),
        ("incomplete.mp3", &b"ID3\x04\x00\x00"[..]),
        ("incomplete.aac", &b"\xff\xf1"[..]),
        ("incomplete.m4a", &b"\x00\x00\x00\x18ftyp"[..]),
        ("incomplete.caf", &b"caff\x00\x01\x00\x00"[..]),
    ] {
        let path = directory.path().join(name);
        std::fs::write(&path, contents).unwrap();
        assert!(probe_audio_metadata(&path).is_err(), "{name}");
        assert_eq!(std::fs::read(path).unwrap(), contents, "{name}");
    }
}

#[test]
fn caf_with_inconsistent_declared_frames_is_rejected_without_partial_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("decoded.wav");
    let error = decode_audio_to_wav(
        &fixture("tone-alac-invalid-frames.caf"),
        &output,
        MAX_IMPORTED_AUDIO_BYTES,
    )
    .unwrap_err();
    assert!(error.to_string().contains("declared duration"), "{error:#}");
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn additional_containers_reject_truncated_payloads_without_partial_output() {
    for name in ADDITIONAL {
        let complete = std::fs::read(fixture(name)).unwrap();
        for length in [complete.len() / 2, complete.len() - 1] {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join(name);
            std::fs::write(&source, &complete[..length]).unwrap();
            let output = directory.path().join("decoded.wav");
            assert!(
                decode_audio_to_wav(&source, &output, MAX_IMPORTED_AUDIO_BYTES).is_err(),
                "{name}: accepted truncated {length}-byte payload"
            );
            assert!(!output.exists(), "{name}");
            assert_eq!(
                std::fs::read_dir(directory.path()).unwrap().count(),
                1,
                "{name}"
            );
        }
    }
}
