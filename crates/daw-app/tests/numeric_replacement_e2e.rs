//! Mouse-only full-value replacement in the actual tempo/gain/generator UI.
mod common;

use common::Harness;
use daw_core::{Clip, Project};
use serde_json::{Value, json};

fn text_rect(state: &Value, matches: impl Fn(&str) -> bool) -> Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"].as_str().is_some_and(&matches))
        .unwrap_or_else(|| panic!("Numeric label missing: {}", state["visible_labels"]))
        .clone()
}

fn replace(app: &mut Harness, rect: &Value, fraction: f64, value: &str, commit: &str) -> Value {
    let x = rect["min"][0].as_f64().unwrap()
        + (rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap()) * fraction;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":value}));
    app.click(commit)
}

#[test]
fn mouse_multiclick_replaces_tempo_gain_and_generator_length_without_keyboard_selection() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    project.tempo = 120.72;
    let track = project.add_midi_track("Numeric channel");
    project.track_mut(track).unwrap().gain = 10.0_f32.powf(-2.5 / 20.0);
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("Numeric phrase", 0.0, 16.0, vec![]));
    let path = app.directory.path().join("numeric-fields.rill");
    project.save(&path).unwrap();
    let state = app.command(json!({"command":"open","path":path}));

    let tempo = text_rect(&state, |text| text.ends_with(" BPM"));
    let state = replace(&mut app, &tempo, 0.35, "72", "TRACKS");
    assert_eq!(state["project"]["tempo"], 72.0);
    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"]["tempo"], 120.72, "one-step tempo Undo");

    let gain = text_rect(&undone, |text| text.ends_with(" dB"));
    let state = replace(&mut app, &gain, 0.35, "-24", "TRACKS");
    let expected = 10.0_f64.powf(-24.0 / 20.0);
    let actual = state["project"]["tracks"][0]["gain"].as_f64().unwrap();
    assert!(
        (actual - expected).abs() < 1e-6,
        "gain replacement: {actual}"
    );
    app.click("Edit");
    let undone = app.click("Undo");
    assert!(
        (undone["project"]["tracks"][0]["gain"].as_f64().unwrap()
            - project.track(track).unwrap().gain as f64)
            .abs()
            < 1e-6
    );

    app.click("Numeric phrase");
    app.click("Window");
    app.click("Clip Editor");
    let state = app.click("Generate…");
    let range = &state["midi_editor"]["generate"]["controls"]["length_bars"];
    let state = replace(&mut app, range, 0.5, "2", "Start bar");
    assert_eq!(state["midi_editor"]["generate"]["length_bars"], 2);
    assert_eq!(state["midi_editor"]["generate"]["length"], 8.0);
    assert_eq!(
        state["project"], undone["project"],
        "draft numbers are not project edits"
    );
    app.quit();
}
