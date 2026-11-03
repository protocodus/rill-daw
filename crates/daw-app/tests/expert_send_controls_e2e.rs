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
    app.click("Routing");
    to_sends(app)
}

/// Sends follow the output: wheel the routing body to its end so the send
/// row is fully visible in short windows.
fn to_sends(app: &mut Harness) -> Value {
    let state = app.inspect();
    let window = &state["window_rects"][0];
    let x = (window["min"][0].as_f64().unwrap() + window["max"][0].as_f64().unwrap()) * 0.5;
    let y = window["max"][1].as_f64().unwrap() - 100.0;
    app.command(json!({"command":"scroll", "x":x, "y":y, "delta":-10000.0}));
    for _ in 0..12 {
        app.inspect();
    }
    app.inspect()
}

/// Locate the complete painted 270-degree ring, independently of diagnostics.
/// Its progress arc, relative drag target and exact readout remain distinct.
fn send_dial(state: &Value) -> (f64, f64) {
    let (level_x, level_y) = center(send_readout(state));
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
            if !(19.0..=23.0).contains(&radius)
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

/// One send row: destination, ring, dB value, Pre/Post and automation share
/// a center line, and the ring itself carries no text.
fn assert_send_row(state: &Value) {
    let (x, y) = send_dial(state);
    for text in ["Room", "Pre", "Post"] {
        let other = center(label(state, text)).1;
        assert!(
            (y - other).abs() < 0.75,
            "The ring must share the row's center line with {text}: y={y}, {other}"
        );
    }
    assert!((y - center(send_readout(state)).1).abs() < 0.75);
    for caption in state["label_rects"].as_array().unwrap() {
        let left = caption["min"][0].as_f64().unwrap();
        let top = caption["min"][1].as_f64().unwrap();
        let right = caption["max"][0].as_f64().unwrap();
        let bottom = caption["max"][1].as_f64().unwrap();
        assert!(
            right <= x - 24.0 || left >= x + 24.0 || bottom <= y - 24.0 || top >= y + 24.0,
            "The small send ring must be text-free: {caption}"
        );
    }
}

fn db_text(text: &str) -> Option<f64> {
    let number = text.strip_suffix(" dB")?.replace('−', "-");
    if number == "-∞" {
        Some(f64::NEG_INFINITY)
    } else {
        number.parse().ok()
    }
}

/// The send's dB value button, right of its destination name on one row.
fn send_readout(state: &Value) -> &Value {
    let (room_x, room_y) = center(label(state, "Room"));
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            let (x, y) = center(row);
            row["text"].as_str().and_then(db_text).is_some()
                && x > room_x
                && (y - room_y).abs() < 4.0
        })
        .min_by(|a, b| center(a).0.total_cmp(&center(b).0))
        .unwrap_or_else(|| panic!("Missing send readout: {}", state["visible_labels"]))
}

/// The level the row shows, in dB (one decimal).
fn shown_db(state: &Value) -> f64 {
    db_text(send_readout(state)["text"].as_str().unwrap()).unwrap()
}

fn db(gain: f64) -> f64 {
    20.0 * gain.log10()
}

fn open_send(app: &mut Harness) -> Value {
    let before = app.inspect();
    let (x, y) = center(send_readout(&before));
    let opened = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    label(&opened, SEND_TITLE);
    label(&opened, "Apply value");
    label(&opened, "Cancel");
    assert!(visible(&opened, "Range: -60 to 24.08 dB"));
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
                (shown_db(&changed) - db(actual)).abs() < 0.051,
                "The visible readout must agree with the live gain"
            );
            one_edit(&changed, &baseline);
            let opened = open_send(&mut app);
            unchanged(&app.click("Apply value"), &opened);
            assert_eq!(undo(&mut app)["project"], baseline["project"]);
            assert_eq!(shown_db(&app.inspect()), -12.0);
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
                to_sends(&mut app);
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
    assert_eq!(
        gain(&applied, source, bus) as f32,
        daw_ui::channel_gain_from_db(0.375_123)
    );
    one_edit(&applied, &baseline);
    assert_eq!(undo(&mut app)["project"], baseline["project"]);

    // Exact dB entry reaches unity and the top of the 0..16 send range.
    for value in ["0", "6", "24.08"] {
        open_send(&mut app);
        let before_draft = app.inspect();
        mouse_draft(&mut app, &before_draft, value);
        let applied = app.click("Apply value");
        assert_eq!(
            gain(&applied, source, bus) as f32,
            daw_ui::channel_gain_from_db(value.parse().unwrap())
        );
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
    assert_eq!(shown_db(&baseline), -12.0);
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
    let expected = daw_ui::channel_gain_from_db(0.375_123);
    assert_eq!(gain(&applied, source, bus) as f32, expected);
    assert_eq!(track(&applied, lane)["automation"]["points"], curve);
    assert!(
        (track(&applied, lane)["automation"]["override_value"]
            .as_f64()
            .unwrap()
            - f64::from(expected) / 16.0)
            .abs()
            < 1e-7
    );
    one_edit(&applied, &baseline);
    open_send(&mut app);
    let resumed = app.click("Resume automation");
    assert!(track(&resumed, lane)["automation"]["override_value"].is_null());
    assert_eq!(track(&resumed, lane)["automation"]["points"], curve);
    assert_eq!(shown_db(&resumed), -12.0);
    assert_eq!(undo(&mut app)["project"], applied["project"]);
    assert_eq!(undo(&mut app)["project"], baseline["project"]);
    app.quit();
}
