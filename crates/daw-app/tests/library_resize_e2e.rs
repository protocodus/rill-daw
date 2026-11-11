//! The instrument-assignment browser page follows the workspace height.
mod common;

use common::Harness;
use serde_json::{Value, json};

fn height(rect: &Value) -> f64 {
    rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap()
}

fn contains(outer: &Value, inner: &Value) -> bool {
    (0..2).all(|axis| {
        inner["min"][axis].as_f64().unwrap() >= outer["min"][axis].as_f64().unwrap() - 1.0
            && inner["max"][axis].as_f64().unwrap() <= outer["max"][axis].as_f64().unwrap() + 1.0
    })
}

fn settle(app: &mut Harness) -> Value {
    for _ in 0..5 {
        app.inspect();
    }
    app.inspect()
}

fn viewport_height(app: &mut Harness, height: f64) -> Value {
    app.command(json!({"command":"viewport", "width":1600, "height":height}));
    settle(app)
}

fn assert_bounded(state: &Value) {
    let library = &state["library"];
    let viewport = json!({"min":[0.0, 0.0], "max":state["viewport"]});
    assert!(contains(&viewport, &library["window"]), "{library}");
    for pane in ["catalog", "sidebar", "details", "footer"] {
        if !library[pane].is_null() {
            assert!(
                contains(&library["window"], &library[pane]),
                "{pane}: {library}"
            );
        }
    }
    assert!(
        library["catalog"]["max"][1].as_f64().unwrap()
            <= library["footer"]["min"][1].as_f64().unwrap()
    );
    for action in ["Assign instrument", "More"] {
        let label = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|label| label["text"] == action && contains(&library["footer"], label))
            .unwrap_or_else(|| panic!("{action} must remain in the browser footer: {library}"));
        assert!(contains(&viewport, label));
    }
}

#[test]
fn instrument_browser_follows_the_workspace_height_and_keeps_panes_and_actions_inside() {
    let fixture = common::plugin_fixture();
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::with_vst_path(fixture.path());
        app.command(json!({"command":"viewport", "width":1600, "height":1200,
            "pixels_per_point":density}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.click("Track");
        app.click("Add MIDI track");
        common::wait_library_device(&mut app, "Rill Fixture A");
        app.click("Rill Fixture A");
        let initial = settle(&mut app);
        assert_bounded(&initial);
        // A page fills the workspace, so the window itself sizes the browser.
        let smaller = viewport_height(&mut app, 1060.0);
        assert!(
            height(&smaller["library"]["window"]) < height(&initial["library"]["window"]) - 100.0,
            "The browser must follow a vertical shrink: {} -> {}",
            initial["library"],
            smaller["library"]
        );
        assert!(
            height(&smaller["library"]["catalog"]) < height(&initial["library"]["catalog"]) - 70.0
        );
        assert_bounded(&smaller);
        let larger = viewport_height(&mut app, 1160.0);
        assert!(
            height(&larger["library"]["window"]) > height(&smaller["library"]["window"]) + 70.0,
            "The browser must follow a vertical grow: {} -> {}",
            smaller["library"],
            larger["library"]
        );
        assert!(
            height(&larger["library"]["catalog"]) > height(&smaller["library"]["catalog"]) + 70.0
        );
        assert_bounded(&larger);
        assert_eq!(larger["project"], initial["project"]);
        assert_eq!(larger["history"], initial["history"]);

        app.command(json!({"command":"viewport", "width":800, "height":600}));
        let compact = settle(&mut app);
        assert_bounded(&compact);
        let menu = app.click("More");
        assert_eq!(
            menu["native_menu"]["open"], true,
            "The compact footer must accept mouse clicks"
        );
        app.key("Escape");
        app.key("Escape");
        app.quit();
    }
}
