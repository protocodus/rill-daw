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

fn disclosed_hint(
    app: &mut Harness,
    label: &str,
    key: &str,
    fragment: &str,
    full_hint: &str,
    before: &Value,
) {
    let opened = app.click("Tool help");
    unchanged(&opened, before);
    // The help remains readable without hovering the opener, and repeated
    // ordinary frames must leave the underlying musical canvas in place.
    let state = app.command(json!({"command":"pointer", "phase":"move", "x":2.0, "y":2.0}));
    let title = format!("{label} · {key}");
    hint(&state, &title, "");
    let panel = hint(&state, full_hint, fragment);
    let left = panel["min"][0].as_f64().unwrap();
    let top = panel["min"][1].as_f64().unwrap();
    let right = panel["max"][0].as_f64().unwrap();
    let bottom = panel["max"][1].as_f64().unwrap();
    assert!(left >= 0.0 && top >= 0.0);
    assert!(right <= state["viewport"][0].as_f64().unwrap());
    assert!(bottom <= state["viewport"][1].as_f64().unwrap());
    unchanged(&state, before);
    let later = app.inspect();
    assert_eq!(hint(&later, full_hint, fragment)["text"], panel["text"]);
    unchanged(&later, before);
    let closed = app.key("Escape");
    unchanged(&closed, before);
    assert!(
        !closed["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label.as_str() == Some(full_hint)),
        "Dismissed help must not leave its full instructions on the canvas"
    );
}

fn tools(app: &mut Harness, context: daw_ui::ToolHintContext, choices: &[(&str, &str, &str)]) {
    let before = app.inspect();
    for (label, key, fragment) in choices {
        let tool = daw_ui::EditorTool::ALL
            .into_iter()
            .find(|tool| tool.key().name() == *key)
            .unwrap();
        let full_hint = context.hint(tool);
        app.click(label);
        disclosed_hint(app, label, key, fragment, full_hint, &before);
        app.key(if *key == "Q" { "P" } else { "Q" });
        app.key(key);
        disclosed_hint(app, label, key, fragment, full_hint, &before);
    }
}

#[test]
fn tool_help_discloses_all_supported_gestures_and_updates_for_pointer_and_keyboard() {
    for (width, dark, density) in [(800, false, 1.0), (1200, true, 2.0)] {
        let mut app = fixture(width, dark, density);
        tools(
            &mut app,
            daw_ui::ToolHintContext::Arrangement,
            &[
                ("Select", "Q", "Click or box-select clips"),
                ("Pencil", "W", "create a MIDI clip"),
                ("Cut", "E", "Click a clip to split"),
                ("Duplicate", "R", "selected group as independent copies"),
                ("Join", "T", "choose the clips to join"),
                ("Ghost copy", "G", "Source edits stay shared"),
                ("Stretch", "S", "MIDI note timing scales"),
                ("Slip", "L", "inside fixed clip edges"),
                ("Erase", "Z", "delete only that clip"),
                ("Cursor", "P", "select a phrase, including rests"),
            ],
        );
        app.click("Select");
        app.click("Hint phrase");
        window(&mut app, "Clip Editor");
        tools(
            &mut app,
            daw_ui::ToolHintContext::Midi,
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
            daw_ui::ToolHintContext::Audio,
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

fn pointer_at_rect(app: &mut Harness, rect: &Value, button: &str) -> Value {
    app.command(json!({
        "command": "pointer", "phase": "click", "button": button,
        "x": (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        "y": (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    }))
}

fn clip_rect(state: &Value, id: &Value) -> Value {
    state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == *id)
        .unwrap()["rect"]
        .clone()
}

fn caption(app: &mut Harness, gesture: &str) -> Value {
    // Guidance must remain visible after the pointer leaves its tool button.
    let state = app.command(json!({"command":"pointer", "phase":"move", "x":2.0, "y":2.0}));
    let rect = hint(&state, gesture, "");
    assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
    assert!(rect["max"][0].as_f64().unwrap() <= state["viewport"][0].as_f64().unwrap());
    state
}

fn exact_label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

#[test]
fn duplicate_caption_leads_to_a_canvas_gesture_without_changing_the_command() {
    let mut app = fixture(1440, true, 1.0);
    let before = app.click("Hint phrase");
    let source = before["selected_clip"].clone();
    let selected = app.click("Duplicate");
    unchanged(&selected, &before);
    let selected = caption(&mut app, "Click a clip to duplicate");
    unchanged(&selected, &before);
    // Clicking the toolbar again still selects a mode; it never edits the song.
    unchanged(&app.click("Duplicate"), &before);
    let copied = pointer_at_rect(&mut app, &clip_rect(&selected, &source), "primary");
    assert_eq!(
        copied["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let copy = copied["selected_clip"].clone();
    assert_ne!(copy, source);
    assert!(copied["project"]["tracks"][0]["clips"][1]["link_group"].is_null());
    assert_eq!(copied["timeline"]["tool"], "Duplicate");
    let repeated = pointer_at_rect(&mut app, &clip_rect(&copied, &copy), "primary");
    assert_eq!(
        repeated["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    app.click("Select");
    let selected = pointer_at_rect(&mut app, &clip_rect(&repeated, &source), "primary");
    assert_eq!(selected["timeline"]["tool"], "Select");
    pointer_at_rect(&mut app, &clip_rect(&selected, &source), "secondary");
    let menu_copy = app.click("Duplicate");
    assert_eq!(
        menu_copy["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(menu_copy["timeline"]["tool"], "Select");

    let selected = pointer_at_rect(&mut app, &clip_rect(&menu_copy, &source), "primary");
    assert_eq!(selected["selected_clip"], source);
    let before = window(&mut app, "Clip Editor");
    unchanged(&app.click("Duplicate"), &before);
    let selected = caption(&mut app, "Click a note to duplicate");
    let note = &selected["midi_editor"]["notes"][0]["rect"];
    let copied = pointer_at_rect(&mut app, note, "primary");
    let notes = copied["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(notes.len(), 2);
    assert_ne!(notes[0]["id"], notes[1]["id"]);
    assert_eq!(notes[1]["start"].as_f64().unwrap(), 1.0);
    let before = app.click("Select");
    let state = pointer_at_rect(
        &mut app,
        &copied["midi_editor"]["notes"][0]["rect"],
        "primary",
    );
    assert_eq!(state["project"], before["project"]);
    app.quit();
}

#[test]
fn active_tool_captions_preserve_toolbar_and_canvas_geometry_in_each_editor() {
    use daw_ui::{EditorTool, ToolHintContext};
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = fixture(1440, dark, density);
        for width in [800, 1100, 1440] {
            app.command(json!({"command":"viewport", "width":width, "height":900, "pixels_per_point":density}));
            for context in [
                ToolHintContext::Arrangement,
                ToolHintContext::Midi,
                ToolHintContext::Audio,
            ] {
                app.click("Arranger");
                app.click("Select");
                if context != ToolHintContext::Arrangement {
                    app.click(if context == ToolHintContext::Midi {
                        "Hint phrase"
                    } else {
                        "Hint recording"
                    });
                    window(&mut app, "Clip Editor");
                    app.click("Select");
                }
                let before = app.inspect();
                for tool in [EditorTool::Cut, EditorTool::Cursor, EditorTool::Select] {
                    unchanged(&app.click(context.label(tool)), &before);
                    let state = caption(&mut app, context.gesture(tool));
                    for label in ["Tool help", "View", "Fit"] {
                        assert_eq!(
                            exact_label(&state, label),
                            exact_label(&before, label),
                            "{context:?} {tool:?} moved {label} at {width}"
                        );
                    }
                    for field in ["timeline", "midi_editor", "audio_editor"] {
                        for rect in ["ruler", "plot", "canvas"] {
                            assert_eq!(
                                state[field][rect], before[field][rect],
                                "{context:?} {tool:?} moved {field}.{rect} at {width}"
                            );
                        }
                    }
                    unchanged(&state, &before);
                }
                if context != ToolHintContext::Audio {
                    unchanged(&app.click("Duplicate"), &before);
                    caption(&mut app, context.gesture(EditorTool::Duplicate));
                }
            }
        }
        app.quit();
    }
}
