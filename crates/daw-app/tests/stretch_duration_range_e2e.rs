//! Stretch duration limits belong to the selected occurrence, before Apply.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, MAX_BEAT, Note, Project};
use serde_json::{Value, json};

fn fixture(midi: bool, length: f64, rate: f64, trimmed_ghost: bool) -> (Harness, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    let source = app.directory.path().join("long-source.wav");
    AudioData {
        sample_rate: 8000,
        channels: 1,
        samples: vec![0.1; 128_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let track = if midi {
        project.add_midi_track("Duration instrument")
    } else {
        project.add_audio_track("Duration audio")
    };
    let mut clip = if midi {
        Clip::midi(
            "Duration phrase",
            2.0,
            length,
            vec![Note::new(60, 0.0, 1.0, 90), Note::new(67, 3.0, 1.0, 80)],
        )
    } else {
        Clip::audio("Duration phrase", 2.0, length, source.to_string_lossy())
    };
    clip.source_rate = rate;
    let mut id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    if trimmed_ghost {
        id = project.clone_clip(track, id).unwrap();
        project.stretch_clip(track, id, 8.0).unwrap();
        let start = project.clip(track, id).unwrap().start;
        project.trim_clip(track, id, start + 1.0, 6.0).unwrap();
        assert_eq!(project.clip(track, id).unwrap().source_length(), 3.0);
    }
    let path = app.directory.path().join("duration-range.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Select");
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    click_rect(&mut app, rect);
    app.click("Window");
    app.click("Properties");
    (app, id)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn row_control<'a>(state: &'a Value, row: &str, symbol: Option<&str>) -> &'a Value {
    let heading = label(state, row);
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| {
            let text = label["text"].as_str().unwrap();
            symbol.map_or_else(
                || text.starts_with("Beat ") || text.ends_with(" beats"),
                |symbol| text == symbol,
            ) && (center(label).1 - center(heading).1).abs() < 12.0
                && label["min"][0].as_f64().unwrap() > heading["max"][0].as_f64().unwrap()
        })
        .unwrap_or_else(|| panic!("Missing {row} {symbol:?}: {}", state["visible_labels"]))
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn control(app: &mut Harness, row: &str, symbol: &str) -> Value {
    let state = app.inspect();
    click_rect(app, row_control(&state, row, Some(symbol)))
}

fn mode(app: &mut Harness, from: &str, to: &str) -> Value {
    let state = control(app, "Duration", from);
    let item = state["menu_label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == to)
        .unwrap();
    click_rect(app, item)
}

fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn unchanged(state: &Value, before: &Value) {
    for key in ["project", "history", "modified"] {
        assert_eq!(state[key], before[key], "Inspection or draft changed {key}");
    }
}

fn enter_mouse(app: &mut Harness, text: &str, before: &Value) -> Value {
    unchanged(&app.click("Clear"), before);
    let mut state = app.inspect();
    for digit in text.chars() {
        state = app.click(&digit.to_string());
        unchanged(&state, before);
    }
    state
}

fn apply_duration(app: &mut Harness, value: &str) -> Value {
    let before = app.inspect();
    control(app, "Duration", "123");
    enter_mouse(app, value, &before);
    app.click("Apply value")
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn one_edit(state: &Value, before: &Value) {
    assert_eq!(
        state["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
}

#[test]
fn invalid_stretch_stays_a_draft_and_both_exact_limits_commit_once() {
    for midi in [false, true] {
        let (mut app, id) = fixture(midi, 4.0, 1.0, false);
        let before = app.inspect();
        unchanged(&mode(&mut app, "Trim", "Stretch"), &before);
        control(&mut app, "Duration", "123");
        let invalid = enter_mouse(&mut app, "56", &before);
        let rejected = click_rect(&mut app, label(&invalid, "Apply value"));
        unchanged(&rejected, &before);
        // The previous app closed the draft and emitted a backend error here.
        // Keep both the invalid number and its correction controls visible.
        label(&rejected, "56");
        label(&rejected, "Clear");
        label(&rejected, "Range: 1 to 16");
        label(&rejected, "The value must be inside the range.");
        unchanged(&app.click("Cancel"), &before);
        for (duration, rate, blocked_step) in [("16", 0.25, "+"), ("1", 4.0, "−")] {
            let applied = apply_duration(&mut app, duration);
            assert_eq!(
                clip(&applied, id)["length"],
                duration.parse::<f64>().unwrap()
            );
            assert_eq!(clip(&applied, id)["source_rate"], rate);
            one_edit(&applied, &before);
            unchanged(&control(&mut app, "Duration", blocked_step), &applied);
            assert_eq!(undo(&mut app)["project"], before["project"]);
        }
        app.quit();
    }
}

#[test]
fn stretched_trimmed_ghost_uses_its_current_window_without_changing_its_peer() {
    for midi in [false, true] {
        let (mut app, id) = fixture(midi, 4.0, 1.0, true);
        let before = mode(&mut app, "Trim", "Stretch");
        assert_eq!(clip(&before, id)["length"], 6.0);
        assert_eq!(clip(&before, id)["source_rate"], 0.5);
        let opened = control(&mut app, "Duration", "123");
        label(&opened, "Range: 0.75 to 12");
        unchanged(&opened, &before);
        unchanged(&app.click("Apply value"), &before);
        for (duration, rate) in [("12", 0.25), ("0.75", 4.0)] {
            let applied = apply_duration(&mut app, duration);
            let mut expected = before["project"].clone();
            let occurrence = expected["tracks"][0]["clips"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|clip| clip["id"] == id)
                .unwrap();
            occurrence["length"] = json!(duration.parse::<f64>().unwrap());
            occurrence["source_rate"] = json!(rate);
            assert_eq!(
                applied["project"], expected,
                "Only occurrence timing changes"
            );
            one_edit(&applied, &before);
            assert_eq!(undo(&mut app)["project"], before["project"]);
        }
        app.quit();
    }
}

#[test]
fn drag_text_and_steps_share_the_stretch_limits() {
    let (mut app, id) = fixture(true, 4.0, 1.0, false);
    let before = mode(&mut app, "Trim", "Stretch");
    let stepped = control(&mut app, "Duration", "+");
    assert_eq!(clip(&stepped, id)["length"], 5.0);
    assert_eq!(clip(&stepped, id)["source_rate"], 0.8);
    assert_eq!(undo(&mut app)["project"], before["project"]);

    let state = app.inspect();
    let (x, y) = center(row_control(&state, "Duration", None));
    let dragged = app.command(
        json!({"command":"pointer", "phase":"drag", "x":x, "y":y, "to_x":x-500.0, "to_y":y}),
    );
    assert_eq!(clip(&dragged, id)["length"], 1.0);
    assert_eq!(clip(&dragged, id)["source_rate"], 4.0);
    one_edit(&dragged, &before);
    assert_eq!(undo(&mut app)["project"], before["project"]);

    let state = app.inspect();
    let (x, y) = center(row_control(&state, "Duration", None));
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"text", "text":"56"}));
    let typed = app.key("Enter");
    assert_eq!(clip(&typed, id)["length"], 16.0);
    assert_eq!(clip(&typed, id)["source_rate"], 0.25);
    one_edit(&typed, &before);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    app.quit();
}

#[test]
fn mode_switch_preserves_trim_and_stretch_respects_the_timeline_end() {
    let (mut app, id) = fixture(true, 4.0, 1.0, false);
    let before = mode(&mut app, "Trim", "Stretch");
    unchanged(&mode(&mut app, "Stretch", "Trim"), &before);
    let trimmed = apply_duration(&mut app, "56");
    assert_eq!(clip(&trimmed, id)["length"], 56.0);
    assert_eq!(clip(&trimmed, id)["source_rate"], 1.0);
    one_edit(&trimmed, &before);
    unchanged(&mode(&mut app, "Trim", "Stretch"), &trimmed);
    let opened = control(&mut app, "Duration", "123");
    label(&opened, "Range: 14 to 224");
    unchanged(&app.click("Cancel"), &trimmed);
    assert_eq!(undo(&mut app)["project"], before["project"]);

    let state = app.inspect();
    control(&mut app, "Start", "123");
    // A displayed start of MAX_BEAT - 5 is zero-based MAX_BEAT - 6.
    enter_mouse(&mut app, &(MAX_BEAT - 5.0).to_string(), &state);
    let moved = app.click("Apply value");
    assert_eq!(clip(&moved, id)["start"], MAX_BEAT - 6.0);
    let opened = control(&mut app, "Duration", "123");
    label(&opened, "Range: 1 to 6");
    enter_mouse(&mut app, "16", &moved);
    unchanged(&app.click("Apply value"), &moved);
    label(&app.inspect(), "Clear");
    enter_mouse(&mut app, "6", &moved);
    let applied = app.click("Apply value");
    assert_eq!(clip(&applied, id)["length"], 6.0);
    one_edit(&applied, &moved);
    assert_eq!(undo(&mut app)["project"], moved["project"]);
    app.quit();
}

#[test]
fn inspecting_precise_already_stretched_values_never_rounds_or_creates_history() {
    let precise = 4.123_456_789_123_45;
    let (mut app, id) = fixture(true, precise, 0.5, false);
    let before = app.inspect();
    for (from, to) in [("Trim", "Stretch"), ("Stretch", "Trim")] {
        unchanged(&mode(&mut app, from, to), &before);
        let opened = control(&mut app, "Duration", "123");
        label(&opened, &precise.to_string());
        unchanged(&opened, &before);
        unchanged(&app.click("Apply value"), &before);
        assert_eq!(clip(&app.inspect(), id)["length"], precise);
    }
    app.quit();
}
