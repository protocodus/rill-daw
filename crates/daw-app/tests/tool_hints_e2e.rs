//! On-demand tool help stays reachable without taking space from the musical canvas.
mod common;

use common::Harness;
use daw_core::{AudioData, AutomationParameter, AutomationTarget, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(width: u32, dark: bool, density: f32) -> Harness {
    let mut app = Harness::new();
    app.command(
        json!({"command":"viewport", "width":width, "height":900, "pixels_per_point":density}),
    );
    app.command(json!({"command":"system_theme", "dark":dark}));
    let mut project = Project::new();
    let midi = project.add_midi_track("Hint notes");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Hint phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let source = app.directory.path().join("Original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let audio = project.add_audio_track("Hint audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Hint recording",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    project
        .add_automation_track(
            "Hint automation",
            AutomationTarget {
                track: audio,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let path = app.directory.path().join("Hints.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app
}

fn window(app: &mut Harness, label: &str) -> Value {
    app.click("Window");
    app.click(label)
}

fn hint<'a>(state: &'a Value, title: &str, fragment: &str) -> &'a Value {
    let matches: Vec<_> = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| {
            let text = label["text"].as_str().unwrap_or_default();
            text.starts_with(title) && text.contains(fragment)
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "Missing or repeated tool help {title}: {}",
        state["visible_labels"]
    );
    matches[0]
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "selected_track",
        "selected_clip",
        "selected_notes",
        "song_session",
    ] {
        assert_eq!(state[field], before[field], "Tool hint changed {field}");
    }
}

fn disclosed_hint(app: &mut Harness, label: &str, key: &str, fragment: &str, before: &Value) {
    let opened = app.click("Tool help");
    unchanged(&opened, before);
    // The help remains readable without hovering the opener, and repeated
    // ordinary frames must leave the underlying musical canvas in place.
    let state = app.command(json!({"command":"pointer", "phase":"move", "x":2.0, "y":2.0}));
    let title = format!("{label} · {key}");
    hint(&state, &title, "");
    let panel = hint(&state, "", fragment);
    let left = panel["min"][0].as_f64().unwrap();
    let top = panel["min"][1].as_f64().unwrap();
    let right = panel["max"][0].as_f64().unwrap();
    let bottom = panel["max"][1].as_f64().unwrap();
    assert!(left >= 0.0 && top >= 0.0);
    assert!(right <= state["viewport"][0].as_f64().unwrap());
    assert!(bottom <= state["viewport"][1].as_f64().unwrap());
    unchanged(&state, before);
    let later = app.inspect();
    assert_eq!(hint(&later, "", fragment)["text"], panel["text"]);
    unchanged(&later, before);
    let closed = app.key("Escape");
    unchanged(&closed, before);
    assert!(
        !closed["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| { label.as_str().is_some_and(|text| text.contains(fragment)) }),
        "Dismissed help must not leave a permanent instruction band"
    );
}

fn tools(app: &mut Harness, choices: &[(&str, &str, &str)]) {
    let before = app.inspect();
    for (label, key, fragment) in choices {
        app.click(label);
        disclosed_hint(app, label, key, fragment, &before);
        app.key(if *key == "Q" { "P" } else { "Q" });
        app.key(key);
        disclosed_hint(app, label, key, fragment, &before);
    }
}

#[test]
fn tool_help_discloses_all_supported_gestures_and_updates_for_pointer_and_keyboard() {
    for (width, dark, density) in [(800, false, 1.0), (1200, true, 2.0)] {
        let mut app = fixture(width, dark, density);
        tools(
            &mut app,
            &[
                ("Select", "Q", "Click or box-select clips"),
                ("Pencil", "W", "create a MIDI clip"),
                ("Cut", "E", "Click a clip to split"),
                ("Duplicate", "R", "selected group as independent copies"),
                ("Join", "T", "choose the clips to join"),
                ("Ghost copy", "G", "Source edits stay shared"),
                ("Stretch", "S", "MIDI note timing scales"),
                ("Slip", "L", "inside fixed clip edges"),
                ("Erase", "Z", "delete the selected clips"),
                ("Cursor", "P", "select a phrase, including rests"),
            ],
        );
        app.click("Select");
        app.click("Hint phrase");
        window(&mut app, "Clip Editor");
        tools(
            &mut app,
            &[
                ("Select", "Q", "Shift-click toggles"),
                ("Pencil", "W", "same pitch merge on release"),
                ("Cut", "E", "Click a note to split"),
                ("Duplicate", "R", "independent copy after it"),
                ("Glue", "T", "join them with the selection"),
                ("Erase", "Z", "drag across notes to erase"),
                ("Multi-draw", "U", "chosen chord or scale"),
                ("Cursor", "P", "Place the playback cursor."),
            ],
        );
        app.click("Arranger");
        app.click("Select");
        app.click("Hint recording");
        window(&mut app, "Clip Editor");
        tools(
            &mut app,
            &[
                ("Select", "Q", "drag an edge to trim"),
                ("Cursor", "P", "drag to select time"),
                ("Cut", "E", "Click the waveform to split"),
                ("Stretch", "S", "keeping its pitch"),
                ("Slip", "L", "inside fixed clip edges"),
                ("Fade in", "I", "where the fade-in ends"),
                ("Fade out", "O", "where the fade-out begins"),
            ],
        );
        app.click("Arranger");
        app.click("Hint automation");
        let before = window(&mut app, "Automation");
        app.click("Tool help");
        app.command(json!({"command":"pointer","phase":"move","x":2.0,"y":2.0}));
        let state = app.inspect();
        let guidance = hint(&state, "", "Click to add; drag points");
        assert!(
            guidance["text"]
                .as_str()
                .unwrap()
                .contains("diamonds to bend. Right-click for options")
        );
        assert!(guidance["max"][0].as_f64().unwrap() <= f64::from(width));
        unchanged(&state, &before);
        // Interpolation is a parameter choice, not a separate pointer tool.
        assert_eq!(
            hint(&app.inspect(), "", "drag points to move")["text"],
            guidance["text"]
        );
        app.quit();
    }
}

#[test]
fn compact_help_preserves_canvas_space_and_toggles_with_the_mouse() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = fixture(800, dark, density);
        app.command(
            json!({"command":"viewport", "width":800, "height":600, "pixels_per_point":density}),
        );
        let before = app.inspect();
        assert!(
            !before["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label
                    .as_str()
                    .is_some_and(|value| value.contains("Ctrl/Cmd+arrows nudge")))
        );
        let expanded = app.click("Tool help");
        hint(&expanded, "", "Ctrl/Cmd+arrows nudge");
        assert_eq!(
            expanded["timeline"]["ruler"], before["timeline"]["ruler"],
            "On-demand help must not move or shrink the canvas"
        );
        unchanged(&expanded, &before);
        let collapsed =
            app.command(json!({"command":"pointer", "phase":"click", "x":2.0, "y":2.0}));
        assert_eq!(collapsed["timeline"]["ruler"], before["timeline"]["ruler"]);
        unchanged(&collapsed, &before);
        assert!(
            !collapsed["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| {
                    label
                        .as_str()
                        .is_some_and(|text| text.contains("Ctrl/Cmd+arrows nudge"))
                })
        );
        app.click("Tool help");
        let dismissed = app.key("Escape");
        unchanged(&dismissed, &before);
        app.quit();
    }
}
