//! The running catalogue reads real native I/O and parameters through the
//! production isolated host, without constructing an editor or a project device.
mod common;
#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::{Harness, wait_until};
use editor_probe_fixture::EditorProbeFixture;
use serde_json::{Value, json};
use std::path::PathBuf;

fn production_helper() -> PathBuf {
    let directory = if let Some(binary) = std::env::var_os("RILL_E2E_BINARY") {
        PathBuf::from(binary).parent().unwrap().to_owned()
    } else {
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned()
    };
    let path = directory.join(if cfg!(windows) {
        "daw-plugin-host.exe"
    } else {
        "daw-plugin-host"
    });
    assert!(
        path.is_file(),
        "Build workspace binaries before testing: {}",
        path.display()
    );
    path
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn visible_buttons(state: &Value) {
    for label in ["Close", "Copy details", "Technical details"] {
        let rect = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == label)
            .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
        assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{rect}");
        assert!(rect["min"][1].as_f64().unwrap() >= 0.0, "{rect}");
        assert!(rect["max"][0].as_f64().unwrap() <= 800.0, "{rect}");
        assert!(rect["max"][1].as_f64().unwrap() <= 600.0, "{rect}");
    }
}

#[test]
fn device_details_read_native_ports_search_parameters_and_preserve_project() {
    let fixture = EditorProbeFixture::build();
    for dark in [false, true] {
        let mut app =
            Harness::with_plugin_host(fixture.path.parent().unwrap(), &production_helper());
        app.command(json!({"command":"viewport", "width":800, "height":600}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.click("Window");
        app.click("Device browser");
        common::wait_library_device(&mut app, "No Editor Probe");
        let before = app.inspect();
        app.click("Device details");
        let ready = wait_until(&mut app, |state| {
            state["plugin_details"]["status"] == "ready"
        });
        let metadata = &ready["plugin_details"]["metadata"];
        assert_eq!(metadata["inputs"], json!([]));
        assert_eq!(metadata["outputs"][0]["channels"], 2);
        assert_eq!(metadata["midi_input"], true);
        assert_eq!(metadata["midi_output"], false);
        assert_eq!(metadata["parameters"].as_array().unwrap().len(), 1);
        assert_eq!(metadata["parameters"][0]["name"], "Level");
        assert_eq!(metadata["parameters"][0]["can_automate"], true);
        assert!(visible(&ready, "0 audio · MIDI"));
        assert!(visible(&ready, "2 audio"));
        assert!(visible(&ready, "Level"));
        visible_buttons(&ready);
        assert_eq!(ready["native_editor"]["open"], false);
        assert_eq!(ready["device_open"], false);
        assert_eq!(ready["project"], before["project"]);

        app.click("Search device parameters");
        let searched = app.command(json!({"command":"text", "text":"absent"}));
        assert!(visible(&searched, "No matching parameters"));
        assert!(!visible(&searched, "Level"));
        let cleared = app.click("Clear");
        assert!(visible(&cleared, "Level"));
        assert_eq!(cleared["plugin_details"]["query"], "");
        app.click("Search device parameters");
        let found = app.command(json!({"command":"text", "text":"%"}));
        assert!(visible(&found, "Level"));
        let copied = app.click("Copy details");
        let report = copied["clipboard_text"].as_str().unwrap();
        assert!(report.contains("No Editor Probe"));
        assert!(report.contains("Outputs: 2 audio channels, 1 buses"));
        assert!(report.contains("#7 Level [%]"));
        assert!(report.contains(fixture.path.to_str().unwrap()));
        app.click("Technical details");
        assert!(visible(&app.inspect(), "Parameters"));
        app.click("Parameters");
        for key in ["Delete", "Space"] {
            let blocked = app.key(key);
            assert_eq!(blocked["project"], before["project"]);
            assert_eq!(blocked["history"], before["history"]);
        }
        let blocked =
            app.command(json!({"command":"key", "key":"N", "cmd":true, "release_modifiers":true}));
        assert_eq!(blocked["song_session"], before["song_session"]);
        assert!(blocked["document_guard"].is_null());
        assert_eq!(blocked["popups"]["library"], true);
        let closed = app.click("Close");
        assert!(closed["plugin_details"].is_null());
        assert_eq!(closed["project"], before["project"]);
        app.click("Device details");
        assert_eq!(app.inspect()["plugin_details"]["status"], "ready");
        let escaped = app.key("Escape");
        assert!(escaped["plugin_details"].is_null());
        assert_eq!(escaped["popups"]["library"], true);
        assert_eq!(
            escaped["library"]["selected"],
            before["library"]["selected"]
        );
        app.quit();
    }
}

#[test]
fn unavailable_device_details_remain_unknown_and_can_retry_or_close() {
    let plugins = common::plugin_fixture();
    let mut app = Harness::with_plugin_host(plugins.path(), &production_helper());
    app.command(json!({"command":"viewport", "width":800, "height":600}));
    common::fixture_library(&mut app);
    let before = app.inspect();
    app.click("Device details");
    let failed = wait_until(&mut app, |state| {
        state["plugin_details"]["status"] == "failed"
    });
    assert!(visible(&failed, "Details unavailable"));
    assert!(failed["plugin_details"]["metadata"].is_null());
    assert_eq!(failed["project"], before["project"]);
    visible_buttons(&failed);
    app.click("Retry details");
    wait_until(&mut app, |state| {
        state["plugin_details"]["status"] == "failed"
    });
    let closed = app.click("Close");
    assert!(closed["plugin_details"].is_null());
    assert_eq!(closed["project"], before["project"]);
    app.quit();
}
