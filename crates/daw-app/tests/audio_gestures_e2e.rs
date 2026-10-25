//! Audio workspace gestures through the real headless app, with rendered oracles.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn fixture(start: f64, drums: bool, neighbour: bool) -> Harness {
    let mut app = Harness::new();
    let path = app.directory.path().join("gestures-source.wav");
    let samples = (0..96000)
        .flat_map(|frame| {
            let value = if drums {
                if (12000..14400).contains(&frame) || (60000..62400).contains(&frame) {
                    0.8
                } else {
                    0.0
                }
            } else {
                (frame as f32 * std::f32::consts::TAU * 440.0 / 48000.0).sin() * 0.4
            };
            [value, value * 0.5]
        })
        .collect();
    AudioData {
        sample_rate: 48000,
        channels: 2,
        samples,
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Audio gestures");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Edit me",
        start,
        4.0,
        path.to_string_lossy(),
    ));
    if neighbour {
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            "Neighbour",
            0.0,
            1.37,
            path.to_string_lossy(),
        ));
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            "Next neighbour",
            6.37,
            1.0,
            path.to_string_lossy(),
        ));
    }
    let project_path = app.directory.path().join("gestures.rill");
    project.save(&project_path).unwrap();
    app.command(json!({"command":"open","path":project_path}));
    app.click("Edit me");
    app.click("Window");
    app.click("Clip Editor");
    app
}

fn selected(state: &Value) -> &Value {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == state["selected_clip"])
        .unwrap()
}
fn clips(state: &Value) -> &[Value] {
    state["project"]["tracks"][0]["clips"].as_array().unwrap()
}
fn near(actual: &Value, expected: f64) {
    assert!(
        (actual.as_f64().unwrap() - expected).abs() < 1e-4,
        "{actual} != {expected}"
    );
}
fn point(state: &Value, beat: f64) -> (f64, f64) {
    let view = &state["audio_editor"];
    (
        view["rect"][0].as_f64().unwrap()
            + (beat - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap(),
        (view["rect"][1].as_f64().unwrap() + view["rect"][3].as_f64().unwrap()) / 2.0,
    )
}
fn handle(state: &Value, end: bool) -> (f64, f64) {
    let rect = &state["audio_editor"][if end { "end_handle" } else { "start_handle" }];
    assert!(!rect.is_null(), "Fit should show the actual edge: {state}");
    (
        (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) / 2.0,
        (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) / 2.0,
    )
}
fn drag(app: &mut Harness, at: (f64, f64), beats: f64, shift: bool) -> Value {
    let ppb = app.inspect()["audio_editor"]["pixels_per_beat"]
        .as_f64()
        .unwrap();
    app.command(json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":at.0 + beats * ppb,"to_y":at.1,"shift":shift}))
}
fn click_at(app: &mut Harness, beat: f64, shift: bool) -> Value {
    let at = point(&app.inspect(), beat);
    app.command(json!({"command":"pointer","phase":"click","x":at.0,"y":at.1,"shift":shift}))
}
fn undo(app: &mut Harness) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true}))
}
fn action(app: &mut Harness, menu: &str, label: &str) -> Value {
    app.click(menu);
    app.click(label)
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn analyze(app: &mut Harness) -> Value {
    action(app, "Process", "Transients");
    app.click("Analyze");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = app.inspect();
        if state["audio_editor"]["pending"] == false {
            assert_eq!(state["audio_editor"]["error"], "");
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Analysis did not finish: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn audio_select_moves_trims_both_edges_and_shift_copies_with_single_gesture_undo() {
    let mut app = fixture(1.0, false, false);
    let original = app.inspect()["project"].clone();
    let state = app.key("Q");
    assert_eq!(state["audio_editor"]["tool"], "Select");
    let moved = drag(&mut app, point(&state, 3.0), 0.5, false);
    near(&selected(&moved)["start"], 1.5);
    near(&selected(&moved)["offset"], 0.0);
    let fitted = app.click("Fit");
    let trimmed = drag(&mut app, handle(&fitted, false), 0.5, false);
    near(&selected(&trimmed)["start"], 2.0);
    near(&selected(&trimmed)["length"], 3.5);
    near(&selected(&trimmed)["offset"], 0.5);
    let fitted = app.click("Fit");
    let trimmed = drag(&mut app, handle(&fitted, true), -0.75, false);
    near(&selected(&trimmed)["length"], 2.75);
    near(&selected(&trimmed)["offset"], 0.5);
    let rendered = export(&mut app, "trimmed.wav");
    let source = AudioData::load_wav(app.directory.path().join("gestures-source.wav")).unwrap();
    // The edited clip begins at beat 2, reading beat .5 of the original source.
    assert_eq!(
        &rendered.samples[48000 * 2..48000 * 2 + 2048],
        &source.samples[12000 * 2..12000 * 2 + 2048]
    );
    near(&selected(&undo(&mut app))["length"], 3.5);
    near(&selected(&undo(&mut app))["start"], 1.5);
    assert_eq!(undo(&mut app)["project"], original);
    let fitted = app.click("Fit");
    let copied = drag(&mut app, point(&fitted, 3.0), 0.5, true);
    assert_eq!(clips(&copied).len(), 2);
    near(&selected(&copied)["start"], 1.5);
    assert!(selected(&copied)["link_group"].is_null());
    assert_eq!(undo(&mut app)["project"], original);
    app.quit();
}

#[test]
fn audio_cursor_drag_selects_a_range_and_splits_or_trims_without_modifying_source() {
    let mut app = fixture(0.0, false, false);
    let original = export(&mut app, "original.wav");
    let state = app.inspect();
    let selected_range = drag(&mut app, point(&state, 0.75), 1.75, false);
    assert_eq!(selected_range["audio_editor"]["range"], json!([0.75, 2.5]));
    assert_eq!(
        clips(&selected_range).len(),
        1,
        "Selecting time does not edit the project"
    );
    let split = action(&mut app, "Clip", "Split selection");
    assert_eq!(clips(&split).len(), 3);
    near(&selected(&split)["start"], 0.75);
    near(&selected(&split)["length"], 1.75);
    assert_eq!(export(&mut app, "split.wav").samples, original.samples);
    undo(&mut app);
    let state = app.click("Fit");
    drag(&mut app, point(&state, 0.75), 1.75, false);
    let trimmed = action(&mut app, "Clip", "Trim to selection");
    near(&selected(&trimmed)["start"], 0.75);
    near(&selected(&trimmed)["length"], 1.75);
    near(&selected(&trimmed)["offset"], 0.75);
    app.quit();
}

#[test]
fn audio_stretch_tool_changes_duration_and_keeps_pitch_in_export() {
    let mut app = fixture(0.0, false, false);
    let state = app.key("S");
    assert_eq!(state["audio_editor"]["tool"], "Stretch");
    let stretched = drag(&mut app, point(&state, 2.0), -2.0, false);
    near(&selected(&stretched)["length"], 2.0);
    near(&selected(&stretched)["source_rate"], 2.0);
    let audio = export(&mut app, "stretched.wav");
    let channel: Vec<_> = audio
        .samples
        .chunks_exact(audio.channels)
        .skip(4800)
        .take(38400)
        .map(|frame| frame[0])
        .collect();
    let crossings = channel
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    let frequency = crossings as f64 * 48000.0 / channel.len() as f64;
    assert!(
        (frequency - 440.0).abs() < 4.0,
        "Expected pitch-preserving 440 Hz, got {frequency}"
    );
    let rms =
        (channel.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / channel.len() as f64).sqrt();
    assert!(rms > 0.1, "Stretched audio must remain audible");
    near(&selected(&undo(&mut app))["length"], 4.0);
    app.quit();
}

#[test]
fn transient_markers_navigate_snap_with_shift_bypass_and_split_only_selected_time() {
    let mut app = fixture(0.0, true, false);
    let original = export(&mut app, "transients-original.wav");
    let preview = analyze(&mut app);
    assert_eq!(preview["audio_editor"]["preview"], json!([0.5, 2.5]));
    near(&app.click("Next transient")["audio_editor"]["cursor"], 0.5);
    near(&app.click("Next transient")["audio_editor"]["cursor"], 2.5);
    near(
        &app.click("Previous transient")["audio_editor"]["cursor"],
        0.5,
    );
    let closed = app.click("Close transients");
    assert_eq!(closed["audio_editor"]["preview"], json!([0.5, 2.5]));
    app.key("E");
    let split = click_at(&mut app, 0.51, false);
    near(&clips(&split)[0]["length"], 0.5);
    assert_eq!(
        export(&mut app, "transients-cut.wav").samples,
        original.samples
    );
    undo(&mut app);
    analyze(&mut app);
    app.click("Close transients");
    let unsnapped = click_at(&mut app, 0.51, true);
    near(&clips(&unsnapped)[0]["length"], 0.51);
    undo(&mut app);
    analyze(&mut app);
    let state = app.key("P");
    let range = drag(&mut app, point(&state, 2.0), 1.0, false);
    assert_eq!(range["audio_editor"]["range"], json!([2.0, 3.0]));
    let split = app.click("Split transients");
    assert_eq!(clips(&split).len(), 2);
    near(&clips(&split)[0]["length"], 2.5);
    assert_eq!(
        export(&mut app, "transients-selected.wav").samples,
        original.samples
    );
    app.quit();
}

#[test]
fn audio_edge_magnets_and_view_options_are_independent_of_grid_and_document() {
    let mut app = fixture(2.0, false, true);
    let original = app.inspect()["project"].clone();
    app.click("View");
    app.click("Show neighbouring clips");
    app.click("Show transient markers");
    let hidden = app.key("Escape");
    assert_eq!(hidden["audio_editor"]["show_neighbours"], false);
    assert_eq!(hidden["audio_editor"]["show_markers"], false);
    assert_eq!(hidden["project"], original);
    app.click("Grid 0.25");
    app.click("Off");
    let state = app.key("Q");
    assert_eq!(
        state["view"], "Clip",
        "Grid selection must leave the editor open"
    );
    assert_eq!(state["audio_editor"]["tool"], "Select");
    let moved = drag(&mut app, point(&state, 4.0), -0.62, false);
    near(&selected(&moved)["start"], 1.37);
    undo(&mut app);
    let state = app.click("Fit");
    let end_aligned = drag(&mut app, point(&state, 4.0), 0.38, false);
    near(&selected(&end_aligned)["start"], 2.37);
    undo(&mut app);
    app.click("Grid off");
    app.click("Clip edges");
    let state = app.click("Fit");
    let moved = drag(&mut app, point(&state, 4.0), -0.62, false);
    near(&selected(&moved)["start"], 1.38);
    app.quit();
}

#[test]
fn split_pieces_remain_visible_and_selectable_in_the_audio_workspace() {
    let mut app = fixture(0.0, false, false);
    app.key("E");
    click_at(&mut app, 2.0, false);
    app.key("Q");
    let chosen = click_at(&mut app, 3.0, false);
    near(&selected(&chosen)["start"], 2.0);
    near(&selected(&chosen)["length"], 2.0);
    let moved = drag(&mut app, point(&chosen, 3.0), 0.5, false);
    near(&selected(&moved)["start"], 2.5);
    near(&selected(&moved)["offset"], 2.0);
    assert_eq!(clips(&moved).len(), 2);
    app.quit();
}

#[test]
fn trimming_back_and_forth_uses_original_source_and_shift_bypasses_grid() {
    let mut app = fixture(0.0, false, false);
    let state = app.key("Q");
    let original = state["project"].clone();
    let at = handle(&state, false);
    let ppb = state["audio_editor"]["pixels_per_beat"].as_f64().unwrap();
    app.command(json!({"command":"pointer","phase":"down","x":at.0,"y":at.1}));
    for beat in [1.0, 2.0, 0.75, 1.13] {
        app.command(
            json!({"command":"pointer","phase":"move","x":at.0 + beat * ppb,"y":at.1,"shift":true}),
        );
    }
    let trimmed = app.command(
        json!({"command":"pointer","phase":"up","x":at.0 + 1.13 * ppb,"y":at.1,"shift":true}),
    );
    near(&selected(&trimmed)["start"], 1.13);
    near(&selected(&trimmed)["offset"], 1.13);
    near(&selected(&trimmed)["length"], 2.87);
    assert_eq!(
        undo(&mut app)["project"],
        original,
        "A complete drag is one history step"
    );
    app.quit();
}
