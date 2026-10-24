//! Native popup close clicks restore the caller, including after text entry.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn window(app: &mut Harness, title: &str) -> Value {
    app.click("Window");
    app.click(title)
}

fn close(app: &mut Harness, title: &str) -> Value {
    let state = app.inspect();
    let title = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == title)
        .unwrap_or_else(|| panic!("Popup title {title:?} missing: {}", state["visible_labels"]));
    let x = (title["min"][0].as_f64().unwrap() + title["max"][0].as_f64().unwrap()) / 2.0;
    let y = (title["min"][1].as_f64().unwrap() + title["max"][1].as_f64().unwrap()) / 2.0;
    let bounds = state["window_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bounds| {
            bounds["min"][0].as_f64().unwrap() < x
                && bounds["max"][0].as_f64().unwrap() > x
                && bounds["min"][1].as_f64().unwrap() < y
                && bounds["max"][1].as_f64().unwrap() > y
        })
        .expect("visible popup contains its title");
    // Native close and title share a centerline. Its distance from the right
    // edge equals that centerline's distance from the top, including the border.
    // Derive from rendered bounds instead of hardcoding the component's padding.
    let x = bounds["max"][0].as_f64().unwrap() - (y - bounds["min"][1].as_f64().unwrap());
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

#[test]
fn close_buttons_and_menus_work_after_typing_and_restore_the_editor() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Popup synth");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Popup pattern",
        0.0,
        8.0,
        vec![Note::new(60, 1.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("popups.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Popup pattern");
    window(&mut app, "Clip Editor");

    for dark in [false, true] {
        for density in [1.0, 2.0] {
            app.command(json!({"command":"system_theme", "dark":dark}));
            app.command(json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}));
            let editor = app.inspect();
            let properties = app.click("Properties");
            assert_eq!(properties["popups"]["properties"], true);
            let name = properties["project"]["tracks"][0]["clips"][0]["name"]
                .as_str()
                .unwrap();
            app.click(name);
            app.command(json!({"command":"key", "key":"A", "cmd":true}));
            let name = format!("Edited pattern {dark} {density}");
            let edited = app.command(json!({"command":"text", "text":name}));
            assert_eq!(edited["project"]["tracks"][0]["clips"][0]["name"], name);

            // No Escape or blur first: a menu must still work with text focus.
            let instrument = window(&mut app, "Instrument");
            assert_eq!(instrument["popups"]["instrument"], true);
            let restored = close(&mut app, "Instrument");
            assert_eq!(restored["popups"]["instrument"], false);
            assert_eq!(restored["popups"]["properties"], true);
            app.click(&name);
            let restored = close(&mut app, "Properties");
            assert_eq!(restored["popups"]["properties"], false);
            assert_eq!(restored["view"], "Clip");
            assert_eq!(restored["selected_clip"], editor["selected_clip"]);
            assert_eq!(restored["project"], edited["project"]);
            assert_eq!(restored["history"], edited["history"]);

            let automation = window(&mut app, "Automation");
            assert_eq!(automation["popups"]["automation"], true);
            let restored = close(&mut app, "Add automation");
            assert_eq!(restored["popups"]["automation"], false);
            assert_eq!(restored["view"], "Clip");
            assert_eq!(restored["project"], edited["project"]);
            assert_eq!(restored["history"], edited["history"]);

            let library = window(&mut app, "Library");
            assert_eq!(library["popups"]["library"], true);
            let restored = close(&mut app, "Device library");
            assert_eq!(restored["popups"]["library"], false);
            assert_eq!(restored["view"], "Clip");
            assert_eq!(restored["project"], edited["project"]);
        }
    }
    app.quit();
}
