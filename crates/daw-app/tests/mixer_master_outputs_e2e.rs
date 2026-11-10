//! Real plug-in outputs retain their audio while mixer groups are folded;
//! Master is a complete, automatable signal path rather than a display-only sum.
mod common;
#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::{Harness, wait_until};
use daw_core::{
    AudioData, AutomationParameter, AutomationTarget, Clip, Effect, EffectSlot, Instrument,
    MASTER_CHANNEL_ID, Note, Project,
};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};

fn center(rect: &Value) -> (f64, f64) {
    assert!(rect.is_object(), "Missing interactive rectangle: {rect}");
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn click(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}
fn right_click(app: &mut Harness, rect: &Value) {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
}
fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}
fn width(rect: &Value) -> f64 {
    rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap()
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    let finished = app.command(json!({"command":"wait_export"}));
    assert!(path.is_file(), "WAV not written: {finished}");
    AudioData::load_wav(path).unwrap()
}
fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    app.click(parent);
    app.click(item)
}

fn reveal_routing_action(app: &mut Harness, label: &str, down: bool) {
    for _ in 0..10 {
        let state = app.inspect();
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| text == label)
        {
            return;
        }
        let window = state["window_rects"].as_array().unwrap().last().unwrap();
        let (x, y) = center(window);
        app.command(json!({"command":"scroll","x":x,"y":y,"delta":if down {-350.0} else {350.0}}));
    }
    panic!("Routing action was not mouse-reachable: {label}");
}

#[test]
fn a_master_automation_lane_reveals_its_master_controls_in_a_wide_session() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let lane = project
        .add_automation_track(
            "Master volume lane",
            AutomationTarget {
                track: MASTER_CHANNEL_ID,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    for index in 0..16 {
        project.add_audio_track(format!("Source {index}"));
    }
    let path = app.directory.path().join("Wide console.rill");
    project.save(&path).unwrap();
    let before = app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1280,"height":800}));
    app.click("Master volume lane");
    let mixer = app.click("Mixer");
    assert_eq!(mixer["selected_track"], lane);
    assert_eq!(mixer["mixer"]["focused_channel"], MASTER_CHANNEL_ID);
    let pane = &mixer["mixer"]["reorder_viewport"];
    let name = &mixer["mixer"]["master"]["name"];
    assert!(name["min"][0].as_f64().unwrap() >= pane["min"][0].as_f64().unwrap());
    assert!(
        name["max"][0].as_f64().unwrap() <= pane["max"][0].as_f64().unwrap(),
        "Master navigation must scroll the actual mouse controls into the viewport"
    );
    assert_eq!(mixer["project"], before["project"]);
    assert_eq!(mixer["history"], before["history"]);
    app.quit();
}

#[test]
fn routing_inspection_enables_real_native_output_pairs_and_connects_a_master_sidechain() {
    let instrument = EditorProbeFixture::build_multi_bus();
    let effect = EditorProbeFixture::build_multi_bus_effect();
    let mut app = Harness::with_virtual_audio(Some(&instrument.path), None);
    let driver_path = app.directory.path().join("Driver.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0.125; 48_000],
    }
    .write_wav(&driver_path)
    .unwrap();
    let mut project = Project::new();
    let owner = project.add_midi_track("Drum machine");
    project
        .set_instrument(
            owner,
            Instrument::Vst3 {
                path: instrument.path.to_string_lossy().into_owned(),
                class_id: Some(CLASS_ID.into()),
            },
        )
        .unwrap();
    project.track_mut(owner).unwrap().gain = 1.0;
    project.track_mut(owner).unwrap().clips.push(Clip::midi(
        "Pattern",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let driver = project.add_audio_track("Sidechain driver");
    project.track_mut(driver).unwrap().gain = 1.0;
    project.track_mut(driver).unwrap().clips.push(Clip::audio(
        "Driver",
        0.0,
        1.0,
        driver_path.to_string_lossy(),
    ));
    let slot = EffectSlot::new(
        "Master processor",
        Effect::Vst3 {
            path: effect.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    );
    project.master.inserts.push(slot);
    project.master.gain = 0.125;
    let path = app.directory.path().join("I-O.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    app.click("Mixer");
    app.click("Drum machine");
    app.click("Routing");
    let before = app.inspect();
    app.click("Inspect I/O");
    let inspected = wait_until(&mut app, |state| state["plugin_io"]["status"] == "ready");
    assert_eq!(
        inspected["plugin_io"]["details"]["outputs"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    reveal_routing_action(&mut app, "Enable all output channels", true);
    let enabled = app.click("Enable all output channels");
    let outputs: Vec<_> = enabled["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|track| track["instrument_output"]["track"] == owner)
        .map(|track| track["id"].as_u64().unwrap())
        .collect();
    assert_eq!(
        outputs.len(),
        4,
        "Wide buses split into stereo pairs plus a final mono channel"
    );
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(
        undone["project"], before["project"],
        "Enabling every output is one undoable operation"
    );
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], enabled["project"]);
    app.click("Mixer");
    let baseline = export(&mut app, "Dry Master.wav");
    let mixer = app.inspect();
    click(&mut app, &mixer["mixer"]["master"]["name"]);
    app.click("Routing");
    reveal_routing_action(&mut app, "Inspect I/O", false);
    app.click("Inspect I/O");
    let inspected = wait_until(&mut app, |state| {
        // The inspector reports ready a frame before the routing panel draws its controls.
        state["plugin_io"]["status"] == "ready"
            && state["plugin_io"]["track"] == MASTER_CHANNEL_ID
            && state["visible_labels"]
                .as_array()
                .is_some_and(|labels| labels.iter().any(|label| label == "Add source…"))
    });
    assert_eq!(
        inspected["plugin_io"]["details"]["inputs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    app.command(json!({"command":"click","label":"Add source…","occurrence":0}));
    let connected = app.click("Sidechain driver");
    let routes = connected["project"]["master"]["inserts"][0]["inputs"]
        .as_array()
        .unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["source_track"], driver);
    assert_eq!(routes[0]["bus"], 1);
    assert_eq!(routes[0]["first_channel"], 0);
    app.click("Mixer");
    let keyed = export(&mut app, "Keyed Master.wav");
    for index in [100, 101] {
        assert!(
            (keyed.samples[index] - baseline.samples[index] - 0.00390625).abs() < 1e-5,
            "An extra input must reach the actual Master processor alongside its main input"
        );
    }
    let archived = app.directory.path().join("Routed Master.rill");
    app.command(json!({"command":"save","path":archived}));
    let recalled = app.command(json!({"command":"open","path":archived}));
    assert_eq!(
        recalled["project"]["master"]["inserts"][0]["inputs"],
        connected["project"]["master"]["inserts"][0]["inputs"]
    );
    assert_eq!(
        export(&mut app, "Recalled Master.wav").samples,
        keyed.samples
    );
    app.quit();
}

#[test]
fn instrument_outputs_default_folded_expand_independently_and_keep_the_same_rendered_audio() {
    let fixture = EditorProbeFixture::build_multi_bus();
    for dark in [false, true] {
        let mut app = Harness::with_virtual_audio(Some(&fixture.path), None);
        let mut project = Project::new();
        let owner = project.add_midi_track("Drum instrument");
        project
            .set_instrument(
                owner,
                Instrument::Vst3 {
                    path: fixture.path.to_string_lossy().into_owned(),
                    class_id: Some(CLASS_ID.into()),
                },
            )
            .unwrap();
        let source = project.track_mut(owner).unwrap();
        source.gain = 1.0;
        source.clips.push(Clip::midi(
            "Pattern",
            0.0,
            1.0,
            vec![Note::new(60, 0.0, 1.0, 100)],
        ));
        let unrelated = project.add_bus("Unrelated bus");
        let outputs = [
            (1, 0, "Kick"),
            (1, 2, "Snare"),
            (1, 4, "Room mono"),
            (2, 0, "Percussion"),
        ]
        .map(|(bus, pair, name)| {
            let id = project
                .add_instrument_output(owner, name, bus, pair)
                .unwrap();
            project.track_mut(id).unwrap().gain = 1.0;
            id
        });
        project.master.gain = 0.125;
        let path = app.directory.path().join("Multi-output.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(json!({"command":"viewport","width":1280,"height":800}));
        app.click("Mixer");
        let before_audio = export(&mut app, "folded.wav");
        assert!(
            before_audio
                .samples
                .iter()
                .any(|sample| sample.abs() > 0.01)
        );
        let original = app.inspect();
        right_click(&mut app, &channel(&original, outputs[0])["name"]);
        let shifted = app.click("Shift Right");
        let shifted_order: Vec<_> = shifted["mixer"]["channel_order"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_u64)
            .collect();
        assert_eq!(
            shifted_order,
            [
                owner, outputs[1], outputs[0], outputs[2], outputs[3], unrelated
            ],
            "Output context movement follows visible output siblings, not unrelated arranger rows"
        );
        let restored = menu(&mut app, "Edit", "Undo");
        assert_eq!(restored["project"], original["project"]);
        let before = app.inspect();
        let order = before["mixer"]["channel_order"].as_array().unwrap();
        assert_eq!(
            order,
            &vec![
                json!(owner),
                json!(outputs[0]),
                json!(outputs[1]),
                json!(outputs[2]),
                json!(outputs[3]),
                json!(unrelated)
            ]
        );
        assert_eq!(channel(&before, owner)["expanded"], true);
        for id in outputs {
            let output = channel(&before, id);
            assert_eq!(output["expanded"], false);
            assert_eq!(output["instrument_group"], owner);
            assert!(width(&output["strip"]) < width(&channel(&before, owner)["strip"]));
            assert!(output["meter"].is_object());
            assert!(output["mute"].is_object() && output["solo"].is_object());
        }
        let group = before["mixer"]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["instrument"] == owner)
            .unwrap();
        let expanded = click(&mut app, &group["rect"]);
        for id in outputs {
            assert_eq!(channel(&expanded, id)["expanded"], true);
            for field in [
                "input_gain",
                "effects",
                "insert_summary",
                "pan",
                "gain",
                "fader",
                "meter",
                "target",
            ] {
                for edge in ["min", "max"] {
                    assert!(
                        (channel(&expanded, id)[field][edge][1].as_f64().unwrap()
                            - channel(&expanded, owner)[field][edge][1].as_f64().unwrap())
                        .abs()
                            < 0.1,
                        "Full output channels retain the {field} calibration and row baseline"
                    );
                }
            }
        }
        assert_eq!(expanded["project"], before["project"]);
        assert_eq!(expanded["history"], before["history"]);
        let collapsed_one = click(&mut app, &channel(&expanded, outputs[1])["disclosure"]);
        assert_eq!(channel(&collapsed_one, outputs[1])["expanded"], false);
        assert_eq!(channel(&collapsed_one, outputs[0])["expanded"], true);
        let restored = click(&mut app, &channel(&collapsed_one, outputs[1])["disclosure"]);
        assert_eq!(channel(&restored, outputs[1])["expanded"], true);
        assert_eq!(restored["project"], before["project"]);
        assert_eq!(restored["history"], before["history"]);
        assert_eq!(
            export(&mut app, "expanded.wav").samples,
            before_audio.samples,
            "Disclosure cannot change processing, output activation, routing or musical state"
        );
        let soloed = click(&mut app, &channel(&restored, outputs[0])["solo"]);
        assert_eq!(
            soloed["project"]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|track| track["id"] == outputs[0])
                .unwrap()["solo"],
            true
        );
        let solo_audio = export(&mut app, "Output solo.wav");
        assert!(
            (solo_audio.samples[100] - 0.03125).abs() < 1e-5,
            "Solo keeps the shared instrument rendering, but routes only this output"
        );
        assert!((solo_audio.samples[101] - 0.046875).abs() < 1e-5);
        let undone = menu(&mut app, "Edit", "Undo");
        assert_eq!(undone["project"], before["project"]);
        assert_eq!(
            export(&mut app, "Output undo.wav").samples,
            before_audio.samples
        );
        app.quit();
    }
}

#[test]
fn master_has_working_insert_volume_pan_input_gain_and_mouse_automation_for_each() {
    let fixture = EditorProbeFixture::build_multi_bus_effect();
    let mut app = Harness::with_virtual_audio(Some(&fixture.path), None);
    let source_path = app.directory.path().join("Tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0.2; 48_000],
    }
    .write_wav(&source_path)
    .unwrap();
    let mut project = Project::new();
    let source = project.add_audio_track("Tone");
    project.track_mut(source).unwrap().gain = 1.0;
    project.track_mut(source).unwrap().clips.push(Clip::audio(
        "Tone clip",
        0.0,
        1.0,
        source_path.to_string_lossy(),
    ));
    project.master.inserts.push(EffectSlot::new(
        "Master level",
        Effect::Vst3 {
            path: fixture.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    ));
    let path = app.directory.path().join("Master.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1280,"height":800}));
    app.click("Mixer");
    let baseline = export(&mut app, "Master baseline.wav");
    assert!(
        (baseline.samples[100] - 0.1).abs() < 1e-5,
        "Master insert must process the summed audio"
    );
    let state = app.inspect();
    let master = &state["mixer"]["master"];
    for field in [
        "input_gain",
        "pan",
        "gain",
        "fader",
        "effects",
        "analyze",
        "statistics",
    ] {
        assert!(master[field].is_object(), "{field}");
    }
    let (x, y) = center(&master["gain"]);
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"-6.0205999"}));
    let changed = app.key("Enter");
    assert!((changed["project"]["master"]["gain"].as_f64().unwrap() - 0.5).abs() < 1e-5);
    let quieter = export(&mut app, "Master quieter.wav");
    for (before, after) in baseline.samples.iter().zip(&quieter.samples) {
        assert!(
            (after - before * 0.5).abs() < 1e-5,
            "Master volume is part of the exported signal path"
        );
    }
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"]["master"]["gain"], 1.0);
    let mixer = app.inspect();
    click(&mut app, &mixer["mixer"]["master"]["input_gain"]);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"-6"}));
    let trimmed = app.key("Enter");
    assert!(
        (trimmed["project"]["master"]["input_gain"].as_f64().unwrap() - 10_f64.powf(-6.0 / 20.0))
            .abs()
            < 1e-6
    );
    let undo = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(
        undo["project"]["master"]["input_gain"], 1.0,
        "Committing exact input gain releases text focus back to document history"
    );
    for (field, parameter) in [
        ("input_gain", AutomationParameter::TrackInputGain),
        ("fader", AutomationParameter::TrackGain),
        ("pan", AutomationParameter::TrackPan),
    ] {
        let mixer = app.click("Mixer");
        right_click(&mut app, &mixer["mixer"]["master"][field]);
        let automated = app.click("Automate");
        let expected = serde_json::to_value(AutomationTarget {
            track: MASTER_CHANNEL_ID,
            parameter,
        })
        .unwrap();
        assert!(
            automated["project"]["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|track| track["automation"]["target"] == expected)
        );
        app.key("Escape");
    }
    let mixer = app.click("Mixer");
    let effects = click(&mut app, &mixer["mixer"]["master"]["effects"]);
    assert_eq!(effects["selected_track"], MASTER_CHANNEL_ID);
    assert_eq!(effects["popups"]["effects"], true);
    app.key("Escape");
    let mixer = app.click("Mixer");
    right_click(&mut app, &channel(&mixer, source)["input_gain"]);
    let automated = app.click("Automate");
    let expected = serde_json::to_value(AutomationTarget {
        track: source,
        parameter: AutomationParameter::TrackInputGain,
    })
    .unwrap();
    assert!(
        automated["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|track| track["automation"]["target"] == expected)
    );
    app.quit();
}
