//! New MIDI tracks inherit the musical position clicked on the empty arranger.
mod common;

use common::Harness;
use daw_core::{MAX_BEAT, Project};
use serde_json::{Value, json};

fn fixture(with_audio_track: bool) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    if with_audio_track {
        let mut project = Project::new();
        project.add_audio_track("Starter audio");
        let path = app.directory.path().join("empty-midi-lane.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
    }
    app
}

fn empty_point(state: &Value, beat: f64) -> (f64, f64) {
    let timeline = &state["timeline"];
    let empty = &timeline["empty_canvas"];
    let x = timeline["canvas"]["min"][0].as_f64().unwrap()
        + (beat - timeline["start_beat"].as_f64().unwrap())
            * timeline["pixels_per_beat"].as_f64().unwrap();
    // Stay below the first-use action cards as well as any existing track.
    let y = empty["max"][1].as_f64().unwrap() - 24.0;
    assert!(
        x > empty["min"][0].as_f64().unwrap()
            && x < empty["max"][0].as_f64().unwrap()
            && y > empty["min"][1].as_f64().unwrap(),
        "The test click must be inside the visible empty arranger: {timeline}"
    );
    (x, y)
}

fn pointer(app: &mut Harness, phase: &str, point: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":point.0,"y":point.1}))
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(state[field], before[field], "Interaction changed {field}");
    }
}

fn assert_new_midi_at(state: &Value, before: &Value, start: f64) {
    let original_tracks = before["project"]["tracks"].as_array().unwrap();
    let tracks = state["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), original_tracks.len() + 1);
    assert_eq!(&tracks[..original_tracks.len()], original_tracks.as_slice());
    let track = tracks.last().unwrap();
    assert_eq!(track["kind"], "Midi");
    let clips = track["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1);
    let clip = &clips[0];
    assert_eq!(clip["start"], start);
    assert_eq!(clip["length"], 4.0);
    assert_eq!(state["selected_track"], track["id"]);
    assert_eq!(state["selected_clip"], clip["id"]);
    assert_eq!(state["popups"]["library"], true);
    assert_eq!(
        state["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
}

fn assert_undo_redo(app: &mut Harness, before: &Value, created: &Value) {
    app.key("Escape");
    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"], before["project"]);
    assert_eq!(undone["history"]["undo"], before["history"]["undo"]);
    app.click("Edit");
    let redone = app.click("Redo");
    assert_eq!(redone["project"], created["project"]);
    assert_eq!(redone["history"]["undo"], created["history"]["undo"]);
}

#[test]
fn double_click_at_bar_three_creates_the_first_midi_clip_there_with_one_undo() {
    for (with_audio_track, phase) in [
        (false, "double_click"),
        (true, "double_click"),
        (true, "double_click_batched"),
    ] {
        let mut app = fixture(with_audio_track);
        let before = app.inspect();
        assert_eq!(before["timeline"]["start_beat"], 0.0);
        // Bar 3 is zero-based beat 8. The nearby click must use the 1/16 grid.
        let point = empty_point(&before, 8.09);
        let created = pointer(&mut app, phase, point);
        assert_new_midi_at(&created, &before, 8.0);
        assert_undo_redo(&mut app, &before, &created);
        app.quit();
    }
}

#[test]
fn new_track_placement_uses_the_scrolled_zoomed_timeline_and_current_snap() {
    let mut app = fixture(true);
    app.click("Snap 1/16");
    let before = app.click("1/8");
    let point = empty_point(&before, 8.0);
    let zoomed = app.command(json!({
        "command":"scroll","x":point.0,"y":point.1,"delta":120.0,"ctrl":true
    }));
    assert!(
        zoomed["timeline"]["pixels_per_beat"].as_f64().unwrap()
            > before["timeline"]["pixels_per_beat"].as_f64().unwrap()
    );
    let scrolled = app.command(json!({
        "command":"scroll","x":point.0,"y":point.1,"delta":-320.0,"shift":true
    }));
    assert!(
        scrolled["timeline"]["start_beat"].as_f64().unwrap()
            > zoomed["timeline"]["start_beat"].as_f64().unwrap()
    );
    unchanged(&scrolled, &before);

    let expected = scrolled["timeline"]["start_beat"].as_f64().unwrap().ceil() + 4.5;
    let point = empty_point(&scrolled, expected + 0.12);
    let created = pointer(&mut app, "double_click", point);
    assert_new_midi_at(&created, &before, expected);
    assert_undo_redo(&mut app, &before, &created);
    app.quit();
}

#[test]
fn single_click_and_navigation_on_the_empty_arranger_do_not_create_tracks() {
    let mut app = fixture(true);
    let before = app.inspect();
    let point = empty_point(&before, 8.0);
    let clicked = pointer(&mut app, "click", point);
    unchanged(&clicked, &before);
    assert_eq!(clicked["popups"]["library"], false);

    // Alt-left is navigation even when the two clicks would otherwise qualify
    // for background MIDI creation.
    let navigation = app.command(json!({
        "command":"pointer","phase":"double_click","alt":true,"x":point.0,"y":point.1
    }));
    unchanged(&navigation, &before);
    assert_eq!(navigation["popups"]["library"], false);
    let panned = app.command(json!({
        "command":"pointer","phase":"drag","button":"middle",
        "x":point.0,"y":point.1,"to_x":point.0-120.0,"to_y":point.1
    }));
    assert!(
        panned["timeline"]["start_beat"].as_f64().unwrap()
            > before["timeline"]["start_beat"].as_f64().unwrap()
    );
    unchanged(&panned, &before);
    assert_eq!(panned["popups"]["library"], false);
    app.quit();
}

#[test]
fn empty_workspace_actions_and_modal_backdrop_do_not_create_a_pattern() {
    let mut app = fixture(false);
    let before = app.inspect();
    let point = empty_point(&before, 8.0);
    let library = app.click("Choose instrument…");
    assert_eq!(library["popups"]["library"], true);
    unchanged(&library, &before);
    let backdrop = pointer(&mut app, "double_click", point);
    unchanged(&backdrop, &before);
    assert!(backdrop["project"]["tracks"].as_array().unwrap().is_empty());
    app.quit();
}

#[test]
fn pattern_creation_at_and_beyond_the_timeline_limit_stays_valid_and_undoable() {
    for (route, view_start, offset) in [
        ("empty", MAX_BEAT - 8.0, 7.0),
        ("empty", MAX_BEAT + 8.0, 4.0),
        ("track", MAX_BEAT + 8.0, 4.0),
        ("toolbar", MAX_BEAT + 8.0, 4.0),
    ] {
        let mut app = fixture(false);
        if route != "empty" {
            let mut project = Project::new();
            project.add_midi_track("Existing instrument");
            let path = app.directory.path().join("existing-instrument.rill");
            project.save(&path).unwrap();
            app.command(json!({"command":"open","path":path}));
            app.click("Existing instrument");
        }
        let before = app.inspect();
        let point = empty_point(&before, 8.0);
        let delta = -view_start * before["timeline"]["pixels_per_beat"].as_f64().unwrap();
        let panned = app.command(json!({
            "command":"scroll","x":point.0,"y":point.1,"delta":delta,"shift":true
        }));
        unchanged(&panned, &before);
        let requested = panned["timeline"]["start_beat"].as_f64().unwrap() + offset;
        assert!(requested > MAX_BEAT - 4.0);
        let (x, mut y) = empty_point(&panned, requested);
        let created = match route {
            "track" => {
                let header = &panned["timeline"]["tracks"][0]["header"];
                y = (header["min"][1].as_f64().unwrap() + header["max"][1].as_f64().unwrap()) * 0.5;
                pointer(&mut app, "double_click", (x, y))
            }
            "toolbar" => {
                let ruler = &panned["timeline"]["ruler"];
                y = (ruler["min"][1].as_f64().unwrap() + ruler["max"][1].as_f64().unwrap()) * 0.5;
                let cursor = pointer(&mut app, "click", (x, y));
                assert!(cursor["timeline"]["cursor"].as_f64().unwrap() > MAX_BEAT - 4.0);
                app.click("Add pattern")
            }
            _ => pointer(&mut app, "double_click", (x, y)),
        };
        let tracks = created["project"]["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 1, "Creation route: {route}");
        assert_eq!(tracks[0]["clips"].as_array().unwrap().len(), 1);
        assert_eq!(tracks[0]["clips"][0]["start"], MAX_BEAT - 4.0);
        assert_eq!(tracks[0]["clips"][0]["length"], 4.0);
        let project: Project = serde_json::from_value(created["project"].clone()).unwrap();
        project.validate().unwrap();
        assert_undo_redo(&mut app, &before, &created);

        let path = app.directory.path().join("valid-boundary-pattern.rill");
        let saved = app.command(json!({"command":"save","path":path}));
        assert_eq!(saved["modified"], false);
        let loaded = Project::load(&path).unwrap();
        loaded.validate().unwrap();
        assert_eq!(json!(loaded)["tracks"], created["project"]["tracks"]);
        app.quit();
    }
}
