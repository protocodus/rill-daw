//! Real app input tests for MIDI editing; no display, audio device, or plug-in required.
mod common;
use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn open(notes: Vec<Note>) -> Harness {
    open_instrument(notes, false)
}
fn open_instrument(notes: Vec<Note>, instrument: bool) -> Harness {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Piano test");
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("MIDI gesture test", 0.0, 16.0, notes));
    if instrument {
        project
            .set_instrument(
                track,
                daw_core::Instrument::Vst3 {
                    path: app
                        .directory
                        .path()
                        .join("Fixture.vst3")
                        .to_string_lossy()
                        .to_string(),
                    class_id: None,
                },
            )
            .unwrap();
    }
    let path = app.directory.path().join("midi-fixture.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    // Select the actual arranger clip before opening its editor.
    app.click("MIDI gesture test");
    app.click("Window");
    app.click("Clip Editor");
    assert!(!app.inspect()["midi_editor"].is_null());
    app
}
fn notes(state: &Value) -> Vec<Value> {
    state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
        .clone()
}
fn point(state: &Value, beat: f64, pitch: u8) -> (f64, f64) {
    let m = &state["midi_editor"];
    let plot = &m["plot"];
    (
        plot["min"][0].as_f64().unwrap()
            + (beat - m["start_beat"].as_f64().unwrap()) * m["pixels_per_beat"].as_f64().unwrap(),
        plot["min"][1].as_f64().unwrap()
            + (127.5 - f64::from(pitch) - m["row_offset"].as_f64().unwrap())
                * m["row_height"].as_f64().unwrap(),
    )
}
fn pointer(app: &mut Harness, phase: &str, from: (f64, f64), to: Option<(f64, f64)>) -> Value {
    let mut cmd = json!({"command":"pointer","phase":phase,"x":from.0,"y":from.1});
    if let Some(to) = to {
        cmd["to_x"] = json!(to.0);
        cmd["to_y"] = json!(to.1);
    }
    app.command(cmd)
}
fn note_rect(state: &Value, id: u64) -> Value {
    state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap()["rect"]
        .clone()
}
fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}
fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}
fn close_enough(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

#[test]
fn draw_resize_both_edges_double_click_and_history_use_real_pointer_gestures() {
    let mut app = open(vec![Note::new(60, 1.0, 1.0, 100)]);
    app.key("W");
    let state = app.inspect();
    let p = point(&state, 3.0, 62);
    let state = pointer(&mut app, "click", p, None);
    assert_eq!(notes(&state).len(), 2);
    let drawn = notes(&state)
        .into_iter()
        .find(|n| n["pitch"] == 62)
        .unwrap();
    close_enough(drawn["start"].as_f64().unwrap(), 3.0);
    let id = drawn["id"].as_u64().unwrap();
    app.key("Q");
    let state = app.inspect();
    let rect = note_rect(&state, id);
    let y = center(&rect).1;
    let x = rect["max"][0].as_f64().unwrap() - 2.0;
    let scale = state["midi_editor"]["pixels_per_beat"].as_f64().unwrap();
    let state = pointer(&mut app, "drag", (x, y), Some((x + scale, y)));
    let note = notes(&state).into_iter().find(|n| n["id"] == id).unwrap();
    close_enough(note["length"].as_f64().unwrap(), 1.25);
    let rect = note_rect(&state, id);
    let x = rect["min"][0].as_f64().unwrap() + 2.0;
    let state = pointer(&mut app, "drag", (x, y), Some((x + scale * 0.5, y)));
    let note = notes(&state).into_iter().find(|n| n["id"] == id).unwrap();
    close_enough(note["start"].as_f64().unwrap(), 3.5);
    close_enough(note["length"].as_f64().unwrap(), 0.75);
    let state = undo(&mut app, false);
    let note = notes(&state).into_iter().find(|n| n["id"] == id).unwrap();
    close_enough(note["start"].as_f64().unwrap(), 3.0);
    undo(&mut app, true);
    let state = app.inspect();
    let p = point(&state, 6.0, 65);
    let state = pointer(&mut app, "double_click", p, None);
    assert_eq!(
        notes(&state).len(),
        3,
        "Double background creates exactly one note"
    );
    let id = notes(&state)
        .into_iter()
        .find(|n| n["pitch"] == 65)
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let rect = note_rect(&state, id);
    let state = pointer(&mut app, "double_click", center(&rect), None);
    assert_eq!(notes(&state).len(), 2, "Double note deletes it");
    let saved = app.directory.path().join("edited.rill");
    app.command(json!({"command":"save","path":saved}));
    let project = Project::load(saved).unwrap();
    project.validate().unwrap();
    assert_eq!(project.tracks[0].clips.len(), 1);
    app.quit();
}

#[test]
fn chord_and_scale_stacks_highlight_keys_and_velocity_lane_edits_multiple_notes() {
    let mut app = open(vec![Note::new(60, 1.0, 1.0, 90)]);
    app.key("U");
    let state = app.inspect();
    let state = pointer(&mut app, "click", point(&state, 3.0, 60), None);
    let chord: Vec<_> = notes(&state)
        .iter()
        .filter(|n| n["start"] == 3.0)
        .map(|n| n["pitch"].as_u64().unwrap())
        .collect();
    assert_eq!(chord, vec![60, 64, 67]);
    let keys = state["midi_editor"]["keys"].as_array().unwrap();
    assert!(keys.iter().any(|k| k["pitch"] == 64 && k["chord"] == true));
    app.click("Chord stack");
    app.click("Scale stack");
    app.click("Natural minor");
    app.click("Major");
    let state = app.inspect();
    let state = pointer(&mut app, "click", point(&state, 5.0, 60), None);
    let scale: Vec<_> = notes(&state)
        .iter()
        .filter(|n| n["start"] == 5.0)
        .map(|n| n["pitch"].as_u64().unwrap())
        .collect();
    assert_eq!(scale, vec![60, 62, 64, 65, 67, 69, 71]);
    app.key("W");
    let state = app.inspect();
    let lane = &state["midi_editor"]["velocity"];
    let y = lane["max"][1].as_f64().unwrap()
        - (lane["max"][1].as_f64().unwrap() - lane["min"][1].as_f64().unwrap()) * 0.4;
    let from = (point(&state, 0.9, 60).0, y);
    let to = (point(&state, 5.1, 60).0, y);
    let state = pointer(&mut app, "drag", from, Some(to));
    assert!(
        notes(&state)
            .iter()
            .all(|n| (n["velocity"].as_i64().unwrap() - 51).abs() <= 1)
    );
    let state = undo(&mut app, false);
    assert!(notes(&state).iter().any(|n| n["velocity"] == 100));
    app.key("Q");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let state = app.inspect();
    let lane = &state["midi_editor"]["velocity"];
    let y = lane["min"][1].as_f64().unwrap() + 20.0;
    let x = point(&state, 1.0, 60).0;
    let before = notes(&state);
    let state = pointer(&mut app, "drag", (x, y), Some((x, y - 10.0)));
    assert!(
        notes(&state)
            .iter()
            .zip(before)
            .all(|(after, before)| after["velocity"].as_u64().unwrap()
                >= before["velocity"].as_u64().unwrap())
    );
    app.quit();
}

#[test]
fn shared_tools_split_glue_duplicate_erase_cursor_and_keyboard_shortcuts() {
    let mut app = open(vec![Note::new(60, 1.0, 2.0, 100)]);
    app.key("E");
    let state = app.inspect();
    let state = pointer(&mut app, "click", point(&state, 2.0, 60), None);
    assert_eq!(notes(&state).len(), 2);
    app.key("Q");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.click("Notes");
    app.click("Glue selected");
    assert_eq!(notes(&app.inspect()).len(), 1);
    app.key("R");
    let state = app.inspect();
    let state = pointer(&mut app, "click", point(&state, 1.5, 60), None);
    assert_eq!(notes(&state).len(), 2);
    app.key("Z");
    let state = app.inspect();
    let state = pointer(&mut app, "click", point(&state, 3.5, 60), None);
    assert_eq!(notes(&state).len(), 1);
    app.key("P");
    let state = app.inspect();
    let state = pointer(&mut app, "click", point(&state, 4.0, 64), None);
    close_enough(state["midi_editor"]["cursor"].as_f64().unwrap(), 4.0);
    app.quit();
}

#[test]
fn anchored_zoom_pan_fit_both_axes_and_held_key_audition_release() {
    let mut app = open(vec![
        Note::new(48, 2.0, 1.0, 80),
        Note::new(84, 10.0, 2.0, 100),
    ]);
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| label.as_str() == Some("Fit"))
        .count()
        - 1;
    app.command(json!({"command":"click","label":"Fit","occurrence":occurrence}));
    let state = app.inspect();
    let m = &state["midi_editor"];
    let plot = &m["plot"];
    for note in m["notes"].as_array().unwrap() {
        let rect = &note["rect"];
        assert!(rect["min"][0].as_f64().unwrap() >= plot["min"][0].as_f64().unwrap() - 0.1);
        assert!(rect["max"][0].as_f64().unwrap() <= plot["max"][0].as_f64().unwrap() + 0.1);
        assert!(rect["min"][1].as_f64().unwrap() >= plot["min"][1].as_f64().unwrap() - 0.1);
        assert!(rect["max"][1].as_f64().unwrap() <= plot["max"][1].as_f64().unwrap() + 0.1);
    }
    let anchor = center(plot);
    let old_x = m["pixels_per_beat"].as_f64().unwrap();
    let old_y = m["row_height"].as_f64().unwrap();
    let state = app
        .command(json!({"command":"scroll","x":anchor.0,"y":anchor.1,"delta":120.0,"ctrl":true}));
    assert!(state["midi_editor"]["pixels_per_beat"].as_f64().unwrap() > old_x);
    let state=app.command(json!({"command":"scroll","x":anchor.0,"y":anchor.1,"delta":120.0,"ctrl":true,"shift":true}));
    assert!(state["midi_editor"]["row_height"].as_f64().unwrap() > old_y);
    let before = state["midi_editor"]["start_beat"].as_f64().unwrap();
    let state=app.command(json!({"command":"pointer","phase":"drag","button":"middle","x":anchor.0,"y":anchor.1,"to_x":anchor.0-100.0,"to_y":anchor.1}));
    assert!(state["midi_editor"]["start_beat"].as_f64().unwrap() > before);
    app.key("F");
    let state = app.inspect();
    close_enough(
        state["midi_editor"]["pixels_per_beat"].as_f64().unwrap(),
        old_x,
    );
    let key = state["midi_editor"]["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["pitch"] == 60)
        .unwrap();
    let pos = center(&key["rect"]);
    let state = pointer(&mut app, "down", pos, None);
    assert_eq!(state["midi_editor"]["audition"][1], 60);
    let state = pointer(&mut app, "up", pos, None);
    assert!(state["midi_editor"]["audition"].is_null());
    assert_eq!(
        notes(&state).len(),
        2,
        "Audition and navigation never edit notes"
    );
    app.quit();
}

#[test]
fn drawn_note_length_group_move_and_pencil_double_delete_are_undoable() {
    let mut app = open(vec![Note::new(60, 1.0, 1.0, 90)]);
    app.key("W");
    let state = app.inspect();
    let state = pointer(
        &mut app,
        "drag",
        point(&state, 4.0, 64),
        Some(point(&state, 6.0, 64)),
    );
    let drawn = notes(&state)
        .into_iter()
        .find(|note| note["pitch"] == 64)
        .unwrap();
    close_enough(drawn["length"].as_f64().unwrap(), 2.0);
    app.key("Q");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let state = app.inspect();
    let from = point(&state, 1.5, 60);
    let to = point(&state, 2.5, 62);
    let state = pointer(&mut app, "drag", from, Some(to));
    let moved = notes(&state);
    assert!(
        moved
            .iter()
            .any(|note| note["pitch"] == 62 && note["start"] == 2.0 && note["length"] == 1.0)
    );
    assert!(
        moved
            .iter()
            .any(|note| note["pitch"] == 66 && note["start"] == 5.0 && note["length"] == 2.0)
    );
    let state = undo(&mut app, false);
    assert!(
        notes(&state)
            .iter()
            .any(|note| note["pitch"] == 60 && note["start"] == 1.0)
    );
    app.key("W");
    let state = app.inspect();
    let state = pointer(&mut app, "double_click", point(&state, 1.5, 60), None);
    assert_eq!(
        notes(&state).len(),
        1,
        "Double-click also erases an existing note with Pencil"
    );
    app.quit();
}

#[test]
fn held_audition_changes_pitch_and_stops_on_focus_loss_and_view_dismissal() {
    let mut app = open_instrument(vec![Note::new(60, 1.0, 1.0, 90)], true);
    let state = app.inspect();
    let keys = state["midi_editor"]["keys"].as_array().unwrap();
    let c = center(&keys.iter().find(|key| key["pitch"] == 60).unwrap()["rect"]);
    let d = center(&keys.iter().find(|key| key["pitch"] == 62).unwrap()["rect"]);
    let state = pointer(&mut app, "down", c, None);
    assert_eq!(state["audition_request"][1], 60);
    let state = pointer(&mut app, "move", d, None);
    assert_eq!(state["audition_request"][1], 62);
    let state = app.command(json!({"command":"focused","focused":false}));
    assert!(state["audition_request"].is_null());
    pointer(&mut app, "up", d, None);
    app.command(json!({"command":"focused","focused":true}));
    let state = pointer(&mut app, "down", c, None);
    assert_eq!(state["audition_request"][1], 60);
    let state = app.key("Escape");
    assert_eq!(state["view"], "Arrangement");
    assert!(state["audition_request"].is_null());
    pointer(&mut app, "up", c, None);
    assert_eq!(notes(&app.inspect()).len(), 1);
    app.quit();
}

#[test]
fn quantize_preview_strength_swing_ends_apply_cancel_and_history() {
    let original = vec![
        Note::new(60, 1.18, 0.31, 80),
        Note::new(64, 1.18, 0.31, 90),
        Note::new(67, 2.1, 0.4, 100),
    ];
    let mut app = open(original);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let before = notes(&app.inspect());
    app.click("Notes");
    let preview = app.click("Quantize...");
    assert_eq!(
        notes(&preview),
        before,
        "Preview must not modify the project"
    );
    close_enough(
        preview["midi_editor"]["preview_notes"][0]["start"]
            .as_f64()
            .unwrap(),
        1.25,
    );
    for command in [
        json!({"command":"key","key":"Delete"}),
        json!({"command":"key","key":"D","cmd":true}),
        json!({"command":"key","key":"ArrowRight"}),
    ] {
        let state = app.command(command);
        assert_eq!(
            notes(&state),
            before,
            "Preview isolates note editing shortcuts"
        );
        assert!(!state["midi_editor"]["preview_notes"].is_null());
    }
    assert_eq!(notes(&app.click("Cancel quantize")), before);
    app.click("Notes");
    app.click("Quantize...");
    let state = app.inspect();
    let slider = &state["midi_editor"]["quantize"]["strength_rect"];
    let center = center(slider);
    let state = pointer(&mut app, "click", center, None);
    let strength = state["midi_editor"]["quantize"]["strength"]
        .as_f64()
        .unwrap();
    assert!((0.4..0.6).contains(&strength));
    let slider = &state["midi_editor"]["quantize"]["swing_rect"];
    let state = pointer(
        &mut app,
        "click",
        (
            slider["max"][0].as_f64().unwrap() - 1.0,
            (slider["min"][1].as_f64().unwrap() + slider["max"][1].as_f64().unwrap()) * 0.5,
        ),
        None,
    );
    assert!(state["midi_editor"]["quantize"]["swing"].as_f64().unwrap() > 0.74);
    let state = app.click("Note ends");
    assert_eq!(notes(&state), before);
    let expected = state["midi_editor"]["preview_notes"]
        .as_array()
        .unwrap()
        .clone();
    assert!(expected[0]["start"].as_f64().unwrap() < 1.18);
    assert!(expected[0]["length"].as_f64().unwrap() > 0.31);
    let applied = app.click("Apply quantize");
    assert_eq!(notes(&applied), expected);
    assert_eq!(notes(&undo(&mut app, false)), before);
    assert_eq!(notes(&undo(&mut app, true)), expected);
    app.click("Notes");
    app.click("Quantize...");
    let cancelled = app.key("Escape");
    assert_eq!(cancelled["view"], "Clip");
    assert!(cancelled["midi_editor"]["preview_notes"].is_null());
    assert_eq!(notes(&cancelled), expected);
    app.click("Notes");
    app.click("Quantize...");
    let undone = undo(&mut app, false);
    assert_eq!(notes(&undone), before);
    assert!(undone["midi_editor"]["preview_notes"].is_null());
    app.quit();
}

#[test]
fn pitch_and_selection_controls_are_disclosed_without_changing_notes() {
    let mut app = open(vec![
        Note::new(60, 1.0, 1.0, 90),
        Note::new(64, 1.0, 1.0, 100),
    ]);
    app.command(json!({"command":"viewport","width":1100.0,"height":760.0}));
    app.key("Q");
    let before = app.inspect();
    let visible = |state: &Value, label: &str| {
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == label)
    };
    for label in [
        "Natural minor",
        "Major triad",
        "Chord stack",
        "Glue selected",
    ] {
        assert!(!visible(&before, label), "{label} should be contextual");
    }
    let pitch = app.click("Pitch");
    assert!(visible(&pitch, "Natural minor"));
    assert!(visible(&pitch, "Major triad"));
    assert_eq!(app.key("Escape")["view"], "Clip");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let selected = app.click("Notes");
    assert!(visible(&selected, "Glue selected"));
    assert_eq!(
        selected["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|value| *value == "Velocity")
            .count(),
        2,
        "Selected velocity control appears alongside the existing velocity lane label"
    );
    app.key("Escape");
    let multi = app.key("U");
    assert!(visible(&multi, "Chord stack"));
    assert!(visible(&multi, "Major triad"), "{}", multi["label_rects"]);
    assert!(visible(&multi, "C · Natural minor"));
    assert!(
        multi["midi_editor"]["plot"]["min"][1].as_f64().unwrap() < 330.0,
        "Context controls wrap without crowding out the editor"
    );
    let select = app.key("Q");
    assert!(!visible(&select, "Chord stack"));
    assert_eq!(select["project"], before["project"]);
    app.quit();
}

#[test]
fn legato_preserves_chords_and_transpose_and_nudge_preserve_group_spacing() {
    let mut app = open(vec![
        Note::new(60, 0.0, 0.2, 90),
        Note::new(64, 0.0, 0.4, 100),
        Note::new(67, 1.0, 2.0, 110),
        Note::new(72, 2.0, 0.5, 80),
    ]);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let original = notes(&app.inspect());
    app.click("Notes");
    let legato = notes(&app.click("Legato"));
    assert_eq!(
        legato
            .iter()
            .map(|note| note["length"].as_f64().unwrap())
            .collect::<Vec<_>>(),
        vec![1.0, 1.0, 1.0, 0.5]
    );
    app.click("Notes");
    let up = notes(&app.click("+1 semitone"));
    assert_eq!(up[0]["pitch"], 61);
    app.click("Notes");
    let octave = notes(&app.click("+1 octave"));
    assert_eq!(octave[0]["pitch"], 73);
    app.key("ArrowDown");
    let restored = app.command(json!({"command":"key","key":"ArrowDown","shift":true}));
    for (note, original) in notes(&restored).iter().zip(&original) {
        assert_eq!(note["pitch"], original["pitch"]);
        assert_eq!(note["id"], original["id"]);
    }
    let right = notes(&app.key("ArrowRight"));
    close_enough(right[0]["start"].as_f64().unwrap(), 0.25);
    let bar = notes(&app.command(json!({"command":"key","key":"ArrowRight","shift":true})));
    close_enough(bar[0]["start"].as_f64().unwrap(), 4.25);
    app.key("ArrowLeft");
    let left = notes(&app.command(json!({"command":"key","key":"ArrowLeft","shift":true})));
    close_enough(left[0]["start"].as_f64().unwrap(), 0.0);
    close_enough(left[2]["start"].as_f64().unwrap(), 1.0);
    app.quit();
}

#[test]
fn selected_note_clipboard_pastes_at_cursor_with_new_ids_and_relative_spacing() {
    let mut app = open(vec![
        Note::new(60, 1.0, 0.5, 80),
        Note::new(64, 1.5, 0.75, 110),
    ]);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let original = notes(&app.inspect());
    let copied = app.command(json!({"command":"key","key":"C","cmd":true}));
    assert_eq!(copied["midi_editor"]["clipboard_count"], 2);
    app.key("P");
    let p = point(&app.inspect(), 6.0, 60);
    pointer(&mut app, "click", p, None);
    let pasted = notes(&app.command(json!({"command":"key","key":"V","cmd":true})));
    assert_eq!(pasted.len(), 4);
    assert_eq!(&pasted[..2], &original);
    close_enough(pasted[2]["start"].as_f64().unwrap(), 6.0);
    close_enough(pasted[3]["start"].as_f64().unwrap(), 6.5);
    assert_ne!(pasted[2]["id"], original[0]["id"]);
    assert_eq!(pasted[2]["velocity"], 80);
    app.click("Notes");
    app.click("Copy notes");
    let p = point(&app.inspect(), 9.0, 60);
    pointer(&mut app, "click", p, None);
    app.click("Notes");
    let second = notes(&app.click("Paste notes at cursor"));
    assert_eq!(second.len(), 6);
    close_enough(second[4]["start"].as_f64().unwrap(), 9.0);
    close_enough(second[5]["start"].as_f64().unwrap(), 9.5);
    assert_eq!(notes(&undo(&mut app, false)).len(), 4);
    app.quit();
}

#[test]
fn shift_drag_duplicates_once_and_shift_edge_resize_temporarily_bypasses_snap() {
    let mut app = open(vec![
        Note::new(60, 1.0, 1.0, 80),
        Note::new(64, 1.0, 1.0, 110),
    ]);
    app.key("Q");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let state = app.inspect();
    let original = notes(&state);
    let id = original[0]["id"].as_u64().unwrap();
    let p = center(&note_rect(&state, id));
    let scale = state["midi_editor"]["pixels_per_beat"].as_f64().unwrap();
    let duplicated = app.command(json!({"command":"pointer","phase":"drag","shift":true,"x":p.0,"y":p.1,"to_x":p.0+2.0*scale,"to_y":p.1}));
    let all = notes(&duplicated);
    assert_eq!(all.len(), 4);
    assert_eq!(&all[..2], &original);
    close_enough(all[2]["start"].as_f64().unwrap(), 3.0);
    close_enough(all[3]["start"].as_f64().unwrap(), 3.0);
    let id = all[2]["id"].as_u64().unwrap();
    let rect = note_rect(&duplicated, id);
    let x = rect["max"][0].as_f64().unwrap() - 2.0;
    let y = center(&rect).1;
    let resized = notes(&app.command(json!({"command":"pointer","phase":"drag","shift":true,"x":x,"y":y,"to_x":x+0.13*scale,"to_y":y})));
    assert_eq!(resized.len(), 4, "Shift resizing must not duplicate");
    close_enough(resized[2]["length"].as_f64().unwrap(), 1.13);
    close_enough(resized[3]["length"].as_f64().unwrap(), 1.13);
    close_enough(resized[0]["length"].as_f64().unwrap(), 1.0);
    assert_eq!(notes(&undo(&mut app, false)).len(), 4);
    assert_eq!(notes(&undo(&mut app, false)).len(), 2);
    app.quit();
}
