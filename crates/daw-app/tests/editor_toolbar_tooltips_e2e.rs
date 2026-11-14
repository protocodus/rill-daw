//! Explanations appear on real hover, including unavailable actions, without edits.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Instrument, Note,
    Project,
};
use serde_json::{Value, json};

fn fixture(dark: bool) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"system_theme","dark":dark}));
    app.command(json!({"command":"viewport","width":1100,"height":950,"pixels_per_point":if dark {2.0} else {1.0}}));
    let mut project = Project::new();
    for (name, instrument, notes) in [
        (
            "Hover keys",
            Instrument::builtin("r1-mini"),
            vec![Note::new(60, 0.0, 0.5, 90)],
        ),
        (
            "Hover pads",
            Instrument::builtin("r1-drums"),
            vec![Note::new(36, 0.0, 0.5, 90), Note::new(60, 1.0, 0.5, 90)],
        ),
    ] {
        let track = project.add_midi_track(name);
        project.set_instrument(track, instrument).unwrap();
        project
            .track_mut(track)
            .unwrap()
            .clips
            .push(Clip::midi(name, 0.0, 8.0, notes));
    }
    let source = app.directory.path().join("hover.wav");
    AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: vec![0.1; 88_200],
    }
    .write_wav(&source)
    .unwrap();
    let audio = project.add_audio_track("Recording");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Hover audio",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    let lane = project
        .add_automation_track(
            "Hover automation",
            AutomationTarget {
                track: audio,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.automation_lane_mut(lane).unwrap().points = vec![
        AutomationPoint::new(0.0, 0.5),
        AutomationPoint::new(4.0, 0.8),
    ];
    let path = app.directory.path().join("toolbar-tooltips.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app
}

fn open_clip(app: &mut Harness, name: &str) {
    app.click("Arranger");
    app.click("Select");
    // A track and its clip may share a name; the clip is the last occurrence.
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|text| *text == name)
        .count()
        - 1;
    app.command(json!({"command":"click","label":name,"occurrence":occurrence}));
    app.click("Window");
    app.click("Clip Editor");
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "song_session",
        "selected_track",
        "selected_clip",
        "selected_notes",
        "cursor",
    ] {
        assert_eq!(state[field], before[field], "Hover changed {field}");
    }
    for editor in ["midi_editor", "audio_editor"] {
        assert_eq!(
            state[editor]["tool"], before[editor]["tool"],
            "Hover changed the active tool"
        );
    }
}

fn hover_at(app: &mut Harness, point: (f64, f64), explanation: &str) {
    let before = app.inspect();
    app.command(json!({"command":"pointer","phase":"move","x":point.0,"y":point.1}));
    let mut shown = app.inspect();
    for _ in 0..40 {
        if shown["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| {
                label
                    .as_str()
                    .is_some_and(|text| text.contains(explanation))
            })
        {
            break;
        }
        shown = app.inspect();
    }
    assert!(
        shown["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label
                .as_str()
                .is_some_and(|text| text.contains(explanation))),
        "Missing hover explanation {explanation:?}: {}",
        shown["visible_labels"]
    );
    unchanged(&shown, &before);
}

fn hover(app: &mut Harness, label: &str, explanation: &str) {
    // A previous tooltip can cover the next control and correctly remove its
    // text from hit-testable label diagnostics. Dismiss it with pointer motion.
    app.command(json!({"command":"pointer","phase":"move","x":2.0,"y":2.0}));
    app.inspect();
    let state = app.inspect();
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    hover_at(app, center(rect), explanation);
}

fn close_popup(app: &mut Harness, _phrase: &str) {
    app.key("Escape");
}

#[test]
fn midi_and_named_note_toolbars_explain_their_actions_and_unavailable_edits_on_hover() {
    for dark in [false, true] {
        let mut app = fixture(dark);
        open_clip(&mut app, "Hover keys");
        for (label, explanation) in [
            ("Select", "Click or box-select notes"),
            ("Pencil", "Click empty space to draw a note"),
            ("Cursor", "Place the playback cursor"),
            ("Cut", "Click a note to split it at the pointer"),
            ("Duplicate", "create an independent copy after it"),
            ("Glue", "Click notes to join them with the selection"),
            ("Erase", "Click a note to delete it"),
            ("Multi-draw", "Draw the chosen chord or scale"),
            ("Snap grid", "Grid step for moving, trimming and drawing"),
            ("Length last", "Initial length for new notes"),
            (
                "Scale guide C · Major",
                "Visual grid and multi-draw harmony",
            ),
            (
                "Clip",
                "Loop playback over this clip or change the timing and pitch",
            ),
            ("Notes", "Edit selected notes: copy, quantize"),
            (
                "Generate…",
                "Create chords, arpeggios or melodies with a live preview",
            ),
            ("View", "Open zoom and display options for this editor"),
            ("Tool help", "Open help for Select"),
            (
                "Fit selection",
                "Select clips, notes or a time range to fit the selection",
            ),
            (
                "Properties",
                "Open this clip’s position, duration, name and other properties",
            ),
        ] {
            hover(&mut app, label, explanation);
        }
        app.click("Notes");
        hover(
            &mut app,
            "Quantize…",
            "Select notes to preview quantizing their timing",
        );
        hover(&mut app, "Paste notes at cursor", "Copy or cut notes first");
        hover(&mut app, "Expression…", "Select exactly one note");
        close_popup(&mut app, "Hover keys");
        app.click("Clip");
        hover(
            &mut app,
            "Loop playback over clip",
            "Set the playback loop to this clip’s start and end",
        );
        close_popup(&mut app, "Hover keys");
        app.click("Scale guide C · Major");
        hover(&mut app, "Root", "Choose the root note for the scale guide");
        hover(
            &mut app,
            "Scale",
            "Choose which pitches the scale guide highlights",
        );
        hover(&mut app, "Chord", "Choose which chord the guide highlights");
        close_popup(&mut app, "Hover keys");
        app.click("Multi-draw");
        hover(
            &mut app,
            "Chord stack",
            "Choose whether multi-draw inserts a chord or the notes of a scale",
        );
        hover(
            &mut app,
            "Major triad",
            "Choose the chord inserted by multi-draw",
        );
        open_clip(&mut app, "Hover pads");
        hover(
            &mut app,
            "Named notes",
            "Use this instrument’s note names or the full piano roll",
        );
        hover(
            &mut app,
            "Other MIDI notes (1)",
            "Show the full piano roll to reveal MIDI notes outside this instrument’s named rows",
        );
        app.quit();
    }
}

#[test]
fn audio_toolbar_and_expanded_fade_and_processing_options_explain_outcomes_and_prerequisites() {
    let mut app = fixture(false);
    open_clip(&mut app, "Hover audio");
    for (label, explanation) in [
        ("Select", "Drag to move; drag an edge to trim"),
        ("Cursor", "drag to select time for copy, cut, or trimming"),
        ("Cut", "Click the waveform to split the clip at the pointer"),
        ("Stretch", "keeping its pitch"),
        ("Slip", "Drag the audio inside fixed clip edges"),
        ("Warp", "Click the waveform to add a warp marker"),
        ("Fade in", "set where the fade-in ends"),
        ("Fade out", "set where the fade-out begins"),
        ("Clip", "Copy, trim, split or arrange this audio clip"),
        ("Process", "Reverse, normalize, detect hits, remove silence"),
        ("Pitch", "Transpose this clip without changing its duration"),
        ("View", "Open zoom and display options for this editor"),
    ] {
        hover(&mut app, label, explanation);
    }
    let state = app.inspect();
    let rect = &state["audio_editor"]["gain_rect"];
    hover_at(
        &mut app,
        (
            (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) * 0.5,
            (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) * 0.5,
        ),
        "Adjust this clip's volume in decibels",
    );
    app.click("Clip");
    hover(
        &mut app,
        "Paste",
        "Copy or cut audio before pasting it at the cursor",
    );
    hover(
        &mut app,
        "Split at cursor",
        "Place the cursor inside the clip to split it",
    );
    hover(
        &mut app,
        "Trim to selection",
        "Select a time range to keep before trimming the clip",
    );
    close_popup(&mut app, "Hover audio");
    app.click("Pitch");
    hover(
        &mut app,
        "Reset pitch",
        "This clip is already at its original pitch",
    );
    close_popup(&mut app, "Hover audio");
    app.click("Fade in");
    hover(
        &mut app,
        "0.00 beats",
        "Set how many beats the audio takes to rise from silence",
    );
    hover(
        &mut app,
        "Linear",
        "Choose how the volume changes through the fade",
    );
    app.quit();
}

#[test]
fn automation_toolbar_and_selected_point_controls_describe_their_effect_without_editing() {
    let mut app = fixture(true);
    app.click("Hover automation");
    app.click("Window");
    app.click("Automation");
    for (label, explanation) in [
        (
            "Linear",
            "Choose how values change between automation points",
        ),
        (
            "Enabled",
            "Enable or bypass this automation lane during playback",
        ),
        (
            "Fit song",
            "Show the full arrangement without changing automation points",
        ),
        (
            "Fit",
            "Zoom to show all automation points without changing their values",
        ),
        (
            "Delete point",
            "Remove the selected automation point or points",
        ),
    ] {
        hover(&mut app, label, explanation);
    }
    let state = app.inspect();
    hover_at(
        &mut app,
        center(&state["automation_editor"]["point_beat_rect"]),
        "Move the selected automation point to a musical position",
    );
    hover_at(
        &mut app,
        center(&state["automation_editor"]["point_value_rect"]),
        "Edit gain in dB",
    );
    app.quit();
}
