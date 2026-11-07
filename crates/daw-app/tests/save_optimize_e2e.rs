//! Optimize the saved song without destroying media retained by Undo or recall.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};
use std::{fs, path::Path};
use zip::{CompressionMethod, ZipArchive};

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn audio(path: &Path, value: f32) {
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![value; 8_000],
    }
    .write_wav(path)
    .unwrap();
}

fn document(state: &Value) -> Project {
    serde_json::from_value(state["project"].clone()).unwrap()
}

fn assets(path: &Path) -> Vec<CompressionMethod> {
    let mut zip = ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .filter_map(|index| {
            let entry = zip.by_index(index).unwrap();
            entry
                .name()
                .unwrap()
                .starts_with("assets/")
                .then_some(entry.compression())
        })
        .collect()
}

#[test]
fn menu_optimization_removes_deleted_assets_deduplicates_and_preserves_undo_and_portability() {
    let mut app = Harness::new();
    let used = app.directory.path().join("used.wav");
    let copy = app.directory.path().join("identical-copy.wav");
    let retained = app.directory.path().join("retained.wav");
    let disposable = app.directory.path().join("disposable.wav");
    audio(&used, 0.25);
    fs::copy(&used, &copy).unwrap();
    audio(&retained, -0.125);
    audio(&disposable, 0.375);
    let originals = [&used, &copy, &retained, &disposable].map(|path| fs::read(path).unwrap());
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    project.tracks[0].gain = 1.0;
    project.tracks[0].audio_input = None;
    for (name, start, path, muted) in [
        ("Used", 0.0, &used, false),
        ("Identical copy", 2.0, &copy, true),
        ("Retained muted original", 4.0, &retained, true),
        ("Disposable", 6.0, &disposable, false),
    ] {
        let mut clip = Clip::audio(name, start, 2.0, path.to_string_lossy());
        clip.muted = muted;
        project.track_mut(track).unwrap().clips.push(clip);
    }
    let path = app.directory.path().join("Optimize.rill");
    project.save(&path).unwrap();
    let old_size = fs::metadata(&path).unwrap().len();
    assert_eq!(assets(&path), vec![CompressionMethod::Stored; 4]);
    let opened = app.command(json!({"command":"open","path":path}));
    let admitted = document(&opened);
    app.click("Disposable");
    let deleted = menu(&mut app, "Edit", "Delete");
    assert_eq!(document(&deleted).tracks[0].clips.len(), 3);
    menu(&mut app, "File", "Save and optimize");
    let saved = app.command(json!({"command":"wait_project"}));
    assert_eq!(saved["project"], deleted["project"]);
    assert_eq!(saved["history"]["undo"], deleted["history"]["undo"]);
    assert_eq!(saved["modified"], false);
    assert_eq!(saved["status"], "Project saved and optimized");
    assert_eq!(assets(&path), vec![CompressionMethod::Deflated; 2]);
    assert!(fs::metadata(&path).unwrap().len() < old_size / 2);

    // Optimizing a song file must leave the media for Undo available in-session.
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(document(&undone), admitted);
    let ClipContent::Audio {
        path: undo_source, ..
    } = &admitted.tracks[0].clips[3].content
    else {
        unreachable!()
    };
    assert_eq!(
        AudioData::load_wav(undo_source).unwrap().samples,
        vec![0.375; 8_000]
    );
    let render = app.directory.path().join("undo-render.wav");
    app.command(json!({"command":"export","path":render}));
    app.command(json!({"command":"wait_export"}));
    let rendered = AudioData::load_wav(&render).unwrap();
    let frame = (3.25 * rendered.sample_rate as f64) as usize;
    assert!((rendered.samples[frame * rendered.channels] - 0.375).abs() < 1e-4);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], deleted["project"]);
    assert_eq!(redone["modified"], false);
    for (source, bytes) in [&used, &copy, &retained, &disposable]
        .into_iter()
        .zip(originals)
    {
        assert_eq!(fs::read(source).unwrap(), bytes);
    }

    app = app.restart();
    // Neither original imports nor old extracted sources are needed for recall.
    for source in [&used, &copy, &retained, &disposable] {
        fs::remove_file(source).unwrap();
    }
    for clip in &admitted.tracks[0].clips {
        if let ClipContent::Audio { path, .. } = &clip.content {
            fs::remove_file(path).unwrap();
        }
    }
    let moved = app.directory.path().join("Moved optimized.rill");
    fs::rename(path, &moved).unwrap();
    let recalled = app.command(json!({"command":"open","path":moved}));
    let project = document(&recalled);
    assert_eq!(project.tracks[0].clips.len(), 3);
    for (clip, expected) in project.tracks[0].clips.iter().zip([0.25, 0.25, -0.125]) {
        let ClipContent::Audio { path, .. } = &clip.content else {
            unreachable!()
        };
        assert_eq!(
            AudioData::load_wav(path).unwrap().samples,
            vec![expected; 8_000]
        );
    }
    assert!(project.tracks[0].clips[1].muted && project.tracks[0].clips[2].muted);
    app.quit();
}

#[test]
fn optimize_picker_cancellation_preserves_unsaved_work_and_can_save_a_new_project() {
    let mut app = Harness::new();
    let before = menu(&mut app, "Track", "Add audio track");
    app.command(json!({"command":"file_dialog","kind":"save","path":null}));
    menu(&mut app, "File", "Save and optimize");
    let cancelled = app.command(json!({"command":"wait_project"}));
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    assert_eq!(cancelled["modified"], true);
    assert_eq!(cancelled["project_path"], Value::Null);
    let chosen = app.directory.path().join("New optimized song");
    app.command(json!({"command":"file_dialog","kind":"save","path":chosen}));
    menu(&mut app, "File", "Save and optimize");
    let saved = app.command(json!({"command":"wait_project"}));
    let path = chosen.with_extension("rill");
    assert_eq!(saved["project_path"], json!(path));
    assert_eq!(saved["modified"], false);
    assert_eq!(Project::load(path).unwrap(), document(&before));
    app.quit();
}

#[test]
fn failed_optimization_preserves_existing_bundle_and_unsaved_document() {
    let mut app = Harness::new();
    let existing = app.directory.path().join("Existing.rill");
    Project::new().save(&existing).unwrap();
    let original = fs::read(&existing).unwrap();
    let source = app.directory.path().join("external.wav");
    audio(&source, 0.25);
    let mut project = Project::new();
    project.add_audio_track("Keep this source");
    project.tracks[0].clips.push(Clip::audio(
        "Missing later",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    let legacy = app.directory.path().join("Legacy.json");
    project.save_legacy_json(&legacy).unwrap();
    app.command(json!({"command":"open","path":legacy}));
    let before = menu(&mut app, "Track", "Add audio track");
    fs::remove_file(&source).unwrap();
    app.command(
        json!({"command":"file_dialog","kind":"save","path":existing,"replace_existing":true}),
    );
    menu(&mut app, "File", "Save and optimize");
    let failed = app.raw_command(json!({"command":"wait_project"}));
    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(failed["state"]["project"], before["project"]);
    assert_eq!(failed["state"]["history"], before["history"]);
    assert_eq!(failed["state"]["project_path"], json!(legacy));
    assert_eq!(failed["state"]["modified"], true);
    assert_eq!(fs::read(&existing).unwrap(), original);
    app.quit();
}
