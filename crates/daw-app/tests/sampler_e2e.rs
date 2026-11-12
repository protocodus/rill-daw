//! R1 Sampler through the running app: a recording loaded by the
//! `set_device_sample` tool, by the panel's Load sample… and by dropping a
//! file on it, and by an audio clip's Use as sampler instrument; keys play it
//! at their interval from the root, a loop sustains it, Undo takes it back.
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

/// A second of a sine at `hz`, at 44.1 kHz.
fn sine(app: &Harness, name: &str, hz: f32) -> std::path::PathBuf {
    let path = app.directory.path().join(name);
    AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: (0..44_100)
            .map(|n| 0.5 * (std::f32::consts::TAU * hz * n as f32 / 44_100.0).sin())
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    path
}

/// A MIDI track with R1 Sampler holding C5 (72) for four beats at 120 BPM,
/// and, muted, an audio clip named Tone playing `source`. Returns the
/// project and the IDs of the instrument track and the audio clip.
fn project(source: &std::path::Path) -> (Project, u64, u64) {
    let mut project = Project::new();
    project.tempo = 120.0;
    let audio = project.add_audio_track("Audio");
    let mut clip = Clip::audio("Tone", 0.0, 2.0, source.to_string_lossy());
    clip.muted = true;
    let clip_id = clip.id;
    project.track_mut(audio).unwrap().clips.push(clip);
    let keys = project.add_instrument_track("Keys");
    project
        .set_instrument(keys, Instrument::builtin("r1-sampler"))
        .unwrap();
    let channel = project.track_mut(keys).unwrap();
    channel.gain = 1.0;
    channel.clips.push(Clip::midi(
        "Note",
        0.0,
        4.0,
        vec![Note::new(72, 0.0, 4.0, 127)],
    ));
    (project, keys, clip_id)
}

fn open(app: &mut Harness, project: &Project) -> Value {
    let path = app.directory.path().join("sampler.rill");
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

fn rms(audio: &AudioData, from: f32, to: f32) -> f32 {
    let samples = window(audio, from, to);
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

fn samples(state: &Value, track: u64) -> Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == track)
        .unwrap()["instrument"]["Builtin"]["samples"]
        .clone()
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
fn the_tool_loads_a_clip_that_keys_play_from_the_root_and_a_loop_sustains() {
    let mut app = Harness::new();
    let source = sine(&app, "a440.wav", 440.0);
    let (project, keys, clip) = project(&source);
    let before = open(&mut app, &project);
    // Nothing loaded: the sampler is silent.
    assert!(rms(&export(&mut app, "empty.wav"), 0.1, 0.4) < 1e-4);

    let (id, revision) = session(&before);
    let rejected = app.command(json!({"command":"song_tool","name":"set_device_sample",
        "arguments":{"session_id":id,"expected_revision":revision,"track_id":keys,"slot":1,"clip_id":clip}}));
    assert_eq!(
        rejected["tool_result"]["error"]["code"], "invalid_arguments",
        "{rejected}"
    );
    let loaded = song_tool(
        &mut app,
        "set_device_sample",
        json!({"session_id":id,"expected_revision":revision,"track_id":keys,"clip_id":clip}),
    );
    assert_eq!(loaded["tool_result"]["result"]["sample"], "Tone");
    assert_eq!(loaded["history"]["undo"][0], "Load sample");
    assert_eq!(samples(&loaded, keys)[0]["slot"], 0);

    // C5 is an octave above the root, C4: the 440 Hz recording sounds at
    // 880 Hz. Without a loop the second-long recording ends after half a
    // second at twice the speed.
    let played = export(&mut app, "octave.wav");
    let hz = frequency(&played, 0.1, 0.4);
    assert!((hz - 880.0).abs() < 8.0, "{hz} Hz");
    assert!(
        rms(&played, 1.0, 1.8) < 1e-4,
        "the note ends with the recording"
    );

    // Looped, the held key sounds on until it is let go.
    let (id, revision) = session(&loaded);
    song_tool(
        &mut app,
        "set_device_parameters",
        json!({"session_id":id,"expected_revision":revision,"track_id":keys,
            "parameters":[{"parameter":"loop.mode","value":"Forward"},
                          {"parameter":"sample.root","value":"A4"}]}),
    );
    let looped = export(&mut app, "looped.wav");
    assert!(rms(&looped, 1.0, 1.8) > 0.1, "the loop sustains");
    // With the root at A4 (69), C5 is three semitones up.
    let hz = frequency(&looped, 1.0, 1.8);
    let expected = 440.0 * 2f32.powf(3.0 / 12.0);
    assert!((hz - expected).abs() < 8.0, "{hz} Hz, expected {expected}");

    // Undo takes back the settings, then the sample, in two steps.
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    assert!(rms(&export(&mut app, "undone.wav"), 0.1, 0.4) < 1e-4);
    app.quit();
}

#[test]
fn the_panel_loads_a_sample_by_its_button_and_by_a_dropped_file() {
    let mut app = Harness::new();
    let first = sine(&app, "a440.wav", 440.0);
    let second = sine(&app, "low.wav", 220.0);
    let (project, keys, _) = project(&first);
    let before = open(&mut app, &project);
    app.command(json!({"command":"viewport","width":1600,"height":1400,"pixels_per_point":1.0}));
    app.click("Keys");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "or drop an audio file here");

    // Load sample… opens the file dialog and imports what it returns.
    app.command(json!({"command":"file_dialog","kind":"import_audio","path":first}));
    let state = app.inspect();
    let (x, y) = label_center(&state, "Load sample…");
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    let loaded = app.command(json!({"command":"wait_audio_import"}));
    let held = samples(&loaded, keys);
    assert_eq!(held[0]["name"], "a440", "{held}");
    assert_ne!(
        held[0]["path"],
        json!(first),
        "a managed copy, like any import"
    );
    assert_eq!(loaded["history"]["undo"][0], "Load sample");
    common::wait_visible(&mut app, "a440");
    let hz = frequency(&export(&mut app, "button.wav"), 0.1, 0.4);
    assert!((hz - 880.0).abs() < 8.0, "{hz} Hz");

    // Undo empties the slot; a file dropped on the empty view loads.
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    common::wait_visible(&mut app, "or drop an audio file here");
    let state = app.inspect();
    let (x, y) = label_center(&state, "or drop an audio file here");
    let dropped = app.command(json!({"command":"drop_at","paths":[second],"x":x,"y":y}));
    assert_eq!(samples(&dropped, keys)[0]["name"], "low", "{dropped}");
    // The panel took the file: no audio track or clip was added.
    assert_eq!(
        dropped["project"]["tracks"].as_array().unwrap().len(),
        before["project"]["tracks"].as_array().unwrap().len()
    );
    let hz = frequency(&export(&mut app, "dropped.wav"), 0.1, 0.4);
    assert!((hz - 440.0).abs() < 8.0, "{hz} Hz");

    // Clear empties it again, as one step.
    let state = app.inspect();
    let (x, y) = label_center(&state, "Clear");
    let cleared = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(samples(&cleared, keys), Value::Null);
    assert_eq!(cleared["history"]["undo"][0], "Clear sample");
    app.quit();
}

#[test]
fn use_as_sampler_instrument_adds_a_track_that_plays_the_clip() {
    let mut app = Harness::new();
    let source = sine(&app, "a440.wav", 440.0);
    let mut project = Project::new();
    project.tempo = 120.0;
    let audio = project.add_audio_track("Audio");
    let clip = Clip::audio("Tone", 0.0, 2.0, source.to_string_lossy());
    let clip_id = clip.id;
    project.track_mut(audio).unwrap().clips.push(clip);
    let before = open(&mut app, &project);
    let region = before["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == clip_id)
        .unwrap_or_else(|| panic!("{}", before["timeline"]));
    let rect = &region["rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = rect["min"][1].as_f64().unwrap() * 0.2 + rect["max"][1].as_f64().unwrap() * 0.8;
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    let used = app.click("Use as sampler instrument");
    let tracks = used["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2, "{}", used["project"]);
    // Right below the clip's track, named after the clip.
    let added = &tracks[1];
    assert_eq!(added["name"], "Tone");
    assert_eq!(added["instrument"]["Builtin"]["device"], "r1-sampler");
    let sample = &added["instrument"]["Builtin"]["samples"][0];
    // The clip's own source, as the opened bundle placed it.
    assert!(sample["path"].is_string(), "{sample}");
    assert_eq!(
        sample["path"],
        before["project"]["tracks"][0]["clips"][0]["content"]["Audio"]["path"]
    );
    assert_eq!(sample["name"], "Tone");
    assert_eq!(used["history"]["undo"][0], "Use as sampler instrument");
    assert_eq!(used["selected_track"], added["id"]);

    // Its keys play the recording: middle C at the recording's own pitch.
    let track = added["id"].as_u64().unwrap();
    let (id, revision) = session(&used);
    song_tool(
        &mut app,
        "apply_edits",
        json!({"session_id":id,"expected_revision":revision,"label":"Phrase","edits":[
            {"op":"add_midi_clip","track":track,"name":"Note","start":0.0,"length":4.0,
             "notes":[{"pitch":60,"start":0.0,"length":4.0,"velocity":127}]},
            {"op":"set_clip_muted","track":audio,"clip":clip_id,"muted":true}]}),
    );
    let hz = frequency(&export(&mut app, "used.wav"), 0.1, 0.4);
    assert!((hz - 440.0).abs() < 8.0, "{hz} Hz");

    // One Undo step takes the edits back, one more the whole track.
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    app.quit();
}

#[test]
fn the_application_import_loads_a_file_into_a_sampler_track() {
    let mut app = Harness::new();
    let source = sine(&app, "a440.wav", 440.0);
    let (project, keys, _) = project(&source);
    let before = open(&mut app, &project);
    let imported = app.command(
        json!({"command":"application_tool","name":"document_action",
        "arguments":{"action":"import","path":source,"track_id":keys}}),
    );
    assert_eq!(imported["tool_result"]["ok"], true, "{imported}");
    let loaded = app.command(json!({"command":"wait_audio_import"}));
    assert_eq!(samples(&loaded, keys)[0]["name"], "a440", "{loaded}");
    assert_eq!(
        loaded["project"]["tracks"].as_array().unwrap().len(),
        before["project"]["tracks"].as_array().unwrap().len(),
        "no clip or track was added"
    );
    // A slot the instrument does not have is refused before importing.
    let refused = app.command(
        json!({"command":"application_tool","name":"document_action",
        "arguments":{"action":"import","path":source,"track_id":keys,"slot":3}}),
    );
    assert_eq!(refused["tool_result"]["ok"], false, "{refused}");
    app.quit();
}
