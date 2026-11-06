//! Gain automation uses its target's dB range and offers song-wide framing.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Clip, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, parent: &str, child: &str) -> Value {
    app.click(parent);
    app.click(child)
}

fn fixture(send: bool, length: f64, meter: u8, value: f32) -> (Harness, u64, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    project.tempo = 80.0;
    project.beats_per_bar = meter;
    let source = project.add_midi_track("Gain source");
    if length > 0.0 {
        project.track_mut(source).unwrap().clips.push(Clip::midi(
            "Song extent",
            0.0,
            length,
            vec![],
        ));
    }
    let parameter = if send {
        let bus = project.add_bus("Gain destination");
        project.add_send(source, bus, 1.0, false).unwrap();
        AutomationParameter::SendGain { bus }
    } else {
        AutomationParameter::TrackGain
    };
    let lane = project
        .add_automation_track(
            "Gain lane",
            AutomationTarget {
                track: source,
                parameter,
            },
        )
        .unwrap();
    let first = AutomationPoint::new(0.0, value);
    let point = first.id;
    project
        .track_mut(lane)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points = vec![first, AutomationPoint::new(4.0, value)];
    let path = app.directory.path().join("gain-units.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Gain lane");
    menu(&mut app, "Window", "Automation");
    (app, lane, point)
}

fn lane(state: &Value, id: u64) -> &Value {
    &state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()["automation"]
}

fn point(state: &Value, lane_id: u64, id: u64) -> &Value {
    lane(state, lane_id)["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|point| point["id"] == id)
        .unwrap()
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

fn open_gain(app: &mut Harness, field: &str) -> Value {
    let state = app.inspect();
    click_rect(app, &state["automation_editor"][field])
}

fn enter(app: &mut Harness, text: &str) -> Value {
    app.command(json!({"command":"text","text":text}));
    app.key("Enter")
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "revision"] {
        assert_eq!(state[field], before[field], "{field} changed while viewing");
    }
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.000_001,
        "{actual} != {expected}"
    );
}

fn assert_primary_popup_contrast(app: &mut Harness, dark: bool) {
    // An idle primary action must remain readable without hover or focus.
    app.command(json!({"command":"pointer","phase":"move","x":10,"y":10}));
    for _ in 0..12 {
        app.inspect();
    }
    let state = app.command(json!({"command":"inspect","paint":true}));
    let text = state["painted_text"]
        .as_array()
        .unwrap()
        .iter()
        .find(|shape| shape["text"] == "Apply value")
        .expect("the exact gain popup paints its primary action");
    let action = if dark {
        daw_ui::theme::DARK.action
    } else {
        daw_ui::theme::LIGHT.action
    };
    let bounds = |rect: &Value| {
        [
            rect["min"][0].as_f64().unwrap(),
            rect["min"][1].as_f64().unwrap(),
            rect["max"][0].as_f64().unwrap(),
            rect["max"][1].as_f64().unwrap(),
        ]
    };
    let [left, top, right, bottom] = bounds(&text["rect"]);
    let background = state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|shape| {
            let [x0, y0, x1, y1] = bounds(&shape["rect"]);
            shape["fill"] == json!(action.to_array())
                && x0 <= left
                && y0 <= top
                && x1 >= right
                && y1 >= bottom
                && y1 - y0 <= 44.0
                && x1 - x0 < right - left + 60.0
        })
        .expect("the primary action's explicit fill must be painted behind its label");
    let luminance = |rgba: &Value| {
        let channel = |index: usize| {
            let value = rgba[index].as_f64().unwrap() / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        assert_eq!(rgba[3], 255);
        0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2)
    };
    let foregrounds = text["colors"].as_array().unwrap();
    assert!(!foregrounds.is_empty());
    for foreground in foregrounds {
        let a = luminance(foreground);
        let b = luminance(&background["fill"]);
        let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(contrast >= 4.5, "dark={dark}, actual contrast={contrast}");
    }
}

#[test]
fn gain_popup_primary_action_is_readable_in_both_themes_and_keeps_commit_guards() {
    for dark in [false, true] {
        let (mut app, lane_id, point_id) = fixture(false, 64.0, 4, 0.4);
        app.command(json!({"command":"system_theme","dark":dark}));
        let original = app.inspect();
        open_gain(&mut app, "point_value_rect");
        app.command(json!({"command":"text","text":"-12"}));
        assert_primary_popup_contrast(&mut app, dark);
        unchanged(&app.inspect(), &original);
        let changed = app.click("Apply value");
        near(
            point(&changed, lane_id, point_id)["value"]
                .as_f64()
                .unwrap(),
            10.0_f64.powf(-12.0 / 20.0) / 1.5,
        );

        open_gain(&mut app, "point_value_rect");
        app.command(json!({"command":"text","text":"invalid"}));
        unchanged(&app.click("Apply value"), &changed);
        unchanged(&app.key("Enter"), &changed);
        unchanged(&app.click("Cancel"), &changed);

        open_gain(&mut app, "point_value_rect");
        app.command(json!({"command":"text","text":"-6"}));
        assert_primary_popup_contrast(&mut app, dark);
        let keyboard = app.key("Enter");
        near(
            point(&keyboard, lane_id, point_id)["value"]
                .as_f64()
                .unwrap(),
            10.0_f64.powf(-6.0 / 20.0) / 1.5,
        );
        app.quit();
    }
}

#[test]
fn track_and_send_points_use_db_with_precise_commit_undo_and_recall() {
    for send in [false, true] {
        let (mut app, lane_id, point_id) = fixture(send, 64.0, 4, 0.374_981_3);
        let baseline = app.inspect();
        let maximum = if send { 16.0 } else { 1.5 };
        let axis = if send { "+24.1 dB" } else { "+3.5 dB" };
        assert!(
            baseline["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == axis)
        );

        open_gain(&mut app, "point_value_rect");
        unchanged(&app.key("Enter"), &baseline);
        open_gain(&mut app, "point_value_rect");
        app.command(json!({"command":"text","text":"-9.125"}));
        unchanged(&app.key("Escape"), &baseline);

        open_gain(&mut app, "point_value_rect");
        let changed = enter(&mut app, "-6.25");
        near(
            point(&changed, lane_id, point_id)["value"]
                .as_f64()
                .unwrap(),
            10.0_f64.powf(-6.25 / 20.0) / maximum,
        );
        assert_eq!(
            changed["history"]["undo"].as_array().unwrap().len(),
            baseline["history"]["undo"].as_array().unwrap().len() + 1
        );
        assert_eq!(
            menu(&mut app, "Edit", "Undo")["project"],
            baseline["project"]
        );
        assert_eq!(
            menu(&mut app, "Edit", "Redo")["project"],
            changed["project"]
        );

        let path = app.directory.path().join("gain-edited.rill");
        app.command(json!({"command":"save","path":path}));
        let recalled = app.command(json!({"command":"open","path":path}));
        assert_eq!(
            point(&recalled, lane_id, point_id),
            point(&changed, lane_id, point_id)
        );
        app.click("Gain lane");
        menu(&mut app, "Window", "Automation");
        open_gain(&mut app, "point_value_rect");
        let silent = app.click("Silence");
        assert_eq!(point(&silent, lane_id, point_id)["value"], 0.0);
        assert!(
            silent["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == "Gain −∞ dB")
        );
        app.quit();
    }
}

#[test]
fn gain_popup_bounds_and_override_entry_keep_normalized_curves() {
    for send in [false, true] {
        let (mut app, lane_id, point_id) = fixture(send, 64.0, 4, 0.2);
        let baseline = app.inspect();
        open_gain(&mut app, "point_value_rect");
        let attempted = enter(&mut app, "24");
        if send {
            near(
                point(&attempted, lane_id, point_id)["value"]
                    .as_f64()
                    .unwrap(),
                10.0_f64.powf(24.0 / 20.0) / 16.0,
            );
            menu(&mut app, "Edit", "Undo");
        } else {
            unchanged(&attempted, &baseline);
            assert!(
                attempted["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| label == "The value must be inside the range.")
            );
            app.key("Escape");
        }

        let overridden = app.click("Override automation");
        assert!(lane(&overridden, lane_id)["override_value"].is_number());
        open_gain(&mut app, "override_value_rect");
        let held = enter(&mut app, "-12");
        near(
            lane(&held, lane_id)["override_value"].as_f64().unwrap(),
            10.0_f64.powf(-12.0 / 20.0) / if send { 16.0 } else { 1.5 },
        );
        assert_eq!(
            lane(&held, lane_id)["points"],
            lane(&baseline, lane_id)["points"]
        );
        let resumed = app.click("Resume automation");
        assert!(lane(&resumed, lane_id)["override_value"].is_null());
        assert_eq!(
            lane(&resumed, lane_id)["points"],
            lane(&baseline, lane_id)["points"]
        );
        app.quit();
    }
}

#[test]
fn viewing_quiet_gain_and_moving_position_preserve_exact_normalized_values() {
    for send in [false, true] {
        for value in [0.0, 0.000_000_01, 0.374_981_3, 1.0] {
            let (mut app, lane_id, point_id) = fixture(send, 64.0, 4, value);
            let before = app.inspect();
            open_gain(&mut app, "point_value_rect");
            unchanged(&app.key("Escape"), &before);
            let state = app.inspect();
            click_rect(&mut app, &state["automation_editor"]["point_beat_rect"]);
            app.command(json!({"command":"key","key":"A","cmd":true}));
            let moved = enter(&mut app, "3.1.1");
            assert_eq!(point(&moved, lane_id, point_id)["beat"], 8.0);
            assert_eq!(
                point(&moved, lane_id, point_id)["value"],
                point(&before, lane_id, point_id)["value"]
            );
            assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
            app.quit();
        }
    }
}

#[test]
fn fit_song_frames_arrangement_and_empty_meter_without_editing_or_replacing_fit() {
    for (width, height) in [(1440, 1000), (800, 600)] {
        for length in [64.0, 0.0] {
            let (mut app, _, _) = fixture(false, length, 3, 0.25);
            app.command(json!({"command":"viewport","width":width,"height":height}));
            let before = app.click("Fit");
            assert!(before["automation_editor"]["end_beat"].as_f64().unwrap() < 5.0);
            let button = before["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|label| label["text"] == "Fit song")
                .unwrap();
            assert!(button["min"][0].as_f64().unwrap() >= 0.0);
            assert!(button["max"][0].as_f64().unwrap() <= f64::from(width));
            assert!(button["max"][1].as_f64().unwrap() <= f64::from(height));
            let fitted = app.click("Fit song");
            unchanged(&fitted, &before);
            assert_eq!(
                fitted["automation_editor"]["selected_point"],
                before["automation_editor"]["selected_point"]
            );
            let expected = length.max(3.0);
            assert_eq!(fitted["automation_editor"]["start_beat"], 0.0);
            let end = fitted["automation_editor"]["end_beat"].as_f64().unwrap();
            assert!(
                end >= expected && end < expected + 3.0,
                "fit song end: {end}"
            );
            let points = app.click("Fit");
            unchanged(&points, &before);
            assert!(points["automation_editor"]["end_beat"].as_f64().unwrap() < 5.0);
            app.click("Fit song");
            let keyboard_fit = app.key("F");
            unchanged(&keyboard_fit, &before);
            assert!(
                keyboard_fit["automation_editor"]["end_beat"]
                    .as_f64()
                    .unwrap()
                    < 5.0
            );
            app.quit();
        }
    }
}
