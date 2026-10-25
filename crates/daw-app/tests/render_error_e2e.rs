//! A native plugin timeout stays contained and remains inspectable from the app.
mod common;
#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::Harness;
use daw_core::{AudioData, Clip, Instrument, Note, Project};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn production_helper() -> PathBuf {
    let directory = if let Some(binary) = std::env::var_os("RILL_E2E_BINARY") {
        PathBuf::from(binary).parent().unwrap().to_owned()
    } else {
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned()
    };
    let helper = directory.join(if cfg!(windows) {
        "daw-plugin-host.exe"
    } else {
        "daw-plugin-host"
    });
    assert!(
        helper.is_file(),
        "Build the workspace binaries before this test"
    );
    helper
}

fn wait_failure(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if state["error"] == true
            && state["status"]
                .as_str()
                .unwrap()
                .contains("Processing instrument")
        {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Missing plugin diagnostic: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn healthy_audio(app: &mut Harness, engine_id: &Value) -> Value {
    let state = app.command(json!({"command":"pump_audio", "frames":4096}));
    assert_eq!(&state["engine"]["engine_id"], engine_id);
    assert_eq!(state["engine"]["playing"], true);
    assert_eq!(
        state["engine"]["render_errors"], 1,
        "Failure must remain latched"
    );
    let samples = state["audio_output"].as_array().unwrap();
    assert_eq!(samples.len(), 4096);
    for frame in samples {
        let left = frame[0].as_f64().unwrap();
        let right = frame[1].as_f64().unwrap();
        assert!((left - 0.3).abs() < 0.0001, "Healthy left output: {frame}");
        assert!(
            (right - 0.15).abs() < 0.0001,
            "Healthy right output: {frame}"
        );
    }
    state
}

#[test]
fn native_render_failure_details_are_copyable_while_healthy_audio_keeps_playing() {
    // The real native processor exceeds the unchanged 100 ms audio deadline.
    // Its startup succeeds; only processing fails, without a desktop or device.
    let plugin = EditorProbeFixture::build_with_delays(Duration::ZERO, Duration::from_millis(500));
    let mut app = Harness::with_virtual_audio(
        Some(plugin.path.parent().unwrap()),
        Some(&production_helper()),
    );
    let source = app.directory.path().join("healthy.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..192_000).flat_map(|_| [0.3, 0.15]).collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = true;
    project.loop_start = 0.0;
    project.loop_end = 8.0;
    let failed = project.add_midi_track("Failed synth 🎹");
    project
        .set_instrument(
            failed,
            Instrument::Vst3 {
                path: plugin.path.to_string_lossy().into_owned(),
                class_id: Some(CLASS_ID.into()),
            },
        )
        .unwrap();
    project.track_mut(failed).unwrap().clips.push(Clip::midi(
        "Synth phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 4.0, 100)],
    ));
    let healthy = project.add_audio_track("Healthy audio");
    let track = project.track_mut(healthy).unwrap();
    track.gain = 1.0;
    track.clips.push(Clip::audio(
        "Healthy loop",
        0.0,
        8.0,
        source.to_string_lossy(),
    ));
    let session = app.directory.path().join("render-failure.rill");
    project.save(&session).unwrap();
    app.command(json!({"command":"open", "path":session}));
    app.click("Play");
    let failure = wait_failure(&mut app);
    let engine_id = failure["engine"]["engine_id"].clone();
    assert_eq!(failure["native_editor"]["open"], false);
    assert_eq!(failure["playback_loading"], false);
    healthy_audio(&mut app, &engine_id);

    let shown = app.click("Details");
    assert_eq!(shown["error_details"]["title"], "Error details");
    assert_eq!(
        shown["error_details"]["summary"],
        "Plugin processing failed on \"Failed synth 🎹\""
    );
    assert!(
        shown["error_details"]["background_shapes"]
            .as_u64()
            .unwrap()
            > 0
    );
    let details = shown["error_details"]["details"]
        .as_str()
        .unwrap()
        .to_owned();
    for expected in [
        "Failed synth 🎹",
        "No Editor Probe",
        "timed out",
        "48000 Hz",
    ] {
        assert!(details.contains(expected), "Missing {expected}: {details}");
    }
    assert!(details.contains(plugin.path.to_str().unwrap()));
    assert!(details.contains(&format!("ID {failed}")));
    let copied = app.click("Copy details");
    assert_eq!(copied["clipboard_text"], details);
    assert_eq!(copied["error_details"]["details"], details);
    let during_modal = healthy_audio(&mut app, &engine_id);
    assert_eq!(during_modal["error_details"]["details"], details);
    assert!(app.key("Escape")["error_details"].is_null());
    app.click("Dismiss");
    let dismissed = healthy_audio(&mut app, &engine_id);
    assert_eq!(
        dismissed["error"], false,
        "A latched failure must not reopen its notice"
    );
    assert!(dismissed["error_details"].is_null());
    app.quit();
}
