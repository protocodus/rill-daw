//! Every native channel slider can create and navigate its exact automation lane.
mod common;
use common::Harness;
use daw_core::{AutomationParameter, AutomationTarget, Project};
use serde_json::{Value, json};

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn right_click(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}))
}
fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}
fn control(state: &Value, view: &str, track: u64, name: &str) -> Value {
    state[view]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == track)
        .unwrap()[name]
        .clone()
}
fn automation(state: &Value, target: AutomationTarget) -> (&Value, u64) {
    let target = serde_json::to_value(target).unwrap();
    let lane = state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["automation"]["target"] == target)
        .unwrap_or_else(|| panic!("No lane for {target}: {}", state["project"]));
    (&lane["automation"], lane["id"].as_u64().unwrap())
}
fn assert_created(state: &Value, target: AutomationTarget, value: f64) -> u64 {
    let (lane, id) = automation(state, target);
    assert_eq!(state["selected_track"], id);
    assert_eq!(state["popups"]["automation"], true);
    assert_eq!(state["popups"]["properties"], false);
    assert_eq!(state["popups"]["routing"], false);
    for point in lane["points"].as_array().unwrap() {
        assert!((point["value"].as_f64().unwrap() - value).abs() < 1e-6);
    }
    assert!(lane["override_value"].is_null());
    id
}

#[test]
fn native_slider_contexts_create_exact_targets_and_preserve_mouse_history_and_resume() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_audio_track("Drums");
        let other = project.add_midi_track("Keys");
        let bus = project.add_bus("Room");
        project.track_mut(source).unwrap().gain = 0.3;
        project.track_mut(source).unwrap().pan = -0.4;
        project.track_mut(other).unwrap().gain = 0.75;
        // A legal send above unity must remain intact while viewing its full
        // 0..16 range and initializing automation from the actual value.
        project.add_send(source, bus, 8.0, false).unwrap();
        let path = app.directory.path().join("slider-automation.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(json!({"command":"viewport","width":1200,"height":900}));
        app.click("Keys");
        let original = app.inspect()["project"].clone();
        let gain_target = AutomationTarget {
            track: source,
            parameter: AutomationParameter::TrackGain,
        };
        let state = app.inspect();
        right_click(&mut app, &control(&state, "timeline", source, "gain"));
        let created = app.click("Create automation");
        let gain_lane = assert_created(&created, gain_target, 0.2);
        let curve = automation(&created, gain_target).0["points"].clone();
        assert_eq!(menu(&mut app, "Edit", "Undo")["project"], original);
        let redone = menu(&mut app, "Edit", "Redo");
        assert_eq!(automation(&redone, gain_target).1, gain_lane);

        app.click("Arranger");
        let state = app.inspect();
        let rect = control(&state, "timeline", source, "gain");
        let (x, y) = center(&rect);
        let to_x = rect["max"][0].as_f64().unwrap() - 8.0;
        let changed = app
            .command(json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":to_x,"to_y":y}));
        assert!(automation(&changed, gain_target).0["override_value"].is_number());
        assert_eq!(automation(&changed, gain_target).0["points"], curve);
        right_click(&mut app, &control(&changed, "timeline", source, "gain"));
        let resumed = app.click("Resume automation");
        assert!(automation(&resumed, gain_target).0["override_value"].is_null());
        assert_eq!(automation(&resumed, gain_target).0["points"], curve);
        let history = resumed["history"].clone();
        right_click(&mut app, &control(&resumed, "timeline", source, "gain"));
        let editing = app.click("Edit automation");
        assert_eq!(editing["selected_track"], gain_lane);
        assert_eq!(
            editing["history"], history,
            "Reopening a lane is navigation"
        );

        let mixer = app.click("Mixer");
        right_click(&mut app, &control(&mixer, "mixer", source, "pan"));
        let panned = app.click("Create automation");
        assert_created(
            &panned,
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::TrackPan,
            },
            0.3,
        );

        app.click("Mixer");
        app.click("Keys");
        // Properties exposes a visible automation menu before any lane exists.
        let properties = app.inspect();
        assert_eq!(properties["popups"]["properties"], true);
        let created = menu(&mut app, "Automation", "Create automation");
        assert_created(
            &created,
            AutomationTarget {
                track: other,
                parameter: AutomationParameter::TrackGain,
            },
            0.5,
        );

        app.click("Mixer");
        app.click("Drums");
        app.click("Routing");
        let sent = menu(&mut app, "Automation", "Create automation");
        assert_created(
            &sent,
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::SendGain { bus },
            },
            0.5,
        );

        let mixer = app.click("Mixer");
        right_click(&mut app, &control(&mixer, "mixer", bus, "pan"));
        let panned = app.click("Create automation");
        assert_created(
            &panned,
            AutomationTarget {
                track: bus,
                parameter: AutomationParameter::TrackPan,
            },
            0.5,
        );
        app.quit();
    }
}
