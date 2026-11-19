//! Headless E2E verification that UI panels, cached backdrop blur, view
//! switches, and live device visualizations never interrupt, glitched-out, or
//! alter running multicore R1 audio rendering.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Effect, EffectSlot, Instrument, Note, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const SAMPLE_RATE: u32 = 48_000;
const FRAMES_PER_BEAT: usize = 24_000; // 120 BPM @ 48 kHz
const CHUNK_FRAMES: usize = 4_000;
const CHUNKS: usize = 24; // 96_000 frames = 4.0 beats

fn wait_until(app: &mut Harness, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if ready(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "Timed out waiting: {state}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn click_label(app: &mut Harness, state: &Value, predicate: impl Fn(&str) -> bool) -> Value {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"].as_str().is_some_and(&predicate))
        .unwrap_or_else(|| panic!("Missing matching label in {}", state["visible_labels"]));
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

fn build_r1_project(wav_path: &std::path::Path) -> (Project, u64) {
    AudioData {
        sample_rate: SAMPLE_RATE,
        channels: 2,
        samples: (0..FRAMES_PER_BEAT * 6)
            .flat_map(|frame| {
                let t = frame as f32 / SAMPLE_RATE as f32;
                let left = (std::f32::consts::TAU * 220.0 * t).sin() * 0.25;
                let right = (std::f32::consts::TAU * 330.0 * t).sin() * 0.25;
                [left, right]
            })
            .collect(),
    }
    .write_wav(wav_path)
    .unwrap();

    let mut project = Project::new();
    project.tempo = 120.0;

    let synth_effects = [
        &["r1-equalizer", "r1-compressor"][..],
        &["r1-filter", "r1-spacer"],
        &["r1-saturator", "r1-modulator"],
        &["r1-equalizer", "r1-spacer"],
    ];
    let mut lead_track = 0;
    for (idx, effects) in synth_effects.iter().enumerate() {
        let track_id = project.add_midi_track(format!("Synth {idx}"));
        if idx == 0 {
            lead_track = track_id;
        }
        let track = project.track_mut(track_id).unwrap();
        track.instrument = Instrument::builtin("r1-mini");
        track.gain = 0.08;
        for &effect in *effects {
            track.inserts.push(EffectSlot::new(
                effect,
                Effect::Builtin {
                    device: effect.into(),
                },
            ));
        }
        track.clips.push(Clip::midi(
            format!("Lead {idx}"),
            0.0,
            6.0,
            vec![
                Note::new(57 + idx as u8 * 2, 0.0, 2.0, 100),
                Note::new(64 + idx as u8 * 2, 2.0, 2.0, 100),
            ],
        ));
    }

    let wav_str = wav_path.to_string_lossy().into_owned();
    let audio_effects = [
        &["r1-saturator"][..],
        &["r1-modulator"],
        &["r1-compressor"],
        &["r1-filter"],
    ];
    for (idx, effects) in audio_effects.iter().enumerate() {
        let track_id = project.add_audio_track(format!("Stem {idx}"));
        let track = project.track_mut(track_id).unwrap();
        track.gain = 0.08;
        for &effect in *effects {
            track.inserts.push(EffectSlot::new(
                effect,
                Effect::Builtin {
                    device: effect.into(),
                },
            ));
        }
        track
            .clips
            .push(Clip::audio(format!("Stem clip {idx}"), 0.0, 6.0, &wav_str));
    }

    project.validate().unwrap();
    (project, lead_track)
}

fn assert_audio_health_and_match(
    state: &Value,
    expected_ref: &[f32],
    offset_frames: usize,
    expect_parallel: bool,
) {
    let engine = &state["engine"];
    assert_eq!(engine["playing"], true, "playback stopped: {engine}");
    assert_eq!(engine["underruns"], 0, "underrun observed: {engine}");
    assert_eq!(engine["render_errors"], 0, "render error: {engine}");
    assert_eq!(engine["edit_errors"], 0, "edit error: {engine}");
    assert_eq!(engine["output_failed"], false, "output failed: {engine}");
    assert_eq!(engine["output_dropouts"], 0, "output dropouts: {engine}");
    if expect_parallel {
        let workers = engine["render_performance"]["worker_count"]
            .as_u64()
            .unwrap_or(0);
        assert!(
            workers >= 2,
            "expected multicore workers for 8-track R1 session, got {workers}: {engine}"
        );
    }

    let samples = state["audio_output"].as_array().unwrap();
    assert_eq!(samples.len(), CHUNK_FRAMES);
    for (frame_idx, stereo) in samples.iter().enumerate() {
        let abs_frame = offset_frames + frame_idx;
        let expected_l = expected_ref[abs_frame * 2];
        let expected_r = expected_ref[abs_frame * 2 + 1];
        let actual_l = stereo[0].as_f64().unwrap() as f32;
        let actual_r = stereo[1].as_f64().unwrap() as f32;
        assert!(
            (actual_l - expected_l).abs() < 1e-6 && (actual_r - expected_r).abs() < 1e-6,
            "UI activity perturbed audio at frame {abs_frame}: actual=[{actual_l}, {actual_r}] expected=[{expected_l}, {expected_r}]"
        );
    }
}

#[test]
fn heavy_ui_interactions_never_interrupt_or_alter_multicore_r1_playback() {
    let mut app = Harness::with_virtual_audio(None, None);
    let wav_path = app.directory.path().join("stem.wav");
    let (project, _lead_track) = build_r1_project(&wav_path);
    let expected = daw_engine::render(&project, SAMPLE_RATE, 4.0).unwrap();

    let project_path = app.directory.path().join("r1_isolation.rill");
    project.save(&project_path).unwrap();
    app.command(json!({"command":"open","path":project_path}));
    app.command(json!({"command":"viewport","width":1400,"height":900}));

    // Select first clip and start playback via mouse click.
    app.click("Lead 0");
    app.click("Play");
    let started = wait_until(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    assert!(
        started["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Pause"),
        "expected Pause button while playing"
    );

    let expect_parallel = std::thread::available_parallelism().map_or(1, usize::from) > 1;
    let mut cursor_frames = 0usize;

    for chunk in 0..CHUNKS {
        let pumped = app.command(json!({"command":"pump_audio","frames":CHUNK_FRAMES}));
        assert_audio_health_and_match(&pumped, &expected, cursor_frames, expect_parallel);
        cursor_frames += CHUNK_FRAMES;

        match chunk {
            2 => {
                // Open R1 Mini instrument panel popup via mouse, switch tab, close with mouse.
                app.click("Window");
                let opened = app.click("Instrument");
                assert_eq!(opened["popups"]["instrument"], true);
                let motion = app.click("Modulation");
                assert!(
                    !motion["backdrop_blur"].as_array().unwrap().is_empty(),
                    "instrument popup must keep backdrop blur"
                );
                let closed = common::close_utility(&mut app, &motion, "Instrument of Synth 0");
                assert_eq!(closed["popups"]["instrument"], false);
            }
            6 => {
                // Open Effects panel (R1 Equalizer spectrum + R1 Compressor), close with mouse.
                app.click("Window");
                let opened = app.click("Effects");
                assert_eq!(opened["popups"]["effects"], true);
                let closed = common::close_utility(&mut app, &opened, "Effects of Synth 0");
                assert_eq!(closed["popups"]["effects"], false);
            }
            10 => {
                // Open Routing popup and dismiss via mouse.
                app.click("Window");
                let opened = app.click("Routing");
                assert_eq!(opened["popups"]["routing"], true);
                let closed = common::close_utility(&mut app, &opened, "Routing of Synth 0");
                assert_eq!(closed["popups"]["routing"], false);
            }
            14 => {
                // Switch to Mixer view and back to Arranger via mouse clicks.
                let mixer = app.click("Mixer");
                assert_eq!(mixer["view"], "Mixer");
                let arranger = app.click("Arranger");
                assert_eq!(arranger["view"], "Arrangement");
            }
            18 => {
                // Open and close CPU load popover via mouse clicks if sample is ready.
                let state = app.inspect();
                if state["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|label| label.starts_with("CPU "))
                {
                    let cpu_open = click_label(&mut app, &state, |s| s.starts_with("CPU "));
                    assert!(
                        cpu_open["visible_labels"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|label| label == "CPU load"),
                        "expected CPU load popover: {}",
                        cpu_open["visible_labels"]
                    );
                    let _ = click_label(&mut app, &cpu_open, |s| s.starts_with("CPU "));
                }
            }
            21 => {
                // Open Setup popup and close via Cancel button.
                app.click("Window");
                let setup = app.click("Setup");
                assert_eq!(setup["popups"]["setup"], true);
                let closed = app.click("Cancel");
                assert_eq!(closed["popups"]["setup"], false);
            }
            _ => {}
        }
    }

    let final_pos = app.inspect()["engine"]["position_beats"].as_f64().unwrap();
    assert!(
        (final_pos - 4.0).abs() < 1e-6,
        "expected playhead at 4.0 beats, got {final_pos}"
    );
    let paused = app.click("Pause");
    assert_eq!(paused["engine"]["playing"], false);
    let stopped = app.click("Stop");
    assert_eq!(stopped["device_open"], false);
    assert!(
        stopped["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Play"),
        "expected Play button after Stop"
    );
    app.quit();
}
