use daw_core::{
    AudioData, AudioFile, AudioSilenceSettings, Clip, ClipContent, TransientAlgorithm,
    TransientSettings, detect_audio_silence, detect_audio_silence_file, detect_transients,
    detect_transients_file, normalization_gain, normalization_gain_file,
};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    sync::atomic::AtomicBool,
};

#[test]
fn disk_analysis_matches_resident_oracle_across_stretch_reverse_offsets_and_channel_phase() {
    let directory = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    for channels in [1, 2] {
        let path = directory.path().join(format!("oracle-{channels}.wav"));
        let data = AudioData {
            sample_rate: 8_000,
            channels,
            samples: (0..96_000)
                .flat_map(|frame| {
                    let value = if frame % 8_000 < 2_000 {
                        (frame as f32 * 0.13).sin() * 0.7
                    } else {
                        0.0
                    };
                    (0..channels).map(move |channel| if channel == 0 { value } else { -value })
                })
                .collect(),
        };
        data.write_wav(&path).unwrap();
        let file = AudioFile::open(&path).unwrap();
        for rate in [0.5, 1.0, 2.0] {
            for reverse in [false, true] {
                let mut clip = Clip::audio("Window", 7.0, 16.0, path.to_string_lossy());
                clip.offset = 3.000125; // A fractional frame; crosses read-cache boundaries.
                clip.source_rate = rate;
                if let ClipContent::Audio {
                    reverse: value,
                    fade_in,
                    gain,
                    ..
                } = &mut clip.content
                {
                    *value = reverse;
                    *fade_in = 2.0;
                    *gain = 0.2;
                }
                let original = clip.clone();
                assert_eq!(
                    normalization_gain_file(&clip, &file, 120.0, -1.0, &cancel).unwrap(),
                    normalization_gain(&clip, &data, 120.0, -1.0).unwrap()
                );
                for algorithm in TransientAlgorithm::ALL {
                    let settings = TransientSettings {
                        algorithm,
                        ..Default::default()
                    };
                    assert_eq!(
                        detect_transients_file(&clip, &file, 120.0, settings, &cancel).unwrap(),
                        detect_transients(&clip, &data, 120.0, settings).unwrap(),
                        "rate={rate}, reverse={reverse}, channels={channels}, algorithm={algorithm:?}"
                    );
                }
                let range = (clip.start + 0.25, clip.end() - 0.5);
                let settings = AudioSilenceSettings::default();
                assert_eq!(
                    detect_audio_silence_file(&clip, &file, 120.0, range, settings, &cancel)
                        .unwrap(),
                    detect_audio_silence(&clip, &data, 120.0, range, settings).unwrap(),
                    "rate={rate}, reverse={reverse}, channels={channels}"
                );
                assert_eq!(
                    clip, original,
                    "Analysis must retain all source/envelope coordinates"
                );
            }
        }
    }
}

#[test]
fn far_window_on_a_large_parent_reads_only_relevant_pcm_and_keeps_whole_clip_normalization() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("long.wav");
    let frames = 17_000_000_usize; // Above the old 64 MiB whole-parent analysis ceiling.
    let bytes = (frames * 4) as u32;
    let first = frames - 32_000;
    let mut writer = File::create(&path).unwrap();
    writer.write_all(b"RIFF").unwrap();
    writer.write_all(&(bytes + 36).to_le_bytes()).unwrap();
    writer.write_all(b"WAVEfmt ").unwrap();
    writer.write_all(&16_u32.to_le_bytes()).unwrap();
    writer.write_all(&3_u16.to_le_bytes()).unwrap();
    writer.write_all(&1_u16.to_le_bytes()).unwrap();
    writer.write_all(&8_000_u32.to_le_bytes()).unwrap();
    writer.write_all(&32_000_u32.to_le_bytes()).unwrap();
    writer.write_all(&4_u16.to_le_bytes()).unwrap();
    writer.write_all(&32_u16.to_le_bytes()).unwrap();
    writer.write_all(b"data").unwrap();
    writer.write_all(&bytes.to_le_bytes()).unwrap();
    writer.set_len(u64::from(bytes) + 44).unwrap();
    for (frame, amplitude) in [(first + 1_000, 0.25_f32), (first + 20_000, 0.75)] {
        writer.seek(SeekFrom::Start(44 + frame as u64 * 4)).unwrap();
        for _ in 0..800 {
            writer.write_all(&amplitude.to_le_bytes()).unwrap();
        }
    }
    writer.sync_all().unwrap();
    drop(writer);
    assert!(AudioData::load_wav_with_limit(&path, 64 * 1024 * 1024).is_err());
    let file = AudioFile::open(&path).unwrap();
    let mut clip = Clip::audio("Far window", 10.0, 8.0, path.to_string_lossy());
    clip.offset = first as f64 / 4_000.0;
    let cancel = AtomicBool::new(false);
    let region = clip.audio_region(10.0, 12.0).unwrap();
    let onsets =
        detect_transients_file(&region, &file, 120.0, TransientSettings::default(), &cancel)
            .unwrap();
    assert_eq!(onsets.len(), 1);
    assert!((onsets[0] - 0.25).abs() < 0.011);
    let spans = detect_audio_silence_file(
        &clip,
        &file,
        120.0,
        (10.0, 12.0),
        AudioSilenceSettings::default(),
        &cancel,
    )
    .unwrap();
    assert_eq!(spans.len(), 1);
    let gain = normalization_gain_file(&clip, &file, 120.0, 0.0, &cancel).unwrap();
    assert_eq!(
        gain,
        1.0 / 0.75,
        "The louder pulse outside the selected range still sets clip normalization"
    );
    assert_eq!(file.info().frames, frames);
}

#[test]
fn cancellation_and_replaced_sources_reject_analysis_without_changing_the_clip() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("changed.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![0.5; 8_000],
    }
    .write_wav(&path)
    .unwrap();
    let file = AudioFile::open(&path).unwrap();
    let clip = Clip::audio("Source", 0.0, 2.0, path.to_string_lossy());
    let original = clip.clone();
    let cancelled = AtomicBool::new(true);
    assert!(
        normalization_gain_file(&clip, &file, 120.0, -1.0, &cancelled)
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert!(
        detect_transients_file(
            &clip,
            &file,
            120.0,
            TransientSettings::default(),
            &cancelled
        )
        .is_err()
    );
    assert!(
        detect_audio_silence_file(
            &clip,
            &file,
            120.0,
            (0.0, 2.0),
            AudioSilenceSettings::default(),
            &cancelled
        )
        .is_err()
    );
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(16)
        .unwrap();
    let cancel = AtomicBool::new(false);
    assert!(normalization_gain_file(&clip, &file, 120.0, -1.0, &cancel).is_err());
    assert!(
        detect_transients_file(&clip, &file, 120.0, TransientSettings::default(), &cancel).is_err()
    );
    assert!(
        detect_audio_silence_file(
            &clip,
            &file,
            120.0,
            (0.0, 2.0),
            AudioSilenceSettings::default(),
            &cancel
        )
        .is_err()
    );
    assert_eq!(clip, original);
}

#[test]
fn finite_extreme_samples_normalize_without_interpolation_overflow_and_bad_timing_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("extreme.wav");
    let data = AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![f32::MAX, -f32::MAX, 0.0],
    };
    data.write_wav(&path).unwrap();
    let file = AudioFile::open(&path).unwrap();
    let clip = Clip::audio("Extremes", 0.0, 3.0 / 4_000.0, path.to_string_lossy());
    let cancel = AtomicBool::new(false);
    assert_eq!(
        normalization_gain_file(&clip, &file, 120.0, -1.0, &cancel).unwrap(),
        normalization_gain(&clip, &data, 120.0, -1.0).unwrap()
    );
    assert_eq!(
        detect_audio_silence_file(
            &clip,
            &file,
            120.0,
            (0.0, clip.end()),
            AudioSilenceSettings::default(),
            &cancel
        )
        .unwrap(),
        detect_audio_silence(
            &clip,
            &data,
            120.0,
            (0.0, clip.end()),
            AudioSilenceSettings::default()
        )
        .unwrap()
    );
    for (field, value) in [
        ("start", f64::NAN),
        ("length", f64::INFINITY),
        ("length", -1.0),
        ("offset", f64::NAN),
        ("offset", -1.0),
        ("rate", f64::MAX),
        ("rate", 0.0),
        ("rate", f64::NAN),
    ] {
        let mut invalid = clip.clone();
        match field {
            "start" => invalid.start = value,
            "length" => invalid.length = value,
            "offset" => invalid.offset = value,
            _ => invalid.source_rate = value,
        }
        assert!(normalization_gain(&invalid, &data, 120.0, -1.0).is_err());
        assert!(detect_transients(&invalid, &data, 120.0, TransientSettings::default()).is_err());
        assert!(
            detect_audio_silence(
                &invalid,
                &data,
                120.0,
                (0.0, clip.end()),
                AudioSilenceSettings::default(),
            )
            .is_err()
        );
        assert!(normalization_gain_file(&invalid, &file, 120.0, -1.0, &cancel).is_err());
        assert!(
            detect_transients_file(
                &invalid,
                &file,
                120.0,
                TransientSettings::default(),
                &cancel
            )
            .is_err()
        );
        assert!(
            detect_audio_silence_file(
                &invalid,
                &file,
                120.0,
                (0.0, clip.end()),
                AudioSilenceSettings::default(),
                &cancel
            )
            .is_err()
        );
    }
}

#[test]
fn fractional_extreme_samples_are_finite_for_resident_and_cached_readers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("alternating.wav");
    let data = AudioData {
        sample_rate: 8_192,
        channels: 1,
        samples: (0..8_192)
            .map(|frame| if frame % 2 == 0 { f32::MAX } else { -f32::MAX })
            .collect(),
    };
    data.write_wav(&path).unwrap();
    let file = AudioFile::open(&path).unwrap();
    let cancel = AtomicBool::new(false);
    let mut clip = Clip::audio("Alternating", 0.0, 0.5, path.to_string_lossy());
    // A power-of-two sample rate makes these source midpoints exact. Opposite
    // extremes cancel to zero; their f32 difference must not overflow to NaN.
    clip.offset = 0.5 / 8_192.0;
    assert_eq!(daw_core::audio_sample(&data, 0.0, 0), f32::MAX);
    assert_eq!(daw_core::audio_sample(&data, 0.5, 0), 0.0);
    let settings = AudioSilenceSettings::default();
    let resident = detect_audio_silence(&clip, &data, 60.0, (0.0, 0.5), settings).unwrap();
    let streamed =
        detect_audio_silence_file(&clip, &file, 60.0, (0.0, 0.5), settings, &cancel).unwrap();
    assert!(resident.is_empty());
    assert_eq!(streamed, resident);
}

#[test]
fn malformed_source_window_returns_an_error_instead_of_panicking() {
    let data = AudioData {
        sample_rate: 1_000,
        channels: 1,
        samples: vec![0.5; 1_000],
    };
    let mut clip = Clip::audio("Invalid window", 0.0, 0.25, "source.wav");
    clip.offset = 0.5;
    clip.length = -0.25;
    // Previously this attempted to take the inverted slice [500..250].
    assert!(normalization_gain(&clip, &data, 60.0, -1.0).is_err());
}
