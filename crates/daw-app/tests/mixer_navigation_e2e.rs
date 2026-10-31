//! Middle-button mixer navigation must never become a channel edit.
mod common;
use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn label_rect<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn paint(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect","paint":true}))
}

fn fader(state: &Value, channel: usize) -> Value {
    state["mixer"]["channels"][channel]["fader_scale"]["rail"].clone()
}

fn pan_dial(state: &Value, channel: usize) -> Value {
    state["mixer"]["channels"][channel]["pan_knob"]["dial"].clone()
}

#[test]
fn middle_drag_over_mixer_faders_pans_without_editing_and_primary_drag_still_edits() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        for channel in 0..10 {
            let id = project.add_bus(format!("Bus {channel}"));
            let track = project.track_mut(id).unwrap();
            track.gain = 0.25;
            track.pan = 0.35;
        }
        let path = app.directory.path().join("mixer-navigation.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(
            json!({"command":"viewport","width":800,"height":600,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        app.click("Mixer");
        let original = paint(&mut app);
        let rail = fader(&original, 2);
        let x = (rail["min"][0].as_f64().unwrap() + rail["max"][0].as_f64().unwrap()) * 0.5;
        // Start far from the gain handle: even the takeover frame must not
        // change its value, before any scrolling movement has occurred.
        let y = rail["min"][1].as_f64().unwrap() * 0.7 + rail["max"][1].as_f64().unwrap() * 0.3;
        let initial_left = label_rect(&original, "Bus 3")["min"][0].as_f64().unwrap();
        let unchanged = |state: &Value| {
            assert_eq!(state["project"], original["project"]);
            assert_eq!(state["history"], original["history"]);
            assert_eq!(state["selected_track"], original["selected_track"]);
        };
        app.command(json!({"command":"pointer","phase":"move","x":760.0,"y":570.0}));
        let down =
            app.command(json!({"command":"pointer","phase":"down","button":"middle","x":x,"y":y}));
        unchanged(&down);
        let pressed = paint(&mut app);
        assert_eq!(
            label_rect(&pressed, "Bus 3")["min"][0].as_f64().unwrap(),
            initial_left,
            "Arriving at the fader and pressing middle must not jump the viewport"
        );
        for distance in [50.0, 100.0] {
            let held = app.command(json!({"command":"pointer","phase":"move","button":"middle","x":x-distance,"y":y+20.0}));
            unchanged(&held);
        }
        let up = app.command(
            json!({"command":"pointer","phase":"up","button":"middle","x":x-100.0,"y":y+20.0}),
        );
        unchanged(&up);
        let panned = paint(&mut app);
        unchanged(&panned);
        let final_left = label_rect(&panned, "Bus 3")["min"][0].as_f64().unwrap();
        assert_eq!(
            final_left,
            label_rect(&up, "Bus 3")["min"][0].as_f64().unwrap(),
            "Middle-button release must not fling the viewport"
        );
        assert!(
            final_left < initial_left - 80.0,
            "The overflowing mixer must actually pan"
        );

        // Repeat from the rotary pan dial. Expanding first resets the
        // horizontal offset, then the compact pane overflows again.
        app.command(json!({"command":"viewport","width":2400,"height":760}));
        app.command(json!({"command":"viewport","width":800,"height":600}));
        let before_pan = paint(&mut app);
        let dial = pan_dial(&before_pan, 2);
        let x = (dial["min"][0].as_f64().unwrap() + dial["max"][0].as_f64().unwrap()) * 0.5;
        let y = (dial["min"][1].as_f64().unwrap() + dial["max"][1].as_f64().unwrap()) * 0.5;
        let left_before = label_rect(&before_pan, "Bus 3")["min"][0].as_f64().unwrap();
        let down =
            app.command(json!({"command":"pointer","phase":"down","button":"middle","x":x,"y":y}));
        unchanged(&down);
        assert_eq!(
            label_rect(&down, "Bus 3")["min"][0].as_f64().unwrap(),
            left_before
        );
        let held = app.command(
            json!({"command":"pointer","phase":"move","button":"middle","x":x-80.0,"y":y-10.0}),
        );
        unchanged(&held);
        let up = app.command(
            json!({"command":"pointer","phase":"up","button":"middle","x":x-80.0,"y":y-10.0}),
        );
        unchanged(&up);
        let panned = paint(&mut app);
        unchanged(&panned);
        let left_after = label_rect(&panned, "Bus 3")["min"][0].as_f64().unwrap();
        assert!(
            left_after < left_before - 60.0,
            "Middle drag over the pan control must scroll too"
        );
        assert_eq!(
            left_after,
            label_rect(&up, "Bus 3")["min"][0].as_f64().unwrap()
        );

        // A regular primary fader edit still changes the level and undoes once.
        app.command(json!({"command":"viewport","width":2400,"height":760}));
        let reset = paint(&mut app);
        let rail = fader(&reset, 2);
        let x = (rail["min"][0].as_f64().unwrap() + rail["max"][0].as_f64().unwrap()) * 0.5;
        let y = rail["min"][1].as_f64().unwrap() * 0.5 + rail["max"][1].as_f64().unwrap() * 0.5;
        let edited = app.command(
            json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x,"to_y":y-30.0}),
        );
        assert!(edited["project"]["tracks"][2]["gain"].as_f64().unwrap() > 0.25);
        assert_eq!(
            app.command(json!({"command":"key","key":"Z","cmd":true}))["project"],
            original["project"]
        );
        app.quit();
    }
}
