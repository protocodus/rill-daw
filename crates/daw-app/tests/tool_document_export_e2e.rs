//! Tool-owned document switches report export failure without a GUI-only decision.
mod common;
#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::Harness;
use daw_core::{Clip, Instrument, Note, Project};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

fn native_helper() -> PathBuf {
    let directory = std::env::var_os("RILL_E2E_BINARY")
        .map(|path| PathBuf::from(path).parent().unwrap().to_owned())
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_owned()
        });
    directory.join(if cfg!(windows) {
        "daw-plugin-host.exe"
    } else {
        "daw-plugin-host"
    })
}

fn application(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"application_tool","name":name,"arguments":arguments}))
}

#[test]
fn failed_export_aborts_tool_owned_new_and_open_without_a_gui_guard_or_lost_error_causes() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_millis(1200));
    for (action, discard) in [("new", false), ("open", true)] {
        let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &native_helper());
        let mut project = Project::new();
        let track = project.add_midi_track("Patient instrument");
        project
            .set_instrument(
                track,
                Instrument::Vst3 {
                    path: plugin.path.to_string_lossy().into_owned(),
                    class_id: Some(CLASS_ID.into()),
                },
            )
            .unwrap();
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            "Patient phrase",
            0.0,
            0.25,
            vec![Note::new(60, 0.0, 0.125, 100)],
        ));
        let saved = app.directory.path().join("Original song.rill");
        project.save(&saved).unwrap();
        app.command(json!({"command":"open","path":saved}));
        let original = app.inspect();
        let replacement = app.directory.path().join("Replacement song.rill");
        let mut next = Project::new();
        next.name = "Replacement song".into();
        next.save(&replacement).unwrap();

        // It renders slowly, then fails when the final WAV file is written.
        // A directory is not an output file on any platform.
        let output = app.directory.path().join("Invalid destination.wav");
        std::fs::create_dir(&output).unwrap();
        app.command(json!({"command":"file_dialog","kind":"export","path":output}));
        app.click("File");
        app.click("Export WAV…");
        app.click("Include effect tails");
        let started = app.click("Export…");
        assert_eq!(started["export_pending"], true);

        let failed = application(
            &mut app,
            "document_action",
            json!({"action":action,"path":replacement,"discard_unsaved":discard}),
        );
        assert_eq!(failed["tool_result"]["ok"], false, "{failed}");
        assert_eq!(failed["tool_result"]["error"]["code"], "operation_failed");
        assert!(failed["document_guard"].is_null(), "{failed}");
        assert_eq!(failed["project"], original["project"]);
        assert_eq!(failed["project_path"], original["project_path"]);
        assert_eq!(failed["export_run"]["failed"], true);
        assert!(output.is_dir());

        // The original diagnostic remains inspectable; clearing the tool guard
        // must not replace a detailed cause chain with its one-line summary.
        let details = app.click("Details");
        let diagnostic = details["error_details"]["details"].as_str().unwrap();
        assert!(diagnostic.contains("Caused by:"), "{diagnostic}");
        assert!(
            diagnostic.contains(output.to_string_lossy().as_ref()),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("Project: Original song"),
            "{diagnostic}"
        );
        app.click("Close");
        let status = application(&mut app, "document_status", json!({}));
        assert_eq!(status["tool_result"]["ok"], true);
        assert_eq!(status["tool_result"]["result"]["pending"], false);

        // A subsequent explicit tool action is possible without mouse input to
        // clear a hidden modal, and starts its own normal snapshot checks.
        let continued = application(
            &mut app,
            "document_action",
            json!({"action":action,"path":replacement,"discard_unsaved":discard}),
        );
        assert_eq!(continued["tool_result"]["ok"], true, "{continued}");
        assert!(continued["document_guard"].is_null());
        assert!(
            continued["project"]["tracks"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        if action == "open" {
            assert_eq!(continued["project_path"], json!(replacement));
        } else {
            assert!(continued["project_path"].is_null());
        }
        app.quit();
    }
}
