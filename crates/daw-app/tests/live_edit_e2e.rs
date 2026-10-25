//! Real app UI → live-edit preparation → production renderer/queue → deterministic
//! device-free callback sink. A protocol fixture supplies isolated instrument/effect
//! processes; this tests uninterrupted musical state, not hardware scheduling speed.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Effect, EffectSlot, Instrument, Note, Project};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

struct Fixture {
    workers: tempfile::TempDir,
    plugins: tempfile::TempDir,
    host: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let workers = tempfile::tempdir().unwrap();
        let source = workers.path().join("live_audio_host.rs");
        std::fs::write(&source, include_str!("common/live_audio_host.rs")).unwrap();
        let host = workers.path().join(if cfg!(windows) {
            "live-audio-host.exe"
        } else {
            "live-audio-host"
        });
        let result = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&host)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Self {
            workers,
            plugins: common::plugin_fixture(),
            host,
        }
    }
    fn app(&self) -> (Harness, u64, u64, u64, u64) {
        let mut app = Harness::with_virtual_audio(Some(self.plugins.path()), Some(&self.host));
        let mut project = Project::new();
        project.tempo = 120.0;
        let synth = project.add_midi_track("Playing synth");
        let plugin = self
            .plugins
            .path()
            .join("RillFixture.vst3")
            .to_string_lossy()
            .into_owned();
        project
            .set_instrument(
                synth,
                Instrument::Vst3 {
                    path: plugin.clone(),
                    class_id: Some("11111111111111111111111111111111".into()),
                },
            )
            .unwrap();
        let sustained = Note::new(60, 0.0, 16.0, 100);
        let future = Note::new(64, 20.0, 2.0, 100);
        let ids = (sustained.id, future.id);
        let track = project.track_mut(synth).unwrap();
        track.gain = 1.0;
        track.clips.push(Clip::midi(
            "Sustained phrase",
            0.0,
            32.0,
            vec![sustained, future],
        ));
        track.inserts.push(EffectSlot::new(
            "Persistent delay",
            Effect::Vst3 {
                path: plugin,
                class_id: Some("33333333333333333333333333333333".into()),
            },
        ));
        let audio = project.add_audio_track("Playing audio");
        let path = app.directory.path().join("constant.wav");
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.1; 96_000],
        }
        .write_wav(&path)
        .unwrap();
        let track = project.track_mut(audio).unwrap();
        track.gain = 1.0;
        track.clips.push(Clip::audio(
            "Future audio",
            4.0,
            4.0,
            path.to_string_lossy(),
        ));
        let import_target = project.add_audio_track("Import target");
        project.track_mut(import_target).unwrap().gain = 1.0;
        let session = app.directory.path().join("live-edits.rill");
        project.save(&session).unwrap();
        app.command(json!({"command":"open", "path":session}));
        app.command(json!({"command":"viewport", "width":1400, "height":900}));
        app.click("Sustained phrase");
        app.click("Play");
        wait(&mut app, |state| {
            state["engine"]["playing"] == true && state["playback_loading"] == false
        });
        (app, synth, audio, ids.0, ids.1)
    }
    fn logs(&self) -> Vec<(PathBuf, Vec<Value>)> {
        let mut logs: Vec<_> = std::fs::read_dir(self.workers.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .map(|path| {
                let lines = std::fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect();
                (path, lines)
            })
            .collect();
        logs.sort_by(|a, b| a.0.cmp(&b.0));
        logs
    }
    fn identities(&self) -> Vec<PathBuf> {
        self.logs().into_iter().map(|(path, _)| path).collect()
    }
    fn note_events(&self, pitch: u8, on: bool) -> usize {
        self.logs()
            .into_iter()
            .flat_map(|(_, lines)| lines)
            .filter(|line| {
                line["SendMidiAt"]["event"][if on { "NoteOn" } else { "NoteOff" }]["note"] == pitch
            })
            .count()
    }
    fn assert_instances(&self, expected: &[(PathBuf, Vec<Value>)]) {
        assert_eq!(
            self.identities(),
            expected
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>(),
            "Edits must retain every sandbox process"
        );
        for ((_, log), (_, baseline)) in self.logs().into_iter().zip(expected) {
            assert_eq!(
                log.iter()
                    .filter(|line| !line["LoadPlugin"].is_null())
                    .count(),
                1
            );
            // Initial preparation/seek legitimately resets the processor. An
            // edit must add no lifecycle or panic commands after that baseline.
            for command in ["StartProcessing", "StopProcessing", "MidiPanic"] {
                let count =
                    |lines: &[Value]| lines.iter().filter(|line| **line == json!(command)).count();
                assert_eq!(
                    count(&log),
                    count(baseline),
                    "Unexpected {command} during edit"
                );
            }
            for command in ["LoadState", "SetTransportPosition", "Reconfigure"] {
                let count =
                    |lines: &[Value]| lines.iter().filter(|line| !line[command].is_null()).count();
                assert_eq!(
                    count(&log),
                    count(baseline),
                    "Unexpected {command} during edit"
                );
            }
            let processed: Vec<_> = log
                .iter()
                .filter_map(|line| line["processed"].as_u64())
                .collect();
            assert!(
                processed.windows(2).all(|pair| pair[1] > pair[0]),
                "DSP state must advance"
            );
        }
    }
}
fn wait(app: &mut Harness, predicate: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if predicate(&state) {
            return state;
        }
        assert!(Instant::now() < end, "Timed out: {state}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn settled(app: &mut Harness) -> Value {
    wait(app, |state| {
        state["engine"]["applied_revision"] == state["revision"]
    })
}
fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}
fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}
fn pointer(app: &mut Harness, phase: &str, from: (f64, f64), to: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer", "phase":phase, "x":from.0, "y":from.1, "to_x":to.0, "to_y":to.1}))
}
fn drag_channel(app: &mut Harness, id: u64, control: &str, target: f64) -> Value {
    let state = app.inspect();
    let channel = state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["track"] == id)
        .unwrap();
    let rect = &channel[control];
    let from = center(rect);
    let x = rect["min"][0].as_f64().unwrap()
        + (rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap()) * target;
    pointer(app, "drag", from, (x, from.1))
}
fn fit(app: &mut Harness) {
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| **label == "Fit")
        .count()
        - 1;
    app.command(json!({"command":"click", "label":"Fit", "occurrence":occurrence}));
}
fn undo(app: &mut Harness, redo: bool) {
    app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":redo}));
}
fn pump(app: &mut Harness, before: &Value, expected: [f64; 2], audible: bool) -> Value {
    pump_frames(app, before, expected, audible, 4096)
}
fn pump_frames(
    app: &mut Harness,
    before: &Value,
    expected: [f64; 2],
    audible: bool,
    frames: usize,
) -> Value {
    let state = app.command(json!({"command":"pump_audio", "frames":frames}));
    let engine = &state["engine"];
    assert_eq!(
        engine["engine_id"], before["engine"]["engine_id"],
        "No device/graph replacement"
    );
    assert_eq!(
        engine["generation"], before["engine"]["generation"],
        "No seek or queue discard"
    );
    assert_eq!(engine["playing"], true);
    assert_eq!(state["playback_loading"], false);
    assert_eq!(engine["render_errors"], 0);
    assert_eq!(engine["edit_errors"], before["engine"]["edit_errors"]);
    assert_eq!(engine["underruns"], before["engine"]["underruns"]);
    assert!(
        engine["position_beats"].as_f64().unwrap()
            > before["engine"]["position_beats"].as_f64().unwrap()
    );
    let samples = state["audio_output"].as_array().unwrap();
    assert_eq!(samples.len(), frames);
    for frame in samples {
        let left = frame[0].as_f64().unwrap();
        let right = frame[1].as_f64().unwrap();
        assert!(left.is_finite() && right.is_finite());
        if audible {
            assert!(
                left.abs() + right.abs() > 1e-5,
                "Unexpected silence during an edit"
            );
        }
    }
    // Existing queue samples intentionally play out. Verify the stable result
    // after the bounded queue and the fixture's 32-frame delay have drained.
    for frame in &samples[samples.len() - 512..] {
        for channel in 0..2 {
            assert!(
                (frame[channel].as_f64().unwrap() - expected[channel]).abs() < 1e-5,
                "PCM after edit: {frame}; expected {expected:?}"
            );
        }
    }
    state
}
fn expected_synth(state: &Value, id: u64, pitch: u8) -> [f64; 2] {
    let t = track(state, id);
    let gain = t["gain"].as_f64().unwrap();
    let pan = t["pan"].as_f64().unwrap();
    let value = (0.2 + (f64::from(pitch) - 60.0) * 0.01) * 0.5 * gain;
    [value * (1.0 - pan.max(0.0)), value * (1.0 + pan.min(0.0))]
}
fn note_rect(state: &Value, id: u64) -> Value {
    state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == id)
        .unwrap()["rect"]
        .clone()
}
fn clip_rect(state: &Value, id: u64) -> Value {
    state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"]
        .clone()
}

#[test]
fn mixer_and_midi_note_edits_keep_sandboxes_sustained_notes_and_continuous_pcm() {
    let fixture = Fixture::new();
    let (mut app, synth, _, held, future) = fixture.app();
    let initial = app.inspect();
    let mut previous = pump(&mut app, &initial, [0.1, 0.1], true);
    let ids = fixture.logs();
    let held_ons = fixture.note_events(60, true);
    let held_offs = fixture.note_events(60, false);
    assert_eq!(
        ids.len(),
        2,
        "Instrument and effect each own an isolated process"
    );
    assert!(held_ons >= 1);
    // A cosmetic edit and its undo must not even enqueue a playback revision.
    let revision = previous["revision"].clone();
    app.click("Window");
    app.click("Properties");
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| **label == "Sustained phrase")
        .count()
        - 1;
    app.command(json!({"command":"click", "label":"Sustained phrase", "occurrence":occurrence}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":"Renamed phrase"}));
    app.key("Escape");
    let renamed = app.inspect();
    assert_eq!(track(&renamed, synth)["clips"][0]["name"], "Renamed phrase");
    assert_eq!(renamed["revision"], revision);
    undo(&mut app, false);
    let restored = app.inspect();
    assert_eq!(
        track(&restored, synth)["clips"][0]["name"],
        "Sustained phrase"
    );
    assert_eq!(restored["revision"], revision);
    previous = pump(&mut app, &previous, [0.1, 0.1], true);
    fixture.assert_instances(&ids);
    // Render real blocks while the mouse remains held, not only after the
    // synthetic drag command has completed. Both intermediate fader positions
    // must reach the same running voice and count as one undoable gesture.
    let state = app.inspect();
    let rect = state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == synth)
        .unwrap()["gain"]
        .clone();
    let down = center(&rect);
    pointer(&mut app, "down", down, down);
    let mut last = down;
    let mut last_gain = json!(1.0);
    let at_fraction = |fraction| {
        (
            rect["min"][0].as_f64().unwrap()
                + (rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap()) * fraction,
            down.1,
        )
    };
    for fraction in [0.8, 0.6] {
        last = at_fraction(fraction);
        pointer(&mut app, "move", last, last);
        let state = settled(&mut app);
        assert_ne!(track(&state, synth)["gain"], last_gain);
        last_gain = track(&state, synth)["gain"].clone();
        previous = pump_frames(
            &mut app,
            &previous,
            expected_synth(&state, synth, 60),
            true,
            2048,
        );
        fixture.assert_instances(&ids);
        assert_eq!(fixture.note_events(60, true), held_ons);
        assert_eq!(fixture.note_events(60, false), held_offs);
    }
    pointer(&mut app, "up", last, last);
    // A second gesture returning to its starting value must restore the audio
    // preview without introducing an empty undo step.
    let before_return = app.inspect();
    pointer(&mut app, "down", last, last);
    for fraction in [0.8, 0.6] {
        last = at_fraction(fraction);
        pointer(&mut app, "move", last, last);
        let state = settled(&mut app);
        previous = pump_frames(
            &mut app,
            &previous,
            expected_synth(&state, synth, 60),
            true,
            2048,
        );
        fixture.assert_instances(&ids);
        assert_eq!(fixture.note_events(60, true), held_ons);
        assert_eq!(fixture.note_events(60, false), held_offs);
    }
    pointer(&mut app, "up", last, last);
    let after_return = app.inspect();
    assert_eq!(
        track(&after_return, synth)["gain"],
        track(&before_return, synth)["gain"]
    );
    assert_eq!(
        after_return["history"]["undo"],
        before_return["history"]["undo"]
    );
    undo(&mut app, false);
    let state = settled(&mut app);
    assert_eq!(
        track(&state, synth)["gain"],
        1.0,
        "One undo restores the entire live fader gesture"
    );
    previous = pump_frames(&mut app, &previous, [0.1, 0.1], true, 2048);
    fixture.assert_instances(&ids);
    for (control, target) in [("gain", 0.9), ("pan", 0.8), ("gain", 0.85), ("pan", 0.2)] {
        drag_channel(&mut app, synth, control, target);
        let state = settled(&mut app);
        previous = pump(&mut app, &previous, expected_synth(&state, synth, 60), true);
        fixture.assert_instances(&ids);
        assert_eq!(
            fixture.note_events(60, true),
            held_ons,
            "Fader edits must not retrigger a held note"
        );
        assert_eq!(fixture.note_events(60, false), held_offs);
    }
    undo(&mut app, false);
    let state = settled(&mut app);
    previous = pump(&mut app, &previous, expected_synth(&state, synth, 60), true);
    undo(&mut app, true);
    let state = settled(&mut app);
    previous = pump(&mut app, &previous, expected_synth(&state, synth, 60), true);
    app.click("Sustained phrase");
    app.click("Window");
    app.click("Clip Editor");
    fit(&mut app);
    let state = app.inspect();
    let rect = note_rect(&state, future);
    let from = center(&rect);
    let beat = state["midi_editor"]["pixels_per_beat"].as_f64().unwrap();
    pointer(&mut app, "drag", from, (from.0 + beat, from.1));
    let state = settled(&mut app);
    let notes = &track(&state, synth)["clips"][0]["content"]["Midi"]["notes"];
    assert_eq!(
        notes
            .as_array()
            .unwrap()
            .iter()
            .find(|note| note["id"] == future)
            .unwrap()["start"],
        21.0
    );
    previous = pump(&mut app, &previous, expected_synth(&state, synth, 60), true);
    assert_eq!(fixture.note_events(60, true), held_ons);
    assert_eq!(fixture.note_events(60, false), held_offs);

    // Change the sounding note itself. Only that voice is released/replaced;
    // the existing effect delay continues and the transport never seeks.
    let state = app.inspect();
    let from = center(&note_rect(&state, held));
    let row = state["midi_editor"]["row_height"].as_f64().unwrap();
    let to = (from.0, from.1 - row);
    pointer(&mut app, "down", from, from);
    pointer(&mut app, "move", to, to);
    let state = settled(&mut app);
    previous = pump_frames(
        &mut app,
        &previous,
        expected_synth(&state, synth, 61),
        true,
        2048,
    );
    fixture.assert_instances(&ids);
    pointer(&mut app, "up", to, to);
    let state = settled(&mut app);
    previous = pump(&mut app, &previous, expected_synth(&state, synth, 61), true);
    assert!(
        fixture.note_events(60, false) >= 1,
        "The old pitch must be released"
    );
    assert!(
        fixture.note_events(61, true) >= 1,
        "The replacement pitch must sound"
    );
    undo(&mut app, false);
    let state = settled(&mut app);
    previous = pump(&mut app, &previous, expected_synth(&state, synth, 60), true);
    fixture.assert_instances(&ids);

    let state = app.inspect();
    let from = center(&note_rect(&state, held));
    pointer(&mut app, "double_click", from, from);
    settled(&mut app);
    pump(&mut app, &previous, [0.0, 0.0], false);
    fixture.assert_instances(&ids);
    for (_, log) in fixture.logs() {
        if let Some(last) = log.iter().rev().find(|entry| entry["effect"] == false) {
            assert_eq!(last["active"], json!([]), "Deleted notes must not hang");
        }
    }
    app.quit();
}

#[test]
fn moving_trimming_splitting_duplicating_and_importing_clips_keep_live_dsp() {
    let fixture = Fixture::new();
    let (mut app, synth, audio, _, _) = fixture.app();
    let state = app.inspect();
    let mut previous = pump(&mut app, &state, [0.1, 0.1], true);
    let ids = fixture.logs();
    let held_ons = fixture.note_events(60, true);
    fit(&mut app);
    let state = app.inspect();
    let id = track(&state, audio)["clips"][0]["id"].as_u64().unwrap();
    let rect = clip_rect(&state, id);
    let from = center(&rect);
    let width = rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap();
    pointer(&mut app, "drag", from, (from.0 - width, from.1));
    let state = settled(&mut app);
    assert_eq!(track(&state, audio)["clips"][0]["start"], 0.0);
    previous = pump(&mut app, &previous, [0.2, 0.2], true);
    fixture.assert_instances(&ids);

    let state = app.inspect();
    let rect = clip_rect(&state, id);
    let width = rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap();
    let from = (rect["max"][0].as_f64().unwrap() - 2.0, center(&rect).1);
    pointer(&mut app, "drag", from, (from.0 - width * 0.25, from.1));
    let state = settled(&mut app);
    assert!(track(&state, audio)["clips"][0]["length"].as_f64().unwrap() < 4.0);
    previous = pump(&mut app, &previous, [0.2, 0.2], true);
    app.click("Edit");
    app.click("Split at cursor");
    let state = settled(&mut app);
    assert_eq!(track(&state, audio)["clips"].as_array().unwrap().len(), 2);
    previous = pump(&mut app, &previous, [0.2, 0.2], true);
    app.command(json!({"command":"key", "key":"D", "cmd":true}));
    let state = settled(&mut app);
    assert_eq!(track(&state, audio)["clips"].as_array().unwrap().len(), 3);
    previous = pump(&mut app, &previous, [0.2, 0.2], true);
    app.click("Sustained phrase");
    app.command(json!({"command":"key", "key":"D", "cmd":true}));
    let state = settled(&mut app);
    assert_eq!(track(&state, synth)["clips"].as_array().unwrap().len(), 2);
    previous = pump(&mut app, &previous, [0.2, 0.2], true);
    assert_eq!(
        fixture.note_events(60, true),
        held_ons,
        "Duplicating a future clip must preserve the sounding voice"
    );

    // A newly dropped source is decoded on the preparation worker while the
    // current queue and existing plugin instances keep their musical state.
    let imported = app.directory.path().join("imported-while-playing.wav");
    AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: vec![0.1; 88_200],
    }
    .write_wav(&imported)
    .unwrap();
    app.command(json!({"command":"drop", "paths":[imported], "track":2, "beat":0.0}));
    settled(&mut app);
    previous = pump(&mut app, &previous, [0.3, 0.3], true);
    fixture.assert_instances(&ids);
    // Undo and redo must travel through the same live path, including removing
    // a decoded source and restoring it from the shared preparation cache.
    undo(&mut app, false);
    settled(&mut app);
    previous = pump(&mut app, &previous, [0.2, 0.2], true);
    undo(&mut app, true);
    settled(&mut app);
    pump(&mut app, &previous, [0.3, 0.3], true);
    fixture.assert_instances(&ids);
    app.quit();
}

#[test]
fn stretching_midi_and_audio_preserves_running_plugins_sustain_and_output() {
    let fixture = Fixture::new();
    let (mut app, synth, audio, _, _) = fixture.app();
    let state = app.inspect();
    let mut previous = pump(&mut app, &state, [0.1, 0.1], true);
    let instances = fixture.logs();
    let note_ons = fixture.note_events(60, true);
    fit(&mut app);
    for (track_id, factor) in [(synth, 0.75), (audio, 2.0)] {
        let state = app.inspect();
        let clip = track(&state, track_id)["clips"][0]["id"].as_u64().unwrap();
        let rect = clip_rect(&state, clip);
        let width = rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap();
        let from = (rect["max"][0].as_f64().unwrap() - 2.0, center(&rect).1);
        let to = (from.0 + width * (factor - 1.0), from.1);
        app.command(json!({"command":"key", "key":"S"}));
        pointer(&mut app, "down", from, from);
        pointer(&mut app, "move", to, to);
        let state = settled(&mut app);
        let rate = track(&state, track_id)["clips"][0]["source_rate"]
            .as_f64()
            .unwrap();
        assert!((rate - 1.0 / factor).abs() < 1e-9);
        previous = pump_frames(&mut app, &previous, [0.1, 0.1], true, 2048);
        fixture.assert_instances(&instances);
        assert_eq!(
            fixture.note_events(60, true),
            note_ons,
            "Stretch must retain the held instrument voice"
        );
        pointer(&mut app, "up", to, to);
        settled(&mut app);
        undo(&mut app, false);
        let state = settled(&mut app);
        assert_eq!(track(&state, track_id)["clips"][0]["source_rate"], 1.0);
        previous = pump_frames(&mut app, &previous, [0.1, 0.1], true, 2048);
        fixture.assert_instances(&instances);
    }
    app.quit();
}

#[test]
fn rapid_edits_and_failed_source_preparation_keep_old_audio_then_recover() {
    let fixture = Fixture::new();
    let (mut app, synth, audio, _, _) = fixture.app();
    let state = app.inspect();
    let mut previous = pump(&mut app, &state, [0.1, 0.1], true);
    let ids = fixture.logs();
    // Submit without waiting between edits: whichever plans finish first, the
    // final heard values must be from the latest project revision.
    for target in [0.2, 0.8, 0.3, 0.7, 0.4, 0.6] {
        drag_channel(&mut app, synth, "pan", target);
    }
    let state = settled(&mut app);
    assert_eq!(state["engine"]["requested_revision"], state["revision"]);
    let expected = expected_synth(&state, synth, 60);
    previous = pump(&mut app, &previous, expected, true);
    fixture.assert_instances(&ids);

    // This is a valid importable WAV but its 1 Hz sample rate exceeds the
    // renderer's bounded conversion ratio. Failure is deterministic, without
    // file races, huge allocations, or delay hooks in production code.
    let unsupported = app.directory.path().join("unsupported-rate.wav");
    AudioData {
        sample_rate: 1,
        channels: 1,
        samples: vec![0.4],
    }
    .write_wav(&unsupported)
    .unwrap();
    let before = app.inspect();
    let count = track(&before, audio)["clips"].as_array().unwrap().len();
    // The background preparation may report its failure during Drop's UI settle
    // or on the next inspect; both timings represent the same rejected edit.
    app.raw_command(json!({"command":"drop", "paths":[unsupported], "track":1, "beat":0.0}));
    let deadline = Instant::now() + Duration::from_secs(15);
    let failed = loop {
        let state = app.inspect();
        if state["engine"]["edit_errors"].as_u64().unwrap_or(0) > 0 && state["error"] == true {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "Preparation failure not reported: {state}"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(
        track(&failed, audio)["clips"].as_array().unwrap().len(),
        count + 1
    );
    assert_eq!(
        failed["engine"]["engine_id"],
        previous["engine"]["engine_id"]
    );
    assert_eq!(
        failed["engine"]["applied_revision"],
        before["engine"]["applied_revision"]
    );
    assert!(failed["status"].as_str().unwrap().contains("range"));
    previous = pump(&mut app, &failed, expected, true);
    fixture.assert_instances(&ids);
    undo(&mut app, false);
    let recovered = settled(&mut app);
    assert_eq!(
        recovered["error"], false,
        "Successful undo clears its own preparation error"
    );
    assert_eq!(
        track(&recovered, audio)["clips"].as_array().unwrap().len(),
        count
    );
    previous = pump(&mut app, &previous, expected, true);
    drag_channel(&mut app, synth, "gain", 0.92);
    let recovered = settled(&mut app);
    previous = pump(
        &mut app,
        &previous,
        expected_synth(&recovered, synth, 60),
        true,
    );
    fixture.assert_instances(&ids);

    // A routing-graph change is outside the live-clip edit contract. It still
    // follows the existing controlled replacement path and resumes playback.
    app.click("Track");
    app.click("Add bus");
    let replacement = wait(&mut app, |state| {
        state["engine"]["playing"] == true
            && state["playback_loading"] == false
            && state["engine"]["engine_id"] != previous["engine"]["engine_id"]
    });
    pump(
        &mut app,
        &replacement,
        expected_synth(&replacement, synth, 60),
        true,
    );
    assert!(
        fixture.identities().len() > ids.len(),
        "Structural replacement has fresh sandbox owners"
    );
    app.quit();
}

#[test]
fn moving_trimming_and_removing_a_sounding_midi_clip_reconciles_only_its_voices() {
    let fixture = Fixture::new();
    let (mut app, synth, _, _, _) = fixture.app();
    let initial = app.inspect();
    let mut previous = pump(&mut app, &initial, [0.1, 0.1], true);
    let ids = fixture.logs();
    fit(&mut app);
    let state = app.inspect();
    let clip = track(&state, synth)["clips"][0]["id"].as_u64().unwrap();
    let rect = clip_rect(&state, clip);
    let width = rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap();
    let from = center(&rect);
    pointer(&mut app, "drag", from, (from.0 + width / 8.0, from.1));
    let state = settled(&mut app);
    assert_eq!(track(&state, synth)["clips"][0]["start"], 4.0);
    previous = pump(&mut app, &previous, [0.0, 0.0], false);
    fixture.assert_instances(&ids);
    undo(&mut app, false);
    settled(&mut app);
    previous = pump(&mut app, &previous, [0.1, 0.1], false);
    let ons = fixture.note_events(60, true);
    let offs = fixture.note_events(60, false);

    let state = app.inspect();
    let rect = clip_rect(&state, clip);
    let width = rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap();
    let from = (rect["max"][0].as_f64().unwrap() - 2.0, center(&rect).1);
    pointer(&mut app, "drag", from, (from.0 - width * 0.875, from.1));
    let state = settled(&mut app);
    assert!(track(&state, synth)["clips"][0]["length"].as_f64().unwrap() <= 4.25);
    previous = pump(&mut app, &previous, [0.1, 0.1], true);
    assert_eq!(
        fixture.note_events(60, true),
        ons,
        "Trimming the far end preserves the held voice"
    );
    assert_eq!(fixture.note_events(60, false), offs);
    app.key("Delete");
    let state = settled(&mut app);
    assert!(track(&state, synth)["clips"].as_array().unwrap().is_empty());
    previous = pump(&mut app, &previous, [0.0, 0.0], false);
    assert!(fixture.note_events(60, false) > offs);
    undo(&mut app, false);
    settled(&mut app);
    pump(&mut app, &previous, [0.1, 0.1], false);
    fixture.assert_instances(&ids);
    app.quit();
}
