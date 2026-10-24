//! Exercise the production executable through its egui menu and popup controls.

mod common;

use common::Harness;
use serde_json::{Value, json};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

/// A menu can repeat text already visible in the arrangement or another window.
/// The last painted matching label belongs to the foreground popup.
fn click_front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == label)
        .count();
    assert!(count > 0, "Missing visible {label:?}: {state}");
    app.command(json!({"command": "click", "label": label, "occurrence": count - 1}))
}

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    click_front(app, item)
}

fn assert_only_popup(state: &Value, popup: &str) {
    for (name, open) in state["popups"].as_object().unwrap() {
        assert_eq!(open, name == popup, "Unexpected popup {name}: {state}");
    }
    assert_eq!(state["view"], "Arrangement");
    assert_eq!(state["device_open"], false);
}

#[test]
fn track_menu_creates_each_track_type_and_edit_menu_undoes_and_redoes_creation() {
    let mut app = Harness::new();
    for (index, (item, kind)) in [
        ("Add MIDI track", "Midi"),
        ("Add audio track", "Audio"),
        ("Add bus", "Bus"),
        ("Add folder", "Folder"),
    ]
    .into_iter()
    .enumerate()
    {
        let state = menu(&mut app, "Track", item);
        assert_eq!(
            state["project"]["tracks"].as_array().unwrap().len(),
            index + 1
        );
        assert_eq!(state["project"]["tracks"][index]["kind"], kind);
        assert_eq!(
            state["selected_track"],
            state["project"]["tracks"][index]["id"]
        );
    }
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 3);
    let state = menu(&mut app, "Edit", "Redo");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 4);
    assert_eq!(state["project"]["tracks"][3]["kind"], "Folder");
    assert_eq!(state["error"], false);
    app.quit();
}

#[test]
fn instrument_and_library_selection_are_undoable_and_escape_closes_the_popup() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add MIDI track");
    let state = menu(&mut app, "Window", "Instrument");
    assert_only_popup(&state, "instrument");
    assert!(visible(&state, "Instrument · New instrument"));
    let state = app.click("Saw");
    assert_eq!(state["project"]["tracks"][0]["instrument"], "Saw");
    let state = app.key("Escape");
    assert_only_popup(&state, "none");
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"]["tracks"][0]["instrument"], "Triangle");
    let state = menu(&mut app, "Edit", "Redo");
    assert_eq!(state["project"]["tracks"][0]["instrument"], "Saw");

    let state = menu(&mut app, "Window", "Library");
    assert_only_popup(&state, "library");
    let state = app.click("Sine");
    assert_eq!(state["project"]["tracks"][0]["instrument"], "Sine");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_only_popup(&state, "none");
    app.quit();
}

#[test]
fn effects_routing_and_mixer_controls_update_the_same_channel_graph() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add MIDI track");
    let state = menu(&mut app, "Track", "Add bus");
    let bus = state["project"]["tracks"][1]["id"].clone();
    app.click("New instrument");
    let state = menu(&mut app, "Window", "Effects");
    assert_only_popup(&state, "effects");
    menu(&mut app, "+ Insert", "Gain");
    let state = menu(&mut app, "+ Insert", "Stereo delay");
    assert_eq!(
        state["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let state = app.click("01  Gain");
    assert_eq!(
        state["project"]["tracks"][0]["inserts"][0]["enabled"],
        false
    );
    let state = app.command(json!({"command": "click", "label": "Up", "occurrence": 1}));
    assert_eq!(
        state["project"]["tracks"][0]["inserts"][0]["name"],
        "Stereo delay"
    );
    assert_eq!(state["project"]["tracks"][0]["inserts"][1]["name"], "Gain");
    let state = app.click("×");
    assert_eq!(
        state["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(
        state["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let state = menu(&mut app, "Window", "Routing");
    assert_only_popup(&state, "routing");
    menu(&mut app, "+ Send", "New bus");
    let state = app.click("Pre");
    assert_eq!(state["project"]["tracks"][0]["sends"][0]["target_bus"], bus);
    assert_eq!(state["project"]["tracks"][0]["sends"][0]["pre_fader"], true);
    app.click("Master output");
    let state = click_front(&mut app, "New bus");
    assert_eq!(state["project"]["tracks"][0]["output_bus"], bus);
    assert_eq!(state["error"], false);

    let state = menu(&mut app, "Window", "Mixer");
    assert_eq!(state["view"], "Mixer");
    assert!(
        state["popups"]
            .as_object()
            .unwrap()
            .values()
            .all(|open| open == false)
    );
    let state = app.click("Mute");
    assert_eq!(state["project"]["tracks"][0]["mute"], true);
    assert_eq!(state["project"]["tracks"][1]["mute"], false);
    let state = app.click("Solo");
    assert_eq!(state["project"]["tracks"][0]["solo"], true);
    app.quit();
}

#[test]
fn properties_assign_folder_membership_and_compact_tracks_without_losing_channels() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add MIDI track");
    let state = menu(&mut app, "Track", "Add folder");
    let folder = state["project"]["tracks"][1]["id"].clone();
    app.click("New instrument");
    let state = menu(&mut app, "Window", "Properties");
    assert_only_popup(&state, "properties");
    app.click("Top level");
    let state = click_front(&mut app, "New folder");
    assert_eq!(state["project"]["tracks"][0]["parent_folder"], folder);
    let state = app.click("Compact track");
    assert_eq!(state["project"]["tracks"][0]["collapsed"], true);
    app.key("Escape");
    let state = app.click("▾ New folder");
    assert_eq!(state["project"]["tracks"][1]["collapsed"], true, "{state}");
    assert!(!visible(&state, "New instrument"));
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 2);
    let state = app.click("▸ New folder");
    assert!(visible(&state, "New instrument"));
    assert_eq!(state["project"]["tracks"][1]["collapsed"], false);
    app.quit();
}

#[test]
fn setup_cancel_preserves_active_settings_and_apply_persists_in_the_isolated_config() {
    let mut app = Harness::new();
    let initial = app.inspect()["settings"].clone();
    menu(&mut app, "Window", "Setup");
    app.click("256 samples");
    let state = click_front(&mut app, "512 samples");
    assert_eq!(state["setup_draft"]["buffer_frames"], 512);
    assert_eq!(state["settings"], initial);
    let state = app.click("Cancel");
    assert_only_popup(&state, "none");
    assert_eq!(state["settings"], initial);
    let config = app.directory.path().join("config/settings.json");
    assert!(!config.exists(), "Cancel must not create a settings file");

    menu(&mut app, "Window", "Setup");
    app.click("256 samples");
    click_front(&mut app, "512 samples");
    let state = app.click("Apply");
    assert_only_popup(&state, "none");
    assert_eq!(state["settings"]["buffer_frames"], 512);
    let saved: Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
    assert_eq!(saved, state["settings"]);
    assert_eq!(state["error"], false);
    app.quit();
}
