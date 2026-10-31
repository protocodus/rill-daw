//! Exact mouse-only numeric drafts exercise the running app and its history.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

const PRECISE_TEMPO: f64 = 120.123_456_789_012_34;

fn fixture(width: u32, dark: bool) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":width, "height":900}));
    app.command(json!({"command":"system_theme", "dark":dark}));
    let mut project = Project::new();
    project.tempo = PRECISE_TEMPO;
    let track = project.add_midi_track("Keypad instrument");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Keypad phrase",
        1.25,
        4.25,
        vec![Note::new(60, 0.0, 1.25, 90)],
    ));
    let path = app.directory.path().join("keypad.rill");
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

fn label(state: &Value, text: &str) -> Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
        .clone()
}

fn field(state: &Value, name: &str) -> Value {
    let labels = state["label_rects"].as_array().unwrap();
    if name == "Duration" {
        let heading = label(state, name);
        return labels
            .iter()
            .find(|label| {
                label["text"].as_str().unwrap().ends_with(" beats")
                    && (center(label).1 - center(&heading).1).abs() < 12.0
            })
            .expect("duration field")
            .clone();
    }
    labels
        .iter()
        .find(|label| {
            let text = label["text"].as_str().unwrap();
            if name == "Tempo" {
                text.ends_with(" BPM")
            } else {
                text.starts_with(name)
            }
        })
        .unwrap_or_else(|| panic!("Missing field {name}: {}", state["visible_labels"]))
        .clone()
}

fn action(state: &Value, name: &str, symbol: &str) -> Value {
    let field = field(state, name);
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
        .unwrap_or_else(|| panic!("Missing {name} {symbol}: {}", state["visible_labels"]))
        .clone()
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn open_keypad(app: &mut Harness, name: &str) -> Value {
    let state = app.inspect();
    click_rect(app, &action(&state, name, "123"))
}

fn unchanged(state: &Value, before: &Value) {
    for key in ["project", "history", "modified"] {
        assert_eq!(state[key], before[key], "Draft changed {key}");
    }
}

fn enter_mouse(app: &mut Harness, number: &str, before: &Value) -> Value {
    unchanged(&app.click("Clear"), before);
    let mut state = app.inspect();
    for digit in number.chars() {
        state = app.click(&digit.to_string());
        unchanged(&state, before);
    }
    state
}

fn undo_count(state: &Value) -> usize {
    state["history"]["undo"].as_array().unwrap().len()
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn clip(state: &Value) -> &Value {
    &state["project"]["tracks"][0]["clips"][0]
}

#[test]
fn tempo_keypad_preserves_precision_and_commits_once_in_both_themes_and_widths() {
    for dark in [false, true] {
        for width in [800, 1440] {
            let mut app = fixture(width, dark);
            let before = app.inspect();
            let original_steps = ["−", "+"].map(|symbol| action(&before, "Tempo", symbol));
            let opened = open_keypad(&mut app, "Tempo");
            unchanged(&opened, &before);
            label(&opened, &daw_ui::exact_value_text(PRECISE_TEMPO));
            for (symbol, original) in ["−", "+"].into_iter().zip(original_steps) {
                assert_eq!(center(&action(&opened, "Tempo", symbol)), center(&original));
            }
            for text in [
                "7",
                "0",
                ".",
                "±",
                "Backspace",
                "Clear",
                "Apply value",
                "Cancel",
            ] {
                let bounds = label(&opened, text);
                assert!(bounds["min"][0].as_f64().unwrap() >= 0.0);
                assert!(bounds["max"][0].as_f64().unwrap() <= f64::from(width));
                assert!(bounds["max"][1].as_f64().unwrap() <= 900.0);
            }
            let apply_position = center(&label(&opened, "Apply value"));
            let cleared = app.click("Clear");
            unchanged(&cleared, &before);
            assert_eq!(center(&label(&cleared, "Apply value")), apply_position);
            enter_mouse(&mut app, "137.625", &before);
            let applied = app.click("Apply value");
            assert_eq!(applied["project"]["tempo"], 137.625);
            assert_eq!(undo_count(&applied), undo_count(&before) + 1);
            assert_eq!(undo(&mut app)["project"], before["project"]);

            // Opening and applying the unrounded original must be a no-op.
            let restored = app.inspect();
            open_keypad(&mut app, "Tempo");
            unchanged(&app.click("Apply value"), &restored);
            app.quit();
        }
    }
}

#[test]
fn invalid_cancelled_and_dismissed_keypad_drafts_never_clamp_or_create_history() {
    let mut app = fixture(800, false);
    let before = app.inspect();
    open_keypad(&mut app, "Tempo");
    for number in ["999", "0", "12..5"] {
        let invalid = enter_mouse(&mut app, number, &before);
        let applied = click_rect(&mut app, &label(&invalid, "Apply value"));
        unchanged(&applied, &before);
        label(&applied, "Clear");
    }
    enter_mouse(&mut app, "72.5", &before);
    unchanged(&app.click("±"), &before);
    unchanged(&app.click("Apply value"), &before);
    unchanged(&app.click("±"), &before);
    unchanged(&app.click("Backspace"), &before);
    let corrected = app.click("6");
    unchanged(&corrected, &before);
    label(&corrected, "72.6");
    unchanged(&app.click("Cancel"), &before);

    open_keypad(&mut app, "Tempo");
    enter_mouse(&mut app, "84", &before);
    let dismissed = app.command(json!({"command":"pointer", "phase":"click", "x":790, "y":880}));
    unchanged(&dismissed, &before);
    let reopened = open_keypad(&mut app, "Tempo");
    label(&reopened, &daw_ui::exact_value_text(PRECISE_TEMPO));
    unchanged(&app.click("Cancel"), &before);
    // Native text entry remains available in the same draft, but non-finite
    // input must never escape the Apply validation used by mouse keys.
    for text in ["NaN", "inf", "1e999"] {
        open_keypad(&mut app, "Tempo");
        let invalid = app.command(json!({"command":"text", "text":text}));
        unchanged(&invalid, &before);
        unchanged(
            &click_rect(&mut app, &label(&invalid, "Apply value")),
            &before,
        );
        unchanged(&app.click("Cancel"), &before);
    }
    app.quit();
}

#[test]
fn clip_duration_and_nested_loop_keypads_commit_exact_values_with_independent_undo() {
    for (width, dark) in [(800, true), (1440, false)] {
        let mut app = fixture(width, dark);
        app.click("Keypad phrase");
        app.click("Window");
        let before = app.click("Properties");
        open_keypad(&mut app, "Duration");
        enter_mouse(&mut app, "9.125", &before);
        let duration = app.click("Apply value");
        assert_eq!(clip(&duration)["length"], 9.125);
        assert_eq!(clip(&duration)["source_rate"], 1.0);
        assert_eq!(clip(&duration)["content"], clip(&before)["content"]);
        assert_eq!(undo_count(&duration), undo_count(&before) + 1);
        // Undo and Range use the ordinary app menus while Properties remains
        // the visible caller; keypad Apply must not replace its navigation.
        assert_eq!(undo(&mut app)["project"], before["project"]);
        app.click("Range");
        let before_loop = app.inspect();
        open_keypad(&mut app, "End beat ");
        enter_mouse(&mut app, "57.25", &before_loop);
        let looped = app.click("Apply value");
        assert_eq!(looped["project"]["loop_end"], 56.25);
        assert_eq!(undo_count(&looped), undo_count(&before_loop) + 1);
        assert_eq!(
            looped["project"]["tracks"],
            before_loop["project"]["tracks"]
        );
        assert_eq!(undo(&mut app)["project"], before_loop["project"]);
        app.click("Range");
        let before_cancel = app.inspect();
        open_keypad(&mut app, "Start beat ");
        enter_mouse(&mut app, "999", &before_cancel);
        let invalid = app.inspect();
        unchanged(
            &click_rect(&mut app, &label(&invalid, "Apply value")),
            &before_cancel,
        );
        unchanged(&app.click("Cancel"), &before_cancel);
        app.quit();
    }
}

#[test]
fn existing_step_drag_and_keyboard_entry_still_work_after_keypad_cancel() {
    let mut app = fixture(1440, true);
    let before = app.inspect();
    open_keypad(&mut app, "Tempo");
    app.click("Cancel");
    let stepped = click_rect(&mut app, &action(&before, "Tempo", "+"));
    assert_eq!(stepped["project"]["tempo"], PRECISE_TEMPO + 1.0);
    assert_eq!(undo_count(&stepped), undo_count(&before) + 1);
    undo(&mut app);
    let rect = field(&app.inspect(), "Tempo");
    let (x, y) = center(&rect);
    app.command(json!({"command":"pointer", "phase":"down", "x":x, "y":y}));
    app.command(json!({"command":"pointer", "phase":"move", "x":x+16.0, "y":y}));
    let dragged = app.command(json!({"command":"pointer", "phase":"up", "x":x+16.0, "y":y}));
    assert_ne!(dragged["project"]["tempo"], before["project"]["tempo"]);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    let rect = field(&app.inspect(), "Tempo");
    let (x, y) = center(&rect);
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"text", "text":"98.75"}));
    let typed = app.key("Enter");
    assert_eq!(typed["project"]["tempo"], 98.75);
    assert_eq!(undo(&mut app)["project"], before["project"]);
    app.quit();
}
