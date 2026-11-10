//! Meter changes at bar lines: inserted and edited from the arranger ruler by
//! mouse, undoable, saved, exported as MIDI time signatures and reachable by tools.
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
        8.0,
        8.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("Meters.rill");
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

/// The ruler point over a beat, from the arranger's exact view mapping.
fn ruler_point(state: &Value, beat: f64) -> (f32, f32) {
    let timeline = &state["timeline"];
    let [left, top, _, bottom] = rect(&timeline["ruler"]);
    let start = timeline["start_beat"].as_f64().unwrap();
    let scale = timeline["pixels_per_beat"].as_f64().unwrap();
    (left + ((beat - start) * scale) as f32, (top + bottom) * 0.5)
}

fn pointer(app: &mut Harness, (x, y): (f32, f32), button: &str) -> Value {
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y,"button":button}))
}

fn meter_flag(state: &Value, bar: u64) -> (f32, f32) {
    let flag = state["timeline"]["timeline_flags"]["flags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|flag| flag["kind"] == "meter" && flag["key"] == bar)
        .unwrap_or_else(|| panic!("No meter flag for bar {bar}: {}", state["timeline"]));
    let [left, top, right, bottom] = rect(&flag["rect"]);
    ((left + right) * 0.5, (top + bottom) * 0.5)
}

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}

#[test]
fn ruler_meter_changes_are_undoable_saved_and_exported() {
    let mut app = fixture();
    let before = app.inspect();
    // Beat 9 lies inside bar 3 of the 4/4 song.
    pointer(&mut app, ruler_point(&before, 9.0), "secondary");
    let dialog = app.click("Insert meter change…");
    assert!(has(&dialog, "Insert meter change") && has(&dialog, "Apply meter change"));
    assert_eq!(
        dialog["project"], before["project"],
        "Opening the dialog edits nothing"
    );
    menu(&mut app, "4/4", "3/4");
    let applied = app.click("Apply meter change");
    assert_eq!(
        applied["project"]["meter_changes"],
        json!([{"bar":3,"beats_per_bar":3}])
    );
    assert_eq!(applied["project"]["beats_per_bar"], 4);
    assert_eq!(
        applied["project"]["tracks"], before["project"]["tracks"],
        "Clips keep their beat positions"
    );
    assert_eq!(applied["history"]["undo"][0], "Meter change");
    assert!(
        !has(&applied, "Apply meter change"),
        "Apply closes the dialog"
    );
    // The flag sits on the bar 3 downbeat (beat 8).
    let flag = meter_flag(&applied, 3);
    assert!((flag.0 - ruler_point(&applied, 8.0).0).abs() < 40.0);

    // Clicking the flag edits it; one step moves it to bar 4.
    let edit = pointer(&mut app, flag, "primary");
    assert!(has(&edit, "Edit meter change") && has(&edit, "Remove meter change"));
    app.click("+");
    let moved = app.click("Apply meter change");
    assert_eq!(
        moved["project"]["meter_changes"],
        json!([{"bar":4,"beats_per_bar":3}])
    );
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(
        undone["project"]["meter_changes"],
        json!([{"bar":3,"beats_per_bar":3}])
    );
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    menu(&mut app, "Edit", "Redo");
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], moved["project"]);

    // The flag's context menu removes it as one undoable edit.
    pointer(&mut app, meter_flag(&redone, 4), "secondary");
    let removed = app.click("Remove meter change");
    assert!(removed["project"].get("meter_changes").is_none());
    let restored = menu(&mut app, "Edit", "Undo");
    assert_eq!(restored["project"], moved["project"]);

    // The transport chooser still edits only the meter of bar 1.
    let initial = menu(&mut app, "4/4", "2/4");
    assert_eq!(initial["project"]["beats_per_bar"], 2);
    assert_eq!(
        initial["project"]["meter_changes"],
        json!([{"bar":4,"beats_per_bar":3}])
    );
    menu(&mut app, "Edit", "Undo");

    let midi = app.directory.path().join("Meters.mid");
    app.command(json!({"command":"file_dialog","kind":"export_midi","path":midi}));
    let exported = menu(&mut app, "File", "Export MIDI…");
    assert_eq!(exported["error"], false, "{}", exported["status"]);
    let imported = daw_core::import_midi(&midi).unwrap();
    assert_eq!(imported.beats_per_bar, Some(4));
    assert_eq!(
        serde_json::to_value(&imported.meter_changes).unwrap(),
        json!([{"bar":4,"beats_per_bar":3}])
    );

    let bundle = app.directory.path().join("Changing meter.rill");
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
    assert_eq!(
        reopened["project"]["meter_changes"],
        json!([{"bar":4,"beats_per_bar":3}])
    );
    meter_flag(&reopened, 4);
    app.quit();
}

#[test]
fn song_tools_read_and_edit_meter_changes_through_shared_history() {
    let mut app = fixture();
    let context =
        app.command(json!({"command":"song_tool","name":"get_song_context","arguments":{}}));
    let result = &context["tool_result"]["result"];
    assert_eq!(result["meter_changes"], json!([]));
    let (session, revision) = (result["session_id"].clone(), result["revision"].clone());
    let edited = app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":session,"expected_revision":revision,"label":"Odd bridge",
        "edits":[{"op":"set_meter_change","bar":5,"beats_per_bar":7},{"op":"set_meter_change","bar":7,"beats_per_bar":4}]
    }}));
    assert_eq!(
        edited["tool_result"]["ok"], true,
        "{}",
        edited["tool_result"]
    );
    assert_eq!(
        edited["project"]["meter_changes"],
        json!([{"bar":5,"beats_per_bar":7},{"bar":7,"beats_per_bar":4}])
    );
    // Bars 1–4 in 4/4, bars 5–6 in 7/4: bar 7 starts at beat 30.
    meter_flag(&edited, 5);
    let context =
        app.command(json!({"command":"song_tool","name":"get_song_context","arguments":{}}));
    assert_eq!(
        context["tool_result"]["result"]["meter_changes"][1]["start_beat"],
        30.0
    );
    let rejected = app.command(
        json!({"command":"song_tool","name":"apply_edits","arguments":{
            "session_id":session,"expected_revision":context["tool_result"]["result"]["revision"],
            "label":"Invalid","edits":[{"op":"remove_meter_change","bar":6}]
        }}),
    );
    assert_eq!(rejected["tool_result"]["ok"], false);
    assert_eq!(rejected["project"], edited["project"]);
    let undone = menu(&mut app, "Edit", "Undo");
    assert!(undone["project"].get("meter_changes").is_none());
    app.quit();
}
