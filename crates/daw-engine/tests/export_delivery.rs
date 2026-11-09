//! Delivery encodings and exact ranges use the production streaming renderer.
use daw_core::{AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Project};
use daw_engine::{
    ExportOptions, ExportRange, WavEncoding, export_project_wav, export_project_wav_with_report,
};
use std::path::Path;

fn fixture(samples: Vec<f32>) -> (tempfile::TempDir, Project) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    let frames = samples.len() / 2;
    AudioData {
        sample_rate: 8_000,
        channels: 2,
        samples,
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let id = project.add_audio_track("Source");
    let track = project.track_mut(id).unwrap();
    track.gain = 1.0;
    track.clips.push(Clip::audio(
        "Source",
        0.0,
        frames as f64 / 4_000.0,
        source.to_string_lossy(),
    ));
    (directory, project)
}

fn integers(path: &Path) -> Vec<i32> {
    hound::WavReader::open(path)
        .unwrap()
        .samples::<i32>()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn delivery_encodings_preserve_float_headroom_and_report_pcm_overloads() {
    let samples: Vec<_> = (0..2_000)
        .flat_map(|_| [1.5, -1.5, 1.0, -1.0, 0.5, -0.5, 0.0, 0.0])
        .collect();
    let (directory, project) = fixture(samples.clone());
    let before = project.clone();
    for encoding in [WavEncoding::Pcm16, WavEncoding::Pcm24, WavEncoding::Float32] {
        let path = directory.path().join(format!("{encoding:?}.wav"));
        let report = export_project_wav_with_report(
            &project,
            &path,
            8_000,
            ExportOptions {
                include_effect_tails: false,
                encoding,
                dither: false,
                ..Default::default()
            },
        )
        .unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().bits_per_sample, encoding.bits_per_sample());
        assert_eq!(reader.spec().sample_rate, 8_000);
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.duration(), 8_000);
        assert_eq!(report.frames, 8_000);
        assert_eq!(report.sample_rate, 8_000);
        assert_eq!(report.encoding, encoding);
        assert!(!report.dither);
        assert_eq!(report.sample_peak, 1.5);
        if encoding == WavEncoding::Float32 {
            assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
            assert_eq!(
                reader
                    .into_samples::<f32>()
                    .map(Result::unwrap)
                    .collect::<Vec<_>>(),
                samples
            );
            assert_eq!(report.clipped_samples, 0);
        } else {
            assert_eq!(reader.spec().sample_format, hound::SampleFormat::Int);
            let scale = 1_i32 << (encoding.bits_per_sample() - 1);
            let actual = integers(&path);
            assert_eq!(
                &actual[..8],
                &[
                    scale - 1,
                    -scale,
                    scale - 1,
                    -scale,
                    scale / 2,
                    -scale / 2,
                    0,
                    0
                ]
            );
            assert_eq!(
                report.clipped_samples, 4_000,
                "Nominal +/-1 endpoints are quantized normally, not reported as overloads"
            );
            assert!(actual.iter().all(|sample| (-scale..scale).contains(sample)));
        }
        let payload = report.frames * 2 * u64::from(encoding.bits_per_sample() / 8);
        let header = std::fs::metadata(&path).unwrap().len() - payload;
        assert!(
            (32..=128).contains(&header),
            "The payload uses the selected sample width"
        );
        assert_eq!(project, before);
    }
}

#[test]
fn default_float_export_is_byte_identical_to_the_existing_api() {
    let (directory, project) = fixture(vec![1.25; 1_600]);
    let old = directory.path().join("old.wav");
    let new = directory.path().join("new.wav");
    export_project_wav(&project, &old, 8_000).unwrap();
    let report =
        export_project_wav_with_report(&project, &new, 8_000, ExportOptions::default()).unwrap();
    assert_eq!(std::fs::read(old).unwrap(), std::fs::read(new).unwrap());
    assert_eq!(report.encoding, WavEncoding::Float32);
    assert!(!report.dither);
    assert_eq!(report.clipped_samples, 0);
}

#[test]
fn integer_dither_has_bounded_unbiased_decorrelated_error_and_can_be_disabled() {
    for encoding in [WavEncoding::Pcm16, WavEncoding::Pcm24] {
        let scale = f64::from(1_u32 << (encoding.bits_per_sample() - 1));
        let levels = [0.25 / scale, -0.375 / scale];
        let samples: Vec<_> = (0..32_000)
            .flat_map(|_| levels.map(|sample| sample as f32))
            .collect();
        let (directory, project) = fixture(samples);
        let plain = directory.path().join("plain.wav");
        let dithered = directory.path().join("dithered.wav");
        let options = ExportOptions {
            encoding,
            include_effect_tails: false,
            dither: false,
            ..Default::default()
        };
        export_project_wav_with_report(&project, &plain, 8_000, options).unwrap();
        assert!(integers(&plain).iter().all(|sample| *sample == 0));
        let report = export_project_wav_with_report(
            &project,
            &dithered,
            8_000,
            ExportOptions {
                dither: true,
                ..options
            },
        )
        .unwrap();
        assert!(report.dither);
        assert_eq!(report.clipped_samples, 0);
        let encoded = integers(&dithered);
        let mut means = [0.0; 2];
        let mut powers = [0.0; 2];
        let mut cross = 0.0;
        for frame in encoded.as_chunks::<2>().0 {
            let errors = [
                f64::from(frame[0]) - levels[0] * scale,
                f64::from(frame[1]) - levels[1] * scale,
            ];
            for channel in 0..2 {
                assert!(errors[channel].abs() <= 1.5);
                means[channel] += errors[channel];
                powers[channel] += errors[channel] * errors[channel];
            }
            cross += errors[0] * errors[1];
        }
        for channel in 0..2 {
            assert!(
                (means[channel] / 32_000.0).abs() < 0.015,
                "Dither must not introduce a DC offset"
            );
            let rms = (powers[channel] / 32_000.0).sqrt();
            assert!(
                (0.40..0.60).contains(&rms),
                "TPDF quantization error should remain around half an LSB, got {rms}"
            );
        }
        assert!(
            (cross / (powers[0] * powers[1]).sqrt()).abs() < 0.03,
            "Left/right quantization errors must not be correlated"
        );
    }
}

#[test]
fn dithered_silence_remains_quiet_and_does_not_report_clipping() {
    let (directory, project) = fixture(vec![0.0; 32_000]);
    let path = directory.path().join("silence.wav");
    let report = export_project_wav_with_report(
        &project,
        &path,
        8_000,
        ExportOptions {
            encoding: WavEncoding::Pcm16,
            dither: true,
            include_effect_tails: false,
            ..Default::default()
        },
    )
    .unwrap();
    let samples = integers(&path);
    assert!(samples.iter().all(|sample| (-1..=1).contains(sample)));
    assert!(samples.iter().any(|sample| *sample != 0));
    assert_eq!(report.sample_peak, 0.0);
    assert_eq!(report.clipped_samples, 0);
}

#[test]
fn exact_ranges_match_full_exports_including_pdc_automation_and_dither() {
    let samples: Vec<_> = (0..24_000)
        .flat_map(|frame| {
            let phase = frame as f64 * std::f64::consts::TAU / 8_000.0;
            [
                (phase * 137.0).sin() as f32 * 0.4,
                (phase * 233.0).cos() as f32 * 0.3,
            ]
        })
        .collect();
    let (directory, mut project) = fixture(samples);
    project.tempo = 137.0;
    project.loop_enabled = true;
    project.loop_start = 2.0;
    project.loop_end = 3.0;
    let source = project.tracks[0].id;
    project.track_mut(source).unwrap().clips[0].length = 8.0;
    project.track_mut(source).unwrap().latency_samples = 257;
    let bus = project.add_bus("Mix bus");
    project.track_mut(bus).unwrap().gain = 1.0;
    project.track_mut(bus).unwrap().latency_samples = 113;
    project.set_output(source, Some(bus)).unwrap();
    let lane = project
        .add_automation_track(
            "Level",
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let points = &mut project.automation_lane_mut(lane).unwrap().points;
    *points = vec![
        AutomationPoint::new(0.0, 0.1),
        AutomationPoint::new(4.0, 0.8),
        AutomationPoint::new(8.0, 0.2),
    ];
    let first = points[0].id;
    project
        .set_automation_point_curve(lane, first, -0.6)
        .unwrap();
    let range = ExportRange {
        start_beat: 1.1234,
        end_beat: 4.7891,
    };
    let before = project.clone();
    for sample_rate in [8_000, 44_100] {
        let start = (range.start_beat * sample_rate as f64 * 60.0 / project.tempo).round() as usize;
        let end = (range.end_beat * sample_rate as f64 * 60.0 / project.tempo).round() as usize;
        for encoding in [WavEncoding::Float32, WavEncoding::Pcm16, WavEncoding::Pcm24] {
            let whole = directory.path().join("whole.wav");
            let selected = directory.path().join("selected.wav");
            let options = ExportOptions {
                encoding,
                dither: true,
                include_effect_tails: false,
                ..Default::default()
            };
            export_project_wav_with_report(&project, &whole, sample_rate, options).unwrap();
            let report = export_project_wav_with_report(
                &project,
                &selected,
                sample_rate,
                ExportOptions {
                    range: Some(range),
                    include_effect_tails: true,
                    ..options
                },
            )
            .unwrap();
            assert_eq!(
                report.frames,
                (end - start) as u64,
                "Explicit range never appends tails"
            );
            assert_eq!(
                hound::WavReader::open(&selected).unwrap().duration(),
                (end - start) as u32
            );
            if encoding.is_pcm() {
                assert_eq!(integers(&selected), integers(&whole)[start * 2..end * 2]);
            } else {
                let all = AudioData::load_wav(&whole).unwrap();
                let section = AudioData::load_wav(&selected).unwrap();
                assert_eq!(section.samples, all.samples[start * 2..end * 2]);
            }
            assert_eq!(project, before);
        }
    }
}

#[test]
fn delivery_rate_is_explicit_and_keeps_exact_duration() {
    let (directory, project) = fixture(vec![0.125; 4_000]);
    let before = project.clone();
    for sample_rate in [44_100, 48_000, 96_000] {
        let path = directory.path().join(format!("{sample_rate}.wav"));
        let report = export_project_wav_with_report(
            &project,
            &path,
            sample_rate,
            ExportOptions {
                include_effect_tails: false,
                encoding: WavEncoding::Pcm24,
                ..Default::default()
            },
        )
        .unwrap();
        let reader = hound::WavReader::open(path).unwrap();
        assert_eq!(reader.spec().sample_rate, sample_rate);
        assert_eq!(reader.duration(), sample_rate / 4);
        assert_eq!(report.frames, u64::from(sample_rate / 4));
        assert_eq!(project, before);
    }
}

#[test]
fn invalid_ranges_and_oversized_delivery_leave_existing_files_untouched() {
    let (directory, project) = fixture(vec![0.25; 4_000]);
    let path = directory.path().join("existing.wav");
    let previous = b"A previous successful export";
    std::fs::write(&path, previous).unwrap();
    for range in [
        ExportRange {
            start_beat: -1.0,
            end_beat: 1.0,
        },
        ExportRange {
            start_beat: 1.0,
            end_beat: 1.0,
        },
        ExportRange {
            start_beat: 2.0,
            end_beat: 1.0,
        },
        ExportRange {
            start_beat: f64::NAN,
            end_beat: 1.0,
        },
        ExportRange {
            start_beat: 0.0,
            end_beat: f64::INFINITY,
        },
        ExportRange {
            start_beat: 0.0,
            end_beat: 0.000_000_01,
        },
        ExportRange {
            start_beat: 0.0,
            end_beat: 28_801.0,
        },
    ] {
        assert!(
            export_project_wav_with_report(
                &project,
                &path,
                8_000,
                ExportOptions {
                    range: Some(range),
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), previous);
    }
    for encoding in [WavEncoding::Pcm16, WavEncoding::Pcm24, WavEncoding::Float32] {
        assert!(
            export_project_wav_with_report(
                &project,
                &path,
                384_000,
                ExportOptions {
                    range: Some(ExportRange {
                        start_beat: 0.0,
                        end_beat: 28_800.0
                    }),
                    encoding,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), previous);
    }
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    let source = directory.path().join("source.wav");
    let original = std::fs::read(&source).unwrap();
    assert!(
        export_project_wav_with_report(&project, &source, 8_000, ExportOptions::default()).is_err()
    );
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[test]
fn nonfinite_mix_preserves_previous_exports_in_every_encoding() {
    let (directory, mut project) = fixture(vec![1.0; 8_000]);
    project.tracks[0].gain = f32::MAX;
    let second = project.add_audio_track("Overflowing second source");
    let track = project.track_mut(second).unwrap();
    track.gain = f32::MAX;
    track.clips.push(Clip::audio(
        "Source",
        0.0,
        1.0,
        directory.path().join("source.wav").to_string_lossy(),
    ));
    project.validate().unwrap();
    let path = directory.path().join("previous.wav");
    let previous = b"Do not replace this export after a failed render";
    std::fs::write(&path, previous).unwrap();
    for encoding in [WavEncoding::Float32, WavEncoding::Pcm24, WavEncoding::Pcm16] {
        assert!(
            export_project_wav_with_report(
                &project,
                &path,
                8_000,
                ExportOptions {
                    encoding,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}
