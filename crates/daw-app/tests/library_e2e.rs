//! Device library actions use real popup controls and effect-chain drag gestures.
mod common;
use common::Harness;
use daw_core::{AutomationParameter, AutomationTarget, PluginParameterValue, Project};
use serde_json::{Value, json};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == label)
}
fn menu(app: &mut Harness, name: &str, item: &str) -> Value {
    app.click(name);
    app.click(item)
}
fn front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| *item == label)
        .count();
    assert!(count > 0, "Missing {label}: {state}");
    app.command(json!({"command":"click","label":label,"occurrence":count - 1}))
}
fn effect(app: &mut Harness, action: &str) -> Value {
    app.click("+ Insert");
    app.click("Search effects");
    app.command(json!({"command":"text","text":"Rill Fixture FX"}));
    common::wait_visible(app, "Add to effect chain");
    front(app, "Rill Fixture FX");
    if action != "Add to effect chain" {
        app.click("More");
    }
    app.click(action)
}
fn rect(state: &Value, label: &str, occurrence: usize) -> (f64, f64) {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["text"] == label)
        .nth(occurrence)
        .unwrap_or_else(|| panic!("Missing {label}: {state}"));
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

#[test]
fn library_filters_names_categories_and_developers_and_creates_or_assigns_instruments() {
    let fixture = common::plugin_fixture();
    let resource = fixture.path().join("Other.vst3/Contents/Resources");
    std::fs::create_dir_all(&resource).unwrap();
    std::fs::write(resource.join("moduleinfo.json"), json!({
        "Name":"Other", "Version":"1", "Factory Info":{"Vendor":"AAA Rill developer"},
        "Classes":[{"CID":"44444444444444444444444444444444","Category":"Audio Module Class","Name":"Other Keys","Sub Categories":["Instrument","Piano"],"Version":"1","SDKVersion":"VST3.7.9","Cardinality":2147483647}]
    }).to_string()).unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Window", "Library");
    app.click("Search instruments");
    app.command(json!({"command":"text","text":"Other"}));
    common::wait_visible(&mut app, "Other Keys");
    app.click("All developers");
    let enter = app.key("Enter");
    assert!(
        enter["project"]["tracks"].as_array().unwrap().is_empty(),
        "Enter in a developer popup must not instantiate a device"
    );
    app.key("Escape");
    menu(&mut app, "Window", "Library");
    app.click("All developers");
    front(&mut app, "AAA Rill developer");
    let state = app.inspect();
    assert!(
        visible(&state, "Other Keys"),
        "labels: {}",
        state["visible_labels"]
    );
    assert!(!visible(&state, "Rill Fixture A"));
    assert!(!visible(&state, "Rill Fixture FX"));
    app.click("Other");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"Piano"}));
    let state = app.inspect();
    assert!(
        visible(&state, "Other Keys"),
        "Category metadata is searchable"
    );
    assert!(!visible(&state, "VST3 · Instrument · Piano"));
    let details = app.click("More");
    assert!(visible(&details, "VST3 · Instrument · Piano"));
    app.key("Escape");
    let created = app.click("Add instrument track");
    assert_eq!(created["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(created["project"]["tracks"][0]["name"], "Other Keys");
    assert_eq!(created["project"]["tracks"][0]["kind"], "Midi");
    assert_eq!(created["popups"]["instrument"], true);
    menu(&mut app, "Window", "Library");
    app.click("AAA Rill developer");
    front(&mut app, "All developers");
    app.click("Piano");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"Rill Fixture"}));
    front(&mut app, "Rill Fixture B");
    app.click("More");
    let assigned = app.click("Assign to selected track");
    assert_eq!(assigned["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(
        assigned["project"]["tracks"][0]["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    app.quit();
}

#[test]
fn effect_insertion_send_returns_drag_reorder_and_archive_recall_preserve_identity() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Track", "Add audio track");
    menu(&mut app, "Window", "Effects");
    effect(&mut app, "Add to effect chain");
    let state = effect(&mut app, "Add to effect chain");
    assert_eq!(state["popups"]["effects"], true);
    let source = state["project"]["tracks"][0]["id"].as_u64().unwrap();
    let ids: Vec<_> = state["project"]["tracks"][0]["inserts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| slot["id"].as_u64().unwrap())
        .collect();
    let seed = app.directory.path().join("stateful.json");
    app.command(json!({"command":"save","path":seed}));
    // The fixture has no native code. Seed opaque state and an automation target
    // in a valid project, then exercise reordering through the production UI.
    let mut project = Project::load(&seed).unwrap();
    let slot = &mut project.track_mut(source).unwrap().inserts[0];
    slot.state = vec![1, 3, 5, 7];
    slot.parameters = vec![PluginParameterValue {
        id: 42,
        value: 0.35,
    }];
    project
        .add_automation_track_with_value(
            "Effect automation",
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::VstParameter {
                    effect: Some(ids[0]),
                    id: 42,
                },
            },
            0.35,
        )
        .unwrap();
    project.save(&seed).unwrap();
    app.command(json!({"command":"open","path":seed}));
    app.click("New audio");
    let before = menu(&mut app, "Window", "Effects");
    let from = rect(&before, "⋮⋮", 0);
    let to = rect(&before, "⋮⋮", 1);
    let after = app.command(
        json!({"command":"pointer","phase":"drag","x":from.0,"y":from.1,"to_x":to.0,"to_y":to.1}),
    );
    assert_eq!(after["project"]["tracks"][0]["inserts"][0]["id"], ids[1]);
    assert_eq!(
        after["project"]["tracks"][0]["inserts"][1],
        before["project"]["tracks"][0]["inserts"][0]
    );
    assert_eq!(
        after["project"]["tracks"][1], before["project"]["tracks"][1],
        "Automation must keep its stable slot target"
    );
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    menu(&mut app, "Edit", "Redo");
    let sent = effect(&mut app, "Create send track");
    let tracks = sent["project"]["tracks"].as_array().unwrap();
    let bus = tracks.iter().find(|track| track["kind"] == "Bus").unwrap();
    assert_eq!(bus["name"], "Rill Fixture FX return");
    assert_eq!(bus["inserts"].as_array().unwrap().len(), 1);
    assert_eq!(tracks[0]["sends"][0]["target_bus"], bus["id"]);
    assert_eq!(tracks[0]["sends"][0]["gain"], 0.25);
    assert_eq!(tracks[0]["sends"][0]["pre_fader"], false);
    assert_eq!(sent["selected_track"], bus["id"]);
    let archive = app.directory.path().join("library-session.rill");
    app.command(json!({"command":"save","path":archive}));
    let reopened = app.command(json!({"command":"open","path":archive}));
    assert_eq!(reopened["project"], sent["project"]);
    // The routing popup can enter the same send-creation browser directly.
    app.click("New audio");
    menu(&mut app, "Window", "Routing");
    let library = app.click("Create send track");
    assert_eq!(library["popups"]["library"], true);
    assert!(visible(&library, "Search effects"));
    let back = app.key("Escape");
    assert_eq!(back["popups"]["routing"], true);
    app.quit();
}

#[test]
fn library_reuses_cached_native_preview_records_for_instruments_and_effects() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let cache = daw_plugins::catalog::PluginCatalog::new(app.directory.path().join("cache"));
    let entries = cache.scan(&[fixture.path().to_path_buf()]).unwrap();
    for entry in entries {
        cache
            .capture(&entry.plugin, |_, path| {
                image::RgbaImage::from_pixel(32, 20, image::Rgba([40, 80, 160, 255])).save(path)?;
                Ok(())
            })
            .unwrap();
    }
    common::fixture_library(&mut app);
    let state = app.inspect();
    let ready: Vec<_> = state["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["name"].as_str().unwrap().starts_with("Rill Fixture"))
        .collect();
    assert_eq!(ready.len(), 3);
    for entry in ready {
        assert_eq!(entry["status"], "Ready");
        assert!(std::path::Path::new(entry["thumbnail"].as_str().unwrap()).exists());
    }
    assert!(!visible(&state, "Refresh preview"));
    let details = app.click("More");
    assert!(visible(&details, "Refresh preview"));
    app.key("Escape");
    app.click("Effects");
    app.click("Search effects");
    app.command(json!({"command":"text","text":"Rill Fixture FX"}));
    let state = app.inspect();
    assert!(visible(&state, "Rill Fixture FX"));
    assert!(!visible(&state, "Rill Fixture A"));
    assert!(!visible(&state, "Refresh preview"));
    let details = app.click("More");
    assert!(visible(&details, "Refresh preview"));
    app.key("Escape");
    app.quit();
}

#[test]
fn send_browser_has_one_contextual_action_and_library_management_is_on_demand() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Track", "Add audio track");
    menu(&mut app, "Window", "Routing");
    app.click("Create send track");
    app.click("Search effects");
    app.command(json!({"command":"text","text":"Rill Fixture FX"}));
    common::wait_visible(&mut app, "Create send track");
    let state = app.inspect();
    assert!(visible(&state, "Scan"));
    assert!(!visible(&state, "Add to effect chain"));
    assert!(!visible(&state, "Capture preview"));
    let added = app.click("Create send track");
    assert_eq!(added["project"]["tracks"][1]["kind"], "Bus");
    assert_eq!(
        added["project"]["tracks"][0]["sends"][0]["target_bus"],
        added["project"]["tracks"][1]["id"]
    );
    menu(&mut app, "Window", "Library");
    let managed = app.click("Manage");
    assert!(visible(&managed, "Folders"));
    let setup = app.click("Folders");
    assert_eq!(setup["popups"]["setup"], true);
    assert_eq!(setup["popups"]["library"], false);
    app.quit();
}
