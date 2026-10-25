//! Production render worker and callback consumer with no physical device.
use super::*;
use daw_core::{AudioData, Clip};

fn fixture() -> (tempfile::TempDir, Project, u64) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("constant.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [0.4, 0.2]).collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = true;
    project.loop_start = 0.0;
    project.loop_end = 2.0;
    let id = project.add_audio_track("Continuous source");
    project.track_mut(id).unwrap().gain = 1.0;
    project.track_mut(id).unwrap().clips.push(Clip::audio(
        "Source",
        0.0,
        2.0,
        path.to_string_lossy(),
    ));
    (directory, project, id)
}

fn engine(project: &Project) -> AudioEngine {
    let engine = AudioEngine::start_virtual_cancellable(
        project,
        DeviceSettings::default(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    engine.set_playing(true);
    engine
}

fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "Live update did not complete");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn virtual_callback_keeps_generation_and_audio_during_coalesced_edits() {
    let (_directory, mut project, id) = fixture();
    let mut engine = engine(&project);
    let first = engine.pump_virtual(1024).unwrap();
    assert!(first.iter().all(|frame| (frame[0] - 0.4).abs() < 1e-6));
    let before = engine.snapshot();
    for revision in 1..=32 {
        let track = project.track_mut(id).unwrap();
        track.gain = 0.25 + revision as f32 / 128.0;
        track.pan = 0.5;
        assert!(engine.try_update_project(&project, revision).unwrap());
    }
    wait_for(|| engine.snapshot().applied_revision == 32);
    let changed = engine.pump_virtual(2048).unwrap();
    assert!(
        changed.iter().all(|frame| frame[0] > 0.0 && frame[1] > 0.0),
        "No edit-created silence"
    );
    for frame in changed.iter().rev().take(256) {
        assert!(
            (frame[0] - 0.1).abs() < 1e-6,
            "Latest gain and pan must reach output: {frame:?}"
        );
        assert!((frame[1] - 0.1).abs() < 1e-6);
    }
    let after = engine.snapshot();
    assert!(after.playing);
    assert_eq!(after.engine_id, before.engine_id);
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.consumed_frames - before.consumed_frames, 2048);
    assert!(after.position_beats > before.position_beats);
    assert_eq!(after.underruns, 0);
    assert_eq!(after.render_errors, 0);
    assert_eq!(after.edit_errors, 0);
    let mut structural = project.clone();
    structural.add_bus("New graph node");
    assert!(!engine.try_update_project(&structural, 33).unwrap());
    assert_eq!(engine.snapshot().requested_revision, 32);
    engine.stop();
    assert!(engine.try_update_project(&project, 34).is_err());
}

#[test]
fn failed_live_asset_preparation_preserves_audio_and_next_valid_edit_recovers() {
    let (directory, project, id) = fixture();
    let mut engine = engine(&project);
    engine.pump_virtual(1024).unwrap();
    let before = engine.snapshot();
    let mut invalid = project.clone();
    invalid.track_mut(id).unwrap().clips[0] = Clip::audio(
        "Unavailable",
        0.0,
        2.0,
        directory.path().join("missing.wav").to_string_lossy(),
    );
    assert!(engine.try_update_project(&invalid, 1).unwrap());
    wait_for(|| engine.snapshot().edit_errors == 1);
    assert!(
        engine
            .take_edit_error()
            .unwrap()
            .contains("playback continues")
    );
    assert!(engine.take_edit_error().is_none());
    assert_eq!(engine.snapshot().applied_revision, 0);
    assert!(
        engine
            .pump_virtual(1024)
            .unwrap()
            .iter()
            .all(|frame| (frame[0] - 0.4).abs() < 1e-6)
    );
    assert!(engine.try_update_project(&project, 2).unwrap());
    wait_for(|| engine.snapshot().applied_revision == 2);
    assert!(
        engine
            .pump_virtual(1024)
            .unwrap()
            .iter()
            .all(|frame| (frame[0] - 0.4).abs() < 1e-6)
    );
    let after = engine.snapshot();
    assert!(after.playing);
    assert_eq!(after.engine_id, before.engine_id);
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.render_errors, 0);
    assert_eq!(after.underruns, 0);
}

#[test]
fn blocked_preparation_keeps_callback_fed_and_a_superseding_edit_wins() {
    let (_directory, mut project, id) = fixture();
    let mut engine = engine(&project);
    engine.pump_virtual(1024).unwrap();
    let before = engine.snapshot();
    let (started, release) = engine
        .live_updates
        .as_ref()
        .unwrap()
        .gate_next_preparation();
    project.track_mut(id).unwrap().gain = 0.1;
    assert!(engine.try_update_project(&project, 1).unwrap());
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let audio = engine.pump_virtual(8192).unwrap();
    assert!(audio.iter().all(|frame| (frame[0] - 0.4).abs() < 1e-6));
    assert_eq!(engine.snapshot().applied_revision, 0);
    assert!(engine.snapshot().playing);
    assert_eq!(
        engine.snapshot().consumed_frames - before.consumed_frames,
        8192
    );
    project.track_mut(id).unwrap().gain = 0.75;
    assert!(engine.try_update_project(&project, 2).unwrap());
    release.send(()).unwrap();
    wait_for(|| engine.snapshot().applied_revision == 2);
    let audio = engine.pump_virtual(2048).unwrap();
    // The obsolete 0.1 gain request never becomes audible, even for one block.
    assert!(audio.iter().all(|frame| frame[0] >= 0.29));
    assert!((audio.last().unwrap()[0] - 0.3).abs() < 1e-6);
    assert_eq!(engine.snapshot().generation, before.generation);
    assert_eq!(engine.snapshot().engine_id, before.engine_id);
    assert_eq!(engine.snapshot().underruns, 0);
    assert_eq!(engine.snapshot().render_errors, 0);
}

#[test]
fn accepted_timeline_update_survives_later_parameter_acknowledgement() {
    let (_directory, mut project, id) = fixture();
    // A muted VST owner has valid persisted parameter metadata without loading
    // a third-party binary; this test isolates the coordinator's metadata race.
    let owner = project.add_midi_track("Parameter owner");
    project.track_mut(owner).unwrap().mute = true;
    project.track_mut(owner).unwrap().instrument = daw_core::Instrument::Vst3 {
        path: "unused.vst3".into(),
        class_id: None,
    };
    let mut engine = engine(&project);
    engine.pump_virtual(1024).unwrap();
    let updates = engine.live_updates.as_ref().unwrap();
    let (started, release) = updates.gate_next_preparation();
    project.track_mut(id).unwrap().gain = 0.5;
    assert!(engine.try_update_project(&project, 1).unwrap());
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    // Model an accepted native-parameter command arriving after the timeline
    // request. Its new metadata cannot invalidate an already accepted clip edit.
    updates.acknowledge_parameter(owner, None, 7, 0.6);
    release.send(()).unwrap();
    wait_for(|| engine.snapshot().applied_revision == 1 || engine.snapshot().edit_errors > 0);
    assert_eq!(engine.snapshot().edit_errors, 0);
    assert_eq!(engine.snapshot().applied_revision, 1);
    let audio = engine.pump_virtual(2048).unwrap();
    assert!((audio.last().unwrap()[0] - 0.2).abs() < 1e-6);
    project
        .track_mut(owner)
        .unwrap()
        .instrument_parameters
        .push(daw_core::PluginParameterValue { id: 7, value: 0.6 });
    project.track_mut(id).unwrap().gain = 0.75;
    assert!(engine.try_update_project(&project, 2).unwrap());
    wait_for(|| engine.snapshot().applied_revision == 2);
    // An actual undo of plugin parameters remains a structural state change.
    project
        .track_mut(owner)
        .unwrap()
        .instrument_parameters
        .clear();
    assert!(!engine.try_update_project(&project, 3).unwrap());
}

#[test]
fn stopping_cancels_a_pending_preparation_without_waiting_for_its_gate() {
    let (_directory, mut project, id) = fixture();
    let mut engine = engine(&project);
    let (started, release) = engine
        .live_updates
        .as_ref()
        .unwrap()
        .gate_next_preparation();
    project.track_mut(id).unwrap().gain = 0.5;
    assert!(engine.try_update_project(&project, 1).unwrap());
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let (stopped, done) = mpsc::channel();
    let shutdown = thread::spawn(move || {
        engine.stop();
        stopped.send(()).unwrap();
    });
    let result = done.recv_timeout(Duration::from_secs(2));
    drop(release);
    shutdown.join().unwrap();
    result.expect("Shutdown must cancel preparation before joining its worker");
}
