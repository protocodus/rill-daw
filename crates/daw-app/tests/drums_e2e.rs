//! R1 Drums through the running app: recordings loaded onto pads by the
//! `set_device_sample` tool and by dropping a file on a pad of the panel;
//! each pad's note plays its own recording, and a hit cuts the pads of its
//! choke group.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Instrument, Note, Project};
use serde_json::{Value, json};

fn song_tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    let state = app.command(json!({"command":"song_tool","name":name,"arguments":arguments}));
    assert_eq!(state["tool_result"]["ok"], true, "{name}: {state}");
    state
}

fn session(state: &Value) -> (Value, Value) {
    (
        state["song_session"]["id"].clone(),
        state["song_session"]["revision"].clone(),
    )
}

/// Two seconds of a sine at `hz`, at 44.1 kHz.
fn sine(app: &Harness, name: &str, hz: f32) -> std::path::PathBuf {
    let path = app.directory.path().join(name);
    AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: (0..88_200)
            .map(|n| 0.5 * (std::f32::consts::TAU * hz * n as f32 / 44_100.0).sin())
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    path
}

/// A MIDI track with R1 Drums playing `notes` (pitch, beat) for a beat
/// each at 120 BPM, and, muted on an audio track, one clip per recording,
/// named as given. Returns the project, the drum track and the clips.
fn project(
    recordings: &[(&str, &std::path::Path)],
    notes: &[(u8, f64)],
) -> (Project, u64, Vec<u64>) {
    let mut project = Project::new();
    project.tempo = 120.0;
    let audio = project.add_audio_track("Audio");
    let mut clips = Vec::new();
    for (index, (name, path)) in recordings.iter().enumerate() {
        let mut clip = Clip::audio(*name, 2.0 * index as f64, 2.0, path.to_string_lossy());
        clip.muted = true;
        clips.push(clip.id);
        project.track_mut(audio).unwrap().clips.push(clip);
    }
    let beat = project.add_instrument_track("Beat");
    project
        .set_instrument(beat, Instrument::builtin("r1-drums"))
        .unwrap();
    let channel = project.track_mut(beat).unwrap();
    channel.gain = 1.0;
    channel.clips.push(Clip::midi(
        "Hits",
        0.0,
        12.0,
        notes
            .iter()
            .map(|(pitch, at)| Note::new(*pitch, *at, 1.0, 127))
            .collect(),
    ));
    (project, beat, clips)
}

fn open(app: &mut Harness, project: &Project) -> Value {
    let path = app.directory.path().join("drums.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}))
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

/// The left channel between `from` and `to` seconds.
fn window(audio: &AudioData, from: f32, to: f32) -> Vec<f32> {
    let rate = audio.sample_rate as f32;
    audio
        .samples
        .iter()
        .step_by(audio.channels)
        .skip((from * rate) as usize)
        .take(((to - from) * rate) as usize)
        .copied()
        .collect()
}

/// Upward zero crossings per second.
fn frequency(audio: &AudioData, from: f32, to: f32) -> f32 {
    let samples = window(audio, from, to);
    let crossings = samples
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    crossings as f32 / (to - from)
}

/// Amplitude of the component at `hz` between `from` and `to` seconds.
fn tone(audio: &AudioData, hz: f32, from: f32, to: f32) -> f32 {
    let samples = window(audio, from, to);
    let rate = audio.sample_rate as f32;
    let (re, im) = samples
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(re, im), (n, x)| {
            let phase = std::f32::consts::TAU * hz * n as f32 / rate;
            (re + x * phase.cos(), im + x * phase.sin())
        });
    2.0 * (re * re + im * im).sqrt() / samples.len().max(1) as f32
}

fn rms(audio: &AudioData, from: f32, to: f32) -> f32 {
    let samples = window(audio, from, to);
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

/// The recording in sample slot `slot` of the drum track.
fn slot(state: &Value, track: u64, slot: u64) -> Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == track)
        .unwrap()["instrument"]["Builtin"]["samples"]
        .as_array()
        .and_then(|samples| samples.iter().find(|sample| sample["slot"] == slot))
        .cloned()
        .unwrap_or(Value::Null)
}

fn label_center(state: &Value, label: &str) -> (f64, f64) {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

#[test]
fn each_pad_plays_its_own_recording_and_a_choke_group_cuts_the_other_pad() {
    let mut app = Harness::new();
    let low = sine(&app, "low.wav", 440.0);
    let closed = sine(&app, "closed.wav", 660.0);
    let open_hat = sine(&app, "open.wav", 880.0);
    // Pad 3 (D1, 38) at once, ringing out its two seconds; pad 7 (42) at
    // 3 s, pad 9 (44) half a second later while pad 7 still rings.
    let (project, beat, clips) = project(
        &[("Snare", &low), ("Closed", &closed), ("Open", &open_hat)],
        &[(38, 0.0), (42, 6.0), (44, 7.0)],
    );
    let before = open(&mut app, &project);
    assert!(
        rms(&export(&mut app, "empty.wav"), 0.1, 0.4) < 1e-4,
        "no pad loaded"
    );

    // A slot the device does not have is refused.
    let (id, revision) = session(&before);
    let refused = app.command(json!({"command":"song_tool","name":"set_device_sample",
        "arguments":{"session_id":id,"expected_revision":revision,"track_id":beat,"slot":16,"clip_id":clips[0]}}));
    assert_eq!(
        refused["tool_result"]["error"]["code"], "invalid_arguments",
        "{refused}"
    );
    let mut state = before.clone();
    for (pad, clip) in [(2, clips[0]), (6, clips[1]), (8, clips[2])] {
        let (id, revision) = session(&state);
        state = song_tool(
            &mut app,
            "set_device_sample",
            json!({"session_id":id,"expected_revision":revision,"track_id":beat,"slot":pad,"clip_id":clip}),
        );
    }
    assert_eq!(slot(&state, beat, 2)["name"], "Snare");
    assert_eq!(slot(&state, beat, 6)["name"], "Closed");

    // Each note sounds its pad's recording at its own pitch; the others stay silent.
    let played = export(&mut app, "pads.wav");
    let hz = frequency(&played, 0.1, 0.4);
    assert!((hz - 440.0).abs() < 8.0, "pad 3: {hz} Hz");
    assert!(rms(&played, 0.1, 0.4) > 0.05);
    assert!(
        rms(&played, 2.3, 2.9) < 1e-3,
        "pad 3 ends with its recording"
    );
    let hz = frequency(&played, 3.1, 3.4);
    assert!((hz - 660.0).abs() < 8.0, "pad 7: {hz} Hz");
    // Without a choke group, pad 7 rings on under pad 9.
    let (closed_hat, open_hat) = (
        tone(&played, 660.0, 3.6, 3.9),
        tone(&played, 880.0, 3.6, 3.9),
    );
    assert!(
        closed_hat > 0.05 && open_hat > 0.05,
        "{closed_hat} {open_hat}"
    );

    // In one choke group, pad 9 cuts pad 7: only its own recording is left.
    let (id, revision) = session(&state);
    song_tool(
        &mut app,
        "set_device_parameters",
        json!({"session_id":id,"expected_revision":revision,"track_id":beat,
            "parameters":[{"parameter":"pad7.choke","value":"1"},
                          {"parameter":"pad9.choke","value":"1"}]}),
    );
    let choked = export(&mut app, "choked.wav");
    let (closed_hat, open_hat) = (
        tone(&choked, 660.0, 3.6, 3.9),
        tone(&choked, 880.0, 3.6, 3.9),
    );
    assert!(closed_hat < 0.002, "pad 7 is cut: {closed_hat}");
    assert!(open_hat > 0.05, "pad 9 sounds: {open_hat}");
    let hz = frequency(&choked, 3.1, 3.4);
    assert!((hz - 660.0).abs() < 8.0, "pad 7 rang until pad 9: {hz} Hz");
    app.quit();
}

#[test]
fn a_file_dropped_on_a_pad_loads_into_that_pad_and_its_note_plays_it() {
    let mut app = Harness::new();
    let first = sine(&app, "first.wav", 440.0);
    let second = sine(&app, "second.wav", 220.0);
    let (project, beat, clips) = project(&[("Kick", &first), ("Snare", &first)], &[(38, 0.0)]);
    let before = open(&mut app, &project);
    let mut state = before.clone();
    for (pad, clip) in [(0, clips[0]), (2, clips[1])] {
        let (id, revision) = session(&state);
        state = song_tool(
            &mut app,
            "set_device_sample",
            json!({"session_id":id,"expected_revision":revision,"track_id":beat,"slot":pad,"clip_id":clip}),
        );
    }
    app.command(json!({"command":"viewport","width":1600,"height":1400,"pixels_per_point":1.0}));
    app.click("Beat");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "Snare");

    // A click on a pad shows its controls.
    let state = app.inspect();
    let (x, y) = label_center(&state, "Snare");
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    common::wait_visible(&mut app, "PAD 3");

    // A file dropped on pad 3 replaces its recording; nothing else changes.
    let dropped = app.command(json!({"command":"drop_at","paths":[second],"x":x,"y":y}));
    let dropped = if slot(&dropped, beat, 2)["name"] == "second" {
        dropped
    } else {
        app.command(json!({"command":"wait_audio_import"}))
    };
    assert_eq!(
        slot(&dropped, beat, 2)["name"],
        "second",
        "{}",
        dropped["project"]
    );
    assert_eq!(slot(&dropped, beat, 0)["name"], "Kick");
    assert_eq!(
        dropped["project"]["tracks"].as_array().unwrap().len(),
        before["project"]["tracks"].as_array().unwrap().len(),
        "the panel took the file"
    );
    assert_eq!(dropped["history"]["undo"][0], "Load sample");
    let hz = frequency(&export(&mut app, "dropped.wav"), 0.1, 0.4);
    assert!((hz - 220.0).abs() < 8.0, "{hz} Hz");
    app.quit();
}
