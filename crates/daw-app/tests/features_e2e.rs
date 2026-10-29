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
    if menu == "+ Insert" {
        let state = app.inspect();
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Search effects")
        {
            app.click("Search effects");
            app.command(json!({"command":"text","text":"Rill Fixture"}));
        }
    }
    if menu == "+ Insert" {
        common::wait_library_device(app, item);
    }
    let state = click_front(app, item);
    if menu == "+ Insert" {
        app.click("Add to effect chain")
    } else {
        state
    }
}

fn show_plugin_parameters(app: &mut Harness) {
    if !visible(&app.inspect(), "Search parameters") {
        common::wait_visible(app, "Automate…");
        app.click("Automate…");
    }
    common::wait_visible(app, "Automate parameter");
}

fn assert_only_popup(state: &Value, popup: &str) {
    for (name, open) in state["popups"].as_object().unwrap() {
        assert_eq!(open, name == popup, "Unexpected popup {name}: {state}");
    }
    assert_eq!(state["view"], "Arrangement");
    assert_eq!(state["device_open"], false);
}

#[test]
fn nested_utilities_return_to_the_selected_notes_and_scroll_position() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add MIDI track");
    app.key("Escape");
    menu(&mut app, "Window", "Clip Editor");
    click_front(&mut app, "Pencil");
    let state = app.inspect();
    let key = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == "C4")
        .unwrap();
    let x = key["max"][0].as_f64().unwrap() + 28.0;
    let y = (key["min"][1].as_f64().unwrap() + key["max"][1].as_f64().unwrap()) * 0.5;
    let state = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(
        state["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|note| note["selected"] == true)
            .count(),
        1
    );
    app.command(json!({"command":"scroll","x":x+200.0,"y":y,"delta":-72.0}));
    for _ in 0..12 {
        app.inspect();
    }
    let before = app.inspect();
    let key_before = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == "C4")
        .unwrap()
        .clone();

    // Properties is opened from the editor, then Routing is nested above it.
    assert_only_popup(&app.click("Properties"), "properties");
    assert_only_popup(&menu(&mut app, "Window", "Routing"), "routing");
    assert_only_popup(&app.key("Escape"), "properties");
    let restored = app.key("Escape");
    assert_eq!(restored["view"], "Clip");
    assert_eq!(restored["selected_track"], before["selected_track"]);
    assert_eq!(restored["selected_clip"], before["selected_clip"]);
    assert_eq!(restored["project"], before["project"]);
    assert_eq!(
        restored["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|note| note["selected"] == true)
            .count(),
        1
    );
    let key_after = restored["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == "C4")
        .unwrap();
    assert_eq!(
        key_after, &key_before,
        "Returning must preserve piano-roll scroll"
    );

    menu(&mut app, "Window", "Setup");
    let canceled = app.click("Cancel");
    assert_eq!(canceled["view"], "Clip");
    assert_eq!(
        canceled["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|note| note["selected"] == true)
            .count(),
        1
    );
    app.click("Properties");
    assert_only_popup(&menu(&mut app, "Window", "Arrangement"), "none");
    assert_only_popup(&app.key("Escape"), "none");
    let reopened = menu(&mut app, "Window", "Clip Editor");
    assert!(
        reopened["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|note| note["selected"] == false),
        "Explicit Arrangement clears note selection"
    );
    app.quit();
}

#[test]
fn escape_closes_dropdown_and_menu_before_leaving_editor_and_new_clears_return_paths() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add MIDI track");
    app.key("Escape");
    menu(&mut app, "Window", "Clip Editor");
    app.click("C · Natural minor");
    let open = app.click("Natural minor");
    assert!(visible(&open, "Dorian"));
    let closed = app.key("Escape");
    assert_eq!(closed["view"], "Clip");
    assert!(!visible(&closed, "Dorian"));
    // Finish the local Pitch menu before opening a different menu bar action.
    if visible(&closed, "Major triad") {
        assert_eq!(app.key("Escape")["view"], "Clip");
    }
    let menu_open = app.click("Edit");
    assert!(visible(&menu_open, "History…"));
    let menu_closed = app.key("Escape");
    assert_eq!(menu_closed["view"], "Clip");
    assert!(!visible(&menu_closed, "History…"));

    app.click("Properties");
    menu(&mut app, "File", "New");
    let cleared = app.click("Don't save");
    assert_only_popup(&cleared, "none");
    assert!(cleared["project"]["tracks"].as_array().unwrap().is_empty());
    let escaped = app.key("Escape");
    assert_only_popup(&escaped, "none");
    assert!(!visible(&escaped, "Clip Editor"));
    app.quit();
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
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Track", "Add MIDI track");
    let state = menu(&mut app, "Window", "Instrument");
    assert_only_popup(&state, "instrument");
    assert!(visible(&state, "No instrument assigned"));
    app.key("Escape");
    common::fixture_library(&mut app);
    app.click("Rill Fixture A");
    let state = app.click("Add instrument track");
    assert_eq!(
        state["project"]["tracks"].as_array().unwrap().len(),
        2,
        "Add always creates its own MIDI track"
    );
    assert_eq!(
        state["project"]["tracks"][1]["instrument"]["Vst3"]["class_id"],
        "11111111111111111111111111111111"
    );
    common::fixture_library(&mut app);
    app.click("Rill Fixture B");
    app.click("More");
    let state = app.click("Assign to selected track");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(
        state["project"]["tracks"][1]["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(
        state["project"]["tracks"][1]["instrument"]["Vst3"]["class_id"],
        "11111111111111111111111111111111"
    );
    let state = menu(&mut app, "Edit", "Redo");
    assert_eq!(
        state["project"]["tracks"][1]["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    app.quit();
}

#[test]
fn effects_routing_and_mixer_controls_update_the_same_channel_graph() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    common::fixture_library(&mut app);
    app.key("Escape");
    menu(&mut app, "Track", "Add MIDI track");
    let state = menu(&mut app, "Track", "Add bus");
    let bus = state["project"]["tracks"][1]["id"].clone();
    app.click("New instrument");
    let state = menu(&mut app, "Window", "Effects");
    assert_only_popup(&state, "effects");
    menu(&mut app, "+ Insert", "Rill Fixture FX");
    let state = menu(&mut app, "+ Insert", "Rill Fixture FX");
    assert_eq!(
        state["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let state = app.click("01  Rill Fixture FX");
    assert_eq!(
        state["project"]["tracks"][0]["inserts"][0]["enabled"],
        false
    );
    app.command(json!({"command": "click", "label": "More", "occurrence": 1}));
    let state = app.click("Up");
    assert_eq!(
        state["project"]["tracks"][0]["inserts"][0]["name"],
        "Rill Fixture FX"
    );
    assert_eq!(
        state["project"]["tracks"][0]["inserts"][1]["name"],
        "Rill Fixture FX"
    );
    app.click("More");
    let state = app.click("Remove insert");
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
    let state = app.click("New folder");
    assert_eq!(state["project"]["tracks"][1]["collapsed"], true, "{state}");
    assert!(!visible(&state, "New instrument"));
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 2);
    let state = app.click("New folder");
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

#[test]
fn custom_plugin_folders_scan_search_and_preserve_multiclass_metadata() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::new();
    menu(&mut app, "Window", "Setup");
    app.click("Plug-ins");
    app.click("Folder path");
    app.command(json!({"command":"text","text":fixture.path()}));
    let state = app.click("Add folder");
    assert_eq!(state["settings"]["vst_dirs"], json!([]));
    assert_eq!(state["setup_draft"]["vst_dirs"], json!([fixture.path()]));
    app.click("Apply");
    common::fixture_library(&mut app);
    let state = app.inspect();
    assert!(visible(&state, "Rill Fixture A"));
    assert!(visible(&state, "Rill Fixture B"));
    assert!(!visible(&state, "Rill Fixture FX"));
    let classes = state["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["name"].as_str().unwrap().starts_with("Rill Fixture"))
        .map(|entry| entry["class_id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(classes.len(), 3);
    assert_ne!(classes[0], classes[1]);
    let saved: Value = serde_json::from_slice(
        &std::fs::read(app.directory.path().join("config/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["vst_dirs"], json!([fixture.path()]));
    app.quit();
}

/// Native editor behavior is tested only with an explicitly installed plug-in.
/// The regular CI catalog fixtures deliberately contain no executable code.
#[test]
#[ignore = "requires an installed instrument and desktop via RILL_TEST_VST_INSTRUMENT"]
fn installed_instrument_editor_thumbnail_parameter_automation_save_export_and_quit() {
    use daw_core::{AudioData, Project};
    use std::time::{Duration, Instant};
    let path = std::path::PathBuf::from(
        std::env::var_os("RILL_TEST_VST_INSTRUMENT")
            .expect("Set RILL_TEST_VST_INSTRUMENT to an installed VST3 instrument"),
    );
    let name = path.file_stem().unwrap().to_string_lossy().to_string();
    let mut app = Harness::with_vst_path(path.parent().unwrap());
    menu(&mut app, "Window", "Library");
    app.click("Search instruments");
    app.command(json!({"command":"text","text":name.chars().take(4).collect::<String>()}));
    common::wait_visible(&mut app, &name);
    click_front(&mut app, &name);
    let state = app.click("Add instrument track");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(state["project"]["tracks"][0]["kind"], "Midi");

    assert_eq!(state["popups"]["instrument"], true);
    assert_eq!(state["project"]["tracks"][0]["clips"][0]["length"], 4.0);
    show_plugin_parameters(&mut app);
    assert_eq!(
        app.inspect()["native_editor"]["open"],
        true,
        "Adding must open the native editor automatically"
    );
    let state = menu(&mut app, "Window", "Clip Editor");
    assert_eq!(state["view"], "Clip", "{state}");
    click_front(&mut app, "Pencil");
    let state = app.inspect();
    let key = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["text"] == "C4")
        .unwrap();
    let x = key["max"][0].as_f64().unwrap() + 28.0;
    let y = (key["min"][1].as_f64().unwrap() + key["max"][1].as_f64().unwrap()) * 0.5;
    let state = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(
        state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    menu(&mut app, "Window", "Instrument");
    show_plugin_parameters(&mut app);
    let state = app.inspect();
    assert_eq!(state["native_editor"]["open"], true, "{state}");
    assert!(state["native_editor"]["parameter_count"].as_u64().unwrap() > 0);

    // Surge's output level is a useful audible parameter; any other instrument
    // can opt into this test by exposing the same conventional parameter name.
    app.click("Search parameters");
    app.command(json!({"command":"text","text":"Global Vol"}));
    common::wait_visible(&mut app, "Global Volume");
    app.click("Global Volume");
    let state = app.inspect();
    let rect = &state["instrument_parameter_rect"];
    let x = rect["min"][0].as_f64().unwrap() + 75.0;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = app.inspect();
        if state["native_editor"]["last_touched"] == state["native_editor"]["selected"]
            && !state["native_editor"]["last_touched"].is_null()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native parameter gesture must arrive"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let state = app.inspect();
    let parameter = state["native_editor"]["last_touched"].as_u64().unwrap();
    let value = state["project"]["tracks"][0]["instrument_parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == parameter)
        .unwrap()["value"]
        .as_f64()
        .unwrap();
    let edited_parameters = state["project"]["tracks"][0]["instrument_parameters"].clone();
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["native_editor"]["open"], false);
    assert_ne!(
        undone["project"]["tracks"][0]["instrument_parameters"],
        edited_parameters
    );
    for _ in 0..5 {
        app.inspect();
    }
    assert_eq!(
        app.inspect()["project"]["tracks"][0]["instrument_parameters"],
        undone["project"]["tracks"][0]["instrument_parameters"],
        "Discarded editor must not overwrite Undo"
    );
    menu(&mut app, "Edit", "Redo");
    app.click("Open editor");
    show_plugin_parameters(&mut app);
    app.click("Search parameters");
    app.command(json!({"command":"text","text":"Global Vol"}));
    app.click("Global Volume");
    // Create through the selected-parameter action after validating last-touched
    // above; both actions resolve to the same exact native parameter ID.
    let state = app.click("Automate parameter");
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["target"]["parameter"]["VstParameter"]["id"],
        parameter
    );
    assert!(
        state["project"]["tracks"][1]["name"]
            .as_str()
            .unwrap()
            .ends_with("Global Volume")
    );

    // Save flushes the open editor, consolidates overrides into the opaque state,
    // and leaves its separate native window alive.
    let saved = app.directory.path().join("native-session.daw.json");
    let state = app.command(json!({"command":"save","path":saved}));
    assert_eq!(state["native_editor"]["open"], true);
    assert!(
        state["project"]["tracks"][0]["instrument_state"]
            .as_array()
            .unwrap()
            .len()
            > 100
    );
    assert_eq!(
        state["project"]["tracks"][0]["instrument_parameters"],
        json!([])
    );
    let project = Project::load(&saved).unwrap();
    let mut restored = daw_plugins::VstInstrument::load(&path, 48_000.0, 256).unwrap();
    restored
        .load_state(&project.tracks[0].instrument_state)
        .unwrap();
    let actual = restored
        .parameters()
        .unwrap()
        .iter()
        .find(|p| p.id as u64 == parameter)
        .unwrap()
        .value;
    assert!(
        (actual - value).abs() < 1e-5,
        "native state lost the edited parameter"
    );
    drop(restored);

    menu(&mut app, "Track", "Add instrument…");
    app.command(json!({"command":"text","text":name.chars().take(4).collect::<String>()}));
    common::wait_visible(&mut app, &name);
    let state = app.key("Enter");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 3);
    assert_eq!(state["project"]["tracks"][2]["kind"], "Midi");
    show_plugin_parameters(&mut app);
    menu(&mut app, "Window", "Clip Editor");
    click_front(&mut app, "Pencil");
    let state = app.inspect();
    let key = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["text"] == "C4")
        .unwrap();
    let x = key["max"][0].as_f64().unwrap() + 28.0;
    let y = (key["min"][1].as_f64().unwrap() + key["max"][1].as_f64().unwrap()) * 0.5;
    let state = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(
        state["project"]["tracks"][2]["clips"][0]["content"]["Midi"]["notes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let wav = app.directory.path().join("native-mix.wav");
    app.command(json!({"command":"export","path":wav}));
    app.command(json!({"command":"wait_export"}));
    let audio = AudioData::load_wav(wav).unwrap();
    assert!(
        audio.samples.iter().any(|sample| sample.abs() > 0.001),
        "instrument export must contain rendered notes"
    );
    // Headless runs never initialize installed devices just to fill previews.
    // This opt-in native scenario explicitly asks for its selected instrument.
    menu(&mut app, "Window", "Library");
    app.click("Instruments");
    app.click("Search instruments");
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":name}));
    common::wait_visible(&mut app, &name);
    app.click(&name);
    let details = app.click("More");
    app.click(if visible(&details, "Refresh preview") {
        "Refresh preview"
    } else {
        "Capture preview"
    });
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let state = app.inspect();
        let entry = state["plugin_catalog"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap();
        if let Some(thumbnail) = entry["thumbnail"].as_str() {
            let image = image::open(thumbnail).unwrap();
            assert!(image.width() >= 100 && image.height() >= 100);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "selected instrument thumbnail was not cached: {entry}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // New must capture and close the second native editor before replacing the
    // project, keeping its latest preset in recovery.
    menu(&mut app, "File", "New");
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let state = app.inspect();
        if state["document_guard"]["phase"] == "confirm" {
            app.click("Don't save");
            continue;
        }
        if state["project"]["tracks"].as_array().unwrap().is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "New did not finish saving the native editor"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let recovery = Project::load(app.directory.path().join("scratch/recovery.daw.json")).unwrap();
    assert_eq!(recovery.tracks.len(), 3);
    assert!(!recovery.tracks[2].instrument_state.is_empty());
    app.command(json!({"command":"open","path":saved}));
    menu(&mut app, "Window", "Instrument");
    app.click("Open editor");
    show_plugin_parameters(&mut app);
    // Opening another project also waits for the active native editor to close.
    let state = app.command(json!({"command":"open","path":saved}));
    assert_eq!(state["native_editor"]["open"], false);
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 2);
    menu(&mut app, "Window", "Instrument");
    app.click("Open editor");
    show_plugin_parameters(&mut app);
    // Quit must close the real native editor and persist its final state before exit.
    let response = app.raw_command(json!({"command":"quit"}));
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["state"]["native_editor"]["open"], false);
    app.wait_exit();
}

#[test]
fn instrument_chooser_keyboard_and_double_click_create_ready_patterns_and_assign_preserves_them() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Track", "Add instrument…");
    // The search receives keyboard focus without another click.
    app.command(json!({"command":"text","text":"Fixture A"}));
    common::wait_visible(&mut app, "Rill Fixture A");
    let state = app.key("Enter");
    assert_eq!(
        state["project"]["tracks"].as_array().unwrap().len(),
        1,
        "labels={} popups={}",
        state["visible_labels"],
        state["popups"]
    );
    assert_eq!(state["project"]["tracks"][0]["clips"][0]["length"], 4.0);
    assert_only_popup(&state, "instrument");
    let original_clips = state["project"]["tracks"][0]["clips"].clone();
    app.click("Change instrument");
    app.command(json!({"command":"text","text":"Fixture B"}));
    common::wait_visible(&mut app, "Rill Fixture B");
    let state = app.key("Enter");
    assert_eq!(
        state["project"]["tracks"].as_array().unwrap().len(),
        1,
        "labels={} popups={}",
        state["visible_labels"],
        state["popups"]
    );
    assert_eq!(state["project"]["tracks"][0]["clips"], original_clips);
    assert_eq!(
        state["project"]["tracks"][0]["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    // Reassignment returns to the explicitly opened overview, without
    // automatically starting the replacement's native editor.
    // The original fixture's startup may still be retiring on a slower host;
    // wait for the replacement overview instead of clicking during that handoff.
    common::wait_visible(&mut app, "Open editor");
    let ready = app.inspect();
    for flag in ["open", "opening", "closing"] {
        assert_eq!(ready["native_editor"][flag], false, "{ready}");
    }
    assert!(ready["native_editor"]["error"].is_null(), "{ready}");
    app.click("Open editor");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
    loop {
        let state = app.inspect();
        if !state["native_editor"]["error"].is_null() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Metadata fixture must surface its missing executable"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        menu(&mut app, "Window", "Clip Editor")["view"],
        "Clip",
        "A failed plug-in stays editable"
    );
    menu(&mut app, "Track", "Add instrument…");
    app.command(json!({"command":"text","text":"Fixture A"}));
    common::wait_visible(&mut app, "Rill Fixture A");
    for _ in 0..15 {
        app.inspect();
    }
    for _ in 0..2 {
        let state = app.inspect();
        let card = state["library"]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["name"] == "Rill Fixture A")
            .unwrap();
        // Click the actual card preview, independent of matching names retained
        // in the caller and persistent selection footer.
        let rect = &card["rect"];
        app.command(json!({"command":"pointer", "phase":"click",
            "x":(rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
            "y":rect["min"][1].as_f64().unwrap() + 36.0}));
    }

    let state = app.inspect();
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(state["project"]["tracks"][1]["clips"][0]["length"], 4.0);
    // This metadata-only fixture cannot open a native editor. Let that expected
    // initialization result arrive before separately testing ordinary shutdown.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
    while app.inspect()["native_editor"]["opening"] == true {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.quit();
}

#[test]
fn adding_midi_track_immediately_assigns_instrument_and_cancel_keeps_ready_pattern() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let state = menu(&mut app, "Track", "Add MIDI track");
    assert_only_popup(&state, "library");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(state["project"]["tracks"][0]["instrument"], "None");
    let clips = state["project"]["tracks"][0]["clips"].clone();
    assert_eq!(clips[0]["length"], 4.0);
    let state = app.key("Escape");
    assert_only_popup(&state, "none");
    assert_eq!(state["project"]["tracks"][0]["clips"], clips);
    menu(&mut app, "Window", "Instrument");
    app.click("Choose instrument");
    app.command(json!({"command":"text","text":"Fixture B"}));
    common::wait_visible(&mut app, "Rill Fixture B");
    let state = app.key("Enter");
    assert_eq!(
        state["project"]["tracks"].as_array().unwrap().len(),
        1,
        "Assignment must use the newly created MIDI track"
    );
    assert_eq!(state["project"]["tracks"][0]["clips"], clips);
    assert_eq!(
        state["project"]["tracks"][0]["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    assert_only_popup(&state, "instrument");
    app.quit();
}

#[test]
fn midi_header_instrument_picker_targets_its_track_and_replaces_without_adding_tracks() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    for _ in 0..2 {
        menu(&mut app, "Track", "Add MIDI track");
        app.key("Escape");
    }
    let original = app.inspect();
    let first = original["project"]["tracks"][0]["id"].clone();
    let second = original["project"]["tracks"][1]["id"].clone();
    assert_eq!(original["selected_track"], second);

    // Both rows are unassigned. Clicking the first row's instrument control
    // must target that row, regardless of which track was selected beforehand.
    let state = app.click("Choose instrument…");
    assert_only_popup(&state, "library");
    assert_eq!(state["selected_track"], first);
    let state = app.key("Escape");
    assert_eq!(
        state["project"], original["project"],
        "Cancel preserves the unassigned tracks"
    );
    app.command(json!({"command":"click","label":"New instrument","occurrence":1}));
    app.click("Choose instrument…");
    app.command(json!({"command":"text","text":"Fixture A"}));
    common::wait_visible(&mut app, "Rill Fixture A");
    assert!(visible(&app.inspect(), "Assign instrument"));
    let assigned = app.key("Enter");
    assert_eq!(assigned["project"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(assigned["selected_track"], first);
    assert_eq!(
        assigned["project"]["tracks"][0]["instrument"]["Vst3"]["class_id"],
        "11111111111111111111111111111111"
    );
    assert_eq!(
        assigned["project"]["tracks"][1],
        original["project"]["tracks"][1]
    );
    assert_eq!(
        assigned["project"]["tracks"][0]["clips"],
        original["project"]["tracks"][0]["clips"]
    );

    // The compact row keeps its instrument control. Replacing from that row
    // must still affect the first track while the second track is selected.
    app.key("Escape");
    menu(&mut app, "Window", "Properties");
    click_front(&mut app, "Track");
    app.click("Compact track");
    app.key("Escape");
    let state = app.command(json!({"command":"click","label":"New instrument","occurrence":1}));
    assert_eq!(state["selected_track"], second);
    assert!(visible(&state, "Rill Fixture A"));
    let before_replacement = state["project"].clone();
    let state = app.click("Rill Fixture A");
    assert_only_popup(&state, "library");
    assert_eq!(state["selected_track"], first);
    app.command(json!({"command":"text","text":"Fixture B"}));
    common::wait_visible(&mut app, "Rill Fixture B");
    let replaced = app.key("Enter");
    assert_eq!(replaced["project"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(
        replaced["project"]["tracks"][0]["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    assert_eq!(replaced["project"]["tracks"][0]["collapsed"], true);
    assert_eq!(
        replaced["project"]["tracks"][0]["clips"],
        original["project"]["tracks"][0]["clips"]
    );
    assert_eq!(
        replaced["project"]["tracks"][1],
        original["project"]["tracks"][1]
    );
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"], before_replacement);
    let state = menu(&mut app, "Edit", "Redo");
    assert_eq!(state["project"], replaced["project"]);
    app.key("Escape");
    assert!(visible(&app.inspect(), "Rill Fixture B"));
    app.quit();
}
