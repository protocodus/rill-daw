//! Send precision and parameter feedback through the running application's UI.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use serde_json::{Value, json};

const SOURCE: &str = "Expert source";
const SEND_TITLE: &str = "Expert source → Room · Send level";

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text:?}: {}", state["visible_labels"]))
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn gain(state: &Value, source: u64, bus: u64) -> f64 {
    track(state, source)["sends"]
        .as_array()
        .unwrap()
        .iter()
        .find(|send| send["target_bus"] == bus)
        .unwrap()["gain"]
        .as_f64()
        .unwrap()
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(
            state[field], before[field],
            "Viewing/drafting changed {field}"
        );
    }
}

fn one_edit(state: &Value, before: &Value) {
    assert_eq!(
        state["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn fixture(app: &mut Harness, stored: f32, automated: bool) -> (u64, u64, Option<u64>) {
    let mut project = Project::new();
    let source = project.add_audio_track(SOURCE);
    project.track_mut(source).unwrap().pan = 0.14;
    let bus = project.add_bus("Room");
    project.add_send(source, bus, stored, false).unwrap();
    let lane = automated.then(|| {
        let lane = project
            .add_automation_track(
                "Send movement",
                AutomationTarget {
                    track: source,
                    parameter: AutomationParameter::SendGain { bus },
                },
            )
            .unwrap();
        project.automation_lane_mut(lane).unwrap().points = vec![
            AutomationPoint::new(0.0, 0.25 / 16.0),
            AutomationPoint::new(8.0, 0.75 / 16.0),
        ];
        lane
    });
    let path = app.directory.path().join("expert-send-controls.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    (source, bus, lane)
}

fn routing(app: &mut Harness) -> Value {
    app.click("Mixer");
    app.click(SOURCE);
    app.click("Routing")
}

/// Locate the actual painted body, independently of a diagnostic that could
/// describe a knob while the application still renders a straight rail.
fn send_dial(state: &Value) -> (f64, f64) {
    let (level_x, level_y) = center(label(state, "Level"));
    let circle = state["painted_circles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|circle| {
            let radius = circle["radius"].as_f64().unwrap();
            let x = circle["center"][0].as_f64().unwrap();
            let y = circle["center"][1].as_f64().unwrap();
            (15.0..=25.0).contains(&radius)
                && (x - level_x).abs() < 130.0
                && (y - level_y).abs() < 65.0
        })
        .max_by(|a, b| {
            a["radius"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["radius"].as_f64().unwrap())
        })
        .unwrap_or_else(|| {
            panic!(
                "Missing visible rotary send control: {}",
                state["visible_labels"]
            )
        });
    (
        circle["center"][0].as_f64().unwrap(),
        circle["center"][1].as_f64().unwrap(),
    )
}

fn send_readout(state: &Value) -> &Value {
    let (level_x, level_y) = center(label(state, "Level"));
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            let (x, y) = center(row);
            row["text"].as_str().is_some_and(|text| {
                text.ends_with('×') && text.trim_end_matches('×').trim().parse::<f64>().is_ok()
            }) && x > level_x
                && x - level_x < 150.0
                && (y - level_y).abs() < 4.0
        })
        .min_by(|a, b| {
            let score = |row: &Value| {
                let (x, y) = center(row);
                (x - level_x).abs() + (y - level_y).abs()
            };
            score(a).total_cmp(&score(b))
        })
        .unwrap_or_else(|| panic!("Missing send readout: {}", state["visible_labels"]))
}

fn shown_gain(state: &Value) -> f64 {
    send_readout(state)["text"]
        .as_str()
        .unwrap()
        .trim_end_matches('×')
        .trim()
        .parse()
        .unwrap()
}

fn open_send(app: &mut Harness) -> Value {
    let before = app.inspect();
    let (x, y) = center(send_readout(&before));
    let opened = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    label(&opened, SEND_TITLE);
    label(&opened, "Apply value");
    label(&opened, "Cancel");
    assert!(visible(&opened, "Range: 0 to 16 ×"));
    unchanged(&opened, &before);
    opened
}

fn mouse_draft(app: &mut Harness, baseline: &Value, digits: &str) -> Value {
    let mut state = app.inspect();
    if !visible(&state, "Clear") {
        state = app.click("123");
        unchanged(&state, baseline);
    }
    unchanged(&app.click("Clear"), baseline);
    for digit in digits.chars() {
        state = app.click(&digit.to_string());
        unchanged(&state, baseline);
    }
    state
}

#[test]
fn send_small_rotary_adjustments_remain_below_unity_and_the_readout_tracks_live_gain() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let (source, bus, _) = fixture(&mut app, 0.25, false);
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":850,
            "pixels_per_point":density}));
        let baseline = routing(&mut app);
        label(&baseline, "1× unity");
        let painted = app.command(json!({"command":"inspect", "paint":true}));
        let (x, y) = send_dial(&painted);
        unchanged(
            &app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y})),
            &baseline,
        );
        for pixels in [4.0, 5.0] {
            // Cross egui's normal six-point drag threshold first. Then test
            // the small adjustment itself while the dial owns the gesture.
            app.command(json!({"command":"pointer", "phase":"down", "x":x, "y":y}));
            let owned = app.command(json!({"command":"pointer", "phase":"move",
                "x":x, "y":y-8.0}));
            let held_gain = gain(&owned, source, bus);
            app.command(json!({"command":"pointer", "phase":"move",
                "x":x, "y":y-8.0-pixels}));
            let changed = app.command(json!({"command":"pointer", "phase":"up",
                "x":x, "y":y-8.0-pixels}));
            let actual = gain(&changed, source, bus);
            assert!(
                actual > held_gain && actual < 0.5 && actual - held_gain < 0.1,
                "A {pixels}-point send adjustment must be useful below unity: {actual}"
            );
            assert!(
                (shown_gain(&changed) - actual).abs() < 0.000_01,
                "The visible readout must agree with the live gain"
            );
            one_edit(&changed, &baseline);
            let opened = open_send(&mut app);
            unchanged(&app.click("Apply value"), &opened);
            assert_eq!(undo(&mut app)["project"], baseline["project"]);
            assert_eq!(shown_gain(&app.inspect()), 0.25);
        }
        app.quit();
    }
}

#[test]
fn send_exact_button_formats_without_rounding_and_mouse_keypad_preserves_precision() {
    let mut app = Harness::new();
    let (source, bus, _) = fixture(&mut app, 0.173_456_79, false);
    app.command(json!({"command":"viewport", "width":1100, "height":850}));
    let baseline = routing(&mut app);
    let original = gain(&baseline, source, bus);
    let opened = open_send(&mut app);
    assert!(
        !visible(&opened, &original.to_string()),
        "The exact editor should not expose f32 conversion noise"
    );
    unchanged(&app.click("Apply value"), &baseline);
    assert_eq!(
        gain(&app.inspect(), source, bus).to_bits(),
        original.to_bits()
    );

    open_send(&mut app);
    mouse_draft(&mut app, &baseline, "0.375123");
    unchanged(&app.click("Cancel"), &baseline);
    open_send(&mut app);
    mouse_draft(&mut app, &baseline, "0.375123");
    let applied = app.click("Apply value");
    assert_eq!(gain(&applied, source, bus) as f32, 0.375_123_f32);
    one_edit(&applied, &baseline);
    assert_eq!(undo(&mut app)["project"], baseline["project"]);

    // The perceptual control must retain the full stored/editable gain domain.
    for value in ["0", "1", "16"] {
        open_send(&mut app);
        let before_draft = app.inspect();
        mouse_draft(&mut app, &before_draft, value);
        let applied = app.click("Apply value");
        assert_eq!(gain(&applied, source, bus), value.parse::<f64>().unwrap());
        assert_eq!(undo(&mut app)["project"], baseline["project"]);
    }
    app.quit();
}

#[test]
fn pan_exact_editor_hides_float_noise_and_unchanged_apply_retains_history() {
    let mut app = Harness::new();
    let (source, _, _) = fixture(&mut app, 0.25, false);
    app.command(json!({"command":"viewport", "width":1100, "height":850}));
    let baseline = app.click("Mixer");
    let channel = baseline["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == source)
        .unwrap();
    let (x, y) = center(&channel["pan"]);
    let opened = app.command(json!({"command":"pointer", "phase":"click",
        "button":"secondary", "x":x, "y":y}));
    label(&opened, "Expert source · Pan");
    label(&opened, "14");
    assert!(!visible(&opened, "14.00000059604645"));
    unchanged(&app.click("Apply value"), &baseline);
    app.quit();
}

#[test]
fn maximum_channel_gain_accepts_untouched_formatted_apply_without_mutation() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Maximum gain");
    project.track_mut(source).unwrap().gain = 1.5;
    let path = app.directory.path().join("maximum-gain-entry.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let before = app.click("Mixer");
    let channel = before["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == source)
        .unwrap();
    let (x, y) = center(&channel["gain"]);
    let opened = app.command(json!({"command":"pointer", "phase":"click",
        "button":"secondary", "x":x, "y":y}));
    label(&opened, "Maximum gain · Volume");
    label(&opened, "3.52183");
    assert!(!visible(&opened, "The value must be inside the range."));
    let applied = app.click("Apply value");
    unchanged(&applied, &before);
    assert!(
        !visible(&applied, "Maximum gain · Volume"),
        "valid untouched Apply closes the editor"
    );
    assert_eq!(track(&applied, source)["gain"], 1.5);
    app.quit();
}

#[test]
fn automated_send_exact_entry_and_small_drag_override_without_rewriting_the_curve() {
    let mut app = Harness::new();
    let (source, bus, lane) = fixture(&mut app, 0.173_456_79, true);
    let lane = lane.unwrap();
    app.command(json!({"command":"viewport", "width":1100, "height":850}));
    let baseline = routing(&mut app);
    let curve = track(&baseline, lane)["automation"]["points"].clone();
    assert_eq!(shown_gain(&baseline), 0.25);
    let painted = app.command(json!({"command":"inspect", "paint":true}));
    let (x, y) = send_dial(&painted);
    let dragged = app.command(json!({"command":"pointer", "phase":"drag",
        "x":x, "y":y, "to_x":x, "to_y":y-12.0}));
    let actual = gain(&dragged, source, bus);
    assert!(actual > 0.25 && actual < 0.5);
    let normalized = track(&dragged, lane)["automation"]["override_value"]
        .as_f64()
        .unwrap();
    assert!(
        (normalized - actual / 16.0).abs() < 1e-7,
        "The knob taper must not change the automation model"
    );
    assert_eq!(track(&dragged, lane)["automation"]["points"], curve);
    one_edit(&dragged, &baseline);
    assert_eq!(undo(&mut app)["project"], baseline["project"]);

    let restored = app.inspect();
    let opened = open_send(&mut app);
    label(&opened, "Override automation");
    unchanged(&app.click("Apply value"), &restored);
    open_send(&mut app);
    mouse_draft(&mut app, &restored, "0.375123");
    let applied = app.click("Apply value");
    assert_eq!(gain(&applied, source, bus) as f32, 0.375_123_f32);
    assert_eq!(track(&applied, lane)["automation"]["points"], curve);
    assert!(
        (track(&applied, lane)["automation"]["override_value"]
            .as_f64()
            .unwrap()
            - f64::from(0.375_123_f32) / 16.0)
            .abs()
            < 1e-7
    );
    one_edit(&applied, &baseline);
    open_send(&mut app);
    let resumed = app.click("Resume automation");
    assert!(track(&resumed, lane)["automation"]["override_value"].is_null());
    assert_eq!(track(&resumed, lane)["automation"]["points"], curve);
    assert_eq!(shown_gain(&resumed), 0.25);
    assert_eq!(undo(&mut app)["project"], applied["project"]);
    assert_eq!(undo(&mut app)["project"], baseline["project"]);
    app.quit();
}
