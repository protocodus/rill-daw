//! Search-first device choice, compact organization and safe destinations in the real app.
mod common;

use common::Harness;
use serde_json::{Value, json};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

fn query(app: &mut Harness, text: &str) -> Value {
    let state = app.inspect();
    let current = state["library"]["query"].as_str().unwrap_or("");
    let label = if current.is_empty() {
        [
            "Search instruments",
            "Search effects",
            "Search unverified devices",
        ]
        .into_iter()
        .find(|label| visible(&state, label))
        .unwrap()
    } else {
        current
    };
    app.click(label);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}))
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(state[field], before[field], "Browsing changed {field}");
    }
}

fn contains(outer: &Value, inner: &Value) -> bool {
    (0..2).all(|axis| {
        inner["min"][axis].as_f64().unwrap() >= outer["min"][axis].as_f64().unwrap() - 1.0
            && inner["max"][axis].as_f64().unwrap() <= outer["max"][axis].as_f64().unwrap() + 1.0
    })
}

fn choose_collection(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    if state["library"]["sidebar"].is_null() {
        app.click("Collection");
        common::click_last_label(app, label);
    } else {
        let state = common::reveal_library_filter(app, label);
        let sidebar = &state["library"]["sidebar"];
        let rect = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == label && contains(sidebar, rect))
            .unwrap();
        let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
        let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
        app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    }
    common::wait_until(app, |state| state["library"]["collection"] == label)
}

#[test]
fn search_first_list_preserves_query_across_types_and_explains_empty_collections() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    common::fixture_library(&mut app);
    let before = app.inspect();
    assert_eq!(before["library"]["view"], "list");
    assert_eq!(before["library"]["columns"], 1);
    assert_eq!(before["library"]["collection"], "All devices");
    assert!(!visible(&before, "With preview"));
    assert!(!visible(&before, "Without preview"));
    assert!(!visible(&before, "Install path"));
    assert!(visible(&before, "Manage plug-ins"));

    let effects = app.click("Effects");
    assert_eq!(effects["library"]["query"], "Rill Fixture");
    assert_eq!(effects["library"]["order"], json!(["Rill Fixture FX"]));
    let instruments = app.click("Instruments");
    assert_eq!(instruments["library"]["query"], "Rill Fixture");
    assert_eq!(instruments["library"]["total"], 2);
    app.click("Clear");
    let favorites = choose_collection(&mut app, "Favorites");
    assert_eq!(favorites["library"]["total"], 0);
    assert!(visible(&favorites, "No favorite instruments yet"));
    assert!(visible(&favorites, "Browse all"));
    app.click("Browse all");
    let recent = choose_collection(&mut app, "Recently used");
    assert_eq!(recent["library"]["total"], 0);
    assert!(visible(&recent, "Browse all"));
    app.click("Browse all");

    query(&mut app, "no-such-instrument-997");
    let empty = app.inspect();
    assert!(visible(&empty, "No matching instruments"));
    assert!(empty["library"]["selected"].is_null());
    unchanged(&app.key("Enter"), &before);
    let reset = app.click("Reset filters");
    assert_eq!(reset["library"]["query"], "");
    assert_eq!(reset["library"]["collection"], "All devices");
    assert!(reset["library"]["total"].as_u64().unwrap() >= 3);
    let unverified = app.click("Unverified");
    assert!(visible(&unverified, "No unverified devices"));
    assert!(!visible(&unverified, "No matching effects"));
    unchanged(&unverified, &before);
    app.click("Back");
    unchanged(&app.inspect(), &before);
    app.quit();
}

#[test]
fn compact_details_support_favorite_category_save_and_reset_without_loading() {
    let fixture = common::plugin_fixture();
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::with_vst_path(fixture.path());
        app.command(
            json!({"command":"viewport","width":800,"height":600,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        common::fixture_library(&mut app);
        common::wait_library_device(&mut app, "Rill Fixture B");
        app.click("Rill Fixture B");
        let before = app.inspect();
        assert!(before["library"]["details"].is_null());
        let viewport = json!({"min":[0.0,0.0],"max":before["viewport"]});
        for pane in ["window", "catalog", "footer"] {
            assert!(
                contains(&viewport, &before["library"][pane]),
                "{pane}: {}",
                before["library"]
            );
        }
        let details = app.click("Details");
        assert!(
            contains(&viewport, &details["library"]["details_popup"]),
            "{}",
            details["library"]
        );
        assert!(visible(&details, "Organize"));
        assert!(visible(&details, "Favorite"));
        assert!(visible(&details, "Device details"));
        app.click("Favorite");
        app.click("Organize");
        app.click("Compressor, Vintage");
        app.command(json!({"command":"text","text":"Bright, Pluck, bright"}));
        let saved = app.click("Save");
        let entry = saved["plugin_catalog"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "Rill Fixture B")
            .unwrap();
        assert_eq!(entry["favorite"], true);
        assert_eq!(entry["categories"], json!(["Bright", "Pluck"]));
        unchanged(&saved, &before);
        let reset = app.click("Reset categories");
        let entry = reset["plugin_catalog"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "Rill Fixture B")
            .unwrap();
        assert_eq!(entry["categories"], json!([]));
        unchanged(&reset, &before);
        app.key("Escape");
        assert_eq!(app.inspect()["popups"]["library"], true);
        let favorites = choose_collection(&mut app, "Favorites");
        assert_eq!(favorites["library"]["order"], json!(["Rill Fixture B"]));
        unchanged(&favorites, &before);
        app.quit();
    }
}

#[test]
fn reopening_same_instrument_target_restores_intent_and_deleted_target_never_adds_a_track() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let added = menu(&mut app, "Track", "Add MIDI track");
    assert!(visible(&added, "Set instrument"));
    let id = added["selected_track"].clone();
    app.click("Effects");
    query(&mut app, "no-such-device-998");
    app.click("Back");
    menu(&mut app, "Window", "Instrument");
    let reopened = app.click("Change instrument");
    assert!(visible(&reopened, "Replace instrument"));
    assert!(visible(&reopened, "Search instruments"));
    assert_eq!(reopened["library"]["query"], "");
    common::wait_library_device(&mut app, "Rill Fixture A");
    app.click("Rill Fixture A");
    let before = app.inspect();
    let deleted = app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":before["song_session"]["id"],"expected_revision":before["song_session"]["revision"],
        "label":"Delete chooser destination","edits":[{"op":"delete_track","track":id}]
    }}));
    assert_eq!(deleted["tool_result"]["ok"], true);
    let state = app.inspect();
    assert!(visible(&state, "Destination no longer available"));
    let after = app.key("Enter");
    unchanged(&after, &deleted);
    assert!(after["project"]["tracks"].as_array().unwrap().is_empty());
    assert_eq!(after["popups"]["library"], true);
    app.quit();
}

#[test]
fn master_effect_browser_names_its_destination_and_never_offers_a_send() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let mixer = app.click("Mixer");
    let rect = &mixer["mixer"]["master"]["effects"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let opened = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    if opened["popups"]["library"] != true {
        app.click("+ Insert");
    }
    common::wait_library_device(&mut app, "Rill Fixture FX");
    app.click("Rill Fixture FX");
    let browser = app.inspect();
    assert_eq!(
        browser["library"]["pinned_source"],
        daw_core::MASTER_CHANNEL_ID
    );
    assert!(
        browser["library"]["destination"]
            .as_str()
            .unwrap()
            .contains("Master")
    );
    assert_eq!(browser["library"]["primary_action"], "Add to effect chain");
    assert_eq!(browser["library"]["primary_enabled"], true);
    let more = app.click("More");
    assert!(!visible(&more, "Create send track"));
    app.key("Escape");
    let added = app.click("Add to effect chain");
    assert_eq!(
        added["project"]["master"]["inserts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(added["project"]["tracks"].as_array().unwrap().is_empty());
    app.key("Escape");
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], browser["project"]);
    app.quit();
}

#[test]
fn short_browser_keeps_a_complete_result_with_query_category_and_collection_active() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":800,"height":600,"pixels_per_point":1.25}));
    menu(&mut app, "Window", "Device browser");
    query(&mut app, "R1 Mini");
    common::wait_library_device(&mut app, "R1 Mini");
    let before = app.inspect();
    app.click("Details");
    app.click("Favorite");
    app.key("Escape");
    app.click("All categories");
    app.click("Synth");
    choose_collection(&mut app, "Favorites");
    let state = app.inspect();
    assert_eq!(state["library"]["query"], "R1 Mini");
    assert_eq!(state["library"]["category"], "Synth");
    assert_eq!(state["library"]["collection"], "Favorites");
    assert_eq!(state["library"]["total"], 1);
    let row = state["library"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["selected"] == true)
        .unwrap();
    assert!(
        contains(&state["library"]["catalog"], &row["rect"]),
        "Selected result clipped: {}",
        state["library"]
    );
    assert!(
        row["rect"]["max"][1].as_f64().unwrap() - row["rect"]["min"][1].as_f64().unwrap()
            >= f64::from(daw_ui::DEVICE_ROW_HEIGHT) - 1.0
    );
    unchanged(&state, &before);
    app.click("View");
    app.click("Grid");
    let grid = app.inspect();
    let card = grid["library"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["selected"] == true)
        .unwrap();
    assert!(
        contains(&grid["library"]["catalog"], &card["rect"]),
        "Selected grid card clipped with active filters: {}",
        grid["library"]
    );
    unchanged(&grid, &before);
    app.quit();
}

#[test]
fn unverified_replacement_names_the_pinned_intent_without_activating_on_selection() {
    let fixture = common::plugin_fixture();
    std::fs::create_dir_all(fixture.path().join("Mystery Box.vst3")).unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    let before = menu(&mut app, "Track", "Add MIDI track");
    app.click("Unverified");
    common::wait_library_device(&mut app, "Mystery Box");
    let selected = app.click("Mystery Box");
    unchanged(&selected, &before);
    let more = app.click("More");
    assert!(visible(&more, "Try replacing instrument"));
    assert!(!visible(&more, "Try as instrument"));
    assert!(
        more["library"]["destination"]
            .as_str()
            .unwrap()
            .contains("New instrument")
    );
    app.key("Escape");
    app.click("Back");
    unchanged(&app.inspect(), &before);
    app.quit();
}

#[test]
fn failed_compact_category_save_keeps_the_draft_visible_and_retryable() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    app.command(json!({"command":"viewport","width":800,"height":600}));
    common::fixture_library(&mut app);
    common::wait_library_device(&mut app, "Rill Fixture B");
    app.click("Rill Fixture B");
    let before = app.inspect();
    let file = app.directory.path().join("config/plugin-library.json");
    if file.is_file() {
        std::fs::remove_file(&file).unwrap();
    }
    std::fs::create_dir_all(file.join("blocked")).unwrap();
    app.click("Details");
    app.click("Organize");
    app.click("Compressor, Vintage");
    app.command(json!({"command":"text","text":"Retryable category"}));
    app.click("Save");
    let failed = app.inspect();
    assert!(visible(&failed, "Could not save device organization"));
    assert!(visible(&failed, "Retryable category"));
    let entry = failed["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "Rill Fixture B")
        .unwrap();
    assert_eq!(entry["categories"], json!([]));
    unchanged(&failed, &before);
    std::fs::remove_dir_all(&file).unwrap();
    app.click("Save");
    let saved = app.inspect();
    assert!(!visible(&saved, "Could not save device organization"));
    let entry = saved["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "Rill Fixture B")
        .unwrap();
    assert_eq!(entry["categories"], json!(["Retryable category"]));
    unchanged(&saved, &before);
    app.quit();
}
