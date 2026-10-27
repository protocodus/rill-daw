//! Mouse-only meter setup uses the current quarter-note time-signature model.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn fixture(virtual_audio: bool) -> (Harness, std::path::PathBuf, Vec<u8>) {
    let mut app = if virtual_audio {
        Harness::with_virtual_audio(None, None)
    } else {
        Harness::new()
    };
    let source = app.directory.path().join("Original loop.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Meter audio");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Original loop",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("Meter setup.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    (app, source, bytes)
}

fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}

fn wait(app: &mut Harness, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if predicate(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Recording state did not settle: {}",
            state["recording"]
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn mouse_meter_choice_is_one_undoable_edit_and_portable_save_recalls_it() {
    let (mut app, source, bytes) = fixture(false);
    let before = app.inspect();
    let chosen = menu(&mut app, "4/4", "3/4");
    let mut expected = before["project"].clone();
    expected["beats_per_bar"] = json!(3);
    assert_eq!(
        chosen["project"], expected,
        "Meter setup never retimes audio or changes clip parameters"
    );
    assert_eq!(chosen["project"]["beats_per_bar"], 3);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(menu(&mut app, "Edit", "Redo")["project"], chosen["project"]);
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    let bundle = app.directory.path().join("Three-quarter song.rill");
    app.command(json!({"command":"file_dialog","kind":"save","path":bundle}));
    menu(&mut app, "File", "Save as…");
    let saved = app.command(json!({"command":"wait_project"}));
    assert_eq!(saved["modified"], false);
    app = app.restart();
    app.command(json!({"command":"file_dialog","kind":"open","path":bundle}));
    menu(&mut app, "File", "Open…");
    let reopened = app.command(json!({"command":"wait_project"}));
    assert_eq!(reopened["project"]["beats_per_bar"], 3);
    assert!(
        reopened["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "3/4")
    );
    let recalled: Project = serde_json::from_value(reopened["project"].clone()).unwrap();
    let ClipContent::Audio { path, .. } = &recalled.tracks[0].clips[0].content else {
        panic!("Expected audio loop")
    };
    assert_eq!(
        std::fs::read(path).unwrap(),
        bytes,
        "Portable save retains the complete source WAV"
    );
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    app.quit();
}

#[test]
fn meter_choice_stays_disabled_during_audio_capture() {
    let (mut app, source, bytes) = fixture(true);
    app.click("Meter audio");
    let inputs = menu(&mut app, "Window", "Recording");
    assert!(
        inputs["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Mono 1")
    );
    app.click("Arm recording");
    app.click("Arranger");
    app.click("Record");
    let recording = wait(&mut app, |state| state["recording"]["phase"] == "recording");
    let clicked = app.click("4/4");
    assert_eq!(clicked["project"]["beats_per_bar"], 4);
    assert_eq!(clicked["project"], recording["project"]);
    assert!(
        !clicked["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "3/4"),
        "Disabled meter never opens its choices"
    );
    app.click("Stop");
    wait(&mut app, |state| state["recording"]["phase"] == "idle");
    let chosen = menu(&mut app, "4/4", "3/4");
    assert_eq!(chosen["project"]["beats_per_bar"], 3);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    app.quit();
}
