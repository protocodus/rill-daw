//! Real-app regression for a final pointer move and release in the same frame.
//! `up` sends PointerMoved + PointerButton(false) together, deliberately without
//! the final held frame supplied by the ordinary `drag` automation command.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn open(source: Vec<Note>, bar_snap: bool) -> Harness {
    let empty = source.is_empty();
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1500,"height":1000}));
    let mut project = Project::new();
    let track = project.add_midi_track("Release test");
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("Release phrase", 0.0, 32.0, source));
    let path = app.directory.path().join("release.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    if bar_snap {
        app.click("Snap 1/16");
        app.click("1 bar");
    }
    app.click("Release phrase");
    app.click("Window");
    app.click("Clip Editor");
    if empty {
        app.click("Fit");
    }
    assert!(!app.inspect()["midi_editor"].is_null());
    app
}

fn notes(state: &Value) -> &[Value] {
    state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}

fn point(state: &Value, beat: f64, pitch: u8) -> (f64, f64) {
    let editor = &state["midi_editor"];
    let row = editor["visible_pitches"]
        .as_array()
        .unwrap()
        .iter()
        .position(|p| p.as_u64() == Some(u64::from(pitch)))
        .expect("pitch is visible") as f64;
    (
        editor["plot"]["min"][0].as_f64().unwrap()
            + (beat - editor["start_beat"].as_f64().unwrap())
                * editor["pixels_per_beat"].as_f64().unwrap(),
        editor["plot"]["min"][1].as_f64().unwrap()
            + (row + 0.5 - editor["row_offset"].as_f64().unwrap())
                * editor["row_height"].as_f64().unwrap(),
    )
}

fn pointer(app: &mut Harness, phase: &str, at: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":at.0,"y":at.1}))
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}

fn near(actual: &Value, expected: f64) {
    let actual = actual.as_f64().unwrap();
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

fn one_undo(app: &mut Harness, before: &Value, after: &Value) {
    assert_eq!(
        after["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1,
        "the held edit and release endpoint form one undo step"
    );
    assert_eq!(undo(app, false)["project"], before["project"]);
    assert_eq!(undo(app, true)["project"], after["project"]);
}

#[test]
fn repeated_chords_use_the_release_endpoint_snap_and_remembered_length() {
    for bar_snap in [false, true] {
        let mut app = open(Vec::new(), bar_snap);
        app.click("Pencil");
        let expected_length = if bar_snap { 12.0 } else { 12.5 };
        for start in [0.0, 16.0] {
            for (index, pitch) in [60, 64, 67].into_iter().enumerate() {
                let before = app.inspect();
                let from = point(&before, start + 0.1, pitch);
                // Different last-held positions model different rendering
                // schedules; the same final endpoint must win in every case.
                let sampled = point(&before, start + 4.0 + index as f64 * 2.0, pitch);
                let end = point(&before, start + 12.38, pitch);
                pointer(&mut app, "down", from);
                let held = pointer(&mut app, "move", sampled);
                assert_eq!(held["history"], before["history"]);
                let released = pointer(&mut app, "up", end);
                let note = notes(&released)
                    .iter()
                    .find(|n| n["pitch"] == pitch && n["start"] == start)
                    .expect("the drawn note retains its snapped start and pitch");
                near(&note["length"], expected_length);
                one_undo(&mut app, &before, &released);
            }
        }
        let chords = app.inspect();
        assert_eq!(notes(&chords).len(), 6);
        assert_eq!(chords["midi_editor"]["draw_length"], "Last note");
        near(
            &chords["midi_editor"]["drawing_length_beats"],
            expected_length,
        );
        let clicked = pointer(&mut app, "click", point(&chords, 4.1, 72));
        let next = notes(&clicked).iter().find(|n| n["pitch"] == 72).unwrap();
        near(&next["length"], expected_length);
        one_undo(&mut app, &chords, &clicked);
        app.quit();
    }
}

#[test]
fn moving_and_both_resize_edges_apply_motion_delivered_with_release() {
    let mut app = open(vec![Note::new(60, 2.0, 2.0, 91)], false);
    app.click("Select");
    let before = app.inspect();
    pointer(&mut app, "down", point(&before, 3.0, 60));
    pointer(&mut app, "move", point(&before, 4.0, 62));
    let moved = pointer(&mut app, "up", point(&before, 5.38, 64));
    assert_eq!(notes(&moved).len(), 1);
    assert_eq!(notes(&moved)[0]["pitch"], 64);
    near(&notes(&moved)[0]["start"], 4.5);
    near(&notes(&moved)[0]["length"], 2.0);
    one_undo(&mut app, &before, &moved);
    app.quit();

    for left in [false, true] {
        let mut app = open(vec![Note::new(60, 2.0, 4.0, 91)], false);
        app.click("Select");
        let before = app.inspect();
        let rect = &before["midi_editor"]["notes"][0]["rect"];
        let x = if left {
            rect["min"][0].as_f64().unwrap() + 2.0
        } else {
            rect["max"][0].as_f64().unwrap() - 2.0
        };
        let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
        let scale = before["midi_editor"]["pixels_per_beat"].as_f64().unwrap();
        pointer(&mut app, "down", (x, y));
        pointer(&mut app, "move", (x + 0.5 * scale, y));
        let resized = pointer(&mut app, "up", (x + 1.38 * scale, y));
        near(&notes(&resized)[0]["start"], if left { 3.5 } else { 2.0 });
        near(&notes(&resized)[0]["length"], if left { 2.5 } else { 5.5 });
        one_undo(&mut app, &before, &resized);
        app.quit();
    }
}

#[test]
fn release_length_merges_covered_notes_once_and_tool_changes_do_not_replay_it() {
    let target = Note::new(60, 6.0, 1.0, 73);
    let retained = target.id;
    let duplicate = Note::new(67, 10.0, 1.0, 82);
    let other_duplicate = Note::new(67, 10.0, 1.0, 94);
    let untouched = [duplicate.clone(), other_duplicate.clone()];
    let mut app = open(vec![target, duplicate, other_duplicate], false);
    app.click("Pencil");
    let before = app.inspect();
    pointer(&mut app, "down", point(&before, 4.0, 60));
    let held = pointer(&mut app, "move", point(&before, 5.0, 60));
    assert_eq!(notes(&held).len(), 4, "IDs stay live until release");
    let released = pointer(&mut app, "up", point(&before, 8.0, 60));
    assert_eq!(notes(&released).len(), 3);
    let merged = notes(&released)
        .iter()
        .find(|n| n["id"] == retained)
        .unwrap();
    near(&merged["start"], 4.0);
    near(&merged["length"], 4.0);
    assert_eq!(merged["velocity"], 73);
    for note in untouched {
        assert_eq!(
            notes(&released)
                .iter()
                .find(|n| n["id"] == note.id)
                .unwrap(),
            &serde_json::to_value(note).unwrap(),
            "unaffected imported overlaps must remain unchanged"
        );
    }
    one_undo(&mut app, &before, &released);
    for tool in ["Select", "Pencil", "Multi-draw", "Select"] {
        let changed = app.click(tool);
        assert_eq!(changed["project"], released["project"]);
        assert_eq!(changed["history"], released["history"]);
    }
    let state = app.inspect();
    let hovered = pointer(&mut app, "move", point(&state, 12.0, 60));
    assert_eq!(hovered["project"], released["project"]);
    app.quit();
}
