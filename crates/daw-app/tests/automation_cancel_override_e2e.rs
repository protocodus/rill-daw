//! Global manual-takeover cancellation is a visible, reversible song action.
mod common;

use common::{Harness, wait_until};
use daw_core::{
    AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Instrument,
    PluginParameterValue, Project,
};
use serde_json::{Value, json};

fn menu(app: &mut Harness, parent: &str, child: &str) -> Value {
    app.click(parent);
    app.click(child)
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
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

fn add_lane(
    project: &mut Project,
    target: AutomationTarget,
    points: [f32; 2],
    held: Option<f32>,
) -> u64 {
    let lane = project
        .add_automation_track_with_value("Envelope", target, points[0])
        .unwrap();
    project.automation_lane_mut(lane).unwrap().points = vec![
        AutomationPoint::new(0.0, points[0]),
        AutomationPoint::new(4.0, points[1]),
    ];
    project.set_automation_override(lane, held).unwrap();
    lane
}

fn cleared_project(before: &Value) -> Value {
    let mut expected = before["project"].clone();
    for track in expected["tracks"].as_array_mut().unwrap() {
        if let Some(lane) = track["automation"].as_object_mut() {
            lane.remove("override_value");
        }
    }
    expected
}

fn assert_selection(state: &Value, before: &Value) {
    for key in ["selected_track", "selected_clip"] {
        assert_eq!(
            state[key], before[key],
            "Global cancellation must preserve {key}"
        );
    }
}

fn assert_single_cancel(state: &Value, before: &Value) {
    assert_eq!(
        state["project"],
        cleared_project(before),
        "Only override fields change"
    );
    assert_eq!(
        state["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert_eq!(state["history"]["undo"][0], "Cancel automation override");
    assert!(
        !visible(state, "Cancel automation override"),
        "Global button hides after cancellation"
    );
    assert_selection(state, before);
}

#[test]
fn global_mouse_action_and_edit_menu_clear_all_targets_in_one_undoable_step() {
    let mut app = Harness::new();
    let audio = app.directory.path().join("Not playing.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0.0; 192_000],
    }
    .write_wav(&audio)
    .unwrap();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    let bus = project.add_bus("Room");
    project.add_send(source, bus, 0.5, false).unwrap();
    let synth = project.add_midi_track("Modeled instrument");
    project
        .set_instrument(
            synth,
            Instrument::Vst3 {
                path: app
                    .directory
                    .path()
                    .join("Unloaded.vst3")
                    .to_string_lossy()
                    .into_owned(),
                class_id: None,
            },
        )
        .unwrap();
    project.track_mut(synth).unwrap().instrument_parameters =
        vec![PluginParameterValue { id: 7, value: 0.3 }];
    project.track_mut(source).unwrap().clips.push(Clip::audio(
        "Selected clip",
        0.0,
        4.0,
        audio.to_string_lossy(),
    ));
    let gain = add_lane(
        &mut project,
        AutomationTarget {
            track: source,
            parameter: AutomationParameter::TrackGain,
        },
        [0.2, 0.8],
        None,
    );
    add_lane(
        &mut project,
        AutomationTarget {
            track: source,
            parameter: AutomationParameter::TrackPan,
        },
        [0.25, 0.75],
        Some(0.9),
    );
    add_lane(
        &mut project,
        AutomationTarget {
            track: source,
            parameter: AutomationParameter::SendGain { bus },
        },
        [0.1, 0.4],
        Some(0.7),
    );
    let bypassed = add_lane(
        &mut project,
        AutomationTarget {
            track: bus,
            parameter: AutomationParameter::TrackGain,
        },
        [0.3, 0.4],
        Some(0.8),
    );
    project.track_mut(bypassed).unwrap().mute = true;
    add_lane(
        &mut project,
        AutomationTarget {
            track: synth,
            parameter: AutomationParameter::VstParameter {
                effect: None,
                id: 7,
            },
        },
        [0.1, 0.9],
        Some(0.6),
    );
    let path = app.directory.path().join("Global override.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    app.click("Selected clip");

    // Put the playhead in the middle of the saved ramps, using the real ruler.
    let state = app.inspect();
    let timeline = &state["timeline"];
    let x = timeline["ruler"]["min"][0].as_f64().unwrap()
        + (2.0 - timeline["start_beat"].as_f64().unwrap())
            * timeline["pixels_per_beat"].as_f64().unwrap();
    let y = (timeline["ruler"]["min"][1].as_f64().unwrap()
        + timeline["ruler"]["max"][1].as_f64().unwrap())
        * 0.5;
    let positioned = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(positioned["timeline"]["cursor"], 2.0);
    app.click("Mixer");

    // One of the holds must come from an actual manual slider gesture, rather
    // than the fixture alone. The saved ramp is retained during takeover.
    let before_drag = app.inspect();
    let scale = &channel(&before_drag, source)["fader_scale"];
    let at = std::array::from_fn::<_, 2, _>(|axis| {
        let minimum = scale["min_position"][axis].as_f64().unwrap();
        minimum + (scale["max_position"][axis].as_f64().unwrap() - minimum) * 0.9
    });
    let held = app.command(json!({"command":"pointer", "phase":"click", "x":at[0], "y":at[1]}));
    assert!(track(&held, gain)["automation"]["override_value"].is_number());
    assert_eq!(
        track(&held, gain)["automation"]["points"],
        track(&before_drag, gain)["automation"]["points"]
    );
    assert!(visible(&held, "Cancel automation override"));
    let cleared = app.click("Cancel automation override");
    assert_single_cancel(&cleared, &held);
    assert_eq!(
        track(&cleared, bypassed)["mute"],
        true,
        "Bypassed lanes retain their bypass"
    );
    let displayed = app.inspect();
    let expected_gain = f64::from(daw_ui::gain_taper(0.75));
    assert!(
        (channel(&displayed, source)["fader_scale"]["value"]
            .as_f64()
            .unwrap()
            - expected_gain)
            .abs()
            < 1e-6
    );
    assert!(
        channel(&displayed, source)["pan_scale"]["value"]
            .as_f64()
            .unwrap()
            .abs()
            < 1e-6,
        "Saved curves resume at current beat 2, not at their first point"
    );
    assert_eq!(
        channel(&displayed, source)["send_automation"][0]["state"],
        "Automated"
    );

    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], held["project"]);
    assert!(visible(&undone, "Cancel automation override"));
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], cleared["project"]);
    assert_selection(&redone, &held);
    let idle = redone.clone();
    app.click("Edit");
    let no_op = app.click("Cancel all automation overrides");
    for key in ["project", "history", "revision", "modified"] {
        assert_eq!(
            no_op[key], idle[key],
            "A disabled no-op must not change {key}"
        );
    }
    app.key("Escape");
    menu(&mut app, "Edit", "Undo");
    let before_menu = app.inspect();
    let from_menu = menu(&mut app, "Edit", "Cancel all automation overrides");
    assert_single_cancel(&from_menu, &before_menu);
    assert_eq!(from_menu["project"], cleared["project"]);
    app.quit();
}

fn rendered_tail(app: &mut Harness, expected: [f64; 2]) {
    wait_until(app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    let output = app.command(json!({"command":"pump_audio", "frames":8192}));
    assert_eq!(output["engine"]["render_errors"], 0);
    let samples = output["audio_output"].as_array().unwrap();
    for sample in &samples[samples.len() - 256..] {
        for axis in 0..2 {
            assert!(
                (sample[axis].as_f64().unwrap() - expected[axis]).abs() < 2e-6,
                "Rendered sample {sample} must resume saved automation {expected:?}"
            );
        }
    }
}

#[test]
fn global_cancellation_resumes_gain_pan_and_send_curves_in_the_running_render_worker() {
    let mut app = Harness::with_virtual_audio(None, None);
    let audio = app.directory.path().join("Constant stereo.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..384_000).flat_map(|_| [0.5, 0.25]).collect(),
    }
    .write_wav(&audio)
    .unwrap();
    let mut project = Project::new();
    let source = project.add_audio_track("Live source");
    let bus = project.add_bus("Return");
    project.add_send(source, bus, 0.5, false).unwrap();
    project.track_mut(source).unwrap().clips.push(Clip::audio(
        "Source",
        0.0,
        16.0,
        audio.to_string_lossy(),
    ));
    for (parameter, value, held) in [
        (AutomationParameter::TrackGain, 0.2, 0.8),
        (AutomationParameter::TrackPan, 0.5, 1.0),
        (AutomationParameter::SendGain { bus }, 0.03125, 0.0625),
    ] {
        add_lane(
            &mut project,
            AutomationTarget {
                track: source,
                parameter,
            },
            [value, value],
            Some(held),
        );
    }
    let path = app.directory.path().join("Live global override.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    app.click("Mixer");
    app.click("Play");
    rendered_tail(&mut app, [0.0, 0.6]);
    let before = app.inspect();
    let cleared = app.click("Cancel automation override");
    assert_single_cancel(&cleared, &before);
    rendered_tail(&mut app, [0.225, 0.1125]);
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    rendered_tail(&mut app, [0.0, 0.6]);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], cleared["project"]);
    rendered_tail(&mut app, [0.225, 0.1125]);
    app.click("Stop");
    app.quit();
}
