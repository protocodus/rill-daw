//! Large source files remain editable through the real source-analysis workers.
mod common;

use common::Harness;
use daw_core::{AudioData, AudioFile, Clip, Project};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    time::{Duration, Instant},
};

fn wait_analysis(app: &mut Harness) -> Value {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        assert_eq!(state["audio_editor"]["error"], "", "{state}");
        if state["audio_editor"]["pending"] == false {
            return state;
        }
        assert!(Instant::now() < until, "Analysis timed out: {state}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn delayed_fixture() -> (Harness, std::path::PathBuf, Value) {
    source_fixture(Harness::with_audio_analysis_delay(3_000))
}

fn source_fixture(mut app: Harness) -> (Harness, std::path::PathBuf, Value) {
    let source = app.directory.path().join("cancel-source.wav");
    let mut samples = vec![0.0; 8_000];
    samples[1_000..2_000].fill(0.25);
    samples[5_000..6_000].fill(0.25);
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples,
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Analysis cancellation");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Cancel window",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    let seed = app.directory.path().join("cancel-seed.rill");
    project.save(&seed).unwrap();
    let before = app.command(json!({"command":"open","path":seed}));
    open_editor(&mut app);
    (app, seed, before)
}

fn wait_silence_started(app: &mut Harness) {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let state = app.inspect();
        assert!(state["audio_silence"]["error"].is_null(), "{state}");
        if state["audio_silence"]["worker_started"] == true {
            return;
        }
        assert!(
            Instant::now() < until,
            "Replacing or closing silence did not release its worker: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn open_editor(app: &mut Harness) {
    app.click("Cancel window");
    app.click("Window");
    app.click("Clip Editor");
}

fn start_normalize(app: &mut Harness) -> Value {
    app.click("Process");
    let state = app.click("Normalize");
    assert_eq!(state["audio_editor"]["pending"], true, "{state}");
    state
}

fn cancelled_worker_stops(app: &mut Harness) -> Value {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let state = app.inspect();
        if state["audio_editor"]["analysis_running"] == false
            && state["audio_editor"]["pending"] == false
        {
            assert_eq!(state["audio_editor"]["error"], "", "{state}");
            return state;
        }
        assert!(
            Instant::now() < until,
            "Cancelled worker retained its slot: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn mouse_cancellation_and_transient_or_silence_close_release_their_worker_slots() {
    let (mut app, _, before) = delayed_fixture();
    start_normalize(&mut app);
    app.click("Cancel analysis");
    assert_eq!(
        cancelled_worker_stops(&mut app)["project"],
        before["project"]
    );
    app.click("Process");
    app.click("Transients");
    assert_eq!(app.click("Analyze")["audio_editor"]["pending"], true);
    app.click("Close transients");
    assert_eq!(
        cancelled_worker_stops(&mut app)["project"],
        before["project"]
    );
    app.click("Process");
    let started = app.click("Strip silence…");
    assert_eq!(started["audio_silence"]["worker_started"], true);
    // Reopening must explicitly cancel the displaced job; cloned Dialog has no Drop hook.
    app.click("Process");
    app.click("Strip silence…");
    wait_silence_started(&mut app);
    app.click("Cancel");
    app.click("Process");
    app.click("Strip silence…");
    wait_silence_started(&mut app);
    let canceled = app.click("Cancel");
    assert_eq!(canceled["project"], before["project"]);
    assert_eq!(canceled["history"], before["history"]);
    app.quit();
}

fn replace_managed_source(state: &Value) {
    let path = state["project"]["tracks"][0]["clips"][0]["content"]["Audio"]["path"]
        .as_str()
        .unwrap();
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![0.5; 8_000],
    }
    .write_wav(path)
    .unwrap();
}

#[test]
fn changed_source_rejects_reviewed_silence_and_transient_edits_without_history_mutation() {
    let (mut app, seed, _) = source_fixture(Harness::new());
    app.click("Process");
    app.click("Strip silence…");
    let until = Instant::now() + Duration::from_secs(5);
    let preview = loop {
        let state = app.inspect();
        assert!(state["audio_silence"]["error"].is_null(), "{state}");
        if state["audio_silence"]["pending"] == false {
            break state;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        preview["audio_silence"]["retained"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    replace_managed_source(&preview);
    let rejected = app.click("Apply strip silence");
    assert_eq!(rejected["project"], preview["project"]);
    assert_eq!(rejected["history"], preview["history"]);
    assert!(
        rejected["audio_silence"]["error"]
            .as_str()
            .unwrap()
            .contains("changed")
    );
    app.click("Cancel");
    app.command(json!({"command":"open","path":seed}));
    open_editor(&mut app);
    app.click("Process");
    app.click("Transients");
    app.click("Analyze");
    let preview = wait_analysis(&mut app);
    assert_eq!(
        preview["audio_editor"]["preview"].as_array().unwrap().len(),
        2
    );
    replace_managed_source(&preview);
    let rejected = app.click("Split transients");
    assert_eq!(rejected["project"], preview["project"]);
    assert_eq!(rejected["history"], preview["history"]);
    assert!(
        rejected["audio_editor"]["error"]
            .as_str()
            .unwrap()
            .contains("changed")
    );
    app.quit();
}

#[test]
fn source_change_after_worker_completion_rejects_normalization_before_ui_publication() {
    let (mut app, _, _) = delayed_fixture();
    let pending = start_normalize(&mut app);
    // The headless UI advances only with commands; finish the worker without a UI frame.
    std::thread::sleep(Duration::from_millis(3_300));
    replace_managed_source(&pending);
    let rejected = app.inspect();
    assert_eq!(rejected["audio_editor"]["pending"], false);
    assert_eq!(rejected["project"], pending["project"]);
    assert_eq!(rejected["history"], pending["history"]);
    assert!(
        rejected["audio_editor"]["error"]
            .as_str()
            .unwrap()
            .contains("changed")
    );
    app.quit();
}

#[test]
fn source_edits_and_document_replacement_cancel_but_navigation_keeps_valid_normalization() {
    let (mut app, seed, before) = delayed_fixture();
    let pending = start_normalize(&mut app);
    let track = pending["project"]["tracks"][0]["id"].clone();
    let clip = pending["project"]["tracks"][0]["clips"][0]["id"].clone();
    let changed = app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":pending["song_session"]["id"],"expected_revision":pending["song_session"]["revision"],"label":"Manual source edit",
        "edits":[{"op":"set_audio_gain","track":track,"clip":clip,"gain":0.5}]
    }}));
    assert_eq!(changed["tool_result"]["ok"], true, "{changed}");
    assert_eq!(
        cancelled_worker_stops(&mut app)["project"],
        changed["project"]
    );
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    let pending = start_normalize(&mut app);
    let renamed = app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":pending["song_session"]["id"],"expected_revision":pending["song_session"]["revision"],"label":"Cosmetic document edit",
        "edits":[{"op":"rename_track","track":track,"name":"Changed title"}]
    }}));
    assert_eq!(renamed["tool_result"]["ok"], true, "{renamed}");
    assert_eq!(
        cancelled_worker_stops(&mut app)["project"],
        renamed["project"]
    );
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    start_normalize(&mut app);
    let reopened = app.command(json!({"command":"open","path":seed}));
    assert_ne!(reopened["song_session"]["id"], before["song_session"]["id"]);
    open_editor(&mut app);
    cancelled_worker_stops(&mut app);
    start_normalize(&mut app);
    app.click("Window");
    assert_eq!(app.click("Mixer")["view"], "Mixer");
    let until = Instant::now() + Duration::from_secs(8);
    let normalized = loop {
        let state = app.inspect();
        let gain = state["project"]["tracks"][0]["clips"][0]["content"]["Audio"]["gain"]
            .as_f64()
            .unwrap();
        if gain != 1.0 {
            assert!((gain - 10_f64.powf(-1.0 / 20.0) / 0.25).abs() < 1e-6);
            break state;
        }
        assert!(
            Instant::now() < until,
            "Navigation stranded valid normalization: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(normalized["view"], "Mixer");
    assert_eq!(normalized["history"]["undo"].as_array().unwrap().len(), 1);
    let restored = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(restored["project"], reopened["project"]);
    app.quit();
}

#[test]
fn far_clip_on_large_parent_supports_selected_ai_analysis_and_mouse_editing_without_full_decode() {
    let mut app = Harness::new();
    let source = app.directory.path().join("long-parent.wav");
    let frames = 17_000_000_usize;
    let bytes = (frames * 4) as u32;
    let first = frames - 32_000;
    let mut file = File::create(&source).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(bytes + 36).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&3_u16.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&8_000_u32.to_le_bytes()).unwrap();
    file.write_all(&32_000_u32.to_le_bytes()).unwrap();
    file.write_all(&4_u16.to_le_bytes()).unwrap();
    file.write_all(&32_u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&bytes.to_le_bytes()).unwrap();
    file.set_len(u64::from(bytes) + 44).unwrap();
    for (frame, amplitude) in [(first + 1_000, 0.25_f32), (first + 20_000, 0.75)] {
        file.seek(SeekFrom::Start(44 + frame as u64 * 4)).unwrap();
        for _ in 0..800 {
            file.write_all(&amplitude.to_le_bytes()).unwrap();
        }
    }
    file.sync_all().unwrap();
    drop(file);
    let descriptor = AudioFile::open(&source).unwrap();
    assert!(descriptor.info().decoded_bytes > 64 * 1024 * 1024);
    let mut project = Project::new();
    let track = project.add_audio_track("Long source editing");
    let mut clip = Clip::audio("Far window", 0.0, 8.0, source.to_string_lossy());
    clip.offset = first as f64 / 4_000.0;
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let seed = app.directory.path().join("external-legacy-fixture.json");
    // Preserve a sparse external test source; exercise legacy loading, not bundle copying.
    project.save_legacy_json(&seed).unwrap();
    let before = app.command(json!({"command":"open","path":seed}));
    let measured = app.command(
        json!({"command":"song_tool","name":"audio_analyze_clip","arguments":{
            "track_id":track,"clip_id":clip_id,"start_beat":0.0,"end_beat":2.0
        }}),
    );
    assert_eq!(measured["tool_result"]["ok"], true, "{measured}");
    let report = &measured["tool_result"]["result"];
    assert_eq!(report["source"]["frames"], frames);
    assert_eq!(
        report["transients"]["clip_relative_beats"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        (report["normalization"]["source_peak_linear"]
            .as_f64()
            .unwrap()
            - 0.75)
            .abs()
            < 1e-6
    );
    assert_eq!(measured["project"], before["project"]);
    assert_eq!(measured["history"], before["history"]);

    app.click("Far window");
    app.click("Window");
    app.click("Clip Editor");
    app.click("Process");
    app.click("Normalize");
    let normalized = wait_analysis(&mut app);
    let gain = normalized["project"]["tracks"][0]["clips"][0]["content"]["Audio"]["gain"]
        .as_f64()
        .unwrap();
    assert!((gain - 10_f64.powf(-1.0 / 20.0) / 0.75).abs() < 1e-6);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    app.click("Process");
    app.click("Transients");
    app.click("Analyze");
    let transients = wait_analysis(&mut app);
    assert_eq!(
        transients["audio_editor"]["preview"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(transients["project"], before["project"]);
    app.click("Close transients");
    app.click("Process");
    app.click("Strip silence…");
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        assert!(state["audio_silence"]["error"].is_null(), "{state}");
        if state["audio_silence"]["pending"] == false {
            assert_eq!(
                state["audio_silence"]["retained"].as_array().unwrap().len(),
                2
            );
            break;
        }
        assert!(
            Instant::now() < until,
            "Silence analysis timed out: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.click("Cancel")["project"], before["project"]);
    descriptor.check_unchanged().unwrap();
    app.quit();
}
