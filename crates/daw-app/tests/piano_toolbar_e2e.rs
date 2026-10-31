//! Piano-roll tools stay focused without hiding musical operations from the mouse.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn label<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]))
}

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn hover_target(app: &mut Harness, name: &str) -> [f64; 4] {
    let state = app.inspect();
    let [left, top, right, bottom] = bounds(label(&state, name));
    app.command(json!({"command":"pointer", "phase":"move",
        "x":(left + right) * 0.5, "y":(top + bottom) * 0.5}));
    let state = app.command(json!({"command":"inspect", "paint":true}));
    state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shape| bounds(&shape["rect"]))
        .filter(|[x0, y0, x1, y1]| {
            *x0 <= left
                && *y0 <= top
                && *x1 >= right
                && *y1 >= bottom
                && *y1 - *y0 >= f64::from(daw_ui::tokens::CONTROL_HEIGHT)
        })
        .min_by(|a, b| {
            let area = |r: &[f64; 4]| (r[2] - r[0]) * (r[3] - r[1]);
            area(a).total_cmp(&area(b))
        })
        .unwrap_or_else(|| panic!("Missing full-size tool target for {name}"))
}

fn selected_notes(state: &Value) -> Vec<u64> {
    let mut ids: Vec<_> = state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|note| note["selected"] == true)
        .map(|note| note["id"].as_u64().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "selected_track",
        "selected_clip",
        "song_session",
    ] {
        assert_eq!(
            state[field], before[field],
            "Tool navigation changed {field}"
        );
    }
    assert_eq!(selected_notes(state), selected_notes(before));
}

#[test]
fn exposed_piano_tools_preserve_mouse_and_key_access_and_wrap_within_the_editor() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Evening keys");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Piano toolbar phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 1.0, 90), Note::new(67, 3.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("piano-toolbar.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Piano toolbar phrase");
    app.click("Window");
    app.click("Clip Editor");
    app.click("Fit");
    let before = app.command(json!({"command":"key", "key":"A", "cmd":true}));
    assert_eq!(selected_notes(&before).len(), 2);
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [800, 1100, 1480] {
            app.command(json!({"command":"viewport", "width":width, "height":720, "pixels_per_point":density}));
            let initial = app.click("Select");
            let settings = label(&initial, "Draw settings").clone();
            for name in [
                "Select",
                "Pencil",
                "Multi-draw",
                "Cut",
                "Duplicate",
                "Glue",
                "Erase",
                "Cursor",
                "Generate…",
                "Notes",
                "Draw settings",
                "Tool help",
                "View",
                "Fit",
            ] {
                let rect = label(&initial, name);
                assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{name} at {width}");
                assert!(
                    rect["max"][0].as_f64().unwrap() <= f64::from(width),
                    "{name} at {width}"
                );
                assert!(
                    rect["max"][1].as_f64().unwrap() < 720.0,
                    "{name} at {width}"
                );
            }
            assert!(
                !initial["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| label == "More tools")
            );
            let mut targets = Vec::new();
            for name in [
                "Select",
                "Pencil",
                "Multi-draw",
                "Cursor",
                "Cut",
                "Glue",
                "Erase",
                "Duplicate",
            ] {
                let rect = hover_target(&mut app, name);
                assert!(
                    rect[0] >= 0.0 && rect[2] <= f64::from(width),
                    "{name}: {rect:?}"
                );
                assert!(
                    rect[3] <= initial["midi_editor"]["plot"]["min"][1].as_f64().unwrap(),
                    "Tool targets stay above the piano canvas: {name}: {rect:?}"
                );
                for (other, previous) in &targets {
                    let previous: &[f64; 4] = previous;
                    assert!(
                        rect[2] <= previous[0]
                            || rect[0] >= previous[2]
                            || rect[3] <= previous[1]
                            || rect[1] >= previous[3],
                        "{name} overlaps {other} at {width}"
                    );
                }
                targets.push((name, rect));
            }
            for (name, key, tool) in [
                ("Select", "Q", "Select"),
                ("Pencil", "W", "Pencil"),
                ("Multi-draw", "U", "MultiDraw"),
                ("Cut", "E", "Cut"),
                ("Duplicate", "R", "Duplicate"),
                ("Glue", "T", "Glue"),
                ("Erase", "Z", "Erase"),
                ("Cursor", "P", "Cursor"),
            ] {
                app.click("Select");
                let chosen = app.click(name);
                assert_eq!(chosen["midi_editor"]["tool"], tool);
                label(&chosen, name);
                unchanged(&chosen, &before);
                assert_eq!(
                    label(&chosen, "Draw settings"),
                    &settings,
                    "Direct tool selection keeps adjacent controls in place"
                );
                app.click(name);
                let reselected = app.inspect();
                assert_eq!(reselected["midi_editor"]["tool"], tool);
                unchanged(&reselected, &before);
                app.click("Select");
                let shortcut = app.key(key);
                assert_eq!(shortcut["midi_editor"]["tool"], tool);
                unchanged(&shortcut, &before);
            }
            app.click("Select");
            let drawing = app.click("Draw settings");
            label(&drawing, "Length · Last note");
            label(&drawing, "Grid scale");
            label(&drawing, "C · Natural minor");
            unchanged(&drawing, &before);
            let dismissed = app.key("Escape");
            unchanged(&dismissed, &before);
            let notes = app.click("Notes");
            label(&notes, "Quantize...");
            label(&notes, "Randomize...");
            let preview = app.click("Quantize...");
            label(&preview, "Apply quantize");
            unchanged(&preview, &before);
            let cancelled = app.click("Cancel quantize");
            unchanged(&cancelled, &before);
            app.click("Notes");
            let preview = app.click("Randomize...");
            label(&preview, "Apply randomize");
            unchanged(&preview, &before);
            let cancelled = app.click("Cancel randomize");
            unchanged(&cancelled, &before);
        }
    }
    app.quit();
}
