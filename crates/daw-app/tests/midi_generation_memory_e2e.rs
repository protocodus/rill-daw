//! Generator choices stay with their clip without becoming project edits.
mod common;

use common::Harness;
use daw_core::{Clip, Project};
use serde_json::{Value, json};

fn fixture() -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    let mut project = Project::new();
    project.beats_per_bar = 3;
    for name in ["Clip A", "Clip B"] {
        let track = project.add_midi_track(format!("{name} channel"));
        project
            .track_mut(track)
            .unwrap()
            .clips
            .push(Clip::midi(name, 0.0, 12.0, vec![]));
    }
    let path = app.directory.path().join("generator-memory.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    open_clip(&mut app, "Clip A");
    app
}

fn open_clip(app: &mut Harness, name: &str) -> Value {
    app.click("Arranger");
    app.click(name);
    app.click("Window");
    app.click("Clip Editor")
}

fn generation(state: &Value) -> &Value {
    let draft = &state["midi_editor"]["generate"];
    assert!(!draft.is_null(), "Generator must be open: {state}");
    draft
}

fn remembered(state: &Value) -> Value {
    let draft = generation(state);
    json!({
        "kind":draft["kind"], "settings":draft["settings"],
        "start_bar":draft["start_bar"], "length_bars":draft["length_bars"],
        "seed":draft["seed"], "preview":draft["preview"], "notes":draft["notes"],
    })
}

fn viewport(state: &Value) -> Value {
    let view = &state["midi_editor"];
    json!([
        view["start_beat"],
        view["pixels_per_beat"],
        view["row_offset"],
        view["row_height"]
    ])
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn control_rect(app: &mut Harness, field: &str) -> Value {
    for _ in 0..16 {
        let state = app.inspect();
        let controls = &generation(&state)["controls"];
        let rect = &controls[field];
        assert!(!rect.is_null(), "Missing {field}: {controls}");
        if ["start_bar", "length_bars"].contains(&field) {
            return rect.clone();
        }
        let options = &controls["options"];
        let (_, y) = center(rect);
        let top = options["min"][1].as_f64().unwrap();
        let bottom = options["max"][1].as_f64().unwrap();
        if y >= top + 2.0 && y <= bottom - 2.0 {
            return rect.clone();
        }
        let (x, anchor_y) = center(options);
        app.command(json!({"command":"scroll", "x":x, "y":anchor_y,
            "delta":if y < top {160.0} else {-160.0}}));
    }
    panic!("Could not reveal {field}");
}

fn number(app: &mut Harness, field: &str, value: &str) -> Value {
    let (x, y) = center(&control_rect(app, field));
    // DragValue selects its text when double-clicked. Typing replaces it;
    // clicking the fixed range label accepts the edit without a shortcut.
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"text", "text":value}));
    app.click("Start bar")
}

fn choice(app: &mut Harness, field: &str, label: &str) -> Value {
    let rect = control_rect(app, field);
    let mut state = click_rect(app, &rect);
    for _ in 0..10 {
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == label)
        {
            break;
        }
        let last = state["label_rects"].as_array().unwrap().last().unwrap();
        let (x, y) = center(last);
        state = app.command(json!({"command":"scroll", "x":x, "y":y, "delta":-150.0}));
    }
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == label)
        .count()
        .checked_sub(1)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    app.command(json!({"command":"click", "label":label, "occurrence":occurrence}))
}

#[test]
fn generator_remembers_each_clips_choices_after_cancel_and_add_without_editing_on_reopen() {
    let mut app = fixture();
    let before = app.inspect();
    app.click("Generate…");
    app.click("Arpeggio");
    number(&mut app, "start_bar", "3");
    number(&mut app, "length_bars", "2");
    choice(&mut app, "root", "D");
    choice(&mut app, "harmony", "Scale");
    choice(&mut app, "scale", "Natural minor");
    choice(&mut app, "harmony", "Chord");
    choice(&mut app, "chord", "Minor 7");
    number(&mut app, "octaves", "2");
    number(&mut app, "start_octave", "3");
    choice(&mut app, "pattern", "Random");
    choice(&mut app, "speed_multiplier", "2×");
    number(&mut app, "gate", "55");
    number(&mut app, "velocity", "73");
    app.click("New variation");
    app.click("Fit preview");
    let selected = app.click("Preview");
    let expected_a = remembered(&selected);
    assert_eq!(expected_a["kind"], "Arpeggio");
    assert_eq!(expected_a["start_bar"], 3);
    assert_eq!(expected_a["length_bars"], 2);
    assert_eq!(expected_a["seed"], 2);
    assert_eq!(expected_a["settings"]["root"], 2);
    assert_eq!(expected_a["settings"]["chord"], "Minor 7");
    assert_eq!(expected_a["settings"]["scale"], "Natural minor");
    assert_eq!(expected_a["settings"]["velocity"], 73);
    assert_eq!(expected_a["preview"], false);
    assert!(!expected_a["notes"].as_array().unwrap().is_empty());
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    assert_eq!(viewport(&cancelled), viewport(&before));
    let reopened = app.click("Generate…");
    assert_eq!(remembered(&reopened), expected_a);

    let advanced = app.click("Next range");
    let expected_advanced = remembered(&advanced);
    assert_eq!(expected_advanced["start_bar"], 5);
    assert_eq!(expected_advanced["length_bars"], 2);
    assert_eq!(expected_advanced["settings"], expected_a["settings"]);
    assert_eq!(expected_advanced["seed"], expected_a["seed"]);
    for (old, new) in expected_a["notes"]
        .as_array()
        .unwrap()
        .iter()
        .zip(expected_advanced["notes"].as_array().unwrap())
    {
        assert_eq!(old["pitch"], new["pitch"]);
        assert_eq!(old["length"], new["length"]);
        assert!(
            (new["start"].as_f64().unwrap() - old["start"].as_f64().unwrap() - 6.0).abs() < 1e-9
        );
    }
    assert_eq!(advanced["project"], before["project"]);
    let open = app.click("Add notes");
    assert!(!open["midi_editor"]["generate"].is_null());
    let added = app.click("Done");
    assert!(added["midi_editor"]["generate"].is_null());
    let notes = added["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(
        notes.len(),
        expected_advanced["notes"].as_array().unwrap().len()
    );
    assert!(
        added["project"]["tracks"][0]["clips"][0]["length"]
            .as_f64()
            .unwrap()
            > 12.0
    );
    // Grid scale is visual/multi-draw context. Changing it must leave both the
    // accepted notes and the generator's independently chosen harmony intact.
    app.click("C · Natural minor");
    app.click("Natural minor");
    app.click("Major");
    app.click("Grid scale");
    app.click("C · Major");
    app.click("C");
    app.click("F♯");
    let changed_grid = app.click("Grid scale");
    assert_eq!(changed_grid["project"], added["project"]);
    assert_eq!(changed_grid["history"], added["history"]);
    let reopened = app.click("Generate…");
    assert_eq!(remembered(&reopened), expected_advanced);
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], added["project"]);
    assert_eq!(cancelled["history"], added["history"]);

    open_clip(&mut app, "Clip B");
    let opened_b = app.click("Generate…");
    assert_eq!(generation(&opened_b)["kind"], "Chord");
    assert_eq!(generation(&opened_b)["seed"], 1);
    app.click("Melody");
    number(&mut app, "start_bar", "2");
    number(&mut app, "length_bars", "1");
    choice(&mut app, "harmony", "Scale");
    choice(&mut app, "scale", "Dorian");
    choice(&mut app, "root", "G");
    choice(&mut app, "algorithm", "Motif variation");
    number(&mut app, "motif_steps", "6");
    number(&mut app, "step_size", "3");
    number(&mut app, "density", "65");
    choice(&mut app, "accompaniment", "Counterpoint");
    number(&mut app, "additional_voices", "2");
    let edited_b = number(&mut app, "velocity", "66");
    let expected_b = remembered(&edited_b);
    app.click("Cancel");

    open_clip(&mut app, "Clip A");
    let a = app.click("Generate…");
    assert_eq!(remembered(&a), expected_advanced);
    app.click("Cancel");
    open_clip(&mut app, "Clip B");
    let b = app.click("Generate…");
    assert_eq!(remembered(&b), expected_b);
    assert_eq!(b["project"], added["project"]);
    assert_eq!(b["history"], added["history"]);
    app.click("Cancel");
    app.quit();
}

#[test]
fn next_range_rejects_zero_length_and_ranges_past_the_supported_timeline() {
    let mut app = fixture();
    let before = app.inspect();
    app.click("Generate…");
    let zero = number(&mut app, "length_bars", "0");
    let unchanged = app.click("Next range");
    assert_eq!(remembered(&unchanged), remembered(&zero));
    assert!(!generation(&unchanged)["error"].is_null());
    number(&mut app, "length_bars", "1");
    let last = number(&mut app, "start_bar", "333333");
    assert!(generation(&last)["error"].is_null());
    let unchanged = app.click("Next range");
    assert_eq!(remembered(&unchanged), remembered(&last));
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    app.quit();
}
