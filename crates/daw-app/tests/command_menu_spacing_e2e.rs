//! Compact command menus retain full mouse targets and scroll on short displays.
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

fn center_y(label: &Value) -> f64 {
    (label["min"][1].as_f64().unwrap() + label["max"][1].as_f64().unwrap()) * 0.5
}

#[test]
fn main_menu_uses_tighter_vertical_insets_and_row_gaps() {
    let mut app = Harness::new();
    for dark in [false, true] {
        for density in [1.0, 1.25, 2.0] {
            app.command(json!({"command":"viewport", "width":800, "height":600,
                "pixels_per_point":density}));
            let state = app.command(json!({"command":"system_theme", "dark":dark}));
            let expected = daw_ui::tokens::SPACE_XS + daw_ui::tokens::CONTROL_HEIGHT * 0.5;
            assert!(
                (center_y(label(&state, "File")) - f64::from(expected)).abs() <= 1.0,
                "Menu titles use 5-point vertical panel padding at {density}×: {}",
                label(&state, "File"),
            );
            app.click("File");
            for _ in 0..3 {
                app.inspect();
            }
            let state = app.inspect();
            let stride = center_y(label(&state, "Open…")) - center_y(label(&state, "New"));
            let expected = daw_ui::tokens::CONTROL_HEIGHT + daw_ui::tokens::SPACE_XS;
            assert!(
                (stride - f64::from(expected)).abs() <= 1.0,
                "Full-height commands use only a 5-point row gap at {density}×: {stride}",
            );
            // Exercise a real mouse action after the geometry changed.
            let state = app.click("New");
            assert_eq!(state["native_menu"]["open"], false);
            assert!(state["project"]["tracks"].as_array().unwrap().is_empty());
        }
    }
    app.quit();
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
