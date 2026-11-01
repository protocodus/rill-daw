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

fn glowing_border(state: &Value, text: &str, dark: bool) -> bool {
    let [left, top, right, bottom] = bounds(label(state, text));
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
            if shape["stroke"] != json!(accent.to_array())
                || (shape["stroke_width"].as_f64().unwrap_or(0.0)
                    - f64::from(daw_ui::tokens::FOCUS_WIDTH))
                .abs()
                    > 0.01
                || x0 > left
                || x0 < left - 55.0
                || x1 < right
                || x1 > right + 34.0
                || y0 > top
                || y0 < top - 21.0
                || y1 < bottom
                || y1 > bottom + 21.0
            {
                return false;
            }
            let extent = f64::from(daw_ui::tokens::SELECTION_GLOW_EXTENT);
            let expected = [x0 - extent, y0 - extent, x1 + extent, y1 + extent];
            state["painted_meshes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mesh| {
                    mesh["vertices"] == 128
                        && mesh["indices"] == 576
                        && mesh["transparent_vertices"].as_u64().unwrap() > 0
                        && mesh["bounds"].as_array().unwrap().iter().zip(expected).all(
                            |(actual, expected)| (actual.as_f64().unwrap() - expected).abs() < 0.2,
                        )
                })
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
fn tool_families_wrap_together_and_every_mode_remains_mouse_accessible() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = fixture(800, 760, density, dark);
        let original = app.inspect();
        for width in [800, 1100, 1600] {
            app.command(json!({"command":"viewport", "width":width, "height":760,
                "pixels_per_point":density}));
            let state = paint(&mut app);
            for family in [
                &["Select", "Pencil", "Multi-draw", "Cursor"][..],
                &["Cut", "Glue", "Erase"][..],
            ] {
                let first = bounds(label(&state, family[0]));
                for text in family {
                    let rect = bounds(label(&state, text));
                    assert!(
                        (rect[1] - first[1]).abs() < 1.0,
                        "Tool family split across rows at {width}: {family:?}"
                    );
                    assert!(rect[0] >= 0.0 && rect[2] <= f64::from(width));
                }
            }
            for (text, expected) in [
                ("Select", "Select"),
                ("Pencil", "Pencil"),
                ("Multi-draw", "MultiDraw"),
                ("Cursor", "Cursor"),
                ("Cut", "Cut"),
                ("Glue", "Glue"),
                ("Erase", "Erase"),
                ("Duplicate", "Duplicate"),
            ] {
                let state = app.click(text);
                mode(&state, expected);
                unchanged(&state, &original);
            }
        }
        app.quit();
    }
}

#[test]
fn tools_keep_a_visible_active_mode_while_commands_and_views_keep_their_own_roles() {
    for (width, height, density) in [(1100, 720, 1.0), (1440, 1000, 2.0)] {
        for dark in [false, true] {
            let mut app = fixture(width, height, density, dark);
            let initial = paint(&mut app);
            mode(&initial, "Select");
            assert!(glowing_border(&initial, "Select", dark));
            assert!(!glowing_border(&initial, "Pencil", dark));

            // All three roles remain visible at narrow and wide desktop sizes.
            for text in ["Select", "Pencil", "Generate…", "View", "Fit"] {
                let [left, top, right, bottom] = bounds(label(&initial, text));
                assert!(left >= 0.0 && top >= 0.0);
                assert!(right <= f64::from(width) && bottom <= f64::from(height));
            }
            for text in ["Generate…", "View", "Fit"] {
                assert!(
                    background(&initial, text).is_none_or(|shape| {
                        shape["fill"][3] == 0 && shape["stroke_width"] == 0.0
                    }),
                    "Idle {text} uses the shared quiet toolbar treatment"
                );
                assert!(!glowing_border(&initial, text, dark));
                let [left, top, right, bottom] = bounds(label(&initial, text));
                app.command(json!({"command":"pointer", "phase":"move",
                    "x":(left+right)*0.5, "y":(top+bottom)*0.5}));
                let hovered = app.command(json!({"command":"inspect", "paint":true}));
                let target = background(&hovered, text).expect("visible native hover target");
                assert!(
                    target["fill"][3].as_u64().unwrap() > 0
                        || target["stroke_width"].as_f64().unwrap() > 0.0
                );
            }
            if let Some(inactive) = background(&initial, "Pencil") {
                assert_eq!(inactive["fill"][3], 0, "Inactive modes remain flat");
                assert_eq!(inactive["stroke_width"], 0.0);
            }

            app.click("Pencil");
            let active = paint(&mut app);
            mode(&active, "Pencil");
            unchanged(&active, &initial);
            assert!(glowing_border(&active, "Pencil", dark));
            assert!(!glowing_border(&active, "Select", dark));
            // Inspecting further frames cannot turn the active mode into a
            // momentary pressed state or lose its non-colour selection marker.
            assert!(glowing_border(&paint(&mut app), "Pencil", dark));

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
            assert!(glowing_border(&paint(&mut app), "Pencil", dark));

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
            app.key("Tab");
            let open_view = paint(&mut app);
            let target = &background(&open_view, "View")
                .expect("An open View menu retains its frame away from the pointer")["rect"];
            let colors = if dark {
                daw_ui::theme::DARK
            } else {
                daw_ui::theme::LIGHT
            };
            let frames: Vec<_> = open_view["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|shape| &shape["rect"] == target)
                .collect();
            assert!(
                frames
                    .iter()
                    .any(|shape| shape["fill"] == json!(colors.track_selected.to_array()))
            );
            assert!(
                frames
                    .iter()
                    .any(|shape| shape["stroke"] == json!(colors.accent.to_array()))
            );
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
            assert!(glowing_border(&paint(&mut app), "Pencil", dark));
            app.quit();
        }
    }
}

#[test]
fn workspace_tabs_keep_the_same_targets_and_persistent_glowing_selection() {
    for (dark, density) in [(false, 1.0), (true, 1.25), (true, 2.0)] {
        let mut app = fixture(1100, 900, density, dark);
        let original = app.inspect();
        let mut labels = None;
        for chosen in ["Arranger", "Mixer", "Arranger"] {
            app.click(chosen);
            app.command(json!({"command":"pointer", "phase":"move", "x":1, "y":1}));
            let state = app.command(json!({"command":"inspect", "paint":true}));
            unchanged(&state, &original);
            let current = [
                bounds(label(&state, "Arranger")),
                bounds(label(&state, "Mixer")),
            ];
            assert_eq!(*labels.get_or_insert(current), current);
            for tab in ["Arranger", "Mixer"] {
                assert_eq!(glowing_border(&state, tab, dark), tab == chosen);
            }
        }
        app.quit();
    }
}
