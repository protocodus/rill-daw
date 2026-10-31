//! Pointer tools must not move the piano roll's drawing and harmony controls.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(dark: bool) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"system_theme", "dark":dark}));
    let mut project = Project::new();
    let track = project.add_midi_track("Toolbar instrument");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Toolbar phrase",
        0.0,
        8.0,
        vec![
            Note::new(60, 0.13, 0.4, 72),
            Note::new(64, 1.13, 0.4, 90),
            Note::new(67, 2.13, 0.4, 110),
        ],
    ));
    let path = app.directory.path().join("toolbar.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Toolbar phrase");
    app.click("Window");
    app.click("Clip Editor");
    app
}

fn label_bounds(state: &Value, prefix: &str) -> [f64; 4] {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"].as_str().unwrap().starts_with(prefix))
        .unwrap_or_else(|| panic!("Missing {prefix}: {}", state["visible_labels"]));
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn unchanged(state: &Value, before: &Value) {
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
    assert_eq!(state["modified"], before["modified"]);
}

fn menu(app: &mut Harness, title: &str, item: &str) -> Value {
    app.click(title);
    app.click(item)
}

fn notes(state: &Value) -> &Vec<Value> {
    state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}

#[test]
fn drawing_controls_keep_their_positions_across_mouse_tools_and_selection() {
    let controls = ["Generate…", "Notes", "Draw settings"];
    let draw_controls = ["Length ·", "Grid scale"];
    for dark in [false, true] {
        let mut app = fixture(dark);
        let original = app.inspect();
        for width in [800, 1100, 1440] {
            app.command(json!({"command":"viewport", "width":width, "height":760}));
            if width != 800 {
                menu(&mut app, "Edit", "Clear note selection");
            }
            let before = app.click("Select");
            let positions: Vec<_> = controls
                .iter()
                .map(|control| label_bounds(&before, control))
                .collect();
            let settings = app.click("Draw settings");
            let draw_positions: Vec<_> = draw_controls
                .iter()
                .map(|control| label_bounds(&settings, control))
                .collect();
            unchanged(&app.click("Draw settings"), &original);
            for (control, bounds) in controls.iter().zip(&positions) {
                assert!(
                    bounds[0] >= 0.0 && bounds[1] >= 0.0,
                    "{control}: {bounds:?}"
                );
                assert!(
                    bounds[2] <= f64::from(width) && bounds[3] <= 760.0,
                    "{control} outside {width}-point viewport: {bounds:?}"
                );
            }
            for selected in [false, true] {
                if selected {
                    menu(&mut app, "Edit", "Select all notes");
                }
                for (label, tool) in [
                    ("Select", "Select"),
                    ("Pencil", "Pencil"),
                    ("Multi-draw", "MultiDraw"),
                    ("Cursor", "Cursor"),
                    ("Cut", "Cut"),
                    ("Duplicate", "Duplicate"),
                    ("Glue", "Glue"),
                    ("Erase", "Erase"),
                ] {
                    let state = app.click(label);
                    assert_eq!(state["midi_editor"]["tool"], tool);
                    unchanged(&state, &original);
                    let selected_count = state["midi_editor"]["notes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|note| note["selected"] == true)
                        .count();
                    assert_eq!(selected_count, if selected { 3 } else { 0 });
                    for (control, expected) in controls.iter().zip(&positions) {
                        let actual = label_bounds(&state, control);
                        for (actual, expected) in actual.iter().zip(expected) {
                            assert!(
                                (actual - expected).abs() < 0.1,
                                "{control} moved after {label}, width={width}, dark={dark}, selected={selected}"
                            );
                        }
                    }
                    let settings = app.click("Draw settings");
                    unchanged(&settings, &original);
                    for (control, expected) in draw_controls.iter().zip(&draw_positions) {
                        let actual = label_bounds(&settings, control);
                        for (actual, expected) in actual.iter().zip(expected) {
                            assert!(
                                (actual - expected).abs() < 0.1,
                                "{control} moved after {label}, width={width}, dark={dark}, selected={selected}"
                            );
                        }
                    }
                    app.click("Draw settings");
                    if !selected {
                        // The stable Notes menu keeps unavailable commands visible and inert.
                        for command in ["Randomize...", "Quantize..."] {
                            app.click("Notes");
                            let state = app.click(command);
                            unchanged(&state, &original);
                            assert!(state["midi_editor"]["randomize"].is_null());
                            assert!(state["midi_editor"]["quantize"].is_null());
                            // Native action menus may dismiss on a disabled-row click.
                            // Close only an actually open menu before testing the next command.
                            if state["native_menu"]["open"] == true {
                                app.click("Notes");
                            }
                        }
                    }
                }
            }
        }
        app.quit();
    }
}

#[test]
fn selection_commands_still_preview_apply_and_undo_from_the_pencil_tool() {
    let mut app = fixture(false);
    app.command(json!({"command":"viewport", "width":1100, "height":760}));
    menu(&mut app, "Edit", "Select all notes");
    let before = app.click("Pencil");
    let preview = menu(&mut app, "Notes", "Randomize...");
    assert!(preview["midi_editor"]["randomize"].is_object());
    unchanged(&preview, &before);
    unchanged(&app.click("Cancel randomize"), &before);

    let preview = menu(&mut app, "Notes", "Quantize...");
    assert!(preview["midi_editor"]["quantize"].is_object());
    unchanged(&preview, &before);
    let applied = app.click("Apply quantize");
    for (index, note) in notes(&applied).iter().enumerate() {
        assert!((note["start"].as_f64().unwrap() - (index as f64 + 0.25)).abs() < 1e-9);
        assert_eq!(note["id"], notes(&before)[index]["id"]);
        assert_eq!(note["length"], notes(&before)[index]["length"]);
        assert_eq!(note["velocity"], notes(&before)[index]["velocity"]);
    }
    assert_eq!(
        applied["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        applied["project"]
    );
    app.quit();
}

#[test]
fn hovering_the_keyboard_or_a_note_row_reveals_pitch_names_without_editing() {
    for dark in [false, true] {
        let mut app = fixture(dark);
        app.command(json!({"command":"viewport", "width":1100, "height":760}));
        let before = app.click("Fit");
        let key = before["midi_editor"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|key| key["pitch"] == 61)
            .unwrap();
        let rect = &key["rect"];
        let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
        let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
        let visible = |state: &Value, pitch| {
            state["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == &json!(daw_core::note_name(pitch)))
        };
        assert!(!visible(&before, 61));
        let plot_x = before["midi_editor"]["plot"]["min"][0].as_f64().unwrap() + 80.0;
        let row = app.command(json!({"command":"pointer", "phase":"move", "x":plot_x, "y":y}));
        assert!(visible(&row, 61));
        assert!(!visible(&row, 62));
        unchanged(&row, &before);
        let keyboard = app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
        assert!(visible(&keyboard, 61));
        assert!(visible(&keyboard, 62));
        unchanged(&keyboard, &before);
        let pressed = app.command(json!({"command":"pointer", "phase":"down", "x":x, "y":y}));
        assert!(visible(&pressed, 61));
        unchanged(&pressed, &before);
        let released = app.command(json!({"command":"pointer", "phase":"up", "x":x, "y":y}));
        unchanged(&released, &before);
        app.quit();
    }
}
