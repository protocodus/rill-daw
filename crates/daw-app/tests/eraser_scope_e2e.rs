//! Pointed Erase and explicit group Delete keep distinct scopes after Cut.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(midi: bool) -> (Harness, u64) {
    let mut app = Harness::new();
    let source = app.directory.path().join("original.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 192_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let track = if midi {
        project.add_midi_track("Cut notes")
    } else {
        project.add_audio_track("Cut audio")
    };
    let mut clip = if midi {
        Clip::midi(
            "Cut this phrase",
            2.0,
            4.0,
            vec![Note::new(60, 1.0, 1.0, 90), Note::new(67, 3.0, 1.0, 80)],
        )
    } else {
        Clip::audio("Cut this phrase", 2.0, 4.0, source.to_string_lossy())
    };
    clip.offset = 1.0;
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let other = project.add_midi_track("Keep this track");
    project.track_mut(other).unwrap().clips.push(Clip::midi(
        "Unrelated phrase",
        2.0,
        4.0,
        vec![Note::new(48, 0.0, 4.0, 75)],
    ));
    let path = app.directory.path().join("erase-scope.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1300, "height":900}));
    app.click("Fit");
    (app, id)
}

fn center(state: &Value, id: u64) -> (f64, f64) {
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"];
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn click_clip(app: &mut Harness, id: u64, secondary: bool) -> Value {
    let state = app.inspect();
    let (x, y) = center(&state, id);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y,
        "button":if secondary { "secondary" } else { "primary" }}))
}

fn menu(app: &mut Harness, item: &str) -> Value {
    app.click("Edit");
    app.click(item)
}

fn undo_count(state: &Value) -> usize {
    state["history"]["undo"].as_array().unwrap().len()
}

fn assert_selection_valid(state: &Value, deleted: &[u64]) {
    let existing: Vec<_> = state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().unwrap())
        .map(|clip| clip["id"].as_u64().unwrap())
        .collect();
    let mut selected: Vec<_> = state["timeline"]["selected_clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_u64().unwrap())
        .collect();
    if let Some(primary) = state["selected_clip"].as_u64() {
        selected.push(primary);
    }
    for id in selected {
        assert!(existing.contains(&id), "Dangling selected clip {id}");
        assert!(!deleted.contains(&id), "Deleted clip {id} remains selected");
    }
}

fn cut_then_erase(midi: bool) {
    let (mut app, left) = fixture(midi);
    let original = app.inspect();
    let tool = app.click("Cut");
    assert_eq!(tool["timeline"]["tool"], "Cut");
    assert_eq!(tool["project"], original["project"]);
    let split = click_clip(&mut app, left, false);
    let right = split["selected_clip"].as_u64().unwrap();
    assert_ne!(left, right);
    assert_eq!(split["timeline"]["selected_clips"], json!([left, right]));
    assert_eq!(
        split["project"]["tracks"][0]["clips"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(split["project"]["tracks"][0]["clips"][0]["length"], 2.0);
    assert_eq!(split["project"]["tracks"][0]["clips"][1]["start"], 4.0);
    assert_eq!(split["project"]["tracks"][0]["clips"][1]["offset"], 3.0);
    assert_eq!(undo_count(&split), undo_count(&original) + 1);

    let tool = app.click("Erase");
    assert_eq!(tool["timeline"]["tool"], "Erase");
    assert_eq!(tool["project"], split["project"]);
    assert_eq!(tool["history"], split["history"]);
    assert_eq!(tool["timeline"]["selected_clips"], json!([left, right]));
    let erased = click_clip(&mut app, right, false);
    let mut expected = split["project"].clone();
    expected["tracks"][0]["clips"]
        .as_array_mut()
        .unwrap()
        .retain(|clip| clip["id"] != right);
    assert_eq!(
        erased["project"], expected,
        "Only the pointed right piece is erased"
    );
    assert_eq!(undo_count(&erased), undo_count(&split) + 1);
    assert_selection_valid(&erased, &[right]);

    let restored = menu(&mut app, "Undo");
    assert_eq!(restored["project"], split["project"]);
    assert_eq!(restored["selected_clip"], split["selected_clip"]);
    assert_eq!(
        restored["timeline"]["selected_clips"],
        split["timeline"]["selected_clips"]
    );
    assert_eq!(undo_count(&restored), undo_count(&split));
    let redone = menu(&mut app, "Redo");
    assert_eq!(redone["project"], erased["project"]);
    assert_eq!(undo_count(&redone), undo_count(&erased));
    assert_selection_valid(&redone, &[right]);

    // The explicit context action remains a group operation on both split pieces.
    menu(&mut app, "Undo");
    app.click("Select");
    let context = click_clip(&mut app, right, true);
    assert_eq!(context["timeline"]["selected_clips"], json!([left, right]));
    let deleted = app.click("Delete");
    let mut expected = split["project"].clone();
    expected["tracks"][0]["clips"] = json!([]);
    assert_eq!(
        deleted["project"], expected,
        "Context Delete removes the selected group only"
    );
    assert_eq!(undo_count(&deleted), undo_count(&split) + 1);
    assert_selection_valid(&deleted, &[left, right]);
    let restored = menu(&mut app, "Undo");
    assert_eq!(restored["project"], split["project"]);
    assert_eq!(
        restored["timeline"]["selected_clips"],
        split["timeline"]["selected_clips"]
    );
    assert_eq!(menu(&mut app, "Redo")["project"], deleted["project"]);
    app.quit();
}

#[test]
fn erase_after_audio_cut_removes_only_the_pointed_piece_and_context_delete_keeps_group_scope() {
    cut_then_erase(false);
}

#[test]
fn erase_after_midi_cut_removes_only_the_pointed_piece_and_context_delete_keeps_group_scope() {
    cut_then_erase(true);
}
