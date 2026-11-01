//! Coalesced native pointer events retain the press target in the running piano roll.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn open(notes: Vec<Note>) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    let track = project.add_midi_track("Press origin");
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("Press phrase", 0.0, 16.0, notes));
    let path = app.directory.path().join("press-origin.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Press phrase");
    app.click("Window");
    app.click("Clip Editor");
    app
}

fn point(state: &Value, beat: f64, pitch: u8) -> (f64, f64) {
    let editor = &state["midi_editor"];
    let row = editor["visible_pitches"]
        .as_array()
        .unwrap()
        .iter()
        .position(|p| p.as_u64() == Some(u64::from(pitch)))
        .unwrap();
    (
        editor["plot"]["min"][0].as_f64().unwrap()
            + (beat - editor["start_beat"].as_f64().unwrap())
                * editor["pixels_per_beat"].as_f64().unwrap(),
        editor["plot"]["min"][1].as_f64().unwrap()
            + (row as f64 + 0.5 - editor["row_offset"].as_f64().unwrap())
                * editor["row_height"].as_f64().unwrap(),
    )
}

fn notes(state: &Value) -> &[Value] {
    state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}

fn batched(app: &mut Harness, from: (f64, f64), to: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":"drag_batched",
        "x":from.0,"y":from.1,"to_x":to.0,"to_y":to.1}))
}

fn near(value: &Value, expected: f64) {
    assert!(
        (value.as_f64().unwrap() - expected).abs() < 1e-8,
        "expected {expected}, got {value}"
    );
}

fn undo_redo(app: &mut Harness, before: &Value, after: &Value) {
    app.click("Edit");
    assert_eq!(
        app.click("Undo")["project"],
        before["project"],
        "one complete gesture is one Undo entry"
    );
    app.click("Edit");
    assert_eq!(app.click("Redo")["project"], after["project"]);
}

#[test]
fn repeated_batched_pencil_drags_keep_the_pressed_onset_and_pitch() {
    let mut app = open(vec![]);
    app.click("Pencil");
    for pitch in [60, 64, 67] {
        let before = app.inspect();
        let from = point(&before, 2.0, pitch);
        // End on a different row: draw pitch belongs to the initial press.
        let to = point(&before, 6.0, pitch + 1);
        let after = batched(&mut app, from, to);
        let note = notes(&after)
            .iter()
            .find(|n| n["pitch"] == pitch)
            .expect("a drawn note retains its pressed pitch");
        near(&note["start"], 2.0);
        near(&note["length"], 4.0);
        assert_eq!(notes(&after).len(), notes(&before).len() + 1);
        undo_redo(&mut app, &before, &after);
    }
    app.quit();
}

#[test]
fn batched_movement_uses_the_note_under_press_and_keeps_its_length() {
    let original = Note::new(60, 2.0, 2.0, 90);
    let id = original.id;
    let mut app = open(vec![original]);
    app.click("Select");
    let before = app.inspect();
    let from = point(&before, 2.5, 60);
    let to = point(&before, 8.5, 60);
    let after = batched(&mut app, from, to);
    assert_eq!(notes(&after).len(), 1);
    assert_eq!(notes(&after)[0]["id"], id);
    near(&notes(&after)[0]["start"], 8.0);
    near(&notes(&after)[0]["length"], 2.0);
    undo_redo(&mut app, &before, &after);
    app.quit();
}

#[test]
fn entering_the_canvas_after_an_outside_press_does_not_draw() {
    let mut app = open(vec![]);
    app.click("Pencil");
    let before = app.inspect();
    let to = point(&before, 4.0, 64);
    // Empty space in the transport row, above the editor canvas.
    let after = batched(&mut app, (1390.0, 75.0), to);
    assert_eq!(after["project"], before["project"]);
    assert_eq!(after["history"], before["history"]);
    app.quit();
}
