//! Every built-in device sits in one frame with its name and presets on the
//! bar, and shows a lane's state in the value of the control it drives.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Effect, EffectSlot,
    Project,
};
use serde_json::{Value, json};

fn labels<'a>(state: &'a Value, text: &str) -> Vec<&'a Value> {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["text"] == text)
        .collect()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn states(state: &Value) -> Vec<Value> {
    state["automation_indicators"]
        .as_array()
        .unwrap()
        .iter()
        .map(|indicator| indicator["state"].clone())
        .collect()
}

#[test]
fn spacer_sits_in_its_frame_and_resumes_an_overridden_lane_from_its_control() {
    let mut app = Harness::new();
    let tone = app.directory.path().join("tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0.0; 2 * 48_000],
    }
    .write_wav(&tone)
    .unwrap();
    let mut project = Project::new();
    let pad = project.add_audio_track("Pad");
    project.track_mut(pad).unwrap().clips.push(Clip::audio(
        "Pad tone",
        0.0,
        2.0,
        tone.to_string_lossy(),
    ));
    let slot = EffectSlot::new(
        "R1 Spacer",
        Effect::Builtin {
            device: "r1-spacer".into(),
        },
    );
    let effect = Some(slot.id);
    project.track_mut(pad).unwrap().inserts.push(slot);
    let mut lane = |id: u32, value: f32| {
        let lane = project
            .add_automation_track_with_value(
                format!("Spacer {id}"),
                AutomationTarget {
                    track: pad,
                    parameter: AutomationParameter::VstParameter { effect, id },
                },
                value,
            )
            .unwrap();
        project.automation_lane_mut(lane).unwrap().points = vec![
            AutomationPoint::new(0.0, value),
            AutomationPoint::new(64.0, value),
        ];
        lane
    };
    // Echo feedback follows its lane; reverb size is held by hand elsewhere.
    lane(daw_devices::devices::spacer::ECHO_FEEDBACK, 0.7);
    let size = lane(daw_devices::devices::spacer::SIZE, 0.8);
    project.set_automation_override(size, Some(0.3)).unwrap();
    let path = app.directory.path().join("frame.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1480,"height":960,"pixels_per_point":1.0}));
    app.click("Pad tone");
    app.click("Window");
    app.click("Effects");
    app.click("Open editor");
    common::wait_visible(&mut app, "PRESET");
    let state = app.inspect();

    // The name and the presets share the bar, above every section, and the
    // body repeats neither.
    let names = labels(&state, "R1 Spacer");
    assert_eq!(names.len(), 1, "{}", state["visible_labels"]);
    let (name_x, name_y) = center(names[0]);
    let preset = labels(&state, "PRESET");
    assert_eq!(preset.len(), 1);
    let (preset_x, preset_y) = center(preset[0]);
    assert!((name_y - preset_y).abs() < 4.0, "{name_y} {preset_y}");
    assert!(preset_x > name_x + 100.0, "presets stand at the right");
    let (_, echo_y) = center(labels(&state, "ECHO")[0]);
    assert!(echo_y > name_y + 40.0);

    let shown = states(&state);
    assert!(shown.contains(&json!("Automated")), "{shown:?}");
    assert!(shown.contains(&json!("Overridden")), "{shown:?}");

    // The overridden control resumes its lane from its own context menu.
    let overridden = state["automation_indicators"]
        .as_array()
        .unwrap()
        .iter()
        .find(|indicator| indicator["state"] == "Overridden")
        .unwrap();
    let (x, y) = center(overridden);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    let resumed = app.click("Resume automation");
    let held = resumed["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == size)
        .unwrap()["automation"]["override_value"]
        .clone();
    assert!(held.is_null(), "{held}");
    let after = states(&app.inspect());
    assert!(!after.contains(&json!("Overridden")), "{after:?}");
    assert_eq!(
        after.iter().filter(|state| **state == "Automated").count(),
        2,
        "{after:?}"
    );

    // Random 10% moves the device's own values in one Undo step and keeps
    // its output level.
    let before = app.inspect();
    let inserts = |state: &Value| state["project"]["tracks"][0]["inserts"][0]["parameters"].clone();
    let rolled = app.click("10%");
    assert_ne!(inserts(&rolled), inserts(&before), "{}", rolled["project"]);
    let output = |state: &Value| {
        inserts(state)
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["id"] == daw_devices::devices::spacer::OUTPUT)
            .cloned()
    };
    assert_eq!(output(&rolled), output(&before));
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(inserts(&undone), inserts(&before));
    app.quit();
}
