//! Native event queues can deliver a complete short drag between two paints.
//! Kept separate from precise_trim_e2e so that suite also runs on the frozen app.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(app: &mut Harness) -> std::path::PathBuf {
    let source = app.directory.path().join("batched.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 240_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Audio phrase",
        2.125,
        4.0625,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("MIDI");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "MIDI phrase",
        2.125,
        4.0625,
        vec![Note::new(60, 0.5, 1.0, 100)],
    ));
    let path = app.directory.path().join("batched.rill");
    project.save(&path).unwrap();
    path
}

fn geometry(state: &Value, track: usize) -> &Value {
    let id = &state["project"]["tracks"][track]["clips"][0]["id"];
    state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| &item["id"] == id)
        .unwrap()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn batched(app: &mut Harness, from: (f64, f64), delta: f64, shift: bool) -> Value {
    app.command(
        json!({"command":"pointer", "phase":"drag_batched", "x":from.0, "y":from.1,
        "to_x":from.0+delta, "to_y":from.1, "shift":shift}),
    )
}

#[test]
fn complete_short_audio_and_midi_edge_drags_in_one_frame_are_not_clicks() {
    let mut app = Harness::new();
    let path = fixture(&mut app);
    for (track, start, delta) in [(0, false, -4.0), (1, true, 3.0)] {
        app.command(json!({"command":"open", "path":path}));
        app.click("Edit");
        let before = app.click("Select all clips");
        let scale = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
        let rect = &geometry(&before, track)["rect"];
        let from = (
            if start {
                rect["min"][0].as_f64().unwrap() + 2.0
            } else {
                rect["max"][0].as_f64().unwrap() - 2.0
            },
            center(rect).1,
        );
        let trimmed = batched(&mut app, from, delta, true);
        let clip = &trimmed["project"]["tracks"][track]["clips"][0];
        let expected_start = 2.125 + if start { delta / scale } else { 0.0 };
        let expected_length = 4.0625 + if start { -delta / scale } else { delta / scale };
        assert!((clip["start"].as_f64().unwrap() - expected_start).abs() < 1e-5);
        assert!(
            (clip["length"].as_f64().unwrap() - expected_length).abs() < 1e-5,
            "track {track}: expected length {expected_length}, got {clip}"
        );
        assert_eq!(trimmed["selected_clips"], before["selected_clips"]);
        assert_eq!(trimmed["view"], "Arrangement");
        assert_eq!(
            trimmed["project"]["tracks"][1 - track],
            before["project"]["tracks"][1 - track]
        );
        assert_eq!(trimmed["history"]["undo"], json!(["Trim clip"]));
        assert!(trimmed["timeline"]["trim_feedback"].is_null());
        let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
        assert_eq!(undone["project"], before["project"]);
    }
    app.quit();
}

#[test]
fn complete_outward_batch_without_an_accepted_edge_hit_is_left_unchanged_limitation() {
    // Explicit limitation, not a successful trim: egui hit-tests a complete
    // batch at its final position. Ending outside the clip awards the empty
    // row, so no edge owns the gesture. Refuse coordinate-only capture through
    // another widget. Ordinary outward gestures are covered in precise_trim_e2e.
    let mut app = Harness::new();
    let path = fixture(&mut app);
    for track in [0, 1] {
        app.command(json!({"command":"open", "path":path}));
        app.click("Edit");
        let before = app.click("Select all clips");
        let rect = &geometry(&before, track)["rect"];
        let from = (rect["max"][0].as_f64().unwrap() - 2.0, center(rect).1);
        let unsupported = batched(&mut app, from, 4.0, true);
        assert_eq!(unsupported["project"], before["project"]);
        assert_eq!(unsupported["history"], before["history"]);
    }
    app.quit();
}

#[test]
fn batched_zero_motion_fade_overlap_and_popup_presses_cannot_trim() {
    let mut app = Harness::new();
    let path = fixture(&mut app);
    app.command(json!({"command":"open", "path":path}));
    app.click("Edit");
    let before = app.click("Select all clips");
    let rect = &geometry(&before, 1)["rect"];
    let edge = (rect["max"][0].as_f64().unwrap() - 2.0, center(rect).1);
    let clicked = batched(&mut app, edge, 0.0, false);
    assert_eq!(clicked["project"], before["project"]);
    assert_eq!(clicked["history"], before["history"]);
    app.click("Edit");
    let selected = app.click("Select all clips");
    let fade = center(&geometry(&selected, 0)["fade_out_handle"]);
    let faded = batched(&mut app, fade, -3.0, false);
    let audio = &faded["project"]["tracks"][0]["clips"][0];
    assert_eq!(audio["start"], 2.125);
    assert_eq!(audio["length"], 4.0625);
    assert!(faded["timeline"]["trim_feedback"].is_null());
    let before_popup = app.inspect();
    app.click("View");
    let outside = batched(&mut app, edge, -4.0, true);
    assert_eq!(outside["project"], before_popup["project"]);
    assert_eq!(outside["history"], before_popup["history"]);
    app.quit();
}
