//! Every arranger mode remains directly accessible when its toolbar wraps.
mod common;

use common::Harness;
use eframe::egui;
use serde_json::{Value, json};

const TOOLS: &[(&str, &str, &str)] = &[
    ("Select", "Q", "Select"),
    ("Pencil", "W", "Pencil"),
    ("Cursor", "P", "Cursor"),
    ("Cut", "E", "Cut"),
    ("Join", "T", "Glue"),
    ("Erase", "Z", "Erase"),
    ("Duplicate", "R", "Duplicate"),
    ("Ghost copy", "G", "Ghost"),
    ("Stretch", "S", "Stretch"),
    ("Slip", "L", "Slip"),
];

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

fn full_button_text_width(text: &str, density: f32) -> f32 {
    let ctx = egui::Context::default();
    daw_ui::theme::apply(&ctx);
    ctx.set_pixels_per_point(density);
    let mut width = 0.0;
    for _ in 0..2 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let font = egui::TextStyle::Button.resolve(&ctx.style());
            width = ctx.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE)
                    .size()
                    .x
            });
        });
    }
    width
}

fn tool_labels_fit(state: &Value, width: i32, density: f32) {
    for name in ["More tools", "Edit tools", "Copy tools", "Timing tools"] {
        assert!(
            !state["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == name),
            "{name} must not hide individual tools"
        );
    }
    for (name, _, _) in TOOLS {
        let [left, top, right, bottom] = bounds(label(state, name));
        assert!(left >= 0.0 && right <= f64::from(width));
        assert!(top >= 0.0 && bottom <= 720.0);
        let painted = state["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .find(|shape| shape["text"] == *name)
            .unwrap();
        let [left, _, right, _] = bounds(&painted["rect"]);
        assert!(
            right - left + 0.5 >= f64::from(full_button_text_width(name, density)),
            "{name} must retain its full name at {width}"
        );
    }
}

fn hovered_target(state: &Value, name: &str) -> [f64; 4] {
    let [left, top, right, bottom] = bounds(label(state, name));
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
                && *y1 - *y0 <= f64::from(daw_ui::tokens::CONTROL_HEIGHT) + 1.0
                && *x1 - *x0 <= right - left + 80.0
        })
        .min_by(|a, b| ((a[2] - a[0]) * (a[3] - a[1])).total_cmp(&((b[2] - b[0]) * (b[3] - b[1]))))
        .unwrap_or_else(|| panic!("{name} needs a full native hover target"))
}

#[test]
fn individual_arranger_tools_keep_full_targets_mouse_access_and_shortcut_parity() {
    let mut app = Harness::new();
    app.click("Add pattern");
    app.click("Arranger");
    let before = app.inspect();
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        for width in [800, 1100, 1480] {
            app.command(json!({"command":"viewport", "width":width, "height":720, "pixels_per_point":density}));
            app.click("Select");
            let state = app.command(json!({"command":"inspect", "paint":true}));
            let initial_snap = label(&state, "Snap 1/16").clone();
            tool_labels_fit(&state, width, density);
            for name in [
                "Nudge",
                "Add pattern",
                "View",
                "Snap 1/16",
                "Fit",
                "Tool help",
            ] {
                let [left, top, right, bottom] = bounds(label(&state, name));
                assert!(left >= 0.0 && right <= f64::from(width));
                assert!(top >= 0.0 && bottom <= 720.0);
            }
            let mut targets: Vec<(&str, [f64; 4])> = Vec::new();
            for &(name, key, mode) in TOOLS {
                let [left, top, right, bottom] = bounds(label(&state, name));
                app.command(json!({"command":"pointer", "phase":"move", "x":(left+right)*0.5, "y":(top+bottom)*0.5}));
                let hovered = app.command(json!({"command":"inspect", "paint":true}));
                let target = hovered_target(&hovered, name);
                assert!(target[0] >= 0.0 && target[2] <= f64::from(width));
                assert!(target[1] >= 0.0 && target[3] <= 720.0);
                for &(other, [left, top, right, bottom]) in &targets {
                    assert!(
                        target[2] <= left
                            || target[0] >= right
                            || target[3] <= top
                            || target[1] >= bottom,
                        "{name} overlaps {other} at width {width}"
                    );
                }
                targets.push((name, target));
                let chosen = app.click(name);
                assert_eq!(chosen["timeline"]["tool"], mode);
                unchanged(&chosen, &before);
                let chosen = app.command(json!({"command":"inspect", "paint":true}));
                tool_labels_fit(&chosen, width, density);
                assert_eq!(
                    label(&chosen, "Snap 1/16"),
                    &initial_snap,
                    "Mode {mode} at width {width} must not move adjacent controls"
                );
                let help = app.click("Tool help");
                label(&help, &format!("{name} · {key}"));
                app.key("Escape");
                app.click(if name == "Select" { "Pencil" } else { "Select" });
                let shortcut = app.key(key);
                assert_eq!(shortcut["timeline"]["tool"], mode);
                unchanged(&shortcut, &before);
                assert_eq!(label(&shortcut, "Snap 1/16"), &initial_snap);
            }
        }
    }
    app.quit();
}
