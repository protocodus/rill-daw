//! A real realtime render worker and CPAL stream, using only ALSA's null PCM.
//! This executes on Linux/containers and cannot select physical audio hardware.
#![cfg(target_os = "linux")]
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn peaks(state: &Value, track: u64) -> [f64; 2] {
    let channel = state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["track"] == track)
        .expect("visible arranger channel");
    [
        channel["peaks"][0].as_f64().unwrap(),
        channel["peaks"][1].as_f64().unwrap(),
    ]
}

#[test]
fn live_arranger_meters_show_channel_audio_pan_and_bus_levels_then_stop() {
    let mut app = Harness::with_null_audio();
    let audio = app.directory.path().join("meter-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [0.8, 0.4]).collect(),
    }
    .write_wav(&audio)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = true;
    project.loop_start = 0.0;
    project.loop_end = 2.0;
    let source = project.add_audio_track("Live source");
    let silent = project.add_audio_track("Silent channel");
    let bus = project.add_bus("Live bus");
    let track = project.track_mut(source).unwrap();
    track.gain = 0.5;
    track.pan = -0.5;
    track
        .clips
        .push(Clip::audio("Loop", 0.0, 2.0, audio.to_string_lossy()));
    project.track_mut(bus).unwrap().gain = 0.5;
    project.set_output(source, Some(bus)).unwrap();
    let path = app.directory.path().join("meter-session.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Play");
    let deadline = Instant::now() + Duration::from_secs(15);
    let active = loop {
        let state = app.inspect();
        assert_eq!(
            state["error"], false,
            "null PCM initialization/render failed: {state}"
        );
        if peaks(&state, source)[0] > 0.1 && peaks(&state, bus)[0] > 0.05 {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "live channel peaks did not arrive: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(active["device_open"], true, "must exercise the real engine");
    let source_peaks = peaks(&active, source);
    let bus_peaks = peaks(&active, bus);
    assert!((source_peaks[0] - 0.4).abs() < 0.01, "{source_peaks:?}");
    assert!(
        (source_peaks[1] - 0.1).abs() < 0.01,
        "pan must affect meter: {source_peaks:?}"
    );
    assert!(
        (bus_peaks[0] - 0.2).abs() < 0.01,
        "bus has own fader: {bus_peaks:?}"
    );
    assert!((bus_peaks[1] - 0.05).abs() < 0.01, "{bus_peaks:?}");
    assert_eq!(peaks(&active, silent), [0.0; 2]);
    app.click("Pause");
    let paused = app.inspect();
    for channel in [source, bus, silent] {
        assert_eq!(
            peaks(&paused, channel),
            [0.0; 2],
            "paused channels must clear"
        );
    }
    app.click("Stop");
    app.quit();
}
