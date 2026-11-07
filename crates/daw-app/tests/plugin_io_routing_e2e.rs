//! Native bus inspection, sidechain wiring and output creation through the app UI.
mod common;
#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::{Harness, wait_until};
use daw_core::{
    AudioData, Clip, Effect, EffectSlot, Instrument, Note, PluginParameterValue, Project,
};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};

fn open(app: &mut Harness, project: &Project) {
    let path = app.directory.path().join("io-session.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
}

fn inspect_io(app: &mut Harness, clip: &str) -> Value {
    app.click(clip);
    app.click("Window");
    app.click("Routing");
    app.click("Inspect I/O");
    let state = wait_until(app, |state| state["plugin_io"]["status"] != "loading");
    assert_eq!(state["plugin_io"]["status"], "ready", "{state}");
    assert_eq!(state["plugin_io"]["current"], true);
    state
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

fn write_source(app: &Harness, name: &str, value: f32) -> String {
    let path = app.directory.path().join(name);
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![value; 96_000],
    }
    .write_wav(&path)
    .unwrap();
    path.to_string_lossy().into_owned()
}

fn sample(audio: &AudioData) -> f32 {
    audio.samples[12_000 * 2]
}

#[test]
fn sidechain_gui_uses_saved_overrides_and_routes_real_audio_with_one_undo() {
    let fixture = EditorProbeFixture::build_multi_bus_effect();
    let mut app = Harness::with_vst_path(&fixture.path);
    let mut project = Project::new();
    let source = project.add_audio_track("Main channel");
    let key = project.add_audio_track("Sidechain source");
    let silent = project.add_bus("Key monitor");
    project.track_mut(silent).unwrap().gain = 0.0;
    project.track_mut(key).unwrap().output_bus = Some(silent);
    for (track, name, value) in [(source, "Main signal", 0.4), (key, "Key signal", 0.1)] {
        let channel = project.track_mut(track).unwrap();
        channel.gain = 1.0;
        channel.clips.push(Clip::audio(
            name,
            0.0,
            2.0,
            write_source(&app, &format!("{track}.wav"), value),
        ));
    }
    let mut effect = EffectSlot::new(
        "Detector",
        Effect::Vst3 {
            path: fixture.path.to_string_lossy().into_owned(),
            class_id: Some(CLASS_ID.into()),
        },
    );
    effect
        .parameters
        .push(PluginParameterValue { id: 7, value: 0.75 });
    project.track_mut(source).unwrap().inserts.push(effect);
    open(&mut app, &project);
    let before = export(&mut app, "before.wav");
    assert!((sample(&before) - 0.3).abs() < 1e-6);
    let inspected = inspect_io(&mut app, "Main signal");
    assert_eq!(
        inspected["plugin_io"]["details"]["inputs"][1]["name"],
        "Restored sidechain"
    );
    app.click("Add source…");
    let routed = app.click("Sidechain source");
    assert_eq!(
        routed["project"]["tracks"][0]["inserts"][0]["inputs"][0]["source_track"],
        key
    );
    assert_eq!(
        routed["project"]["tracks"][0]["inserts"][0]["inputs"][0]["bus"],
        1
    );
    assert_eq!(routed["history"]["undo"].as_array().unwrap().len(), 1);
    let after = export(&mut app, "sidechain.wav");
    assert!(
        (sample(&after) - 0.3375).abs() < 1e-6,
        "Sidechain must reach the actual native processor"
    );
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(before.samples, export(&mut app, "undo.wav").samples);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(after.samples, export(&mut app, "redo.wav").samples);
    let archive = app.directory.path().join("routed.rill");
    app.command(json!({"command":"save","path":archive}));
    app.command(json!({"command":"open","path":archive}));
    assert_eq!(after.samples, export(&mut app, "reopened.wav").samples);
    app.quit();
}

#[test]
fn native_output_gui_creates_all_stereo_pairs_and_odd_mono_outputs_atomically() {
    let fixture = EditorProbeFixture::build_multi_bus();
    let mut app = Harness::with_vst_path(&fixture.path);
    let mut project = Project::new();
    let owner = project.add_midi_track("Multi-output instrument");
    project
        .set_instrument(
            owner,
            Instrument::Vst3 {
                path: fixture.path.to_string_lossy().into_owned(),
                class_id: Some(CLASS_ID.into()),
            },
        )
        .unwrap();
    let track = project.track_mut(owner).unwrap();
    track.gain = 1.0;
    track.clips.push(Clip::midi(
        "Output phrase",
        0.0,
        2.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    open(&mut app, &project);
    let before = export(&mut app, "main-only.wav");
    assert!((sample(&before) - 0.125).abs() < 1e-6);
    let state = inspect_io(&mut app, "Output phrase");
    assert_eq!(state["plugin_io"]["details"]["outputs"][1]["channels"], 5);
    let created = app.click("Enable all output channels");
    let tracks = created["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 5);
    assert_eq!(created["history"]["undo"].as_array().unwrap().len(), 1);
    let pairs: Vec<_> = tracks
        .iter()
        .filter_map(|track| {
            let output = &track["instrument_output"];
            output.is_object().then(|| {
                (
                    output["bus"].as_u64().unwrap(),
                    output["first_channel"].as_u64().unwrap(),
                )
            })
        })
        .collect();
    assert_eq!(pairs, [(1, 0), (1, 2), (1, 4), (2, 0)]);
    let mixed = export(&mut app, "all-outputs.wav");
    assert!((sample(&mixed) - 2.625).abs() < 1e-6);
    assert!((mixed.samples[12_000 * 2 + 1] - 2.875).abs() < 1e-6);
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(before.samples, export(&mut app, "outputs-undo.wav").samples);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(mixed.samples, export(&mut app, "outputs-redo.wav").samples);
    let archive = app.directory.path().join("multi-output.rill");
    app.command(json!({"command":"save","path":archive}));
    app.command(json!({"command":"open","path":archive}));
    assert_eq!(
        mixed.samples,
        export(&mut app, "outputs-reopened.wav").samples
    );
    app.quit();
}
