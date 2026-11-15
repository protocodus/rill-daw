//! Audition and explicit sample use stay separate across asynchronous work.
mod common;

use common::Harness;
use daw_core::{AudioData, Instrument, Project};
use serde_json::{Value, json};
use std::path::Path;

fn tone(path: &Path, frequency: f32) {
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..48_000)
            .map(|frame| 0.25 * (std::f32::consts::TAU * frequency * frame as f32 / 48_000.0).sin())
            .collect(),
    }
    .write_wav(path)
    .unwrap();
}

fn browser(app: &mut Harness) {
    app.click("Window");
    app.click("Device browser");
    app.click("Samples");
}

fn configure_root(app: &mut Harness, root: &Path, count: usize) {
    app.click("Folders…");
    app.click("Sample folder path");
    app.command(json!({"command":"text","text":root}));
    let added = app.click("Add folder");
    assert_eq!(added["library"]["indexed"], 0);
    assert_eq!(added["library"]["scanning"], false);
    app.click("Done");
    app.click("Scan folders");
    common::wait_until(app, |state| {
        state["library"]["indexed"] == count && state["library"]["scanning"] == false
    });
}

fn sample(state: &Value, track: u64) -> Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == track)
        .unwrap()["instrument"]["Builtin"]["samples"]
        .as_array()
        .and_then(|samples| samples.first())
        .cloned()
        .unwrap_or(Value::Null)
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "revision", "modified"] {
        assert_eq!(
            state[field], before[field],
            "Sample browsing changed {field}"
        );
    }
}

fn seed_sampler(app: &mut Harness, pooled: Option<&Path>) -> u64 {
    let mut project = Project::new();
    let track = project.add_instrument_track("Destination");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    if let Some(path) = pooled {
        project
            .add_project_sample("Pool tone", path.to_string_lossy())
            .unwrap();
    }
    let song = app.directory.path().join("sample-browser.rill");
    project.save(&song).unwrap();
    app.command(json!({"command":"open","path":song}));
    track
}

fn captured_browser(app: &mut Harness) {
    app.click("Window");
    app.click("Instrument");
    app.click("Browse…");
    assert_eq!(app.inspect()["library"]["captured"], true);
}

#[cfg(unix)]
fn decoder_gate() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    use std::os::unix::fs::PermissionsExt;
    let gate = tempfile::tempdir().unwrap();
    let release = gate.path().join("release");
    let started = gate.path().join("started");
    let helper = gate.path().join("decoder");
    let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
    let binary =
        std::env::var_os("RILL_E2E_BINARY").unwrap_or_else(|| env!("CARGO_BIN_EXE_rill").into());
    std::fs::write(
        &helper,
        format!(
            "#!/bin/sh\ntouch {}\nwhile [ ! -f {} ]; do sleep 0.01; done\nexec {} \"$@\"\n",
            quote(&started),
            quote(&release),
            quote(Path::new(&binary))
        ),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    (gate, helper, started, release)
}

#[cfg(unix)]
#[test]
fn captured_load_waits_for_commit_and_preserves_selection_on_cancel_or_failure() {
    let (_gate, helper, started, release) = decoder_gate();
    let mut app = Harness::with_audio_import_helper(&helper, 20_000);
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    let track = seed_sampler(&mut app, None);
    let root = app.directory.path().join("Library");
    std::fs::create_dir(&root).unwrap();
    let source = root.join("Waiting tone.wav");
    tone(&source, 440.0);
    captured_browser(&mut app);
    configure_root(&mut app, &root, 1);
    click_result(&mut app, &source, "click");
    let before = app.inspect();
    app.click("Load sample");
    let waiting = common::wait_until(&mut app, |state| {
        state["audio_import"]["busy"] == true && started.exists()
    });
    assert_eq!(waiting["popups"]["library"], true);
    assert!(waiting["library"]["pending_import"].is_u64());
    assert_eq!(
        waiting["library"]["selected"],
        before["library"]["selected"]
    );
    unchanged(&waiting, &before);
    app.click("Cancel import");
    let canceled = common::wait_until(&mut app, |state| {
        state["audio_import"]["busy"] == false && state["library"]["pending_import"].is_null()
    });
    assert_eq!(canceled["popups"]["library"], true);
    assert_eq!(
        canceled["audio_import"]["latest_outcome"]["state"],
        "cancelled"
    );
    assert_eq!(
        canceled["library"]["selected"],
        before["library"]["selected"]
    );
    unchanged(&canceled, &before);

    // Failure after admission must also leave the picker and its selection intact.
    std::fs::remove_file(&source).unwrap();
    std::fs::write(&release, b"ready").unwrap();
    app.click("Load sample");
    let failed = common::wait_until(&mut app, |state| {
        state["audio_import"]["latest_outcome"]["state"] == "failed"
            && state["library"]["pending_import"].is_null()
    });
    assert_eq!(failed["popups"]["library"], true);
    assert_eq!(failed["library"]["selected"], before["library"]["selected"]);
    assert_eq!(failed["library"]["notice"][1], true);
    unchanged(&failed, &before);

    // Repairing the selected source needs neither a scan nor a new picker.
    tone(&source, 440.0);
    app.click("Load sample");
    let loaded = common::wait_until(&mut app, |state| {
        sample(state, track)["name"] == "Waiting tone" && state["popups"]["library"] == false
    });
    assert_eq!(loaded["popups"]["instrument"], true);
    assert_eq!(
        loaded["audio_import"]["latest_outcome"]["state"],
        "succeeded"
    );
    assert_eq!(
        loaded["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert_eq!(loaded["project"]["samples"], before["project"]["samples"]);
    assert_ne!(
        sample(&loaded, track)["path"],
        source.to_string_lossy().as_ref()
    );

    // A successful late commit cannot navigate a user who already left the picker.
    std::fs::remove_file(&release).unwrap();
    std::fs::remove_file(&started).unwrap();
    captured_browser(&mut app);
    app.click("Load sample");
    common::wait_until(&mut app, |state| {
        state["audio_import"]["busy"] == true && started.exists()
    });
    app.click("Back");
    app.click("Window");
    app.click("Device browser");
    app.click("Instruments");
    let away = app.inspect();
    assert_eq!(away["popups"]["instrument"], false);
    std::fs::write(&release, b"ready").unwrap();
    let finished_away =
        common::wait_until(&mut app, |state| state["audio_import"]["busy"] == false);
    assert_eq!(
        finished_away["audio_import"]["latest_outcome"]["state"],
        "succeeded"
    );
    assert_eq!(finished_away["popups"]["instrument"], false);
    assert_eq!(finished_away["popups"]["library"], true);
    assert_eq!(finished_away["library"]["view"], away["library"]["view"]);
    app.quit();
}

#[test]
fn ordinary_managed_load_confirms_success_without_leaving_the_browser() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    let source = app.directory.path().join("tone.wav");
    tone(&source, 440.0);
    let track = seed_sampler(&mut app, Some(&source));
    browser(&mut app);
    let id = app.inspect()["library"]["samples"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    click_result_key(&mut app, &id, "click");
    let before = app.inspect();
    assert_eq!(before["library"]["captured"], false);
    app.click("Use…");
    app.click("Load into instrument…");
    app.click("Load sample");
    let loaded = common::wait_until(&mut app, |state| {
        sample(state, track)["name"] == "Pool tone" && state["library"]["pending_import"].is_null()
    });
    assert_eq!(loaded["popups"]["library"], true);
    assert_eq!(loaded["library"]["captured"], false);
    assert_eq!(loaded["library"]["selected"], before["library"]["selected"]);
    assert_eq!(loaded["library"]["notice"][1], false);
    assert_eq!(
        loaded["audio_import"]["latest_outcome"]["state"],
        "succeeded"
    );
    assert_eq!(loaded["project"]["samples"], before["project"]["samples"]);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert!(sample(&undone, track).is_null());
    assert_eq!(undone["popups"]["library"], true);
    app.quit();
}

fn click_result(app: &mut Harness, id: &Path, phase: &str) -> Value {
    click_result_key(app, id.to_str().unwrap(), phase)
}

fn click_result_key(app: &mut Harness, id: &str, phase: &str) -> Value {
    let state = app.inspect();
    let row = state["library"]["samples"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("Missing result {}: {}", id, state["library"]));
    app.command(json!({"command":"pointer","phase":phase,
        "x": row["min"][0].as_f64().unwrap() + 80.0,
        "y": (row["min"][1].as_f64().unwrap() + row["max"][1].as_f64().unwrap()) / 2.0}))
}

#[cfg(unix)]
#[test]
fn loading_preview_can_be_stopped_before_the_decoder_finishes() {
    let (_gate, helper, started, release) = decoder_gate();
    let mut app = Harness::with_audio_import_helper_and_virtual_audio(&helper, 20_000);
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    let root = app.directory.path().join("Library");
    std::fs::create_dir(&root).unwrap();
    let source = root.join("Slow preview.wav");
    tone(&source, 440.0);
    browser(&mut app);
    configure_root(&mut app, &root, 1);
    click_result(&mut app, &source, "click");
    let before = app.inspect();
    app.click("Preview");
    let loading = common::wait_until(&mut app, |state| {
        state["library"]["audition"]["state"] == "loading" && started.exists()
    });
    assert_eq!(
        loading["library"]["audition"]["path"],
        source.to_string_lossy().as_ref()
    );
    unchanged(&loading, &before);
    app.click("Stop preview");
    std::fs::write(&release, b"ready").unwrap();
    let stopped = common::wait_until(&mut app, |state| {
        state["library"]["audition"]["state"] == "idle"
    });
    assert_eq!(
        stopped["library"]["selected"],
        source.to_string_lossy().as_ref()
    );
    let output = app.command(json!({"command":"pump_audio","frames":4096}));
    assert_eq!(output["engine"]["sample_audition_active"], false);
    assert!(
        output["audio_output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| frame[0].as_f64().unwrap().abs() < 0.00001)
    );
    unchanged(&output, &before);
    // Retry starts a fresh generation after the cancelled decoder is reaped.
    app.click("Preview");
    common::wait_until(&mut app, |state| {
        state["library"]["audition"]["state"] == "playing"
    });
    app.quit();
}

#[test]
fn focused_preview_keys_and_metadata_retry_never_edit_the_song_or_scan() {
    let mut app = Harness::with_virtual_audio(None, None);
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    seed_sampler(&mut app, None);
    let root = app.directory.path().join("Library");
    std::fs::create_dir(&root).unwrap();
    let first = root.join("A tone.wav");
    let second = root.join("B tone.wav");
    let broken = root.join("C repaired.wav");
    tone(&first, 440.0);
    tone(&second, 880.0);
    std::fs::write(&broken, b"not an audio file").unwrap();
    browser(&mut app);
    configure_root(&mut app, &root, 3);
    app.command(
        json!({"command":"song_tool","name":"transport","arguments":{"action":"seek","beat":7.5}}),
    );
    let before = app.inspect();
    click_result(&mut app, &first, "click");
    let previewed = app.key("Enter");
    assert!(
        matches!(
            previewed["library"]["audition"]["state"].as_str(),
            Some("loading" | "playing")
        ),
        "Focused Enter did not start preview: {} / {} / {}",
        previewed["library"],
        previewed["engine"],
        previewed["status"]
    );
    let playing = common::wait_until(&mut app, |state| {
        state["library"]["audition"]["state"] == "playing"
    });
    assert_eq!(
        playing["library"]["selected"],
        first.to_string_lossy().as_ref()
    );
    unchanged(&playing, &before);
    app.key("Space");
    let stopped = app.command(json!({"command":"pump_audio","frames":1024}));
    assert_eq!(stopped["library"]["audition"]["state"], "idle");
    assert_eq!(stopped["engine"]["playing"], false);
    assert_eq!(stopped["engine"]["position_beats"], 7.5);
    let next = app.key("ArrowDown");
    assert_eq!(
        next["library"]["selected"],
        second.to_string_lossy().as_ref()
    );
    app.key("Enter");
    common::wait_until(&mut app, |state| {
        state["library"]["audition"]["state"] == "playing"
    });
    unchanged(&app.inspect(), &before);
    app.click("Stop preview");

    // Text-entry focus owns its spaces and Enter; it cannot audition or load.
    app.click("Search samples");
    app.command(json!({"command":"text","text":"tone"}));
    app.key("Space");
    let typed = app.key("Enter");
    assert_eq!(typed["library"]["audition"]["state"], "idle");
    assert_eq!(typed["popups"]["library"], true);
    assert_eq!(typed["engine"]["playing"], false);
    app.click("Clear search");

    click_result(&mut app, &broken, "click");
    common::wait_until(&mut app, |state| {
        state["library"]["metadata"]["state"] == "failed"
    });
    app.click("Preview");
    let failed = common::wait_until(&mut app, |state| {
        state["library"]["audition"]["state"] == "failed"
    });
    assert_eq!(
        failed["library"]["selected"],
        broken.to_string_lossy().as_ref()
    );
    assert!(
        failed["library"]["audition"]["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty())
    );
    assert_eq!(failed["library"]["scanning"], false);
    tone(&broken, 660.0);
    app.click("Retry details");
    let repaired = common::wait_until(&mut app, |state| {
        state["library"]["metadata"]["state"] == "ready"
    });
    assert_eq!(repaired["library"]["indexed"], 3);
    assert_eq!(repaired["library"]["scanning"], false);
    app.click("Preview");
    common::wait_until(&mut app, |state| {
        state["library"]["audition"]["state"] == "playing"
    });
    let final_state = app.command(json!({"command":"pump_audio","frames":4096}));
    assert!(
        final_state["audio_output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|frame| frame[0].as_f64().unwrap().abs() > 0.1)
    );
    unchanged(&final_state, &before);
    assert_eq!(final_state["engine"]["position_beats"], 7.5);
    assert_eq!(final_state["engine"]["playing"], false);
    app.click("Stop preview");
    let replayed = click_result(&mut app, &first, "double_click");
    assert!(
        matches!(
            replayed["library"]["audition"]["state"].as_str(),
            Some("loading" | "playing")
        ),
        "Double click did not start preview: {}",
        replayed["library"]
    );
    unchanged(&replayed, &before);
    app.quit();
}
