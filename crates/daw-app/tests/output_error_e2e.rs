//! Device failures stay distinct from plugin failures in the running app.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

fn ready(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if state["engine"]["playing"] == true && state["playback_loading"] == false {
            return state;
        }
        assert!(Instant::now() < deadline, "{state}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn audible(app: &mut Harness) {
    let state = app.command(json!({"command":"pump_audio", "frames":512}));
    assert_eq!(state["engine"]["render_errors"], 0);
    assert_eq!(state["engine"]["underruns"], 0);
    assert!(
        state["audio_output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| {
                (frame[0].as_f64().unwrap() - 0.4).abs() < 1e-6
                    && (frame[1].as_f64().unwrap() - 0.2).abs() < 1e-6
            })
    );
}

#[test]
fn playback_device_failure_keeps_backend_details_and_recovers_with_stop_and_play() {
    let mut app = Harness::with_virtual_audio(None, None);
    let wav = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [0.4, 0.2]).collect(),
    }
    .write_wav(&wav)
    .unwrap();
    let mut project = Project::new();
    let id = project.add_audio_track("Playback only");
    project.track_mut(id).unwrap().gain = 1.0;
    project.track_mut(id).unwrap().clips.push(Clip::audio(
        "Sound",
        0.0,
        2.0,
        wav.to_string_lossy(),
    ));
    let path = app.directory.path().join("Output error.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Window");
    app.click("Recording");
    // New audio tracks already select Mono 1; arm that visible default route.
    let armed = app.click("Arm recording");
    assert_eq!(
        armed["project"]["tracks"][0]["audio_input"],
        json!({"Mono":{"channel":0}})
    );
    assert!(
        armed["recording"]["armed"]
            .as_array()
            .unwrap()
            .contains(&json!(id))
    );
    app.key("Escape");
    app.click("Play");
    let initial = ready(&mut app);
    audible(&mut app);
    let failed = app.command(json!({"command":"output_error"}));
    assert_eq!(failed["engine"]["playing"], false);
    assert_eq!(failed["engine"]["output_failed"], true);
    assert_eq!(failed["engine"]["render_errors"], 0);
    assert!(
        failed["status"]
            .as_str()
            .unwrap()
            .contains("Audio output stream failed")
    );
    let shown = app.click("Details");
    let detail = shown["error_details"]["details"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(detail.contains("Virtual output disconnected"));
    assert!(detail.contains("Retry playback or press Play"));
    assert!(!detail.contains("Plug-in/render"));
    assert!(!detail.contains("MIDI recording"));
    assert!(
        failed["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Audio setup…")
    );
    assert_eq!(app.click("Copy details")["clipboard_text"], detail);
    app.click("Close");
    app.click("Dismiss");
    let duplicate = app.command(json!({"command":"output_error"}));
    assert_eq!(
        duplicate["error"], false,
        "A latched device error must not reappear after dismissal"
    );
    assert_eq!(duplicate["engine"]["render_errors"], 0);
    let retry = app.click("Record");
    assert_eq!(retry["recording"]["phase"], "idle");
    assert_eq!(retry["engine"]["engine_id"], initial["engine"]["engine_id"]);
    assert_eq!(retry["engine"]["output_failed"], true);
    assert_eq!(retry["engine"]["playing"], false);
    assert!(
        retry["status"]
            .as_str()
            .unwrap()
            .contains("Retry playback or press Play")
    );
    let silent = app.command(json!({"command":"pump_audio", "frames":512}));
    assert!(
        silent["audio_output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|frame| { frame[0].as_f64() == Some(0.0) && frame[1].as_f64() == Some(0.0) })
    );
    app.click("Dismiss");
    app.click("Stop");
    app.click("Play");
    let resumed = ready(&mut app);
    assert_ne!(
        initial["engine"]["engine_id"],
        resumed["engine"]["engine_id"]
    );
    assert_eq!(resumed["engine"]["output_failed"], false);
    audible(&mut app);
    app.click("Stop");
    app.quit();
}

fn has_dropout_notice(state: &Value) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| {
            label
                .as_str()
                .is_some_and(|label| label.starts_with("Audio output dropout ·"))
        })
}

#[test]
fn recovered_output_xrun_keeps_playback_and_shows_one_nonfatal_notice_per_stream() {
    let mut app = Harness::with_virtual_audio(None, None);
    let wav = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [0.4, 0.2]).collect(),
    }
    .write_wav(&wav)
    .unwrap();
    let mut project = Project::new();
    let id = project.add_audio_track("Continuous playback");
    project.track_mut(id).unwrap().gain = 1.0;
    project.track_mut(id).unwrap().clips.push(Clip::audio(
        "Sound",
        0.0,
        2.0,
        wav.to_string_lossy(),
    ));
    let path = app.directory.path().join("Output dropout.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Mixer");
    app.click("Play");
    let initial = ready(&mut app);
    audible(&mut app);
    let dropout = app.command(json!({"command":"output_xrun"}));
    assert_eq!(dropout["error"], false);
    assert_eq!(
        dropout["engine"]["engine_id"],
        initial["engine"]["engine_id"]
    );
    assert_eq!(dropout["engine"]["playing"], true);
    assert_eq!(dropout["engine"]["output_failed"], false);
    assert_eq!(dropout["engine"]["output_dropouts"], 1);
    assert_eq!(dropout["engine"]["render_errors"], 0);
    assert!(dropout["last_error_report"].is_null());
    assert!(has_dropout_notice(&dropout));
    assert!(!visible(&dropout, "Retry playback"));
    audible(&mut app);
    let painted = app.inspect();
    assert!((painted["mixer"]["master_peaks"][0].as_f64().unwrap() - 0.4).abs() < 1e-6);
    assert_eq!(app.click("Audio setup…")["popups"]["setup"], true);
    app.key("Escape");
    let repeated = app.command(json!({"command":"output_xrun"}));
    assert_eq!(repeated["engine"]["output_dropouts"], 2);
    assert!(
        repeated["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Audio output dropout · 2 · Try a larger buffer in Audio setup")
    );
    app.click("Dismiss");
    let dismissed = app.command(json!({"command":"output_xrun"}));
    assert_eq!(dismissed["engine"]["output_dropouts"], 3);
    assert_eq!(dismissed["error"], false);
    assert!(!has_dropout_notice(&dismissed));
    audible(&mut app);
    app.click("Pause");
    let paused = app.command(json!({"command":"output_xrun"}));
    assert_eq!(
        paused["engine"]["playing"], false,
        "Xrun must not start a paused transport"
    );
    assert_eq!(paused["engine"]["output_failed"], false);
    app.click("Play");
    audible(&mut app);
    app.click("Stop");
    app.click("Play");
    let restarted = ready(&mut app);
    assert_ne!(
        restarted["engine"]["engine_id"],
        initial["engine"]["engine_id"]
    );
    let new_stream = app.command(json!({"command":"output_xrun"}));
    assert!(has_dropout_notice(&new_stream));
    assert_eq!(new_stream["engine"]["output_dropouts"], 1);
    audible(&mut app);
    let fatal = app.command(json!({"command":"output_error"}));
    assert_eq!(fatal["error"], true);
    assert_eq!(fatal["engine"]["output_failed"], true);
    assert_eq!(fatal["engine"]["playing"], false);
    assert_eq!(fatal["engine"]["render_errors"], 0);
    assert!(!has_dropout_notice(&fatal));
    assert!(
        fatal["last_error_report"]["details"]
            .as_str()
            .unwrap()
            .contains("Virtual output disconnected")
    );
    app.click("Stop");
    app.quit();
}

#[test]
fn input_and_unrelated_failures_do_not_reuse_the_output_retry_action() {
    let mut app = Harness::with_virtual_audio(None, None);
    app.click("Play");
    let initial = ready(&mut app);
    let input = app.command(json!({"command":"input_error"}));
    assert_eq!(input["engine"]["output_failed"], false);
    assert_eq!(input["engine"]["engine_id"], initial["engine"]["engine_id"]);
    assert!(
        input["status"]
            .as_str()
            .unwrap()
            .contains("Audio input stream failed")
    );
    assert!(!visible(&input, "Retry playback"));
    app.click("Dismiss");
    app.click("Stop");
    app.click("Play");
    ready(&mut app);
    let output = app.command(json!({"command":"output_error"}));
    assert!(visible(&output, "Retry playback"));

    // A different operation's failure replaces the current notice even while
    // the old output is still failed. Its details must not expose a stale retry.
    let destination = app.directory.path().join("not-a-file.wav");
    std::fs::create_dir(&destination).unwrap();
    app.raw_command(json!({"command":"export", "path":destination}));
    app.raw_command(json!({"command":"wait_export"}));
    let unrelated = app.inspect();
    assert_eq!(unrelated["error"], true);
    assert!(!visible(&unrelated, "Retry playback"));
    app.click("Dismiss");
    let dismissed = app.inspect();
    assert!(!visible(&dismissed, "Retry playback"));
    app.click("Stop");
    app.quit();
}

#[test]
fn explicit_retry_reopens_failed_output_at_the_same_position_and_play_still_recovers_after_setup() {
    let mut app = Harness::with_virtual_audio(None, None);
    let wav = app.directory.path().join("position-sensitive.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000)
            .flat_map(|frame| {
                if frame < 4096 {
                    [0.1, 0.05]
                } else {
                    [0.4, 0.2]
                }
            })
            .collect(),
    }
    .write_wav(&wav)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Position-sensitive playback");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Changing level",
        0.0,
        2.0,
        wav.to_string_lossy(),
    ));
    let path = app.directory.path().join("Explicit output retry.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Play");
    ready(&mut app);
    app.command(json!({"command":"pump_audio", "frames":8192}));
    for (action, apply_unchanged) in [
        ("Retry playback", false),
        ("Retry playback", false),
        ("Play", true),
    ] {
        let failed = app.command(json!({"command":"output_error"}));
        assert_eq!(failed["engine"]["output_failed"], true);
        assert!(visible(&failed, "Retry playback"));
        let position = failed["engine"]["position_beats"].as_f64().unwrap();
        assert!(position > 0.3);
        if apply_unchanged {
            app.click("Audio setup…");
            app.click("Refresh devices");
            let applied = app.click("Apply");
            assert_eq!(applied["error"], false);
            assert_eq!(
                applied["engine"]["engine_id"],
                failed["engine"]["engine_id"]
            );
            assert_eq!(applied["engine"]["output_failed"], true);
        }
        app.click(action);
        let resumed = ready(&mut app);
        assert_eq!(resumed["error"], false);
        assert!(!visible(&resumed, "Retry playback"));
        assert_ne!(
            resumed["engine"]["engine_id"],
            failed["engine"]["engine_id"]
        );
        assert_eq!(resumed["engine"]["output_failed"], false);
        assert!((resumed["engine"]["position_beats"].as_f64().unwrap() - position).abs() < 1e-9);
        for key in [
            "project",
            "history",
            "revision",
            "song_session",
            "project_path",
            "selected_track",
        ] {
            assert_eq!(resumed[key], failed[key], "Retry changed {key}");
        }
        // Restarting at zero would produce [0.1,0.05] and fail this real-PCM check.
        audible(&mut app);
    }
    app.click("Stop");
    app.quit();
}
