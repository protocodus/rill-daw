//! Bulk recording tools use the real capture lifecycle without opening a view.
mod common;

use common::Harness;
use daw_core::{AudioData, ClipContent, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"song_tool","name":name,"arguments":arguments}))
}

fn session(state: &Value) -> (Value, Value) {
    (
        state["song_session"]["id"].clone(),
        state["song_session"]["revision"].clone(),
    )
}

fn configure(app: &mut Harness, tracks: Value) -> Value {
    let state = app.inspect();
    let (id, revision) = session(&state);
    tool(
        app,
        "configure_recording",
        json!({
            "session_id":id,"expected_revision":revision,"tracks":tracks,
            "timing":{"precount_bars":0,"click_enabled":false,"click_gain":0.3}
        }),
    )
}

fn fixture() -> (Harness, u64, u64, u64) {
    let mut app = Harness::with_virtual_audio(None, None);
    let state = app.inspect();
    let (id, revision) = session(&state);
    let state = tool(
        &mut app,
        "apply_edits",
        json!({
            "session_id":id,"expected_revision":revision,"label":"Create capture tracks",
            "edits":[
                {"op":"add_track","kind":"audio","name":"Mono capture","key":"mono"},
                {"op":"add_track","kind":"audio","name":"Stereo capture","key":"stereo"},
                {"op":"add_track","kind":"bus","name":"Capture bus","key":"bus"}
            ]
        }),
    );
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    let created = state["tool_result"]["result"]["created"]
        .as_array()
        .unwrap();
    let id = |key: &str| {
        created.iter().find(|entity| entity["key"] == key).unwrap()["id"]
            .as_u64()
            .unwrap()
    };
    let (mono, stereo, bus) = (id("mono"), id("stereo"), id("bus"));
    (app, mono, stereo, bus)
}

fn wait(app: &mut Harness, phase: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = app.inspect();
        if state["recording"]["phase"] == phase {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Recording did not reach {phase}: {state}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn project(state: &Value) -> Project {
    serde_json::from_value(state["project"].clone()).unwrap()
}

#[test]
fn bulk_configure_capture_two_tracks_stop_and_undo_without_opening_recording() {
    let (mut app, mono, stereo, _) = fixture();
    let before = app.inspect();
    let state = configure(
        &mut app,
        json!([
            {"track_id":mono,"input":{"kind":"audio_mono","channel":1},"armed":true,"monitor":"off"},
            {"track_id":stereo,"input":{"kind":"audio_stereo","left":0,"right":1},"armed":true,"monitor":"off"}
        ]),
    );
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    assert_eq!(state["tool_result"]["result"]["phase"], "idle");
    assert_eq!(state["selected_track"], before["selected_track"]);
    assert_eq!(state["view"], before["view"]);

    let state = tool(&mut app, "get_recording", json!({}));
    assert_eq!(
        state["tool_result"]["result"]["armed"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (id, revision) = session(&state);
    let started = tool(
        &mut app,
        "start_recording",
        json!({"session_id":id,"expected_revision":revision}),
    );
    assert_eq!(started["tool_result"]["ok"], true, "{started}");
    wait(&mut app, "recording");
    let busy = configure(&mut app, json!([{"track_id":mono,"armed":false}]));
    assert_eq!(busy["tool_result"]["ok"], false);
    assert_eq!(busy["tool_result"]["error"]["code"], "busy");
    let samples: Vec<f32> = (0..1024).flat_map(|_| [0.125, -0.25]).collect();
    app.command(json!({"command":"pump_input","channels":2,"samples":samples}));
    let stopped = tool(&mut app, "stop_recording", json!({"after":"continue"}));
    assert_eq!(stopped["tool_result"]["ok"], true, "{stopped}");
    let state = wait(&mut app, "idle");
    assert_eq!(state["selected_track"], before["selected_track"]);
    assert_eq!(state["view"], before["view"]);
    let recorded = project(&state);
    for (track_id, channels) in [(mono, 1), (stereo, 2)] {
        let track = recorded.track(track_id).unwrap();
        assert_eq!(
            track
                .clips
                .iter()
                .filter(|clip| !clip.is_take_source())
                .count(),
            1
        );
        let clip = track
            .clips
            .iter()
            .find(|clip| !clip.is_take_source())
            .unwrap();
        let ClipContent::Audio { path, .. } = &clip.content else {
            panic!("Expected captured audio");
        };
        let audio = AudioData::load_wav(path).unwrap();
        assert_eq!(audio.channels, channels);
        assert_eq!(audio.samples.len(), 1024 * channels);
        for frame in audio.samples.chunks_exact(channels) {
            if channels == 1 {
                assert!((frame[0] + 0.25).abs() < 1e-6);
            } else {
                assert!((frame[0] - 0.125).abs() < 1e-6);
                assert!((frame[1] + 0.25).abs() < 1e-6);
            }
        }
    }
    let state = app.command(json!({"command":"key","key":"Z","cmd":true}));
    let restored = project(&state);
    assert!(restored.track(mono).unwrap().clips.is_empty());
    assert!(restored.track(stereo).unwrap().clips.is_empty());
    assert_eq!(
        restored.track(mono).unwrap().audio_input,
        Some(daw_core::AudioInputChannels::Mono { channel: 1 })
    );
    app.quit();
}

#[test]
fn invalid_bulk_configuration_is_atomic_and_stale_configuration_is_rejected() {
    let (mut app, mono, stereo, bus) = fixture();
    let before = app.inspect();
    let failed = configure(
        &mut app,
        json!([
            {"track_id":mono,"input":{"kind":"audio_mono","channel":1},"armed":true,"monitor":"on"},
            {"track_id":bus,"armed":true}
        ]),
    );
    assert_eq!(failed["tool_result"]["ok"], false);
    assert_eq!(failed["tool_result"]["error"]["code"], "invalid_arguments");
    assert_eq!(failed["project"], before["project"]);
    assert_eq!(failed["recording"]["armed"], before["recording"]["armed"]);
    assert_eq!(
        failed["recording"]["monitors"],
        before["recording"]["monitors"]
    );
    let state = configure(
        &mut app,
        json!([
            {"track_id":mono,"input":{"kind":"audio_mono","channel":1},"armed":true},
            {"track_id":stereo,"input":{"kind":"audio_stereo","left":0,"right":1},"monitor":"auto"}
        ]),
    );
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    let (old_id, old_revision) = session(&before);
    let stale = tool(
        &mut app,
        "configure_recording",
        json!({
            "session_id":old_id,"expected_revision":old_revision,
            "tracks":[{"track_id":mono,"armed":false}]
        }),
    );
    assert_eq!(stale["tool_result"]["error"]["code"], "conflict");
    assert_eq!(stale["recording"]["armed"], state["recording"]["armed"]);
    let cleared = configure(
        &mut app,
        json!([
            {"track_id":mono,"input":{"kind":"none"},"armed":false,"monitor":"off"}
        ]),
    );
    assert_eq!(cleared["tool_result"]["ok"], true, "{cleared}");
    assert_eq!(project(&cleared).track(mono).unwrap().audio_input, None);
    assert_eq!(cleared["recording"]["phase"], "idle");
    app.quit();
}
