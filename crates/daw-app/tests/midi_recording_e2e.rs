//! Real UI → timestamped dummy MIDI → capture/live plug-in → project/export.
//! The protocol worker is a deterministic instrument, not a native VST test.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Instrument, Note, Project};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

const RATE: u32 = 48_000;
const BEAT: usize = 24_000;

struct Fixture {
    workers: tempfile::TempDir,
    plugins: tempfile::TempDir,
    host: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Ok(entries) = std::fs::read_dir(self.workers.path())
        {
            for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
                if path
                    .extension()
                    .is_none_or(|extension| extension != "jsonl")
                {
                    continue;
                }
                let Ok(contents) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let commands: Vec<_> = contents.lines().collect();
                eprintln!(
                    "MIDI fixture {} (last commands): {:?}",
                    path.display(),
                    &commands[commands.len().saturating_sub(16)..]
                );
            }
        }
    }
}

impl Fixture {
    fn new() -> Self {
        let workers = tempfile::tempdir().unwrap();
        let source = workers.path().join("midi_recording_host.rs");
        std::fs::write(&source, include_str!("common/live_audio_host.rs")).unwrap();
        let host = workers.path().join(if cfg!(windows) {
            "midi-recording-host.exe"
        } else {
            "midi-recording-host"
        });
        let build = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .args(["-C", "opt-level=2"])
            .arg(&source)
            .arg("-o")
            .arg(&host)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        Self {
            workers,
            plugins: common::plugin_fixture(),
            host,
        }
    }

    fn app(&self, backing: bool, audio: bool, looping: bool) -> (Harness, u64, Option<u64>) {
        let mut app = Harness::with_virtual_audio(Some(self.plugins.path()), Some(&self.host));
        let mut p = Project::new();
        p.tempo = 120.0;
        p.loop_enabled = looping;
        p.loop_start = 0.0;
        p.loop_end = 1.0;
        let midi = p.add_midi_track("Input synth");
        p.set_instrument(
            midi,
            Instrument::Vst3 {
                path: self
                    .plugins
                    .path()
                    .join("RillFixture.vst3")
                    .to_string_lossy()
                    .into_owned(),
                class_id: Some("11111111111111111111111111111111".into()),
            },
        )
        .unwrap();
        p.track_mut(midi).unwrap().gain = 1.0;
        if backing {
            p.track_mut(midi).unwrap().clips.push(Clip::midi(
                "Existing phrase",
                0.0,
                if looping { 1.0 } else { 16.0 },
                vec![Note::new(48, 0.0, if looping { 1.0 } else { 16.0 }, 100)],
            ));
        }
        let audio = audio.then(|| {
            let id = p.add_audio_track("Audio capture");
            p.track_mut(id).unwrap().gain = 1.0;
            id
        });
        let path = app.directory.path().join("midi-recording.json");
        p.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.click("Input synth");
        (app, midi, audio)
    }

    fn logs(&self) -> Vec<(PathBuf, Vec<Value>)> {
        let mut logs: Vec<_> = std::fs::read_dir(self.workers.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .map(|path| {
                let values = std::fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect();
                (path, values)
            })
            .collect();
        logs.sort_by(|a, b| a.0.cmp(&b.0));
        logs
    }

    fn assert_instances(&self, baseline: &[(PathBuf, Vec<Value>)]) {
        let current = self.logs();
        assert_eq!(current.len(), baseline.len());
        for ((path, commands), (old_path, old_commands)) in current.iter().zip(baseline) {
            assert_eq!(
                path, old_path,
                "Timing controls restarted the instrument process"
            );
            for command in [
                "LoadPlugin",
                "StartProcessing",
                "StopProcessing",
                "MidiPanic",
                "LoadState",
                "SetTransportPosition",
                "Reconfigure",
            ] {
                let count = |events: &[Value]| {
                    events
                        .iter()
                        .filter(|event| **event == json!(command) || !event[command].is_null())
                        .count()
                };
                assert_eq!(
                    count(commands),
                    count(old_commands),
                    "{command} changed during timing control"
                );
            }
        }
    }
}

fn project(state: &Value) -> Project {
    serde_json::from_value(state["project"].clone()).unwrap()
}
fn wait(app: &mut Harness, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = app.inspect();
        if predicate(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "MIDI recording did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}
fn popup(app: &mut Harness) -> Value {
    menu(app, "Window", "Recording")
}
fn arm_in_header(app: &mut Harness, track: u64) -> Value {
    let state = app.inspect();
    let arm = &state["timeline"]["recording_controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|control| control["track"] == track)
        .unwrap()["arm"];
    app.command(json!({"command":"pointer","phase":"click",
        "x":(arm["min"][0].as_f64().unwrap()+arm["max"][0].as_f64().unwrap())*0.5,
        "y":(arm["min"][1].as_f64().unwrap()+arm["max"][1].as_f64().unwrap())*0.5}))
}
fn expose_popup_label(app: &mut Harness, label: &str) {
    for _ in 0..8 {
        let state = app.inspect();
        let window = state["window_rects"]
            .as_array()
            .unwrap()
            .last()
            .expect("Open popup");
        let inside = state["label_rects"].as_array().unwrap().iter().any(|item| {
            item["text"] == label
                && item["min"][0].as_f64().unwrap() >= window["min"][0].as_f64().unwrap()
                && item["min"][1].as_f64().unwrap() >= window["min"][1].as_f64().unwrap()
                && item["max"][1].as_f64().unwrap() <= window["max"][1].as_f64().unwrap()
        });
        if inside {
            return;
        }
        app.command(json!({"command":"scroll", "delta":-220.0,
            "x":(window["min"][0].as_f64().unwrap()+window["max"][0].as_f64().unwrap())*0.5,
            "y":(window["min"][1].as_f64().unwrap()+window["max"][1].as_f64().unwrap())*0.5}));
    }
    panic!("Popup control {label:?} is unreachable in the compact viewport");
}
fn select_midi_channel(app: &mut Harness, current: &str, channel: &str) {
    app.click(current);
    for _ in 0..8 {
        let state = app.inspect();
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .contains(&json!(channel))
        {
            break;
        }
        let row = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            // Combo contents are painted after the selected-value button.
            // Scroll over an option, not the "Channel 16" button behind it.
            .rev()
            .find(|row| {
                row["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("Channel "))
            })
            .expect("Open MIDI channel picker");
        let delta = if matches!(channel, "All channels" | "No input") {
            350.0
        } else {
            -350.0
        };
        app.command(json!({"command":"scroll", "delta":delta,
            "x":(row["min"][0].as_f64().unwrap()+row["max"][0].as_f64().unwrap())*0.5,
            "y":(row["min"][1].as_f64().unwrap()+row["max"][1].as_f64().unwrap())*0.5}));
    }
    app.click(channel);
}
fn configure_midi(app: &mut Harness, channel: &str, monitor: &str, armed: bool) {
    let state = popup(app);
    if state["settings"]["midi_input_device"].is_null() {
        app.click("No MIDI input");
        app.click("Rill virtual MIDI input");
    }
    select_midi_channel(app, "All channels", channel);
    if armed {
        app.click("Arm recording");
    }
    let state = app.inspect();
    let track = state["selected_track"].as_u64().unwrap().to_string();
    let current = state["recording"]["monitors"][&track]
        .as_str()
        .unwrap_or("Off");
    if monitor != current {
        app.click(current);
        app.click(monitor);
    }
    app.key("Escape");
}
fn precount(app: &mut Harness, bars: usize) {
    popup(app);
    app.click("No pre-count");
    app.click(match bars {
        0 => "No pre-count",
        1 => "1 bar",
        2 => "2 bars",
        _ => unreachable!(),
    });
    app.key("Escape");
}
fn start(app: &mut Harness, counting: bool) -> Value {
    app.click("Record");
    let state = wait(app, |state| {
        state["recording"]["phase"] == if counting { "counting_in" } else { "recording" }
    });
    assert_eq!(state["error"], false, "{}", state["status"]);
    state
}
fn stop(app: &mut Harness) -> Value {
    app.click("Stop");
    let state = wait(app, |state| state["recording"]["phase"] == "idle");
    assert_eq!(state["error"], false, "{}", state["status"]);
    state
}
fn midi(app: &mut Harness, frame: usize, bytes: [u8; 3]) -> Value {
    app.command(json!({"command":"midi_input", "bytes":bytes, "offset_frames":frame}))
}
fn pump(app: &mut Harness, frames: usize) -> Value {
    let mut remaining = frames;
    let mut samples = Vec::new();
    let mut state = Value::Null;
    while remaining > 0 {
        let count = remaining.min(8192);
        state = app.command(json!({"command":"pump_audio", "frames":count}));
        samples.extend(state["audio_output"].as_array().unwrap().iter().cloned());
        remaining -= count;
    }
    state["audio_output"] = Value::Array(samples);
    state
}
fn output(state: &Value) -> Vec<[f32; 2]> {
    state["audio_output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| {
            [
                frame[0].as_f64().unwrap() as f32,
                frame[1].as_f64().unwrap() as f32,
            ]
        })
        .collect()
}
fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}
#[track_caller]
fn settled(state: &Value, expected: f32) {
    let frames = output(state);
    assert!(frames.len() >= 256);
    for (index, frame) in frames.iter().enumerate().skip(frames.len() - 256) {
        assert!(
            frame.iter().all(|sample| (*sample - expected).abs() < 1e-5),
            "Monitored frame {index}: {frame:?}, expected {expected}; engine={}, MIDI={}, status={}, error={}",
            state["engine"],
            state["recording"]["midi"],
            state["status"],
            state["error"],
        );
    }
}
fn notes(clip: &Clip) -> &[Note] {
    let ClipContent::Midi { notes } = &clip.content else {
        panic!("Expected a MIDI clip")
    };
    notes
}
fn assert_notes(clip: &Clip, expected: &[(u8, usize, usize, u8)]) {
    let mut actual: Vec<_> = notes(clip)
        .iter()
        .map(|note| {
            (
                note.pitch,
                ((clip.start + (note.start - clip.offset) / clip.source_rate) * BEAT as f64).round()
                    as usize,
                (note.length / clip.source_rate * BEAT as f64).round() as usize,
                note.velocity,
            )
        })
        .collect();
    let mut expected = expected.to_vec();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    for note in notes(clip) {
        assert!((note.start * BEAT as f64 - (note.start * BEAT as f64).round()).abs() < 1e-6);
        assert!((note.length * BEAT as f64 - (note.length * BEAT as f64).round()).abs() < 1e-6);
    }
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export", "path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn selected_midi_channel_records_exact_chords_and_velocities_and_exports_the_instrument() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(false, false, false);
    configure_midi(&mut app, "Channel 2", "Off", true);
    start(&mut app, false);
    // The unselected channel must never enter either the document or instrument.
    midi(&mut app, 0, [0x90, 72, 127]);
    midi(&mut app, 100, [0x91, 60, 100]);
    midi(&mut app, 100, [0x91, 64, 80]);
    midi(&mut app, 300, [0x91, 67, 60]);
    midi(&mut app, 2100, [0x81, 60, 0]);
    midi(&mut app, 2400, [0x81, 64, 0]);
    midi(&mut app, 3300, [0x81, 67, 0]);
    midi(&mut app, 3400, [0x80, 72, 0]);
    pump(&mut app, 4000);
    let state = stop(&mut app);
    let p = project(&state);
    let clips = &p.track(track).unwrap().clips;
    assert_eq!(clips.len(), 1);
    assert_notes(
        &clips[0],
        &[
            (60, 100, 2000, 100),
            (64, 100, 2300, 80),
            (67, 300, 3000, 60),
        ],
    );
    let rendered = export(&mut app, "recorded-chord.wav");
    assert_eq!(rendered.sample_rate, RATE);
    assert_eq!(rendered.channels, 2);
    for (frame, stereo) in rendered.samples[..8000]
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
    {
        let expected = if (100..2100).contains(&frame) {
            0.2
        } else {
            0.0
        } + if (100..2400).contains(&frame) {
            0.24 * 0.8
        } else {
            0.0
        } + if (300..3300).contains(&frame) {
            0.27 * 0.6
        } else {
            0.0
        };
        near(stereo[0], expected);
        near(stereo[1], expected);
    }
    assert!(
        !fixture
            .logs()
            .iter()
            .flat_map(|(_, events)| events)
            .any(|event| event["SendMidiAt"]["event"]["NoteOn"]["note"] == 72)
    );
    app.quit();
}

#[test]
fn retrigger_sustain_zero_velocity_and_held_stop_produce_precise_note_lengths() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(false, false, false);
    configure_midi(&mut app, "All channels", "Off", true);
    start(&mut app, false);
    for (at, bytes) in [
        (0, [0x90, 60, 90]),
        (2400, [0x90, 60, 70]),
        (4800, [0x80, 60, 0]),
        (6000, [0x90, 64, 80]),
        (7000, [0xb0, 64, 127]),
        (8000, [0x80, 64, 0]),
        (10000, [0xb0, 64, 0]),
        (11000, [0x90, 67, 100]),
        (12000, [0x90, 67, 0]),
        (13000, [0x90, 72, 50]),
    ] {
        midi(&mut app, at, bytes);
    }
    pump(&mut app, 15000);
    let state = stop(&mut app);
    let p = project(&state);
    let clips = &p.track(track).unwrap().clips;
    assert_eq!(clips.len(), 1);
    assert_notes(
        &clips[0],
        &[
            (60, 0, 2400, 90),
            (60, 2400, 2400, 70),
            (64, 6000, 4000, 80),
            (67, 11000, 1000, 100),
            (72, 13000, 2000, 50),
        ],
    );
    assert_eq!(state["recording"]["midi"]["held_notes"], 0);
    app.quit();
}

#[test]
fn loop_recording_splits_held_notes_and_overdubs_without_changing_existing_notes() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(true, false, true);
    let original = project(&app.inspect()).track(track).unwrap().clips[0].clone();
    configure_midi(&mut app, "All channels", "Off", true);
    start(&mut app, false);
    midi(&mut app, 23000, [0x90, 60, 100]);
    midi(&mut app, 25000, [0x80, 60, 0]);
    midi(&mut app, 30000, [0x90, 64, 80]);
    midi(&mut app, 32000, [0x80, 64, 0]);
    pump(&mut app, 36000);
    let state = stop(&mut app);
    let p = project(&state);
    let clips = &p.track(track).unwrap().clips;
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[0], original);
    assert_notes(
        &clips[1],
        &[
            (60, 23000, 1000, 100),
            (60, 0, 1000, 100),
            (64, 6000, 2000, 80),
        ],
    );
    let rendered = export(&mut app, "loop-overdub.wav");
    for (frame, stereo) in rendered.samples[..BEAT * 2]
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
    {
        let expected =
            0.08 + if !(1000..23000).contains(&frame) {
                0.2
            } else {
                0.0
            } + if (6000..8000).contains(&frame) {
                0.24 * 0.8
            } else {
                0.0
            };
        near(stereo[0], expected);
        near(stereo[1], expected);
    }
    app.quit();
}

#[test]
fn midi_capture_is_one_undo_transaction_and_archive_recall_preserves_routes_without_arming() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(false, false, false);
    configure_midi(&mut app, "Channel 3", "Off", true);
    let before = app.inspect()["project"].clone();
    start(&mut app, false);
    midi(&mut app, 240, [0x92, 65, 93]);
    midi(&mut app, 2400, [0x82, 65, 0]);
    pump(&mut app, 3000);
    let recorded = stop(&mut app);
    let undo = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undo["project"], before);
    let redo = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redo["project"], recorded["project"]);
    let path = app.directory.path().join("midi-recorded.rill");
    app.command(json!({"command":"save","path":path}));
    app = app.restart();
    let recalled = app.command(json!({"command":"open","path":path}));
    assert_eq!(recalled["project"], recorded["project"]);
    assert_eq!(recalled["recording"]["armed"], json!([]));
    assert_eq!(recalled["recording"]["midi"]["input_ready"], false);
    assert_eq!(
        recalled["settings"]["midi_input_device"],
        "rill-virtual-midi"
    );
    assert_notes(
        &project(&recalled).track(track).unwrap().clips[0],
        &[(65, 240, 2160, 93)],
    );
    app.quit();
}

fn capture_with_precount(bars: usize) {
    let fixture = Fixture::new();
    let (mut app, midi_track, audio_track) = fixture.app(false, true, false);
    let auto_monitor = bars == 1;
    configure_midi(
        &mut app,
        "All channels",
        if auto_monitor { "Auto" } else { "Off" },
        true,
    );
    app.click("Audio capture");
    popup(&mut app);
    app.click("Arm recording");
    app.key("Escape");
    app.click("Input synth");
    precount(&mut app, bars);
    start(&mut app, bars > 0);
    let count = bars * 4 * BEAT;
    if count > 0 {
        midi(&mut app, 0, [0x90, 60, 100]);
    }
    midi(&mut app, count, [0x90, 64, 100]);
    if count > 0 {
        midi(&mut app, count + 1000, [0x80, 60, 0]);
    }
    midi(&mut app, count + 2400, [0x80, 64, 0]);
    if count > 0 {
        let before = app.command(
            json!({"command":"pump_input","channels":2,"samples":vec![-0.75; (count-1)*2]}),
        );
        assert_eq!(before["recording"]["phase"], "counting_in");
        assert_eq!(before["engine"]["position_beats"], 0.0);
        assert_eq!(before["recording"]["frames"], 0);
        let mut clicks = output(&before);
        clicks.extend(output(&app.command(
            json!({"command":"pump_input","channels":2,"samples":[-0.75,-0.75]}),
        )));
        assert_eq!(clicks.len(), count);
        // MIDI audition is suspended while the timeline is prepared. Count-in
        // output therefore contains only the click, including in Auto mode.
        let mut rms = Vec::new();
        for beat in 0..bars * 4 {
            let begin = beat * BEAT;
            near(clicks[begin][0], 0.0);
            assert!(
                clicks[begin + 1][0].abs() > 1e-7,
                "Missing click at beat {beat}"
            );
            let energy = clicks[begin..begin + 960]
                .iter()
                .map(|frame| f64::from(frame[0]).powi(2))
                .sum::<f64>()
                / 960.0;
            rms.push(energy.sqrt());
            for frame in &clicks[begin + 960..begin + BEAT] {
                near(frame[0], 0.0);
                near(frame[1], 0.0);
            }
        }
        for (beat, value) in rms.iter().enumerate() {
            let ratio = value / rms[beat / 4 * 4 + 1];
            if beat % 4 == 0 {
                assert!(
                    (1.4..1.7).contains(&ratio),
                    "Downbeat was not accented: {ratio}"
                );
            } else {
                assert!((ratio - 1.0).abs() < 0.001);
            }
        }
    }
    let expected: Vec<f32> = (0..2401)
        .map(|frame| 0.1 + frame as f32 / 100_000.0)
        .collect();
    let input: Vec<f32> = expected.iter().flat_map(|left| [*left, -*left]).collect();
    let captured = app.command(json!({"command":"pump_input","channels":2,"samples":input}));
    if auto_monitor {
        let live = output(&captured);
        assert!(
            live.iter().any(|frame| (frame[0] - 0.44).abs() < 1e-5),
            "Count-in preview and new live note did not overlap"
        );
        assert!(
            live.iter().any(|frame| (frame[0] - 0.24).abs() < 1e-5),
            "Auto monitoring dropped the first recorded note"
        );
    }
    let state = stop(&mut app);
    let p = project(&state);
    let midi_clips = &p.track(midi_track).unwrap().clips;
    assert_eq!(midi_clips.len(), 1);
    assert_notes(&midi_clips[0], &[(64, 0, 2400, 100)]);
    let audio_clips = &p.track(audio_track.unwrap()).unwrap().clips;
    assert_eq!(audio_clips.len(), 1);
    let ClipContent::Audio { path, .. } = &audio_clips[0].content else {
        panic!("Missing audio capture")
    };
    assert_eq!(AudioData::load_wav(path).unwrap().samples, expected);
    let rendered = export(&mut app, &format!("precount-{bars}.wav"));
    // Count-in is a monitoring aid, so neither its sentinel input nor its click
    // may leak into the document or the exported audio.
    for (frame, stereo) in rendered.samples[..4802]
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
    {
        let wanted = expected[frame] + if frame < 2400 { 0.24 } else { 0.0 };
        near(stereo[0], wanted);
        near(stereo[1], wanted);
    }
    app.quit();
}

#[test]
fn no_precount_records_audio_and_midi_from_the_first_frame() {
    capture_with_precount(0)
}
#[test]
fn one_bar_precount_holds_the_cursor_accents_clicks_and_excludes_early_audio_and_midi() {
    capture_with_precount(1)
}
#[test]
fn two_bar_precount_holds_the_cursor_accents_clicks_and_excludes_early_audio_and_midi() {
    capture_with_precount(2)
}

#[test]
fn stopping_during_precount_creates_no_audio_or_midi_clip_and_leaves_the_project_usable() {
    let fixture = Fixture::new();
    let (mut app, _, _) = fixture.app(false, true, false);
    configure_midi(&mut app, "All channels", "Off", true);
    app.click("Audio capture");
    popup(&mut app);
    app.click("Arm recording");
    app.key("Escape");
    app.click("Input synth");
    precount(&mut app, 2);
    let before = app.inspect()["project"].clone();
    start(&mut app, true);
    midi(&mut app, 0, [0x90, 60, 100]);
    app.command(json!({"command":"pump_input","channels":2,"samples":vec![0.7;2048]}));
    let stopped = stop(&mut app);
    assert_eq!(stopped["project"], before);
    assert_eq!(stopped["recording"]["midi"]["held_notes"], 0);
    start(&mut app, true);
    app.command(json!({"command":"pump_input","channels":2,"samples":vec![0.7;2048]}));
    app.click("Stop recording");
    let cancelled = wait(&mut app, |state| state["recording"]["phase"] == "idle");
    assert_eq!(cancelled["project"], before);
    assert_ne!(cancelled["engine"]["playing"], true);
    // Cancelling the count must also clear playback intent: a later structural
    // edit rebuilds the graph, but must not start transport unexpectedly.
    menu(&mut app, "Track", "Add audio track");
    for _ in 0..16 {
        let changed = wait(&mut app, |state| state["playback_loading"] == false);
        assert_ne!(changed["engine"]["playing"], true);
        assert_eq!(changed["timeline"]["cursor"], 0.0);
    }
    menu(&mut app, "File", "New");
    let prompt = wait(&mut app, |state| {
        state["document_guard"]["phase"] == "confirm"
    });
    assert_eq!(prompt["recording"]["phase"], "idle");
    app.click("Don't save");
    wait(&mut app, |state| state["project"]["tracks"] == json!([]));
    app.quit();
}

#[test]
fn metronome_toggles_during_playback_keep_plugin_instances_and_never_change_export() {
    let fixture = Fixture::new();
    let (mut app, _, _) = fixture.app(true, false, false);
    app.click("Play");
    wait(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    let before = pump(&mut app, 4096);
    settled(&before, 0.08);
    let workers = fixture.logs();
    assert_eq!(workers.len(), 1);
    app.click("Click");
    let clicked = pump(&mut app, BEAT * 4);
    assert_eq!(
        clicked["engine"]["engine_id"],
        before["engine"]["engine_id"]
    );
    assert_eq!(
        clicked["engine"]["generation"],
        before["engine"]["generation"]
    );
    let samples = output(&clicked);
    for (offset, frame) in samples.iter().enumerate() {
        let absolute = 4096 + offset;
        if absolute % BEAT >= 960 {
            near(frame[0], 0.08);
            near(frame[1], 0.08);
        }
    }
    assert!(samples.iter().any(|frame| (frame[0] - 0.08).abs() > 0.02));
    app.click("Click");
    let silent = pump(&mut app, 4096);
    settled(&silent, 0.08);
    fixture.assert_instances(&workers);
    app.click("Click");
    app.click("Stop");
    let rendered = export(&mut app, "without-metronome.wav");
    for stereo in rendered.samples[..BEAT * 4 * 2].as_chunks::<2>().0 {
        near(stereo[0], 0.08);
        near(stereo[1], 0.08);
    }
    app.quit();
}

#[test]
fn live_midi_monitor_plays_filtered_chords_and_sustain_through_the_isolated_instrument() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(false, false, false);
    configure_midi(&mut app, "Channel 2", "On", false);
    wait(&mut app, |state| {
        state["recording"]["midi"]["input_ready"] == true
    });
    midi(&mut app, 0, [0x90, 72, 127]);
    midi(&mut app, 0, [0x91, 60, 100]);
    midi(&mut app, 0, [0x91, 64, 50]);
    let chord = pump(&mut app, 4096);
    settled(&chord, 0.32);
    assert_eq!(chord["engine"]["playing"], false);
    assert!(project(&chord).track(track).unwrap().clips.is_empty());
    midi(&mut app, 0, [0xb1, 64, 127]);
    midi(&mut app, 0, [0x81, 60, 0]);
    midi(&mut app, 0, [0x81, 64, 0]);
    let sustained = pump(&mut app, 4096);
    settled(&sustained, 0.32);
    assert_eq!(
        sustained["engine"]["engine_id"],
        chord["engine"]["engine_id"]
    );
    assert_eq!(
        sustained["engine"]["consumed_frames"].as_u64().unwrap(),
        chord["engine"]["consumed_frames"].as_u64().unwrap() + 4096,
        "UI frames releasing piano-roll audition must preserve MIDI monitoring: {}",
        sustained["engine"]
    );
    for (index, frame) in output(&sustained).iter().enumerate() {
        assert!(
            frame.iter().all(|sample| (*sample - 0.32).abs() < 1e-5),
            "Sustain interrupted at frame {index}: {frame:?}; engine={}",
            sustained["engine"]
        );
    }
    midi(&mut app, 0, [0xb1, 64, 0]);
    let released = pump(&mut app, 4096);
    settled(&released, 0.0);
    assert!(project(&released).track(track).unwrap().clips.is_empty());
    midi(&mut app, 0, [0x91, 67, 100]);
    midi(&mut app, 128, [0x81, 67, 0]);
    midi(&mut app, 256, [0x91, 69, 100]);
    midi(&mut app, 384, [0x91, 69, 100]);
    midi(&mut app, 512, [0x81, 69, 0]);
    settled(&pump(&mut app, 4096), 0.0);
    let logs = fixture.logs();
    assert_eq!(logs.len(), 1);
    for pitch in [60, 64] {
        assert!(
            logs[0]
                .1
                .iter()
                .any(|event| event["SendMidiAt"]["event"]["NoteOn"]["note"] == pitch)
        );
        assert!(
            logs[0]
                .1
                .iter()
                .any(|event| event["SendMidiAt"]["event"]["NoteOff"]["note"] == pitch)
        );
    }
    assert!(
        !logs[0]
            .1
            .iter()
            .any(|event| event["SendMidiAt"]["event"]["NoteOn"]["note"] == 72)
    );
    for (pitch, count) in [(67, 1), (69, 2)] {
        for kind in ["NoteOn", "NoteOff"] {
            assert_eq!(
                logs[0]
                    .1
                    .iter()
                    .filter(|event| event["SendMidiAt"]["event"][kind]["note"] == pitch)
                    .count(),
                count,
                "Lost short-note/retrigger {kind} for pitch{pitch}"
            );
        }
    }
    app.quit();
}

#[test]
fn recording_while_playing_skips_precount_and_stopping_capture_keeps_the_instrument_running() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(true, false, false);
    configure_midi(&mut app, "All channels", "Auto", true);
    precount(&mut app, 2);
    app.click("Play");
    wait(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    let before = pump(&mut app, 4096);
    settled(&before, 0.08);
    let workers = fixture.logs();
    start(&mut app, false);
    midi(&mut app, 0, [0x90, 60, 100]);
    midi(&mut app, 2400, [0x80, 60, 0]);
    let during = pump(&mut app, 4096);
    let audible = output(&during);
    assert!(audible.iter().any(|frame| (frame[0] - 0.28).abs() < 1e-5));
    settled(&during, 0.08);
    app.click("Stop recording");
    let stopped = wait(&mut app, |state| state["recording"]["phase"] == "idle");
    assert_eq!(stopped["engine"]["playing"], true);
    assert_eq!(
        stopped["engine"]["engine_id"],
        before["engine"]["engine_id"]
    );
    assert_eq!(
        stopped["engine"]["generation"],
        before["engine"]["generation"]
    );
    fixture.assert_instances(&workers);
    let p = project(&stopped);
    let clips = &p.track(track).unwrap().clips;
    assert_eq!(clips.len(), 2);
    assert_notes(&clips[1], &[(60, 4096, 2400, 100)]);
    settled(&pump(&mut app, 4096), 0.08);
    app.click("Stop");
    app.quit();
}

#[test]
fn midi_disconnect_retains_held_notes_releases_the_instrument_and_allows_retry() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(false, false, false);
    configure_midi(&mut app, "All channels", "Auto", true);
    start(&mut app, false);
    midi(&mut app, 0, [0x90, 60, 100]);
    settled(&pump(&mut app, 4096), 0.2);
    app.command(json!({"command":"midi_error"}));
    let failed = wait(&mut app, |state| {
        state["recording"]["phase"] == "idle" && state["error"] == true
    });
    let p = project(&failed);
    assert_eq!(p.track(track).unwrap().clips.len(), 1);
    assert_notes(&p.track(track).unwrap().clips[0], &[(60, 0, 4096, 100)]);
    assert!(
        failed["recording"]["error"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("midi")
    );
    app.click("Dismiss");
    popup(&mut app);
    app.click("Retry input");
    wait(&mut app, |state| {
        state["recording"]["midi"]["input_ready"] == true
    });
    app.key("Escape");
    settled(&pump(&mut app, 4096), 0.0);
    app.click("Stop");
    start(&mut app, false);
    midi(&mut app, 0, [0x90, 67, 80]);
    midi(&mut app, 2000, [0x80, 67, 0]);
    pump(&mut app, 3000);
    let state = stop(&mut app);
    let p = project(&state);
    assert_eq!(p.track(track).unwrap().clips.len(), 2);
    assert_notes(&p.track(track).unwrap().clips[1], &[(67, 0, 2000, 80)]);
    app.quit();
}

#[test]
fn midi_save_new_cancel_and_sample_rate_change_finalize_notes_before_the_action() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(false, false, false);
    configure_midi(&mut app, "All channels", "Off", true);
    start(&mut app, false);
    midi(&mut app, 0, [0x90, 60, 100]);
    pump(&mut app, 1200);
    let path = app.directory.path().join("saved-mid-take.rill");
    let saved = app.command(json!({"command":"save","path":path}));
    assert_eq!(saved["recording"]["phase"], "idle");
    assert_eq!(saved["modified"], false);
    assert_notes(
        &project(&saved).track(track).unwrap().clips[0],
        &[(60, 0, 1200, 100)],
    );
    app.click("Stop");
    start(&mut app, false);
    midi(&mut app, 0, [0x90, 64, 80]);
    pump(&mut app, 1800);
    menu(&mut app, "File", "New");
    let prompt = wait(&mut app, |state| {
        state["document_guard"]["phase"] == "confirm"
    });
    assert_eq!(prompt["recording"]["phase"], "idle");
    assert_eq!(project(&prompt).track(track).unwrap().clips.len(), 2);
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], prompt["project"]);
    app.click("Stop");
    start(&mut app, false);
    midi(&mut app, 0, [0x90, 67, 70]);
    pump(&mut app, 2400);
    menu(&mut app, "Window", "Setup");
    app.click("Audio");
    app.click("48000 Hz");
    app.click("44100 Hz");
    app.click("Apply");
    let changed = wait(&mut app, |state| {
        state["recording"]["phase"] == "idle" && state["settings"]["sample_rate"] == 44100
    });
    let p = project(&changed);
    assert_eq!(p.track(track).unwrap().clips.len(), 3);
    assert_notes(&p.track(track).unwrap().clips[2], &[(67, 0, 2400, 70)]);
    assert_eq!(changed["error"], false);
    app.quit();
}

#[test]
fn an_unselected_midi_channel_records_no_empty_clip() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(false, false, false);
    app.command(json!({"command":"viewport","width":800,"height":600}));
    configure_midi(&mut app, "Channel 16", "Off", false);
    let armed = arm_in_header(&mut app, track);
    assert_eq!(armed["recording"]["armed"], json!([track]));
    assert_eq!(armed["recording"]["monitors"][track.to_string()], "Auto");
    popup(&mut app);
    app.click("Auto");
    app.click("Off");
    expose_popup_label(&mut app, "No pre-count");
    expose_popup_label(&mut app, "Click");
    expose_popup_label(&mut app, "Record");
    app.key("Escape");
    let before = app.inspect()["project"].clone();
    start(&mut app, false);
    midi(&mut app, 0, [0x90, 60, 100]);
    midi(&mut app, 1000, [0x80, 60, 0]);
    pump(&mut app, 2000);
    let state = stop(&mut app);
    assert_eq!(state["project"], before);
    assert!(project(&state).track(track).unwrap().clips.is_empty());
    popup(&mut app);
    select_midi_channel(&mut app, "Channel 16", "No input");
    app.key("Escape");
    app.click("Record");
    let disabled = app.inspect();
    assert_ne!(disabled["recording"]["phase"], "recording");
    assert!(project(&disabled).track(track).unwrap().clips.is_empty());
    if disabled["error"] == true {
        app.click("Dismiss");
    }
    app.key("Escape");
    app.quit();
}

#[test]
fn arming_without_a_midi_port_is_quiet_until_a_port_is_selected() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app(false, false, false);
    let armed = arm_in_header(&mut app, track);
    assert_eq!(armed["recording"]["armed"], json!([track]));
    assert_eq!(armed["recording"]["monitors"][track.to_string()], "Auto");
    for _ in 0..16 {
        let state = app.inspect();
        assert!(state["settings"]["midi_input_device"].is_null());
        assert_eq!(state["error"], false, "{}", state["status"]);
        assert_eq!(state["recording"]["midi"]["input_opening"], false);
        assert_eq!(state["recording"]["midi"]["input_ready"], false);
        assert_eq!(state["recording"]["phase"], "idle");
    }
    configure_midi(&mut app, "All channels", "Auto", false);
    wait(&mut app, |state| {
        state["recording"]["midi"]["input_ready"] == true
    });
    midi(&mut app, 0, [0x90, 60, 100]);
    settled(&pump(&mut app, 4096), 0.2);
    midi(&mut app, 0, [0x80, 60, 0]);
    settled(&pump(&mut app, 4096), 0.0);
    assert!(
        project(&app.inspect())
            .track(track)
            .unwrap()
            .clips
            .is_empty()
    );
    start(&mut app, false);
    midi(&mut app, 0, [0x90, 64, 80]);
    midi(&mut app, 1200, [0x80, 64, 0]);
    pump(&mut app, 2000);
    let state = stop(&mut app);
    assert_notes(
        &project(&state).track(track).unwrap().clips[0],
        &[(64, 0, 1200, 80)],
    );
    app.quit();
}
