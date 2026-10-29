//! Exact channel control entry is visible, deferred, and compatible with automation.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use daw_ui::theme;
use serde_json::{Value, json};

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
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

fn channel<'a>(state: &'a Value, view: &str, id: u64) -> &'a Value {
    state[view]["channels"]
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

fn lane(state: &Value, id: u64) -> &Value {
    &track(state, id)["automation"]
}

fn unchanged(actual: &Value, before: &Value) {
    for field in ["project", "history", "song_session", "modified"] {
        assert_eq!(
            actual[field], before[field],
            "Viewing or drafting changed {field}"
        );
    }
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn open_control(app: &mut Harness, view: &str, id: u64, name: &str) -> Value {
    let state = app.inspect();
    let opened = right_click(app, &channel(&state, view, id)[name]);
    assert!(visible(&opened, "Value"));
    assert!(visible(&opened, "Apply value"));
    assert!(visible(&opened, "Cancel"));
    unchanged(&opened, &state);
    opened
}

fn range_text(state: &Value) -> &str {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|text| text.starts_with("Range:"))
        .unwrap_or_else(|| panic!("Missing explicit range: {}", state["visible_labels"]))
}

fn send_value_rect(state: &Value) -> &Value {
    let (_, level_y) = center(label(state, "Level"));
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            row["text"]
                .as_str()
                .and_then(|text| text.trim_end_matches('×').trim().parse::<f64>().ok())
                .is_some()
                && (center(row).1 - level_y).abs() < 4.0
        })
        .max_by(|left, right| center(left).0.total_cmp(&center(right).0))
        .unwrap_or_else(|| panic!("No send value control: {}", state["visible_labels"]))
}

fn shows_number(state: &Value, expected: f64) {
    assert!(
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|text| text.parse::<f64>().ok())
            .any(|number| (number - expected).abs() < 1e-4),
        "Exact current value {expected} is missing: {}",
        state["visible_labels"]
    );
}

fn assert_pan_marker(app: &mut Harness, source: u64, dark: bool, marker: &str) {
    app.command(json!({"command":"pointer","phase":"move","x":1095,"y":755}));
    let state = app.command(json!({"command":"inspect","paint":true}));
    let handle = &channel(&state, "mixer", source)["pan_scale"]["handle_center"];
    let palette = if dark { theme::DARK } else { theme::LIGHT };
    let color = match marker {
        "A" => palette.automation,
        "!" => palette.automation_override,
        _ => palette.muted,
    };
    assert!(
        state["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| {
                let (x, y) = center(&text["rect"]);
                text["text"] == marker
                    && text["colors"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(color.to_array()))
                    && (x - handle[0].as_f64().unwrap()).abs() <= 24.0
                    && (y - handle[1].as_f64().unwrap()).abs() <= 12.0
            }),
        "Missing {marker} state marker beside the actual pan handle"
    );
}

/// Opening the real menu focuses and selects the current number, so typing and
/// clicking Apply need neither Ctrl+A nor Enter. Drafting is not a song edit.
fn draft(app: &mut Harness, before: &Value, text: &str) -> Value {
    let state = app.command(json!({"command":"text","text":text}));
    unchanged(&state, before);
    assert!(
        visible(&state, text),
        "The old value was not fully selected: {}",
        state["visible_labels"]
    );
    state
}

fn fixture(app: &mut Harness, compact: bool, automated: bool) -> (u64, u64, Option<u64>) {
    let mut project = Project::new();
    let source = project.add_audio_track("Precise channel");
    project.track_mut(source).unwrap().gain = 0.75;
    project.track_mut(source).unwrap().pan = 0.173456;
    project.track_mut(source).unwrap().collapsed = compact;
    project.add_audio_track("Other channel");
    let bus = project.add_bus("Room");
    project.add_send(source, bus, 8.0, false).unwrap();
    let automation = automated.then(|| {
        let id = project
            .add_automation_track_with_value(
                "Pan movement",
                AutomationTarget {
                    track: source,
                    parameter: AutomationParameter::TrackPan,
                },
                0.25,
            )
            .unwrap();
        project.automation_lane_mut(id).unwrap().points = vec![
            AutomationPoint::new(8.0, 0.25),
            AutomationPoint::new(12.0, 0.75),
        ];
        id
    });
    let path = app.directory.path().join("practical-controls.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    (source, bus, automation)
}

#[test]
fn mixer_pan_exact_entry_shows_range_defers_commit_and_rejects_invalid_values() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let (source, _, _) = fixture(&mut app, false, false);
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1100,"height":760,"pixels_per_point":density}),
        );
        let before = app.click("Mixer");
        let opened = open_control(&mut app, "mixer", source, "pan");
        let range = range_text(&opened);
        assert!(range.contains("-100") && range.contains('%'), "{range}");
        shows_number(&opened, 17.3456);
        draft(&mut app, &before, "-23.4567");
        let applied = app.click("Apply value");
        near(track(&applied, source)["pan"].as_f64().unwrap(), -0.234567);
        assert_eq!(
            applied["history"]["undo"].as_array().unwrap().len(),
            before["history"]["undo"].as_array().unwrap().len() + 1
        );
        let reopened = open_control(&mut app, "mixer", source, "pan");
        shows_number(&reopened, -23.4567);
        app.click("Cancel");

        for invalid in ["not a number", "NaN", "101"] {
            let baseline = app.inspect();
            open_control(&mut app, "mixer", source, "pan");
            let drafted = draft(&mut app, &baseline, invalid);
            assert!(visible(
                &drafted,
                if invalid == "101" {
                    "The value must be inside the range."
                } else {
                    "Enter a finite number."
                }
            ));
            let (x, y) = center(label(&drafted, invalid));
            let focused = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
            unchanged(&focused, &baseline);
            assert!(
                visible(&focused, "Cancel"),
                "Clicking the input must not dismiss its editor"
            );
            let rejected = app.click("Apply value");
            unchanged(&rejected, &baseline);
            assert!(
                visible(&rejected, "Cancel"),
                "Invalid entry must remain editable"
            );
            assert!(visible(&rejected, invalid));
            let cancelled = app.click("Cancel");
            unchanged(&cancelled, &baseline);
        }
        let baseline = app.inspect();
        open_control(&mut app, "mixer", source, "pan");
        draft(&mut app, &baseline, "75.125");
        let cancelled = app.click("Cancel");
        unchanged(&cancelled, &baseline);
        assert_eq!(undo(&mut app)["project"], before["project"]);
        app.quit();
    }
}

#[test]
fn compact_header_gain_and_pan_have_exact_entry_without_expanding_the_track() {
    let mut app = Harness::new();
    let (source, _, _) = fixture(&mut app, true, false);
    app.command(json!({"command":"viewport","width":1100,"height":720}));
    // The action belongs to its channel even while another track is selected.
    app.click("Other channel");
    let original = app.inspect();
    assert_eq!(channel(&original, "timeline", source)["compact"], true);
    let gain = open_control(&mut app, "timeline", source, "gain");
    assert!(range_text(&gain).contains("dB"));
    draft(&mut app, &original, "-18.125");
    let gained = app.click("Apply value");
    near(
        track(&gained, source)["gain"].as_f64().unwrap(),
        10_f64.powf(-18.125 / 20.0),
    );
    assert_eq!(track(&gained, source)["collapsed"], true);
    assert_eq!(channel(&gained, "timeline", source)["compact"], true);
    assert_eq!(gained["selected_track"], original["selected_track"]);
    let gain = open_control(&mut app, "timeline", source, "gain");
    shows_number(&gain, -18.125);
    app.click("Cancel");
    let before_pan = app.inspect();
    open_control(&mut app, "timeline", source, "pan");
    draft(&mut app, &before_pan, "12.345");
    let panned = app.click("Apply value");
    near(track(&panned, source)["pan"].as_f64().unwrap(), 0.12345);
    assert_eq!(track(&panned, source)["collapsed"], true);
    assert_eq!(undo(&mut app)["project"], gained["project"]);
    assert_eq!(undo(&mut app)["project"], original["project"]);
    app.quit();
}

#[test]
fn exact_automated_pan_entry_and_explicit_override_preserve_the_curve_and_resume() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let (source, _, automation) = fixture(&mut app, false, true);
        let automation = automation.unwrap();
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(json!({"command":"viewport","width":1100,"height":760}));
        let original = app.click("Mixer");
        let points = lane(&original, automation)["points"].clone();
        assert_pan_marker(&mut app, source, dark, "A");
        let opened = open_control(&mut app, "mixer", source, "pan");
        assert!(visible(&opened, "Automated"));
        assert!(visible(&opened, "Override automation"));
        shows_number(&opened, -50.0);
        // Merely opening or drafting must not store the automated effective value.
        near(track(&opened, source)["pan"].as_f64().unwrap(), 0.173456);
        draft(&mut app, &original, "33.333");
        let overridden = app.click("Apply value");
        near(track(&overridden, source)["pan"].as_f64().unwrap(), 0.33333);
        near(
            lane(&overridden, automation)["override_value"]
                .as_f64()
                .unwrap(),
            0.666665,
        );
        assert_eq!(lane(&overridden, automation)["points"], points);
        assert_eq!(
            channel(&overridden, "mixer", source)["automation_state"],
            "Overridden"
        );
        assert_pan_marker(&mut app, source, dark, "!");
        let opened = open_control(&mut app, "mixer", source, "pan");
        assert!(visible(&opened, "Overridden") && visible(&opened, "Resume automation"));
        shows_number(&opened, 33.333);
        let resumed = app.click("Resume automation");
        assert!(lane(&resumed, automation)["override_value"].is_null());
        assert_eq!(lane(&resumed, automation)["points"], points);
        assert_eq!(
            channel(&resumed, "mixer", source)["automation_state"],
            "Automated"
        );
        assert_pan_marker(&mut app, source, dark, "A");
        let opened = open_control(&mut app, "mixer", source, "pan");
        shows_number(&opened, -50.0);
        let held = app.click("Override automation");
        near(
            lane(&held, automation)["override_value"].as_f64().unwrap(),
            0.25,
        );
        assert_eq!(lane(&held, automation)["points"], points);
        assert_eq!(
            channel(&held, "mixer", source)["automation_state"],
            "Overridden"
        );
        assert_eq!(undo(&mut app)["project"], resumed["project"]);
        assert_eq!(undo(&mut app)["project"], overridden["project"]);
        assert_eq!(undo(&mut app)["project"], original["project"]);
        open_control(&mut app, "mixer", source, "pan");
        app.click("Edit automation");
        app.click("Enabled");
        let bypassed = app.click("Mixer");
        assert_eq!(
            channel(&bypassed, "mixer", source)["automation_state"],
            "Bypassed"
        );
        assert_eq!(lane(&bypassed, automation)["points"], points);
        assert_pan_marker(&mut app, source, dark, "–");
        let opened = open_control(&mut app, "mixer", source, "pan");
        assert!(visible(&opened, "Bypassed"));
        shows_number(&opened, 17.3456);
        app.click("Cancel");
        assert_eq!(undo(&mut app)["project"], original["project"]);
        app.quit();
    }
}

#[test]
fn routing_send_exact_entry_exposes_full_range_and_recalls_the_fraction() {
    let mut app = Harness::new();
    let (source, bus, _) = fixture(&mut app, false, false);
    app.click("Mixer");
    app.click("Precise channel");
    let before = app.click("Routing");
    let opened = right_click(&mut app, send_value_rect(&before));
    assert!(visible(&opened, "Value"));
    assert_eq!(range_text(&opened), "Range: 0 to 16 ×");
    shows_number(&opened, 8.0);
    draft(&mut app, &before, "12.3456");
    let applied = app.click("Apply value");
    let sends = track(&applied, source)["sends"].as_array().unwrap();
    let send = sends.iter().find(|send| send["target_bus"] == bus).unwrap();
    near(send["gain"].as_f64().unwrap(), 12.3456);
    let saved = app.directory.path().join("exact-send.rill");
    app.command(json!({"command":"save","path":saved}));
    let recalled = Project::load(&saved).unwrap();
    near(
        f64::from(recalled.track(source).unwrap().sends[0].gain),
        12.3456,
    );
    let state = app.inspect();
    let opened = right_click(&mut app, send_value_rect(&state));
    shows_number(&opened, 12.3456);
    app.click("Cancel");
    assert_eq!(undo(&mut app)["project"], before["project"]);
    app.quit();
}

#[test]
fn channel_properties_volume_readout_is_directly_editable_to_fractional_db() {
    let mut app = Harness::new();
    let (source, _, _) = fixture(&mut app, false, false);
    app.click("Mixer");
    let before = app.click("Precise channel");
    assert_eq!(before["popups"]["properties"], true);
    let (_, gain_y) = center(label(&before, "Gain"));
    let numeric = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|row| {
            row["text"]
                .as_str()
                .is_some_and(|text| text.ends_with(" dB"))
                && (center(row).1 - gain_y).abs() < 12.0
        })
        .unwrap_or_else(|| {
            panic!(
                "No editable channel dB readout: {}",
                before["visible_labels"]
            )
        });
    let (x, y) = center(numeric);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":"-9.875"}));
    // Clicking the field label commits/focuses away without a keyboard shortcut.
    let edited = app.click("Gain");
    near(
        track(&edited, source)["gain"].as_f64().unwrap(),
        10_f64.powf(-9.875 / 20.0),
    );
    assert_eq!(undo(&mut app)["project"], before["project"]);
    app.quit();
}
