use daw_core::{AudioData, Clip, ClipContent, FadeCurve, Project};
use daw_engine::render;

#[test]
fn boundary_smoothing_changes_only_cut_edges_in_forward_reverse_and_stretched_audio() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.wav");
    AudioData {
        sample_rate: 8000,
        channels: 2,
        samples: (0..32000)
            .flat_map(|i| {
                let value = 0.3 + (i as f32 * 0.02).sin() * 0.1;
                [value, -value * 0.5]
            })
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    let source_bytes = std::fs::read(&path).unwrap();
    for reverse in [false, true] {
        for rate in [0.5, 1.0, 2.0] {
            let mut project = Project::new();
            project.tempo = 120.0;
            let track = project.add_audio_track("Audio");
            project.track_mut(track).unwrap().gain = 1.0;
            let mut clip = Clip::audio("Cut", 0.0, 2.0, path.to_string_lossy());
            clip.source_rate = rate;
            clip.offset = 1.0;
            let id = clip.id;
            if let ClipContent::Audio { reverse: value, .. } = &mut clip.content {
                *value = reverse;
            }
            project.track_mut(track).unwrap().clips.push(clip);
            let baseline = render(&project, 8000, 2.0).unwrap();
            project
                .smooth_audio_boundaries(&[(track, id)], 3.0, 5.0)
                .unwrap();
            let smoothed = render(&project, 8000, 2.0).unwrap();
            assert_eq!(
                &baseline[48..(8000 - 40) * 2],
                &smoothed[48..(8000 - 40) * 2],
                "rate={rate}, reverse={reverse}"
            );
            assert_eq!(&smoothed[..2], &[0.0, 0.0]);
            assert!(smoothed[7999 * 2].abs() < baseline[7999 * 2].abs() * 0.1);
            // The PCM envelope matches the shared editor/preview evaluation.
            let clip = project.clip(track, id).unwrap();
            for frame in [1, 12, 24, 7960, 7980, 7999] {
                let expected =
                    baseline[frame * 2] * clip.audio_envelope_gain(frame as f64 / 4000.0);
                assert!((smoothed[frame * 2] - expected).abs() < 2e-6);
            }
            let right = project.split_clip(track, id, 0.75).unwrap();
            let split = render(&project, 8000, 2.0).unwrap();
            assert!(
                smoothed
                    .iter()
                    .zip(&split)
                    .all(|(a, b)| (*a - *b).abs() < 2e-6)
            );
            project.trim_clip(track, right, 1.0, 1.0).unwrap();
            let trimmed = render(&project, 8000, 2.0).unwrap();
            assert_eq!(&trimmed[8000..], &split[8000..]);
        }
    }
    assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
}

#[test]
fn existing_long_musical_fades_export_identically_after_smoothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.wav");
    AudioData {
        sample_rate: 8000,
        channels: 1,
        samples: vec![0.4; 8000],
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    let clip = Clip::audio("Musical fade", 0.0, 2.0, path.to_string_lossy());
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    project
        .set_audio_fades(
            track,
            id,
            1.0,
            0.5,
            FadeCurve::SCurve,
            FadeCurve::EqualPower,
        )
        .unwrap();
    let original = render(&project, 8000, 2.0).unwrap();
    assert_eq!(
        project
            .smooth_audio_boundaries(&[(track, id)], 3.0, 5.0)
            .unwrap(),
        0
    );
    assert_eq!(render(&project, 8000, 2.0).unwrap(), original);
}
