//! Middle-button mixer navigation must never become a channel edit.
mod common;
use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn channel_left(state: &Value, channel: usize) -> f64 {
    state["mixer"]["channels"][channel]["strip"]["min"][0]
        .as_f64()
        .unwrap()
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

fn reveal_control(app: &mut Harness, target: fn(&Value, usize) -> Value, channel: usize) -> Value {
    let state = paint(app);
    let rect = target(&state, channel);
    let viewport = &state["mixer"]["reorder_viewport"];
    let top = viewport["min"][1].as_f64().unwrap();
    let bottom = viewport["max"][1].as_f64().unwrap();
    if rect["min"][1].as_f64().unwrap() < top + 8.0
        || rect["max"][1].as_f64().unwrap() > bottom - 8.0
    {
        let center = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
        app.command(
            json!({"command":"scroll", "x":viewport["max"][0].as_f64().unwrap() - 5.0,
            "y":(top + bottom) * 0.5, "delta":(top + bottom) * 0.5 - center}),
        );
        for _ in 0..20 {
            app.inspect();
        }
    }
    let visible = paint(app);
    let rect = target(&visible, channel);
    let viewport = &visible["mixer"]["reorder_viewport"];
    assert!(rect["min"][1].as_f64().unwrap() >= viewport["min"][1].as_f64().unwrap());
    assert!(rect["max"][1].as_f64().unwrap() <= viewport["max"][1].as_f64().unwrap());
    for field in ["project", "history", "selected_track"] {
        assert_eq!(visible[field], state[field]);
    }
    visible
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
        let original = reveal_control(&mut app, fader, 2);
        let rail = fader(&original, 2);
        let x = (rail["min"][0].as_f64().unwrap() + rail["max"][0].as_f64().unwrap()) * 0.5;
        // Start far from the gain handle: even the takeover frame must not
        // change its value, before any scrolling movement has occurred.
        let y = rail["min"][1].as_f64().unwrap() * 0.7 + rail["max"][1].as_f64().unwrap() * 0.3;
        let initial_left = channel_left(&original, 3);
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
            channel_left(&pressed, 3),
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
        let final_left = channel_left(&panned, 3);
        assert_eq!(
            final_left,
            channel_left(&up, 3),
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
        let before_pan = reveal_control(&mut app, pan_dial, 2);
        let dial = pan_dial(&before_pan, 2);
        let x = (dial["min"][0].as_f64().unwrap() + dial["max"][0].as_f64().unwrap()) * 0.5;
        let y = (dial["min"][1].as_f64().unwrap() + dial["max"][1].as_f64().unwrap()) * 0.5;
        let left_before = channel_left(&before_pan, 3);
        let down =
            app.command(json!({"command":"pointer","phase":"down","button":"middle","x":x,"y":y}));
        unchanged(&down);
        assert_eq!(channel_left(&down, 3), left_before);
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
        let left_after = channel_left(&panned, 3);
        assert!(
            left_after < left_before - 60.0,
            "Middle drag over the pan control must scroll too"
        );
        assert_eq!(left_after, channel_left(&up, 3));

        // A regular primary fader edit still changes the level and undoes once.
        app.command(json!({"command":"viewport","width":2400,"height":760}));
        let reset = reveal_control(&mut app, fader, 2);
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
