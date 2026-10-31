//! Mouse note entry and duration choices through the running piano roll.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn open(ghost: bool, meter: u8) -> (Harness, u64, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    project.beats_per_bar = meter;
    let track = project.add_midi_track("Note entry");
    let source = Clip::midi(
        "Mouse phrase",
        0.0,
        f64::from(meter),
        vec![Note::new(60, 0.5, 0.5, 90)],
    );
    let original = source.id;
    project.track_mut(track).unwrap().clips.push(source);
    let selected = if ghost {
        let id = project.clone_clip(track, original).unwrap();
        let clip = project.clip_mut(track, id).unwrap();
        clip.offset = 1.5;
        clip.source_rate = 0.75;
        id
    } else {
        original
    };
    let path = app.directory.path().join("note-entry.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == selected)
        .unwrap()["rect"];
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.click("Window");
    let state = app.click("Clip Editor");
    assert_eq!(state["selected_clip"], selected);
    (app, original, selected)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn point(state: &Value, beat: f64, pitch: u8) -> (f64, f64) {
    let editor = &state["midi_editor"];
    let row = editor["visible_pitches"]
        .as_array()
        .unwrap()
        .iter()
        .position(|value| value.as_u64() == Some(u64::from(pitch)))
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

fn enter(app: &mut Harness, phase: &str, beat: f64, pitch: u8, end: Option<f64>) -> Value {
    let state = app.inspect();
    let (x, y) = point(&state, beat, pitch);
    let mut command = json!({"command":"pointer","phase":phase,"x":x,"y":y});
    if let Some(end) = end {
        let (to_x, to_y) = point(&state, end, pitch);
        command["to_x"] = json!(to_x);
        command["to_y"] = json!(to_y);
    }
    app.command(command)
}

fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn note(state: &Value, id: u64, pitch: u8, start: f64) -> &Value {
    clip(state, id)["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| {
            note["pitch"] == pitch && (note["start"].as_f64().unwrap() - start).abs() < 1e-8
        })
        .unwrap()
}

fn near(value: &Value, expected: f64) {
    assert!(
        (value.as_f64().unwrap() - expected).abs() < 1e-8,
        "{value} != {expected}"
    );
}

fn undo_redo(app: &mut Harness, before: &Value, after: &Value) {
    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(
        undone["project"], before["project"],
        "one gesture has one Undo step"
    );
    app.click("Edit");
    assert_eq!(app.click("Redo")["project"], after["project"]);
}

fn choose_length(app: &mut Harness, choice: &str) -> Value {
    let state = app.inspect();
    let selected = state["midi_editor"]["draw_length"].as_str().unwrap();
    app.click("Draw settings");
    app.click(&format!("Length · {selected}"));
    app.click(choice)
}

#[test]
fn clicks_drags_and_double_clicks_extend_only_the_selected_stretched_ghost_bounds() {
    let (mut app, original, selected) = open(true, 4);
    let baseline = app.inspect();
    assert!(!baseline["midi_editor"]["clip_end_label"].is_null());
    app.click("Pencil");
    let before = app.inspect();
    let clicked = enter(&mut app, "click", 5.0, 64, None);
    near(&note(&clicked, selected, 64, 5.25)["length"], 0.1875);
    near(&clip(&clicked, selected)["length"], 5.25);
    undo_redo(&mut app, &before, &clicked);

    let before = app.inspect();
    let dragged = enter(&mut app, "drag", 6.0, 66, Some(8.0));
    near(&note(&dragged, selected, 66, 6.0)["length"], 1.5);
    near(&clip(&dragged, selected)["length"], 8.0);
    undo_redo(&mut app, &before, &dragged);

    app.click("Select");
    let before = app.inspect();
    let doubled = enter(&mut app, "double_click", 9.0, 68, None);
    near(&note(&doubled, selected, 68, 8.25)["length"], 1.5);
    near(&clip(&doubled, selected)["length"], 11.0);
    undo_redo(&mut app, &before, &doubled);
    for field in ["start", "length", "offset", "source_rate", "link_group"] {
        assert_eq!(
            clip(&doubled, original)[field],
            clip(&baseline, original)[field],
            "peer {field} stays local"
        );
    }
    for field in ["start", "offset", "source_rate", "link_group"] {
        assert_eq!(
            clip(&doubled, selected)[field],
            clip(&baseline, selected)[field]
        );
    }
    assert_eq!(
        clip(&doubled, original)["content"],
        clip(&doubled, selected)["content"],
        "source notes remain shared"
    );
    let path = app.directory.path().join("extended.rill");
    app.command(json!({"command":"save","path":path}));
    let loaded = Project::load(path).unwrap();
    near(
        &json!(loaded.clip(loaded.tracks[0].id, selected).unwrap().length),
        11.0,
    );
    loaded.validate().unwrap();
    app.quit();
}

#[test]
fn mouse_length_choices_use_the_meter_and_apply_to_all_note_entry_tools() {
    let (mut app, _, selected) = open(false, 3);
    let baseline = app.inspect();
    let drawing = app.click("Draw settings");
    assert_eq!(drawing["project"], baseline["project"]);
    assert_eq!(drawing["history"], baseline["history"]);
    assert!(
        drawing["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Grid scale")
    );
    app.key("Escape");
    let choice = choose_length(&mut app, "1 bar");
    assert_eq!(choice["project"], baseline["project"]);
    assert_eq!(choice["history"], baseline["history"]);
    app.click("Pencil");
    let state = enter(&mut app, "click", 4.0, 64, None);
    near(&note(&state, selected, 64, 4.0)["length"], 3.0);
    near(&clip(&state, selected)["length"], 7.0);

    choose_length(&mut app, "1/8 note");
    app.click("Select");
    let state = enter(&mut app, "double_click", 8.0, 62, None);
    near(&note(&state, selected, 62, 8.0)["length"], 0.5);

    choose_length(&mut app, "1/2 note");
    app.click("Multi-draw");
    let state = enter(&mut app, "click", 9.0, 60, None);
    for pitch in [60, 64, 67] {
        near(&note(&state, selected, pitch, 9.0)["length"], 2.0);
    }
    app.click("Pencil");
    let state = enter(&mut app, "drag", 12.0, 65, Some(13.0));
    near(&note(&state, selected, 65, 12.0)["length"], 1.0);
    let state = enter(&mut app, "click", 14.0, 66, None);
    near(&note(&state, selected, 66, 14.0)["length"], 2.0);
    near(&clip(&state, selected)["length"], 16.0);
    choose_length(&mut app, "Last note");
    let state = enter(&mut app, "click", 16.0, 67, None);
    near(&note(&state, selected, 67, 16.0)["length"], 1.0);
    app.quit();
}
