//! Real portal/fallback calls from the app, with isolated child helper processes.
#![cfg(target_os = "linux")]
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};

fn menu(app: &mut Harness, group: &str, item: &str) -> Value {
    app.click(group);
    app.click(item)
}

fn document_action(app: &mut Harness, action: &str) -> Value {
    let state = menu(app, "File", action);
    if action == "Export WAV…" {
        app.click("Export…")
    } else {
        state
    }
}

fn fixture(app: &mut Harness) -> Value {
    let mut project = Project::new();
    let midi = project.add_midi_track("Picker test notes");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Phrase",
        0.0,
        2.0,
        vec![Note::new(60, 0.0, 1.0, 90)],
    ));
    let path = app.directory.path().join("Original.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    menu(app, "Track", "Add audio track")
}

fn helper(app: &Harness, body: &str) {
    let path = app.directory.path().join("picker-bin/zenity");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn select(app: &Harness, path: &Path) {
    std::fs::write(
        app.directory.path().join("picker-bin/selection"),
        format!("{}\n", path.display()),
    )
    .unwrap();
    helper(
        app,
        "IFS= read -r selection < \"${0%/*}/selection\"\nprintf '%s\\n' \"$selection\"\nexit 0",
    );
}

fn unchanged(before: &Value, after: &Value) {
    for key in [
        "project",
        "project_path",
        "modified",
        "history",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(after[key], before[key], "Changed {key}: {after}");
    }
    assert_eq!(after["project_pending"], false);
    assert_eq!(after["export_pending"], false);
}

fn wait_guard(app: &mut Harness, phase: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if state["document_guard"]["phase"] == phase {
            return state;
        }
        assert!(Instant::now() < deadline, "{state}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn missing_native_helper_is_visible_for_every_document_picker_and_preserves_work() {
    let mut app = Harness::with_isolated_file_picker();
    let before = fixture(&mut app);
    for action in [
        "Save as…",
        "Open…",
        "Import…",
        "Export WAV…",
        "Export MIDI…",
    ] {
        let failed = document_action(&mut app, action);
        assert_eq!(failed["error"], true, "{action}: {failed}");
        assert!(
            failed["status"]
                .as_str()
                .unwrap()
                .contains("Could not open the file picker")
        );
        unchanged(&before, &failed);
        let details = app.click("Details");
        let diagnostic = details["error_details"]["details"].as_str().unwrap();
        assert!(
            diagnostic.contains("Desktop file picker unavailable"),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("Zenity fallback failed"),
            "{diagnostic}"
        );
        assert!(diagnostic.contains("PATH"), "{diagnostic}");
        assert_eq!(app.click("Copy details")["clipboard_text"], diagnostic);
        app.click("Close");
        app.click("Dismiss");
    }
    app.quit();
}

#[test]
fn native_cancel_stays_silent_and_failed_helper_can_be_restored_without_restarting() {
    let mut app = Harness::with_isolated_file_picker();
    let before = fixture(&mut app);
    helper(&app, "printf 'GTK warning\\n' >&2\nexit 1");
    for action in [
        "Save as…",
        "Open…",
        "Import…",
        "Export WAV…",
        "Export MIDI…",
    ] {
        let cancelled = document_action(&mut app, action);
        assert_eq!(cancelled["error"], false, "{action}: {cancelled}");
        assert!(cancelled["last_error_report"].is_null());
        unchanged(&before, &cancelled);
    }

    let rejected = app.directory.path().join("Must not write.rill");
    // A broken helper's stdout must never authorize a save despite its failure.
    select(&app, &rejected);
    helper(
        &app,
        "IFS= read -r selection < \"${0%/*}/selection\"\nprintf '%s\\n' \"$selection\"\nprintf 'picker backend failed\\n' >&2\nexit 255",
    );
    let failed = document_action(&mut app, "Save as…");
    assert_eq!(failed["error"], true);
    assert!(
        failed["status"]
            .as_str()
            .unwrap()
            .contains("picker backend failed")
    );
    unchanged(&before, &failed);
    assert!(!rejected.exists());
    app.click("Dismiss");

    let saved = app.directory.path().join("Recovered 音楽.rill");
    select(&app, &saved);
    document_action(&mut app, "Save as…");
    let state = app.command(json!({"command":"wait_project"}));
    assert_eq!(state["project_path"], json!(saved));
    assert_eq!(state["error"], false);
    assert_eq!(Project::load(&saved).unwrap().tracks.len(), 2);

    let wav = app.directory.path().join("Recovered.wav");
    select(&app, &wav);
    document_action(&mut app, "Export WAV…");
    let state = app.command(json!({"command":"wait_export"}));
    assert_eq!(state["exported_file"], json!(wav));
    assert!(AudioData::load_wav(&wav).unwrap().frames() > 0);
    let midi = app.directory.path().join("Recovered.mid");
    select(&app, &midi);
    document_action(&mut app, "Export MIDI…");
    assert!(std::fs::metadata(&midi).unwrap().len() > 0);
    app.quit();
}

#[test]
fn unsaved_document_guard_retains_picker_failure_and_retries_the_save() {
    let mut app = Harness::with_isolated_file_picker();
    let before = menu(&mut app, "Track", "Add audio track");
    menu(&mut app, "File", "New");
    wait_guard(&mut app, "confirm");
    let failed = app.click("Save");
    assert_eq!(failed["document_guard"]["phase"], "confirm");
    assert!(
        failed["document_guard"]["error"]
            .as_str()
            .unwrap()
            .contains("Could not open the file picker")
    );
    unchanged(&before, &failed);
    let details = app.click("Details");
    assert!(
        details["error_details"]["details"]
            .as_str()
            .unwrap()
            .contains("Zenity fallback failed")
    );
    app.click("Close");

    let saved = app.directory.path().join("Retained work.rill");
    select(&app, &saved);
    app.click("Save");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if state["document_guard"].is_null() && state["project_pending"] == false {
            assert!(state["project"]["tracks"].as_array().unwrap().is_empty());
            break;
        }
        assert!(Instant::now() < deadline, "{state}");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(Project::load(saved).unwrap().tracks.len(), 1);
    app.quit();
}
