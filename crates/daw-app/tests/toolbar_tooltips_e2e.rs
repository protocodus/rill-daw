//! Toolbar explanations appear on real mouse hover without editing the song.
mod common;

use common::{Harness, wait_until};
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn fixture() -> Harness {
    let mut app = Harness::with_virtual_audio(None, None);
    let mut project = Project::new();
    let track = project.add_midi_track("Tooltip keys");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Tooltip phrase",
        0.0,
        16.0,
        vec![Note::new(60, 0.0, 1.0, 90)],
    ));
    let path = app.directory.path().join("toolbar-tooltips.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Tooltip phrase");
    app
}

fn unchanged(state: &Value, before: &Value, control: &str) {
    for field in [
        "project",
        "history",
        "modified",
        "selected_track",
        "selected_clip",
        "selected_clips",
        "selected_notes",
        "selection_span",
        "view",
        "popups",
    ] {
        assert_eq!(
            state[field], before[field],
            "Hovering {control} changed {field}"
        );
    }
}

fn explains(text: &str, fragments: &[&str]) -> bool {
    let text = text.to_lowercase();
    fragments
        .iter()
        .all(|fragment| text.contains(&fragment.to_lowercase()))
}

fn hover(app: &mut Harness, label: &str, fragments: &[&str]) -> Value {
    // Clear the previous tooltip before locating the next control. A wide
    // tooltip must not intercept the mouse route to an adjacent button.
    let state = app.inspect();
    app.command(json!({"command":"pointer", "phase":"move",
        "x":state["viewport"][0].as_f64().unwrap()-2.0,
        "y":state["viewport"][1].as_f64().unwrap()-2.0}));
    let before = app.inspect();
    let rect = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["text"] == label)
        .min_by(|a, b| {
            a["min"][1]
                .as_f64()
                .unwrap()
                .total_cmp(&b["min"][1].as_f64().unwrap())
        })
        .unwrap_or_else(|| panic!("Missing {label}: {}", before["visible_labels"]));
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let moved = app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
    unchanged(&moved, &before, label);
    let mut last = moved;
    // Ordinary application frames advance the production hover delay. There
    // is no tooltip-specific command, timing override or wall-clock sleep.
    for _ in 0..90 {
        last = app.command(json!({"command":"inspect", "paint":true}));
        unchanged(&last, &before, label);
        let visible = last["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .any(|text| text != label && explains(text, fragments));
        let painted = last["painted_text"].as_array().unwrap().iter().any(|text| {
            text["rendered_text"]
                .as_str()
                .is_some_and(|text| text != label && explains(text, fragments))
        });
        if visible && painted {
            return last;
        }
    }
    panic!(
        "{label} never painted an explanation containing {fragments:?}: {}",
        last["visible_labels"]
    );
}

#[test]
fn transport_and_arranger_controls_explain_their_actions_without_edits() {
    let mut app = fixture();
    for (width, density, dark) in [(800, 1.0, false), (1440, 2.0, true)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":width, "height":900,
            "pixels_per_point":density}));
        for (label, fragments) in [
            ("Arranger", &["arrange", "clips", "timeline"][..]),
            ("Mixer", &["volume", "pan", "route audio"]),
            ("Start", &["beginning", "song"]),
            ("Play", &["play", "current cursor", "space"]),
            ("Stop", &["stop playback", "beginning", "instruments"]),
            ("Record", &["record", "armed", "inputs"]),
            ("120.0 BPM", &["tempo", "drag"]),
            ("4/4", &["beats per bar", "bar lines"]),
            ("Click", &["metronome", "pre-count"]),
            ("Loop", &["repeat playback", "range"]),
            ("Range", &["loop start and end", "duration"]),
            ("Add pattern", &["midi pattern", "cursor"]),
            ("Snap grid", &["grid step", "moving", "drawing"]),
            ("Nudge", &["move selected clips", "millisecond", "sample"]),
        ] {
            hover(&mut app, label, fragments);
        }
        app.click("Range");
        hover(
            &mut app,
            "Loop playback over selection",
            &["playback looping", "selected clips", "rests"],
        );
        app.key("Escape");
    }

    app.click("Play");
    wait_until(&mut app, |state| state["engine"]["playing"] == true);
    let playing = hover(&mut app, "Pause", &["pause playback", "keep", "position"]);
    assert_eq!(
        playing["engine"]["playing"], true,
        "Hover must not pause playback"
    );
    app.click("Pause");
    app.quit();
}

#[test]
fn mixer_header_explains_navigation_channel_order_and_routing_without_edits() {
    let mut app = fixture();
    app.command(json!({"command":"viewport", "width":1440, "height":900}));
    app.click("Mixer");
    for (label, fragments) in [
        ("Show in Arranger", &["arrangement", "reveal", "track"][..]),
        ("Song order", &["choose", "channels", "arranged"]),
        (
            "Add send…",
            &["choose a bus", "post-fader send", "tooltip keys"],
        ),
        (
            "Add bus",
            &["bus channel", "combine tracks", "effect sends"],
        ),
    ] {
        hover(&mut app, label, fragments);
    }
    app.quit();
}
