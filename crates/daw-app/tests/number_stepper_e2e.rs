//! Visible numeric step buttons preserve precise values and ordinary history.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(tempo: f64, width: u32, dark: bool) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":width, "height":760}));
    app.command(json!({"command":"system_theme", "dark":dark}));
    let mut project = Project::new();
    project.tempo = tempo;
    let track = project.add_midi_track("Stepper instrument");
    let mut clip = Clip::midi(
        "Stepper phrase",
        1.25,
        4.25,
        vec![Note::new(60, 0.0, 1.0, 90), Note::new(67, 3.0, 1.0, 100)],
    );
    clip.offset = 1.25;
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("stepper.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn field(state: &Value, row: &str) -> Value {
    let labels = state["label_rects"].as_array().unwrap();
    if row == "Tempo" {
        return labels
            .iter()
            .find(|label| label["text"].as_str().unwrap().ends_with(" BPM"))
            .unwrap()
            .clone();
    }
    let heading = labels
        .iter()
        .rev()
        .find(|label| label["text"] == row)
        .unwrap();
    labels
        .iter()
        .find(|label| {
            let text = label["text"].as_str().unwrap();
            (text.starts_with("Beat ") || text.ends_with(" beats"))
                && (center(label).1 - center(heading).1).abs() < 12.0
                && label["min"][0].as_f64().unwrap() > heading["max"][0].as_f64().unwrap()
        })
        .unwrap_or_else(|| {
            panic!(
                "Missing numeric field for {row}: {}",
                state["visible_labels"]
            )
        })
        .clone()
}

fn step_button(state: &Value, row: &str, symbol: &str) -> Value {
    let field = field(state, row);
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| {
            label["text"] == symbol
                && (center(label).1 - center(&field).1).abs() < 12.0
                && label["min"][0].as_f64().unwrap() > field["max"][0].as_f64().unwrap()
        })
        .min_by(|a, b| center(a).0.total_cmp(&center(b).0))
        .unwrap_or_else(|| panic!("Missing {row} {symbol}: {}", state["visible_labels"]))
        .clone()
}

fn step(app: &mut Harness, row: &str, symbol: &str) -> Value {
    let state = app.inspect();
    let (x, y) = center(&step_button(&state, row, symbol));
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn menu(app: &mut Harness, title: &str, item: &str) -> Value {
    app.click(title);
    app.click(item)
}

fn undo_count(state: &Value) -> usize {
    state["history"]["undo"].as_array().unwrap().len()
}

fn clip(state: &Value) -> &Value {
    &state["project"]["tracks"][0]["clips"][0]
}

fn unchanged(state: &Value, before: &Value) {
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
}

#[test]
fn tempo_steps_preserve_fractions_and_each_click_has_its_own_undo() {
    for dark in [false, true] {
        for width in [800, 1100, 1440] {
            let mut app = fixture(120.75, width, dark);
            let before = app.inspect();
            for symbol in ["−", "+"] {
                let rect = step_button(&before, "Tempo", symbol);
                assert!(rect["max"][0].as_f64().unwrap() <= f64::from(width));
                assert!(rect["max"][1].as_f64().unwrap() <= 760.0);
            }
            let first = step(&mut app, "Tempo", "+");
            assert_eq!(first["project"]["tempo"], 121.75);
            assert_eq!(undo_count(&first), undo_count(&before) + 1);
            let second = step(&mut app, "Tempo", "+");
            assert_eq!(second["project"]["tempo"], 122.75);
            assert_eq!(undo_count(&second), undo_count(&before) + 2);
            assert_eq!(menu(&mut app, "Edit", "Undo")["project"], first["project"]);
            assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
            let lower = step(&mut app, "Tempo", "−");
            assert_eq!(lower["project"]["tempo"], 119.75);
            app.quit();
        }
    }
}

#[test]
fn boundary_buttons_do_not_round_clamp_or_create_history() {
    for (tempo, symbol) in [(20.25, "−"), (399.75, "+")] {
        let mut app = fixture(tempo, 1100, false);
        let before = app.inspect();
        unchanged(&step(&mut app, "Tempo", symbol), &before);
        app.quit();
    }
    let mut app = fixture(120.75, 1100, false);
    app.click("Stepper phrase");
    menu(&mut app, "Window", "Properties");
    let shortened = step(&mut app, "Source offset", "−");
    assert_eq!(clip(&shortened)["offset"], 0.25);
    unchanged(&step(&mut app, "Source offset", "−"), &shortened);
    let moved = step(&mut app, "Start", "−");
    assert_eq!(clip(&moved)["start"], 0.25);
    unchanged(&step(&mut app, "Start", "−"), &moved);
    app.quit();
}

#[test]
fn clip_timing_steps_remain_visible_and_preserve_source_notes_and_fractional_values() {
    for dark in [false, true] {
        for width in [800, 1100, 1440] {
            let mut app = fixture(120.75, width, dark);
            app.click("Stepper phrase");
            let before = menu(&mut app, "Window", "Properties");
            for row in ["Start", "Duration", "Source offset"] {
                for symbol in ["−", "+"] {
                    let rect = step_button(&before, row, symbol);
                    assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
                    assert!(rect["max"][0].as_f64().unwrap() <= f64::from(width));
                    assert!(rect["max"][1].as_f64().unwrap() <= 760.0);
                }
            }
            let moved = step(&mut app, "Start", "+");
            assert_eq!(clip(&moved)["start"], 2.25);
            assert_eq!(undo_count(&moved), undo_count(&before) + 1);
            let extended = step(&mut app, "Duration", "+");
            assert_eq!(clip(&extended)["length"], 5.25);
            assert_eq!(clip(&extended)["source_rate"], 1.0);
            assert_eq!(undo_count(&extended), undo_count(&before) + 2);
            let slipped = step(&mut app, "Source offset", "+");
            assert_eq!(clip(&slipped)["offset"], 2.25);
            assert_eq!(clip(&slipped)["content"], clip(&before)["content"]);
            assert_eq!(undo_count(&slipped), undo_count(&before) + 3);
            assert_eq!(
                menu(&mut app, "Edit", "Undo")["project"],
                extended["project"]
            );
            assert_eq!(menu(&mut app, "Edit", "Undo")["project"], moved["project"]);
            assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
            app.quit();
        }
    }
}
