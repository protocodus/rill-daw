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
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            for (path, commands) in self.logs() {
                eprintln!(
                    "Live audio fixture {} (last commands): {:?}",
                    path.display(),
                    &commands[commands.len().saturating_sub(12)..]
                );
            }
        }
    }
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
            // Keep the deterministic fixture comfortably inside the unchanged
            // production IPC deadline, even when debug E2Es run concurrently.
            .args(["-C", "opt-level=2"])
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
        self.app_with_audio(false)
    }
    fn app_with_audio(&self, tonal_audio: bool) -> (Harness, u64, u64, u64, u64) {
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
            samples: if tonal_audio {
                // Long enough to exercise the live proxy -> full-rate path.
                (0..48_000 * 8)
                    .map(|frame| {
                        (std::f64::consts::TAU * 440.0 * frame as f64 / 48_000.0).sin() as f32
                            * 0.04
                    })
                    .collect()
            } else {
                vec![0.1; 96_000]
            },
        }
        .write_wav(&path)
        .unwrap();
        let track = project.track_mut(audio).unwrap();
        track.gain = 1.0;
        track.clips.push(Clip::audio(
            "Future audio",
            if tonal_audio { 0.0 } else { 4.0 },
            if tonal_audio { 16.0 } else { 4.0 },
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
    assert_eq!(
        engine["render_errors"], 0,
        "Render failed: {engine}; status={}, error={}, details={}",
        state["status"], state["error"], state["error_details"]
    );
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
fn generation_preview_commit_and_undo_keep_running_plugins_and_continuous_pcm() {
    let fixture = Fixture::new();
    let (mut app, synth, _, held, _) = fixture.app();
    let initial = app.inspect();
    let mut previous = pump(&mut app, &initial, [0.1, 0.1], true);
    let instances = fixture.logs();
    let held_ons = fixture.note_events(60, true);
    let held_offs = fixture.note_events(60, false);
    assert_eq!(
        instances.len(),
        2,
        "Both instrument and effect are isolated"
    );
    assert!(held_ons > 0);

    app.click("Window");
    app.click("Clip Editor");
    let baseline = app.inspect();
    let original_notes = track(&baseline, synth)["clips"][0]["content"]["Midi"]["notes"].clone();
    let original_held = original_notes
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == held)
        .unwrap()
        .clone();
    let assert_preview_only = |state: &Value| {
        assert_eq!(state["project"], baseline["project"]);
        assert_eq!(state["history"], baseline["history"]);
        assert_eq!(state["revision"], baseline["revision"]);
        for field in ["requested_revision", "applied_revision"] {
            assert_eq!(state["engine"][field], baseline["engine"][field]);
        }
    };
    let assert_held = || {
        fixture.assert_instances(&instances);
        assert_eq!(fixture.note_events(60, true), held_ons);
        assert_eq!(fixture.note_events(60, false), held_offs);
    };

    let opened = app.click("Generate…");
    assert!(!opened["midi_editor"]["generate"].is_null());
    assert_preview_only(&opened);
    previous = pump(&mut app, &previous, [0.1, 0.1], true);
    assert_held();

    let melody = app.click("Melody");
    let varied = app.click("New variation");
    assert_ne!(
        varied["midi_editor"]["generate"]["seed"],
        melody["midi_editor"]["generate"]["seed"]
    );
    assert_ne!(
        varied["midi_editor"]["generate"]["notes"],
        melody["midi_editor"]["generate"]["notes"]
    );
    assert_preview_only(&varied);
    previous = pump(&mut app, &previous, [0.1, 0.1], true);
    assert_held();

    let cancelled = app.click("Cancel");
    assert!(cancelled["midi_editor"]["generate"].is_null());
    assert_preview_only(&cancelled);
    previous = pump(&mut app, &previous, [0.1, 0.1], true);
    assert_held();

    app.click("Generate…");
    app.click("Chord");
    // Range controls are edited through their actual text fields. Put the draft
    // well ahead of playback so every sampled frame still belongs to the held
    // note and persistent effect, making a restart or retrigger observable.
    for (field, value) in [("start_bar", "3"), ("length_bars", "1")] {
        let state = app.inspect();
        let rect = &state["midi_editor"]["generate"]["controls"][field];
        assert!(!rect.is_null(), "Missing range control {field}");
        let point = center(rect);
        pointer(&mut app, "double_click", point, point);
        app.command(json!({"command":"key", "key":"A", "cmd":true}));
        app.command(json!({"command":"text", "text":value}));
        app.key("Enter");
    }
    let preview = app.inspect();
    assert_preview_only(&preview);
    let draft = preview["midi_editor"]["generate"]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(draft.len(), 3);
    assert!(
        draft
            .iter()
            .all(|note| note["start"] == 8.0 && note["length"] == 4.0)
    );
    let covered: Vec<_> = draft
        .iter()
        .filter(|note| note["pitch"] == original_held["pitch"])
        .collect();
    assert_eq!(covered.len(), 1);
    assert!(
        covered[0]["start"].as_f64().unwrap() >= original_held["start"].as_f64().unwrap()
            && covered[0]["start"].as_f64().unwrap() + covered[0]["length"].as_f64().unwrap()
                <= original_held["start"].as_f64().unwrap()
                    + original_held["length"].as_f64().unwrap(),
        "The draft C is already covered by the playing note"
    );
    let new_tones: Vec<_> = draft
        .iter()
        .filter(|note| note["pitch"] != original_held["pitch"])
        .collect();
    assert_eq!(
        new_tones
            .iter()
            .map(|note| note["pitch"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![64, 67]
    );
    app.click("Add notes");
    let added = settled(&mut app);
    assert!(added["midi_editor"]["generate"].is_null());
    let notes = track(&added, synth)["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(notes.len(), original_notes.as_array().unwrap().len() + 2);
    let added_notes: Vec<_> = notes
        .iter()
        .filter(|note| {
            !original_notes
                .as_array()
                .unwrap()
                .iter()
                .any(|old| old["id"] == note["id"])
        })
        .collect();
    assert_eq!(
        added_notes.len(),
        2,
        "Only uncovered draft tones are inserted"
    );
    for expected in new_tones {
        assert!(added_notes.iter().any(|actual| {
            ["pitch", "start", "length", "velocity"]
                .into_iter()
                .all(|field| actual[field] == expected[field])
        }));
    }
    assert_eq!(
        notes.iter().find(|note| note["id"] == held).unwrap(),
        &original_held
    );
    assert_eq!(notes.iter().filter(|note| note["pitch"] == 60).count(), 1);
    assert!(
        added["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["id"] == held && note["selected"] == true),
        "The covered generated C selects the existing held identity"
    );
    assert_eq!(
        added["history"]["undo"].as_array().unwrap().len(),
        baseline["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert!(added["revision"].as_u64().unwrap() > baseline["revision"].as_u64().unwrap());
    previous = pump(&mut app, &previous, [0.1, 0.1], true);
    assert_held();

    undo(&mut app, false);
    let restored = settled(&mut app);
    assert_eq!(restored["project"], baseline["project"]);
    assert_eq!(restored["history"]["undo"], baseline["history"]["undo"]);
    let final_audio = pump(&mut app, &previous, [0.1, 0.1], true);
    assert!(final_audio["engine"]["position_beats"].as_f64().unwrap() < 4.0);
    assert_held();
    undo(&mut app, true);
    let redone = settled(&mut app);
    assert_eq!(redone["project"], added["project"]);
    let final_audio = pump(&mut app, &final_audio, [0.1, 0.1], true);
    assert!(final_audio["engine"]["position_beats"].as_f64().unwrap() < 4.0);
    assert_held();
    app.quit();
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
    // Split retains both cues as a group. This scenario intentionally copies
    // only its right-hand cue while the same DSP instances keep sounding.
    let right = state["selected_clip"].as_u64().unwrap();
    let at = center(&clip_rect(&state, right));
    app.command(
        json!({"command":"pointer","phase":"click","button":"secondary","x":at.0,"y":at.1}),
    );
    app.click("Select only this clip");
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
fn audio_pitch_and_algorithm_edits_keep_running_plugins_queue_and_audible_output() {
    let fixture = Fixture::new();
    let (mut app, _, audio_track, _, _) = fixture.app_with_audio(true);
    let initial = app.inspect();
    let close_pitch = |app: &mut Harness| {
        for _ in 0..3 {
            let state = app.inspect();
            if !state["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|text| text == "Transpose")
            {
                return;
            }
            app.key("Escape");
        }
        panic!("Pitch popup did not dismiss");
    };
    let pump_tone = |app: &mut Harness, previous: &Value, pitch: f64| {
        let state = app.command(json!({"command":"pump_audio", "frames":8192}));
        for field in [
            "engine_id",
            "generation",
            "underruns",
            "render_errors",
            "edit_errors",
        ] {
            assert_eq!(
                state["engine"][field], previous["engine"][field],
                "{field} changed during pitch editing: engine={}, status={}, error={}, details={}",
                state["engine"], state["status"], state["error"], state["error_details"]
            );
        }
        assert_eq!(state["engine"]["playing"], true);
        assert_eq!(state["playback_loading"], false);
        assert!(
            state["engine"]["position_beats"].as_f64().unwrap()
                > previous["engine"]["position_beats"].as_f64().unwrap()
        );
        let samples = state["audio_output"].as_array().unwrap();
        assert_eq!(samples.len(), 8192);
        for frame in samples {
            let left = frame[0].as_f64().unwrap();
            let right = frame[1].as_f64().unwrap();
            assert!(left.is_finite() && right.is_finite());
            assert!(
                left.abs() + right.abs() > 0.01,
                "a held synth voice must cover the edit continuously"
            );
        }
        // The held fixture instrument contributes exactly 0.1. Removing that
        // DC level exposes the audio clip's actual sounding pitch after queued
        // pre-edit blocks and the existing effect delay have played through.
        let tone: Vec<_> = samples[4096..]
            .iter()
            .map(|frame| frame[0].as_f64().unwrap() - 0.1)
            .collect();
        let crossings = tone
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        let frequency = crossings as f64 * 48_000.0 / tone.len() as f64;
        let expected = 440.0 * 2.0_f64.powf(pitch / 12.0);
        assert!(
            (frequency - expected).abs() < 18.0,
            "live pitch {frequency} Hz, expected {expected} Hz"
        );
        let rms =
            (tone.iter().map(|sample| sample * sample).sum::<f64>() / tone.len() as f64).sqrt();
        assert!(
            (0.01..0.06).contains(&rms),
            "the edited audio voice must keep sounding: {rms}"
        );
        state
    };
    let mut previous = pump_tone(&mut app, &initial, 0.0);
    let instances = fixture.logs();
    let note_ons = fixture.note_events(60, true);
    let note_offs = fixture.note_events(60, false);
    app.click("Future audio");
    app.click("Window");
    app.click("Clip Editor");
    for (pitch, algorithm) in [
        (12.0, "Signalsmith"),
        (12.0, "Bungee"),
        (-12.0, "Bungee"),
        (-12.0, "Signalsmith"),
    ] {
        let state = app.inspect();
        let processing = &track(&state, audio_track)["clips"][0]["content"]["Audio"]["processing"];
        let old_pitch = processing["transpose_semitones"].as_f64().unwrap();
        app.click(&if old_pitch == 0.0 {
            "Pitch".into()
        } else {
            format!("Pitch {old_pitch:+.2} st")
        });
        if old_pitch != pitch {
            let state = app.inspect();
            let rect = &state["audio_transpose_rect"];
            let at = (
                (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap()) * 0.5,
                (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap()) * 0.5,
            );
            pointer(&mut app, "double_click", at, at);
            app.command(json!({"command":"key", "key":"A", "cmd":true}));
            app.command(json!({"command":"text", "text":pitch.to_string()}));
            app.key("Enter");
        }
        let old_algorithm = processing["algorithm"].as_str().unwrap();
        if old_algorithm != algorithm {
            app.click(old_algorithm);
            app.click(algorithm);
        }
        close_pitch(&mut app);
        let state = settled(&mut app);
        let processing = &track(&state, audio_track)["clips"][0]["content"]["Audio"]["processing"];
        assert_eq!(processing["transpose_semitones"], pitch);
        assert_eq!(processing["algorithm"], algorithm);
        previous = pump_tone(&mut app, &previous, pitch);
        wait(&mut app, |state| {
            state["engine"]["applied_revision"] == state["revision"]
                && state["engine"]["refining_audio"] == false
        });
        // The final pass must keep the same pitch, transport and sandbox DSP
        // state when its full-bandwidth asset replaces the interactive proxy.
        previous = pump_tone(&mut app, &previous, pitch);
        fixture.assert_instances(&instances);
        assert_eq!(fixture.note_events(60, true), note_ons);
        assert_eq!(fixture.note_events(60, false), note_offs);
    }
    for redo in [false, true] {
        undo(&mut app, redo);
        settled(&mut app);
        previous = pump_tone(&mut app, &previous, -12.0);
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
