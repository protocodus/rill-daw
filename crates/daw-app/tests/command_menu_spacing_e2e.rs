//! Generous command menus remain bounded and mouse-scrollable on short displays.
mod common;

use common::Harness;
use serde_json::{Value, json};

fn label<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|rect| rect["text"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]))
}

#[test]
fn top_level_menus_scroll_inside_the_viewport_and_last_window_action_is_clickable() {
    let mut app = Harness::new();
    for dark in [false, true] {
        for density in [1.0, 1.25, 2.0] {
            app.command(json!({"command":"viewport", "width":800, "height":600,
                "pixels_per_point":density}));
            app.command(json!({"command":"system_theme", "dark":dark}));
            for menu in ["File", "Edit", "Track", "Help", "Window"] {
                app.click(menu);
                for _ in 0..3 {
                    app.inspect();
                }
                let state = app.inspect();
                let original = state["project"].clone();
                let scroll = &state["native_menu"]["scroll_rect"];
                assert!(scroll.is_array(), "{menu} must have a bounded command body");
                let x = (scroll[0].as_f64().unwrap() + scroll[2].as_f64().unwrap()) * 0.5;
                let y = (scroll[1].as_f64().unwrap() + scroll[3].as_f64().unwrap()) * 0.5;
                app.command(json!({"command":"scroll", "x":x, "y":y, "delta":-2000.0}));
                for _ in 0..15 {
                    app.inspect();
                }
                let state = app.inspect();
                assert_eq!(state["project"], original);
                for area in state["native_menu"]["foreground_areas"].as_array().unwrap() {
                    assert!(area[0].as_f64().unwrap() >= 0.0, "{menu}: {area}");
                    assert!(area[1].as_f64().unwrap() >= 0.0, "{menu}: {area}");
                    assert!(area[2].as_f64().unwrap() <= 800.0, "{menu}: {area}");
                    assert!(area[3].as_f64().unwrap() <= 600.0, "{menu}: {area}");
                }
                if menu == "Window" {
                    let setup = label(&state, "Setup");
                    let x = (setup["min"][0].as_f64().unwrap() + setup["max"][0].as_f64().unwrap())
                        * 0.5;
                    let y = (setup["min"][1].as_f64().unwrap() + setup["max"][1].as_f64().unwrap())
                        * 0.5;
                    assert!(y + 17.0 <= 600.0, "The entire Setup target must fit");
                    let opened =
                        app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
                    assert_eq!(opened["popups"]["setup"], true);
                    app.click("Cancel");
                } else {
                    app.command(json!({"command":"pointer", "phase":"click", "x":790, "y":590}));
                }
            }
        }
    }
    app.quit();
}
