//! The plug-in catalog syncs on open and on request, captures previews in the
//! background and keeps the user's organization, all through the real app.
//! Bundles contain metadata only; a test editor worker produces the previews.
mod common;

use common::{Harness, wait_until};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn bundle(root: &Path, name: &str, vendor: &str, category: &str, class: u32) {
    let contents = root.join(format!("{name}.vst3/Contents"));
    std::fs::create_dir_all(contents.join("Resources")).unwrap();
    std::fs::create_dir_all(contents.join("x86_64-linux")).unwrap();
    std::fs::write(
        contents.join("Resources/moduleinfo.json"),
        json!({"Name":name, "Version":"1", "Factory Info":{"Vendor":vendor},
            "Classes":[{"CID":format!("{class:032X}"), "Category":"Audio Module Class",
                "Name":name, "Sub Categories":category.split('|').collect::<Vec<_>>(),
                "Version":"2.5", "SDKVersion":"VST3.7.9", "Cardinality":2147483647}]})
        .to_string(),
    )
    .unwrap();
}

/// Compile the standard-library editor worker that answers capture requests.
fn preview_editor(directory: &Path) -> PathBuf {
    let helper = directory.join(if cfg!(windows) {
        "preview-editor.exe"
    } else {
        "preview-editor"
    });
    let result = std::process::Command::new(std::env::var_os("RUSTC").unwrap_or("rustc".into()))
        .args(["--edition=2024", "-C", "opt-level=1"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/common/preview_editor.rs"
        ))
        .arg("-o")
        .arg(&helper)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    helper
}

fn captures(directory: &Path) -> usize {
    std::fs::read_to_string(directory.join("captures.log")).map_or(0, |log| log.lines().count())
}

fn entry<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not in the catalog: {}", state["plugin_catalog"]))
}

fn has(state: &Value, name: &str) -> bool {
    state["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["name"] == name)
}

fn settled(state: &Value) -> bool {
    state["plugin_scanning"] == false
        && state["library"]["sync"]["phase"]
            .as_str()
            .is_none_or(|phase| phase == "Idle")
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == label)
}

fn card<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["library"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["name"] == name)
        .unwrap_or_else(|| panic!("No card for {name}: {}", state["library"]))
}

/// Filter chips sit above the sidebar row that shares their label.
fn click_topmost(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["text"] == label)
        .min_by(|a, b| {
            a["min"][1]
                .as_f64()
                .unwrap()
                .total_cmp(&b["min"][1].as_f64().unwrap())
        })
        .unwrap_or_else(|| panic!("Missing {label}"))
        .clone();
    pointer(app, &json!({"rect": rect}), "click")
}

fn pointer(app: &mut Harness, card: &Value, phase: &str) -> Value {
    let rect = &card["rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":phase, "x":x, "y":y}))
}

#[test]
fn catalog_syncs_on_open_captures_previews_in_background_and_refreshes_on_request() {
    let workers = tempfile::tempdir().unwrap();
    let helper = preview_editor(workers.path());
    let plugins = tempfile::tempdir().unwrap();
    bundle(plugins.path(), "Acme Keys", "Acme", "Instrument|Piano", 1);
    bundle(plugins.path(), "Acme Echo", "Acme", "Fx|Delay", 2);
    std::fs::create_dir_all(plugins.path().join("Mystery Box.vst3")).unwrap();
    let mut app = Harness::with_plugin_previews(plugins.path(), &helper);

    // Opening the app indexes the folders and captures verified previews
    // without opening the library. Unverified bundles are left alone.
    let state = wait_until(&mut app, |state| {
        settled(state)
            && has(state, "Acme Echo")
            && entry(state, "Acme Keys")["status"] == "Ready"
            && entry(state, "Acme Echo")["status"] == "Ready"
    });
    assert_eq!(state["popups"]["library"], false);
    assert_eq!(entry(&state, "Mystery Box")["status"], "Missing");
    assert_eq!(captures(workers.path()), 2);
    let keys = entry(&state, "Acme Keys");
    assert_eq!(keys["version"], "2.5");
    assert_eq!(keys["architectures"], json!(["x86_64-linux"]));
    let keys_added = keys["added_unix_ms"].as_u64().unwrap();
    assert!(keys_added > 0);
    let index = app.directory.path().join("cache/plugin-index-v1.json");
    assert!(index.is_file(), "the catalog index is written to the cache");

    app.click("Window");
    let library = app.click("Library");
    assert!(card(&library, "Acme Keys")["preview"].is_boolean());
    let library = wait_until(&mut app, |state| {
        card(state, "Acme Keys")["preview"] == true
    });
    assert!(visible(&library, "Format") && visible(&library, "2.5"));
    assert!(visible(&library, "x86_64-linux"));
    assert!(visible(&library, "Captured"));

    // Selecting the unverified bundle captures it on demand; More refreshes it.
    let selected = pointer(&mut app, card(&library, "Mystery Box"), "click");
    assert!(selected["project"]["tracks"].as_array().unwrap().is_empty());
    let state = wait_until(&mut app, |state| {
        settled(state) && entry(state, "Mystery Box")["status"] == "Ready"
    });
    assert_eq!(captures(workers.path()), 3);
    assert!(visible(&state, "Unverified VST3"));
    app.click("More");
    app.click("Refresh preview");
    wait_until(&mut app, |_| captures(workers.path()) == 4);
    wait_until(&mut app, settled);
    app.key("Escape");

    // A new session shows the indexed catalog and its previews again
    // without relaunching any editor.
    let mut app = app.restart();
    let state = wait_until(&mut app, |state| {
        settled(state)
            && has(state, "Mystery Box")
            && ["Acme Keys", "Acme Echo", "Mystery Box"]
                .iter()
                .all(|name| entry(state, name)["status"] == "Ready")
    });
    assert_eq!(captures(workers.path()), 4);
    assert_eq!(
        entry(&state, "Acme Keys")["added_unix_ms"].as_u64(),
        Some(keys_added),
        "an indexed plug-in keeps its first-indexed time"
    );

    // Refresh picks up installs and removals; the new device is captured
    // in the background and sorts first by Recently added.
    std::fs::remove_dir_all(plugins.path().join("Acme Echo.vst3")).unwrap();
    bundle(plugins.path(), "Nova Pad", "Nova", "Instrument|Synth", 3);
    std::thread::sleep(std::time::Duration::from_millis(5));
    common::refresh_library(&mut app);
    let state = wait_until(&mut app, |state| {
        settled(state) && has(state, "Nova Pad") && entry(state, "Nova Pad")["status"] == "Ready"
    });
    assert!(
        !has(&state, "Acme Echo"),
        "an uninstalled plug-in leaves the catalog"
    );
    assert_eq!(captures(workers.path()), 5);
    assert!(entry(&state, "Nova Pad")["added_unix_ms"].as_u64().unwrap() > keys_added);
    assert_eq!(state["library"]["order"][0], "Acme Keys");
    app.click("Sort: Name");
    let sorted = app.click("Recently added");
    assert_eq!(sorted["library"]["sort"], "Recently added");
    assert_eq!(sorted["library"]["order"][0], "Nova Pad");
    assert!(sorted["project"]["tracks"].as_array().unwrap().is_empty());
    app.quit();
}

#[test]
fn favorites_categories_filters_and_list_view_work_by_mouse_and_persist() {
    let fixture = common::plugin_fixture();
    let bundle_path = fixture
        .path()
        .join("RillFixture.vst3")
        .canonicalize()
        .unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    common::fixture_library(&mut app);
    let library = app.inspect();
    pointer(&mut app, card(&library, "Rill Fixture B"), "click");

    // Favorites: the detail pane toggle, the More menu and a card badge.
    let state = app.click("Favorite");
    assert_eq!(entry(&state, "Rill Fixture B")["favorite"], true);
    assert_eq!(card(&state, "Rill Fixture B")["favorite"], true);
    assert_eq!(card(&state, "Rill Fixture A")["favorite"], false);
    app.click("More");
    let state = app.click("Remove from favorites");
    assert_eq!(entry(&state, "Rill Fixture B")["favorite"], false);
    app.click("More");
    let state = app.click("Add to favorites");
    assert_eq!(entry(&state, "Rill Fixture B")["favorite"], true);

    // Collections filter with a removable chip.
    let favorites = app.click("Favorites");
    assert_eq!(favorites["library"]["collection"], "Favorites");
    assert_eq!(favorites["library"]["total"], 1);
    assert!(favorites["library"]["sidebar"]["min"][1].as_f64() > Some(0.0));
    let chip = click_topmost(&mut app, "Favorites");
    assert_eq!(chip["library"]["collection"], "All plug-ins");
    assert_eq!(chip["library"]["total"], 2);

    // Categories are the user's own once saved, and drive the category filter.
    app.click("Compressor, Vintage");
    app.command(json!({"command":"text", "text":"Vintage, Keys, vintage"}));
    let saved = app.click("Save");
    assert_eq!(
        entry(&saved, "Rill Fixture B")["categories"],
        json!(["Vintage", "Keys"])
    );
    assert!(visible(&saved, "Yours · save empty to reset"));
    assert!(
        visible(&saved, "Vintage, Keys"),
        "the field shows the saved, normalized categories"
    );
    assert!(saved["project"]["tracks"].as_array().unwrap().is_empty());
    app.click("All categories");
    let filtered = app.click("Vintage");
    assert_eq!(filtered["library"]["category"], "Vintage");
    assert_eq!(filtered["library"]["total"], 1);
    app.click("Rill tests");
    let both = app.inspect();
    assert_eq!(both["library"]["developer"], "Rill tests");
    assert!(visible(&both, "Clear filters"));
    let cleared = app.click("Clear filters");
    assert_eq!(cleared["library"]["category"], "");
    assert_eq!(cleared["library"]["developer"], "");
    assert_eq!(cleared["library"]["total"], 2);

    // Copy path uses the selected device's install path.
    let state = app.click("Copy path");
    assert_eq!(
        state["clipboard_text"],
        json!(bundle_path.display().to_string())
    );

    // The list view keeps selection and double-click creation.
    app.click("View");
    let list = app.click("List");
    assert_eq!(list["library"]["view"], "list");
    assert_eq!(list["library"]["card_size"][1], 56.0);
    assert!(
        card(&list, "Rill Fixture B")["rect"]["max"][0]
            .as_f64()
            .unwrap()
            - card(&list, "Rill Fixture B")["rect"]["min"][0]
                .as_f64()
                .unwrap()
            > 300.0,
        "list rows span the catalog"
    );
    let created = pointer(&mut app, card(&list, "Rill Fixture B"), "double_click");
    assert_eq!(created["project"]["tracks"][0]["name"], "Rill Fixture B");

    // Organization persists across sessions and is shared with the AI tools.
    assert!(
        app.directory
            .path()
            .join("config/plugin-library.json")
            .is_file()
    );
    let mut app = app.restart();
    let state = wait_until(&mut app, |state| has(state, "Rill Fixture B"));
    assert_eq!(entry(&state, "Rill Fixture B")["favorite"], true);
    assert_eq!(entry(&state, "Rill Fixture A")["favorite"], false);
    let listed = app.command(json!({"command":"song_tool", "name":"list_plugins",
        "arguments":{"query":"Rill Fixture B"}}));
    let plugin = &listed["tool_result"]["result"]["plugins"][0];
    assert_eq!(plugin["favorite"], true, "{listed}");
    assert_eq!(plugin["categories"], json!(["Vintage", "Keys"]));

    // Tools organize the same library and can request a rescan.
    let tool = |app: &mut Harness, name: &str, arguments: Value| {
        app.command(json!({"command":"song_tool", "name":name, "arguments":arguments}))
            ["tool_result"]
            .clone()
    };
    let listed = tool(
        &mut app,
        "list_plugins",
        json!({"query":"Rill Fixture A", "refresh":true}),
    );
    assert_eq!(listed["result"]["scanning"], true, "{listed}");
    let id = listed["result"]["plugins"][0]["id"].clone();
    let organized = tool(
        &mut app,
        "organize_plugin",
        json!({"plugin_id":id, "favorite":true, "categories":["Bright"]}),
    );
    assert_eq!(organized["ok"], true, "{organized}");
    assert_eq!(organized["result"]["categories"], json!(["Bright"]));
    let rejected = tool(
        &mut app,
        "organize_plugin",
        json!({"plugin_id":id, "categories":["a,b"]}),
    );
    assert_eq!(rejected["ok"], false, "{rejected}");
    let state = wait_until(&mut app, |state| state["plugin_scanning"] == false);
    assert_eq!(entry(&state, "Rill Fixture A")["favorite"], true);
    assert_eq!(
        entry(&state, "Rill Fixture A")["categories"],
        json!(["Bright"])
    );

    // Saving an empty field restores the plug-in's own categories.
    common::fixture_library(&mut app);
    let library = app.inspect();
    pointer(&mut app, card(&library, "Rill Fixture B"), "click");
    app.click("Vintage, Keys");
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"key", "key":"Backspace"}));
    let reset = app.click("Save");
    assert_eq!(entry(&reset, "Rill Fixture B")["categories"], json!([]));
    assert!(visible(&reset, "From the plug-in"));
    app.quit();
}
