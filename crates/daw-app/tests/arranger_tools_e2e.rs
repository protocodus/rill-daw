//! Common arranger gestures through the running app, for both audio and MIDI.
//! Sources, recall, and exported PCM are observed without native UI or hardware.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
enum Kind {
    Audio,
    Midi,
}

#[test]
fn trimming_very_short_audio_and_midi_clips_keeps_the_app_and_source_window_valid() {
    for kind in [Kind::Audio, Kind::Midi] {
        let (mut app, id, _) = fixture(kind);
        let mut project: Project =
            serde_json::from_value(app.inspect()["project"].clone()).unwrap();
        project.tracks[0].clips[0].length = 0.01;
        let path = app.directory.path().join("tiny-clip.daw.json");
        project.save(&path).unwrap();
        let state = app.command(json!({"command":"open", "path":path}));
        let at = (
            rect(&state, id)["min"][0].as_f64().unwrap() + 0.25,
            center(&state, id).1,
        );
        app.key("Q");
        let state = drag(&mut app, at, 24.0, true);
        assert_eq!(state["error"], false, "{state}");
        near(value(&state, id, "length"), 0.01);
        near(value(&state, id, "offset"), 0.0);
        near(value(&state, id, "start"), 2.0);
        app.quit();
    }
}

#[test]
fn compact_toolbar_keeps_every_tool_mouse_accessible_at_compact_and_wide_sizes() {
    let (mut app, _, _) = fixture(Kind::Midi);
    let project = app.inspect()["project"].clone();
    for width in [800.0, 1100.0, 1800.0] {
        app.command(json!({"command":"viewport", "width":width, "height":900.0}));
        for (label, tool) in [
            ("Select", "Select"),
            ("Pencil", "Pencil"),
            ("Cut", "Cut"),
            ("Duplicate", "Duplicate"),
            ("Join", "Glue"),
            ("Ghost copy", "Ghost"),
            ("Stretch", "Stretch"),
            ("Slip", "Slip"),
            ("Erase", "Erase"),
            ("Cursor", "Cursor"),
        ] {
            if !matches!(label, "Select" | "Pencil" | "Cursor" | "Cut") {
                app.click("Select");
                app.click("More tools");
            }
            let state = app.inspect();
            let rect = state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == label)
                .unwrap();
            assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
            assert!(
                rect["max"][0].as_f64().unwrap() <= width,
                "{label} must fit at {width}"
            );
            app.click(label);
            let state = app.inspect();
            assert_eq!(state["timeline"]["tool"], tool, "{label} at {width}");
            assert_eq!(
                state["project"], project,
                "Tool navigation does not edit clips"
            );
        }
    }
    app.quit();
}

fn fixture(kind: Kind) -> (Harness, u64, Option<PathBuf>) {
    fixture_with_playback(kind, false)
}

fn fixture_with_playback(kind: Kind, playback: bool) -> (Harness, u64, Option<PathBuf>) {
    let mut app = if playback {
        Harness::with_virtual_audio(None, None)
    } else {
        Harness::new()
    };
    app.command(json!({"command":"viewport","width":1800.0,"height":1000.0}));
    let mut project = Project::new();
    project.tempo = 120.0;
    if playback {
        project.loop_enabled = true;
        project.loop_start = 2.0;
        project.loop_end = 6.0;
    }
    let (track, clip, source) = match kind {
        Kind::Audio => {
            let path = app.directory.path().join("tone.wav");
            AudioData {
                sample_rate: 48_000,
                channels: 2,
                samples: (0..96_000)
                    .flat_map(|frame| {
                        let sample =
                            (std::f32::consts::TAU * 440.0 * frame as f32 / 48_000.0).sin() * 0.25;
                        [sample, sample * -0.5]
                    })
                    .collect(),
            }
            .write_wav(&path)
            .unwrap();
            (
                project.add_audio_track("Audio tools"),
                Clip::audio("Source", 2.0, 4.0, path.to_string_lossy()),
                Some(path),
            )
        }
        Kind::Midi => (
            project.add_midi_track("MIDI tools"),
            Clip::midi("Source", 2.0, 4.0, vec![Note::new(60, 1.0, 2.0, 100)]),
            None,
        ),
    };
    let id = clip.id;
    let track = project.track_mut(track).unwrap();
    track.gain = 1.0;
    track.clips.push(clip);
    let path = app.directory.path().join("arranger-tools.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Source");
    (app, id, source)
}

fn clips(state: &Value) -> &[Value] {
    state["project"]["tracks"][0]["clips"].as_array().unwrap()
}
fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == id)
        .unwrap()
}
fn selected(state: &Value) -> u64 {
    state["selected_clip"].as_u64().unwrap()
}
fn value(state: &Value, id: u64, key: &str) -> f64 {
    clip(state, id)[key].as_f64().unwrap()
}
fn source_content(state: &Value, id: u64) -> Value {
    let mut content = clip(state, id)["content"].clone();
    if let Some(audio) = content.get_mut("Audio").and_then(Value::as_object_mut) {
        // Trimming can make the original fade window explicit without changing
        // the source audio or its processing settings.
        audio.remove("envelope");
    }
    content
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-4, "{actual} != {expected}");
}
fn rect(state: &Value, id: u64) -> &Value {
    &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"]
}
fn center(state: &Value, id: u64) -> (f64, f64) {
    let r = rect(state, id);
    (
        (r["min"][0].as_f64().unwrap() + r["max"][0].as_f64().unwrap()) * 0.5,
        (r["min"][1].as_f64().unwrap() + r["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn edge(state: &Value, id: u64, right: bool) -> (f64, f64) {
    let r = rect(state, id);
    (
        if right {
            r["max"][0].as_f64().unwrap() - 2.0
        } else {
            r["min"][0].as_f64().unwrap() + 2.0
        },
        center(state, id).1,
    )
}
fn pixels(state: &Value, beats: f64) -> f64 {
    state["timeline"]["pixels_per_beat"].as_f64().unwrap() * beats
}
fn pointer(app: &mut Harness, phase: &str, at: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":at.0,"y":at.1}))
}
fn drag(app: &mut Harness, at: (f64, f64), delta: f64, shift: bool) -> Value {
    app.command(json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":at.0+delta,"to_y":at.1,"shift":shift}))
}
fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}
fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}
fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}
fn wait_join(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if state["join_pending"] == false {
            assert_eq!(state["error"], false, "{state}");
            return state;
        }
        assert!(Instant::now() < deadline, "Join did not finish: {state}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn export_exact(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    let state = menu(app, "File", "Export WAV…");
    if state["export_include_effect_tails"] == true {
        app.click("Include effect tails");
    }
    app.command(json!({"command":"file_dialog","kind":"export","path":path}));
    app.click("Export…");
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn assert_pitch(audio: &AudioData, from: f64, to: f64) {
    let start = (from * f64::from(audio.sample_rate)) as usize;
    let end = (to * f64::from(audio.sample_rate)) as usize;
    let channel: Vec<_> = audio
        .samples
        .chunks_exact(audio.channels)
        .skip(start)
        .take(end - start)
        .map(|frame| frame[0])
        .collect();
    let crossings = channel
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    let frequency = crossings as f64 * f64::from(audio.sample_rate) / channel.len() as f64;
    assert!(
        (frequency - 440.0).abs() < 4.0,
        "Stretch must retain 440 Hz, measured {frequency}"
    );
    let rms = (channel
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / channel.len() as f64)
        .sqrt();
    assert!(rms > 0.1, "Stretched signal was lost: rms={rms}");
}

#[test]
fn common_duplicate_cut_ghost_and_join_tools_select_the_result_for_audio_and_midi() {
    for kind in [Kind::Audio, Kind::Midi] {
        let (mut app, original, _) = fixture(kind);
        let state = app.key("R");
        assert_eq!(state["timeline"]["tool"], "Duplicate");
        let copied = pointer(&mut app, "click", center(&state, original));
        let first = selected(&copied);
        assert_ne!(
            first, original,
            "{kind:?}: duplicate tool selects its result"
        );
        near(value(&copied, first, "start"), 6.0);
        assert!(clip(&copied, first)["link_group"].is_null());
        let repeated = app.command(json!({"command":"key","key":"D","cmd":true}));
        let second = selected(&repeated);
        assert_ne!(second, first);
        near(value(&repeated, second, "start"), 10.0);
        let state = app.key("E");
        let split = pointer(&mut app, "click", center(&state, second));
        let right = selected(&split);
        assert_ne!(right, second, "{kind:?}: cut selects the right-hand result");
        near(value(&split, second, "length"), 2.0);
        near(value(&split, right, "start"), 12.0);
        near(value(&split, right, "offset"), 2.0);
        let state = app.key("G");
        let ghosted = pointer(&mut app, "click", center(&state, right));
        let ghost = selected(&ghosted);
        assert_ne!(ghost, right);
        near(value(&ghosted, ghost, "start"), 14.0);
        assert!(!clip(&ghosted, ghost)["link_group"].is_null());
        assert_eq!(
            clip(&ghosted, ghost)["link_group"],
            clip(&ghosted, right)["link_group"]
        );
        assert_eq!(
            clip(&ghosted, ghost)["content"],
            clip(&ghosted, right)["content"]
        );
        let copied = app.command(json!({"command":"key","key":"D","cmd":true}));
        let independent = selected(&copied);
        near(value(&copied, independent, "start"), 16.0);
        assert!(clip(&copied, independent)["link_group"].is_null());
        let state = app.key("T");
        assert!(
            visible(&state, "Join"),
            "{kind:?}: arranger uses a contextual Join label"
        );
        let popup = pointer(&mut app, "click", center(&state, independent));
        assert_eq!(popup["popups"]["join"], true);
        assert_eq!(
            popup["project"], copied["project"],
            "Opening Join does not edit clips"
        );
        let dismissed = app.key("Escape");
        assert_eq!(dismissed["popups"]["join"], false);
        assert_eq!(dismissed["project"], copied["project"]);
        app.quit();
    }
}

#[test]
fn shift_body_drag_duplicates_once_and_keeps_the_entire_gesture_in_one_undo_step() {
    for kind in [Kind::Audio, Kind::Midi] {
        let (mut app, original_id, _) = fixture(kind);
        let original = app.key("Q");
        let from = center(&original, original_id);
        let mut shift_pointer = |phase: &str, delta: f64| {
            app.command(json!({"command":"pointer","phase":phase,
                "x":from.0+delta,"y":from.1,"shift":true}))
        };
        shift_pointer("down", 0.0);
        let first = shift_pointer("move", pixels(&original, 2.0));
        let copied_id = selected(&first);
        assert_ne!(
            copied_id, original_id,
            "{kind:?}: drag selects its duplicate"
        );
        assert_eq!(clips(&first).len(), 2);
        near(value(&first, copied_id, "start"), 4.0);
        let second = shift_pointer("move", pixels(&original, 4.0));
        assert_eq!(selected(&second), copied_id);
        assert_eq!(clips(&second).len(), 2, "One copy throughout the held drag");
        near(value(&second, copied_id, "start"), 6.0);
        assert_eq!(clip(&second, original_id), clip(&original, original_id));
        assert!(clip(&second, copied_id)["link_group"].is_null());
        let completed = shift_pointer("up", pixels(&original, 4.0));
        assert_eq!(completed["history"]["undo"].as_array().unwrap().len(), 1);
        assert_eq!(undo(&mut app, false)["project"], original["project"]);
        assert_eq!(undo(&mut app, true)["project"], completed["project"]);
        app.quit();
    }
}

#[test]
fn both_edge_trims_shift_snap_bypass_and_stretch_are_distinct_single_undo_gestures() {
    for kind in [Kind::Audio, Kind::Midi] {
        let (mut app, id, _) = fixture(kind);
        let original = app.key("Q");
        let content = source_content(&original, id);
        let left = drag(
            &mut app,
            edge(&original, id, false),
            pixels(&original, 1.0),
            false,
        );
        near(value(&left, id, "start"), 3.0);
        near(value(&left, id, "length"), 3.0);
        near(value(&left, id, "offset"), 1.0);
        near(value(&left, id, "source_rate"), 1.0);
        assert_eq!(source_content(&left, id), content);
        let right = drag(&mut app, edge(&left, id, true), pixels(&left, 1.0), false);
        near(value(&right, id, "length"), 4.0);
        near(value(&right, id, "offset"), 1.0);
        near(value(&right, id, "source_rate"), 1.0);
        undo(&mut app, false);
        let restored = undo(&mut app, false);
        assert_eq!(restored["project"], original["project"]);
        let free = drag(
            &mut app,
            edge(&restored, id, true),
            pixels(&restored, 0.37),
            true,
        );
        near(value(&free, id, "length"), 4.37);
        let restored = undo(&mut app, false);
        assert_eq!(restored["project"], original["project"]);
        let state = app.key("S");
        assert_eq!(state["timeline"]["tool"], "Stretch");
        let from = edge(&state, id, true);
        let delta = pixels(&state, 4.0);
        pointer(&mut app, "down", from);
        let middle = pointer(&mut app, "move", (from.0 + delta * 0.5, from.1));
        near(value(&middle, id, "length"), 6.0);
        near(value(&middle, id, "source_rate"), 4.0 / 6.0);
        let final_drag = pointer(&mut app, "move", (from.0 + delta, from.1));
        near(value(&final_drag, id, "length"), 8.0);
        near(value(&final_drag, id, "source_rate"), 0.5);
        assert_eq!(final_drag["history"]["undo"], original["history"]["undo"]);
        let stretched = pointer(&mut app, "up", (from.0 + delta, from.1));
        assert_eq!(
            stretched["history"]["undo"].as_array().unwrap().len(),
            1,
            "{kind:?}: one held stretch, one Undo"
        );
        near(value(&stretched, id, "start"), 2.0);
        near(value(&stretched, id, "offset"), 0.0);
        assert_eq!(
            source_content(&stretched, id),
            content,
            "Stretch retains source data"
        );
        let from = edge(&stretched, id, true);
        let delta = pixels(&stretched, 1.0);
        pointer(&mut app, "down", from);
        pointer(&mut app, "move", (from.0 + delta, from.1));
        pointer(&mut app, "move", from);
        let reverted = pointer(&mut app, "up", from);
        assert_eq!(reverted["project"], stretched["project"]);
        assert_eq!(reverted["history"]["undo"], stretched["history"]["undo"]);
        assert_eq!(undo(&mut app, false)["project"], original["project"]);
        assert_eq!(undo(&mut app, true)["project"], stretched["project"]);
        let state = app.key("Q");
        let trimmed = drag(
            &mut app,
            edge(&state, id, false),
            pixels(&state, 2.0),
            false,
        );
        near(value(&trimmed, id, "start"), 4.0);
        near(value(&trimmed, id, "length"), 6.0);
        near(value(&trimmed, id, "offset"), 1.0);
        near(value(&trimmed, id, "source_rate"), 0.5);
        assert_eq!(undo(&mut app, false)["project"], stretched["project"]);
        let state = app.key("E");
        let split = pointer(&mut app, "click", center(&state, id));
        let right = selected(&split);
        assert_ne!(right, id);
        near(value(&split, right, "start"), 6.0);
        near(value(&split, right, "length"), 4.0);
        near(value(&split, right, "offset"), 2.0);
        near(value(&split, right, "source_rate"), 0.5);
        assert_eq!(undo(&mut app, false)["project"], stretched["project"]);
        app.quit();
    }
}

#[test]
fn ghost_stretch_is_local_survives_portable_recall_and_join_preserves_rendered_timing() {
    for kind in [Kind::Audio, Kind::Midi] {
        let (mut app, original, source) = fixture(kind);
        let state = app.key("G");
        let linked = pointer(&mut app, "click", center(&state, original));
        let ghost = selected(&linked);
        let state = app.key("S");
        let stretched = drag(
            &mut app,
            edge(&state, ghost, true),
            pixels(&state, 4.0),
            false,
        );
        near(value(&stretched, original, "start"), 2.0);
        near(value(&stretched, original, "length"), 4.0);
        near(value(&stretched, original, "source_rate"), 1.0);
        near(value(&stretched, ghost, "start"), 6.0);
        near(value(&stretched, ghost, "length"), 8.0);
        near(value(&stretched, ghost, "source_rate"), 0.5);
        assert_eq!(
            clip(&stretched, ghost)["link_group"],
            clip(&stretched, original)["link_group"]
        );
        assert_eq!(
            clip(&stretched, ghost)["content"],
            clip(&stretched, original)["content"]
        );
        let archive = app.directory.path().join("stretched-ghost.rill");
        app.command(json!({"command":"save","path":archive}));
        if let Some(source) = source {
            std::fs::remove_file(source).unwrap();
        }
        let restored = app.command(json!({"command":"open","path":archive}));
        near(value(&restored, ghost, "source_rate"), 0.5);
        near(value(&restored, original, "source_rate"), 1.0);
        assert_eq!(
            clip(&restored, ghost)["link_group"],
            clip(&restored, original)["link_group"]
        );
        let rendered = export_exact(&mut app, "stretched.wav");
        assert_eq!(rendered.frames(), rendered.sample_rate as usize * 7);
        assert!(rendered.samples.iter().all(|sample| sample.is_finite()));
        if matches!(kind, Kind::Audio) {
            assert_pitch(&rendered, 1.25, 2.75);
            assert_pitch(&rendered, 3.3, 6.7);
            for frame in rendered.samples.as_chunks::<2>().0 {
                assert!(
                    (frame[0] + frame[1] * 2.0).abs() < 0.002,
                    "Stretch must keep stereo channels aligned"
                );
            }
        } else {
            assert!(
                rendered.samples.iter().all(|sample| *sample == 0.0),
                "Unassigned MIDI stays silent"
            );
        }
        let state = app.key("T");
        let popup = pointer(&mut app, "click", center(&state, ghost));
        assert_eq!(popup["popups"]["join"], true);
        app.click("Join");
        let joined = wait_join(&mut app);
        assert_eq!(clips(&joined).len(), 1);
        let id = selected(&joined);
        near(value(&joined, id, "start"), 2.0);
        near(value(&joined, id, "length"), 12.0);
        near(value(&joined, id, "source_rate"), 1.0);
        assert!(clip(&joined, id)["link_group"].is_null());
        if matches!(kind, Kind::Midi) {
            let notes = clip(&joined, id)["content"]["Midi"]["notes"]
                .as_array()
                .unwrap();
            assert_eq!(notes.len(), 2);
            near(notes[0]["start"].as_f64().unwrap(), 1.0);
            near(notes[0]["length"].as_f64().unwrap(), 2.0);
            near(notes[1]["start"].as_f64().unwrap(), 6.0);
            near(notes[1]["length"].as_f64().unwrap(), 4.0);
        }
        let joined_audio = export_exact(&mut app, "joined.wav");
        assert_eq!(joined_audio.frames(), rendered.frames());
        let max_error = joined_audio
            .samples
            .iter()
            .zip(&rendered.samples)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_error < 0.002,
            "Joining must preserve stretched PCM: {max_error}"
        );
        app.quit();
    }
}

#[test]
fn switching_editors_resets_unsupported_tools_without_editing_the_project() {
    let (mut app, id, _) = fixture(Kind::Midi);
    let original = app.inspect()["project"].clone();
    menu(&mut app, "Window", "Clip Editor");
    let multi = app.key("U");
    assert_eq!(multi["midi_editor"]["tool"], "MultiDraw");
    let arranger = menu(&mut app, "Window", "Arrangement");
    assert_eq!(arranger["timeline"]["tool"], "Select");
    assert_eq!(arranger["project"], original);
    let state = app.key("S");
    assert_eq!(state["timeline"]["tool"], "Stretch");
    let editor = menu(&mut app, "Window", "Clip Editor");
    assert_eq!(editor["midi_editor"]["tool"], "Select");
    assert_eq!(editor["project"], original);
    let state = menu(&mut app, "Window", "Arrangement");
    let selected = pointer(&mut app, "click", center(&state, id));
    assert_eq!(selected["project"], original);
    assert_eq!(selected["selected_clip"], id);
    app.quit();
}

#[test]
fn stretching_and_undo_keep_the_running_audio_graph_and_continuous_pcm() {
    let (mut app, id, _) = fixture_with_playback(Kind::Audio, true);
    let state = app.key("P");
    pointer(&mut app, "click", center(&state, id));
    app.key("Space");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if state["engine"]["playing"] == true {
            break;
        }
        assert_eq!(state["error"], false, "{state}");
        assert!(Instant::now() < deadline, "Virtual playback did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
    let initial = pump_tone(&mut app, 2048);
    let state = app.key("S");
    let stretched = drag(&mut app, edge(&state, id, true), pixels(&state, 4.0), false);
    near(value(&stretched, id, "source_rate"), 0.5);
    wait_audio_revision(&mut app, &initial);
    let applied = pump_tone(&mut app, 2048);
    assert_same_engine(&initial, &applied);
    let undone = undo(&mut app, false);
    near(value(&undone, id, "source_rate"), 1.0);
    wait_audio_revision(&mut app, &initial);
    let restored = pump_tone(&mut app, 2048);
    assert_same_engine(&initial, &restored);
    assert!(
        restored["engine"]["consumed_frames"].as_u64().unwrap()
            > initial["engine"]["consumed_frames"].as_u64().unwrap()
    );
    app.quit();
}

fn assert_same_engine(before: &Value, after: &Value) {
    for key in ["engine_id", "generation"] {
        assert_eq!(after["engine"][key], before["engine"][key], "Changed {key}");
    }
    assert_eq!(after["engine"]["playing"], true);
    assert_eq!(after["engine"]["render_errors"], 0);
    assert_eq!(after["engine"]["edit_errors"], 0);
}

fn pump_tone(app: &mut Harness, frames: usize) -> Value {
    let state = app.command(json!({"command":"pump_audio","frames":frames}));
    let output = state["audio_output"].as_array().unwrap();
    assert_eq!(output.len(), frames);
    let mut silent_run = 0;
    for frame in output {
        let left = frame[0].as_f64().unwrap();
        let right = frame[1].as_f64().unwrap();
        assert!(left.is_finite() && right.is_finite());
        silent_run = if left.abs() < 1e-7 { silent_run + 1 } else { 0 };
        assert!(
            silent_run < 16,
            "A live stretch inserted silence into the sustained tone"
        );
    }
    state
}

fn wait_audio_revision(app: &mut Harness, initial: &Value) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = pump_tone(app, 128);
        assert_same_engine(initial, &state);
        if state["engine"]["applied_revision"] == state["revision"] {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Stretch preparation did not complete: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn rejected_shift_duplicate_cannot_turn_into_a_move_of_the_original_while_held() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1800.0,"height":1000.0}));
    let mut project = Project::new();
    let track = project.add_midi_track("Timeline boundary");
    let source = Clip::midi(
        "Boundary clip",
        daw_core::MAX_BEAT - 2.0,
        2.0,
        vec![Note::new(60, 0.25, 1.0, 100)],
    );
    let id = source.id;
    project.track_mut(track).unwrap().clips.push(source);
    let path = app.directory.path().join("boundary.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Fit");
    let before = app.key("Q");
    let from = center(&before, id);
    let delta = pixels(&before, 1.0);
    let mut shifted = |phase: &str, dx: f64| {
        let response = app.raw_command(json!({"command":"pointer","phase":phase,
            "x":from.0+dx,"y":from.1,"shift":true}));
        assert!(
            response["state"].is_object(),
            "App must report the rejected gesture: {response}"
        );
        response["state"].clone()
    };
    shifted("down", 0.0);
    let rejected = shifted("move", delta);
    assert_eq!(
        rejected["error"], true,
        "The initial copy must exceed the timeline limit"
    );
    assert!(
        rejected["status"].as_str().unwrap().contains("timeline"),
        "{rejected}"
    );
    assert_eq!(rejected["project"], before["project"]);
    let returned = shifted("move", -delta);
    assert_eq!(
        returned["project"], before["project"],
        "A rejected copy must not turn into moving its original"
    );
    let released = shifted("up", -delta);
    assert_eq!(released["project"], before["project"]);
    assert_eq!(released["history"]["undo"], before["history"]["undo"]);
    let alive = app.raw_command(json!({"command":"inspect"}));
    assert_eq!(alive["ok"], true);
    assert_eq!(alive["state"]["project"], before["project"]);
    app.quit();
}

#[test]
fn join_tool_prepares_the_clicked_track_before_opening_its_context() {
    let (mut app, original, _) = fixture(Kind::Midi);
    let mut project: Project = serde_json::from_value(app.inspect()["project"].clone()).unwrap();
    let track = project.add_midi_track("Join destination");
    let first = Clip::midi("First target", 8.0, 1.0, vec![Note::new(62, 0.0, 1.0, 90)]);
    let first_id = first.id;
    let second = Clip::midi(
        "Second target",
        10.0,
        1.0,
        vec![Note::new(64, 0.0, 1.0, 80)],
    );
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([first, second]);
    let path = app.directory.path().join("cross-track-join.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Source");
    let before = app.click("Fit");
    assert_eq!(selected(&before), original);
    let state = app.key("T");
    let popup = pointer(&mut app, "click", center(&state, first_id));
    assert_eq!(popup["selected_track"], track);
    assert_eq!(popup["popups"]["join"], true);
    app.click("Join");
    let joined = wait_join(&mut app);
    assert_eq!(
        joined["project"]["tracks"][0],
        before["project"]["tracks"][0]
    );
    let target = joined["project"]["tracks"][1]["clips"].as_array().unwrap();
    assert_eq!(target.len(), 1);
    near(target[0]["start"].as_f64().unwrap(), 8.0);
    near(target[0]["length"].as_f64().unwrap(), 3.0);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}
