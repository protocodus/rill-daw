//! Import real encoded files through the application, with separate controlled
//! worker failures exercising the asynchronous transaction and process boundary.
mod common;

use common::Harness;
use daw_core::{AudioData, Project};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

fn fixture(extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../daw-core/tests/fixtures/audio-import")
        .join(format!("tone.{extension}"))
}

fn source_copy(app: &Harness, extension: &str, name: &str) -> PathBuf {
    let directory = app.directory.path().join("External sources");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    std::fs::copy(fixture(extension), &path).unwrap();
    path
}

fn has(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(
            state[field], before[field],
            "Uncommitted import changed {field}"
        );
    }
}

fn media_files(root: &Path) -> BTreeSet<PathBuf> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "wav") {
                found.insert(path);
            }
        }
    }
    found
}

fn scratch(state: &Value) -> &Path {
    Path::new(state["scratch_dir"].as_str().unwrap())
}

fn wait_import(app: &mut Harness) -> Value {
    let result = app.raw_command(json!({"command":"wait_audio_import", "timeout_ms":5000}));
    assert!(
        result.get("state").is_some(),
        "Missing import state: {result}"
    );
    let state = result["state"].clone();
    assert_eq!(
        state["audio_import"]["busy"], false,
        "Import did not finish: {result}"
    );
    assert_eq!(state["audio_import"]["queued"], 0);
    state
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn redo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Redo")
}

#[test]
fn real_formats_use_private_wavs_and_survive_undo_and_portable_project_reopen() {
    let mut app = Harness::new();
    let initial = app.inspect();
    let wav = source_copy(&app, "wav", "Original wave.wav");
    let original = std::fs::read(&wav).unwrap();
    app.command(json!({"command":"file_dialog", "kind":"import_audio", "path":wav}));
    app.click("File");
    app.click("Import audio…");
    let mut state = wait_import(&mut app);
    assert_eq!(state["error"], false, "{state}");
    assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(
        std::fs::read(&wav).unwrap(),
        original,
        "Import must not rewrite its source"
    );
    let destination = state["project"]["tracks"][0]["id"].as_u64().unwrap();
    let mut originals = vec![wav];
    let mut canonical = Vec::new();
    let mut metadata = Vec::new();
    for (index, extension) in ["wav", "mp3", "flac", "ogg"].into_iter().enumerate() {
        if index > 0 {
            let source = source_copy(
                &app,
                extension,
                &format!("Original {extension}.{extension}"),
            );
            let original = std::fs::read(&source).unwrap();
            app.click("Edit");
            let selected = app.click("Select all clips");
            assert_eq!(
                selected["selected_clips"].as_array().unwrap().len(),
                index,
                "Seed the import with the previously selected arranger clips"
            );
            let before = app.inspect();
            state = app.command(json!({"command":"drop", "paths":[source], "track":0,
                "beat":index as f64 * 1.5}));
            assert_eq!(state["error"], false, "{state}");
            assert_eq!(std::fs::read(&source).unwrap(), original);
            assert_eq!(
                state["history"]["undo"].as_array().unwrap().len(),
                before["history"]["undo"].as_array().unwrap().len() + 1
            );
            originals.push(source);
        }
        assert_eq!(state["project"]["tracks"].as_array().unwrap().len(), 1);
        let clips = track(&state, destination)["clips"].as_array().unwrap();
        assert_eq!(clips.len(), index + 1);
        let clip = &clips[index];
        assert_eq!(
            state["selected_clips"],
            json!([[destination, clip["id"]]]),
            "Import must replace stale multi-selection with the newly imported clip"
        );
        assert!(state["selection_span"].is_null());
        assert!((clip["start"].as_f64().unwrap() - index as f64 * 1.5).abs() < 1e-4);
        let path = PathBuf::from(clip["content"]["Audio"]["path"].as_str().unwrap());
        assert!(
            path.starts_with(scratch(&state)),
            "Imported media must be privately owned"
        );
        assert!(!originals.contains(&path));
        let decoded = AudioData::load_wav(&path).unwrap();
        assert!((8_000..=384_000).contains(&decoded.sample_rate));
        assert!((1..=2).contains(&decoded.channels));
        assert!(decoded.frames() > 0);
        assert!(decoded.samples.iter().all(|sample| sample.is_finite()));
        assert!(
            decoded.samples.iter().any(|sample| sample.abs() > 0.01),
            "{extension} decoded silence"
        );
        metadata.push((decoded.sample_rate, decoded.channels, decoded.frames()));
        canonical.push(path);
    }
    assert_eq!(
        state["history"]["undo"].as_array().unwrap().len(),
        initial["history"]["undo"].as_array().unwrap().len() + 4
    );

    let before_undo = state["project"].clone();
    let undone = undo(&mut app);
    assert_eq!(
        track(&undone, destination)["clips"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        canonical.iter().all(|path| path.exists()),
        "Undo must retain media needed by Redo"
    );
    assert_eq!(redo(&mut app)["project"], before_undo);

    for source in originals {
        std::fs::remove_file(source).unwrap();
    }
    let portable = app.directory.path().join("Portable imported song.rill");
    app.command(json!({"command":"save", "path":portable}));
    // Force reopen to use the bundle's embedded assets, not still-available
    // external or managed files from this process's previous import.
    for path in canonical {
        std::fs::remove_file(path).unwrap();
    }
    let mut app = app.restart();
    let reopened = app.command(json!({"command":"open", "path":portable}));
    for (clip, expected) in track(&reopened, destination)["clips"]
        .as_array()
        .unwrap()
        .iter()
        .zip(metadata)
    {
        let decoded =
            AudioData::load_wav(clip["content"]["Audio"]["path"].as_str().unwrap()).unwrap();
        assert_eq!(
            (decoded.sample_rate, decoded.channels, decoded.frames()),
            expected
        );
        assert!(decoded.samples.iter().any(|sample| sample.abs() > 0.01));
    }
    app.quit();
}

#[test]
fn corrupt_and_oversized_batches_leave_no_partial_edit_or_managed_audio() {
    let mut app = Harness::new();
    let valid = source_copy(&app, "mp3", "Valid source.mp3");
    let corrupt = app.directory.path().join("Broken source.flac");
    std::fs::write(&corrupt, b"This is not a FLAC stream").unwrap();
    let before = app.inspect();
    let assets = media_files(scratch(&before));
    app.raw_command(json!({"command":"drop", "paths":[valid,corrupt], "beat":0}));
    let failed = wait_import(&mut app);
    unchanged(&failed, &before);
    assert_eq!(failed["error"], true);
    assert_eq!(media_files(scratch(&failed)), assets);

    // A failed batch must not poison the next valid import or require the
    // user to dismiss its notice before continuing with their song.
    let recovery = source_copy(&app, "wav", "Recovered source.wav");
    let recovered = app.command(json!({"command":"drop", "paths":[recovery], "beat":1.5}));
    assert_eq!(
        recovered["error"], false,
        "A valid import must clear its older import failure"
    );
    let tracks = recovered["project"]["tracks"].as_array().unwrap();
    assert_eq!(
        tracks.len(),
        1,
        "The failed mixed batch must remain entirely uncommitted"
    );
    assert_eq!(tracks[0]["kind"], "Audio");
    let clips = tracks[0]["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0]["name"], "Recovered source");
    assert!((clips[0]["start"].as_f64().unwrap() - 1.5).abs() < 1e-4);
    let decoded =
        AudioData::load_wav(clips[0]["content"]["Audio"]["path"].as_str().unwrap()).unwrap();
    assert!(decoded.samples.iter().any(|sample| sample.abs() > 0.01));
    let history = recovered["history"]["undo"].as_array().unwrap();
    assert_eq!(
        history.len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert_eq!(history.last().unwrap(), "Import audio");
    let recovered_assets = media_files(scratch(&recovered));

    let paths: Vec<_> = (0..9)
        .map(|index| source_copy(&app, "wav", &format!("Too many {index}.wav")))
        .collect();
    app.raw_command(json!({"command":"drop", "paths":paths, "beat":0}));
    let refused = wait_import(&mut app);
    unchanged(&refused, &recovered);
    assert_eq!(refused["error"], true);
    assert_eq!(media_files(scratch(&refused)), recovered_assets);
    app.quit();
}

struct ControlledWorker {
    directory: tempfile::TempDir,
    binary: PathBuf,
}

impl ControlledWorker {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let actual = std::env::var_os("RILL_E2E_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_rill").into());
        let source = directory.path().join("audio_import_worker.rs");
        let code = r#"
use std::{path::PathBuf, process::{Command, Stdio}, time::Duration};
const ACTUAL: &str = __ACTUAL__;
const CONTROL: &str = __CONTROL__;
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    assert_eq!(args.len(), 5);
    let input = PathBuf::from(&args[2]);
    let name = input.file_name().unwrap().to_string_lossy();
    let marker = |suffix: &str| PathBuf::from(CONTROL).join(format!("{name}.{suffix}"));
    if name.starts_with("crash-") || name.starts_with("hang-") {
        std::fs::write(&args[3], b"incomplete WAV output").unwrap();
    }
    std::fs::write(marker("started"), std::process::id().to_string()).unwrap();
    if name.starts_with("crash-") { std::process::exit(27); }
    if name.starts_with("hang-") { loop { std::thread::sleep(Duration::from_millis(10)); } }
    if name.starts_with("gate-") {
        while !marker("release").exists() { std::thread::sleep(Duration::from_millis(5)); }
    }
    let status = Command::new(ACTUAL).args(&args[1..]).stdin(Stdio::null()).status().unwrap();
    std::process::exit(status.code().unwrap_or(1));
}
"#;
        let code = code
            .replace("__ACTUAL__", &format!("{:?}", actual.to_string_lossy()))
            .replace(
                "__CONTROL__",
                &format!("{:?}", directory.path().to_string_lossy()),
            );
        std::fs::write(&source, code).unwrap();
        let binary = directory.path().join(if cfg!(windows) {
            "audio-worker.exe"
        } else {
            "audio-worker"
        });
        let result = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Self { directory, binary }
    }

    fn marker(&self, source: &Path, suffix: &str) -> PathBuf {
        self.directory.path().join(format!(
            "{}.{suffix}",
            source.file_name().unwrap().to_string_lossy()
        ))
    }

    fn started(&self, source: &Path) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !self.marker(source, "started").exists() {
            assert!(Instant::now() < deadline, "The import helper did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn release(&self, source: &Path) {
        std::fs::write(self.marker(source, "release"), b"continue").unwrap();
    }
}

fn destinations(app: &mut Harness) -> (u64, u64) {
    let mut project = Project::new();
    let destination = project.add_audio_track("Destination");
    let other = project.add_audio_track("Other destination");
    let path = app.directory.path().join("Destinations.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    (destination, other)
}

fn start_pending(app: &mut Harness, path: &Path, beat: f64) -> Value {
    let state = app
        .command(json!({"command":"drop", "paths":[path], "track":0, "beat":beat, "wait":false}));
    assert_eq!(state["audio_import"]["busy"], true);
    state
}

#[test]
fn asynchronous_drop_keeps_its_track_and_beat_and_rejects_a_deleted_destination() {
    let worker = ControlledWorker::new();
    let mut app = Harness::with_audio_import_helper(&worker.binary, 10_000);
    let (destination, other) = destinations(&mut app);
    let source = source_copy(&app, "wav", "gate-captured.wav");
    let before = app.inspect();
    start_pending(&mut app, &source, 3.25);
    worker.started(&source);
    app.click("Other destination");
    let blocked = app.click("Record");
    unchanged(&blocked, &before);
    assert_eq!(
        blocked["status"],
        "Finish or cancel audio import before recording"
    );
    assert_eq!(blocked["error"], true);
    assert_eq!(blocked["recording"]["phase"], "idle");
    assert_eq!(blocked["recording"]["timing"]["capturing"], false);
    assert_eq!(blocked["recording"]["midi"]["recording"], false);
    assert_eq!(blocked["recording"]["frames"], 0);
    worker.release(&source);
    let done = wait_import(&mut app);
    assert_eq!(
        done["error"], true,
        "Import must preserve a newer unrelated Record error"
    );
    assert_eq!(done["status"], blocked["status"]);
    assert_eq!(track(&done, destination)["clips"][0]["start"], 3.25);
    assert!(track(&done, other)["clips"].as_array().unwrap().is_empty());
    assert_eq!(
        done["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    app.click("Dismiss");

    let assets = media_files(scratch(&done));
    let source = source_copy(&app, "wav", "gate-deleted.wav");
    start_pending(&mut app, &source, 1.5);
    worker.started(&source);
    let state = app.inspect();
    let name = &state["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["track"] == destination)
        .unwrap()["name"];
    let x = (name["min"][0].as_f64().unwrap() + name["max"][0].as_f64().unwrap()) * 0.5;
    let y = (name["min"][1].as_f64().unwrap() + name["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":"click", "button":"secondary", "x":x, "y":y}));
    let deleted = app.click("Delete track");
    worker.release(&source);
    let stale = wait_import(&mut app);
    unchanged(&stale, &deleted);
    assert_eq!(stale["error"], true);
    assert!(track(&stale, other)["clips"].as_array().unwrap().is_empty());
    assert_eq!(media_files(scratch(&stale)), assets);
    app.quit();
}

fn cursor_at(app: &mut Harness, beat: f64) -> Value {
    let state = app.inspect();
    let timeline = &state["timeline"];
    let x = timeline["ruler"]["min"][0].as_f64().unwrap()
        + (beat - timeline["start_beat"].as_f64().unwrap())
            * timeline["pixels_per_beat"].as_f64().unwrap();
    let y = (timeline["ruler"]["min"][1].as_f64().unwrap()
        + timeline["ruler"]["max"][1].as_f64().unwrap())
        * 0.5;
    let positioned = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert!((positioned["timeline"]["cursor"].as_f64().unwrap() - beat).abs() < 1e-6);
    positioned
}

#[test]
fn header_drop_uses_the_captured_playhead_and_creates_audio_for_a_midi_header() {
    let worker = ControlledWorker::new();
    let mut app = Harness::with_audio_import_helper(&worker.binary, 10_000);
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio destination");
    let midi = project.add_midi_track("MIDI destination");
    let path = app.directory.path().join("Header destinations.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let before = cursor_at(&mut app, 3.25);
    let source = source_copy(&app, "wav", "gate-header.wav");
    let pending = app.command(json!({"command":"drop", "paths":[source], "track":0,
        "header":true, "beat":0.0, "wait":false}));
    assert_eq!(pending["audio_import"]["busy"], true);
    worker.started(&source);
    cursor_at(&mut app, 6.0);
    worker.release(&source);
    let done = wait_import(&mut app);
    assert_eq!(done["error"], false);
    assert_eq!(done["project"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(
        track(&done, audio)["clips"][0]["start"],
        3.25,
        "Header placement captures the playhead when the file is dropped"
    );
    assert!(track(&done, midi)["clips"].as_array().unwrap().is_empty());
    assert_eq!(
        done["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );

    cursor_at(&mut app, 4.5);
    let source = source_copy(&app, "mp3", "MIDI header audio.mp3");
    let imported = app.command(json!({"command":"drop", "paths":[source], "track":1,
        "header":true, "beat":0.0}));
    assert_eq!(imported["error"], false);
    assert!(
        track(&imported, midi)["clips"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let tracks = imported["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 3);
    let created = tracks
        .iter()
        .find(|track| track["id"] != audio && track["id"] != midi)
        .unwrap();
    assert_eq!(created["kind"], "Audio");
    assert_eq!(created["clips"].as_array().unwrap().len(), 1);
    assert_eq!(created["clips"][0]["start"], 4.5);
    assert_eq!(
        imported["selected_clips"],
        json!([[created["id"], created["clips"][0]["id"]]])
    );
    assert!(imported["selection_span"].is_null());
    app.quit();
}

#[test]
fn new_and_open_cancel_active_and_queued_imports_before_switching_documents() {
    let worker = ControlledWorker::new();
    for open in [false, true] {
        let mut app = Harness::with_audio_import_helper(&worker.binary, 10_000);
        destinations(&mut app);
        let prefix = if open { "open" } else { "new" };
        let active = source_copy(&app, "wav", &format!("gate-{prefix}-active.wav"));
        let queued = source_copy(&app, "wav", &format!("gate-{prefix}-queued.wav"));
        let before = app.inspect();
        let assets = media_files(scratch(&before));
        start_pending(&mut app, &active, 1.0);
        worker.started(&active);
        app.command(
            json!({"command":"drop", "paths":[queued], "track":0, "beat":2.0, "wait":false}),
        );
        assert_eq!(app.inspect()["audio_import"]["queued"], 1);
        let mut replacement = Project::new();
        if open {
            replacement.add_audio_track("Replacement project");
            let path = app.directory.path().join("Replacement.rill");
            replacement.save(&path).unwrap();
            app.command(json!({"command":"open", "path":path, "wait":false}));
        } else {
            app.click("File");
            app.click("New");
        }
        let done = app.command(json!({"command":"wait_project", "timeout_ms":5000}));
        assert_eq!(done["project"], serde_json::to_value(&replacement).unwrap());
        assert_eq!(done["audio_import"]["busy"], false);
        assert_eq!(done["audio_import"]["queued"], 0);
        assert!(
            !worker.marker(&queued, "started").exists(),
            "Cancelled queued work must never start"
        );
        assert_eq!(media_files(scratch(&done)), assets);
        app.quit();
    }
}

#[test]
fn worker_crash_timeout_and_user_cancel_do_not_commit_or_leave_partial_media() {
    let worker = ControlledWorker::new();
    for mode in ["crash", "hang", "cancel"] {
        let timeout = if mode == "hang" { 200 } else { 10_000 };
        let mut app = Harness::with_audio_import_helper(&worker.binary, timeout);
        destinations(&mut app);
        let prefix = if mode == "cancel" { "hang" } else { mode };
        let source = source_copy(&app, "wav", &format!("{prefix}-{mode}.wav"));
        let before = app.inspect();
        let assets = media_files(scratch(&before));
        let started = Instant::now();
        // An immediate crash may already be observed during the drop's own
        // settling frames; do not require an artificial intermediate busy state.
        app.raw_command(json!({"command":"drop", "paths":[source], "track":0,
            "beat":2.0, "wait":false}));
        worker.started(&source);
        if mode == "cancel" {
            assert!(has(&app.inspect(), "Cancel import"));
            app.click("Cancel import");
        }
        let failed = wait_import(&mut app);
        unchanged(&failed, &before);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Supervisor failed to stop {mode}"
        );
        if mode != "cancel" {
            assert_eq!(failed["error"], true);
        }
        assert_eq!(media_files(scratch(&failed)), assets);
        app.quit();
    }
}
