//! Live mixer analysis uses the production renderer and actual submitted plots.
mod common;

use common::{Harness, wait_until};
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

const RATE: u32 = 48_000;

fn fixture(opposite: bool, muted: bool) -> (Harness, u64, u64) {
    let mut app = Harness::with_virtual_audio(None, None);
    let source = app.directory.path().join("1 kHz stereo.wav");
    AudioData {
        sample_rate: RATE,
        channels: 2,
        samples: (0..RATE * 16)
            .flat_map(|frame| {
                let sample =
                    (frame as f32 * std::f32::consts::TAU * 1000.0 / RATE as f32).sin() * 0.25;
                [sample, if opposite { -sample } else { sample }]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let tone = project.add_audio_track("Measured tone");
    let bus = project.add_bus("Measured bus");
    let track = project.track_mut(tone).unwrap();
    track.gain = 1.0;
    track.mute = muted;
    track.clips.push(Clip::audio(
        "Known sine",
        0.0,
        32.0,
        source.to_string_lossy(),
    ));
    project.track_mut(bus).unwrap().gain = 0.5;
    project.set_output(tone, Some(bus)).unwrap();
    let path = app.directory.path().join("Analysis.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1400,"height":900}));
    app.click("Mixer");
    (app, tone, bus)
}

fn open(app: &mut Harness, track: Option<u64>) -> Value {
    let state = app.inspect();
    let rect = match track {
        Some(id) => &state["mixer"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["track"] == id)
            .unwrap()["analyze"],
        None => &state["mixer"]["master"]["analyze"],
    };
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let state = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(state["popups"]["analysis"], true);
    assert_eq!(state["signal_analysis"]["track"], json!(track));
    state
}

fn play(app: &mut Harness) {
    app.click("Play");
    wait_until(app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
}

fn measure(app: &mut Harness, frames: u64) -> Value {
    // Pace a bounded production queue while consuming actual rendered audio.
    // Each 4096-frame pump is several renderer blocks, never a fake snapshot.
    for _ in 0..frames.div_ceil(4096) {
        app.command(json!({"command":"pump_audio","frames":4096}));
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    wait_until(app, |state| {
        state["signal_analysis"]["snapshot"]["observed_frames"]
            .as_u64()
            .is_some_and(|count| count >= frames)
    })
}

fn unchanged(state: &Value, baseline: &Value) {
    for field in [
        "project",
        "history",
        "song_session",
        "modified",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(state[field], baseline[field], "Analysis changed {field}");
    }
}

fn trace(state: &Value) -> &Value {
    let plot = &state["signal_analysis"]["plot"];
    state["painted_meshes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mesh| {
            let bounds = &mesh["bounds"];
            mesh["valid"] == true
                && mesh["vertices"] == state["signal_analysis"]["trace_vertices"]
                && bounds[0].as_f64().unwrap() >= plot["min"][0].as_f64().unwrap() - 2.0
                && bounds[1].as_f64().unwrap() >= plot["min"][1].as_f64().unwrap() - 2.0
                && bounds[2].as_f64().unwrap() <= plot["max"][0].as_f64().unwrap() + 2.0
                && bounds[3].as_f64().unwrap() <= plot["max"][1].as_f64().unwrap() + 2.0
        })
        .expect("Actual analyzer trace mesh submitted within its plot")
}

fn phase(app: &mut Harness, opposite: bool) {
    app.click("Phase");
    let state = app.command(json!({"command":"inspect","paint":true}));
    let mesh = trace(&state);
    let bounds = &mesh["bounds"];
    let width = bounds[2].as_f64().unwrap() - bounds[0].as_f64().unwrap();
    let height = bounds[3].as_f64().unwrap() - bounds[1].as_f64().unwrap();
    assert!(mesh["vertices"].as_u64().unwrap() >= 64);
    if opposite {
        assert!(width > height * 8.0, "Antiphase is horizontal: {bounds}");
    } else {
        assert!(height > width * 8.0, "In-phase is vertical: {bounds}");
    }
}

#[test]
fn channel_bus_and_master_measure_live_audio_without_editing_or_starting_playback() {
    let (mut app, tone, bus) = fixture(false, false);
    let baseline = app.inspect();
    let opened = open(&mut app, Some(tone));
    unchanged(&opened, &baseline);
    // The focused utility retains inert context: background drags and editing
    // shortcuts cannot modify the song while this measurement is open.
    let background_fader = &baseline["mixer"]["channels"][0]["fader"];
    let x = (background_fader["min"][0].as_f64().unwrap()
        + background_fader["max"][0].as_f64().unwrap())
        * 0.5;
    app.command(json!({"command":"pointer","phase":"drag","x":x,
        "y":background_fader["max"][1].as_f64().unwrap()-10.0,
        "to_x":x,"to_y":background_fader["min"][1].as_f64().unwrap()+10.0}));
    unchanged(&app.key("Delete"), &baseline);
    assert_eq!(app.inspect()["popups"]["analysis"], true);
    assert!(opened["signal_analysis"]["snapshot"].is_null());
    assert_ne!(opened["engine"]["playing"], true);
    assert!(
        opened["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Play to analyze this signal")
    );
    play(&mut app);
    let measured = measure(&mut app, 160_000);
    let snapshot = &measured["signal_analysis"]["snapshot"];
    let source_peak = snapshot["peak_dbfs"].as_f64().unwrap();
    assert!((source_peak + 12.0412).abs() < 0.15, "{source_peak}");
    assert!((snapshot["rms_dbfs"].as_f64().unwrap() + 15.0515).abs() < 0.15);
    let bins = snapshot["spectrum_db"].as_array().unwrap();
    let maximum = bins
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.as_f64().unwrap().total_cmp(&b.as_f64().unwrap()))
        .unwrap()
        .0;
    let frequency = maximum as f64 * RATE as f64 / snapshot["fft_size"].as_f64().unwrap();
    assert!(
        (frequency - 1000.0).abs() < 12.0,
        "Measured peak {frequency} Hz"
    );
    let painted = app.command(json!({"command":"inspect","paint":true}));
    let spectrum_mesh = trace(&painted);
    assert_eq!(spectrum_mesh["vertices"], 384);
    let strongest_vertex = spectrum_mesh["vertex_data"]
        .as_array()
        .unwrap()
        .iter()
        .min_by(|left, right| {
            left["position"][1]
                .as_f64()
                .unwrap()
                .total_cmp(&right["position"][1].as_f64().unwrap())
        })
        .unwrap();
    let plot = &painted["signal_analysis"]["plot"];
    let plotted_frequency = (strongest_vertex["position"][0].as_f64().unwrap()
        - plot["min"][0].as_f64().unwrap())
        / (plot["max"][0].as_f64().unwrap() - plot["min"][0].as_f64().unwrap());
    let expected_frequency = (1000.0_f64 / 20.0).ln() / (24_000.0_f64 / 20.0).ln();
    assert!(
        (plotted_frequency - expected_frequency).abs() < 0.02,
        "Actual mesh peak follows logarithmic 1 kHz position"
    );
    let generation = snapshot["generation"].clone();
    app.click("Loudness");
    let loudness = app.inspect();
    for name in ["lufs_momentary", "lufs_short_term", "lufs_integrated"] {
        let value = loudness["signal_analysis"]["snapshot"][name]
            .as_f64()
            .expect("LUFS becomes valid after its audio window");
        assert!((-20.0..-8.0).contains(&value), "{name} = {value}");
    }
    app.click("Correlation");
    assert!(
        app.inspect()["signal_analysis"]["snapshot"]["correlation"]
            .as_f64()
            .unwrap()
            > 0.999
    );
    phase(&mut app, false);
    let after_tabs = app.click("Spectrum");
    assert_eq!(
        after_tabs["signal_analysis"]["snapshot"]["generation"],
        generation
    );
    unchanged(&after_tabs, &baseline);
    let reset_live = app.click("Reset measurement");
    assert!(reset_live["signal_analysis"]["snapshot"].is_null());
    let restarted = measure(&mut app, 24_000);
    assert!(restarted["signal_analysis"]["snapshot"]["lufs_short_term"].is_null());
    assert!(
        (restarted["signal_analysis"]["snapshot"]["peak_dbfs"]
            .as_f64()
            .unwrap()
            - source_peak)
            .abs()
            < 0.15
    );
    // Stop retains the last real measurement and Reset clears it without audio.
    let stopped = app.click("Stop");
    assert!(stopped["signal_analysis"]["snapshot"]["peak_dbfs"].is_number());
    let reset = app.click("Reset measurement");
    assert!(reset["signal_analysis"]["snapshot"].is_null());
    unchanged(&reset, &baseline);
    assert_eq!(app.click("Done")["view"], "Mixer");

    for target in [Some(bus), None] {
        let opened = open(&mut app, target);
        assert_ne!(opened["engine"]["playing"], true);
        assert!(opened["signal_analysis"]["snapshot"].is_null());
        play(&mut app);
        let measured = measure(&mut app, 24_000);
        let peak = measured["signal_analysis"]["snapshot"]["peak_dbfs"]
            .as_f64()
            .unwrap();
        assert!(
            (source_peak - peak - 6.0206).abs() < 0.15,
            "Post-fader {target:?}: {peak}"
        );
        unchanged(&measured, &baseline);
        app.click("Stop");
        app.click("Done");
    }
    // Replacing a document cannot display an old target or its readings.
    open(&mut app, Some(tone));
    app.click("File");
    app.click("New");
    let fresh = app.command(json!({"command":"wait_project"}));
    assert_eq!(fresh["popups"]["analysis"], false);
    assert!(fresh["signal_analysis"].is_null());
    app.quit();
}

#[test]
fn opposite_phase_and_muted_channel_are_measured_without_invented_loudness() {
    for muted in [false, true] {
        let (mut app, tone, _) = fixture(true, muted);
        let before = app.inspect();
        open(&mut app, Some(tone));
        play(&mut app);
        let measured = measure(&mut app, 24_000);
        let snapshot = &measured["signal_analysis"]["snapshot"];
        if muted {
            for name in [
                "peak_dbfs",
                "rms_dbfs",
                "lufs_momentary",
                "lufs_integrated",
                "correlation",
            ] {
                assert!(
                    snapshot[name].is_null(),
                    "Muted {name} must be undefined: {}",
                    snapshot[name]
                );
            }
            let loudness = app.click("Loudness");
            assert!(
                loudness["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| label == "— LUFS")
            );
        } else {
            app.click("Correlation");
            assert!(
                app.inspect()["signal_analysis"]["snapshot"]["correlation"]
                    .as_f64()
                    .unwrap()
                    < -0.999
            );
            phase(&mut app, true);
        }
        unchanged(&app.inspect(), &before);
        app.click("Stop");
        app.click("Done");
        app.quit();
    }
}

#[test]
fn target_deletion_undo_and_compact_views_never_restore_stale_measurements() {
    let (mut app, tone, _) = fixture(false, false);
    app.command(json!({"command":"viewport","width":800,"height":600}));
    open(&mut app, Some(tone));
    let baseline = app.inspect();
    for view in ["Spectrum", "Loudness", "Correlation", "Phase"] {
        let state = app.click(view);
        assert!(state["signal_analysis"]["snapshot"].is_null());
        let reset = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == "Reset measurement")
            .unwrap();
        assert!(reset["min"][1].as_f64().unwrap() >= 0.0);
        assert!(
            reset["max"][1].as_f64().unwrap() <= 600.0,
            "Footer remains mouse-reachable: {reset}"
        );
        unchanged(&state, &baseline);
    }
    let state = app.inspect();
    let deleted = app.command(json!({"command":"song_tool","name":"apply_edits","arguments":{
        "session_id":state["song_session"]["id"],"expected_revision":state["song_session"]["revision"],
        "label":"Delete analyzed channel","edits":[{"op":"delete_track","track":tone}]
    }}));
    assert_eq!(deleted["tool_result"]["ok"], true);
    assert_eq!(deleted["popups"]["analysis"], false);
    assert!(deleted["signal_analysis"].is_null());
    let restored = app.command(json!({"command":"song_tool","name":"history","arguments":{
        "action":"undo","session_id":deleted["song_session"]["id"],"expected_revision":deleted["song_session"]["revision"]
    }}));
    assert_eq!(restored["tool_result"]["ok"], true);
    app.click("Mixer");
    let reopened = open(&mut app, Some(tone));
    assert!(reopened["signal_analysis"]["snapshot"].is_null());
    assert_ne!(reopened["engine"]["playing"], true);
    app.key("Escape");
    assert_eq!(app.inspect()["popups"]["analysis"], false);
    app.quit();
}

#[test]
fn analyzer_hint_cannot_intercept_a_coalesced_click_on_effects() {
    let (mut app, _, _) = fixture(false, false);
    app.command(json!({"command":"viewport","width":800,"height":600}));
    let before = app.inspect();
    let analyze = &before["mixer"]["channels"][0]["analyze"];
    let x = (analyze["min"][0].as_f64().unwrap() + analyze["max"][0].as_f64().unwrap()) * 0.5;
    let y = (analyze["min"][1].as_f64().unwrap() + analyze["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"move","x":x,"y":y}));
    let mut hovered = app.inspect();
    for _ in 0..40 {
        hovered = app.inspect();
    }
    assert!(
        hovered["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Live spectrum, LUFS loudness, stereo correlation and phase")
    );
    // The native hint overlaps the adjacent Effects caption. This semantic
    // mouse click delivers pointer arrival and press together in one frame.
    let effects = app.click("Effects");
    assert_eq!(effects["popups"]["effects"], true);
    assert_eq!(effects["popups"]["analysis"], false);
    unchanged(&effects, &before);
    app.key("Escape");
    app.quit();
}
