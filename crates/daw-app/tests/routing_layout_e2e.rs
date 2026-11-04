//! Routing lists the output, then its sends, below its fixed navigation and AI row.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use serde_json::{Value, json};

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]));
    assert!(
        rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap() >= 13.0,
        "Clipped {text}: {rect}"
    );
    for axis in 0..2 {
        assert!(rect["min"][axis].as_f64().unwrap() >= 0.0, "{rect}");
        assert!(
            rect["max"][axis].as_f64().unwrap() <= state["viewport"][axis].as_f64().unwrap(),
            "{rect}"
        );
    }
    rect
}

fn scroll(app: &mut Harness, direction: f64) -> Value {
    let state = app.inspect();
    let window = &state["window_rects"][0];
    let x = (window["min"][0].as_f64().unwrap() + window["max"][0].as_f64().unwrap()) * 0.5;
    // Wheel over the scrolling body, above the fixed action row.
    let y = window["max"][1].as_f64().unwrap() - 100.0;
    app.command(json!({"command":"scroll","x":x,"y":y,"delta":direction * 10000.0}));
    for _ in 0..12 {
        app.inspect();
    }
    app.inspect()
}

/// Wheel the routing body down in small steps until `text` is fully visible.
fn reveal(app: &mut Harness, text: &str) -> Value {
    for _ in 0..40 {
        let state = app.inspect();
        if let Some(rect) = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == text)
            && rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap() >= 13.0
        {
            return state;
        }
        let window = &state["window_rects"][0];
        let x = (window["min"][0].as_f64().unwrap() + window["max"][0].as_f64().unwrap()) * 0.5;
        let y = window["max"][1].as_f64().unwrap() - 100.0;
        app.command(json!({"command":"scroll","x":x,"y":y,"delta":-34.0}));
    }
    panic!("{text} never became visible");
}

#[test]
fn routing_from_properties_keeps_sends_and_output_reachable_across_live_display_changes() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    let buses: Vec<_> = (0..8)
        .map(|index| project.add_bus(format!("Return {index}")))
        .collect();
    for bus in &buses {
        project.add_send(source, *bus, 0.5, false).unwrap();
    }
    let lane = project
        .add_automation_track(
            "Send envelope",
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::SendGain { bus: buses[0] },
            },
        )
        .unwrap();
    project.automation_lane_mut(lane).unwrap().points = vec![AutomationPoint::new(0.0, 0.5)];
    let path = app.directory.path().join("routing-layout.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open","path":path}));

    // Reuse one process: a previously remembered popup height must remain
    // useful after live theme, display density, and viewport changes.
    for (width, height, dark, density) in [
        (1100, 850, false, 1.0),
        (1100, 850, true, 2.0),
        (800, 600, false, 1.0),
        (800, 600, true, 2.0),
    ] {
        app.command(
            json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        let mixer = app.click("Mixer");
        let pane = &mixer["mixer"]["reorder_viewport"];
        app.command(json!({"command":"scroll",
            "x":pane["min"][0].as_f64().unwrap() + 20.0,
            "y":pane["min"][1].as_f64().unwrap() + 20.0,
            "delta":0.0,"delta_x":10000.0}));
        for _ in 0..12 {
            app.inspect();
        }
        app.click("Source");
        app.click("Routing");
        // Preserve legitimate remembered scroll positions, then use the mouse
        // to bring the first send back into view after a display change.
        let opened = scroll(&mut app, 1.0);
        assert_eq!(opened["popups"]["routing"], true);
        assert_eq!(opened["project"], original["project"]);
        // Signal order: the output, then the sends with their level in dB.
        for text in ["AI for this context", "Output", "Master output"] {
            label(&opened, text);
        }
        let sends = reveal(&mut app, "Automated");
        // Return 0 follows its automation lane (0.5 of 16×, +18.1 dB).
        for text in ["Return 0", "18.1 dB", "Pre", "Post", "Automated"] {
            label(&sends, text);
        }
        let row = label(&sends, "Return 0")["min"][1].as_f64().unwrap();
        for text in ["Output", "Sends"] {
            if let Some(heading) = sends["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == text)
            {
                assert!(
                    heading["max"][1].as_f64().unwrap() < row,
                    "{text} precedes the sends"
                );
            }
        }

        let bottom = scroll(&mut app, -1.0);
        for text in ["Add send…", "New return with effect…"] {
            label(&bottom, text);
        }
        assert_eq!(bottom["project"], original["project"]);
        scroll(&mut app, 1.0);
        let choices = app.click("Master output");
        let destination = choices["menu_label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == "Return 3")
            .unwrap_or_else(|| {
                panic!(
                    "No menu destination: menus={} labels={}",
                    choices["menu_label_rects"], choices["visible_labels"]
                )
            });
        let x = (destination["min"][0].as_f64().unwrap() + destination["max"][0].as_f64().unwrap())
            * 0.5;
        let y = (destination["min"][1].as_f64().unwrap() + destination["max"][1].as_f64().unwrap())
            * 0.5;
        let routed = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
        assert_eq!(routed["project"]["tracks"][0]["output_bus"], buses[3]);
        assert_eq!(
            routed["project"]["tracks"][0]["sends"],
            original["project"]["tracks"][0]["sends"]
        );
        app.click("Edit");
        assert_eq!(app.click("Undo")["project"], original["project"]);
        scroll(&mut app, 1.0);
        let top = reveal(&mut app, "Automated");
        label(&top, "Return 0");
        label(&top, "Automated");
        app.command(json!({"command":"click","label":"Pre","occurrence":0}));
        let changed = app.inspect();
        assert_eq!(
            changed["project"]["tracks"][0]["sends"][0]["pre_fader"],
            true
        );
        app.click("Edit");
        assert_eq!(app.click("Undo")["project"], original["project"]);
    }
    app.quit();
}
