//! Mixer Pan is a calibrated rotary control with relative mouse adjustment.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use serde_json::{Value, json};

fn point(value: &Value) -> [f64; 2] {
    [value[0].as_f64().unwrap(), value[1].as_f64().unwrap()]
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
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

fn caption_in_row(state: &Value, row: &Value, text: &str) {
    let min = point(&row["min"]);
    let max = point(&row["max"]);
    assert!(
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| {
                if label["text"] != text {
                    return false;
                }
                let a = point(&label["min"]);
                let b = point(&label["max"]);
                a[0] >= min[0] - 0.5
                    && a[1] >= min[1] - 0.5
                    && b[0] <= max[0] + 0.5
                    && b[1] <= max[1] + 0.5
            }),
        "Missing bounded {text:?} caption in {row}"
    );
}

fn radial(center: [f64; 2], radius: f64, fraction: f64) -> [f64; 2] {
    let angle = std::f64::consts::PI * (0.75 + 1.5 * fraction);
    [
        center[0] + radius * angle.cos(),
        center[1] + radius * angle.sin(),
    ]
}

/// Check real painted paths and lines as well as diagnostics. The knob's
/// endpoints form a 270-degree arc, not a straight slider travel range.
fn assert_knob_paint(state: &Value, id: u64, expected: f64) {
    let channel = channel(state, id);
    assert!(
        channel["pan_scale"].is_null(),
        "Pan must not masquerade as a slider"
    );
    let knob = &channel["pan_knob"];
    assert!(knob.is_object(), "Missing rotary geometry");
    assert_eq!(knob["range"], json!([-1.0, 1.0]));
    assert_eq!(knob["divisions"], 0);
    assert!(
        knob["number"].is_null(),
        "The compact row uses its shared exact-value context menu"
    );
    near(knob["value"].as_f64().unwrap(), expected);
    let center = point(&knob["center"]);
    let radius = knob["radius"].as_f64().unwrap();
    let dial_min = point(&knob["dial"]["min"]);
    let dial_max = point(&knob["dial"]["max"]);
    let row_min = point(&channel["pan"]["min"]);
    let row_max = point(&channel["pan"]["max"]);
    near(dial_max[0] - dial_min[0], 48.0);
    near(dial_max[1] - dial_min[1], 48.0);
    near(
        row_max[1] - row_min[1],
        f64::from(daw_ui::Knob::compact_height(48.0).max(daw_ui::tokens::PARAMETER_ROW_HEIGHT)),
    );
    assert!(dial_min[0] >= row_min[0] && dial_min[1] >= row_min[1]);
    assert!(dial_max[0] <= row_max[0] && dial_max[1] <= row_max[1]);
    assert!((10.0..=18.0).contains(&radius));
    let minimum = radial(center, radius, 0.0);
    let maximum = radial(center, radius, 1.0);
    let index = radial(center, radius, (expected + 1.0) * 0.5);
    assert!(distance(point(&knob["min_position"]), minimum) < 0.6);
    assert!(distance(point(&knob["max_position"]), maximum) < 0.6);
    assert!(distance(point(&knob["index"]), index) < 0.6);

    assert!(
        state["painted_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| {
                let points = path["points"].as_array().unwrap();
                path["closed"] == false
                    && points.len() >= 32
                    && distance(point(points.first().unwrap()), minimum) < 0.6
                    && distance(point(points.last().unwrap()), maximum) < 0.6
            }),
        "The complete rotary scale must be painted between its endpoints"
    );
    assert!(
        state["painted_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| {
                let points = path["points"].as_array().unwrap();
                path["closed"] == false
                    && !points.is_empty()
                    && (path["stroke_width"].as_f64().unwrap() - 2.5).abs() < 0.1
                    && distance(point(points.first().unwrap()), minimum) < 0.6
                    && distance(point(points.last().unwrap()), index) < 0.6
            }),
        "The ring's active arc must end at the effective pan value"
    );
    assert!(
        !state["painted_circles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|circle| distance(point(&circle["center"]), center) < 0.6),
        "The compact ring has no dial face or center cap"
    );
    for label in state["label_rects"].as_array().unwrap() {
        let a = point(&label["min"]);
        let b = point(&label["max"]);
        assert!(
            b[0] <= dial_min[0]
                || a[0] >= dial_max[0]
                || b[1] <= dial_min[1]
                || a[1] >= dial_max[1],
            "The compact dial must be text-free: {label}"
        );
    }
    caption_in_row(
        state,
        &channel["pan"],
        &format!("Pan {}", daw_ui::channel_pan_label(expected as f32)),
    );
}

fn drag(app: &mut Harness, id: u64, dx: f64, dy: f64, fine: bool) -> Value {
    let state = app.inspect();
    let [x, y] = point(&channel(&state, id)["pan_knob"]["center"]);
    app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y,
        "to_x":x+dx, "to_y":y+dy, "shift":fine}))
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
fn mixer_pan_knob_arc_and_relative_drags_preserve_click_values_and_undo() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let id = project.add_audio_track("Rotary pan");
        project.track_mut(id).unwrap().gain = 0.6;
        let path = app.directory.path().join("mixer-pan-knob.rill");
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
            assert_knob_paint(&state, id, 0.0);
            unchanged(&state, &baseline);
        }
        // Clicking near an endpoint is focus, not absolute angle/value entry.
        let state = app.inspect();
        let dial = &channel(&state, id)["pan_knob"]["dial"];
        let x = dial["max"][0].as_f64().unwrap() - 2.0;
        let y = dial["min"][1].as_f64().unwrap() + 2.0;
        let down = app.command(json!({"command":"pointer", "phase":"down", "x":x, "y":y}));
        unchanged(&down, &baseline);
        let clicked = app.command(json!({"command":"pointer", "phase":"up", "x":x, "y":y}));
        unchanged(&clicked, &baseline);

        for (dx, dy, fine, expected) in [
            (0.0, 100.0, false, -1.0),
            (0.0, -50.0, false, 0.5),
            (0.0, -100.0, false, 1.0),
            (40.0, 0.0, false, 0.4),
            (0.0, -40.0, true, 0.04),
        ] {
            let edited = drag(&mut app, id, dx, dy, fine);
            near(track(&edited, id)["pan"].as_f64().unwrap(), expected);
            assert_eq!(
                edited["history"]["undo"].as_array().unwrap().len(),
                baseline["history"]["undo"].as_array().unwrap().len() + 1
            );
            let shown = painted(&mut app);
            assert_knob_paint(&shown, id, expected);
            unchanged(&shown, &edited);
            assert_eq!(undo(&mut app)["project"], baseline["project"]);
            assert_knob_paint(&painted(&mut app), id, 0.0);
        }
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
        near(track(&applied, id)["pan"].as_f64().unwrap(), 0.234567);
        assert_eq!(undo(&mut app)["project"], baseline["project"]);
        app.quit();
    }
}

#[test]
fn mixer_pan_knob_drag_and_exact_entry_override_automation_without_rewriting_the_curve() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let id = project.add_audio_track("Automated rotary pan");
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
    let path = app.directory.path().join("mixer-pan-knob-automation.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1100, "height":900}));
    let baseline = app.click("Mixer");
    let curve = track(&baseline, lane)["automation"]["points"].clone();
    near(
        channel(&baseline, id)["pan_knob"]["value"]
            .as_f64()
            .unwrap(),
        -0.5,
    );
    near(track(&baseline, id)["pan"].as_f64().unwrap(), 0.173456);

    let dragged = drag(&mut app, id, 0.0, -30.0, false);
    near(track(&dragged, id)["pan"].as_f64().unwrap(), -0.2);
    near(
        track(&dragged, lane)["automation"]["override_value"]
            .as_f64()
            .unwrap(),
        0.4,
    );
    assert_eq!(track(&dragged, lane)["automation"]["points"], curve);
    assert_eq!(channel(&dragged, id)["automation_state"], "Overridden");
    assert_eq!(undo(&mut app)["project"], baseline["project"]);

    let opened = open_exact(&mut app, id);
    assert!(visible(&opened, "Automated") && visible(&opened, "Override automation"));
    let draft = app.command(json!({"command":"text", "text":"33.333"}));
    unchanged(&draft, &opened);
    let applied = app.click("Apply value");
    near(track(&applied, id)["pan"].as_f64().unwrap(), 0.33333);
    near(
        track(&applied, lane)["automation"]["override_value"]
            .as_f64()
            .unwrap(),
        0.666665,
    );
    assert_eq!(track(&applied, lane)["automation"]["points"], curve);
    assert_eq!(
        applied["history"]["undo"].as_array().unwrap().len(),
        baseline["history"]["undo"].as_array().unwrap().len() + 1
    );
    open_exact(&mut app, id);
    let resumed = app.click("Resume automation");
    assert!(track(&resumed, lane)["automation"]["override_value"].is_null());
    assert_eq!(track(&resumed, lane)["automation"]["points"], curve);
    near(
        channel(&resumed, id)["pan_knob"]["value"].as_f64().unwrap(),
        -0.5,
    );
    assert_eq!(undo(&mut app)["project"], applied["project"]);
    assert_eq!(undo(&mut app)["project"], baseline["project"]);
    app.quit();
}

#[test]
fn mixer_pan_applies_the_owned_release_position_once_regardless_of_held_sampling() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let id = project.add_audio_track("Release position");
        let path = app.directory.path().join("pan-release.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":900,
            "pixels_per_point":density}));
        let baseline = app.click("Mixer");

        for (sampled, held_fine, released_fine, expected) in [
            (12.0, false, false, 0.6),
            (28.0, false, false, 0.6),
            (44.0, false, false, 0.6),
            (20.0, true, true, 0.06),
            (20.0, false, true, 0.24),
        ] {
            let state = app.inspect();
            let [x, y] = point(&channel(&state, id)["pan_knob"]["center"]);
            let dial_bottom = channel(&state, id)["pan_knob"]["dial"]["max"][1]
                .as_f64()
                .unwrap();
            assert!(y + 60.0 > dial_bottom, "release leaves the dial's hit area");
            let down = app.command(json!({"command":"pointer", "phase":"down",
                "x":x, "y":y, "shift":held_fine}));
            unchanged(&down, &state);
            // A downward drag decreases pan. Its last movement is delivered
            // together with release, without the endpoint frame used by `drag`.
            let held = app.command(json!({"command":"pointer", "phase":"move",
                "x":x, "y":y+sampled, "shift":held_fine}));
            near(
                track(&held, id)["pan"].as_f64().unwrap(),
                -sampled * if held_fine { 0.001 } else { 0.01 },
            );
            assert_eq!(held["history"], state["history"]);
            let released = app.command(json!({"command":"pointer", "phase":"up",
                "x":x, "y":y+60.0, "shift":released_fine}));
            near(track(&released, id)["pan"].as_f64().unwrap(), -expected);
            assert_eq!(
                released["history"]["undo"].as_array().unwrap().len(),
                baseline["history"]["undo"].as_array().unwrap().len() + 1,
                "held and released movement form one Undo operation"
            );
            assert_knob_paint(&painted(&mut app), id, -expected);
            let hovered = app.command(json!({"command":"pointer", "phase":"move",
                "x":x+10.0, "y":y+65.0}));
            unchanged(&hovered, &released);
            assert_eq!(undo(&mut app)["project"], baseline["project"]);
        }
        app.quit();
    }
}

#[test]
fn mixer_pan_release_overrides_the_effective_value_without_rewriting_automation() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let id = project.add_audio_track("Automated release position");
    project.track_mut(id).unwrap().pan = 0.173456;
    let lane = project
        .add_automation_track(
            "Pan movement",
            AutomationTarget {
                track: id,
                parameter: AutomationParameter::TrackPan,
            },
        )
        .unwrap();
    project.automation_lane_mut(lane).unwrap().points = vec![
        AutomationPoint::new(0.0, 0.25),
        AutomationPoint::new(8.0, 0.75),
    ];
    let path = app.directory.path().join("automated-pan-release.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1100, "height":900}));
    let baseline = app.click("Mixer");
    let curve = track(&baseline, lane)["automation"]["points"].clone();
    let [x, y] = point(&channel(&baseline, id)["pan_knob"]["center"]);
    near(
        channel(&baseline, id)["pan_knob"]["value"]
            .as_f64()
            .unwrap(),
        -0.5,
    );
    near(track(&baseline, id)["pan"].as_f64().unwrap(), 0.173456);
    app.command(json!({"command":"pointer", "phase":"down", "x":x, "y":y}));
    let held = app.command(json!({"command":"pointer", "phase":"move",
        "x":x+20.0, "y":y}));
    near(track(&held, id)["pan"].as_f64().unwrap(), -0.3);
    assert_eq!(held["history"], baseline["history"]);
    let released = app.command(json!({"command":"pointer", "phase":"up",
        "x":x+40.0, "y":y}));
    near(track(&released, id)["pan"].as_f64().unwrap(), -0.1);
    near(
        track(&released, lane)["automation"]["override_value"]
            .as_f64()
            .unwrap(),
        0.45,
    );
    assert_eq!(track(&released, lane)["automation"]["points"], curve);
    assert_eq!(channel(&released, id)["automation_state"], "Overridden");
    assert_eq!(
        released["history"]["undo"].as_array().unwrap().len(),
        baseline["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert_eq!(undo(&mut app)["project"], baseline["project"]);
    app.quit();
}
