//! Range-selection gestures and future cuts must not seek a running transport.
//! The virtual device consumes the real renderer queue only when explicitly pumped.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const FRAMES_PER_BEAT: usize = 24_000;

fn wait(app: &mut Harness, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if ready(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "Timed out: {state}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn source_frame(frame: usize) -> [f32; 2] {
    // Exactly representable, distinct samples expose repeats, skips, swaps and
    // unexpected silence more precisely than a constant or periodic tone.
    let ramp = frame as f32 / 16_777_216.0;
    [0.125 + ramp, -0.25 + ramp]
}

fn assert_same_transport(state: &Value, before: &Value) {
    for field in [
        "engine_id",
        "generation",
        "underruns",
        "render_errors",
        "edit_errors",
    ] {
        assert_eq!(
            state["engine"][field], before["engine"][field],
            "{field} changed during audio editing"
        );
    }
    assert_eq!(state["engine"]["playing"], true);
    assert_eq!(state["playback_loading"], false);
}

fn pump_and_check(app: &mut Harness, before: &Value, start: usize, end: usize, cut: bool) -> Value {
    let mut cursor = start;
    let mut state = before.clone();
    while cursor < end {
        let count = (end - cursor).min(8192);
        state = app.command(json!({"command":"pump_audio", "frames":count}));
        assert_same_transport(&state, before);
        let samples = state["audio_output"].as_array().unwrap();
        assert_eq!(samples.len(), count);
        for (offset, stereo) in samples.iter().enumerate() {
            let frame = cursor + offset;
            let expected = if cut && (FRAMES_PER_BEAT * 3..FRAMES_PER_BEAT * 4).contains(&frame) {
                [0.0; 2]
            } else {
                source_frame(frame)
            };
            for channel in 0..2 {
                let actual = stereo[channel].as_f64().unwrap() as f32;
                assert!(
                    (actual - expected[channel]).abs() < 1e-7,
                    "PCM discontinuity at frame {frame}, channel {channel}: {actual} != {}",
                    expected[channel]
                );
            }
        }
        cursor += count;
        assert!(
            (state["engine"]["position_beats"].as_f64().unwrap()
                - cursor as f64 / FRAMES_PER_BEAT as f64)
                .abs()
                < 1e-8,
            "Playhead lost sample alignment: {}",
            state["engine"]
        );
    }
    state
}

#[test]
fn selecting_copying_and_cutting_future_audio_preserve_running_transport_and_exact_pcm() {
    let mut app = Harness::with_virtual_audio(None, None);
    let path = app.directory.path().join("continuous-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..FRAMES_PER_BEAT * 8).flat_map(source_frame).collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Continuous audio");
    let owner = project.track_mut(track).unwrap();
    owner.gain = 1.0;
    owner.clips.push(Clip::audio(
        "Continuous source",
        0.0,
        8.0,
        path.to_string_lossy(),
    ));
    let project_path = app.directory.path().join("continuous-playback.rill");
    project.save(&project_path).unwrap();
    app.command(json!({"command":"open", "path":project_path}));
    app.command(json!({"command":"viewport", "width":1400, "height":900}));
    app.click("Continuous source");
    app.click("Window");
    app.click("Clip Editor");
    app.click("Fit");
    app.key("P");
    app.click("Play");
    let started = wait(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    let before = pump_and_check(&mut app, &started, 0, FRAMES_PER_BEAT, false);

    let view = &before["audio_editor"];
    let x_at = |beat: f64| {
        view["rect"][0].as_f64().unwrap()
            + (beat - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap()
    };
    let y = (view["rect"][1].as_f64().unwrap() + view["rect"][3].as_f64().unwrap()) * 0.5;
    let selected = app.command(json!({
        "command":"pointer", "phase":"drag", "x":x_at(3.0), "y":y,
        "to_x":x_at(4.0), "to_y":y
    }));
    assert_eq!(selected["audio_editor"]["range"], json!([3.0, 4.0]));
    assert_same_transport(&selected, &before);
    assert_eq!(
        selected["engine"]["position_beats"], before["engine"]["position_beats"],
        "Selecting a future edit range must not move the playing transport"
    );
    assert_eq!(selected["project"], before["project"]);

    let copied = app.command(json!({"command":"key", "key":"C", "cmd":true}));
    assert_same_transport(&copied, &before);
    assert_eq!(copied["project"], before["project"]);
    assert_eq!(
        copied["engine"]["position_beats"],
        before["engine"]["position_beats"]
    );
    let cut = app.command(json!({"command":"key", "key":"X", "cmd":true}));
    assert_same_transport(&cut, &before);
    assert_eq!(
        cut["engine"]["position_beats"],
        before["engine"]["position_beats"]
    );
    assert_eq!(
        cut["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let applied = wait(&mut app, |state| {
        state["engine"]["applied_revision"] == state["revision"]
    });
    assert_same_transport(&applied, &before);
    // Includes every sample leading into, throughout and following the cut.
    // The future edit is well beyond the bounded pre-edit render lookahead.
    pump_and_check(
        &mut app,
        &before,
        FRAMES_PER_BEAT,
        FRAMES_PER_BEAT * 5,
        true,
    );
    app.quit();
}
