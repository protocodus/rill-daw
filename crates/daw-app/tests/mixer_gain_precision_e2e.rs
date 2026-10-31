//! Concise gain captions retain precise editing, storage and automation values.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use serde_json::{Value, json};

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn readout(state: &Value, gain: f32) -> &Value {
    let text = format!("{:.1} dB", daw_ui::channel_gain_to_db(gain));
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn focus(app: &mut Harness, gain: f32) -> Value {
    let state = app.inspect();
    let (x, y) = center(readout(&state, gain));
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}))
}

fn unchanged(state: &Value, baseline: &Value) {
    for key in ["project", "history", "modified"] {
        assert_eq!(state[key], baseline[key], "Viewing a gain changed {key}");
    }
}

fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}

fn gain(state: &Value) -> f32 {
    state["project"]["tracks"][0]["gain"].as_f64().unwrap() as f32
}

#[test]
fn fractional_and_out_of_range_gains_survive_focus_blur_and_cancel_exactly() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let gains = [0.752_345_6_f32, 0.000_05, 2.0, 0.0];
    for (index, gain) in gains.into_iter().enumerate() {
        let id = project.add_bus(format!("Stored gain {index}"));
        project.track_mut(id).unwrap().gain = gain;
    }
    let path = app.directory.path().join("precise-stored-gains.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1200, "height":900}));
    let baseline = app.click("Mixer");
    for dark in [false, true] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        for gain in gains {
            let idle = app.inspect();
            readout(&idle, gain);
            unchanged(&idle, &baseline);
            let editing = focus(&mut app, gain);
            assert!(
                visible(&editing, &daw_ui::channel_gain_to_db(gain).to_string()),
                "Editing must expose the complete stored dB precision: {}",
                editing["visible_labels"]
            );
            unchanged(&editing, &baseline);
            unchanged(&app.click("Mixer"), &baseline);
            focus(&mut app, gain);
            let draft = app.command(json!({"command":"text", "text":"-9.876543"}));
            unchanged(&draft, &baseline);
            let cancelled = app.key("Escape");
            unchanged(&cancelled, &baseline);
            assert_eq!(cancelled["view"], "Mixer");
            unchanged(&app.inspect(), &baseline);
        }
    }
    // A canceled/blurred editor must not leave a stale navigation guard.
    let exited = app.key("Escape");
    assert_eq!(exited["view"], "Arrangement");
    unchanged(&exited, &baseline);
    app.quit();
}

#[test]
fn precise_commits_mouse_drags_limits_and_recall_preserve_gain_automation() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_bus("Precise automated gain");
    project.track_mut(track).unwrap().gain = 0.463_217_8;
    let target = AutomationTarget {
        track,
        parameter: AutomationParameter::TrackGain,
    };
    let lane = project
        .add_automation_track("Gain envelope", target)
        .unwrap();
    let normalized = 0.501_563_7;
    project.automation_lane_mut(lane).unwrap().points = vec![
        AutomationPoint::new(0.0, normalized),
        AutomationPoint::new(8.0, normalized),
    ];
    let effective = AutomationParameter::TrackGain.denormalize(normalized);
    let path = app.directory.path().join("precise-automated-gain.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1100, "height":850}));
    let baseline = app.click("Mixer");
    focus(&mut app, effective);
    unchanged(&app.click("Mixer"), &baseline);

    let state = app.inspect();
    let (x, y) = center(readout(&state, effective));
    let dragged = app.command(json!({"command":"pointer", "phase":"drag",
        "x":x, "y":y, "to_x":x+22.0, "to_y":y}));
    assert!(gain(&dragged) > effective);
    assert!(dragged["project"]["tracks"][1]["automation"]["override_value"].is_number());
    assert_eq!(
        dragged["project"]["tracks"][1]["automation"]["points"],
        baseline["project"]["tracks"][1]["automation"]["points"]
    );
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        baseline["project"]
    );

    focus(&mut app, effective);
    let draft = app.command(json!({"command":"text", "text":"-6.312345"}));
    assert_eq!(draft["project"], baseline["project"]);
    let edited = app.click("Mixer");
    let expected = daw_ui::channel_gain_from_db(-6.312_345);
    assert_eq!(gain(&edited).to_bits(), expected.to_bits());
    readout(&edited, expected);
    assert!(edited["project"]["tracks"][1]["automation"]["override_value"].is_number());
    assert_eq!(
        edited["project"]["tracks"][1]["automation"]["points"],
        baseline["project"]["tracks"][1]["automation"]["points"]
    );
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        baseline["project"]
    );
    assert_eq!(menu(&mut app, "Edit", "Redo")["project"], edited["project"]);

    for (text, clamped) in [("99", 1.5_f32), ("-90", 0.0)] {
        focus(&mut app, expected);
        app.command(json!({"command":"text", "text":text}));
        let limited = app.click("Mixer");
        assert!((gain(&limited) - clamped).abs() < 1e-6);
        assert_eq!(menu(&mut app, "Edit", "Undo")["project"], edited["project"]);
    }

    let bundle = app.directory.path().join("precise-gain-saved.rill");
    app.command(json!({"command":"file_dialog", "kind":"save", "path":bundle}));
    menu(&mut app, "File", "Save as…");
    app.command(json!({"command":"wait_project"}));
    let mut app = app.restart();
    app.command(json!({"command":"file_dialog", "kind":"open", "path":bundle}));
    menu(&mut app, "File", "Open…");
    let restored = app.command(json!({"command":"wait_project"}));
    assert_eq!(gain(&restored).to_bits(), expected.to_bits());
    assert_eq!(restored["project"], edited["project"]);
    let mixer = app.click("Mixer");
    readout(&mixer, expected);
    app.quit();
}
