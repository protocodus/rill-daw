//! Inline naming and exact clip fields use the ordinary mouse/keyboard UI.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Note, Project};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    clip: u64,
    ghost: u64,
    comp: u64,
    midi: u64,
    source: PathBuf,
    original_bytes: Vec<u8>,
}

fn fixture(app: &mut Harness) -> Fixture {
    let source = app.directory.path().join("original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..288_000)
            .map(|i| ((i as f32 * 0.031).sin()) * 0.15)
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Audio channel");
    let mut clip = Clip::audio("Reverse cue", 2.0, 4.0, source.to_string_lossy());
    clip.offset = 1.0;
    clip.source_rate = 0.5;
    if let ClipContent::Audio {
        reverse,
        fade_in,
        fade_out,
        ..
    } = &mut clip.content
    {
        *reverse = true;
        *fade_in = 0.25;
        *fade_out = 0.5;
    }
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let ghost = project.clone_clip(track, clip_id).unwrap();
    let takes = project.add_audio_track("Comp channel");
    let retained = Clip::audio("Comp source", 0.0, 4.0, source.to_string_lossy());
    let retained_id = retained.id;
    project.track_mut(takes).unwrap().clips.push(retained);
    let group = project.group_audio_takes(takes, &[retained_id]).unwrap();
    let comp = project.audio_take_regions(takes, group)[0].id;
    let midi_track = project.add_midi_track("MIDI channel");
    let midi = Clip::midi(
        "Precise phrase",
        4.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 91), Note::new(67, 3.0, 1.0, 76)],
    );
    let midi_id = midi.id;
    project.track_mut(midi_track).unwrap().clips.push(midi);
    let path = app.directory.path().join("precise.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    Fixture {
        clip: clip_id,
        ghost,
        comp,
        midi: midi_id,
        source,
        original_bytes,
    }
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

fn portable_project(state: &Value, original_bytes: &[u8]) -> Value {
    let mut project = state["project"].clone();
    for track in project["tracks"].as_array_mut().unwrap() {
        for clip in track["clips"].as_array_mut().unwrap() {
            if let Some(audio) = clip["content"].get_mut("Audio") {
                let path = audio["path"].as_str().unwrap();
                assert_eq!(
                    std::fs::read(path).unwrap(),
                    original_bytes,
                    "Every portable source keeps the complete original WAV bytes"
                );
                audio["path"] = json!("same original source");
            }
        }
    }
    project
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn select(app: &mut Harness, id: u64) -> Value {
    app.click("Arranger");
    app.key("Q");
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    let (x, y) = center(rect);
    let state = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(state["selected_clip"], id);
    state
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":redo}))
}

fn properties(app: &mut Harness, id: u64) -> Value {
    select(app, id);
    app.click("Window");
    app.click("Properties")
}

fn front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == label)
        .count()
        - 1;
    app.command(json!({"command":"click", "label":label, "occurrence":occurrence}))
}

fn edit_number(app: &mut Harness, row: &str, value: &str) -> Value {
    let state = app.inspect();
    let labels = state["label_rects"].as_array().unwrap();
    let label = labels
        .iter()
        .rev()
        .find(|label| label["text"] == row)
        .unwrap();
    let y = center(label).1;
    let field = labels
        .iter()
        .find(|field| {
            let text = field["text"].as_str().unwrap_or_default();
            (text.starts_with("Beat ") || text.chars().next().is_some_and(|c| c.is_ascii_digit()))
                && (center(field).1 - y).abs() < 12.0
                && field["min"][0].as_f64().unwrap() > label["max"][0].as_f64().unwrap()
        })
        .unwrap_or_else(|| panic!("No numeric field for {row}: {}", state["visible_labels"]));
    let (x, y) = center(field);
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    let editing = app.inspect();
    for key in ["ArrowLeft", "ArrowRight"] {
        let caret = app.key(key);
        assert_eq!(caret["project"], editing["project"]);
        assert_eq!(caret["selected_clip"], editing["selected_clip"]);
        assert_eq!(caret["selected_track"], editing["selected_track"]);
    }
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter")
}

#[test]
fn inline_rename_has_mouse_actions_f2_cancel_ghost_sync_and_one_undo() {
    let mut app = Harness::new();
    let f = fixture(&mut app);
    let before = select(&mut app, f.clip);
    // The menu and visible Apply/Cancel controls make F2 optional.
    app.click("Edit");
    app.click("Rename…");
    app.command(json!({"command":"text", "text":"Blue reverse"}));
    let renamed = app.click("Apply name");
    assert_eq!(renamed["view"], "Arrangement");
    for id in [f.clip, f.ghost] {
        assert_eq!(clip(&renamed, id)["name"], "Blue reverse");
    }
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    assert_eq!(undo(&mut app, true)["project"], renamed["project"]);
    select(&mut app, f.clip);
    let before = app.inspect();
    app.key("F2");
    app.command(json!({"command":"text", "text":"Discard this draft"}));
    for key in ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"] {
        let caret = app.key(key);
        assert_eq!(caret["project"], before["project"]);
        assert_eq!(caret["selected_clip"], before["selected_clip"]);
        assert_eq!(caret["selected_track"], before["selected_track"]);
    }
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    app.key("F2");
    app.command(json!({"command":"text", "text":"Escaped draft"}));
    let cancelled = app.key("Escape");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["view"], "Arrangement");
    app.click("Window");
    app.click("Clip Editor");
    app.key("F2");
    app.command(json!({"command":"text", "text":"Editor named cue"}));
    let renamed = app.key("Enter");
    assert_eq!(renamed["view"], "Clip");
    assert_eq!(clip(&renamed, f.clip)["name"], "Editor named cue");
    app.click("Arranger");
    app.click("Audio channel");
    assert!(app.inspect()["selected_clip"].is_null());
    app.key("F2");
    app.command(json!({"command":"text", "text":"Named channel"}));
    let track = app.key("Enter");
    assert_eq!(track["project"]["tracks"][0]["name"], "Named channel");
    app.key("F2");
    app.command(json!({"command":"text", "text":"Saved channel"}));
    let saved = app.command(json!({"command":"key", "key":"S", "cmd":true}));
    assert_eq!(saved["project"]["tracks"][0]["name"], "Saved channel");
    for _ in 0..100 {
        if app.inspect()["modified"] == false {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(app.inspect()["modified"], false);
    let saved_project = Project::load(app.directory.path().join("precise.rill")).unwrap();
    assert_eq!(saved_project.tracks[0].name, "Saved channel");
    assert_eq!(std::fs::read(&f.source).unwrap(), f.original_bytes);
    app.quit();
}

#[test]
fn exact_fields_move_trim_stretch_and_slip_reversed_ghosts_without_changing_sources() {
    let mut app = Harness::new();
    let f = fixture(&mut app);
    let original = properties(&mut app, f.clip);
    let moved = edit_number(&mut app, "Start", "5.125");
    assert_eq!(clip(&moved, f.clip)["start"], 4.125);
    for field in ["length", "offset", "source_rate", "content"] {
        assert_eq!(clip(&moved, f.clip)[field], clip(&original, f.clip)[field]);
    }
    assert_eq!(clip(&moved, f.ghost), clip(&original, f.ghost));
    assert_eq!(undo(&mut app, false)["project"], original["project"]);
    undo(&mut app, true);
    let trimmed = edit_number(&mut app, "Duration", "2.5");
    assert_eq!(clip(&trimmed, f.clip)["length"], 2.5);
    assert_eq!(clip(&trimmed, f.clip)["offset"], 1.75);
    assert_eq!(clip(&trimmed, f.clip)["source_rate"], 0.5);
    assert_eq!(clip(&trimmed, f.ghost), clip(&moved, f.ghost));
    assert_eq!(undo(&mut app, false)["project"], moved["project"]);
    undo(&mut app, true);
    app.click("Trim");
    front(&mut app, "Stretch");
    let stretched = edit_number(&mut app, "Duration", "5");
    assert_eq!(clip(&stretched, f.clip)["length"], 5.0);
    assert_eq!(clip(&stretched, f.clip)["source_rate"], 0.25);
    assert_eq!(clip(&stretched, f.clip)["offset"], 1.75);
    assert_eq!(undo(&mut app, false)["project"], trimmed["project"]);
    undo(&mut app, true);
    let slipped = edit_number(&mut app, "Source offset", "4.125");
    assert_eq!(clip(&slipped, f.clip)["offset"], 4.125);
    for field in ["start", "length", "source_rate", "content"] {
        assert_eq!(
            clip(&slipped, f.clip)[field],
            clip(&stretched, f.clip)[field]
        );
    }
    assert_eq!(clip(&slipped, f.ghost), clip(&stretched, f.ghost));
    assert_eq!(undo(&mut app, false)["project"], stretched["project"]);
    undo(&mut app, true);
    let expected_portable = portable_project(&slipped, &f.original_bytes);
    let save = app.directory.path().join("precise-edited.rill");
    app.command(json!({"command":"save", "path":save}));
    let mut app = app.restart();
    let recalled = app.command(json!({"command":"open", "path":save}));
    assert_eq!(
        portable_project(&recalled, &f.original_bytes),
        expected_portable
    );
    assert_eq!(std::fs::read(&f.source).unwrap(), f.original_bytes);
    app.quit();
}

#[test]
fn comp_timing_retains_archived_takes_and_f2_respects_text_and_menu_focus() {
    let mut app = Harness::new();
    let f = fixture(&mut app);
    let before = properties(&mut app, f.comp);
    let retained: Vec<_> = before["project"]["tracks"][1]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|clip| clip["id"] != f.comp)
        .cloned()
        .collect();
    let changed = edit_number(&mut app, "Source offset", "2");
    assert_eq!(clip(&changed, f.comp)["offset"], 2.0);
    let after: Vec<_> = changed["project"]["tracks"][1]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|clip| clip["id"] != f.comp)
        .cloned()
        .collect();
    assert_eq!(after, retained);
    let (x, y) = (
        changed["clip_name_rect"][0].as_f64().unwrap() + 20.0,
        changed["clip_name_rect"][1].as_f64().unwrap() + 12.0,
    );
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    let focused = app.inspect();
    assert_eq!(app.key("F2")["project"], focused["project"]);
    assert!(
        !app.inspect()["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "Apply name")
    );
    app.key("Escape");
    app.click("Edit");
    let menu = app.inspect();
    let ignored = app.key("F2");
    assert_eq!(ignored["project"], menu["project"]);
    assert!(
        !ignored["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "Apply name")
    );
    app.key("Escape");
    assert_eq!(std::fs::read(&f.source).unwrap(), f.original_bytes);
    app.quit();
}

#[test]
fn exact_midi_window_and_stretch_keep_all_source_notes_recoverable() {
    let mut app = Harness::new();
    let f = fixture(&mut app);
    let before = properties(&mut app, f.midi);
    let notes = clip(&before, f.midi)["content"].clone();
    let slipped = edit_number(&mut app, "Source offset", "2");
    assert_eq!(clip(&slipped, f.midi)["offset"], 2.0);
    assert_eq!(clip(&slipped, f.midi)["content"], notes);
    let trimmed = edit_number(&mut app, "Duration", "2");
    assert_eq!(clip(&trimmed, f.midi)["source_rate"], 1.0);
    assert_eq!(clip(&trimmed, f.midi)["offset"], 2.0);
    assert_eq!(clip(&trimmed, f.midi)["content"], notes);
    app.click("Trim");
    front(&mut app, "Stretch");
    let stretched = edit_number(&mut app, "Duration", "4");
    assert_eq!(clip(&stretched, f.midi)["source_rate"], 0.5);
    assert_eq!(clip(&stretched, f.midi)["content"], notes);
    assert_eq!(undo(&mut app, false)["project"], trimmed["project"]);
    assert_eq!(undo(&mut app, false)["project"], slipped["project"]);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}
