//! Focused arranger tools preserve mouse access, keyboard parity and song state.
mod common;

use common::Harness;
use serde_json::{Value, json};

fn label<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]))
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
            "Choosing tools changed {field}"
        );
    }
}

#[test]
fn compact_arranger_modes_stay_visible_and_preserve_selection_history_and_shortcuts() {
    let mut app = Harness::new();
    app.click("Add pattern");
    app.click("Arranger");
    let before = app.inspect();
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [800, 1100, 1480] {
            app.command(json!({"command":"viewport", "width":width, "height":720, "pixels_per_point":density}));
            let state = app.click("Select");
            let initial_snap = label(&state, "Snap 1/16").clone();
            for name in [
                "Select",
                "Pencil",
                "Cursor",
                "Cut",
                "More tools",
                "Nudge",
                "Add pattern",
                "View",
                "Snap 1/16",
                "Fit",
            ] {
                let rect = label(&state, name);
                assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
                assert!(rect["max"][0].as_f64().unwrap() <= f64::from(width));
                if width >= 1100 {
                    assert!(
                        (rect["min"][1].as_f64().unwrap()
                            - label(&state, "Select")["min"][1].as_f64().unwrap())
                        .abs()
                            <= 3.0,
                        "One editing row at {width}: {name} {}",
                        state["label_rects"]
                    );
                }
            }
            for (name, key, mode) in [
                ("Duplicate", "R", "Duplicate"),
                ("Join", "T", "Glue"),
                ("Ghost copy", "G", "Ghost"),
                ("Stretch", "S", "Stretch"),
                ("Slip", "L", "Slip"),
                ("Erase", "Z", "Erase"),
            ] {
                app.click("Select");
                let menu = app.click("More tools");
                for advanced in [
                    "Duplicate",
                    "Join",
                    "Ghost copy",
                    "Stretch",
                    "Slip",
                    "Erase",
                ] {
                    label(&menu, advanced);
                }
                let chosen = app.click(name);
                assert_eq!(chosen["timeline"]["tool"], mode);
                unchanged(&chosen, &before);
                label(&chosen, name);
                assert_eq!(
                    label(&chosen, "Snap 1/16"),
                    &initial_snap,
                    "Mode changes do not move adjacent controls"
                );
                app.click(name);
                let dismissed = app.key("Escape");
                assert_eq!(dismissed["timeline"]["tool"], mode);
                app.click("Select");
                let shortcut = app.key(key);
                assert_eq!(shortcut["timeline"]["tool"], mode);
                unchanged(&shortcut, &before);
                label(&shortcut, name);
            }
        }
    }
    app.quit();
}
