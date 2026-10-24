use daw_core::{AudioData, Clip, FadeCurve, Project};
use daw_engine::render;

#[test]
fn reverse_curves_and_nondestructive_split_trim_preserve_stereo_samples() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stereo.wav");
    let source = AudioData {
        sample_rate: 8000,
        channels: 2,
        samples: (0..16000)
            .flat_map(|i| [i as f32 / 20000.0, -(i as f32 / 30000.0)])
            .collect(),
    };
    source.write_wav(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    for reverse in [false, true] {
        for curve in FadeCurve::ALL {
            let mut project = Project::new();
            project.tempo = 120.0;
            let track = project.add_audio_track("Audio");
            project.track_mut(track).unwrap().gain = 1.0;
            let mut clip = Clip::audio("Region", 0.0, 2.0, path.to_string_lossy());
            clip.offset = 1.0;
            let id = clip.id;
            project.track_mut(track).unwrap().clips.push(clip);
            project
                .set_audio_fades(track, id, 0.8, 0.9, curve, curve)
                .unwrap();
            if reverse {
                project.reverse_audio_clip(track, id).unwrap();
            }
            let original = render(&project, 8000, 2.0).unwrap();
            for frame in [100, 1000, 4500, 7900] {
                let clip = project.clip(track, id).unwrap();
                let source_frame = if reverse { 11999 - frame } else { 4000 + frame };
                let envelope = clip.audio_envelope_gain(frame as f64 / 4000.0);
                for channel in 0..2 {
                    assert!(
                        (original[frame * 2 + channel]
                            - source.samples[source_frame * 2 + channel] * envelope)
                            .abs()
                            < 2e-6
                    );
                }
            }
            let mut split = project.clone();
            split
                .split_audio_transients(track, id, &[0.3, 0.75, 1.4])
                .unwrap();
            split.validate().unwrap();
            let split_audio = render(&split, 8000, 2.0).unwrap();
            assert!(
                original
                    .iter()
                    .zip(split_audio)
                    .all(|(a, b)| (*a - b).abs() < 2e-6),
                "Split changed {curve:?}, reverse={reverse}"
            );
            project.trim_clip(track, id, 0.3, 1.1).unwrap();
            project.validate().unwrap();
            let trimmed = render(&project, 8000, 2.0).unwrap();
            assert!(trimmed[..2400].iter().all(|s| *s == 0.0));
            assert!(
                trimmed[2400..11200]
                    .iter()
                    .zip(&original[2400..11200])
                    .all(|(a, b)| (*a - *b).abs() < 2e-6)
            );
            assert!(trimmed[11200..].iter().all(|s| *s == 0.0));
        }
    }
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
