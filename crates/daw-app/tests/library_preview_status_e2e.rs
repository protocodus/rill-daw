//! Image availability is separate from instrument assignment and loading.
//! These discovery-only VST3 fixtures cannot produce audio or open an editor.
mod common;

use common::Harness;
use daw_plugins::catalog::PluginCatalog;
use serde_json::{Value, json};

const CAPTURE_ERROR: &str = "Fixture editor has no image capture surface";

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

fn preserved(state: &Value, baseline: &Value) {
    for key in ["project", "history", "modified"] {
        assert_eq!(state[key], baseline[key], "Preview UI changed {key}");
    }
}

fn entry(state: &Value) -> &Value {
    state["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "Rill Fixture A")
        .unwrap()
}

#[test]
fn unavailable_images_keep_details_retry_and_the_original_assignment_identity() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    let cache = PluginCatalog::new(app.directory.path().join("cache"));
    let plugin = cache
        .scan(&[fixture.path().to_owned()])
        .unwrap()
        .into_iter()
        .find(|entry| entry.plugin.name == "Rill Fixture A")
        .unwrap()
        .plugin;
    cache
        .capture(&plugin, |_, _| anyhow::bail!(CAPTURE_ERROR))
        .unwrap();
    let baseline = app.inspect();
    common::refresh_library(&mut app);
    common::wait_library_device(&mut app, "Rill Fixture A");
    app.click("Rill Fixture A");
    let opened = app.inspect();
    preserved(&opened, &baseline);
    assert!(
        entry(&opened)["status"]
            .as_str()
            .unwrap()
            .contains(CAPTURE_ERROR)
    );
    assert!(entry(&opened)["thumbnail"].is_null());
    assert!(visible(&opened, "Without preview"));
    assert!(!visible(&opened, "Preview failed"));

    // Missing and failed images belong to a neutral collection, not a list of
    // broken instruments. An empty image collection is not an empty inventory.
    let without = app.click("Without preview");
    assert_eq!(without["library"]["total"], 2);
    preserved(&without, &baseline);
    let with = app.click("With preview");
    assert_eq!(with["library"]["total"], 0);
    assert!(visible(&with, "No matching instruments"));
    assert!(!visible(&with, "No VST3 instruments found"));
    preserved(&with, &baseline);
    app.click("All plug-ins");
    app.click("Rill Fixture A");

    // The full underlying capture error remains available in the scrollable
    // details pane, alongside language that makes no audio-readiness promise.
    let details = app.inspect()["library"]["details"].clone();
    let state = app.command(json!({
        "command":"scroll",
        "x":(details["min"][0].as_f64().unwrap() + details["max"][0].as_f64().unwrap()) * 0.5,
        "y":(details["min"][1].as_f64().unwrap() + details["max"][1].as_f64().unwrap()) * 0.5,
        "delta":-500.0
    }));
    assert!(visible(&state, "Preview image"));
    assert!(visible(&state, "Unavailable"));
    assert!(visible(
        &state,
        "Optional image; assignment remains available."
    ));
    assert!(visible(&state, CAPTURE_ERROR));
    preserved(&state, &baseline);

    let more = app.click("More");
    assert!(visible(&more, "Image unavailable"));
    assert!(visible(&more, "Capture preview"));
    preserved(&more, &baseline);
    app.click("Capture preview");
    let retried = common::wait_until(&mut app, |state| {
        let status = entry(state)["status"].as_str().unwrap();
        status.starts_with("Failed") && !status.contains(CAPTURE_ERROR)
    });
    preserved(&retried, &baseline);
    assert_eq!(entry(&retried)["class_id"], json!(plugin.class_id));
    assert!(entry(&retried)["thumbnail"].is_null());
    app.key("Escape");

    app.click("Track");
    app.click("Add MIDI track");
    common::wait_library_device(&mut app, "Rill Fixture A");
    app.click("Rill Fixture A");
    let caller = app.inspect();
    assert!(visible(&caller, "Assign instrument"));
    let assigned = app.click("Assign instrument");
    assert_eq!(assigned["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(
        assigned["project"]["tracks"][0]["id"],
        caller["project"]["tracks"][0]["id"]
    );
    assert_eq!(
        assigned["project"]["tracks"][0]["clips"],
        caller["project"]["tracks"][0]["clips"]
    );
    assert_eq!(
        assigned["project"]["tracks"][0]["instrument"]["Vst3"],
        json!({"path":plugin.path, "class_id":plugin.class_id})
    );
    assert_eq!(assigned["popups"]["library"], false);
    assert_eq!(assigned["native_editor"]["open"], false);
    // Assignment must not turn the failed image into a false success status.
    assert!(
        entry(&assigned)["status"]
            .as_str()
            .unwrap()
            .starts_with("Failed")
    );
    app.quit();
}
