//! Background project I/O uses the same document requests as File menus.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::path::Path;

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}
fn rename_track(app: &mut Harness, old: &str, new: &str) -> Value {
    menu(app, "Window", "Properties");
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == old)
        .count();
    app.command(json!({"command":"click","label":old,"occurrence":count-1}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let state = app.command(json!({"command":"text","text":new}));
    app.key("Escape");
    state
}
fn add_source(project: &mut Project, source: &Path) {
    let track = project.add_audio_track("Audio source");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Clip",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
}

#[test]
fn background_save_preserves_snapshot_and_keeps_later_cosmetic_edits_dirty() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add audio track");
    let path = app.directory.path().join("snapshot.rill");
    let started = app.command(json!({"command":"save","path":path,"wait":false}));
    assert_eq!(started["project_pending"], true);
    rename_track(&mut app, "New audio", "Renamed during save");
    let done = app.command(json!({"command":"wait_project"}));
    assert_eq!(done["project_pending"], false);
    assert_eq!(done["modified"], true);
    assert_eq!(Project::load(&path).unwrap().tracks[0].name, "New audio");
    assert_eq!(done["project"]["tracks"][0]["name"], "Renamed during save");
    let saved = app.command(json!({"command":"save","path":path}));
    assert_eq!(saved["modified"], false);
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][0]["name"],
        "Renamed during save"
    );
    app.quit();
}

#[test]
fn failed_save_and_failed_open_preserve_document_and_existing_bundle() {
    let mut app = Harness::new();
    let existing = app.directory.path().join("existing.rill");
    let mut good = Project::new();
    good.add_audio_track("Existing on disk");
    good.save(&existing).unwrap();
    let original = std::fs::read(&existing).unwrap();
    let source = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.125; 480],
    }
    .write_wav(&source)
    .unwrap();
    let mut broken = Project::new();
    add_source(&mut broken, &source);
    let seed = app.directory.path().join("current.json");
    broken.save(&seed).unwrap();
    app.command(json!({"command":"open","path":seed}));
    rename_track(&mut app, "Audio source", "Keep this work");
    std::fs::remove_file(&source).unwrap();
    let before = app.inspect()["project"].clone();
    app.command(json!({"command":"save","path":existing,"wait":false}));
    let failed = app.raw_command(json!({"command":"wait_project"}));
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["state"]["project"], before);
    assert_eq!(failed["state"]["modified"], true);
    assert_eq!(
        std::fs::read(&existing).unwrap(),
        original,
        "Atomic save must preserve prior bytes"
    );
    let invalid = app.directory.path().join("corrupt.rill");
    std::fs::write(&invalid, b"not an archive").unwrap();
    let failed = app.raw_command(json!({"command":"open","path":invalid}));
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["state"]["project"], before);
    assert_eq!(failed["state"]["project_path"], json!(seed));
    app.quit();
}

#[test]
fn quit_waits_for_background_bundle_save_and_open_prepares_waveforms() {
    let mut app = Harness::new();
    let external = tempfile::tempdir().unwrap();
    let source = external.path().join("tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..48_000)
            .map(|index| (index as f32 * 0.03).sin() * 0.2)
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    add_source(&mut project, &source);
    // Reused source data should decode once for multiple clips.
    let second = Clip::audio("Second", 3.0, 2.0, source.to_string_lossy());
    project.tracks[0].clips.push(second);
    let seed = external.path().join("source.rill");
    project.save(&seed).unwrap();
    std::fs::remove_file(&source).unwrap();
    let opened = app.command(json!({"command":"open","path":seed}));
    assert_eq!(opened["waveform_count"], 1);
    assert_eq!(opened["waveform_warning"], Value::Null);
    let output = external.path().join("saved-before-quit.rill");
    app.command(json!({"command":"save","path":output,"wait":false}));
    app.quit();
    let saved = Project::load(&output).unwrap();
    assert_eq!(saved.tracks[0].clips.len(), 2);
    let daw_core::ClipContent::Audio { path, .. } = &saved.tracks[0].clips[0].content else {
        panic!()
    };
    assert_eq!(AudioData::load_wav(path).unwrap().frames(), 48_000);
}

#[test]
fn declared_huge_waveform_is_skipped_without_allocating_it_or_losing_the_clip() {
    let mut app = Harness::new();
    let path = app.directory.path().join("huge-declaration.wav");
    let mut header = Vec::new();
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36u32 + 1024 * 1024 * 1024).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&48_000u32.to_le_bytes());
    header.extend_from_slice(&48_000u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&8u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&(1024u32 * 1024 * 1024).to_le_bytes());
    std::fs::write(&path, header).unwrap();
    let mut project = Project::new();
    add_source(&mut project, &path);
    let bundle = app.directory.path().join("bounded.rill");
    project.save(&bundle).unwrap();
    let state = app.command(json!({"command":"open","path":bundle}));
    assert_eq!(state["waveform_count"], 0);
    assert!(
        state["waveform_warning"]
            .as_str()
            .unwrap()
            .contains("1 waveform")
    );
    assert_eq!(
        state["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(state["error"], false);
    app.quit();
}

/// An OS pipe holds a tiny source file open. This proves the real UI remains
/// responsive during blocked filesystem I/O without giant files or test sleeps.
#[cfg(unix)]
#[test]
fn blocked_load_keeps_ui_responsive_and_serializes_following_save_and_new() {
    use std::{
        io::Write,
        sync::mpsc,
        time::{Duration, Instant},
    };
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add audio track");
    let pipe = app.directory.path().join("delayed-project.json");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap()
            .success()
    );
    let mut incoming = Project::new();
    incoming.add_audio_track("Incoming document");
    let json_path = app.directory.path().join("incoming.json");
    incoming.save(&json_path).unwrap();
    let bytes = std::fs::read(&json_path).unwrap();
    let (release, wait) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let mut file = std::fs::OpenOptions::new().write(true).open(pipe).unwrap();
        wait.recv_timeout(Duration::from_secs(20)).unwrap();
        file.write_all(&bytes).unwrap();
    });
    let start = Instant::now();
    let loading = app.command(json!({"command":"open","path":app.directory.path().join("delayed-project.json"),"wait":false}));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(loading["project_pending"], true);
    rename_track(&mut app, "New audio", "Edited while loading");
    let responsive = app.inspect();
    assert_eq!(
        responsive["project"]["tracks"][0]["name"],
        "Edited while loading"
    );
    assert_eq!(responsive["project_pending"], true);
    let saved = app.directory.path().join("queued-save.rill");
    app.command(json!({"command":"save","path":saved,"wait":false}));
    let queued = menu(&mut app, "File", "New");
    assert_eq!(
        queued["project"]["tracks"][0]["name"],
        "Edited while loading"
    );
    release.send(()).unwrap();
    writer.join().unwrap();
    let done = app.command(json!({"command":"wait_project"}));
    assert!(done["project"]["tracks"].as_array().unwrap().is_empty());
    assert_eq!(
        Project::load(&saved).unwrap().tracks[0].name,
        "Incoming document"
    );
    let recovery = Project::load(app.directory.path().join("scratch/recovery.daw.json")).unwrap();
    assert_eq!(recovery.tracks[0].name, "Edited while loading");
    app.quit();
}
