//! Mixer Pan is a labelled bipolar knob in the width knob's row shape, with a
//! center detent, its value beside it, exact entry and automation override.
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

fn text_inside(state: &Value, text: &str, left: f64, right: f64, row: &Value) -> bool {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| {
            label["text"] == text
                && label["min"][0].as_f64().unwrap() >= left - 0.5
                && label["max"][0].as_f64().unwrap() <= right + 0.5
                && label["min"][1].as_f64().unwrap() >= row["min"][1].as_f64().unwrap() - 0.5
                && label["max"][1].as_f64().unwrap() <= row["max"][1].as_f64().unwrap() + 0.5
        })
}

/// "Pan" on the left of the row, the dial and its value caption at its right
/// end, directly above the shared Mute/Solo row.
fn assert_pan_knob(state: &Value, id: u64, expected: f64) {
    let channel = channel(state, id);
    let knob = &channel["pan_knob"];
    assert!(knob.is_object(), "Missing pan knob geometry");
    assert_eq!(knob["range"], json!([-1.0, 1.0]));
    near(knob["value"].as_f64().unwrap(), expected, 2e-6);
    let row = &channel["pan"];
    let dial = &knob["dial"];
    let (row_left, row_right) = (
        row["min"][0].as_f64().unwrap(),
        row["max"][0].as_f64().unwrap(),
    );
    let (dial_left, dial_right) = (
        dial["min"][0].as_f64().unwrap(),
        dial["max"][0].as_f64().unwrap(),
    );
    assert!(
        text_inside(state, "Pan", row_left, dial_left, row),
        "Missing the Pan name: {row}"
    );
    let caption = daw_ui::mixer_pan_caption(expected as f32);
    assert!(
        text_inside(state, &caption, dial_right, row_right, row),
        "Missing {caption:?} beside the pan knob: {row}"
    );
    // The pan and width dials stack on one axis.
    let width = &channel["stereo_width"];
    assert!(width["max"][0] == row["max"][0]);
    assert!(channel["mute_solo"]["min"][1].as_f64().unwrap() >= row["max"][1].as_f64().unwrap());
}

/// Drag the knob horizontally to a travel fraction (0 = hard left, 1 = hard
/// right): a knob covers its range in 200 points of drag.
fn drag_to(app: &mut Harness, id: u64, fraction: f64) -> Value {
    let state = app.inspect();
    let knob = &channel(&state, id)["pan_knob"];
    let [x, y] = point(&knob["center"]);
    let current = knob["value"].as_f64().unwrap();
    let to_x = x + (fraction * 2.0 - 1.0 - current) * 100.0;
    app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y,
        "to_x":to_x, "to_y":y}))
}

fn open_exact(app: &mut Harness, id: u64) -> Value {
    let state = app.inspect();
    let [x, y] = point(&channel(&state, id)["pan_knob"]["center"]);
    let opened = app.command(json!({"command":"pointer", "phase":"click",
        "button":"secondary", "x":x, "y":y}));
    assert!(visible(&opened, "Apply value") && visible(&opened, "Cancel"));
    unchanged(&opened, &state);
    opened
}

#[test]
fn mixer_pan_knob_drags_exact_entry_and_undo() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let id = project.add_audio_track("Pan knob");
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
            assert_pan_knob(&state, id, 0.0);
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
            assert_pan_knob(&painted(&mut app), id, pan);
            assert_eq!(undo(&mut app)["project"], baseline["project"]);
        }
        // The center detent: a drag ending just beside center lands on C.
        drag_to(&mut app, id, 0.25);
        let centered = drag_to(&mut app, id, 0.51);
        assert_eq!(track(&centered, id)["pan"].as_f64().unwrap(), 0.0);
        assert_pan_knob(&painted(&mut app), id, 0.0);
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
        assert_pan_knob(&painted(&mut app), id, 0.234567);
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
    // The knob shows the automated value, not the stored one.
    near(
        channel(&baseline, id)["pan_knob"]["value"]
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
        channel(&resumed, id)["pan_knob"]["value"].as_f64().unwrap(),
        -0.5,
        2e-6,
    );
    assert_eq!(undo(&mut app)["project"], applied["project"]);
    assert_eq!(undo(&mut app)["project"], baseline["project"]);
    app.quit();
}
