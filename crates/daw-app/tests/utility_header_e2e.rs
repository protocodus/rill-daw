//! Channel utility windows share one header row: page links in a fixed
//! order with the current window selected, the contextual AI action trailing,
//! and bodies that start at the same height.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

/// Click the front-most painted label (the window's, not the mixer's behind it).
fn click_front(app: &mut Harness, text: &str) -> Value {
    let state = app.inspect();
    let rect = label(&state, text);
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = center_y(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn center_y(rect: &Value) -> f64 {
    (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5
}

fn left(rect: &Value) -> f64 {
    rect["min"][0].as_f64().unwrap()
}

/// The header row of the front window; returns the y of its first body line.
fn assert_header(state: &Value, first_body_line: &str) -> f64 {
    let row = center_y(label(state, "AI for this context"));
    let links = ["Channel", "Effects", "Routing", "Mixer"];
    for link in links {
        assert!(
            (center_y(label(state, link)) - row).abs() < 0.75,
            "{link} shares the header row with AI"
        );
    }
    for pair in links.windows(2) {
        assert!(
            left(label(state, pair[0])) < left(label(state, pair[1])),
            "{pair:?}"
        );
    }
    assert!(left(label(state, "Mixer")) < left(label(state, "AI for this context")));
    // Windows hug their content and sit centered, so measure from each one's top.
    label(state, first_body_line)["min"][1].as_f64().unwrap()
        - state["window_rects"][0]["min"][1].as_f64().unwrap()
}

#[test]
fn effects_and_routing_share_one_header_and_switch_through_it() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        project.add_audio_track("Lead");
        project.add_bus("Room");
        let path = app.directory.path().join("utility-header.rill");
        project.save(&path).unwrap();
        let original = app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1440, "height":1000,
            "pixels_per_point":density}));
        app.click("Mixer");
        app.click("Lead");
        let effects = click_front(&mut app, "Effects");
        assert_eq!(effects["popups"]["effects"], true);
        let effects_body = assert_header(&effects, "Inserts");
        // The current window stays in the row; the next one is a click away.
        let routing = click_front(&mut app, "Routing");
        assert_eq!(routing["popups"]["routing"], true);
        let routing_body = assert_header(&routing, "Output");
        assert!(
            (effects_body - routing_body).abs() < 1.0,
            "bodies start at one height: {effects_body} vs {routing_body}"
        );
        let back = click_front(&mut app, "Effects");
        assert_eq!(back["popups"]["effects"], true);
        assert_eq!(back["project"], original["project"]);
        app.quit();
    }
}
