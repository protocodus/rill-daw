//! Commands, persistent pointer modes and representation controls have distinct roles.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(width: u32, height: u32, density: f64, dark: bool) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":width, "height":height,
        "pixels_per_point":density}));
    app.command(json!({"command":"system_theme", "dark":dark}));
    let mut project = Project::new();
    let track = project.add_midi_track("Control roles");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Role phrase",
        0.0,
        8.0,
        vec![
            Note::new(60, 1.0, 0.5, 70),
            Note::new(61, 2.0, 0.5, 90),
            Note::new(67, 3.0, 0.5, 110),
        ],
    ));
    let path = app.directory.path().join("control-roles.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Role phrase");
    app.click("Window");
    app.click("Clip Editor");
    app
}

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn paint(app: &mut Harness) -> Value {
    // Remove hover as an explanation for the active mode's appearance. Merely
    // moving the pointer into empty canvas space cannot edit any notes.
    let state = app.inspect();
    let [left, top, right, bottom] = bounds(&state["midi_editor"]["plot"]);
    app.command(json!({"command":"pointer", "phase":"move",
        "x":left+(right-left)*0.9, "y":top+(bottom-top)*0.9}));
    app.command(json!({"command":"inspect", "paint":true}))
}

fn background<'a>(state: &'a Value, text: &str) -> Option<&'a Value> {
    let [left, top, right, bottom] = bounds(label(state, text));
    state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|shape| {
            let [x0, y0, x1, y1] = bounds(&shape["rect"]);
            x0 <= left + 0.5
                && y0 <= top + 0.5
                && x1 >= right - 0.5
                && y1 >= bottom - 0.5
                && x1 - x0 <= right - left + 90.0
                && y1 - y0 >= 24.0
                && y1 - y0 <= 44.0
        })
        .min_by(|a, b| {
            let area = |shape: &Value| {
                let [x0, y0, x1, y1] = bounds(&shape["rect"]);
                (x1 - x0) * (y1 - y0)
            };
            area(a).total_cmp(&area(b))
        })
}

fn underline(state: &Value, text: &str, dark: bool) -> bool {
    let [left, _, right, bottom] = bounds(label(state, text));
    let accent = if dark {
        daw_ui::theme::DARK.accent
    } else {
        daw_ui::theme::LIGHT.accent
    };
    state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|shape| {
            let [x0, y0, x1, y1] = bounds(&shape["rect"]);
            shape["fill"] == json!(accent.to_array())
                && (y1 - y0 - 3.0).abs() < 0.2
                && x0 <= left
                && x0 >= left - 45.0
                && x1 >= right - 1.0
                && x1 <= right + 20.0
                && y0 >= bottom - 1.0
                && y1 <= bottom + 15.0
        })
}

fn mode(state: &Value, expected: &str) {
    assert_eq!(state["midi_editor"]["tool"], expected);
}

fn unchanged(state: &Value, original: &Value) {
    assert_eq!(state["project"], original["project"]);
    assert_eq!(state["history"], original["history"]);
    assert_eq!(state["modified"], original["modified"]);
}

#[test]
fn tools_keep_a_visible_active_mode_while_commands_and_views_keep_their_own_roles() {
    for (width, height, density) in [(1100, 720, 1.0), (1440, 1000, 2.0)] {
        for dark in [false, true] {
            let mut app = fixture(width, height, density, dark);
            let initial = paint(&mut app);
            mode(&initial, "Select");
            assert!(underline(&initial, "Select", dark));
            assert!(!underline(&initial, "Pencil", dark));

            // All three roles remain visible at narrow and wide desktop sizes.
            for text in ["Select", "Pencil", "Generate…", "View", "Fit"] {
                let [left, top, right, bottom] = bounds(label(&initial, text));
                assert!(left >= 0.0 && top >= 0.0);
                assert!(right <= f64::from(width) && bottom <= f64::from(height));
            }
            let command = background(&initial, "Generate…").expect("filled command");
            assert!(command["fill"][3].as_u64().unwrap() > 0);
            assert_eq!(command["stroke_width"], 0.0);
            for text in ["View", "Fit"] {
                let view = background(&initial, text).expect("outlined view control");
                assert_eq!(
                    view["fill"][3], 0,
                    "{text} must have an idle transparent fill"
                );
                assert!(view["stroke_width"].as_f64().unwrap() > 0.0);
            }
            if let Some(inactive) = background(&initial, "Pencil") {
                assert_eq!(inactive["fill"][3], 0, "Inactive modes remain flat");
                assert_eq!(inactive["stroke_width"], 0.0);
            }

            app.click("Pencil");
            let active = paint(&mut app);
            mode(&active, "Pencil");
            unchanged(&active, &initial);
            assert!(underline(&active, "Pencil", dark));
            assert!(!underline(&active, "Select", dark));
            // Inspecting further frames cannot turn the active mode into a
            // momentary pressed state or lose its non-colour selection marker.
            assert!(underline(&paint(&mut app), "Pencil", dark));

            app.command(json!({"command":"key", "key":"A", "cmd":true}));
            let before_action = app.inspect();
            app.click("Notes");
            let transposed = app.click("+1 semitone");
            mode(&transposed, "Pencil");
            assert_ne!(transposed["project"], before_action["project"]);
            let pitches: Vec<_> = transposed["project"]["tracks"][0]["clips"][0]["content"]["Midi"]
                ["notes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|note| note["pitch"].as_u64().unwrap())
                .collect();
            assert_eq!(pitches, [61, 62, 68]);
            let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
            mode(&undone, "Pencil");
            assert_eq!(undone["project"], initial["project"]);
            assert!(underline(&paint(&mut app), "Pencil", dark));

            // Generate is a command that opens a decision; cancelling it leaves
            // both the project and the chosen interaction mode intact.
            let before_generate = app.inspect();
            app.click("Generate…");
            let cancelled = app.click("Cancel");
            mode(&cancelled, "Pencil");
            unchanged(&cancelled, &before_generate);

            let before_view = app.inspect();
            app.click("View");
            let highlighted = app.click("Highlight scale");
            assert_ne!(
                highlighted["midi_editor"]["highlight_scale"],
                before_view["midi_editor"]["highlight_scale"]
            );
            unchanged(&highlighted, &before_view);
            let folded = app.click("Only scale keys");
            assert_eq!(folded["midi_editor"]["only_scale_keys"], true);
            assert_eq!(folded["midi_editor"]["hidden_notes"], 1);
            assert!(
                folded["midi_editor"]["visible_pitches"]
                    .as_array()
                    .unwrap()
                    .len()
                    < 128
            );
            mode(&folded, "Pencil");
            unchanged(&folded, &before_view);
            app.key("Escape");
            let fitted = app.click("Fit");
            mode(&fitted, "Pencil");
            unchanged(&fitted, &before_view);
            assert!(underline(&paint(&mut app), "Pencil", dark));
            app.quit();
        }
    }
}
