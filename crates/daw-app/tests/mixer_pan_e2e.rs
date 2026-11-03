//! Mixer Pan is a horizontal rail in the fader's skin, with center and end
//! detents, its value beside it, exact entry and automation override.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use serde_json::{Value, json};

fn point(value: &Value) -> [f64; 2] {
    [value[0].as_f64().unwrap(), value[1].as_f64().unwrap()]
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn painted(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn unchanged(actual: &Value, before: &Value) {
    for field in ["project", "history", "modified", "selected_track"] {
        assert_eq!(actual[field], before[field], "Viewing Pan changed {field}");
    }
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

/// The rail is horizontal, carries one center detent mark and no generic
/// divisions; its value caption sits in the row, right of the rail.
fn assert_pan_rail(state: &Value, id: u64, expected: f64) {
    let channel = channel(state, id);
    let scale = &channel["pan_scale"];
    assert!(scale.is_object(), "Missing pan rail geometry");
    assert_eq!(scale["range"], json!([-1.0, 1.0]));
    assert_eq!(scale["divisions"], 0);
    let ticks: Vec<f64> = scale["ticks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tick| tick["fraction"].as_f64().unwrap())
        .collect();
    assert_eq!(ticks, [0.5]);
    near(scale["value"].as_f64().unwrap(), expected, 2e-6);
    let minimum = point(&scale["min_position"]);
    let maximum = point(&scale["max_position"]);
    assert_eq!(minimum[1], maximum[1], "pan travels horizontally");
    let handle = point(&scale["handle_center"]);
    let fraction = (expected.clamp(-1.0, 1.0) + 1.0) * 0.5;
    near(
        handle[0],
        minimum[0] + (maximum[0] - minimum[0]) * fraction,
        0.01,
    );
    let row = &channel["pan"];
    let caption = daw_ui::mixer_pan_caption(expected as f32);
    let rail_right = scale["rail"]["max"][0].as_f64().unwrap();
    assert!(
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label["text"] == caption.as_str()
                && label["min"][0].as_f64().unwrap() >= rail_right
                && label["max"][0].as_f64().unwrap() <= row["max"][0].as_f64().unwrap() + 0.5
                && label["min"][1].as_f64().unwrap() >= row["min"][1].as_f64().unwrap() - 0.5
                && label["max"][1].as_f64().unwrap() <= row["max"][1].as_f64().unwrap() + 0.5),
        "Missing {caption:?} beside the pan rail: {row}"
    );
    // Mute and Solo share one row directly below the pan rail.
    assert!(channel["mute_solo"]["min"][1].as_f64().unwrap() >= row["max"][1].as_f64().unwrap());
}

/// Drag the cap to a travel fraction (0 = hard left, 1 = hard right).
fn drag_to(app: &mut Harness, id: u64, fraction: f64) -> Value {
    let state = app.inspect();
    let scale = &channel(&state, id)["pan_scale"];
    let [x, y] = point(&scale["handle_center"]);
    let minimum = point(&scale["min_position"]);
    let maximum = point(&scale["max_position"]);
    let to_x = minimum[0] + (maximum[0] - minimum[0]) * fraction;
    app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y,
        "to_x":to_x, "to_y":y}))
}

fn open_exact(app: &mut Harness, id: u64) -> Value {
    let state = app.inspect();
    let [x, y] = point(&channel(&state, id)["pan_scale"]["handle_center"]);
    let opened = app.command(json!({"command":"pointer", "phase":"click",
        "button":"secondary", "x":x, "y":y}));
    assert!(visible(&opened, "Apply value") && visible(&opened, "Cancel"));
    unchanged(&opened, &state);
    opened
}

#[test]
fn mixer_pan_rail_drags_exact_entry_and_undo() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let id = project.add_audio_track("Pan rail");
        project.track_mut(id).unwrap().gain = 0.6;
        let path = app.directory.path().join("mixer-pan.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":900,
            "pixels_per_point":density}));
        let baseline = app.click("Mixer");
        for width in [800, 1100] {
            app.command(json!({"command":"viewport", "width":width, "height":900,
                "pixels_per_point":density}));
            let state = painted(&mut app);
            assert_pan_rail(&state, id, 0.0);
            unchanged(&state, &baseline);
        }
        for (fraction, expected) in [(0.0, -1.0), (0.25, -0.5), (0.75, 0.5), (1.0, 1.0)] {
            let edited = drag_to(&mut app, id, fraction);
            near(track(&edited, id)["pan"].as_f64().unwrap(), expected, 0.02);
            assert_eq!(
                edited["history"]["undo"].as_array().unwrap().len(),
                baseline["history"]["undo"].as_array().unwrap().len() + 1,
                "one drag is one Undo operation"
            );
            let pan = track(&edited, id)["pan"].as_f64().unwrap();
            assert_pan_rail(&painted(&mut app), id, pan);
            assert_eq!(undo(&mut app)["project"], baseline["project"]);
        }
        // The center detent: a drag ending just beside center lands on C.
        drag_to(&mut app, id, 0.25);
        let centered = drag_to(&mut app, id, 0.51);
        assert_eq!(track(&centered, id)["pan"].as_f64().unwrap(), 0.0);
        assert_pan_rail(&painted(&mut app), id, 0.0);
        undo(&mut app);
        assert_eq!(undo(&mut app)["project"], baseline["project"]);
        let opened = open_exact(&mut app, id);
        assert!(
            opened["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .any(|text| text.starts_with("Range:")
                    && text.contains("-100")
                    && text.contains('%'))
        );
        let draft = app.command(json!({"command":"text", "text":"23.4567"}));
        unchanged(&draft, &opened);
        let applied = app.click("Apply value");
        near(track(&applied, id)["pan"].as_f64().unwrap(), 0.234567, 2e-6);
        assert_pan_rail(&painted(&mut app), id, 0.234567);
        assert_eq!(undo(&mut app)["project"], baseline["project"]);
        app.quit();
    }
}

#[test]
fn mixer_pan_drag_and_exact_entry_override_automation_without_rewriting_the_curve() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let id = project.add_audio_track("Automated pan");
    project.track_mut(id).unwrap().pan = 0.173456;
    let target = AutomationTarget {
        track: id,
        parameter: AutomationParameter::TrackPan,
    };
    let lane = project
        .add_automation_track("Pan movement", target)
        .unwrap();
    project.automation_lane_mut(lane).unwrap().points = vec![
        AutomationPoint::new(0.0, 0.25),
        AutomationPoint::new(8.0, 0.75),
    ];
    let path = app.directory.path().join("mixer-pan-automation.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1100, "height":900}));
    let baseline = app.click("Mixer");
    let curve = track(&baseline, lane)["automation"]["points"].clone();
    // The rail shows the automated value, not the stored one.
    near(
        channel(&baseline, id)["pan_scale"]["value"]
            .as_f64()
            .unwrap(),
        -0.5,
        2e-6,
    );
    let dragged = drag_to(&mut app, id, 0.75);
    let pan = track(&dragged, id)["pan"].as_f64().unwrap();
    near(pan, 0.5, 0.02);
    near(
        track(&dragged, lane)["automation"]["override_value"]
            .as_f64()
            .unwrap(),
        (pan + 1.0) * 0.5,
        2e-6,
    );
    assert_eq!(track(&dragged, lane)["automation"]["points"], curve);
    assert_eq!(channel(&dragged, id)["automation_state"], "Overridden");
    assert_eq!(undo(&mut app)["project"], baseline["project"]);

    let opened = open_exact(&mut app, id);
    assert!(visible(&opened, "Automated") && visible(&opened, "Override automation"));
    app.command(json!({"command":"text", "text":"33.333"}));
    let applied = app.click("Apply value");
    near(track(&applied, id)["pan"].as_f64().unwrap(), 0.33333, 2e-6);
    assert_eq!(track(&applied, lane)["automation"]["points"], curve);
    open_exact(&mut app, id);
    let resumed = app.click("Resume automation");
    assert!(track(&resumed, lane)["automation"]["override_value"].is_null());
    near(
        channel(&resumed, id)["pan_scale"]["value"]
            .as_f64()
            .unwrap(),
        -0.5,
        2e-6,
    );
    assert_eq!(undo(&mut app)["project"], applied["project"]);
    assert_eq!(undo(&mut app)["project"], baseline["project"]);
    app.quit();
}
