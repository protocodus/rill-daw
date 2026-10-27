//! View shortcuts belong to the foreground editor even over its toolbar.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Note, Project,
};
use serde_json::{Value, json};

fn fixture() -> Harness {
    let mut app = Harness::new();
    let mut project = Project::new();
    let midi = project.add_midi_track("Foreground synth");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Foreground phrase",
        0.0,
        20.0,
        vec![Note::new(60, 0.0, 1.0, 100), Note::new(72, 16.0, 2.0, 90)],
    ));
    let source = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let audio = project.add_audio_track("Foreground audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Foreground recording",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    let automation = project
        .add_automation_track(
            "Foreground automation",
            AutomationTarget {
                track: audio,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project
        .track_mut(automation)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points = vec![
        AutomationPoint::new(32.0, 0.25),
        AutomationPoint::new(64.0, 0.75),
    ];
    let path = app.directory.path().join("foreground.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(state[field], before[field], "Navigation changed {field}");
    }
}

fn framing(state: &Value, editor: &str) -> Value {
    let view = &state[editor];
    json!({
        "start":view["start_beat"], "zoom":view["pixels_per_beat"],
        "row":view["row_offset"], "height":view["row_height"],
    })
}

fn toolbar_keys(app: &mut Harness, editor: &str) {
    let fitted = app.click("Fit");
    let zoom = fitted[editor]["pixels_per_beat"].as_f64().unwrap();
    // The pointer never enters or clicks the canvas in this editor.
    app.command(json!({"command":"pointer", "phase":"move", "x":20.0, "y":20.0}));
    let plus = app.key("Equals");
    assert!(plus[editor]["pixels_per_beat"].as_f64().unwrap() > zoom);
    unchanged(&plus, &fitted);
    let minus = app.key("Minus");
    assert!(
        (minus[editor]["pixels_per_beat"].as_f64().unwrap() - zoom).abs() < 1e-3,
        "{editor} minus must undo the view zoom"
    );
    unchanged(&minus, &fitted);
    app.key("Plus");
    let fit = app.key("F");
    assert_eq!(framing(&fit, editor), framing(&fitted, editor));
    unchanged(&fit, &fitted);
    if editor == "midi_editor" {
        let selection = |state: &Value| {
            state[editor]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|note| (note["id"].clone(), note["selected"].clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(selection(&fit), selection(&fitted));
    }
    if editor == "automation_editor" {
        assert_eq!(fit[editor]["point_beat"], fitted[editor]["point_beat"]);
    }
}

fn window(app: &mut Harness, label: &str) -> Value {
    app.click("Window");
    app.click(label)
}

#[test]
fn fit_and_zoom_work_from_toolbar_in_every_foreground_music_editor() {
    let mut app = fixture();
    // Reproduce opening a project and returning using persistent navigation.
    app.click("Mixer");
    app.click("Arranger");
    toolbar_keys(&mut app, "timeline");

    app.click("Foreground phrase");
    window(&mut app, "Clip Editor");
    toolbar_keys(&mut app, "midi_editor");

    app.click("Arranger");
    app.click("Foreground recording");
    window(&mut app, "Clip Editor");
    toolbar_keys(&mut app, "audio_editor");
    let state = app.inspect();
    let gain = &state["audio_editor"]["gain_rect"];
    app.command(json!({
        "command":"pointer", "phase":"double_click",
        "x":(gain[0].as_f64().unwrap()+gain[2].as_f64().unwrap())*0.5,
        "y":(gain[1].as_f64().unwrap()+gain[3].as_f64().unwrap())*0.5,
    }));
    let focused = app.inspect();
    for key in ["F", "Equals", "Minus"] {
        let state = app.key(key);
        unchanged(&state, &focused);
        assert_eq!(
            framing(&state, "audio_editor"),
            framing(&focused, "audio_editor")
        );
    }
    app.key("Enter");

    app.click("Arranger");
    app.click("Foreground automation");
    window(&mut app, "Automation");
    toolbar_keys(&mut app, "automation_editor");
    app.quit();
}

#[test]
fn text_entry_menus_utilities_and_document_modal_keep_view_shortcuts() {
    let mut app = fixture();
    app.click("Foreground phrase");
    window(&mut app, "Clip Editor");
    app.click("Fit");

    // A text input in a utility keeps focus and the inactive editor's framing.
    window(&mut app, "Properties");
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == "Foreground phrase")
        .count();
    assert!(count > 0, "Clip properties must expose the name input");
    app.command(json!({"command":"click", "label":"Foreground phrase", "occurrence":count-1}));
    let before = app.inspect();
    for key in ["F", "Equals", "Minus"] {
        let state = app.key(key);
        unchanged(&state, &before);
        assert_eq!(
            framing(&state, "midi_editor"),
            framing(&before, "midi_editor")
        );
        assert_eq!(state["popups"]["properties"], true);
    }
    app.key("Escape");

    window(&mut app, "Effects");
    let before = app.inspect();
    for key in ["F", "Equals", "Minus"] {
        let state = app.key(key);
        unchanged(&state, &before);
        assert_eq!(state["popups"]["effects"], true);
    }
    app.key("Escape");
    let returned = app.inspect();
    assert_eq!(
        framing(&returned, "midi_editor"),
        framing(&before, "midi_editor")
    );

    app.click("File");
    let before = app.inspect();
    for key in ["F", "Equals", "Minus"] {
        let state = app.key(key);
        unchanged(&state, &before);
        assert_eq!(
            framing(&state, "midi_editor"),
            framing(&before, "midi_editor")
        );
    }
    app.key("Escape");

    // A real unsaved-document modal owns input above the arrangement.
    app.click("Arranger");
    app.click("Track");
    app.click("Add audio track");
    app.click("File");
    let before = app.click("New");
    assert_eq!(before["document_guard"]["phase"], "confirm");
    for key in ["F", "Equals", "Minus"] {
        let state = app.key(key);
        unchanged(&state, &before);
        assert_eq!(framing(&state, "timeline"), framing(&before, "timeline"));
        assert_eq!(state["document_guard"]["phase"], "confirm");
    }
    app.click("Cancel");
    app.quit();
}
