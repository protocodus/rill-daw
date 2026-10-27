//! Fully covered MIDI notes merge at gesture commit, preserving source identity.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(source: Vec<Note>, rate: f64, ghost: bool) -> (Harness, u64) {
    fixture_window(source, rate, ghost, 8.0 / rate)
}

fn fixture_window(source: Vec<Note>, rate: f64, ghost: bool, length: f64) -> (Harness, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1500,"height":1000}));
    let mut project = Project::new();
    let track = project.add_midi_track("Merge test");
    let clip = Clip::midi("Merge phrase", 0.0, 8.0, source);
    let original = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let selected = if ghost {
        project.clone_clip(track, original).unwrap()
    } else {
        original
    };
    let clip = project.clip_mut(track, selected).unwrap();
    clip.source_rate = rate;
    clip.length = length;
    let path = app.directory.path().join("merge.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == selected)
        .unwrap()["rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.click("Window");
    app.click("Clip Editor");
    (app, selected)
}

fn notes(state: &Value, clip: u64) -> &[Value] {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == clip)
        .unwrap()["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}

fn open_editor(app: &mut Harness, id: u64) -> Value {
    app.click("Arranger");
    app.key("Q");
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap()["rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.click("Window");
    app.click("Clip Editor")
}

fn point(state: &Value, beat: f64, pitch: u8) -> (f64, f64) {
    let v = &state["midi_editor"];
    let row = v["visible_pitches"]
        .as_array()
        .unwrap()
        .iter()
        .position(|p| p.as_u64() == Some(u64::from(pitch)))
        .unwrap() as f64;
    (
        v["plot"]["min"][0].as_f64().unwrap()
            + (beat - v["start_beat"].as_f64().unwrap()) * v["pixels_per_beat"].as_f64().unwrap(),
        v["plot"]["min"][1].as_f64().unwrap()
            + (row + 0.5 - v["row_offset"].as_f64().unwrap()) * v["row_height"].as_f64().unwrap(),
    )
}

fn pointer(app: &mut Harness, phase: &str, p: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":p.0,"y":p.1}))
}

fn selected(state: &Value) -> Vec<u64> {
    let mut ids: Vec<_> = state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["selected"] == true)
        .map(|n| n["id"].as_u64().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}

#[test]
fn pencil_containment_waits_for_release_and_keeps_existing_identity_velocity_and_one_undo() {
    let target = Note::new(60, 2.0, 1.0, 73);
    let id = target.id;
    let (mut app, clip) = fixture(vec![target], 1.0, false);
    let before = app.inspect();
    app.key("W");
    let state = app.inspect();
    let start = point(&state, 1.0, 60);
    let end = point(&state, 4.0, 60);
    let held = pointer(&mut app, "down", start);
    assert_eq!(notes(&held, clip).len(), 2);
    let held = pointer(&mut app, "move", end);
    assert_eq!(
        notes(&held, clip).len(),
        2,
        "IDs must survive while resizing the drawn note"
    );
    assert_eq!(
        held["history"], before["history"],
        "held edit is one pending gesture"
    );
    let merged = pointer(&mut app, "up", end);
    assert_eq!(notes(&merged, clip).len(), 1);
    let note = &notes(&merged, clip)[0];
    assert_eq!(
        (
            note["id"].as_u64().unwrap(),
            note["velocity"].as_u64().unwrap()
        ),
        (id, 73)
    );
    assert_eq!(
        (
            note["start"].as_f64().unwrap(),
            note["length"].as_f64().unwrap()
        ),
        (1.0, 3.0)
    );
    assert_eq!(selected(&app.inspect()), vec![id]);
    assert_eq!(merged["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    assert_eq!(undo(&mut app, true)["project"], merged["project"]);
    app.quit();
}

#[test]
fn move_shift_copy_and_native_paste_merge_full_coverage_but_keep_partial_overlap_and_chords() {
    let target = Note::new(60, 2.0, 1.0, 71);
    let id = target.id;
    let moving = Note::new(60, 0.0, 1.0, 99);
    let chord = Note::new(64, 2.0, 1.0, 87);
    let (mut app, clip) = fixture(vec![target, moving, chord], 1.0, false);
    let before = app.inspect();
    let start = point(&before, 0.5, 60);
    let end = point(&before, 2.5, 60);
    pointer(&mut app, "down", start);
    let held = pointer(&mut app, "move", end);
    assert_eq!(
        notes(&held, clip).len(),
        3,
        "don't delete moving IDs midway through drag"
    );
    let merged = pointer(&mut app, "up", end);
    assert_eq!(notes(&merged, clip).len(), 2);
    assert!(
        notes(&merged, clip)
            .iter()
            .any(|n| n["id"] == id && n["velocity"] == 71)
    );
    assert!(notes(&merged, clip).iter().any(|n| n["pitch"] == 64));
    assert_eq!(selected(&app.inspect()), vec![id]);
    undo(&mut app, false);
    // A partial overlap remains two independent notes of the same pitch.
    let state = app.inspect();
    let start = point(&state, 0.5, 60);
    let end = point(&state, 2.0, 60);
    app.command(json!({"command":"pointer","phase":"drag","x":start.0,"y":start.1,"to_x":end.0,"to_y":end.1}));
    assert_eq!(notes(&app.inspect(), clip).len(), 3);
    undo(&mut app, false);
    // Shift-copy retains the original note and consumes only its overlapping copy.
    let state = app.inspect();
    let start = point(&state, 0.5, 60);
    let end = point(&state, 2.5, 60);
    let copied=app.command(json!({"command":"pointer","phase":"drag","x":start.0,"y":start.1,"to_x":end.0,"to_y":end.1,"shift":true}));
    assert_eq!(notes(&copied, clip).len(), 3);
    assert_eq!(copied["project"], before["project"]);
    assert_eq!(selected(&app.inspect()), vec![id]);
    // Native paste at exactly that target is a selection change, with no model/history churn.
    app.command(json!({"command":"clipboard","action":"copy"}));
    app.key("P");
    let p = point(&app.inspect(), 2.0, 60);
    pointer(&mut app, "click", p);
    app.key("Q");
    let before = app.inspect();
    let pasted = app.command(json!({"command":"clipboard","action":"paste"}));
    assert_eq!(pasted["project"], before["project"]);
    assert_eq!(pasted["history"], before["history"]);
    assert_eq!(pasted["revision"], before["revision"]);
    assert_eq!(selected(&app.inspect()), vec![id]);
    app.quit();
}

#[test]
fn stretched_ghost_move_merges_source_notes_and_survives_reopen() {
    let target = Note::new(60, 2.0, 1.0, 69);
    let id = target.id;
    let moving = Note::new(60, 0.0, 1.0, 101);
    let (mut app, clip) = fixture(vec![target, moving], 2.0, true);
    let before = app.inspect();
    let start = point(&before, 0.25, 60);
    let end = point(&before, 1.25, 60);
    let merged=app.command(json!({"command":"pointer","phase":"drag","x":start.0,"y":start.1,"to_x":end.0,"to_y":end.1}));
    assert_eq!(notes(&merged, clip).len(), 1);
    for c in merged["project"]["tracks"][0]["clips"].as_array().unwrap() {
        let note = &c["content"]["Midi"]["notes"][0];
        assert_eq!(note["id"], id);
        assert_eq!(note["start"], 2.0);
        assert_eq!(note["length"], 1.0);
        assert_eq!(note["velocity"], 69);
    }
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    undo(&mut app, true);
    let path = app.directory.path().join("merged.rill");
    app.command(json!({"command":"save","path":path}));
    let mut app = app.restart();
    let restored = app.command(json!({"command":"open","path":path}));
    assert_eq!(restored["project"], merged["project"]);
    app.quit();
}

#[test]
fn imported_duplicates_survive_selection_velocity_and_an_out_and_back_drag() {
    let first = Note::new(60, 2.0, 1.0, 72);
    let second = Note::new(60, 2.0, 1.0, 93);
    let (mut app, clip) = fixture(vec![first, second], 1.0, false);
    let before = app.inspect();
    let p = point(&before, 2.5, 60);
    app.command(json!({"command":"pointer","phase":"click","x":p.0,"y":p.1,"shift":true}));
    assert_eq!(app.inspect()["project"], before["project"]);
    // This gesture temporarily separates the imported doubles, then returns unchanged.
    let state = app.inspect();
    let p = point(&state, 2.5, 60);
    let away = point(&state, 10.5, 60);
    pointer(&mut app, "down", p);
    pointer(&mut app, "move", away);
    pointer(&mut app, "move", p);
    pointer(&mut app, "up", p);
    let after = app.inspect();
    assert_eq!(after["project"], before["project"]);
    assert_eq!(after["history"], before["history"]);
    assert_eq!(notes(&after, clip).len(), 2);
    let marker = &after["midi_editor"]["notes"][0]["velocity_rect"];
    let p = (
        marker["min"][0].as_f64().unwrap() + 4.0,
        marker["min"][1].as_f64().unwrap() + 20.0,
    );
    pointer(&mut app, "click", p);
    assert_eq!(
        notes(&app.inspect(), clip).len(),
        2,
        "velocity is not a geometric merge edit"
    );
    app.quit();
}

#[test]
fn multidraw_selects_the_existing_chord_tone_and_keyboard_transpose_merges_immediately() {
    let tone = Note::new(64, 2.0, 1.0, 77);
    let id = tone.id;
    let (mut app, clip) = fixture(
        vec![
            tone,
            Note::new(55, 6.0, 0.5, 80),
            Note::new(72, 6.0, 0.5, 80),
        ],
        1.0,
        false,
    );
    let before = app.inspect();
    app.key("U");
    let p = point(&app.inspect(), 2.0, 60);
    pointer(&mut app, "down", p);
    let committed = pointer(&mut app, "up", p);
    assert_eq!(notes(&committed, clip).len(), 5);
    let pitches: Vec<_> = notes(&committed, clip)
        .iter()
        .filter(|n| n["start"] == 2.0)
        .map(|n| n["pitch"].as_u64().unwrap())
        .collect();
    assert!(pitches.contains(&60) && pitches.contains(&64) && pitches.contains(&67));
    assert!(selected(&app.inspect()).contains(&id));
    assert!(
        notes(&committed, clip)
            .iter()
            .any(|n| n["id"] == id && n["velocity"] == 77 && n["length"] == 1.0)
    );
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();

    let target = Note::new(60, 2.0, 1.0, 68);
    let id = target.id;
    let (mut app, clip) = fixture(vec![target, Note::new(59, 2.0, 1.0, 103)], 1.0, false);
    let before = app.inspect();
    let p = point(&before, 2.5, 59);
    pointer(&mut app, "click", p);
    let merged = app.command(json!({"command":"key","key":"ArrowUp","cmd":true}));
    assert_eq!(notes(&merged, clip).len(), 1);
    assert_eq!(notes(&merged, clip)[0]["id"], id);
    assert_eq!(notes(&merged, clip)[0]["velocity"], 68);
    assert_eq!(selected(&app.inspect()), vec![id]);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn saving_during_a_held_draw_ends_the_gesture_before_merging_its_ids() {
    let target = Note::new(60, 2.0, 1.0, 75);
    let id = target.id;
    let (mut app, clip) = fixture(vec![target], 1.0, false);
    app.key("W");
    let state = app.inspect();
    let start = point(&state, 1.0, 60);
    let end = point(&state, 4.0, 60);
    let farther = point(&state, 6.0, 60);
    pointer(&mut app, "down", start);
    pointer(&mut app, "move", end);
    assert_eq!(notes(&app.inspect(), clip).len(), 2);
    app.command(json!({"command":"key","key":"S","cmd":true}));
    let saved = app.command(json!({"command":"wait_project"}));
    assert_eq!(notes(&saved, clip).len(), 1);
    assert_eq!(notes(&saved, clip)[0]["id"], id);
    pointer(&mut app, "move", farther);
    let released = pointer(&mut app, "up", farther);
    assert_eq!(
        released["project"], saved["project"],
        "held frames cannot resize a merged-away ID after Save"
    );
    assert_eq!(saved["history"]["undo"].as_array().unwrap().len(), 1);
    app.quit();
}

#[test]
fn generated_chord_merges_its_covered_tone_without_rewriting_existing_velocity() {
    for folded in [false, true] {
        let target = Note::new(if folded { 64 } else { 60 }, 0.0, 8.0, 74);
        let id = target.id;
        let (mut app, clip) = fixture(
            vec![
                target,
                Note::new(55, 6.0, 0.5, 80),
                Note::new(72, 6.0, 0.5, 80),
            ],
            1.0,
            false,
        );
        if folded {
            app.click("View");
            app.click("Only scale keys");
            app.key("Escape");
        }
        let before = app.inspect();
        app.click("Generate…");
        for (field, value) in [("start_bar", "1"), ("length_bars", "2")] {
            let state = app.inspect();
            let rect = &state["midi_editor"]["generate"]["controls"][field];
            let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
            let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
            pointer(&mut app, "double_click", (x, y));
            app.command(json!({"command":"key","key":"A","cmd":true}));
            app.command(json!({"command":"text","text":value}));
            app.key("Enter");
        }
        let draft = app.inspect();
        assert_eq!(
            draft["midi_editor"]["generate"]["notes"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(draft["project"], before["project"]);
        app.click("Add notes");
        let committed = app.click("Done");
        assert_eq!(notes(&committed, clip).len(), 5);
        assert!(
            notes(&committed, clip)
                .iter()
                .any(|n| n["id"] == id && n["velocity"] == 74 && n["length"] == 8.0)
        );
        assert_eq!(selected(&app.inspect()).contains(&id), !folded);
        assert_eq!(committed["history"]["undo"].as_array().unwrap().len(), 1);
        if folded {
            let deleted = app.key("Delete");
            assert_eq!(
                notes(&deleted, clip),
                notes(&before, clip),
                "hidden merged target must stay outside the visible editing selection"
            );
        } else {
            assert_eq!(undo(&mut app, false)["project"], before["project"]);
        }
        app.quit();
    }
}

#[test]
fn covered_paste_outside_a_trimmed_ghost_keeps_its_intended_extension() {
    let target = Note::new(60, 6.0, 2.0, 83);
    let id = target.id;
    let (mut app, ghost) = fixture_window(vec![target], 1.0, true, 4.0);
    let state = app.inspect();
    let original = state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] != ghost)
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let state = open_editor(&mut app, original);
    let p = point(&state, 6.5, 60);
    pointer(&mut app, "click", p);
    app.command(json!({"command":"clipboard","action":"copy"}));
    open_editor(&mut app, ghost);
    app.key("P");
    let p = point(&app.inspect(), 6.0, 60);
    pointer(&mut app, "click", p);
    app.key("Q");
    let before = app.inspect();
    let pasted = app.command(json!({"command":"clipboard","action":"paste"}));
    assert_eq!(notes(&pasted, ghost), notes(&before, ghost));
    assert_eq!(notes(&pasted, ghost)[0]["id"], id);
    let clips = pasted["project"]["tracks"][0]["clips"].as_array().unwrap();
    let target = clips.iter().find(|c| c["id"] == ghost).unwrap();
    assert_eq!(
        target["length"], 8.0,
        "the final pasted region intentionally reaches beyond the old visible end"
    );
    assert_eq!(
        clips.iter().find(|c| c["id"] == original).unwrap()["length"],
        8.0
    );
    assert_eq!(pasted["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn drawing_outside_then_back_inside_reverses_only_the_temporary_extension() {
    let (mut app, clip) = fixture(vec![Note::new(64, 2.0, 1.0, 79)], 1.0, false);
    let before = app.inspect();
    app.key("W");
    let state = app.inspect();
    let start = point(&state, 1.0, 60);
    let outside = point(&state, 10.0, 60);
    pointer(&mut app, "down", start);
    let extended = pointer(&mut app, "move", outside);
    assert!(
        extended["project"]["tracks"][0]["clips"][0]["length"]
            .as_f64()
            .unwrap()
            > 8.0
    );
    pointer(&mut app, "move", start);
    let finished = pointer(&mut app, "up", start);
    assert_eq!(finished["project"]["tracks"][0]["clips"][0]["length"], 8.0);
    assert_eq!(notes(&finished, clip).len(), 2);
    assert!(
        notes(&finished, clip)
            .iter()
            .any(|n| n["pitch"] == 60 && n["start"] == 1.0 && n["length"] == 0.25)
    );
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}
