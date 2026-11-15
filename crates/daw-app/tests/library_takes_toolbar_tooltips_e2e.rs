//! Browser and take controls explain their actions on real mouse hover.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

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
fn browser_toolbar_explains_filters_actions_and_destinations() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":1480, "height":960}));
    app.click("Window");
    app.click("Device browser");
    for (label, fragments) in [
        ("Instruments", &["MIDI notes", "sound"][..]),
        ("Effects", &["processing audio", "channel"]),
        ("Unverified", &["type", "verified"]),
        ("Manage plug-ins", &["folders", "scan"]),
        ("All categories", &["Filter devices", "category"]),
        ("All developers", &["Filter devices", "developer"]),
        ("View", &["grid", "list"]),
        ("Sort: Name", &["Sort", "devices"]),
        (
            "Add instrument track",
            &["Create", "MIDI track", "instrument"],
        ),
        ("More", &["device", "favorites", "preview"]),
        ("Details", &["device", "information"]),
        ("Device details", &["information", "controls"]),
    ] {
        hover(&mut app, label, fragments);
    }
    app.click("Manage plug-ins");
    for (label, fragments) in [
        ("Plug-in folders…", &["folders", "scans"][..]),
        ("Refresh", &["Rescan", "previews"]),
    ] {
        hover(&mut app, label, fragments);
    }
    app.key("Escape");
    app.quit();
}

#[test]
fn take_toolbar_explains_import_comp_and_disabled_actions() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":1200, "height":900}));
    let path = app.directory.path().join("Original.wav");
    AudioData {
        sample_rate: 48000,
        channels: 1,
        samples: vec![0.1; 192000],
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Voice takes");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Original",
        0.0,
        8.0,
        path.to_string_lossy(),
    ));
    let session = app.directory.path().join("take-hints.rill");
    project.save(&session).unwrap();
    app.command(json!({"command":"open", "path":session}));
    app.click("Original");
    app.click("Window");
    app.click("Clip Editor");
    app.click("Clip");
    app.click("Takes…");
    for (label, fragments) in [
        ("Add takes…", &["Import", "alternate takes"][..]),
        ("Group overlapping clips", &["at least two", "overlapping"]),
        ("View", &["waveform", "spectrogram", "takes"]),
        ("Time selection", &["time range", "whole take"]),
        ("Use whole take", &["Add or group", "takes first"]),
        ("Layer as overdub", &["separate audio track", "Both tracks"]),
        ("Back to editor", &["Close Audio takes", "Clip Editor"]),
    ] {
        hover(&mut app, label, fragments);
    }
    app.quit();
}
