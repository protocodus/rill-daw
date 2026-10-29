//! Workspace reordering keeps live DSP instances, queued audio and later edits intact.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Effect, EffectSlot, Instrument, Note, Project};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

fn wait(app: &mut Harness, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if ready(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Playback did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn center(rect: &Value) -> [f64; 2] {
    std::array::from_fn(|axis| {
        (rect["min"][axis].as_f64().unwrap() + rect["max"][axis].as_f64().unwrap()) * 0.5
    })
}

fn drag_before(app: &mut Harness, source: u64, target: u64) -> Value {
    let state = app.inspect();
    let start = center(&channel(&state, source)["reorder_source"]);
    let drop = &channel(&state, target)["drop"];
    app.command(json!({"command":"pointer", "phase":"drag",
        "x":start[0],"y":start[1],
        "to_x":drop["min"][0].as_f64().unwrap() + 4.0,"to_y":start[1]}))
}

fn assert_engine(state: &Value, before: &Value) {
    for field in [
        "engine_id",
        "generation",
        "underruns",
        "render_errors",
        "edit_errors",
    ] {
        assert_eq!(
            state["engine"][field], before["engine"][field],
            "{field} changed"
        );
    }
    assert_eq!(state["engine"]["playing"], true);
    assert_eq!(state["playback_loading"], false);
}

fn pump(app: &mut Harness, before: &Value, value: f64, settled: bool) -> Value {
    let state = app.command(json!({"command":"pump_audio", "frames":8192}));
    assert_engine(&state, before);
    let audio = state["audio_output"].as_array().unwrap();
    let start = if settled { 0 } else { audio.len() - 512 };
    for frame in &audio[start..] {
        for sample in frame.as_array().unwrap() {
            assert!(
                (sample.as_f64().unwrap() - value).abs() < 1e-6,
                "Unexpected gap, reset or wrong channel edit: {frame}, expected {value}"
            );
        }
    }
    state
}

fn logs(directory: &Path) -> Vec<(PathBuf, Vec<Value>)> {
    let mut result: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .map(|path| {
            let lines = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            (path, lines)
        })
        .collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

fn assert_processors(directory: &Path, before: &[(PathBuf, Vec<Value>)]) {
    let after = logs(directory);
    assert_eq!(
        after.iter().map(|(path, _)| path).collect::<Vec<_>>(),
        before.iter().map(|(path, _)| path).collect::<Vec<_>>(),
        "The same instrument and insert worker processes must survive reordering"
    );
    for ((_, after), (_, before)) in after.iter().zip(before) {
        for command in [
            "LoadPlugin",
            "LoadState",
            "SetTransportPosition",
            "Reconfigure",
        ] {
            let count =
                |lines: &[Value]| lines.iter().filter(|line| !line[command].is_null()).count();
            assert_eq!(count(after), count(before), "Unexpected {command}");
        }
        for command in ["StartProcessing", "StopProcessing", "MidiPanic"] {
            let count =
                |lines: &[Value]| lines.iter().filter(|line| **line == json!(command)).count();
            assert_eq!(count(after), count(before), "Unexpected {command}");
        }
    }
}

#[test]
fn mixer_reorder_and_following_gain_edit_preserve_plugin_instances_and_live_audio() {
    let workers = tempfile::tempdir().unwrap();
    let source = workers.path().join("live_audio_host.rs");
    std::fs::write(&source, include_str!("common/live_audio_host.rs")).unwrap();
    let host = workers.path().join(if cfg!(windows) {
        "live-audio-host.exe"
    } else {
        "live-audio-host"
    });
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--edition=2024")
        .args(["-C", "opt-level=2"])
        .arg(&source)
        .arg("-o")
        .arg(&host)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let plugins = common::plugin_fixture();
    let mut app = Harness::with_virtual_audio(Some(plugins.path()), Some(&host));
    let plugin = plugins
        .path()
        .join("RillFixture.vst3")
        .to_string_lossy()
        .into_owned();
    let mut project = Project::new();
    let synth = project.add_midi_track("Held instrument");
    project
        .set_instrument(
            synth,
            Instrument::Vst3 {
                path: plugin.clone(),
                class_id: Some("11111111111111111111111111111111".into()),
            },
        )
        .unwrap();
    let instrument = project.track_mut(synth).unwrap();
    instrument.gain = 1.0;
    instrument.clips.push(Clip::midi(
        "Held phrase",
        0.0,
        16.0,
        vec![Note::new(60, 0.0, 16.0, 100)],
    ));
    instrument.inserts.push(EffectSlot::new(
        "Persistent delay",
        Effect::Vst3 {
            path: plugin,
            class_id: Some("33333333333333333333333333333333".into()),
        },
    ));
    let audio = project.add_audio_track("Audio level");
    let audio_path = app.directory.path().join("constant.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.125; 48_000 * 8],
    }
    .write_wav(&audio_path)
    .unwrap();
    project.track_mut(audio).unwrap().gain = 1.0;
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Continuous source",
        0.0,
        16.0,
        audio_path.to_string_lossy(),
    ));
    let bus = project.add_bus("Instrument bus");
    project.set_output(synth, Some(bus)).unwrap();
    let path = app.directory.path().join("reorder-playback.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1400,"height":950}));
    app.click("Mixer");
    app.click("Play");
    let started = wait(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    let baseline = pump(&mut app, &started, 0.225, false);
    let instances = logs(workers.path());
    assert_eq!(
        instances.len(),
        2,
        "An instrument and a stateful insert must actually run"
    );

    let first = drag_before(&mut app, bus, synth);
    assert_engine(&first, &baseline);
    assert_eq!(
        first["revision"], baseline["revision"],
        "Pure order is a cosmetic edit"
    );
    let state = drag_before(&mut app, audio, bus);
    assert_engine(&state, &baseline);
    assert_eq!(
        state["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![audio, bus, synth]
    );
    assert_eq!(state["revision"], baseline["revision"]);
    assert_eq!(
        state["engine"]["position_beats"],
        baseline["engine"]["position_beats"]
    );
    let reordered = pump(&mut app, &baseline, 0.225, true);
    assert_processors(workers.path(), &instances);

    let gain = center(&channel(&reordered, audio)["gain"]);
    app.command(json!({"command":"pointer","phase":"double_click","x":gain[0],"y":gain[1]}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":"-60"}));
    let edited = app.key("Enter");
    assert_eq!(edited["project"]["tracks"][0]["gain"], 0.0);
    let settled = wait(&mut app, |state| {
        state["engine"]["applied_revision"] == state["revision"]
    });
    assert_engine(&settled, &baseline);
    pump(&mut app, &baseline, 0.1, false);
    assert_processors(workers.path(), &instances);
    app.quit();
}
