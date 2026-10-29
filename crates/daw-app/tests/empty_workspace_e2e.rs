//! First-use actions use real application navigation, pickers and music edits.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn unchanged(actual: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "song_session",
        "modified",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(actual[field], before[field], "Viewing changed {field}");
    }
}

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

fn empty_actions_fit(state: &Value, width: f64, height: f64) {
    for name in [
        "Start a song",
        "Choose instrument…",
        "Import audio or MIDI…",
        "Compose with AI",
    ] {
        let rect = label(state, name);
        assert!(
            rect["min"][0].as_f64().unwrap() >= 0.0
                && rect["min"][1].as_f64().unwrap() >= 0.0
                && rect["max"][0].as_f64().unwrap() <= width
                && rect["max"][1].as_f64().unwrap() <= height,
            "{name} is clipped: {rect}"
        );
    }
}

fn replace_prompt(app: &mut Harness, text: &str) {
    let state = app.inspect();
    let (x, y) = center(&state["assistant"]["prompt_rect"]);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}));
}

fn picker(app: &mut Harness, path: Option<&Path>) {
    app.command(json!({"command":"file_dialog","kind":"import","path":path}));
}

fn source_audio(app: &Harness) -> PathBuf {
    let path = app.directory.path().join("First recording.wav");
    AudioData {
        sample_rate: 24_000,
        channels: 1,
        samples: vec![0.125; 48_000],
    }
    .write_wav(&path)
    .unwrap();
    path
}

#[test]
fn starting_actions_fit_and_browsing_keeps_song_and_ai_draft_unchanged() {
    for (width, height, dark, density) in [
        (800, 600, false, 1.0),
        (800, 600, true, 1.0),
        (1200, 900, false, 2.0),
        (1200, 900, true, 2.0),
    ] {
        let mut app = Harness::new();
        app.command(
            json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}),
        );
        let before = app.command(json!({"command":"system_theme","dark":dark}));
        empty_actions_fit(&before, f64::from(width), f64::from(height));

        let library = app.click("Choose instrument…");
        assert_eq!(library["popups"]["library"], true);
        unchanged(&library, &before);
        let closed = app.key("Escape");
        assert_eq!(closed["popups"]["library"], false);
        empty_actions_fit(&closed, f64::from(width), f64::from(height));
        unchanged(&closed, &before);

        let assistant = app.click("Compose with AI");
        assert_eq!(assistant["assistant"]["scope"]["kind"], "song");
        assert_eq!(assistant["assistant"]["page"], "Examples");
        assert_eq!(assistant["assistant"]["busy"], false);
        assert_eq!(assistant["assistant"]["messages"], json!([]));
        unchanged(&assistant, &before);
        replace_prompt(&mut app, "Keep my unsent musical idea.");
        app.key("Escape");
        let reopened = app.click("Compose with AI");
        assert_eq!(
            reopened["assistant"]["prompt"],
            "Keep my unsent musical idea."
        );
        assert_eq!(reopened["assistant"]["busy"], false);
        assert_eq!(reopened["assistant"]["messages"], json!([]));
        unchanged(&reopened, &before);
        app.quit();
    }
}

#[test]
fn native_import_action_cancels_cleanly_and_imports_audio_and_midi_with_undo() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":800,"height":600}));
    let initial = app.inspect();
    let x = initial["timeline"]["canvas"]["min"][0].as_f64().unwrap()
        + 3.0 * initial["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let y = center(&initial["timeline"]["ruler"]).1;
    let before = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(before["timeline"]["cursor"], 3.0);
    picker(&mut app, None);
    let cancelled = app.click("Import audio or MIDI…");
    assert_eq!(cancelled["file_dialog_pending"], false);
    unchanged(&cancelled, &before);
    assert!(visible(&cancelled, "Start a song"));

    let wav = source_audio(&app);
    let original_bytes = std::fs::read(&wav).unwrap();
    picker(&mut app, Some(&wav));
    let imported = app.click("Import audio or MIDI…");
    assert_eq!(imported["file_dialog_pending"], false);
    assert_eq!(imported["project"]["tracks"].as_array().unwrap().len(), 1);
    let track = &imported["project"]["tracks"][0];
    assert_eq!(track["kind"], "Audio");
    assert_eq!(track["clips"].as_array().unwrap().len(), 1);
    assert_eq!(track["clips"][0]["start"], 3.0);
    assert_eq!(track["clips"][0]["length"], 4.0);
    assert_eq!(imported["selected_clip"], track["clips"][0]["id"]);
    assert_eq!(imported["history"]["undo"].as_array().unwrap().len(), 1);
    assert!(!visible(&imported, "Start a song"));
    assert_eq!(std::fs::read(&wav).unwrap(), original_bytes);
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    assert!(visible(&undone, "Start a song"));

    let mut source = Project::new();
    let track = source.add_midi_track("First melody");
    source.track_mut(track).unwrap().clips.push(Clip::midi(
        "First phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 90), Note::new(67, 2.0, 0.5, 70)],
    ));
    let midi = app.directory.path().join("First melody.MID");
    source.export_midi(&midi).unwrap();
    let original_bytes = std::fs::read(&midi).unwrap();
    picker(&mut app, Some(&midi));
    let imported = app.click("Import audio or MIDI…");
    assert_eq!(imported["file_dialog_pending"], false);
    assert_eq!(imported["project"]["tracks"].as_array().unwrap().len(), 1);
    let track = &imported["project"]["tracks"][0];
    assert_eq!(track["kind"], "Midi");
    assert_eq!(track["clips"][0]["start"], 3.0);
    let notes = track["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0]["pitch"], 60);
    assert_eq!(notes[0]["start"], 0.0);
    assert_eq!(notes[1]["pitch"], 67);
    assert_eq!(notes[1]["start"], 2.0);
    assert_eq!(imported["history"]["undo"].as_array().unwrap().len(), 1);
    assert!(!visible(&imported, "Start a song"));
    assert_eq!(std::fs::read(&midi).unwrap(), original_bytes);
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    assert!(visible(&undone, "Start a song"));
    app.quit();
}

#[test]
fn removing_the_open_clip_channel_returns_to_the_empty_arranger_without_extra_edits() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        app.command(
            json!({"command":"viewport","width":800,"height":600,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        let mut project = Project::new();
        let track = project.add_midi_track("Empty editor");
        let phrase = Clip::midi("Remove this phrase", 0.0, 4.0, Vec::new());
        project.track_mut(track).unwrap().clips.push(phrase);
        let path = app.directory.path().join("empty-editor.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.click("Remove this phrase");
        let editor = menu(&mut app, "Window", "Clip Editor");
        assert_eq!(editor["view"], "Clip");
        let removed = app.command(
            json!({"command":"song_tool","name":"apply_edits","arguments":{
                "session_id":editor["song_session"]["id"],
                "expected_revision":editor["song_session"]["revision"],
                "label":"Remove open clip channel",
                "edits":[{"op":"delete_track","track":track}]
            }}),
        );
        assert_eq!(removed["tool_result"]["ok"], true);
        assert_eq!(removed["view"], "Arrangement");
        assert!(visible(&removed, "Start a song"));
        assert!(removed["project"]["tracks"].as_array().unwrap().is_empty());
        assert_eq!(removed["history"]["undo"].as_array().unwrap().len(), 1);
        let restored = menu(&mut app, "Edit", "Undo");
        assert_eq!(restored["project"], editor["project"]);
        assert!(!visible(&restored, "Start a song"));
        app.quit();
    }
}

#[test]
fn empty_canvas_still_accepts_file_drop_and_direct_pattern_creation() {
    let mut app = Harness::new();
    let before = app.inspect();
    let wav = source_audio(&app);
    let dropped = app.command(json!({"command":"drop","paths":[wav],"beat":3.0}));
    assert_eq!(dropped["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(dropped["project"]["tracks"][0]["clips"][0]["start"], 3.0);
    assert!(!visible(&dropped, "Start a song"));
    let empty = menu(&mut app, "Edit", "Undo");
    assert_eq!(empty["project"], before["project"]);
    assert!(visible(&empty, "Start a song"));

    let canvas = &empty["timeline"]["canvas"];
    let x = canvas["max"][0].as_f64().unwrap() - 24.0;
    let y = canvas["max"][1].as_f64().unwrap() - 30.0;
    let created = app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    assert_eq!(created["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(created["project"]["tracks"][0]["kind"], "Midi");
    assert_eq!(
        created["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(created["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(created["popups"]["library"], true);
    app.quit();
}
