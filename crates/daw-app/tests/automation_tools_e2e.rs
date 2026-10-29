//! Tool mix changes share native takeover, atomic intent, and app history.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use serde_json::{Value, json};

fn lane(state: &Value, id: u64) -> &Value {
    &state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()["automation"]
}

fn call(app: &mut Harness, name: &str, arguments: Value) -> Value {
    let state = app.command(json!({"command":"song_tool","name":name,"arguments":arguments}));
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    state
}

fn edits(app: &mut Harness, edits: Value) -> Value {
    let state = app.inspect();
    call(
        app,
        "apply_edits",
        json!({
            "session_id":state["song_session"]["id"],
            "expected_revision":state["song_session"]["revision"],
            "label":"Automation tool edit","edits":edits
        }),
    )
}

fn history(app: &mut Harness, action: &str) -> Value {
    let state = app.inspect();
    call(
        app,
        "history",
        json!({
            "session_id":state["song_session"]["id"],
            "expected_revision":state["song_session"]["revision"],"action":action
        }),
    )
}

fn near(actual: &Value, expected: f64) {
    assert!(
        (actual.as_f64().unwrap() - expected).abs() < 1e-6,
        "{actual} != {expected}"
    );
}

#[test]
fn tool_mix_edits_take_over_preserve_curves_and_respect_explicit_resume_and_bypass() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    project.track_mut(source).unwrap().gain = 0.9;
    let bus = project.add_bus("Room");
    project.add_send(source, bus, 0.25, false).unwrap();
    let lanes: Vec<_> = [
        AutomationParameter::TrackGain,
        AutomationParameter::TrackPan,
        AutomationParameter::SendGain { bus },
    ]
    .into_iter()
    .map(|parameter| {
        let lane = project
            .add_automation_track(
                parameter.name(),
                AutomationTarget {
                    track: source,
                    parameter,
                },
            )
            .unwrap();
        project.automation_lane_mut(lane).unwrap().points = vec![
            AutomationPoint::new(2.0, 0.2),
            AutomationPoint::new(6.0, 0.8),
        ];
        lane
    })
    .collect();
    let [gain, pan, send] = lanes.as_slice() else {
        unreachable!()
    };
    let path = app.directory.path().join("tool-automation.rill");
    project.save(&path).unwrap();
    let initial = app.command(json!({"command":"open","path":path}));
    let mixed = edits(
        &mut app,
        json!([
            {"op":"set_mix","channels":[{"track":source,"gain":0.6,"pan":-0.25}]},
            {"op":"update_send","track":source,"bus":bus,"gain":0.8,"pre_fader":false}
        ]),
    );
    near(&lane(&mixed, *gain)["override_value"], 0.4);
    near(&lane(&mixed, *pan)["override_value"], 0.375);
    near(&lane(&mixed, *send)["override_value"], 0.05);
    for id in &lanes {
        assert_eq!(lane(&mixed, *id)["points"], lane(&initial, *id)["points"]);
    }
    let mixer = app.click("Mixer");
    assert!(
        mixer["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Overridden")
    );
    let undone = history(&mut app, "undo");
    assert_eq!(
        undone["project"], initial["project"],
        "The complete tool mix is one undo step"
    );
    let redone = history(&mut app, "redo");
    assert_eq!(redone["project"], mixed["project"]);

    let resumed = edits(
        &mut app,
        json!([
            {"op":"set_automation_override","track":gain,"value":null}
        ]),
    );
    assert!(lane(&resumed, *gain)["override_value"].is_null());
    let undone = history(&mut app, "undo");
    near(&lane(&undone, *gain)["override_value"], 0.4);
    let redone = history(&mut app, "redo");
    assert!(lane(&redone, *gain)["override_value"].is_null());

    // A value-only diff has no gesture when the stored setting is unchanged.
    let unchanged = edits(
        &mut app,
        json!([
            {"op":"set_mix","channels":[{"track":source,"gain":0.6}]}
        ]),
    );
    assert!(lane(&unchanged, *gain)["override_value"].is_null());
    assert_eq!(unchanged["song_session"], redone["song_session"]);

    // Explicit Resume from an already-read lane still wins over implicit
    // takeover in the same atomic batch, in either order.
    for resume_first in [true, false] {
        let mix = json!({"op":"set_mix","channels":[{"track":source,"gain":if resume_first {0.3} else {0.45}}]});
        let resume = json!({"op":"set_automation_override","track":gain,"value":null});
        let batch = if resume_first {
            json!([resume, mix])
        } else {
            json!([mix, resume])
        };
        let state = edits(&mut app, batch);
        assert!(lane(&state, *gain)["override_value"].is_null());
        assert_eq!(
            lane(&state, *gain)["points"],
            lane(&initial, *gain)["points"]
        );
    }
    // Even an unchanged explicit override is authoritative in a mixed batch.
    let explicit = edits(
        &mut app,
        json!([
            {"op":"set_automation_override","track":send,"value":0.05},
            {"op":"update_send","track":source,"bus":bus,"gain":0.4,"pre_fader":false}
        ]),
    );
    near(&lane(&explicit, *send)["override_value"], 0.05);

    // Bypassing the lane leaves its stored ownership untouched while a command
    // edits the manual source. Re-enabling it resumes the retained read curve.
    edits(
        &mut app,
        json!([{"op":"set_mix","channels":[{"track":gain,"mute":true}]}]),
    );
    let bypassed = edits(
        &mut app,
        json!([{"op":"set_mix","channels":[{"track":source,"gain":0.75}]}]),
    );
    assert!(lane(&bypassed, *gain)["override_value"].is_null());
    let enabled = edits(
        &mut app,
        json!([{"op":"set_mix","channels":[{"track":gain,"mute":false}]}]),
    );
    assert!(lane(&enabled, *gain)["override_value"].is_null());
    assert_eq!(
        lane(&enabled, *gain)["points"],
        lane(&initial, *gain)["points"]
    );
    app.quit();
}
