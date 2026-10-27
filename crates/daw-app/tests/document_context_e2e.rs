//! Saved-file identity and the next song's working folder survive real UI flows.
mod common;

use common::Harness;
use daw_core::{AudioData, Project};
use serde_json::{Value, json};
use std::path::Path;

fn menu(app: &mut Harness, group: &str, item: &str) -> Value {
    app.click(group);
    app.click(item)
}

fn save_as(app: &mut Harness, path: &Path) -> Value {
    app.command(json!({"command":"file_dialog","kind":"save","path":path}));
    menu(app, "File", "Save as…");
    app.command(json!({"command":"wait_project"}))
}

fn new_song(app: &mut Harness) -> Value {
    menu(app, "File", "New");
    app.command(json!({"command":"wait_project"}))
}

#[test]
fn new_song_remembers_the_successful_save_folder_across_cancel_and_restart() {
    let mut app = Harness::new();
    assert!(app.inspect()["file_dialog_defaults"]["directory"].is_null());
    let folder = app.directory.path().join("Songs 音楽 with spaces");
    std::fs::create_dir(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let first = folder.join("First melody.rill");
    menu(&mut app, "Track", "Add audio track");
    save_as(&mut app, &first);

    let next = new_song(&mut app);
    assert!(next["project_path"].is_null());
    assert_eq!(next["project_title"], "Untitled session");
    assert_eq!(next["file_dialog_defaults"]["directory"], json!(folder));
    assert_eq!(
        next["file_dialog_defaults"]["save"],
        "Untitled session.rill"
    );
    assert_eq!(next["settings"]["last_project_directory"], json!(folder));

    // Cancelling a fresh song's chooser must not forget or replace its folder.
    app.command(json!({"command":"file_dialog","kind":"save","path":null}));
    let cancelled = menu(&mut app, "File", "Save as…");
    assert!(cancelled["project_path"].is_null());
    assert_eq!(
        cancelled["file_dialog_defaults"]["directory"],
        json!(folder)
    );

    app = app.restart();
    assert_eq!(
        app.inspect()["file_dialog_defaults"]["directory"],
        json!(folder)
    );
    menu(&mut app, "Track", "Add audio track");
    let second = folder.join("Second melody");
    let saved = save_as(&mut app, &second);
    let second = second.with_extension("rill");
    assert_eq!(saved["project_path"], json!(second));
    assert_eq!(saved["project_title"], "Second melody");
    assert_eq!(saved["file_dialog_defaults"]["wav"], "Second melody.wav");
    assert_eq!(saved["file_dialog_defaults"]["directory"], json!(folder));
    assert_eq!(Project::load(&first).unwrap().tracks.len(), 1);
    assert_eq!(Project::load(&second).unwrap().tracks.len(), 1);
    app.quit();
}

#[test]
fn failed_saves_do_not_replace_the_working_folder_and_deleted_folders_are_ignored() {
    let mut app = Harness::new();
    let folder = app.directory.path().join("Good folder");
    std::fs::create_dir(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let first = folder.join("First.rill");
    menu(&mut app, "Track", "Add audio track");
    save_as(&mut app, &first);

    // A directory cannot be replaced by the project archive on any platform.
    let bad = app.directory.path().join("Other folder/Not a file.rill");
    std::fs::create_dir_all(&bad).unwrap();
    app.command(json!({"command":"file_dialog","kind":"save","path":bad}));
    menu(&mut app, "File", "Save as…");
    let failed = app.raw_command(json!({"command":"wait_project"}));
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["state"]["project_path"], json!(first));
    assert_eq!(
        failed["state"]["settings"]["last_project_directory"],
        json!(folder)
    );
    assert_eq!(
        failed["state"]["file_dialog_defaults"]["directory"],
        json!(folder)
    );

    // A later missing removable/project folder should use the native default,
    // while retaining unrelated valid settings rather than resetting setup.
    app.click("Dismiss");
    new_song(&mut app);
    std::fs::remove_file(&first).unwrap();
    std::fs::remove_dir(&folder).unwrap();
    let missing = app.inspect();
    assert!(missing["file_dialog_defaults"]["directory"].is_null());
    assert_eq!(missing["settings"]["sample_rate"], 48_000);
    app = app.restart();
    let restarted = app.inspect();
    assert!(restarted["file_dialog_defaults"]["directory"].is_null());
    assert_eq!(restarted["settings"]["sample_rate"], 48_000);
    app.quit();
}

#[test]
fn export_diagnostics_name_the_saved_file_and_keep_the_musical_title_secondary() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Phrase.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: vec![0.1; 4800],
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop","paths":[source],"beat":0.0}));
    let saved = app.directory.path().join("Recognizable 音楽 song.rill");
    save_as(&mut app, &saved);
    menu(&mut app, "Track", "Add audio track");
    assert_eq!(app.inspect()["project_title"], "Recognizable 音楽 song •");

    let bad_output = app.directory.path().join("Not a WAV file.wav");
    std::fs::create_dir(&bad_output).unwrap();
    app.command(json!({"command":"file_dialog","kind":"export","path":bad_output}));
    menu(&mut app, "File", "Export WAV…");
    app.click("Export…");
    let failed = app.raw_command(json!({"command":"wait_export"}));
    assert_eq!(failed["ok"], false);
    let opened = app.click("Details");
    let details = opened["error_details"]["details"].as_str().unwrap();
    assert!(
        details.contains("\nProject: Recognizable 音楽 song\n"),
        "{details}"
    );
    assert!(
        details.contains(&format!("\nProject file: {}\n", saved.display())),
        "{details}"
    );
    assert!(
        details.contains("\nSession title: Untitled session\n"),
        "{details}"
    );
    assert!(
        !details.contains("\nProject: Untitled session"),
        "{details}"
    );
    assert!(!details.contains("Recognizable 音楽 song •"), "{details}");
    let copied = app.click("Copy details");
    assert_eq!(copied["clipboard_text"], opened["error_details"]["details"]);
    app.click("Close");
    app.quit();
}
