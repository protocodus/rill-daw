//! R1 Drums through the running app: recordings loaded onto pads by the
//! `set_device_sample` tool and by dropping a file on a pad of the panel;
//! each pad's note plays its own recording, and a hit cuts the pads of its
//! choke group.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, DeviceSample, Instrument, Note, Project};
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

/// A MIDI track with R1 Drums playing `notes` (pitch, beat, velocity) for a beat
/// each at 120 BPM, and, muted on an audio track, one clip per recording,
/// named as given. Returns the project, the drum track and the clips.
fn project(
    recordings: &[(&str, &std::path::Path)],
    notes: &[(u8, f64, u8)],
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
            .map(|&(pitch, at, velocity)| Note::new(pitch, at, 1.0, velocity))
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
        &[(38, 0.0, 127), (42, 6.0, 127), (44, 7.0, 127)],
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
    let (project, beat, clips) = project(&[("Kick", &first), ("Snare", &first)], &[(38, 0.0, 127)]);
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
    app.command(json!({"command":"viewport","width":1480,"height":960,"pixels_per_point":2.0}));
    app.click("Beat");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "Snare");

    // A click on a pad shows its controls.
    let state = app.inspect();
    let (x, y) = label_center(&state, "Snare");
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    common::wait_visible(&mut app, "PAD 3");

    // Rename the selected pad from its source header, then restore its original name.
    let selected = app.inspect();
    app.click("Rename…");
    app.command(json!({"command":"text","text":"Backbeat"}));
    let renamed = app.click("Apply name");
    let instrument = &renamed["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == beat)
        .unwrap()["instrument"]["Builtin"];
    assert_eq!(instrument["sample_names"]["2"], "Backbeat");
    assert_eq!(slot(&renamed, beat, 2)["name"], "Snare");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], selected["project"]);

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
    // The footer stays below the entire keyboard on a native Retina display.
    // Press the bottom of the D2 key, which plays the recording on pad 3.
    let state = app.inspect();
    let keyboard = &state["native_editor"]["performance"]["keyboard"];
    let left = keyboard["min"][0].as_f64().unwrap();
    let key_width = (keyboard["max"][0].as_f64().unwrap() - left) / 28.0;
    let x = left + 1.5 * key_width;
    let y = keyboard["max"][1].as_f64().unwrap() - 1.0;
    let pressed = app.command(json!({"command":"pointer","phase":"down","x":x,"y":y}));
    assert_eq!(pressed["audition_request"][0], beat);
    assert_eq!(pressed["audition_request"][1], 38);
    app.command(json!({"command":"pointer","phase":"up","x":x,"y":y}));
    // The pad number, recording and duration share a line even in a narrow inspector.
    // The narrower toolbar exposes all its commands in a second row.
    for width in [1600, 800] {
        app.command(
            json!({"command":"viewport","width":width,"height":960,"pixels_per_point":1.0}),
        );
        let state = app.inspect();
        let (_, pad_y) = label_center(&state, "PAD 3");
        let (_, duration_y) = label_center(&state, "2.00 s");
        assert!((pad_y - duration_y).abs() < 3.0);
        let name_in_header = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| {
                label["text"] == "second"
                    && label["min"][1].as_f64().unwrap() < pad_y
                    && label["max"][1].as_f64().unwrap() > pad_y
            });
        assert!(name_in_header, "Recording name hidden at {width} points");
        let (_, actions_y) = label_center(&state, "Sample actions");
        assert_eq!(actions_y > pad_y + 20.0, width == 800);
        let rect = |text| {
            state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == text)
                .unwrap()
        };
        let one_shot = rect("One-shot");
        let gate = rect("Gate");
        let horizontal_gap =
            gate["min"][0].as_f64().unwrap() - one_shot["max"][0].as_f64().unwrap();
        let vertical_gap = gate["min"][1].as_f64().unwrap() - one_shot["max"][1].as_f64().unwrap();
        assert!(
            horizontal_gap >= 5.0 || vertical_gap >= 5.0,
            "Touching trigger labels at width {width}"
        );
    }
    let hz = frequency(&export(&mut app, "dropped.wav"), 0.1, 0.4);
    assert!((hz - 220.0).abs() < 8.0, "{hz} Hz");
    app.quit();
}

fn choose_bit_depth(app: &mut Harness, current: &str, choice: &str) {
    let (x, y) = label_center(&app.inspect(), "Bit depth");
    app.command(json!({"command":"pointer","phase":"move","x":x,"y":y}));
    app.click(current);
    let picked = app.click(choice);
    assert_eq!(picked["history"]["undo"][0], "Instrument parameter");
}

fn difference_rms(a: &AudioData, b: &AudioData, from: f32, to: f32) -> f32 {
    assert_eq!(a.sample_rate, b.sample_rate);
    let a = window(a, from, to);
    let b = window(b, from, to);
    assert!(!a.is_empty());
    assert_eq!(a.len(), b.len());
    (a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / a.len() as f32).sqrt()
}

#[test]
fn soft_hit_pitch_and_bit_depth_shape_only_the_selected_pad_and_survive_reopen() {
    let mut app = Harness::new();
    let path = sine(&app, "drum-tone.wav", 440.0);
    // Separate hits in time: an untouched soft pad, then hard and soft hits
    // on the pad to edit. Both pads play exactly the same recording.
    let (mut project, track, _) = project(&[], &[(36, 0.0, 64), (38, 4.5, 127), (38, 9.0, 64)]);
    for (slot, name) in [(0, "Reference"), (2, "Snare")] {
        project
            .load_device_sample(
                track,
                DeviceSample {
                    slot,
                    name: name.into(),
                    path: path.to_string_lossy().into_owned(),
                },
            )
            .unwrap();
    }
    open(&mut app, &project);
    let clean = export(&mut app, "clean.wav");
    for start in [0.1, 2.4, 4.6] {
        let hz = frequency(&clean, start, start + 0.4);
        assert!(
            (hz - 440.0).abs() < 5.0,
            "Default pitch: {hz} Hz at {start}"
        );
    }

    app.command(json!({"command":"viewport","width":1600,"height":1400,"pixels_per_point":1.0}));
    app.click("Beat");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "Snare");
    app.click("Snare");
    common::wait_visible(&mut app, "PAD 3");
    let (x, caption_y) = label_center(&app.inspect(), "Soft hit pitch");
    let y = caption_y + 29.0;
    let pitched = app.command(json!({"command":"pointer","phase":"drag","x":x,"y":y,
        "to_x":x,"to_y":y-200.0}));
    assert_eq!(pitched["history"]["undo"][0], "Instrument parameter");
    choose_bit_depth(&mut app, "Clean", "12-bit");
    let twelve = export(&mut app, "twelve.wav");
    choose_bit_depth(&mut app, "12-bit", "8-bit");

    let save = app.directory.path().join("drum-performance.rill");
    app.command(json!({"command":"save","path":save}));
    app.command(json!({"command":"open","path":save}));
    let eight = export(&mut app, "eight-after-reopen.wav");
    let soft_hz = 440.0 * 2.0_f32.powf(1.0 - 64.0 / 127.0);
    for audio in [&twelve, &eight] {
        let hard = frequency(audio, 2.4, 2.8);
        let soft = frequency(audio, 4.6, 5.0);
        assert!((hard - 440.0).abs() < 5.0, "Full velocity: {hard} Hz");
        assert!((soft - soft_hz).abs() < 5.0, "Half velocity: {soft} Hz");
        assert!(
            difference_rms(&clean, audio, 0.1, 0.5) < 1e-6,
            "Changing pad 3 altered pad 1"
        );
    }
    // The full-velocity hit is still on pitch, isolating the resolution
    // change. Four fewer bits make each quantization step 16 times larger.
    let error_12 = difference_rms(&clean, &twelve, 2.4, 2.8);
    let error_8 = difference_rms(&clean, &eight, 2.4, 2.8);
    assert!(error_12 > 1e-5, "12-bit matched Clean: {error_12}");
    let ratio = error_8 / error_12;
    assert!(
        (12.0..20.0).contains(&ratio),
        "8-bit/12-bit noise ratio: {ratio}"
    );
    app.quit();
}
