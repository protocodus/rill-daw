//! Arrow navigation selects musical items without editing their source data.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Clip, Note, Project};
use serde_json::{Value, json};

fn fixture() -> (Harness, Vec<u64>, Vec<u64>) {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Navigation keys");
    let notes = vec![
        Note::new(60, 0.0, 1.0, 90),
        Note::new(60, 4.0, 1.0, 80),
        Note::new(64, 4.0, 1.0, 100),
        Note::new(67, 32.0, 1.0, 70),
    ];
    let ids = notes.iter().map(|note| note.id).collect();
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("Navigation phrase", 0.0, 40.0, notes));
    let automation = project
        .add_automation_track(
            "Navigation events",
            AutomationTarget {
                track,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let points = vec![
        AutomationPoint::new(0.0, 0.2),
        AutomationPoint::new(4.0, 0.8),
        AutomationPoint::new(8.0, 0.5),
        AutomationPoint::new(64.0, 0.9),
    ];
    let point_ids = points.iter().map(|point| point.id).collect();
    project
        .track_mut(automation)
        .unwrap()
        .automation
        .as_mut()
        .unwrap()
        .points = points;
    let path = app.directory.path().join("navigation.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    (app, ids, point_ids)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

fn open_notes(app: &mut Harness) -> Value {
    app.click("Navigation phrase");
    app.click("Window");
    app.click("Clip Editor")
}

fn selection(state: &Value) -> Vec<u64> {
    let mut ids: Vec<_> = state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|note| note["selected"] == true)
        .map(|note| note["id"].as_u64().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "revision", "modified", "playing"] {
        assert_eq!(state[field], before[field], "selection changed {field}");
    }
}

#[test]
fn note_context_menu_toggles_scattered_selection_with_mouse_only() {
    let (mut app, ids, _) = fixture();
    open_notes(&mut app);
    let state = app.click("Fit");
    let rect = |state: &Value, id: u64| {
        state["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|note| note["id"] == id)
            .unwrap()["rect"]
            .clone()
    };
    let before = click_rect(&mut app, &rect(&state, ids[0]));
    let (x, y) = center(&rect(&before, ids[2]));
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    let added = app.click("Add to selection");
    let mut expected = vec![ids[0], ids[2]];
    expected.sort_unstable();
    assert_eq!(selection(&added), expected);
    unchanged(&added, &before);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    let removed = app.click("Remove from selection");
    assert_eq!(selection(&removed), vec![ids[0]]);
    unchanged(&removed, &before);
    let clicked = click_rect(&mut app, &rect(&removed, ids[0]));
    assert_eq!(selection(&clicked), vec![ids[0]]);
    unchanged(&clicked, &before);
    app.quit();
}

#[test]
fn arrows_select_notes_and_shift_extends_while_modified_arrows_edit() {
    let (mut app, ids, _) = fixture();
    let state = open_notes(&mut app);
    let rect = state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == ids[0])
        .unwrap()["rect"]
        .clone();
    let before = click_rect(&mut app, &rect);
    assert_eq!(selection(&before), vec![ids[0]]);
    let right = app.key("ArrowRight");
    assert_eq!(selection(&right), vec![ids[1]]);
    unchanged(&right, &before);
    let up = app.key("ArrowUp");
    assert_eq!(selection(&up), vec![ids[2]]);
    unchanged(&up, &before);
    app.click("Notes");
    let in_menu = app.key("ArrowRight");
    assert_eq!(selection(&in_menu), vec![ids[2]]);
    unchanged(&in_menu, &before);
    app.key("Escape");
    assert_eq!(selection(&app.key("ArrowDown")), vec![ids[1]]);
    let extended = app.command(json!({"command":"key","key":"ArrowRight","shift":true}));
    let mut wanted = vec![ids[1], ids[3]];
    wanted.sort_unstable();
    assert_eq!(selection(&extended), wanted);
    unchanged(&extended, &before);
    assert_eq!(
        extended["midi_editor"]["pixels_per_beat"],
        before["midi_editor"]["pixels_per_beat"]
    );
    assert_eq!(
        extended["midi_editor"]["row_height"],
        before["midi_editor"]["row_height"]
    );
    assert!(
        extended["midi_editor"]["start_beat"].as_f64().unwrap() > 0.0,
        "selected offscreen note is revealed"
    );
    let left = app.key("ArrowLeft");
    assert_eq!(selection(&left), vec![ids[2]]);
    unchanged(&left, &before);
    let edited = app.command(json!({"command":"key","key":"ArrowUp","cmd":true}));
    let note = edited["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == ids[2])
        .unwrap();
    assert_eq!(note["pitch"], 65);
    assert_eq!(
        app.command(json!({"command":"key","key":"Z","cmd":true}))["project"],
        before["project"]
    );
    app.quit();
}

#[test]
fn piano_keys_have_a_cursor_and_enter_audition_releases_without_editing() {
    let (mut app, _, _) = fixture();
    let state = open_notes(&mut app);
    let rect = state["midi_editor"]["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["pitch"] == 60)
        .unwrap()["rect"]
        .clone();
    let before = click_rect(&mut app, &rect);
    assert_eq!(before["midi_editor"]["keyboard_pitch"], 60);
    let up = app.key("ArrowUp");
    assert_eq!(up["midi_editor"]["keyboard_pitch"], 61);
    assert!(
        up["midi_editor"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|key| key["pitch"] == 61 && key["selected"] == true)
    );
    unchanged(&up, &before);
    let down = app.key("ArrowDown");
    assert_eq!(down["midi_editor"]["keyboard_pitch"], 60);
    let released = app.key("Enter");
    assert!(released["audition_request"].is_null());
    unchanged(&released, &before);
    app.quit();
}

#[test]
fn fit_selected_notes_and_previous_view_preserve_selection_and_audio() {
    let (mut app, ids, _) = fixture();
    open_notes(&mut app);
    app.click("View");
    app.click("Only scale keys");
    app.click("View");
    let state = app.inspect();
    let rect = state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == ids[0])
        .unwrap()["rect"]
        .clone();
    let before = click_rect(&mut app, &rect);
    let frame = |state: &Value| {
        let view = &state["midi_editor"];
        json!([
            view["start_beat"],
            view["pixels_per_beat"],
            view["row_offset"],
            view["row_height"]
        ])
    };
    assert_eq!(
        selection(&before),
        vec![ids[0]],
        "fit requires a selected note"
    );
    app.click("View");
    let fitted = app.click("Fit selection");
    assert!(
        fitted["midi_editor"]["pixels_per_beat"].as_f64().unwrap()
            > before["midi_editor"]["pixels_per_beat"].as_f64().unwrap(),
        "fit selection zoom: before={}, after={}, selection={:?}",
        before["midi_editor"]["pixels_per_beat"],
        fitted["midi_editor"]["pixels_per_beat"],
        selection(&fitted)
    );
    assert_eq!(selection(&fitted), vec![ids[0]]);
    unchanged(&fitted, &before);
    app.click("View");
    let previous = app.click("Previous view");
    assert_eq!(frame(&previous), frame(&before));
    assert_eq!(selection(&previous), vec![ids[0]]);
    unchanged(&previous, &before);
    app.click("View");
    assert_eq!(frame(&app.click("Previous view")), frame(&fitted));
    app.quit();
}

#[test]
fn automation_arrows_select_and_extend_points_with_one_undo_for_group_delete() {
    let (mut app, _, ids) = fixture();
    app.click("Navigation events");
    app.click("Window");
    let before = app.click("Automation");
    assert_eq!(before["automation_editor"]["selected_point"], ids[0]);
    let curve_point = |state: &Value, beat: f64, value: f64| {
        let view = &state["automation_editor"];
        (
            view["plot"]["min"][0].as_f64().unwrap()
                + (beat - view["start_beat"].as_f64().unwrap())
                    * view["pixels_per_beat"].as_f64().unwrap(),
            view["plot"]["max"][1].as_f64().unwrap()
                - value
                    * (view["plot"]["max"][1].as_f64().unwrap()
                        - view["plot"]["min"][1].as_f64().unwrap()),
        )
    };
    let (x, y) = curve_point(&before, 4.0, 0.8);
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    let mouse_extended = app.click("Add to selection");
    assert_eq!(
        mouse_extended["automation_editor"]["selected_points"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    unchanged(&mouse_extended, &before);
    let (x, y) = curve_point(&mouse_extended, 0.0, 0.2);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    let right = app.key("ArrowRight");
    assert_eq!(right["automation_editor"]["selected_point"], ids[1]);
    unchanged(&right, &before);
    let down = app.key("ArrowDown");
    assert_eq!(down["automation_editor"]["selected_point"], ids[2]);
    unchanged(&down, &before);
    let up = app.key("ArrowUp");
    assert_eq!(
        up["automation_editor"]["selected_point"],
        ids[1],
        "up: points={:?}, selected={}, unchanged={}",
        up["project"]["tracks"][1]["automation"]["points"],
        up["automation_editor"]["selected_points"],
        up["project"] == before["project"]
    );
    let extended = app.command(json!({"command":"key","key":"ArrowRight","shift":true}));
    assert_eq!(
        extended["automation_editor"]["selected_points"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    unchanged(&extended, &before);
    let deleted = app.click("Delete selected points");
    assert_eq!(
        deleted["project"]["tracks"][1]["automation"]["points"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        app.command(json!({"command":"key","key":"Z","cmd":true}))["project"],
        before["project"]
    );
    let restored = app.inspect();
    let (x, y) = curve_point(&restored, 4.0, 0.8);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"ArrowRight","shift":true}));
    let deleted = app.key("Delete");
    assert_eq!(
        deleted["project"]["tracks"][1]["automation"]["points"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        app.command(json!({"command":"key","key":"Z","cmd":true}))["project"],
        before["project"]
    );
    app.quit();
}
