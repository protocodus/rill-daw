//! Fast chords retain their key-event modifiers even after all keys are up.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn chord(app: &mut Harness, key: &str, shift: bool, repetitions: usize) -> Value {
    app.command(json!({
        "command":"key", "key":key, "cmd":true, "shift":shift,
        "release_modifiers":true, "repetitions":repetitions,
    }))
}

fn clips(state: &Value) -> &[Value] {
    state["project"]["tracks"][0]["clips"].as_array().unwrap()
}

fn audio() -> Harness {
    let mut app = Harness::new();
    let path = app.directory.path().join("Drum phrase.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..48_000)
            .map(|i| (i as f32 * 0.025).sin() * 0.2)
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    app.command(json!({"command":"drop", "paths":[path], "beat":0.0}));
    app
}

#[test]
fn released_shortcut_modifiers_still_duplicate_audio_and_undo_redo() {
    let mut app = audio();
    let before = app.inspect();
    let changed = chord(&mut app, "D", false, 1);
    assert_eq!(clips(&changed).len(), 2);
    assert_eq!(clips(&changed)[1]["start"], 2.0);
    assert_ne!(clips(&changed)[0]["id"], clips(&changed)[1]["id"]);
    assert_eq!(changed["selected_clip"], clips(&changed)[1]["id"]);
    assert_eq!(clips(&changed)[0]["content"], clips(&changed)[1]["content"]);

    let undone = chord(&mut app, "Z", false, 1);
    assert_eq!(undone["project"], before["project"]);
    let redone = chord(&mut app, "Z", true, 1);
    assert_eq!(redone["project"], changed["project"]);
    chord(&mut app, "Z", false, 1);
    let alternate_redo = chord(&mut app, "Y", false, 1);
    assert_eq!(alternate_redo["project"], changed["project"]);
    app.quit();
}

#[test]
fn several_duplicate_presses_between_repaints_are_not_collapsed() {
    for release_modifiers in [false, true] {
        let mut app = audio();
        let before = app.inspect();
        let changed = app.command(json!({
            "command":"key", "key":"D", "cmd":true,
            "release_modifiers":release_modifiers, "repetitions":3,
        }));
        assert_eq!(clips(&changed).len(), 4);
        for (index, clip) in clips(&changed).iter().enumerate() {
            assert_eq!(clip["start"], index as f64 * 2.0);
            assert_eq!(clip["length"], 2.0);
            assert_eq!(clip["content"], clips(&before)[0]["content"]);
        }
        let ids: std::collections::HashSet<_> = clips(&changed)
            .iter()
            .map(|clip| clip["id"].as_u64().unwrap())
            .collect();
        assert_eq!(ids.len(), 4);
        assert_eq!(changed["selected_clip"], clips(&changed)[3]["id"]);
        // Edits received in one frame share one reversible history checkpoint.
        assert_eq!(chord(&mut app, "Z", false, 1)["project"], before["project"]);
        assert_eq!(chord(&mut app, "Z", true, 1)["project"], changed["project"]);
        app.quit();
    }
}

#[test]
fn repeated_fast_undo_and_redo_process_each_key_press() {
    let mut app = audio();
    let before = app.inspect();
    chord(&mut app, "D", false, 1);
    let changed = chord(&mut app, "D", false, 1);
    assert_eq!(clips(&changed).len(), 3);
    assert_eq!(chord(&mut app, "Z", false, 2)["project"], before["project"]);
    assert_eq!(chord(&mut app, "Z", true, 2)["project"], changed["project"]);
    app.quit();
}

#[test]
fn fast_select_all_targets_midi_notes_then_duplicates_the_complete_phrase() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Shortcut synth");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Shortcut melody",
        0.0,
        4.0,
        vec![
            Note::new(60, 0.0, 0.5, 90),
            Note::new(64, 0.5, 0.5, 100),
            Note::new(67, 1.5, 0.5, 110),
        ],
    ));
    let path = app.directory.path().join("Shortcut melody.rill");
    project.save(&path).unwrap();
    let opened = app.command(json!({"command":"open", "path":path}));
    // The saved filename now also appears in the project header. Select the
    // actual clip body, keeping this test about shortcut focus and note edits.
    let clip = &opened["timeline"]["clips"][0]["rect"];
    let x = (clip["min"][0].as_f64().unwrap() + clip["max"][0].as_f64().unwrap()) * 0.5;
    let y = (clip["min"][1].as_f64().unwrap() + clip["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.click("Window");
    app.click("Clip Editor");
    let before = app.inspect();
    chord(&mut app, "A", false, 1);
    let changed = chord(&mut app, "D", false, 1);
    assert_eq!(
        clips(&changed).len(),
        1,
        "The selection is notes, not the clip"
    );
    let notes = clips(&changed)[0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(notes.len(), 6);
    for (pitch, start, velocity) in [(60, 2.0, 90), (64, 2.5, 100), (67, 3.5, 110)] {
        assert!(notes.iter().any(|note| {
            note["pitch"] == pitch
                && note["start"] == start
                && note["length"] == 0.5
                && note["velocity"] == velocity
        }));
    }
    assert_eq!(chord(&mut app, "Z", false, 1)["project"], before["project"]);
    app.quit();
}

#[test]
fn text_focus_and_unsaved_document_modal_keep_shortcuts_in_their_context() {
    let mut app = audio();
    app.click("Track");
    app.click("Add audio track");
    app.click("Window");
    app.click("Properties");
    let state = app.inspect();
    let names = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == "New audio")
        .count();
    assert!(names > 0);
    app.command(json!({"command":"click", "label":"New audio", "occurrence":names - 1}));
    let before = app.inspect();
    let focused = chord(&mut app, "Z", false, 1);
    assert_eq!(
        focused["project"], before["project"],
        "Text undo must not remove the track"
    );
    assert_eq!(focused["history"], before["history"]);
    let focused = chord(&mut app, "D", false, 1);
    assert_eq!(focused["project"], before["project"]);
    assert_eq!(focused["history"], before["history"]);
    chord(&mut app, "A", false, 1);
    let renamed = app.command(json!({"command":"text", "text":"Focused name"}));
    assert_eq!(renamed["project"]["tracks"][1]["name"], "Focused name");
    app.key("Escape");
    app.click("File");
    let guarded = app.click("New");
    assert_eq!(guarded["document_guard"]["phase"], "confirm");
    for (key, shift) in [("D", false), ("Z", false), ("Z", true), ("Y", false)] {
        let blocked = chord(&mut app, key, shift, 1);
        assert_eq!(blocked["project"], guarded["project"]);
        assert_eq!(blocked["history"], guarded["history"]);
        assert_eq!(blocked["document_guard"]["phase"], "confirm");
    }
    app.click("Cancel");
    app.quit();
}
