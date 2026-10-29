//! Routing retains useful send controls below its fixed navigation and AI row.
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
    let y = window["max"][1].as_f64().unwrap() - 50.0;
    app.command(json!({"command":"scroll","x":x,"y":y,"delta":direction * 10000.0}));
    for _ in 0..12 {
        app.inspect();
    }
    app.inspect()
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
        app.click("Mixer");
        app.click("Source");
        app.click("Routing");
        // Preserve legitimate remembered scroll positions, then use the mouse
        // to bring the first send back into view after a display change.
        let opened = scroll(&mut app, 1.0);
        assert_eq!(opened["popups"]["routing"], true);
        assert_eq!(opened["project"], original["project"]);
        for text in [
            "AI for this context",
            "Return 0",
            "Level",
            "Pre",
            "Post",
            "Automated",
        ] {
            label(&opened, text);
        }

        let bottom = scroll(&mut app, -1.0);
        for text in ["OUTPUT", "Master output", "+ Send", "Create send track"] {
            label(&bottom, text);
        }
        assert_eq!(bottom["project"], original["project"]);
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
        let top = scroll(&mut app, 1.0);
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
