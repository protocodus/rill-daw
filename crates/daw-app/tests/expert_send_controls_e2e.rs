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

/// Locate the complete painted 270-degree ring, independently of diagnostics.
/// Its progress arc, relative drag target and exact readout remain distinct.
fn send_dial(state: &Value) -> (f64, f64) {
    let (level_x, level_y) = center(label(state, "Level"));
    state["painted_paths"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|path| {
            let points = path["points"].as_array().unwrap();
            if path["closed"] != false || points.len() < 32 {
                return None;
            }
            let minimum = points.iter().fold([f64::INFINITY; 2], |minimum, point| {
                [
                    minimum[0].min(point[0].as_f64().unwrap()),
                    minimum[1].min(point[1].as_f64().unwrap()),
                ]
            });
            let right = points
                .iter()
                .map(|point| point[0].as_f64().unwrap())
                .fold(f64::NEG_INFINITY, f64::max);
            let radius = (right - minimum[0]) * 0.5;
            let x = minimum[0] + radius;
            let y = minimum[1] + radius;
            if !(23.0..=26.0).contains(&radius)
                || x >= level_x
                || level_x - x > 130.0
                || (y - level_y).abs() > 80.0
            {
                return None;
            }
            let endpoint = radius * std::f64::consts::FRAC_1_SQRT_2;
            let first = points.first().unwrap();
            let last = points.last().unwrap();
            if (first[0].as_f64().unwrap() - (x - endpoint)).abs() > 0.6
                || (first[1].as_f64().unwrap() - (y + endpoint)).abs() > 0.6
                || (last[0].as_f64().unwrap() - (x + endpoint)).abs() > 0.6
                || (last[1].as_f64().unwrap() - (y + endpoint)).abs() > 0.6
            {
                return None;
            }
            Some((x, y))
        })
        .unwrap_or_else(|| panic!("Missing visible send ring: {}", state["visible_labels"]))
}

fn assert_send_row(state: &Value) {
    let (x, y) = send_dial(state);
    let first = center(label(state, "Level")).1;
    let last = ["Pre", "Post", "Automation", "Automated", "Overridden"]
        .iter()
        .filter(|text| visible(state, text))
        .map(|text| center(label(state, text)).1)
        .fold(first, f64::max);
    assert!(
        (y - (first + last) * 0.5).abs() < 0.75,
        "The ring must center on the measured detail rows: y={y}, first={first}, last={last}"
    );
    for caption in state["label_rects"].as_array().unwrap() {
        let left = caption["min"][0].as_f64().unwrap();
        let top = caption["min"][1].as_f64().unwrap();
        let right = caption["max"][0].as_f64().unwrap();
        let bottom = caption["max"][1].as_f64().unwrap();
        assert!(
            right <= x - 32.0 || left >= x + 32.0 || bottom <= y - 32.0 || top >= y + 32.0,
            "The small send ring must be text-free: {caption}"
        );
    }
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
        assert!(!visible(&baseline, "1× unity"));
        let painted = app.command(json!({"command":"inspect", "paint":true}));
        assert_send_row(&painted);
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
fn send_ring_stays_centered_and_tab_reaches_its_adjacent_exact_value() {
    for (dark, density) in [(false, 1.0), (true, 1.25), (true, 2.0)] {
        for automated in [false, true] {
            let mut app = Harness::new();
            fixture(&mut app, 0.25, automated);
            app.command(json!({"command":"system_theme", "dark":dark}));
            app.command(json!({"command":"viewport", "width":800, "height":600,
                "pixels_per_point":density}));
            let original = routing(&mut app);
            for (width, height) in [(800, 600), (1100, 850)] {
                app.command(json!({"command":"viewport", "width":width, "height":height,
                    "pixels_per_point":density}));
                let state = app.command(json!({"command":"inspect", "paint":true}));
                unchanged(&state, &original);
                assert_send_row(&state);
                let status = if automated { "Automated" } else { "Automation" };
                assert!(
                    (center(label(&state, status)).1 - center(label(&state, "Pre")).1).abs() < 0.75,
                    "The supported popup width must retain both complete detail rows"
                );
                let (x, y) = send_dial(&state);
                unchanged(
                    &app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y})),
                    &original,
                );
                // Native focus follows the visual ring → adjacent exact value
                // order, before the second-row mode and automation actions.
                app.key("Tab");
                let opened = app.key("Enter");
                label(&opened, SEND_TITLE);
                label(&opened, "Apply value");
                unchanged(&opened, &original);
                let cancelled = app.click("Cancel");
                unchanged(&cancelled, &original);
            }
            app.quit();
        }
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
