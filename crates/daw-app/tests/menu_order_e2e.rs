//! Menus follow the editing workflow, show shortcuts in a muted column, and
//! the menu bar ends with the project identity and the AI button.
mod common;

use common::Harness;
use daw_core::{Clip, Project};
use serde_json::{Value, json};

fn menu_label(state: &Value, name: &str) -> [f64; 4] {
    let rect = state["menu_label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == name)
        .unwrap_or_else(|| panic!("Missing menu item {name}: {}", state["menu_label_rects"]));
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn assert_order(state: &Value, labels: &[&str]) {
    let ys: Vec<f64> = labels
        .iter()
        .map(|label| menu_label(state, label)[1])
        .collect();
    for (pair, y) in labels.windows(2).zip(ys.windows(2)) {
        assert!(y[0] < y[1], "{} must precede {}: {ys:?}", pair[0], pair[1]);
    }
}

fn label(state: &Value, name: &str) -> [f64; 4] {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| {
            rect["text"]
                .as_str()
                .is_some_and(|text| text.starts_with(name))
        })
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]));
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

#[test]
fn edit_track_and_window_menus_follow_workflow_with_a_shortcut_column() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Menu synth");
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("Menu phrase", 0.0, 4.0, vec![]));
    let path = app.directory.path().join("menus.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1600,"height":1200}));
    app.click("Menu phrase");

    app.click("Edit");
    let edit = app.inspect();
    assert_order(
        &edit,
        &[
            "Undo",
            "Redo",
            "History…",
            "Cut",
            "Copy",
            "Paste",
            "Duplicate",
            "Delete",
            "Select all clips",
            "Rename…",
            "Split at cursor",
            "Join clips…",
            "Stretch",
            "Ghost copy",
            "Repeat to loop end",
            "Audio",
        ],
    );
    // The shortcut column is right-aligned and shares each item's row.
    let undo = menu_label(&edit, "Undo");
    // Shortcuts render in platform notation (⌘Z on macOS).
    let shortcut = menu_label(&edit, &daw_ui::shortcut_text("Mod+Z"));
    assert!(shortcut[0] > undo[2] + 20.0, "{undo:?} {shortcut:?}");
    assert!(((shortcut[1] + shortcut[3]) - (undo[1] + undo[3])).abs() < 2.0);
    let copy = menu_label(&edit, &daw_ui::shortcut_text("Mod+C"));
    assert!(
        (copy[2] - shortcut[2]).abs() < 1.0,
        "shortcuts share a right edge"
    );
    app.click("Audio");
    let audio = app.inspect();
    assert_order(
        &audio,
        &["Crossfade…", "Smooth edit boundaries", "Boundary fades…"],
    );
    app.key("Escape");
    app.key("Escape");

    app.click("Track");
    let track_menu = app.inspect();
    assert_order(
        &track_menu,
        &[
            "Add instrument track…",
            "Add audio track",
            "Add bus",
            "Add folder",
            "Add automation track…",
            "Rename…",
            "Properties",
            "Channel",
            "Delete track",
        ],
    );
    let deleted = app.click("Delete track");
    assert_eq!(deleted["project"]["tracks"].as_array().unwrap().len(), 0);
    app.click("Edit");
    app.click("Undo");

    app.click("Window");
    let window = app.inspect();
    assert_order(
        &window,
        &[
            "Arranger",
            "Clip Editor",
            "Mixer",
            "Instrument",
            "Effects",
            "Routing",
            "Recording",
            "Automation",
            "Properties",
            "Library",
            "Assistant",
            "Setup",
        ],
    );
    let library = app.click("Library");
    assert_eq!(library["popups"]["library"], true);
    label(&library, "Library");
    app.key("Escape");

    // Menu bar: project name and duration centered, AI at the far right.
    let state = app.inspect();
    let title = label(&state, "menus.daw");
    let ai = label(&state, "AI");
    let help = label(&state, "Help");
    assert!(
        title[0] > help[2] && ai[0] > title[2],
        "{help:?} {title:?} {ai:?}"
    );
    let text = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| {
            rect["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("menus.daw"))
        })
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.contains(" · 0:02"), "{text}");
    app.click("File");
    let file = app.inspect();
    assert_order(
        &file,
        &["New", "Open…", "Recover last session…", "Save", "Save as…"],
    );
    app.quit();
}
