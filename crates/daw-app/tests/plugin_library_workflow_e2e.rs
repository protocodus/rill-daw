//! Discovery, personal collections and insertion through the running app.
//! The fixture only declares VST metadata; browsing never needs a native editor.
mod common;

use common::{Harness, wait_until};
use serde_json::{Value, json};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn menu(app: &mut Harness, menu: &str, label: &str) -> Value {
    app.click(menu);
    app.click(label)
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn contains(outer: &Value, inner: &Value) -> bool {
    // Painted device rectangles round to physical pixels; the layout pane
    // retains subpixel coordinates at both supported display densities.
    (0..2).all(|axis| {
        inner["min"][axis].as_f64().unwrap() >= outer["min"][axis].as_f64().unwrap() - 1.0
            && inner["max"][axis].as_f64().unwrap() <= outer["max"][axis].as_f64().unwrap() + 1.0
    })
}

fn select_device(app: &mut Harness, name: &str) -> Value {
    common::wait_library_device(app, name);
    let state = app.inspect();
    let card = state["library"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["name"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["library"]));
    click_rect(app, &card["rect"])
}

fn collection(app: &mut Harness, label: &str) -> Value {
    let state = common::reveal_library_filter(app, label);
    let sidebar = &state["library"]["sidebar"];
    let label_rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == label && contains(sidebar, rect))
        .unwrap_or_else(|| panic!("Missing sidebar {label}: {}", state["library"]));
    click_rect(app, label_rect);
    wait_until(app, |state| state["library"]["collection"] == label)
}

fn footer_favorite(app: &mut Harness) -> Value {
    let state = app.inspect();
    let footer = &state["library"]["footer"];
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == "Favorite" && contains(footer, rect))
        .unwrap_or_else(|| {
            panic!(
                "Favorite must remain in the pinned footer: {}",
                state["label_rects"]
            )
        });
    assert!(rect["max"][0].as_f64().unwrap() <= state["viewport"][0].as_f64().unwrap());
    assert!(rect["max"][1].as_f64().unwrap() <= state["viewport"][1].as_f64().unwrap());
    click_rect(app, rect)
}

fn compact_layout(state: &Value) {
    let viewport = json!({"min":[0.0, 0.0], "max":state["viewport"]});
    for pane in ["window", "catalog", "footer"] {
        assert!(
            contains(&viewport, &state["library"][pane]),
            "{pane} must stay within the viewport: {}",
            state["library"]
        );
    }
    let labels = state["label_rects"].as_array().unwrap();
    let mut centers = Vec::new();
    for label in ["Rill Fixture", "All categories", "All producers", "View"] {
        let rect = labels.iter().find(|rect| rect["text"] == label).unwrap();
        centers.push((rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5);
    }
    assert!(
        centers
            .iter()
            .all(|center| (center - centers[0]).abs() <= 1.0),
        "Search and filter controls share one center line: {centers:?}"
    );
    assert!(
        state["library"]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| contains(&state["library"]["catalog"], &card["rect"])),
        "At least one complete card remains visible on a compact screen: {}",
        state["library"]
    );
}

fn entry<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap()
}

fn saved_library(app: &Harness) -> Value {
    let file = app.directory.path().join("config/plugin-library.json");
    if file.is_file() {
        serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap()
    } else {
        json!({"recently_used":[]})
    }
}

fn fixture_key(fixture: &std::path::Path, class: &str) -> String {
    format!(
        "{}#{class}",
        fixture
            .join("RillFixture.vst3")
            .canonicalize()
            .unwrap()
            .display()
    )
}

fn replace_instrument(app: &mut Harness, name: &str) -> Value {
    menu(app, "Window", "Instrument");
    app.click("Change instrument");
    collection(app, "All plug-ins");
    select_device(app, name);
    app.click("Assign instrument")
}

#[test]
fn recently_used_tracks_assignments_in_mru_order_and_survives_restart() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    common::fixture_library(&mut app);

    // Selecting or favouriting a device does not count as using it.
    select_device(&mut app, "Rill Fixture B");
    footer_favorite(&mut app);
    let unused = collection(&mut app, "Recently used");
    assert_eq!(unused["library"]["total"], 0);
    assert_eq!(saved_library(&app)["recently_used"], json!([]));
    collection(&mut app, "All plug-ins");

    menu(&mut app, "Track", "Add MIDI track");
    select_device(&mut app, "Rill Fixture A");
    let first = app.click("Assign instrument");
    assert_eq!(first["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(first["native_editor"]["open"], false);
    replace_instrument(&mut app, "Rill Fixture B");
    replace_instrument(&mut app, "Rill Fixture A");

    let a = fixture_key(fixture.path(), "11111111111111111111111111111111");
    let b = fixture_key(fixture.path(), "22222222222222222222222222222222");
    assert_eq!(saved_library(&app)["recently_used"], json!([a, b]));
    menu(&mut app, "Window", "Library");
    let recent = collection(&mut app, "Recently used");
    assert_eq!(
        recent["library"]["order"],
        json!(["Rill Fixture A", "Rill Fixture B"])
    );

    // Browsing does not change the MRU order or the song.
    let project = recent["project"].clone();
    let selected = select_device(&mut app, "Rill Fixture B");
    assert_eq!(selected["project"], project);
    assert_eq!(selected["library"]["order"], recent["library"]["order"]);
    let mut app = app.restart();
    common::fixture_library(&mut app);
    let recalled = collection(&mut app, "Recently used");
    assert_eq!(recalled["library"]["order"], recent["library"]["order"]);
    assert_eq!(entry(&recalled, "Rill Fixture B")["favorite"], true);
    app.quit();
}

#[test]
fn insert_and_send_actions_record_recent_effects_without_duplicate_entries() {
    let fixture = common::plugin_fixture();
    let path = fixture
        .path()
        .join("RillFixture.vst3/Contents/Resources/moduleinfo.json");
    let mut metadata: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut second = metadata["Classes"][2].clone();
    second["CID"] = json!("44444444444444444444444444444444");
    second["Name"] = json!("Rill Fixture Echo");
    metadata["Classes"].as_array_mut().unwrap().push(second);
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Track", "Add audio track");
    menu(&mut app, "Window", "Effects");
    app.click("+ Insert");
    select_device(&mut app, "Rill Fixture FX");
    let inserted = app.click("Add to effect chain");
    assert_eq!(
        inserted["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    app.click("+ Insert");
    select_device(&mut app, "Rill Fixture Echo");
    app.click("More");
    let sent = app.click("Create send track");
    assert_eq!(sent["project"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(
        sent["project"]["tracks"][0]["sends"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    menu(&mut app, "Window", "Library");
    app.click("Effects");
    let recent = collection(&mut app, "Recently used");
    assert_eq!(
        recent["library"]["order"],
        json!(["Rill Fixture Echo", "Rill Fixture FX"])
    );

    select_device(&mut app, "Rill Fixture FX");
    app.click("Add to effect chain");
    menu(&mut app, "Window", "Library");
    app.click("Effects");
    let reused = collection(&mut app, "Recently used");
    assert_eq!(
        reused["library"]["order"],
        json!(["Rill Fixture FX", "Rill Fixture Echo"])
    );
    let fx = fixture_key(fixture.path(), "33333333333333333333333333333333");
    let echo = fixture_key(fixture.path(), "44444444444444444444444444444444");
    assert_eq!(saved_library(&app)["recently_used"], json!([fx, echo]));
    app.quit();
}

#[test]
fn search_combines_name_producer_and_categories_in_any_word_order() {
    let fixture = common::plugin_fixture();
    let path = fixture
        .path()
        .join("RillFixture.vst3/Contents/Resources/moduleinfo.json");
    let mut metadata: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    metadata["Classes"][0]["Sub Categories"] = json!(["Instrument", "Synth"]);
    metadata["Classes"][1]["Sub Categories"] = json!(["Instrument", "Piano"]);
    metadata["Classes"][2]["Sub Categories"] = json!(["Fx", "Delay"]);
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    common::fixture_library(&mut app);
    app.click("Rill Fixture");
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    let mixed = app.command(json!({"command":"text", "text":"  pIaNo  tests B  "}));
    assert_eq!(mixed["library"]["order"], json!(["Rill Fixture B"]));
    assert!(mixed["project"]["tracks"].as_array().unwrap().is_empty());

    // Explicit category and producer filters still compose with the free search.
    app.click("All categories");
    let category = app.click("Piano");
    assert_eq!(category["library"]["order"], json!(["Rill Fixture B"]));
    app.click("All producers");
    app.click("Search producers");
    let producers = app.command(json!({"command":"text", "text":"TESTS"}));
    assert!(visible(&producers, "Rill tests"));
    // Choose the popup's copy of the producer, rather than its sidebar row.
    let occurrences = producers["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == "Rill tests")
        .count();
    app.command(json!({"command":"click", "label":"Rill tests", "occurrence":occurrences - 1}));
    assert_eq!(app.inspect()["library"]["developer"], "Rill tests");
    app.click("Clear filters");

    // User categories participate in the same all-word query.
    select_device(&mut app, "Rill Fixture B");
    app.click("Piano");
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":"Vintage, Keys"}));
    app.click("Save");
    app.click("Search instruments");
    let custom = app.command(json!({"command":"text", "text":"keys TESTS b vintage"}));
    assert_eq!(custom["library"]["order"], json!(["Rill Fixture B"]));
    app.click("keys TESTS b vintage");
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    let absent = app.command(json!({"command":"text", "text":"tests nonexistent"}));
    assert_eq!(absent["library"]["total"], 0);
    assert!(visible(&absent, "No matching instruments"));
    assert!(absent["project"]["tracks"].as_array().unwrap().is_empty());
    app.quit();
}

#[test]
fn favorite_action_remains_in_the_footer_on_small_screens_and_persists() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    app.command(json!({"command":"viewport", "width":800, "height":600, "pixels_per_point":2.0}));
    common::fixture_library(&mut app);
    compact_layout(&app.inspect());
    let cleared = app.click("Clear");
    assert!(visible(&cleared, "Search instruments"));
    // The two fixtures and the built-in R1 Mini.
    assert_eq!(cleared["library"]["total"], 3);
    select_device(&mut app, "Rill Fixture B");
    assert!(app.inspect()["library"]["details"].is_null());
    let marked = footer_favorite(&mut app);
    assert_eq!(entry(&marked, "Rill Fixture B")["favorite"], true);
    let filtered = collection(&mut app, "Favorites");
    assert_eq!(filtered["library"]["order"], json!(["Rill Fixture B"]));
    assert!(filtered["project"]["tracks"].as_array().unwrap().is_empty());

    let mut app = app.restart();
    app.command(json!({"command":"viewport", "width":800, "height":600}));
    common::fixture_library(&mut app);
    let recalled = collection(&mut app, "Favorites");
    assert_eq!(recalled["library"]["order"], json!(["Rill Fixture B"]));
    footer_favorite(&mut app);
    let removed = wait_until(&mut app, |state| state["library"]["total"] == 0);
    assert_eq!(entry(&removed, "Rill Fixture B")["favorite"], false);
    assert_eq!(saved_library(&app)["favorites"], json!([]));
    app.quit();
}
