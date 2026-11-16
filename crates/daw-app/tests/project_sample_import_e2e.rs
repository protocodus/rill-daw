//! Add reusable recordings directly from the Library without creating tracks.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

fn tone(path: &Path) {
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..4_800)
            .map(|index| (index as f32 * std::f32::consts::TAU * 330.0 / 48_000.0).sin() * 0.3)
            .collect(),
    }
    .write_wav(path)
    .unwrap();
}

fn seed_song(app: &mut Harness) {
    let mut project = Project::new();
    let track = project.add_instrument_track("Existing part");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Original notes",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 0.5, 100)],
    ));
    let path = app.directory.path().join("Original.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
}

fn index_folder(app: &mut Harness, root: &Path, count: usize) -> Value {
    app.command(json!({"command":"viewport", "width":1500, "height":1050}));
    app.click("Window");
    app.click("Device browser");
    app.click("Samples");
    app.add_sample_folder(root);
    app.click("Scan folders");
    common::wait_until(app, |state| {
        state["library"]["indexed"] == count && state["library"]["scanning"] == false
    })
}

fn wait_import(app: &mut Harness) -> Value {
    let response = app.raw_command(json!({"command":"wait_audio_import", "timeout_ms":5000}));
    let state = response["state"].clone();
    assert_eq!(state["audio_import"]["busy"], false, "{response}");
    assert_eq!(state["audio_import"]["queued"], 0, "{response}");
    state
}

fn undo_count(state: &Value) -> usize {
    state["history"]["undo"].as_array().unwrap().len()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn wav_files(root: &Path) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    let mut directories = vec![root.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "wav") {
                files.insert(path);
            }
        }
    }
    files
}

fn drag_into_project_samples(app: &mut Harness, source: &Path) -> Value {
    let state = app.inspect();
    let from = state["library"]["samples"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == source.to_string_lossy().as_ref())
        .unwrap_or_else(|| panic!("Missing source row: {state}"));
    let to = state["library"]["drop_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"].as_str().is_some_and(|id| id == "project"))
        .unwrap_or_else(|| panic!("Missing project sample drop target: {state}"));
    let (x, y) = center(&from["rect"]);
    let (to_x, to_y) = center(&to["rect"]);
    app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y,
        "to_x":to_x, "to_y":to_y}));
    wait_import(app)
}

#[test]
fn add_to_project_samples_is_managed_idempotent_undoable_and_portable() {
    let mut app = Harness::new();
    seed_song(&mut app);
    let root = app
        .directory
        .path()
        .canonicalize()
        .unwrap()
        .join("External library");
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("Copper hit.wav");
    tone(&source);
    let original_bytes = std::fs::read(&source).unwrap();
    let original_audio = AudioData::load_wav(&source).unwrap();
    index_folder(&mut app, &root, 1);
    app.click("Copper hit.wav");
    let before = app.inspect();
    app.click("Add to Project Samples");
    let added = wait_import(&mut app);
    assert_eq!(added["error"], false, "{added}");
    let added = common::wait_until(&mut app, |state| {
        state["audio_sources"]["metadata_sources"] == 1
            && state["audio_sources"]["pending"] == false
    });
    assert!(
        added["audio_sources"]["resident_pcm_bytes"]
            .as_u64()
            .unwrap()
            >= 4_800 * 4
    );
    assert_eq!(
        added["revision"], before["revision"],
        "pool imports do not restart playback"
    );
    assert_eq!(added["project"]["tracks"], before["project"]["tracks"]);
    assert_eq!(undo_count(&added), undo_count(&before) + 1);
    let samples = added["project"]["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0]["name"], "Copper hit");
    let sample_id = samples[0]["id"].as_u64().unwrap();
    let managed = PathBuf::from(samples[0]["path"].as_str().unwrap());
    assert_ne!(managed, source);
    assert!(managed.starts_with(added["scratch_dir"].as_str().unwrap()));
    assert!(managed.is_file());
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    assert_eq!(
        AudioData::load_wav(&managed).unwrap().samples,
        original_audio.samples
    );

    // Adding the named project entry again reuses its existing managed media.
    app.click("Copper hit");
    app.click("In Project Samples");
    let repeated = wait_import(&mut app);
    assert_eq!(repeated["error"], false, "{repeated}");
    assert_eq!(repeated["project"], added["project"]);
    assert_eq!(repeated["history"], added["history"]);

    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"], before["project"]);
    assert_eq!(undone["revision"], before["revision"]);
    assert!(managed.is_file(), "Redo still owns the managed recording");
    app.click("Edit");
    let redone = app.click("Redo");
    assert_eq!(redone["project"], added["project"]);
    assert_eq!(redone["revision"], before["revision"]);

    let portable = app.directory.path().join("Reusable samples.rill");
    app.command(json!({"command":"save", "path":portable}));
    std::fs::remove_file(&source).unwrap();
    std::fs::remove_file(&managed).unwrap();
    let mut app = app.restart();
    let opened = app.command(json!({"command":"open", "path":portable}));
    assert_eq!(opened["project"]["tracks"], before["project"]["tracks"]);
    let sample = &opened["project"]["samples"][0];
    assert_eq!(sample["id"], sample_id);
    assert_eq!(sample["name"], "Copper hit");
    let restored = AudioData::load_wav(sample["path"].as_str().unwrap()).unwrap();
    assert_eq!(restored.sample_rate, original_audio.sample_rate);
    assert_eq!(restored.channels, original_audio.channels);
    assert_eq!(restored.samples, original_audio.samples);
    app.quit();
}

#[test]
fn dragging_a_library_sample_to_the_project_header_adds_once_without_tracks() {
    let mut app = Harness::new();
    seed_song(&mut app);
    let root = app
        .directory
        .path()
        .canonicalize()
        .unwrap()
        .join("Draggable library");
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("Wood tick.wav");
    tone(&source);
    let before = index_folder(&mut app, &root, 1);
    let added = drag_into_project_samples(&mut app, &source);
    assert_eq!(added["error"], false, "{added}");
    assert_eq!(
        added["revision"], before["revision"],
        "pool drops do not restart playback"
    );
    assert_eq!(added["project"]["tracks"], before["project"]["tracks"]);
    assert_eq!(added["project"]["samples"].as_array().unwrap().len(), 1);
    assert_eq!(added["project"]["samples"][0]["name"], "Wood tick");
    assert_eq!(undo_count(&added), undo_count(&before) + 1);
    let managed = PathBuf::from(added["project"]["samples"][0]["path"].as_str().unwrap());
    let repeated = drag_into_project_samples(&mut app, &managed);
    assert_eq!(repeated["project"], added["project"]);
    assert_eq!(repeated["history"], added["history"]);
    app.click("Edit");
    assert_eq!(app.click("Undo")["project"], before["project"]);
    app.click("Edit");
    assert_eq!(app.click("Redo")["project"], added["project"]);
    app.quit();
}

#[test]
fn an_unreadable_library_source_does_not_add_an_empty_sample_or_undo_step() {
    let mut app = Harness::new();
    seed_song(&mut app);
    let root = app
        .directory
        .path()
        .canonicalize()
        .unwrap()
        .join("Invalid library");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Broken.flac"), b"This is not a FLAC stream").unwrap();
    index_folder(&mut app, &root, 1);
    app.click("Broken.flac");
    let before = app.inspect();
    app.click("Add to Project Samples");
    let failed = wait_import(&mut app);
    assert_eq!(failed["error"], true);
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(
            failed[field], before[field],
            "failed import changed {field}"
        );
    }
    app.quit();
}

#[test]
fn adding_an_audio_track_from_a_project_sample_reuses_its_media_and_name() {
    let mut app = Harness::new();
    let source = app.directory.path().join("opaque-source.wav");
    tone(&source);
    let mut project = Project::new();
    project
        .add_project_sample("Friendly shaker", source.to_string_lossy())
        .unwrap();
    let song = app.directory.path().join("Named sample.rill");
    project.save(&song).unwrap();
    app.command(json!({"command":"open", "path":song}));
    app.command(json!({"command":"viewport", "width":1500, "height":1050}));
    app.click("Window");
    app.click("Device browser");
    app.click("Samples");
    let before = app.click("Friendly shaker");
    let managed = before["project"]["samples"][0]["path"].as_str().unwrap();
    assert_ne!(
        managed,
        source.to_str().unwrap(),
        "opening uses the bundled recording"
    );
    let scratch = PathBuf::from(before["scratch_dir"].as_str().unwrap());
    let media_before = wav_files(&scratch);
    app.click("Use…");
    app.click("Add audio track");
    let added = wait_import(&mut app);
    assert_eq!(added["error"], false, "{added}");
    assert_eq!(added["project"]["samples"], before["project"]["samples"]);
    let tracks = added["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0]["kind"], "Audio");
    let clips = tracks[0]["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0]["name"], "Friendly shaker");
    assert_eq!(clips[0]["content"]["Audio"]["path"], managed);
    assert_eq!(
        wav_files(&scratch),
        media_before,
        "reuse does not write another recording"
    );
    assert_eq!(undo_count(&added), undo_count(&before) + 1);
    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"], before["project"]);
    assert_eq!(wav_files(&scratch), media_before);
    app.quit();
}
