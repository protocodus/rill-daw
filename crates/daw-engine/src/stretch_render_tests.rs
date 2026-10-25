use super::*;
use daw_core::{Clip, FadeCurve, Note};

fn fixture() -> (tempfile::TempDir, Project, u64, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.wav");
    AudioData {
        sample_rate: 48000,
        channels: 1,
        samples: (0..96000)
            .map(|i| (std::f64::consts::TAU * 440.0 * i as f64 / 48000.0).sin() as f32 * 0.25)
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Source");
    let mut clip = Clip::audio("Window", 0.0, 2.0, path.to_string_lossy());
    clip.offset = 1.0;
    let id = clip.id;
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(clip);
    (dir, project, track, id)
}

#[test]
fn stretched_audio_split_join_reverse_and_fades_preserve_rendered_samples() {
    for reverse in [false, true] {
        let (dir, mut project, track, id) = fixture();
        project
            .set_audio_fades(
                track,
                id,
                0.5,
                0.5,
                FadeCurve::SCurve,
                FadeCurve::EqualPower,
            )
            .unwrap();
        if reverse {
            project.reverse_audio_clip(track, id).unwrap();
        }
        project.stretch_clip(track, id, 4.0).unwrap();
        let baseline = render(&project, 48000, 4.0).unwrap();
        let right = project.split_clip(track, id, 1.125).unwrap();
        let split = render(&project, 48000, 4.0).unwrap();
        let error = baseline
            .iter()
            .zip(&split)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(error < 1e-5, "split reverse={reverse}: {error}");
        let joined_path = dir.path().join("joined.wav");
        crate::export_audio_join(
            &project,
            track,
            &[id, right],
            &joined_path,
            48000,
            RenderOptions::default(),
        )
        .unwrap();
        let joined = project
            .replace_audio_clips_with_joined(track, &[id, right], joined_path.to_string_lossy())
            .unwrap();
        assert_eq!(project.clip(track, joined).unwrap().source_rate, 1.0);
        let output = render(&project, 48000, 4.0).unwrap();
        let error = baseline
            .iter()
            .zip(&output)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(error < 1e-5, "join reverse={reverse}: {error}");
    }
}

#[test]
fn live_stretch_reuses_original_audio_and_shares_prepared_data_after_split() {
    let (dir, mut project, track, id) = fixture();
    let mut renderer = Renderer::new(&project, 48000, true).unwrap();
    let original = Arc::clone(&renderer.tracks[0].as_ref().unwrap().audio[0].data);
    let mut preparer = renderer.live_edit_preparer();
    std::fs::remove_file(dir.path().join("source.wav")).unwrap();
    project.stretch_clip(track, id, 4.0).unwrap();
    let mut plan = preparer.prepare(&project).unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| renderer
            .apply_live_edit(&mut plan)
            .unwrap()),
        0
    );
    let data = Arc::clone(&renderer.tracks[0].as_ref().unwrap().audio[0].data);
    assert!(!Arc::ptr_eq(&data, &original));
    assert!(Arc::ptr_eq(
        &renderer.tracks[0].as_ref().unwrap().audio[0].original,
        &original
    ));
    project.split_clip(track, id, 2.0).unwrap();
    let mut plan = preparer.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut plan).unwrap();
    let clips = &renderer.tracks[0].as_ref().unwrap().audio;
    assert!(Arc::ptr_eq(&clips[0].data, &clips[1].data));
    assert!(Arc::ptr_eq(&clips[0].data, &data));
    project.stretch_clip(track, id, 1.0).unwrap();
    let mut plan = preparer.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut plan).unwrap();
    assert!(Arc::ptr_eq(
        &renderer.tracks[0].as_ref().unwrap().audio[0].original,
        &original
    ));
}

#[test]
fn midi_stretch_compiles_source_time_into_arrangement_time() {
    let mut project = Project::new();
    let track = project.add_midi_track("Notes");
    let mut clip = Clip::midi("Offset", 2.0, 2.0, vec![Note::new(60, 1.5, 1.0, 100)]);
    clip.offset = 1.0;
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    project.stretch_clip(track, id, 4.0).unwrap();
    let renderer = Renderer::new(&project, 8000, false).unwrap();
    let note = renderer.tracks[0].as_ref().unwrap().notes[0];
    assert_eq!(note.start, 12000);
    assert_eq!(note.end, 20000);
}
