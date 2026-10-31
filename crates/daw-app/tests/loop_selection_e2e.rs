//! Selection-driven looping uses the real menus and retained editing context.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn fixture() -> Harness {
    let mut app = Harness::with_virtual_audio(None, None);
    let source = app.directory.path().join("loop-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Loop audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "First phrase",
        1.375,
        1.25,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("Loop MIDI");
    project.track_mut(midi).unwrap().clips.extend([
        Clip::midi("Last phrase", 6.125, 1.75, vec![]),
        Clip::midi("Outside phrase", 9.5, 1.0, vec![]),
    ]);
    let path = app.directory.path().join("loop-selection.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1300, "height":900}));
    app
}

fn menu(app: &mut Harness, parent: &str, label: &str) -> Value {
    app.click(parent);
    app.click(label)
}

fn context(app: &mut Harness, name: &str) -> Value {
    let state = app.inspect();
    let id = state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["name"] == name)
        .unwrap()["id"]
        .clone();
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    app.command(json!({
        "command":"pointer", "phase":"click", "button":"secondary",
        "x":(rect["min"][0].as_f64().unwrap()+rect["max"][0].as_f64().unwrap())*0.5,
        "y":(rect["min"][1].as_f64().unwrap()+rect["max"][1].as_f64().unwrap())*0.5,
    }))
}

fn select_phrases(app: &mut Harness) -> Value {
    context(app, "First phrase");
    app.click("Select only this clip");
    context(app, "Last phrase");
    app.click("Add to selection")
}

fn assert_range(state: &Value, start: f64, end: f64) {
    assert_eq!(state["project"]["loop_enabled"], true);
    assert_eq!(state["project"]["loop_start"], start);
    assert_eq!(state["project"]["loop_end"], end);
}

fn assert_context_unchanged(before: &Value, after: &Value) {
    for field in [
        "selected_track",
        "selected_clip",
        "selected_clips",
        "selection_span",
    ] {
        assert_eq!(after[field], before[field], "Loop changed {field}");
    }
    assert_eq!(after["timeline"]["cursor"], before["timeline"]["cursor"]);
    assert_eq!(after["project"]["tracks"], before["project"]["tracks"]);
}

fn wait_recording(app: &mut Harness, phase: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = app.inspect();
        if state["recording"]["phase"] == phase {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Recording did not reach {phase}: {state}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn range_loops_discontiguous_clips_exactly_in_one_undo_and_preserves_context() {
    let mut app = fixture();
    let before = select_phrases(&mut app);
    assert_eq!(before["selected_clips"].as_array().unwrap().len(), 2);
    let looped = menu(&mut app, "Range", "Loop selection");
    assert_range(&looped, 1.375, 7.875);
    assert_context_unchanged(&before, &looped);
    assert_eq!(looped["history"]["undo"], json!(["Loop selection"]));
    assert_eq!(looped["native_menu"]["open"], false);
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    assert_context_unchanged(&before, &undone);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], looped["project"]);
    assert_context_unchanged(&before, &redone);
    let repeated = menu(&mut app, "Range", "Loop selection");
    assert_eq!(repeated["history"], redone["history"]);
    assert_eq!(repeated["project"], redone["project"]);

    let archive = app.directory.path().join("selected-loop.rill");
    app.command(json!({"command":"save", "path":archive}));
    app.click("Loop");
    let reopened = app.command(json!({"command":"open", "path":archive}));
    assert_range(&reopened, 1.375, 7.875);
    app.quit();
}

#[test]
fn context_loop_selection_keeps_phrase_rests_and_loop_clip_keeps_existing_selection() {
    let mut app = fixture();
    app.click("Cursor");
    let state = app.inspect();
    let timeline = &state["timeline"];
    let x = |beat: f64| {
        timeline["canvas"]["min"][0].as_f64().unwrap()
            + (beat - timeline["start_beat"].as_f64().unwrap())
                * timeline["pixels_per_beat"].as_f64().unwrap()
    };
    let first = &timeline["tracks"][0]["header"];
    let last = &timeline["tracks"][1]["header"];
    let before = app.command(json!({
        "command":"pointer", "phase":"drag", "x":x(0.5),
        "y":first["min"][1].as_f64().unwrap()+2.0, "to_x":x(8.5),
        "to_y":last["max"][1].as_f64().unwrap()-2.0,
    }));
    assert_eq!(before["selection_span"], json!([0.5, 8.5]));
    assert_eq!(before["selected_clips"].as_array().unwrap().len(), 2);
    context(&mut app, "First phrase");
    let looped = app.click("Loop selection");
    assert_range(&looped, 0.5, 8.5);
    assert_context_unchanged(&before, &looped);
    assert_eq!(looped["history"]["undo"], json!(["Loop selection"]));
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"], before["project"]);
    assert_context_unchanged(&before, &undone);
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"], looped["project"]);
    context(&mut app, "Outside phrase");
    let clip_loop = app.click("Loop clip");
    assert_range(&clip_loop, 9.5, 10.5);
    assert_context_unchanged(&before, &clip_loop);
    assert_eq!(clip_loop["history"]["undo"].as_array().unwrap().len(), 2);
    let restored = menu(&mut app, "Edit", "Undo");
    assert_eq!(restored["project"], looped["project"]);
    assert_context_unchanged(&before, &restored);
    app.quit();
}

#[test]
fn loop_selection_is_disabled_without_selection_and_while_recording() {
    let mut app = fixture();
    let before = menu(&mut app, "Edit", "Clear clip selection");
    assert!(before["selected_clips"].as_array().unwrap().is_empty());
    app.click("Range");
    let disabled = app.click("Loop selection");
    assert_eq!(disabled["project"], before["project"]);
    assert_eq!(disabled["history"], before["history"]);
    assert_eq!(disabled["native_menu"]["open"], true);
    app.key("Escape");

    context(&mut app, "First phrase");
    app.click("Select only this clip");
    menu(&mut app, "Window", "Recording");
    app.click("Arm recording");
    app.key("Escape");
    app.click("Record");
    let recording = wait_recording(&mut app, "recording");
    app.click("Range");
    let disabled = app.click("Loop selection");
    assert_eq!(disabled["project"], recording["project"]);
    assert_eq!(disabled["history"], recording["history"]);
    assert_eq!(disabled["recording"]["phase"], "recording");
    app.key("Escape");
    context(&mut app, "First phrase");
    let disabled = app.click("Loop selection");
    assert_eq!(disabled["project"], recording["project"]);
    assert_eq!(disabled["history"], recording["history"]);
    assert_eq!(disabled["recording"]["phase"], "recording");
    app.key("Escape");
    app.click("Stop");
    wait_recording(&mut app, "idle");
    app.quit();
}

#[test]
fn clip_editor_range_loops_only_the_open_clip_instead_of_the_retained_arranger_group() {
    let mut app = fixture();
    select_phrases(&mut app);
    let before = menu(&mut app, "Window", "Clip Editor");
    let clip = before["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .find(|clip| clip["id"] == before["selected_clip"])
        .unwrap();
    let start = clip["start"].as_f64().unwrap();
    let end = start + clip["length"].as_f64().unwrap();
    let looped = menu(&mut app, "Range", "Loop clip");
    assert_range(&looped, start, end);
    assert_context_unchanged(&before, &looped);
    assert_eq!(looped["history"]["undo"].as_array().unwrap().len(), 1);
    app.quit();
}
