//! Diagnostic reports are complete, copyable, and preserve the caller's context.
mod common;

use common::Harness;
use daw_core::AudioData;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

fn menu(app: &mut Harness, name: &str, item: &str) -> Value {
    app.click(name);
    app.click(item)
}

fn assert_buttons_visible(state: &Value, width: f64, height: f64) {
    for label in ["Copy details", "Close"] {
        let rect = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == label)
            .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
        assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
        assert!(rect["min"][1].as_f64().unwrap() >= 0.0);
        assert!(rect["max"][0].as_f64().unwrap() <= width);
        assert!(rect["max"][1].as_f64().unwrap() <= height);
    }
}

#[test]
fn export_error_details_preserve_causes_copy_and_block_background_edits() {
    for dark in [false, true] {
        let mut app = Harness::new();
        app.command(json!({"command":"viewport", "width":800, "height":600}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        let source = app.directory.path().join("Phrase.wav");
        AudioData {
            channels: 1,
            sample_rate: 48_000,
            samples: vec![0.125; 4800],
        }
        .write_wav(&source)
        .unwrap();
        app.command(json!({"command":"drop", "paths":[source], "beat":0.0}));
        let before = app.inspect();
        assert!(!before["selected_clip"].is_null());
        // An existing directory is an invalid output file on every platform.
        let destination = app
            .directory
            .path()
            .join("Exports 音楽 🎹/This is a directory.wav");
        std::fs::create_dir_all(&destination).unwrap();
        app.command(json!({"command":"export", "path":destination}));
        let export = app.raw_command(json!({"command":"wait_export"}));
        assert_eq!(export["ok"], false);
        let failed = app.inspect();
        assert_eq!(failed["error"], true);
        let opened = app.click("Details");
        let report = opened["error_details"].clone();
        assert_eq!(report["title"], "Error details");
        assert_eq!(report["summary"], "WAV export failed");
        assert!(report["background_shapes"].as_u64().unwrap() > 0);
        let details = report["details"].as_str().unwrap();
        for expected in [
            "Caused by:",
            "Exporting WAV to",
            "Exports 音楽 🎹",
            "RILL ",
            "Platform:",
            "Project:",
            "Audio:",
            "Output device:",
        ] {
            assert!(details.contains(expected), "Missing {expected}: {details}");
        }
        assert!(
            details.contains(destination.to_string_lossy().as_ref()),
            "{details}"
        );
        assert_buttons_visible(&opened, 800.0, 600.0);
        let copied = app.click("Copy details");
        assert_eq!(copied["clipboard_text"], report["details"]);
        for command in [
            json!({"command":"key", "key":"Delete"}),
            json!({"command":"key", "key":"Space"}),
            json!({"command":"key", "key":"Z", "cmd":true, "release_modifiers":true}),
        ] {
            let state = app.command(command);
            assert_eq!(state["project"], before["project"]);
            assert_eq!(state["history"], before["history"]);
            assert_eq!(state["error_details"], report);
            assert_eq!(state["device_open"], false);
        }
        // The driver cannot resolve an active arranger drop target behind a modal.
        let rejected =
            app.raw_command(json!({"command":"drop", "paths":[source], "track":0, "beat":0.0}));
        assert_eq!(rejected["ok"], false);
        assert_eq!(app.inspect()["project"], before["project"]);
        let escaped = app.key("Escape");
        assert!(escaped["error_details"].is_null());
        assert_eq!(escaped["view"], before["view"]);
        assert_eq!(escaped["error"], true);
        assert_eq!(escaped["project"], before["project"]);
        assert_eq!(
            app.click("Details")["error_details"]["details"],
            report["details"]
        );
        let closed = app.click("Close");
        assert!(closed["error_details"].is_null());
        assert_eq!(closed["history"], before["history"]);
        app.quit();
    }
}

fn crashing_editor_host(directory: &std::path::Path) -> PathBuf {
    let source = directory.join("crashing_editor.rs");
    std::fs::write(
        &source,
        r#"fn main() {
        let mut request = String::new();
        std::io::stdin().read_line(&mut request).unwrap();
        assert!(request.contains("\"command\":\"open\""));
        std::process::exit(19);
    }"#,
    )
    .unwrap();
    let executable = directory.join(if cfg!(windows) {
        "crashing-editor.exe"
    } else {
        "crashing-editor"
    });
    let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    executable
}

#[test]
fn editor_crash_details_identify_effect_channel_path_and_exit_code() {
    let exit_status = if cfg!(windows) {
        "exit code: 19"
    } else {
        "exit status: 19"
    };
    let plugins = common::plugin_fixture();
    let workers = tempfile::tempdir().unwrap();
    let host = crashing_editor_host(workers.path());
    let mut app = Harness::with_plugin_host(plugins.path(), &host);
    menu(&mut app, "Track", "Add audio track");
    menu(&mut app, "Window", "Effects");
    app.click("+ Insert");
    common::wait_visible(&mut app, "Search effects");
    app.click("Search effects");
    app.command(json!({"command":"text", "text":"Rill Fixture FX"}));
    common::wait_library_device(&mut app, "Rill Fixture FX");
    app.command(json!({"command":"click", "label":"Rill Fixture FX", "occurrence":1}));
    let inserted = app.click("Add to effect chain");
    let before = inserted["project"].clone();
    app.click("Open editor");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if !state["native_editor"]["error"].is_null() {
            assert!(
                state["native_editor"]["error"]
                    .as_str()
                    .unwrap()
                    .contains(exit_status),
                "{}",
                state["native_editor"]
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Missing editor failure: {}",
            state["native_editor"]
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let opened = app.click("Details");
    let report = opened["error_details"].clone();
    assert!(
        report["title"]
            .as_str()
            .unwrap()
            .contains("Rill Fixture FX")
    );
    assert!(report["background_shapes"].as_u64().unwrap() > 0);
    let details = report["details"].as_str().unwrap();
    for expected in [
        exit_status,
        "Last request: Open native editor",
        "Editor helper PID:",
        "Device: Rill Fixture FX",
        "Channel:",
        "Class ID:",
    ] {
        assert!(details.contains(expected), "Missing {expected}: {details}");
    }
    assert!(details.contains(before["tracks"][0]["name"].as_str().unwrap()));
    let plugin_path = before["tracks"][0]["inserts"][0]["effect"]["Vst3"]["path"]
        .as_str()
        .unwrap();
    // The scanner stores a canonical path. On Windows that can expand an 8.3
    // temporary-directory alias and add a verbatim prefix. Check identity first,
    // then require that exact project path in the human-readable report.
    assert_eq!(
        std::fs::canonicalize(plugin_path).unwrap(),
        plugins
            .path()
            .join("RillFixture.vst3")
            .canonicalize()
            .unwrap()
    );
    assert!(
        details.contains(plugin_path),
        "Missing plugin path {plugin_path}: {details}"
    );
    assert_eq!(
        app.click("Copy details")["clipboard_text"],
        report["details"]
    );
    let escaped = app.key("Escape");
    assert!(escaped["error_details"].is_null());
    assert_eq!(escaped["popups"]["instrument"], true);
    assert_eq!(escaped["project"], before);
    app.click("Details");
    let closed = app.click("Close");
    assert!(closed["error_details"].is_null());
    assert_eq!(closed["popups"]["instrument"], true);
    let chain = app.key("Escape");
    assert_eq!(chain["popups"]["effects"], true);
    assert_eq!(chain["project"], before);
    app.quit();
}
