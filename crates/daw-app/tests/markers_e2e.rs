//! Timeline markers: created, moved, renamed, navigated and deleted from the
//! arranger by mouse, undoable, saved, exported as MIDI markers and tool-editable.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn fixture() -> Harness {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Keys");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Riff",
        0.0,
        16.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("Markers.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app
}

fn rect(value: &Value) -> [f32; 4] {
    [
        value["min"][0].as_f64().unwrap() as f32,
        value["min"][1].as_f64().unwrap() as f32,
        value["max"][0].as_f64().unwrap() as f32,
        value["max"][1].as_f64().unwrap() as f32,
    ]
}

fn x_at(state: &Value, beat: f64) -> f32 {
    let timeline = &state["timeline"];
    let start = timeline["start_beat"].as_f64().unwrap();
    let scale = timeline["pixels_per_beat"].as_f64().unwrap();
    rect(&timeline["ruler"])[0] + ((beat - start) * scale) as f32
}

fn ruler_point(state: &Value, beat: f64) -> (f32, f32) {
    let [_, top, _, bottom] = rect(&state["timeline"]["ruler"]);
    (x_at(state, beat), (top + bottom) * 0.5)
}

fn pointer(app: &mut Harness, (x, y): (f32, f32), phase: &str, button: &str) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":x,"y":y,"button":button}))
}

fn markers(state: &Value) -> Vec<(u64, f64, String)> {
    state["project"]["markers"]
        .as_array()
        .map(|markers| {
            markers
                .iter()
                .map(|marker| {
                    (
                        marker["id"].as_u64().unwrap(),
                        marker["beat"].as_f64().unwrap(),
                        marker["name"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The left part of a marker flag, where its position line and label begin.
fn flag(state: &Value, id: u64) -> (f32, f32) {
    let flag = state["timeline"]["timeline_flags"]["flags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|flag| flag["kind"] == "marker" && flag["key"] == id)
        .unwrap_or_else(|| {
            panic!(
                "No marker flag {id}: {}",
                state["timeline"]["timeline_flags"]
            )
        });
    let [left, top, _, bottom] = rect(&flag["rect"]);
    (left + 6.0, (top + bottom) * 0.5)
}

fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}

fn markers_menu(app: &mut Harness, item: &str) -> Value {
    app.click("Edit");
    app.click("Markers");
    app.click(item)
}

#[test]
fn mouse_markers_move_rename_navigate_undo_save_and_export() {
    let mut app = fixture();
    let before = app.inspect();
    let first = markers_menu(&mut app, "Add marker at playhead");
    let listed = markers(&first);
    assert_eq!(listed.len(), 1, "{}", first["project"]);
    let intro = listed[0].0;
    assert_eq!(listed[0], (intro, 0.0, "Marker 1".to_owned()));
    assert_eq!(first["history"]["undo"][0], "Add marker");

    pointer(&mut app, ruler_point(&first, 8.0), "click", "secondary");
    let second = app.click("Add marker here");
    let verse = markers(&second)[1].0;
    assert_eq!(markers(&second)[1].1, 8.0);

    // Dragging the flag moves the marker with the arranger's snap.
    let (x, y) = flag(&second, verse);
    let to = x + (x_at(&second, 12.0) - x_at(&second, 8.0));
    let moved =
        app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":to,"to_y":y}));
    assert_eq!(markers(&moved)[1], (verse, 12.0, "Marker 2".to_owned()));
    assert_eq!(moved["history"]["undo"][0], "Move marker");

    // Double-click opens the inline name editor beside the flag.
    pointer(&mut app, flag(&moved, verse), "double_click", "primary");
    app.command(json!({"command":"text","text":"Chorus"}));
    let renamed = app.click("Apply name");
    assert_eq!(markers(&renamed)[1].2, "Chorus");
    assert_eq!(renamed["history"]["undo"][0], "Rename marker");

    // Clicking a flag moves the playhead; menus and keys step between markers.
    let at_intro = pointer(&mut app, flag(&renamed, intro), "click", "primary");
    assert_eq!(at_intro["timeline"]["cursor"], 0.0);
    let next = markers_menu(&mut app, "Next marker");
    assert_eq!(next["timeline"]["cursor"], 12.0);
    let previous = app.key("Comma");
    assert_eq!(previous["timeline"]["cursor"], 0.0);
    assert_eq!(app.key("Period")["timeline"]["cursor"], 12.0);
    let added = app.key("M");
    assert_eq!(markers(&added).len(), 3, "M adds a marker at the playhead");
    let added = menu(&mut app, "Edit", "Undo");
    assert_eq!(added["project"], renamed["project"]);

    // The flag context menu deletes; Undo/Redo restore the exact song.
    pointer(&mut app, flag(&renamed, verse), "click", "secondary");
    let deleted = app.click("Delete marker");
    assert_eq!(markers(&deleted).len(), 1);
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        renamed["project"]
    );
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        deleted["project"]
    );
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        renamed["project"]
    );
    for _ in 0..4 {
        menu(&mut app, "Edit", "Undo");
    }
    assert_eq!(app.inspect()["project"], before["project"]);
    for _ in 0..4 {
        menu(&mut app, "Edit", "Redo");
    }
    assert_eq!(app.inspect()["project"], renamed["project"]);

    let midi = app.directory.path().join("Markers.mid");
    app.command(json!({"command":"file_dialog","kind":"export_midi","path":midi}));
    let exported = menu(&mut app, "File", "Export MIDI…");
    assert_eq!(exported["error"], false, "{}", exported["status"]);
    let bytes = std::fs::read(&midi).unwrap();
    for name in ["Marker 1", "Chorus"] {
        let mut event = vec![0xff, 0x06, name.len() as u8];
        event.extend(name.as_bytes());
        assert!(
            bytes.windows(event.len()).any(|window| window == event),
            "MIDI file lacks marker {name}"
        );
    }

    let bundle = app.directory.path().join("Marked song.rill");
    app.command(json!({"command":"file_dialog","kind":"save","path":bundle}));
    menu(&mut app, "File", "Save as…");
    assert_eq!(
        app.command(json!({"command":"wait_project"}))["modified"],
        false
    );
    app = app.restart();
    app.command(json!({"command":"file_dialog","kind":"open","path":bundle}));
    menu(&mut app, "File", "Open…");
    let reopened = app.command(json!({"command":"wait_project"}));
    assert_eq!(markers(&reopened), markers(&renamed));
    flag(&reopened, verse);
    app.quit();
}

#[test]
fn song_tools_list_and_edit_markers() {
    let mut app = fixture();
    let context =
        app.command(json!({"command":"song_tool","name":"get_song_context","arguments":{}}));
    let result = &context["tool_result"]["result"];
    let (session, revision) = (result["session_id"].clone(), result["revision"].clone());
    let created = app.command(
        json!({"command":"song_tool","name":"apply_edits","arguments":{
            "session_id":session,"expected_revision":revision,"label":"Song form",
            "edits":[
                {"op":"add_marker","beat":4.0,"name":"Verse","color":[220,120,40],"key":"verse"},
                {"op":"add_marker","beat":12.0,"name":"Hook","key":"hook"},
                {"op":"move_marker","marker":"hook","beat":10.0}
            ]
        }}),
    );
    assert_eq!(
        created["tool_result"]["ok"], true,
        "{}",
        created["tool_result"]
    );
    let ids: Vec<_> = created["tool_result"]["result"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entity| entity["id"].as_u64().unwrap())
        .collect();
    assert_eq!(
        markers(&created),
        [
            (ids[0], 4.0, "Verse".to_owned()),
            (ids[1], 10.0, "Hook".to_owned())
        ]
    );
    assert_eq!(
        created["project"]["markers"][0]["color"],
        json!([220, 120, 40])
    );
    assert_eq!(created["history"]["undo"][0], "Song form");
    flag(&created, ids[0]);
    let context =
        app.command(json!({"command":"song_tool","name":"get_song_context","arguments":{}}));
    let listed = &context["tool_result"]["result"];
    assert_eq!(listed["marker_count"], 2);
    assert_eq!(listed["markers"][1]["name"], "Hook");
    let removed = app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":session,"expected_revision":listed["revision"],"label":"Drop hook",
        "edits":[{"op":"remove_marker","marker":ids[1]},{"op":"rename_marker","marker":ids[0],"name":"Intro"}]
    }}));
    assert_eq!(markers(&removed), [(ids[0], 4.0, "Intro".to_owned())]);
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], created["project"]);
    app.quit();
}
