//! Held mouse trims expose their actual applied timing without editing state.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(app: &mut Harness) -> std::path::PathBuf {
    let source = app.directory.path().join("Phrase.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: vec![0.1; 240_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 93.5;
    let audio = project.add_audio_track("Audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Audio phrase",
        2.25,
        4.0,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("MIDI");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "MIDI phrase",
        2.25,
        4.0,
        vec![Note::new(60, 0.5, 1.0, 100)],
    ));
    let path = app.directory.path().join("Trim feedback.rill");
    project.save(&path).unwrap();
    path
}

fn pointer(app: &mut Harness, phase: &str, x: f64, y: f64) -> Value {
    app.command(json!({"command":"pointer", "phase":phase, "x":x, "y":y}))
}

fn clip_rect(state: &Value, track: usize) -> &Value {
    let id = &state["project"]["tracks"][track]["clips"][0]["id"];
    &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| &clip["id"] == id)
        .unwrap()["rect"]
}

fn reveal_clip(app: &mut Harness, track: usize) -> Value {
    let before = app.inspect();
    let mut state = before.clone();
    for _ in 0..8 {
        let rect = clip_rect(&state, track);
        let canvas = &state["timeline"]["canvas"];
        let top = canvas["min"][1].as_f64().unwrap();
        let bottom = canvas["max"][1].as_f64().unwrap();
        let clip_top = rect["min"][1].as_f64().unwrap();
        let clip_bottom = rect["max"][1].as_f64().unwrap();
        if clip_top >= top && clip_bottom <= bottom {
            assert_eq!(state["project"], before["project"]);
            assert_eq!(state["selected_clips"], before["selected_clips"]);
            assert_eq!(state["history"], before["history"]);
            return state;
        }
        let x = (canvas["min"][0].as_f64().unwrap() + canvas["max"][0].as_f64().unwrap()) * 0.5;
        let y = (top + bottom) * 0.5;
        let delta = if clip_bottom > bottom {
            bottom - clip_bottom - 8.0
        } else {
            top - clip_top + 8.0
        };
        state = app.command(json!({"command":"scroll", "x":x, "y":y, "delta":delta}));
    }
    panic!(
        "Could not reveal clip {track} for trimming: {}",
        clip_rect(&state, track)
    );
}

fn assert_feedback(state: &Value, track: usize, width: f64, height: f64) {
    let clip = &state["project"]["tracks"][track]["clips"][0];
    let start = clip["start"].as_f64().unwrap();
    let length = clip["length"].as_f64().unwrap();
    let tempo = state["project"]["tempo"].as_f64().unwrap();
    let feedback = &state["timeline"]["trim_feedback"];
    assert_eq!(feedback["clip"], clip["id"]);
    assert_eq!(feedback["end_beat"], start + length + 1.0);
    assert_eq!(feedback["duration_beats"], length);
    assert!(
        (feedback["end_seconds"].as_f64().unwrap() - (start + length) * 60.0 / tempo).abs() < 1e-10
    );
    assert!((feedback["duration_seconds"].as_f64().unwrap() - length * 60.0 / tempo).abs() < 1e-10);
    let expected = format!(
        "End {:.3} s · Beat {:.2}\nDuration {:.3} s · {:.2} beats",
        (start + length) * 60.0 / tempo,
        start + length + 1.0,
        length * 60.0 / tempo,
        length,
    );
    assert_eq!(feedback["text"], expected);
    assert!(
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == &expected),
        "Feedback must actually be painted: {feedback}"
    );
    let rect = &feedback["rect"];
    assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
    assert!(rect["min"][1].as_f64().unwrap() >= 0.0);
    assert!(rect["max"][0].as_f64().unwrap() <= width);
    assert!(rect["max"][1].as_f64().unwrap() <= height);
    assert!(rect["max"][0].as_f64().unwrap() > rect["min"][0].as_f64().unwrap());
}

fn assert_no_feedback(state: &Value) {
    assert!(state["timeline"]["trim_feedback"].is_null());
    assert!(
        !state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .any(|label| label.starts_with("End ") && label.contains("\nDuration "))
    );
}

#[test]
fn held_audio_and_midi_trims_report_applied_bounds_without_changing_selection_or_undo() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let path = fixture(&mut app);
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [800, 1440] {
            app.command(json!({"command":"viewport", "width":width, "height":800}));
            for track in [0, 1] {
                for trim_start in [false, true] {
                    app.command(json!({"command":"open", "path":path}));
                    app.click("Edit");
                    let before = app.click("Select all clips");
                    assert_no_feedback(&before);
                    let rect = clip_rect(&before, track);
                    let x = if trim_start {
                        rect["min"][0].as_f64().unwrap() + 2.0
                    } else {
                        rect["max"][0].as_f64().unwrap() - 2.0
                    };
                    let y =
                        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
                    assert_no_feedback(&pointer(&mut app, "down", x, y));
                    let zoom = before["timeline"]["pixels_per_beat"].as_f64().unwrap();
                    let target_x = x + if trim_start { zoom } else { -zoom };
                    let held = pointer(&mut app, "move", target_x, y);
                    assert_feedback(&held, track, width as f64, 800.0);
                    let clip = &held["project"]["tracks"][track]["clips"][0];
                    assert_eq!(clip["length"], 3.0);
                    assert_eq!(clip["start"], if trim_start { 3.25 } else { 2.25 });
                    let stationary = app.inspect();
                    assert_feedback(&stationary, track, width as f64, 800.0);
                    assert_eq!(stationary["project"], held["project"]);
                    assert_eq!(stationary["selected_clips"], before["selected_clips"]);
                    let released = pointer(&mut app, "up", target_x, y);
                    assert_no_feedback(&released);
                    assert_eq!(released["project"], held["project"]);
                    assert_eq!(released["selected_clips"], before["selected_clips"]);
                    assert_eq!(released["history"]["undo"], json!(["Trim clip"]));
                    app.click("Edit");
                    let undone = app.click("Undo");
                    assert_eq!(undone["project"], before["project"]);
                    assert_no_feedback(&undone);
                }
            }
        }
        app.quit();
    }
}

#[test]
fn trim_feedback_stays_inside_the_viewport_at_pointer_edges_and_does_not_block_release() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let path = fixture(&mut app);
        app.command(json!({"command":"system_theme", "dark":dark}));
        for (width, height) in [(800, 600), (1440, 900)] {
            app.command(json!({"command":"viewport", "width":width, "height":height}));
            for track in [0, 1] {
                app.command(json!({"command":"open", "path":path}));
                // Compact transport/header rows can leave the second lane partly
                // below the viewport. Reveal it with the mouse before trimming.
                let before = reveal_clip(&mut app, track);
                let rect = clip_rect(&before, track);
                let x = rect["max"][0].as_f64().unwrap() - 2.0;
                let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
                pointer(&mut app, "down", x, y);
                pointer(&mut app, "move", x + 20.0, y);
                let held = pointer(&mut app, "move", width as f64 - 2.0, height as f64 - 2.0);
                assert_feedback(&held, track, width as f64, height as f64);
                let released = pointer(&mut app, "up", width as f64 - 2.0, height as f64 - 2.0);
                assert_no_feedback(&released);
                assert_eq!(released["project"], held["project"]);
                assert_eq!(released["history"]["undo"], json!(["Trim clip"]));
            }
        }
        app.quit();
    }
}
