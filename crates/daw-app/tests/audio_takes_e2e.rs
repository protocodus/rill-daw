//! Take selection and comping through the running app, with actual WAV oracles.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn fixture(virtual_audio: bool) -> (Harness, PathBuf) {
    let mut app = if virtual_audio {
        Harness::with_virtual_audio(None, None)
    } else {
        Harness::new()
    };
    let original = app.directory.path().join("Original.wav");
    let alternate = app.directory.path().join("Alternative.wav");
    for (path, value) in [(&original, 0.1), (&alternate, 0.4)] {
        AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: vec![value; 192000],
        }
        .write_wav(path)
        .unwrap();
    }
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = true;
    project.loop_end = 8.0;
    let track = project.add_audio_track("Voice takes");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Original",
        0.0,
        8.0,
        original.to_string_lossy(),
    ));
    let path = app.directory.path().join("takes.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Original");
    app.click("Window");
    app.click("Clip Editor");
    (app, alternate)
}
fn takes(app: &mut Harness) -> Value {
    app.click("Clip");
    app.click("Takes…")
}
fn wait(app: &mut Harness, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        assert_eq!(state["error"], false, "{}", state["status"]);
        if predicate(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "{state}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn import(app: &mut Harness, path: &PathBuf) -> Value {
    takes(app);
    app.command(json!({"command":"audio_take_files","paths":[path]}));
    app.click("Add takes…");
    wait(app, |state| {
        state["audio_takes"]["busy"] == false
            && state["audio_takes"]["rows"]
                .as_array()
                .is_some_and(|rows| rows.len() == 2)
    })
}
fn project(state: &Value) -> Project {
    serde_json::from_value(state["project"].clone()).unwrap()
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn amplitude(audio: &AudioData, seconds: f64) -> f32 {
    audio.samples[(seconds * audio.sample_rate as f64).round() as usize * audio.channels]
}
fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
}
fn select_range(app: &mut Harness, start: f64, end: f64) {
    app.key("P");
    let state = app.inspect();
    let view = &state["audio_editor"];
    let x = |beat: f64| {
        view["rect"][0].as_f64().unwrap()
            + (beat - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap()
    };
    let y = (view["rect"][1].as_f64().unwrap() + view["rect"][3].as_f64().unwrap()) / 2.0;
    app.command(
        json!({"command":"pointer","phase":"drag","x":x(start),"y":y,"to_x":x(end),"to_y":y}),
    );
}

#[test]
fn compact_take_selection_keeps_commit_actions_visible_and_stable() {
    for (dark, density) in [(false, 1.0), (true, 1.25), (true, 2.0)] {
        let (mut app, alternate) = fixture(false);
        import(&mut app, &alternate);
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":800,"height":600,"pixels_per_point":density}),
        );
        let label_rect = |state: &Value, label: &str| {
            let rect = state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == label)
                .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
            assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
            assert!(rect["max"][0].as_f64().unwrap() <= 800.0);
            assert!(rect["min"][1].as_f64().unwrap() >= 0.0);
            assert!(rect["max"][1].as_f64().unwrap() <= 600.0);
            rect.clone()
        };
        let before = app.inspect();
        label_rect(&before, "Use whole take");
        let back = label_rect(&before, "Back to editor");
        let layer = label_rect(&before, "Layer as overdub");
        // A whole take remains visible above the pinned actions. Scroll its
        // body to reach range settings, keeping the commit targets in place.
        let row = &before["audio_takes"]["rows"][0];
        let x = (row["min"][0].as_f64().unwrap() + row["max"][0].as_f64().unwrap()) * 0.5;
        let y = (row["min"][1].as_f64().unwrap() + row["max"][1].as_f64().unwrap()) * 0.5;
        assert!(row["max"][1].as_f64().unwrap() < back["min"][1].as_f64().unwrap());
        app.command(json!({"command":"scroll","x":x,"y":y,"delta":-1000.0}));
        for _ in 0..20 {
            app.inspect();
        }
        let selection = app.click("Time selection");
        label_rect(&selection, "Use selection");
        for (label, expected) in [("Back to editor", back), ("Layer as overdub", layer)] {
            let actual = label_rect(&selection, label);
            assert_eq!(
                actual["min"][1], expected["min"][1],
                "{label} moved after enabling a range"
            );
        }
        let selected = app.click("Use selection");
        assert!(!selected["error"].as_bool().unwrap());
        assert_editor_comp(&selected);
        assert_eq!(selected["audio_takes"]["open"], false);
        app.quit();
    }
}

#[test]
fn import_select_whole_take_undo_archive_and_restore() {
    let (mut app, alternate) = fixture(false);
    let state = import(&mut app, &alternate);
    assert!(state["popup_context"]["shapes"].as_u64().unwrap() > 0);
    let p = project(&state);
    assert_eq!(
        p.tracks[0]
            .clips
            .iter()
            .filter(|clip| clip.is_take_source())
            .count(),
        2
    );
    near(amplitude(&export(&mut app, "before-choice.wav"), 1.0), 0.1);
    app.click("Use whole take");
    near(amplitude(&export(&mut app, "chosen.wav"), 1.0), 0.4);
    let state = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(
        project(&state).tracks[0]
            .clips
            .iter()
            .filter(|clip| clip.is_take_source())
            .count(),
        2
    );
    near(amplitude(&export(&mut app, "undo.wav"), 1.0), 0.1);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    let bundle = app.directory.path().join("takes.rill");
    app.command(json!({"command":"save","path":bundle}));
    // The self-contained archive must include inactive takes too.
    std::fs::remove_file(alternate).unwrap();
    std::fs::remove_file(app.directory.path().join("Original.wav")).unwrap();
    app = app.restart();
    let state = app.command(json!({"command":"open","path":bundle}));
    assert_eq!(
        project(&state).tracks[0]
            .clips
            .iter()
            .filter(|clip| clip.is_take_source())
            .count(),
        2
    );
    near(amplitude(&export(&mut app, "reopened.wav"), 1.0), 0.4);
    app.quit();
}

#[test]
fn comp_time_selection_keeps_outer_audio_and_smooths_both_boundaries() {
    let (mut app, alternate) = fixture(false);
    select_range(&mut app, 2.0, 4.0);
    let state = import(&mut app, &alternate);
    assert_eq!(state["audio_takes"]["range"], json!([2.0, 4.0]));
    let state = app.click("Use selection");
    let p = project(&state);
    p.validate().unwrap();
    let regions: Vec<_> = p.tracks[0]
        .clips
        .iter()
        .filter(|clip| !clip.is_take_source())
        .collect();
    assert_eq!(regions.len(), 3);
    let audio = export(&mut app, "comp.wav");
    near(amplitude(&audio, 0.5), 0.1);
    near(amplitude(&audio, 1.5), 0.4);
    near(amplitude(&audio, 2.5), 0.1);
    near(amplitude(&audio, 1.005), 0.25);
    near(amplitude(&audio, 2.005), 0.25);
    for seconds in [1.0, 2.0] {
        let first = (seconds * 48000.0) as usize - 1;
        for frame in first..first + 482 {
            assert!(
                (audio.samples[(frame + 1) * 2] - audio.samples[frame * 2]).abs() < 0.002,
                "hard boundary at {frame}"
            );
        }
    }
    // Every comp piece remains an ordinary clip: split is lossless.
    app.key("E");
    let view = app.inspect()["audio_editor"].clone();
    let x = view["rect"][0].as_f64().unwrap()
        + (3.0 - view["start_beat"].as_f64().unwrap()) * view["pixels_per_beat"].as_f64().unwrap();
    let y = (view["rect"][1].as_f64().unwrap() + view["rect"][3].as_f64().unwrap()) / 2.0;
    let split_state = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(
        project(&split_state).tracks[0]
            .clips
            .iter()
            .filter(|clip| !clip.is_take_source())
            .count(),
        4
    );
    let split = export(&mut app, "split-comp.wav");
    assert_eq!(audio.samples, split.samples);
    app.quit();
}

#[test]
fn layer_overdub_creates_independent_colored_channel_with_same_output() {
    let (mut app, alternate) = fixture(false);
    import(&mut app, &alternate);
    let state = app.click("Layer as overdub");
    let p = project(&state);
    p.validate().unwrap();
    assert_eq!(p.tracks.len(), 2);
    assert_ne!(p.tracks[0].color, p.tracks[1].color);
    assert_eq!(p.tracks[0].output_bus, p.tracks[1].output_bus);
    assert!(p.tracks[1].clips[0].audio_take.is_none());
    assert!(p.tracks[1].clips[0].link_group.is_none());
    near(
        amplitude(&export(&mut app, "overdub.wav"), 1.0),
        0.1 + 0.4 * p.tracks[1].gain,
    );
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    near(amplitude(&export(&mut app, "undo-overdub.wav"), 1.0), 0.1);
    app.quit();
}

#[test]
fn take_change_keeps_running_transport_and_callback_audio() {
    let (mut app, alternate) = fixture(true);
    app.click("Play");
    let before = wait(&mut app, |state| state["engine"]["playing"] == true);
    import(&mut app, &alternate);
    app.click("Use whole take");
    let applied = wait(&mut app, |state| {
        state["engine"]["requested_revision"] == state["engine"]["applied_revision"]
    });
    assert_eq!(
        applied["engine"]["engine_id"],
        before["engine"]["engine_id"]
    );
    assert_eq!(
        applied["engine"]["generation"],
        before["engine"]["generation"]
    );
    assert_eq!(applied["engine"]["playing"], true);
    for _ in 0..2 {
        let state = app.command(json!({"command":"pump_audio","frames":4096}));
        assert_eq!(state["engine"]["underruns"], 0);
        assert_eq!(state["engine"]["render_errors"], 0);
        assert_eq!(state["engine"]["edit_errors"], 0);
        let samples = state["audio_output"].as_array().unwrap();
        assert_eq!(samples.len(), 4096);
        for frame in samples {
            for channel in frame.as_array().unwrap() {
                let sample = channel.as_f64().unwrap();
                assert!(
                    (0.0999..=0.4001).contains(&sample),
                    "silence or invalid sample during take change: {frame}"
                );
            }
        }
        assert!(samples.windows(2).all(|pair| {
            (pair[1][0].as_f64().unwrap() - pair[0][0].as_f64().unwrap()).abs() < 0.002
        }));
        for frame in &samples[samples.len() - 512..] {
            near(frame[0].as_f64().unwrap() as f32, 0.4);
            near(frame[1].as_f64().unwrap() as f32, 0.4);
        }
    }
    app.click("Stop");
    app.quit();
}

fn assert_editor_comp(state: &Value) {
    assert_eq!(state["view"], "Clip");
    assert_eq!(state["audio_takes"]["open"], false);
    let p = project(state);
    let selected = p
        .clip(
            state["selected_track"].as_u64().unwrap(),
            state["selected_clip"].as_u64().unwrap(),
        )
        .unwrap();
    assert!(
        !selected.is_take_source(),
        "the editor must return to audible audio"
    );
    assert!(!state["audio_editor"].is_null());
}

fn choose_candidate(app: &mut Harness, name: &str) -> Value {
    let state = app.inspect();
    let p = project(&state);
    let source = p
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.is_take_source() && clip.name == name)
        .unwrap();
    let row = state["audio_takes"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == source.id)
        .unwrap();
    let x = (row["min"][0].as_f64().unwrap() + row["max"][0].as_f64().unwrap()) / 2.0;
    let y = (row["min"][1].as_f64().unwrap() + row["max"][1].as_f64().unwrap()) / 2.0;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

#[test]
fn group_overlapping_clips_keeps_selected_take_and_back_returns_to_audible_comp() {
    let (mut app, alternate) = fixture(false);
    let mut p = project(&app.inspect());
    p.tracks[0].clips.push(Clip::audio(
        "Alternative",
        2.0,
        4.0,
        alternate.to_string_lossy(),
    ));
    let file = app.directory.path().join("overlapping.daw.json");
    p.save(&file).unwrap();
    app.command(json!({"command":"open","path":file}));
    app.click("Original");
    app.click("Window");
    app.click("Clip Editor");
    near(amplitude(&export(&mut app, "overlap-before.wav"), 1.5), 0.4);
    takes(&mut app);
    let grouped = app.click("Group overlapping clips");
    let p = project(&grouped);
    p.validate().unwrap();
    assert_eq!(
        p.tracks[0]
            .clips
            .iter()
            .filter(|clip| clip.is_take_source())
            .count(),
        2
    );
    assert_eq!(
        p.tracks[0]
            .clips
            .iter()
            .filter(|clip| !clip.is_take_source())
            .count(),
        1
    );
    let back = app.click("Back to editor");
    assert_editor_comp(&back);
    near(amplitude(&export(&mut app, "grouped.wav"), 1.5), 0.1);
    let undo = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert!(
        project(&undo).tracks[0]
            .clips
            .iter()
            .all(|clip| clip.audio_take.is_none())
    );
    near(amplitude(&export(&mut app, "undo-group.wav"), 1.5), 0.4);
    app.quit();
}

#[test]
fn escape_and_back_after_import_restore_the_editor_without_selecting_retained_sources() {
    let (mut app, alternate) = fixture(false);
    let imported = import(&mut app, &alternate);
    let escaped = app.key("Escape");
    assert_editor_comp(&escaped);
    assert_eq!(escaped["project"], imported["project"]);
    takes(&mut app);
    let back = app.click("Back to editor");
    assert_editor_comp(&back);
    assert_eq!(back["project"], imported["project"]);
    assert_eq!(back["history"], imported["history"]);
    near(
        amplitude(&export(&mut app, "after-navigation.wav"), 1.0),
        0.1,
    );
    app.quit();
}

#[test]
fn track_menu_recovers_retained_takes_after_last_comp_is_deleted() {
    let (mut app, alternate) = fixture(false);
    import(&mut app, &alternate);
    app.click("Back to editor");
    app.click("Edit");
    let deleted = app.click("Delete");
    let p = project(&deleted);
    p.validate().unwrap();
    assert_eq!(p.tracks[0].clips.len(), 2);
    assert!(p.tracks[0].clips.iter().all(Clip::is_take_source));
    assert_eq!(p.end_beat(), 0.0);
    app.click("Track");
    let reopened = app.click("Audio takes…");
    assert_eq!(reopened["audio_takes"]["rows"].as_array().unwrap().len(), 2);
    choose_candidate(&mut app, "Alternative");
    let recovered = app.click("Use whole take");
    assert_editor_comp(&recovered);
    assert_eq!(project(&recovered).tracks[0].clips.len(), 3);
    near(amplitude(&export(&mut app, "recovered.wav"), 1.0), 0.4);
    app.quit();
}

#[test]
fn keep_comp_only_preserves_rendered_audio_and_undo_restores_alternatives() {
    let (mut app, alternate) = fixture(false);
    select_range(&mut app, 2.0, 4.0);
    import(&mut app, &alternate);
    let comp = app.click("Use selection");
    let audio = export(&mut app, "before-detach.wav");
    takes(&mut app);
    app.click("Takes");
    let detached = app.click("Keep comp only");
    assert_editor_comp(&detached);
    let p = project(&detached);
    p.validate().unwrap();
    assert_eq!(p.tracks[0].clips.len(), 3);
    assert!(
        p.tracks[0]
            .clips
            .iter()
            .all(|clip| clip.audio_take.is_none())
    );
    assert!(alternate.exists() && app.directory.path().join("Original.wav").exists());
    assert_eq!(export(&mut app, "after-detach.wav").samples, audio.samples);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], comp["project"]);
    assert_eq!(
        project(&undone).tracks[0]
            .clips
            .iter()
            .filter(|clip| clip.is_take_source())
            .count(),
        2
    );
    assert_eq!(export(&mut app, "undo-detach.wav").samples, audio.samples);
    app.quit();
}

#[test]
fn take_group_picker_and_waveform_rows_choose_only_the_requested_group() {
    let (mut app, alternate) = fixture(false);
    let mut p = project(&app.inspect());
    let track = p.tracks[0].id;
    let first = p.tracks[0].clips[0].id;
    let second = Clip::audio("Alternative", 0.0, 8.0, alternate.to_string_lossy());
    let second_id = second.id;
    p.tracks[0].clips.push(second);
    let first_group = p.group_audio_takes(track, &[first, second_id]).unwrap();
    let original = app.directory.path().join("Original.wav");
    let a = Clip::audio("Second original", 8.0, 8.0, original.to_string_lossy());
    let b = Clip::audio("Second alternative", 8.0, 8.0, alternate.to_string_lossy());
    let ids = [a.id, b.id];
    p.tracks[0].clips.extend([a, b]);
    let second_group = p.group_audio_takes(track, &ids).unwrap();
    let file = app.directory.path().join("two-groups.daw.json");
    p.save(&file).unwrap();
    app.command(json!({"command":"open","path":file}));
    app.click("Track");
    let opened = app.click("Audio takes…");
    assert_eq!(opened["audio_takes"]["anchor"], first);
    app.click("Take group");
    let switched = app.click("Second original");
    assert_eq!(switched["audio_takes"]["anchor"], ids[0]);
    let row_ids = switched["audio_takes"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(row_ids, ids);
    let selected = choose_candidate(&mut app, "Second alternative");
    assert_eq!(selected["audio_takes"]["source"], ids[1]);
    let chosen = app.click("Use whole take");
    let p = project(&chosen);
    assert_eq!(p.audio_take_regions(track, first_group)[0].name, "Original");
    assert_eq!(
        p.audio_take_regions(track, second_group)[0].name,
        "Second alternative"
    );
    let audio = export(&mut app, "groups.wav");
    near(amplitude(&audio, 1.0), 0.1);
    near(amplitude(&audio, 5.0), 0.4);
    // Picking a source by its waveform must work both ways, independently of
    // the import path's automatic initial selection.
    app.click("Track");
    app.click("Audio takes…");
    app.click("Take group");
    app.click("Second original");
    choose_candidate(&mut app, "Second original");
    app.click("Use whole take");
    near(amplitude(&export(&mut app, "group-restored.wav"), 5.0), 0.1);
    app.quit();
}

#[test]
fn shorter_take_requires_fitting_the_selection_before_comping() {
    let (mut app, _) = fixture(false);
    select_range(&mut app, 2.0, 6.0);
    let short = app.directory.path().join("Short.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.4; 72_000],
    }
    .write_wav(&short)
    .unwrap();
    let imported = import(&mut app, &short);
    assert_eq!(imported["audio_takes"]["range"], json!([2.0, 6.0]));
    assert!(
        imported["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Fit selection to take")
    );
    let disabled = app.click("Use selection");
    assert_eq!(disabled["project"], imported["project"]);
    assert_eq!(disabled["history"], imported["history"]);
    assert_eq!(disabled["error"], false);
    assert_eq!(disabled["audio_takes"]["open"], true);
    let fitted = app.click("Fit selection to take");
    assert_eq!(fitted["audio_takes"]["range"], json!([2.0, 3.0]));
    assert_eq!(fitted["project"], imported["project"]);
    assert_eq!(fitted["history"], imported["history"]);
    let selected = app.click("Use selection");
    assert_editor_comp(&selected);
    project(&selected).validate().unwrap();
    let audio = export(&mut app, "short-comp.wav");
    near(amplitude(&audio, 0.5), 0.1);
    near(amplitude(&audio, 1.25), 0.4);
    near(amplitude(&audio, 2.0), 0.1);
    app.quit();
}

#[test]
fn cancelled_and_invalid_imports_leave_project_and_history_unchanged() {
    let (mut app, _) = fixture(false);
    let before = app.inspect();
    takes(&mut app);
    app.command(json!({"command":"audio_take_files","paths":null}));
    let state = app.click("Add takes…");
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
    let absent = app.directory.path().join("absent.wav");
    app.command(json!({"command":"audio_take_files","paths":[absent]}));
    app.click("Add takes…");
    let deadline = Instant::now() + Duration::from_secs(5);
    let state = loop {
        let state = app.inspect();
        if state["audio_takes"]["busy"] == false {
            break state;
        }
        assert!(Instant::now() < deadline);
    };
    assert_eq!(state["error"], true);
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
    app.click("Dismiss");
    app.quit();
}
