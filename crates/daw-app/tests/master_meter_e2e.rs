//! The running mixer reads the production output callback's measured PCM.
//! A virtual sink controls sample consumption without ALSA null's unpaced clock.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn wait_for_engine(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        assert_eq!(state["error"], false, "{state}");
        if state["engine"]["playing"] == true && state["playback_loading"] == false {
            return state;
        }
        assert!(Instant::now() < deadline, "No running engine: {state}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn stereo(value: &Value) -> [f64; 2] {
    [value[0].as_f64().unwrap(), value[1].as_f64().unwrap()]
}

fn near(actual: [f64; 2], expected: [f64; 2]) {
    for channel in 0..2 {
        assert!(
            (actual[channel] - expected[channel]).abs() < 2e-6,
            "{actual:?} != {expected:?}"
        );
    }
}

fn master(state: &Value) -> [f64; 2] {
    let displayed = stereo(&state["mixer"]["master_peaks"]);
    near(
        displayed,
        [
            state["engine"]["peak_left"].as_f64().unwrap(),
            state["engine"]["peak_right"].as_f64().unwrap(),
        ],
    );
    displayed
}

fn channel(state: &Value, id: u64) -> [f64; 2] {
    stereo(
        &state["mixer"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["track"] == id)
            .unwrap()["peaks"],
    )
}

fn pump(app: &mut Harness, frames: usize) -> Value {
    let state = app.command(json!({"command":"pump_audio", "frames":frames}));
    assert_eq!(state["engine"]["underruns"], 0);
    assert_eq!(state["engine"]["render_errors"], 0);
    assert_eq!(state["audio_output"].as_array().unwrap().len(), frames);
    state
}

#[test]
fn master_and_channel_meters_match_actual_bus_send_and_muted_mix_pcm() {
    let mut app = Harness::with_virtual_audio(None, None);
    let source = app.directory.path().join("Stereo source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [0.8, 0.4]).collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = true;
    project.loop_start = 0.0;
    project.loop_end = 2.0;
    let lead = project.add_audio_track("Lead");
    let bass = project.add_audio_track("Bass");
    let muted = project.add_audio_track("Muted");
    let bus = project.add_bus("Group");
    let send = project.add_bus("Pre-fader return");
    for id in [lead, bass, muted] {
        project.track_mut(id).unwrap().clips.push(Clip::audio(
            "Source",
            0.0,
            2.0,
            source.to_string_lossy(),
        ));
    }
    project.track_mut(lead).unwrap().gain = 0.5;
    project.track_mut(lead).unwrap().pan = -0.5;
    project.track_mut(bass).unwrap().gain = 0.25;
    project.track_mut(bass).unwrap().pan = 1.0;
    project.track_mut(muted).unwrap().mute = true;
    project.track_mut(bus).unwrap().gain = 0.5;
    project.track_mut(bus).unwrap().pan = -0.25;
    project.track_mut(send).unwrap().gain = 0.5;
    project.set_output(lead, Some(bus)).unwrap();
    project.set_output(bass, Some(bus)).unwrap();
    project.add_send(lead, send, 0.25, true).unwrap();
    let path = app.directory.path().join("Master meters.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1440, "height":1000}));
    app.click("Mixer");
    app.click("Play");
    let ready = wait_for_engine(&mut app);
    let state = pump(&mut app, 512);
    assert_eq!(state["engine"]["engine_id"], ready["engine"]["engine_id"]);
    for sample in state["audio_output"].as_array().unwrap() {
        near(stereo(sample), [0.3, 0.125]);
    }
    near(master(&state), [0.3, 0.125]);
    near(channel(&state, lead), [0.4, 0.1]);
    near(channel(&state, bass), [0.0, 0.1]);
    near(channel(&state, bus), [0.2, 0.075]);
    near(channel(&state, send), [0.1, 0.05]);
    near(channel(&state, muted), [0.0, 0.0]);
    assert_eq!(state["engine"]["consumed_frames"], 512);

    // Pause must clear the displayed meter even when a deterministic sink is
    // idle; a subsequent callback may still publish live input monitoring.
    let paused = app.click("Pause");
    near(master(&paused), [0.0, 0.0]);
    for id in [lead, bass, bus, send, muted] {
        near(channel(&paused, id), [0.0, 0.0]);
    }
    app.click("Stop");
    app.command(json!({"command":"click", "label":"Mute", "occurrence":0}));
    app.click("Play");
    wait_for_engine(&mut app);
    let state = pump(&mut app, 512);
    for sample in state["audio_output"].as_array().unwrap() {
        near(stereo(sample), [0.0, 0.0375]);
    }
    near(master(&state), [0.0, 0.0375]);
    near(channel(&state, lead), [0.0, 0.0]);
    near(channel(&state, send), [0.0, 0.0]);
    near(channel(&state, bus), [0.0, 0.0375]);
    app.click("Stop");
    app.quit();
}

#[test]
fn master_keeps_a_short_transient_visible_after_the_next_silent_callback() {
    let mut app = Harness::with_virtual_audio(None, None);
    let source = app.directory.path().join("Short transient.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000)
            .flat_map(|frame| if frame < 256 { [0.4, 0.2] } else { [0.0; 2] })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = false;
    let track = project.add_audio_track("Transient");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Hit",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("Transient meters.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Mixer");
    app.click("Play");
    wait_for_engine(&mut app);
    let hit = pump(&mut app, 256);
    near(master(&hit), [0.4, 0.2]);
    assert!(
        hit["audio_output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| stereo(value)[0] > 0.39)
    );
    let silence = pump(&mut app, 256);
    assert!(
        silence["audio_output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| stereo(value) == [0.0; 2])
    );
    let release = 10.0_f64.powf(-24.0 * (256.0 / 48_000.0) / 20.0);
    near(master(&silence), [0.4 * release, 0.2 * release]);
    assert!(
        !silence["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "−∞ dB")
    );
    near(master(&app.inspect()), master(&silence));
    near(master(&app.click("Pause")), [0.0; 2]);
    app.click("Stop");
    app.quit();
}
