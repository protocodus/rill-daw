//! Selected-note duration previews and real group gestures retain source precision.
mod common;

use common::Harness;
use daw_core::{Clip, Midi2NoteData, MidiNoteExpression, Note, Project, UmpPacket};
use serde_json::{Value, json};

const A: u64 = 90_001;
const B: u64 = 90_002;

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == id)
        .unwrap()
}
fn notes(state: &Value, id: u64) -> Vec<Value> {
    clip(state, id)["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
        .clone()
}
fn note(state: &Value, clip: u64, id: u64) -> Value {
    notes(state, clip)
        .into_iter()
        .find(|note| note["id"] == id)
        .unwrap()
}
fn history(state: &Value) -> usize {
    state["history"]["undo"].as_array().unwrap().len()
}
fn near(actual: &Value, expected: f64) {
    let actual = actual.as_f64().unwrap();
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}
fn menu(app: &mut Harness, menu: &str, action: &str) -> Value {
    app.click(menu);
    app.click(action)
}
fn undo(app: &mut Harness) -> Value {
    menu(app, "Edit", "Undo")
}
fn rect(state: &Value, id: u64) -> Value {
    state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == id)
        .unwrap_or_else(|| panic!("Note {id} absent: {}", state["midi_editor"]))["rect"]
        .clone()
}
fn select(app: &mut Harness, id: u64, add: bool) -> Value {
    let state = app.inspect();
    let (x, y) = center(&rect(&state, id));
    let selected = app.command(json!({"command":"pointer","phase":"click","button":if add {"secondary"} else {"primary"},"x":x,"y":y}));
    if add {
        app.click("Add to selection")
    } else {
        selected
    }
}
fn fixture(rate: f64) -> (Harness, u64, u64) {
    fixture_with_container(rate, false)
}
fn fixture_with_container(rate: f64, containing_note: bool) -> (Harness, u64, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    let track = project.add_midi_track("Batch note editing");
    let mut source = vec![
        Note::new(40, 0.125, 0.125, 50),
        Note::new(60, 1.0, 0.25, 81),
        Note::new(64, 2.0, 0.75, 91),
        Note::new(72, 3.25, 0.25, 103),
        Note::new(90, 9.0, 0.5, 110),
    ];
    if containing_note {
        source.push(Note::new(60, 2.9, 0.75, 77));
    }
    for (index, note) in source.iter_mut().enumerate() {
        note.id = 90_000 + index as u64;
    }
    let mut data = Midi2NoteData::from_legacy(&source[1]);
    data.protocol = 2;
    data.velocity = 43_001;
    data.release_velocity = 11_113;
    data.expressions = [0.0, 0.1, 0.2, 0.4, 0.8]
        .into_iter()
        .enumerate()
        .map(|(index, beat)| {
            MidiNoteExpression::new(
                beat,
                UmpPacket::from_words(&[0x40a0_3c00, 0x8000_0001 + index as u32]).unwrap(),
            )
        })
        .collect();
    source[1].set_midi2(data).unwrap();
    source[2].set_velocity16(51_019);
    let mut source = Clip::midi("Precision source", 2.0, 8.0, source);
    source.offset = 0.5;
    source.source_rate = rate;
    let id = source.id;
    project.track_mut(track).unwrap().clips.push(source);
    let ghost = project.clone_clip(track, id).unwrap();
    let linked = project.clip_mut(track, ghost).unwrap();
    linked.start = 12.0;
    linked.offset = 0.25;
    linked.source_rate = 1.3;
    linked.length = 4.0;
    let path = app.directory.path().join("batch-source.rill");
    project.save(&path).unwrap();
    let state = app.command(json!({"command":"open","path":path}));
    let area = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    let (x, y) = center(area);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    app.click("Edit notes");
    app.key("Q");
    select(&mut app, A, false);
    select(&mut app, B, true);
    (app, id, ghost)
}
fn lengths(app: &mut Harness, mode: &str, amount: &str) -> Value {
    menu(app, "Notes", "Note lengths…");
    app.click(mode);
    let state = app.inspect();
    let (x, y) = center(&state["midi_editor"]["length_edit"]["amount_rect"]);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":amount}));
    app.click("Note length preview")
}
fn unchanged_others(after: &Value, before: &Value, id: u64) {
    for old in notes(before, id) {
        if old["id"] != A && old["id"] != B {
            assert_eq!(note(after, id, old["id"].as_u64().unwrap()), old);
        }
    }
}
fn bounds(after: &Value, before: &Value, id: u64) {
    for key in [
        "id",
        "start",
        "offset",
        "length",
        "source_rate",
        "link_group",
    ] {
        assert_eq!(clip(after, id)[key], clip(before, id)[key], "{key}");
    }
}
fn drag(app: &mut Harness, id: u64, edge: i8, beats: f64, ctrl: bool) -> Value {
    let state = app.inspect();
    let r = rect(&state, id);
    let (center_x, y) = center(&r);
    let x = match edge {
        -1 => r["min"][0].as_f64().unwrap() + 1.0,
        1 => r["max"][0].as_f64().unwrap() - 1.0,
        _ => center_x,
    };
    let ppb = state["midi_editor"]["pixels_per_beat"].as_f64().unwrap();
    app.command(json!({"command":"pointer","phase":"drag","ctrl":ctrl,"x":x,"y":y,"to_x":x+beats*ppb,"to_y":y}))
}

#[test]
fn fixed_grow_shrink_previews_are_atomic_cancelable_and_keep_expression_ghosts_and_recall() {
    let (mut app, id, ghost) = fixture(0.5);
    app.command(json!({"command":"viewport","width":900,"height":700}));
    let original = app.inspect();
    let preview = lengths(&mut app, "Fixed", "0.75");
    assert_eq!(preview["project"], original["project"]);
    assert_eq!(preview["history"], original["history"]);
    assert_eq!(preview["midi_editor"]["length_edit"]["changed"], 2);
    let canceled = app.click("Cancel lengths");
    assert_eq!(canceled["project"], original["project"]);
    assert_eq!(canceled["history"], original["history"]);
    lengths(&mut app, "Fixed", "0.75");
    let fixed = app.click("Apply lengths");
    for n in [A, B] {
        near(&note(&fixed, id, n)["length"], 0.375);
        for key in ["id", "start", "pitch", "velocity", "midi2"] {
            assert_eq!(note(&fixed, id, n)[key], note(&original, id, n)[key]);
        }
    }
    unchanged_others(&fixed, &original, id);
    assert_eq!(notes(&fixed, id), notes(&fixed, ghost));
    bounds(&fixed, &original, id);
    bounds(&fixed, &original, ghost);
    assert_eq!(history(&fixed), history(&original) + 1);
    assert_eq!(undo(&mut app)["project"], original["project"]);
    assert_eq!(menu(&mut app, "Edit", "Redo")["project"], fixed["project"]);
    assert_eq!(undo(&mut app)["project"], original["project"]);
    lengths(&mut app, "Grow", "0.5");
    let grown = app.click("Apply lengths");
    near(&note(&grown, id, A)["length"], 0.5);
    near(&note(&grown, id, B)["length"], 1.0);
    assert_eq!(history(&grown), history(&original) + 1);
    assert_eq!(undo(&mut app)["project"], original["project"]);
    let preview = lengths(&mut app, "Shrink", "2");
    near(
        &preview["midi_editor"]["length_edit"]["applied_delta"],
        -0.484375,
    );
    let shrunk = app.click("Apply lengths");
    near(&note(&shrunk, id, A)["length"], 0.0078125);
    near(&note(&shrunk, id, B)["length"], 0.5078125);
    assert_eq!(
        note(&shrunk, id, A)["midi2"],
        note(&original, id, A)["midi2"]
    );
    unchanged_others(&shrunk, &original, id);
    assert_eq!(notes(&shrunk, id), notes(&shrunk, ghost));
    assert_eq!(history(&shrunk), history(&original) + 1);
    let path = app.directory.path().join("retained-batch.rill");
    app.command(json!({"command":"save","path":path}));
    let restored = app.command(json!({"command":"open","path":path}));
    assert_eq!(restored["project"], shrunk["project"]);
    app.quit();
}

#[test]
fn group_edge_resizing_uses_one_delta_and_preserves_per_note_controller_absolute_time() {
    let (mut app, id, ghost) = fixture(0.5);
    let original = app.inspect();
    let grown = drag(&mut app, A, 1, 0.75, false);
    near(&note(&grown, id, A)["length"], 0.625);
    near(&note(&grown, id, B)["length"], 1.125);
    assert_eq!(
        note(&grown, id, A)["midi2"],
        note(&original, id, A)["midi2"]
    );
    assert_eq!(history(&grown), history(&original) + 1);
    assert_eq!(notes(&grown, id), notes(&grown, ghost));
    assert_eq!(undo(&mut app)["project"], original["project"]);
    let shrunk = drag(&mut app, A, 1, -1.0, false);
    near(&note(&shrunk, id, A)["length"], 0.0078125);
    near(&note(&shrunk, id, B)["length"], 0.5078125);
    assert_eq!(
        note(&shrunk, id, A)["midi2"],
        note(&original, id, A)["midi2"]
    );
    assert_eq!(undo(&mut app)["project"], original["project"]);
    let trimmed = drag(&mut app, A, -1, 0.25, false);
    for (n, start, length) in [(A, 1.125, 0.125), (B, 2.125, 0.625)] {
        near(&note(&trimmed, id, n)["start"], start);
        near(&note(&trimmed, id, n)["length"], length);
        assert_eq!(
            note(&trimmed, id, n)["midi2"]["velocity"],
            note(&original, id, n)["midi2"]["velocity"]
        );
    }
    let n = note(&trimmed, id, A);
    let points = n["midi2"]["expressions"].as_array().unwrap();
    assert_eq!(points.len(), 4);
    for (point, beat, old_index) in [(0, 0.0, 1), (1, 0.075, 2), (2, 0.275, 3), (3, 0.675, 4)] {
        near(&points[point]["beat"], beat);
        assert_eq!(
            points[point]["packet"],
            note(&original, id, A)["midi2"]["expressions"][old_index]["packet"]
        );
    }
    unchanged_others(&trimmed, &original, id);
    assert_eq!(notes(&trimmed, id), notes(&trimmed, ghost));
    assert_eq!(history(&trimmed), history(&original) + 1);
    assert_eq!(undo(&mut app)["project"], original["project"]);
    // Zoom out enough to pull beyond source beat zero while keeping the real
    // pointer within the application viewport.
    for _ in 0..4 {
        app.command(json!({"command":"click","label":"−","occurrence":1}));
    }
    let extended = drag(&mut app, A, -1, -2.25, false);
    for (n, start, length) in [(A, 0.0, 1.25), (B, 1.0, 1.75)] {
        near(&note(&extended, id, n)["start"], start);
        near(&note(&extended, id, n)["length"], length);
    }
    for (new, old) in note(&extended, id, A)["midi2"]["expressions"]
        .as_array()
        .unwrap()
        .iter()
        .zip(
            note(&original, id, A)["midi2"]["expressions"]
                .as_array()
                .unwrap(),
        )
    {
        near(&new["beat"], old["beat"].as_f64().unwrap() + 1.0);
        assert_eq!(new["id"], old["id"]);
        assert_eq!(new["packet"], old["packet"]);
    }
    assert_eq!(history(&extended), history(&original) + 1);
    assert_eq!(undo(&mut app)["project"], original["project"]);
    app.quit();
}

#[test]
fn control_drag_copy_move_and_mouse_delete_are_grouped_and_keep_exact_source_performance() {
    let (mut app, id, ghost) = fixture(0.7);
    let original = app.inspect();
    let copied = drag(&mut app, A, 0, 1.0, true);
    assert_eq!(notes(&copied, id).len(), 7);
    for old in notes(&original, id) {
        assert_eq!(note(&copied, id, old["id"].as_u64().unwrap()), old);
    }
    let new: Vec<_> = notes(&copied, id)
        .into_iter()
        .filter(|note| {
            !notes(&original, id)
                .iter()
                .any(|old| old["id"] == note["id"])
        })
        .collect();
    assert_eq!(new.len(), 2);
    for n in &new {
        let old = note(&original, id, if n["pitch"] == 60 { A } else { B });
        near(&n["start"], old["start"].as_f64().unwrap() + 0.7);
        for key in ["length", "pitch", "velocity"] {
            assert_eq!(n[key], old[key]);
        }
        let mut data = n["midi2"].clone();
        let mut original_data = old["midi2"].clone();
        if let Some(points) = data["expressions"].as_array_mut() {
            let old_points = original_data["expressions"].as_array_mut().unwrap();
            for (point, old_point) in points.iter_mut().zip(old_points.iter_mut()) {
                assert_ne!(point["id"], old_point["id"]);
                point["id"] = Value::Null;
                old_point["id"] = Value::Null;
            }
        }
        assert_eq!(data, original_data);
    }
    assert_eq!(notes(&copied, id), notes(&copied, ghost));
    assert_eq!(history(&copied), history(&original) + 1);
    let copied_a = new.iter().find(|n| n["pitch"] == 60).unwrap()["id"]
        .as_u64()
        .unwrap();
    let moved = drag(&mut app, copied_a, 0, 0.5, false);
    for old in &new {
        let n = note(&moved, id, old["id"].as_u64().unwrap());
        near(&n["start"], old["start"].as_f64().unwrap() + 0.35);
        for key in ["length", "pitch", "velocity", "midi2"] {
            assert_eq!(n[key], old[key]);
        }
    }
    assert_eq!(history(&moved), history(&copied) + 1);
    let deleted = menu(&mut app, "Edit", "Delete");
    assert_eq!(notes(&deleted, id), notes(&original, id));
    assert_eq!(notes(&deleted, id), notes(&deleted, ghost));
    assert_eq!(history(&deleted), history(&moved) + 1);
    assert_eq!(undo(&mut app)["project"], moved["project"]);
    assert_eq!(undo(&mut app)["project"], copied["project"]);
    assert_eq!(undo(&mut app)["project"], original["project"]);
    app.quit();
}

#[test]
fn automatic_containment_cleanup_keeps_rich_notes_when_group_moved_into_legacy_notes() {
    let (mut app, id, ghost) = fixture_with_container(0.5, true);
    let original = app.inspect();
    let moved = drag(&mut app, A, 0, 4.0, false);
    assert_eq!(notes(&moved, id).len(), notes(&original, id).len());
    near(&note(&moved, id, A)["start"], 3.0);
    near(&note(&moved, id, B)["start"], 4.0);
    for n in [A, B] {
        for key in ["id", "length", "pitch", "velocity", "midi2"] {
            assert_eq!(note(&moved, id, n)[key], note(&original, id, n)[key]);
        }
    }
    unchanged_others(&moved, &original, id);
    assert_eq!(notes(&moved, id), notes(&moved, ghost));
    assert_eq!(history(&moved), history(&original) + 1);
    assert_eq!(undo(&mut app)["project"], original["project"]);
    assert_eq!(menu(&mut app, "Edit", "Redo")["project"], moved["project"]);
    app.quit();
}
