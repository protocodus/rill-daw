//! Undo travels through actual menus/keys and pointer gestures in the app process.
mod common;
use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}
fn add_track(app: &mut Harness) -> Value {
    app.click("Track");
    app.click("Add audio track")
}

#[test]
fn named_history_jumps_saved_checkpoint_and_branching_work_through_ui() {
    let mut app = Harness::new();
    add_track(&mut app);
    add_track(&mut app);
    let saved = app.directory.path().join("history.rill");
    let state = app.command(json!({"command":"save","path":saved}));
    assert_eq!(state["modified"], false);
    add_track(&mut app);
    let state = add_track(&mut app);
    assert_eq!(state["history"]["undo"].as_array().unwrap().len(), 4);
    app.click("Edit");
    let state = app.click("History…");
    assert_eq!(state["popups"]["history"], true);
    let state = app.command(json!({"command":"click","label":"Undo · Add track","occurrence":1}));
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(
        state["modified"], false,
        "Returning to the saved state clears dirty status"
    );
    assert_eq!(state["history"]["redo"].as_array().unwrap().len(), 2);
    // Keyboard undo/redo also works while a focused utility is open.
    let state = undo(&mut app, true);
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 3);
    assert_eq!(state["modified"], true);
    undo(&mut app, false);
    app.key("Escape");
    let state = add_track(&mut app);
    assert!(state["history"]["redo"].as_array().unwrap().is_empty());
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 3);
    app.click("File");
    let state = app.click("New");
    assert!(state["history"]["undo"].as_array().unwrap().is_empty());
    assert!(state["history"]["redo"].as_array().unwrap().is_empty());
    app.quit();
}

#[test]
fn a_multi_frame_note_resize_is_one_reversible_step_and_navigation_is_not_an_edit() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("History instrument");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "History notes",
        0.0,
        8.0,
        vec![Note::new(60, 1.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("notes.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("History notes");
    app.click("Window");
    app.click("Clip Editor");
    app.key("F");
    let state = app.inspect();
    assert!(state["history"]["undo"].as_array().unwrap().is_empty());
    let note = &state["midi_editor"]["notes"][0]["rect"];
    let x = note["max"][0].as_f64().unwrap() - 2.0;
    let y = (note["min"][1].as_f64().unwrap() + note["max"][1].as_f64().unwrap()) * 0.5;
    let width = state["midi_editor"]["pixels_per_beat"].as_f64().unwrap();
    app.command(json!({"command":"pointer","phase":"down","x":x,"y":y}));
    for delta in [0.25, 0.5, 0.75] {
        app.command(json!({"command":"pointer","phase":"move","x":x+width*delta,"y":y}));
    }
    let changed = app.command(json!({"command":"pointer","phase":"up","x":x+width*0.75,"y":y}));
    assert_eq!(changed["history"]["undo"].as_array().unwrap().len(), 1);
    assert_ne!(changed["project"], state["project"]);
    let restored = undo(&mut app, false);
    assert_eq!(restored["project"], state["project"]);
    assert_eq!(restored["modified"], false);
    let redone = undo(&mut app, true);
    assert_eq!(redone["project"], changed["project"]);
    app.quit();
}

#[test]
fn labelled_icon_tools_remain_clickable_in_arranger_and_note_editor() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Toolbar instrument");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Toolbar notes",
        0.0,
        8.0,
        vec![Note::new(60, 1.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("toolbar.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let original = app.inspect()["project"].clone();
    for label in ["Pencil", "Cut", "Duplicate", "Erase", "Cursor", "Select"] {
        let state = app.click(label);
        assert_eq!(state["timeline"]["tool"], label);
    }
    app.click("Toolbar notes");
    app.click("Window");
    app.click("Clip Editor");
    for (label, tool) in [
        ("Pencil", "Pencil"),
        ("Cut", "Cut"),
        ("Duplicate", "Duplicate"),
        ("Glue", "Glue"),
        ("Erase", "Erase"),
        ("Multi-draw", "MultiDraw"),
        ("Cursor", "Cursor"),
        ("Select", "Select"),
    ] {
        let state = app.inspect();
        let count = state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| *item == label)
            .count();
        let state = app.command(json!({"command":"click","label":label,"occurrence":count-1}));
        assert_eq!(state["midi_editor"]["tool"], tool);
        assert_eq!(
            state["project"], original,
            "Switching tools only changes editing mode"
        );
    }
    app.quit();
}
