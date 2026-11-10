//! FLAC, Ogg Vorbis and MP3 deliveries are decoded with the importer's strict
//! decoder and compared with the WAV render of the same song.
use daw_core::{AudioData, Clip, MAX_IMPORTED_AUDIO_BYTES, Project, decode_audio_to_wav};
use daw_engine::{
    ExportFormat, ExportOptions, ExportRange, WavEncoding, export_project_cancellable,
    export_project_wav_with_report,
};
use std::{path::Path, sync::atomic::AtomicBool};

const RATE: u32 = 44_100;

/// Two seconds (four beats at 120 BPM) of a stereo chord with a little noise.
fn fixture(amplitude: f32) -> (tempfile::TempDir, Project) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    let mut seed = 7_u32;
    let samples: Vec<f32> = (0..RATE as usize * 2)
        .flat_map(|i| {
            let t = i as f32 / RATE as f32;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = ((seed >> 8) as f32 / 8_388_608.0 - 1.0) * 0.01;
            [
                amplitude * (0.5 * (t * 440.0 * 6.283).sin() + 0.2 * (t * 2_500.0 * 6.283).sin())
                    + noise,
                amplitude * (0.4 * (t * 660.0 * 6.283).sin()) + noise,
            ]
        })
        .collect();
    AudioData {
        sample_rate: RATE,
        channels: 2,
        samples,
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let id = project.add_audio_track("Source");
    let track = project.track_mut(id).unwrap();
    track.gain = 1.0;
    track
        .clips
        .push(Clip::audio("Source", 0.0, 4.0, source.to_string_lossy()));
    (directory, project)
}

fn options(format: ExportFormat, encoding: WavEncoding) -> ExportOptions {
    ExportOptions {
        include_effect_tails: false,
        format,
        encoding,
        ..Default::default()
    }
}

fn decode(directory: &Path, path: &Path) -> AudioData {
    let wav = directory.join(format!(
        "decoded-{}.wav",
        path.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_file(&wav);
    decode_audio_to_wav(path, &wav, MAX_IMPORTED_AUDIO_BYTES).unwrap();
    AudioData::load_wav(&wav).unwrap()
}

fn integer_wav(path: &Path, bits: u16) -> Vec<f32> {
    let scale = (1_i64 << (bits - 1)) as f32;
    hound::WavReader::open(path)
        .unwrap()
        .samples::<i32>()
        .map(|sample| sample.unwrap() as f32 / scale)
        .collect()
}

/// Noise-to-signal ratio in dB after aligning the decoded copy by up to
/// `search` frames (lossy codecs may carry a decoder delay).
fn noise_to_signal_db(reference: &[f32], decoded: &[f32], search: usize) -> (f64, usize) {
    let mut best = (f64::MAX, 0);
    for delay in 0..=search {
        let (mut error, mut signal) = (0.0_f64, 0.0_f64);
        for index in 4_000..reference.len().saturating_sub(4_000 + search * 2) {
            let (a, b) = (
                f64::from(reference[index]),
                f64::from(decoded[index + delay * 2]),
            );
            error += (a - b).powi(2);
            signal += a * a;
        }
        if error / signal < best.0 {
            best = (error / signal, delay);
        }
    }
    (10.0 * best.0.log10(), best.1)
}

#[test]
fn flac_is_bit_exact_with_the_wav_render_including_dither_and_overload_reports() {
    for amplitude in [0.8, 1.6] {
        let (directory, project) = fixture(amplitude);
        for (encoding, bits) in [(WavEncoding::Pcm16, 16), (WavEncoding::Pcm24, 24)] {
            for dither in [false, true] {
                let wav = directory.path().join("reference.wav");
                let flac = directory.path().join("export.flac");
                let wav_options = ExportOptions {
                    dither,
                    ..options(ExportFormat::Wav, encoding)
                };
                let wav_report =
                    export_project_wav_with_report(&project, &wav, RATE, wav_options).unwrap();
                let report = export_project_wav_with_report(
                    &project,
                    &flac,
                    RATE,
                    ExportOptions {
                        format: ExportFormat::Flac,
                        ..wav_options
                    },
                )
                .unwrap();
                assert_eq!(report.format, ExportFormat::Flac);
                assert_eq!(report.frames, u64::from(RATE) * 2);
                assert_eq!(report.clipped_samples, wav_report.clipped_samples);
                assert_eq!(report.sample_peak, wav_report.sample_peak);
                assert!(report.dither == dither);
                assert_eq!(report.clipped_samples > 0, amplitude > 1.0);
                // The importer verifies the FLAC MD5 and the declared length.
                let decoded = decode(directory.path(), &flac);
                assert_eq!(decoded.sample_rate, RATE);
                assert_eq!(decoded.channels, 2);
                assert_eq!(decoded.samples, integer_wav(&wav, bits));
            }
        }
    }
}

#[test]
fn vorbis_decodes_to_the_exact_length_and_follows_the_mix() {
    let (directory, project) = fixture(0.8);
    let wav = directory.path().join("reference.wav");
    export_project_wav_with_report(
        &project,
        &wav,
        RATE,
        options(ExportFormat::Wav, WavEncoding::Float32),
    )
    .unwrap();
    let reference = AudioData::load_wav(&wav).unwrap();
    let mut sizes = Vec::new();
    for quality in [2, 8] {
        let path = directory.path().join(format!("q{quality}.ogg"));
        let report = export_project_wav_with_report(
            &project,
            &path,
            RATE,
            options(ExportFormat::Vorbis { quality }, WavEncoding::Float32),
        )
        .unwrap();
        assert!(!report.dither);
        let decoded = decode(directory.path(), &path);
        assert_eq!(decoded.sample_rate, RATE);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.frames(), reference.frames());
        let (ratio, delay) = noise_to_signal_db(&reference.samples, &decoded.samples, 0);
        assert_eq!(delay, 0);
        assert!(ratio < -18.0, "quality {quality}: {ratio:.1} dB");
        sizes.push(std::fs::metadata(&path).unwrap().len());
    }
    assert!(sizes[0] < sizes[1], "higher quality should be larger");
    // Reproducible: no random stream serial.
    let again = directory.path().join("again.ogg");
    export_project_wav_with_report(
        &project,
        &again,
        RATE,
        options(ExportFormat::Vorbis { quality: 8 }, WavEncoding::Float32),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(again).unwrap(),
        std::fs::read(directory.path().join("q8.ogg")).unwrap()
    );
}

#[test]
fn mp3_decodes_with_the_right_format_length_and_fidelity() {
    let (directory, project) = fixture(0.8);
    let wav = directory.path().join("reference.wav");
    export_project_wav_with_report(
        &project,
        &wav,
        RATE,
        options(ExportFormat::Wav, WavEncoding::Float32),
    )
    .unwrap();
    let reference = AudioData::load_wav(&wav).unwrap();
    for kbps in ExportFormat::MP3_BITRATES {
        let path = directory.path().join(format!("{kbps}.mp3"));
        export_project_wav_with_report(
            &project,
            &path,
            RATE,
            options(ExportFormat::Mp3 { kbps }, WavEncoding::Float32),
        )
        .unwrap();
        let bytes = std::fs::metadata(&path).unwrap().len();
        let expected = u64::from(kbps) * 1000 / 8 * 2;
        assert!(
            bytes > expected * 9 / 10 && bytes < expected * 12 / 10,
            "{kbps} kbit/s file is {bytes} bytes"
        );
        let decoded = decode(directory.path(), &path);
        assert_eq!(decoded.sample_rate, RATE);
        assert_eq!(decoded.channels, 2);
        assert!(
            decoded.frames().abs_diff(reference.frames()) <= 2_304,
            "{} vs {} frames",
            decoded.frames(),
            reference.frames()
        );
        let (ratio, _) = noise_to_signal_db(&reference.samples, &decoded.samples, 2_400);
        assert!(ratio < -20.0, "{kbps} kbit/s: {ratio:.1} dB");
    }
}

#[test]
fn every_format_exports_the_exact_range_and_clamps_overloads() {
    let (directory, project) = fixture(1.6);
    // Beats 1 to 3 at 120 BPM and 44.1 kHz is exactly one second.
    let range = Some(ExportRange {
        start_beat: 1.0,
        end_beat: 3.0,
    });
    for (format, encoding) in [
        (ExportFormat::Wav, WavEncoding::Pcm16),
        (ExportFormat::Flac, WavEncoding::Pcm24),
        (ExportFormat::Vorbis { quality: 5 }, WavEncoding::Float32),
        (ExportFormat::Mp3 { kbps: 192 }, WavEncoding::Float32),
    ] {
        let path = directory
            .path()
            .join(format!("range.{}", format.extension()));
        let report = export_project_wav_with_report(
            &project,
            &path,
            RATE,
            ExportOptions {
                range,
                ..options(format, encoding)
            },
        )
        .unwrap();
        assert_eq!(report.frames, u64::from(RATE), "{format:?}");
        assert!(
            report.clipped_samples > 0,
            "{format:?} reports its overloads"
        );
        assert!(report.sample_peak > 1.0);
        let decoded = decode(directory.path(), &path);
        if !matches!(format, ExportFormat::Mp3 { .. }) {
            assert_eq!(decoded.frames(), u64::from(RATE) as usize, "{format:?}");
        }
        // Lossless and PCM outputs never exceed full scale.
        if !format.is_lossy() {
            assert!(decoded.samples.iter().all(|sample| sample.abs() <= 1.0));
        }
    }
}

#[test]
fn unsupported_choices_fail_before_writing_anything() {
    let (directory, project) = fixture(0.5);
    for (format, encoding, rate) in [
        (ExportFormat::Flac, WavEncoding::Float32, RATE),
        (
            ExportFormat::Mp3 { kbps: 192 },
            WavEncoding::Float32,
            96_000,
        ),
        (ExportFormat::Mp3 { kbps: 100 }, WavEncoding::Float32, RATE),
        (
            ExportFormat::Vorbis { quality: 11 },
            WavEncoding::Float32,
            RATE,
        ),
    ] {
        let path = directory.path().join("never");
        assert!(
            export_project_wav_with_report(&project, &path, rate, options(format, encoding))
                .is_err(),
            "{format:?}"
        );
    }
    assert!(!directory.path().join("never").exists());
    // Vorbis has no sample-rate limit.
    export_project_wav_with_report(
        &project,
        directory.path().join("high.ogg"),
        96_000,
        options(ExportFormat::VORBIS_DEFAULT, WavEncoding::Float32),
    )
    .unwrap();
}

#[test]
fn cancelled_compressed_exports_keep_the_previous_file_and_leave_nothing_behind() {
    let (directory, project) = fixture(0.5);
    let cancel = AtomicBool::new(true);
    for format in [
        ExportFormat::Flac,
        ExportFormat::VORBIS_DEFAULT,
        ExportFormat::MP3_DEFAULT,
    ] {
        let path = directory
            .path()
            .join(format!("previous.{}", format.extension()));
        std::fs::write(&path, b"previous").unwrap();
        let before = std::fs::read_dir(directory.path()).unwrap().count();
        let error = export_project_cancellable(
            &project,
            &path,
            RATE,
            options(format, WavEncoding::Pcm16),
            &cancel,
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancel"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), before);
    }
}
