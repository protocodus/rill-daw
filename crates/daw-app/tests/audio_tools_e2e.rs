//! High-level audio tools execute analysis and publish one ordinary song edit.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn fixture(pulses: bool) -> (Harness, Value, std::path::PathBuf, Vec<u8>) {
    let mut app = Harness::new();
    let source = app.directory.path().join("tool-source.wav");
    let samples = (0..48_000)
        .flat_map(|frame| {
            let value = if pulses {
                if (6_000..12_000).contains(&frame) || (30_000..36_000).contains(&frame) {
                    0.25
                } else {
                    0.0
                }
            } else {
                (std::f64::consts::TAU * 1_000.0 * f64::from(frame) / 48_000.0).sin() as f32 * 0.25
            };
            [value, value]
        })
        .collect();
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples,
    }
    .write_wav(&source)
    .unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Audio tool fixture");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Source",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("audio-tool.rill");
    project.save(&path).unwrap();
    let state = app.command(json!({"command":"open","path":path}));
    (app, state, source, original_bytes)
}

fn process_args(state: &Value, operation: &str) -> Value {
    json!({
        "session_id":state["song_session"]["id"],
        "expected_revision":state["song_session"]["revision"],
        "track_id":state["project"]["tracks"][0]["id"],
        "clip_id":state["project"]["tracks"][0]["clips"][0]["id"],
        "operation":operation
    })
}

fn process(app: &mut Harness, args: Value) -> Value {
    app.command(json!({"command":"song_tool","name":"audio_process_clip","arguments":args}))
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn normalize_tool_changes_actual_output_and_undo_restores_audio_in_one_step() {
    let (mut app, before, source, source_bytes) = fixture(false);
    let baseline = export(&mut app, "baseline.wav");
    let mut args = process_args(&before, "normalize");
    args["normalize_target_db"] = json!(-6.0);
    let normalized = process(&mut app, args);
    assert_eq!(normalized["tool_result"]["ok"], true, "{normalized}");
    assert_eq!(normalized["history"]["undo"].as_array().unwrap().len(), 1);
    let gain = normalized["project"]["tracks"][0]["clips"][0]["content"]["Audio"]["gain"]
        .as_f64()
        .unwrap();
    assert!((gain - 10_f64.powf(-6.0 / 20.0) / 0.25).abs() < 1e-6);
    let rendered = export(&mut app, "normalized.wav");
    let peak = rendered
        .samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0_f32, f32::max);
    assert!(
        (f64::from(peak) - 10_f64.powf(-6.0 / 20.0)).abs() < 1e-5,
        "peak: {peak}"
    );
    for (&before, &after) in baseline.samples.iter().zip(&rendered.samples) {
        assert!((f64::from(after) - f64::from(before) * gain).abs() < 1e-6);
    }
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    assert!(undone["history"]["undo"].as_array().unwrap().is_empty());
    assert_eq!(export(&mut app, "undone.wav").samples, baseline.samples);
    assert_eq!(std::fs::read(source).unwrap(), source_bytes);
    app.quit();
}

#[test]
fn analysis_backed_clip_cuts_are_each_one_undoable_action() {
    let (mut app, before, source, source_bytes) = fixture(true);
    let baseline = export(&mut app, "pulses-before.wav");
    let split = process(&mut app, process_args(&before, "split_transients"));
    assert_eq!(split["tool_result"]["ok"], true, "{split}");
    assert_eq!(
        split["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(split["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(export(&mut app, "split.wav").samples, baseline.samples);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    let stripped = process(&mut app, process_args(&undone, "strip_silence"));
    assert_eq!(stripped["tool_result"]["ok"], true, "{stripped}");
    assert_eq!(
        stripped["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(stripped["history"]["undo"].as_array().unwrap().len(), 1);
    let clips = stripped["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap();
    let kept_length: f64 = clips
        .iter()
        .map(|clip| clip["length"].as_f64().unwrap())
        .sum();
    assert!(
        kept_length < 1.0,
        "retained duration in beats: {kept_length}"
    );
    let restored = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(restored["project"], before["project"]);
    assert_eq!(std::fs::read(source).unwrap(), source_bytes);
    app.quit();
}

#[test]
fn invalid_or_stale_audio_process_requests_leave_project_and_history_unchanged() {
    let (mut app, before, source, source_bytes) = fixture(false);
    for (field, value) in [
        ("normalize_target_db", json!(4.0)),
        ("path", json!("/forbidden/source.wav")),
        ("operation", json!("erase_file")),
    ] {
        let mut args = process_args(&before, "normalize");
        args[field] = value;
        let rejected = process(&mut app, args);
        assert_eq!(rejected["tool_result"]["ok"], false, "{rejected}");
        assert_eq!(rejected["project"], before["project"]);
        assert_eq!(rejected["history"], before["history"]);
        assert_eq!(rejected["song_session"], before["song_session"]);
    }
    let changed = process(&mut app, process_args(&before, "normalize"));
    assert_eq!(changed["tool_result"]["ok"], true, "{changed}");
    let stale = process(&mut app, process_args(&before, "normalize"));
    assert_eq!(stale["tool_result"]["ok"], false, "{stale}");
    assert_eq!(stale["tool_result"]["error"]["code"], "conflict");
    assert_eq!(stale["project"], changed["project"]);
    assert_eq!(stale["history"], changed["history"]);
    assert_eq!(std::fs::read(source).unwrap(), source_bytes);
    app.quit();
}
