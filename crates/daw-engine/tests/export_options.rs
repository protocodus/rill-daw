use daw_core::{AudioData, Clip, Project};
use daw_engine::{
    ExportOptions, RenderOptions, export_project_wav, export_project_wav_with_options,
};

#[test]
fn exact_project_export_ends_at_the_last_clip_and_retains_compensated_audio() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![0.25; 8_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 137.0;
    // Export always renders the full arrangement, independently of playback looping.
    project.loop_enabled = true;
    project.loop_start = 0.0;
    project.loop_end = 1.0;
    let id = project.add_audio_track("Compensated audio");
    let track = project.track_mut(id).unwrap();
    track.gain = 1.0;
    track.latency_samples = 257;
    for (start, length) in [(0.0, 0.5), (2.345, 0.456)] {
        track.clips.push(Clip::audio(
            "Source",
            start,
            length,
            source.to_string_lossy(),
        ));
    }
    let before = project.clone();

    for sample_rate in [8_000, 44_100, 48_000] {
        let exact = directory.path().join("exact.wav");
        let default = directory.path().join("default.wav");
        let quality_only = directory.path().join("quality-only.wav");
        export_project_wav_with_options(
            &project,
            &exact,
            sample_rate,
            ExportOptions {
                include_effect_tails: false,
                ..Default::default()
            },
        )
        .unwrap();
        export_project_wav(&project, &default, sample_rate).unwrap();
        export_project_wav_with_options(
            &project,
            &quality_only,
            sample_rate,
            RenderOptions::default(),
        )
        .unwrap();

        let exact = AudioData::load_wav(exact).unwrap();
        let default = AudioData::load_wav(default).unwrap();
        let quality_only = AudioData::load_wav(quality_only).unwrap();
        let expected = ((2.345 + 0.456) * (f64::from(sample_rate) * 60.0 / 137.0)).round() as usize;
        assert_eq!(exact.frames(), expected);
        assert_eq!(exact.channels, 2);
        assert_eq!(exact.sample_rate, sample_rate);
        assert_eq!(default.frames(), expected + sample_rate as usize / 5);
        assert_eq!(default.samples, quality_only.samples);
        assert_eq!(&default.samples[..exact.samples.len()], exact.samples);
        assert!(exact.samples.iter().all(|sample| sample.is_finite()));
        assert!(
            exact.samples[0].abs() > 0.1,
            "PDC must not add export preroll"
        );
        assert!(exact.samples[exact.samples.len() - 2].abs() > 0.1);
        assert!(
            default.samples[exact.samples.len()..]
                .iter()
                .all(|sample| *sample == 0.0)
        );
        assert_eq!(project, before);
    }
}

#[test]
fn exact_project_export_failure_preserves_the_existing_destination() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![1.0; 4_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    for _ in 0..2 {
        let id = project.add_audio_track("Overflow");
        let track = project.track_mut(id).unwrap();
        track.gain = f32::MAX;
        track
            .clips
            .push(Clip::audio("Tone", 0.0, 1.0, source.to_string_lossy()));
    }
    let output = directory.path().join("previous.wav");
    let previous = b"Existing export must survive a failed render";
    std::fs::write(&output, previous).unwrap();
    assert!(
        export_project_wav_with_options(
            &project,
            &output,
            8_000,
            ExportOptions {
                include_effect_tails: false,
                ..Default::default()
            },
        )
        .is_err()
    );
    assert_eq!(std::fs::read(output).unwrap(), previous);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}
