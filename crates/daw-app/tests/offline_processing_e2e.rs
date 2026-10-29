//! Offline exports allow expensive native blocks without relaxing live playback.
//! The native fixture also rejects process calls unless VST3 kOffline is sent.
mod common;
#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::Harness;
use daw_core::{AudioData, Clip, Instrument, Note, Project};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn helper() -> PathBuf {
    let directory = std::env::var_os("RILL_E2E_BINARY")
        .map(|path| PathBuf::from(path).parent().unwrap().to_owned())
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_owned()
        });
    let path = directory.join(if cfg!(windows) {
        "daw-plugin-host.exe"
    } else {
        "daw-plugin-host"
    });
    assert!(
        path.is_file(),
        "Build the workspace binaries before this test"
    );
    path
}

fn open_native_song(app: &mut Harness, plugin: &EditorProbeFixture) -> Value {
    let mut project = Project::new();
    let track = project.add_midi_track("Offline synth");
    project
        .set_instrument(
            track,
            Instrument::Vst3 {
                path: plugin.path.to_string_lossy().into_owned(),
                class_id: Some(CLASS_ID.into()),
            },
        )
        .unwrap();
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Short phrase",
        0.0,
        0.25,
        vec![Note::new(60, 0.0, 0.125, 100)],
    ));
    let path = app.directory.path().join("Offline song.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}))
}

fn start_export(app: &mut Harness, path: &Path) {
    app.command(json!({"command":"file_dialog", "kind":"export", "path":path}));
    app.click("File");
    app.click("Export WAV…");
    app.click("Export…");
}

#[test]
fn offline_export_accepts_a_slow_first_block_and_advertises_offline_processing() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_millis(400));
    let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &helper());
    let original = open_native_song(&mut app, &plugin);
    let path = app.directory.path().join("Slow first block.wav");
    start_export(&mut app, &path);
    let completed = app.command(json!({"command":"wait_export", "timeout_ms":15000}));
    assert_eq!(completed["error"], false, "{}", completed["status"]);
    assert_eq!(completed["export_pending"], false);
    assert_eq!(completed["project"], original["project"]);
    assert_eq!(completed["history"], original["history"]);
    let audio = AudioData::load_wav(path).unwrap();
    assert_eq!(audio.sample_rate, 48_000);
    assert_eq!(audio.channels, 2);
    assert!(audio.samples.iter().all(|sample| sample.is_finite()));
    assert!(
        audio.samples.iter().any(|sample| sample.abs() > 0.01),
        "Native offline processor must produce its note, not a silent fallback"
    );
    app.quit();
}

#[test]
fn local_audio_analysis_uses_offline_processing_without_changing_the_song() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_millis(400));
    let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &helper());
    let original = open_native_song(&mut app, &plugin);
    // Application dispatch uses the same local worker as song tools, MCP and
    // the assistant. This call does not start a model/provider or use a network.
    let measured = app.command(json!({"command":"application_tool", "name":"audio_analyze",
        "arguments":{"start_beat":0.0,"end_beat":0.25}}));
    assert_eq!(measured["tool_result"]["ok"], true, "{measured}");
    let result = &measured["tool_result"]["result"];
    assert_eq!(result["frames"], 6_000);
    assert_eq!(result["silent"], false);
    assert!(result["sample_peak_linear"]["left"].as_f64().unwrap() > 0.01);
    assert_eq!(measured["project"], original["project"]);
    assert_eq!(measured["history"], original["history"]);
    assert_eq!(measured["song_session"], original["song_session"]);
    assert_eq!(measured["device_open"], false);
    app.quit();
}

#[test]
fn stopping_a_local_analysis_job_keeps_notes_and_ignores_its_late_result() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_secs(2));
    let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &helper());
    let original = open_native_song(&mut app, &plugin);
    // A deterministic in-process script supplies the tool request. No API key,
    // external provider or network is involved in this cancellation regression.
    app.command(json!({"command":"assistant_script", "turns":[
        {"calls":[{"id":"measure", "name":"audio_analyze",
            "arguments":{"start_beat":0.0,"end_beat":0.25}}]},
        {"text":"Analysis completed after the result."}
    ]}));
    app.click("Window");
    let opened = app.click("Assistant");
    let rect = &opened["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"text","text":"Measure this short phrase locally."}));
    app.click("Send");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = app.inspect();
        if state["assistant"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["role"] == "Tool"
                    && message["text"] == daw_ai::tool_activity("audio_analyze")
            })
        {
            assert_eq!(state["assistant"]["busy"], true);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Local analysis was not dispatched: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    app.click("Stop");
    // Let an already-running native block finish, so a late successful response
    // cannot silently replace the user's explicit stopped state.
    let deadline = Instant::now() + Duration::from_millis(2_500);
    loop {
        let state = app.inspect();
        assert_eq!(state["assistant"]["busy"], false);
        assert_eq!(state["assistant"]["status"], "Stopped");
        assert_eq!(state["project"], original["project"]);
        assert_eq!(state["history"], original["history"]);
        assert_eq!(state["song_session"], original["song_session"]);
        assert!(
            !state["assistant"]["messages"]
                .to_string()
                .contains("Analysis completed after")
        );
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    app.quit();
}

#[test]
fn offline_hung_block_still_fails_with_a_bounded_deadline_and_preserves_the_old_wav() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_secs(10));
    let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &helper());
    let original = open_native_song(&mut app, &plugin);
    let path = app.directory.path().join("Existing output.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0.23; 64],
    }
    .write_wav(&path)
    .unwrap();
    let old_wav = std::fs::read(&path).unwrap();
    let started = Instant::now();
    start_export(&mut app, &path);
    app.raw_command(json!({"command":"wait_export", "timeout_ms":15000}));
    let failed = app.inspect();
    assert_eq!(failed["error"], true, "{failed}");
    assert_eq!(failed["export_pending"], false);
    assert!(
        started.elapsed() >= Duration::from_secs(5),
        "Offline work must get its own budget"
    );
    assert!(
        started.elapsed() < Duration::from_secs(12),
        "Hung native work must stay bounded"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        old_wav,
        "Failed export must not replace an existing file"
    );
    assert_eq!(failed["project"], original["project"]);
    assert_eq!(failed["history"], original["history"]);
    let details = app.click("Details");
    assert!(
        details["error_details"]["details"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
    app.key("Escape");
    app.quit();
}
