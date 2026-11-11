//! Utility windows and pages leave wrapped transport controls reachable at every size.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn label<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]))
}

fn popup_bounds<'a>(state: &'a Value, title: &str) -> &'a Value {
    let [left, top, right, bottom] = bounds(label(state, title));
    state["window_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|window| {
            let [x0, y0, x1, y1] = bounds(window);
            x0 <= left && y0 <= top && x1 >= right && y1 >= bottom
        })
        .unwrap_or_else(|| panic!("No window contains {title}: {}", state["window_rects"]))
}

fn unchanged(state: &Value, original: &Value) {
    for field in [
        "project",
        "history",
        "selected_track",
        "selected_clip",
        "modified",
    ] {
        assert_eq!(state[field], original[field], "Navigation changed {field}");
    }
}

#[test]
fn utility_windows_fit_below_transport_and_close_with_the_mouse_in_both_themes() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Studio synth");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Studio phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 2.0, 100)],
    ));
    let path = app.directory.path().join("studio-windows.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let original = app.click("Studio phrase");

    for dark in [false, true] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        // Large windows first, then a compact viewport: persisted popup positions
        // must be constrained again when the transport wraps onto another row.
        for (width, height, density) in [(1600, 1000, 2.0), (800, 600, 1.0)] {
            app.command(json!({"command":"viewport", "width":width, "height":height,
                "pixels_per_point":density}));
            let margin = f64::from(daw_ui::tokens::POPUP_MARGIN) - 1.0 / density;
            for (menu, key, title, action) in [
                ("Effects", "effects", "Effects of Studio synth", "+ Insert"),
                (
                    "Routing",
                    "routing",
                    "Routing of Studio synth",
                    "Master output",
                ),
                ("Setup", "setup", "Setup", "Apply"),
                (
                    "Properties",
                    "properties",
                    "Properties · Studio phrase",
                    "Name",
                ),
                (
                    "Device browser",
                    "library",
                    "Device browser",
                    "Search instruments",
                ),
            ] {
                app.click("Window");
                app.click(menu);
                let state = app.inspect();
                assert_eq!(state["popups"][key], true, "{menu}");
                unchanged(&state, &original);
                let [left, top, right, bottom] = bounds(popup_bounds(&state, title));
                if menu == "Effects" {
                    assert!(
                        left >= margin && right <= width as f64 - margin,
                        "{title}: {left}..{right}"
                    );
                    assert!(bottom <= height as f64 - margin, "{title}: bottom {bottom}");
                } else {
                    // A page fills the workspace below the transport.
                    assert_eq!(
                        [left, right, bottom],
                        [0.0, width as f64, height as f64],
                        "{title}"
                    );
                }

                for control in ["Start", "Play", "Stop", "Record", "Click", "Loop", "Range"] {
                    // Properties can also contain Start/Loop fields. Transport
                    // controls are the uppermost matching label in the frame.
                    let transport = state["label_rects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|label| label["text"] == control)
                        .min_by(|a, b| bounds(a)[1].total_cmp(&bounds(b)[1]))
                        .unwrap();
                    let [x0, _, x1, y1] = bounds(transport);
                    assert!(
                        x0 >= 0.0 && x1 <= width as f64,
                        "Transport {control} is clipped"
                    );
                    assert!(
                        top >= y1 + 8.0,
                        "{title} overlaps transport {control} at {width}px: window top {top}, label bottom {y1}"
                    );
                }
                for control in [title, action] {
                    let [x0, y0, x1, y1] = bounds(label(&state, control));
                    assert!(
                        x0 >= left && y0 >= top && x1 <= right && y1 <= bottom,
                        "{title}: {control} is outside its window"
                    );
                }

                // Drive the actual close target (a popup's close button, a page's
                // Back); no Escape shortcut may substitute for mouse dismissal.
                let restored = common::close_utility(&mut app, &state, title);
                assert_eq!(
                    restored["popups"][key], false,
                    "Mouse close failed for {title}"
                );
                assert!(
                    restored["popups"]
                        .as_object()
                        .unwrap()
                        .values()
                        .all(|open| open == false)
                );
                assert_eq!(restored["view"], "Arrangement");
                unchanged(&restored, &original);
            }
        }
    }
    app.quit();
}
