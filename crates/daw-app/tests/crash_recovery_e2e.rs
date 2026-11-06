//! Recovery is written by real commits, independently of graceful shutdown.
mod common;
use common::Harness;
use daw_core::Project;
use serde_json::json;

fn add_track(app: &mut Harness) {
    app.click("Track");
    app.click("Add audio track");
    app.command(json!({"command":"wait_project"}));
}

#[test]
fn committed_edits_survive_a_hard_kill_without_a_save_or_quit() {
    let mut app = Harness::new();
    add_track(&mut app);
    add_track(&mut app);
    let expected = app.inspect()["project"].clone();
    let recovery = app.directory.path().join("scratch/recovery.daw.json");
    let previous = app
        .directory
        .path()
        .join("scratch/recovery.previous.daw.json");
    assert_eq!(Project::load(&recovery).unwrap().tracks.len(), 2);
    assert_eq!(Project::load(&previous).unwrap().tracks.len(), 1);

    let mut app = app.crash_and_restart();
    app.click("File");
    app.click("Recover last session…");
    let restored = app.command(json!({"command":"wait_project"}));
    assert_eq!(restored["project"], expected);
    assert_eq!(restored["modified"], true);
    assert_eq!(restored["project_path"], serde_json::Value::Null);
    // Recovery remains an editable document, with ordinary explicit Save.
    let saved = app.directory.path().join("recovered.rill");
    app.command(json!({"command":"save","path":saved}));
    assert_eq!(Project::load(saved).unwrap().tracks.len(), 2);
    app.quit();
}

#[test]
fn recovery_write_failure_retains_document_and_allows_explicit_save() {
    let mut app = Harness::new();
    // An existing directory at the atomic replacement destination reliably
    // fails even under a privileged test user; no filesystem permissions trick.
    let recovery = app.directory.path().join("scratch/recovery.daw.json");
    std::fs::create_dir_all(&recovery).unwrap();
    app.click("Track");
    app.click("Add audio track");
    let failed = app.raw_command(json!({"command":"wait_project"}));
    assert_eq!(failed["ok"], false);
    assert_eq!(
        failed["state"]["project"]["tracks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(failed["state"]["modified"], true);
    assert!(
        failed["state"]["status"]
            .as_str()
            .unwrap()
            .contains("Recovery could not be saved")
    );
    std::fs::remove_dir(recovery).unwrap();
    let saved = app.directory.path().join("safe.rill");
    app.command(json!({"command":"save","path":saved}));
    assert_eq!(Project::load(saved).unwrap().tracks.len(), 1);
    app.quit();
}

#[test]
fn corrupt_latest_recovery_falls_back_to_the_previous_complete_checkpoint() {
    let mut app = Harness::new();
    add_track(&mut app);
    add_track(&mut app);
    let mut app = app.crash_and_restart();
    std::fs::write(
        app.directory.path().join("scratch/recovery.daw.json"),
        b"damaged",
    )
    .unwrap();
    app.click("File");
    app.click("Recover last session…");
    let restored = app.command(json!({"command":"wait_project"}));
    assert_eq!(restored["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(restored["modified"], true);
    assert_eq!(restored["error"], false);
    assert!(
        restored["status"]
            .as_str()
            .unwrap()
            .contains("Previous recovery opened")
    );
    assert_eq!(
        Project::load(
            app.directory
                .path()
                .join("scratch/recovery.previous.daw.json")
        )
        .unwrap()
        .tracks
        .len(),
        1
    );
    app.quit();
}
