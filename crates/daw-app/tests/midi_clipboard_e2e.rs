//! Native clipboard events and selection gestures through the running piano roll.
mod common;
use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn open() -> Harness {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Keyboard");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Phrase",
        0.0,
        16.0,
        vec![
            Note::new(60, 1.0, 1.0, 70),
            Note::new(64, 3.0, 1.0, 90),
            Note::new(67, 5.0, 1.0, 110),
        ],
    ));
    let path = app.directory.path().join("phrase.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Phrase");
    app.click("Window");
    app.click("Clip Editor");
    app
}

fn notes(state: &Value) -> &[Value] {
    state["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}
fn selected(state: &Value) -> Vec<u64> {
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
fn point(state: &Value, beat: f64, pitch: u8) -> (f64, f64) {
    let view = &state["midi_editor"];
    let row = view["visible_pitches"]
        .as_array()
        .unwrap()
        .iter()
        .position(|value| value.as_u64() == Some(u64::from(pitch)))
        .unwrap() as f64;
    (
        view["plot"]["min"][0].as_f64().unwrap()
            + (beat - view["start_beat"].as_f64().unwrap())
                * view["pixels_per_beat"].as_f64().unwrap(),
        view["plot"]["min"][1].as_f64().unwrap()
            + (row + 0.5 - view["row_offset"].as_f64().unwrap())
                * view["row_height"].as_f64().unwrap(),
    )
}
fn click_note(app: &mut Harness, beat: f64, pitch: u8, shift: bool) -> Value {
    let state = app.inspect();
    let p = point(&state, beat, pitch);
    app.command(json!({"command":"pointer","phase":"click","x":p.0,"y":p.1,"shift":shift}))
}
fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}
fn cursor(app: &mut Harness, beat: f64) {
    app.key("P");
    let state = app.inspect();
    let p = point(&state, beat, 60);
    app.command(json!({"command":"pointer","phase":"click","x":p.0,"y":p.1}));
    app.key("Q");
}

#[test]
fn shift_click_toggles_selection_without_edits_and_shift_drag_still_duplicates() {
    let mut app = open();
    let before = app.inspect();
    let ids: Vec<_> = notes(&before)
        .iter()
        .map(|note| note["id"].as_u64().unwrap())
        .collect();
    click_note(&mut app, 1.5, 60, false);
    assert_eq!(selected(&click_note(&mut app, 3.5, 64, true)).len(), 2);
    let toggled = click_note(&mut app, 1.5, 60, true);
    assert_eq!(
        selected(&toggled),
        vec![ids[1]],
        "Shift-click removes an already selected note"
    );
    assert_eq!(toggled["project"], before["project"]);
    assert_eq!(toggled["history"], before["history"]);
    let restored = click_note(&mut app, 1.5, 60, true);
    assert_eq!(
        selected(&restored).len(),
        2,
        "A rapid second Shift-click adds the note without deleting it"
    );
    let plain = click_note(&mut app, 1.5, 60, false);
    assert_eq!(
        notes(&plain).len(),
        3,
        "A plain selection click immediately after Shift-click must not double-delete the note"
    );
    let restored = click_note(&mut app, 3.5, 64, true);
    let origin = point(&restored, 3.5, 64);
    let scale = restored["midi_editor"]["pixels_per_beat"].as_f64().unwrap();
    let duplicated = app.command(json!({"command":"pointer","phase":"drag","shift":true,"x":origin.0,"y":origin.1,"to_x":origin.0+2.0*scale,"to_y":origin.1}));
    assert_eq!(notes(&duplicated).len(), 5);
    assert_eq!(selected(&duplicated).len(), 2);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn shift_selection_remembers_press_modifiers_when_shift_is_released_before_the_mouse() {
    for already_selected in [true, false] {
        let mut app = open();
        let before = app.inspect();
        let first = notes(&before)[0]["id"].as_u64().unwrap();
        let second = notes(&before)[1]["id"].as_u64().unwrap();
        click_note(&mut app, 1.5, 60, false);
        let (beat, pitch) = if already_selected {
            (1.5, 60)
        } else {
            (3.5, 64)
        };
        let p = point(&app.inspect(), beat, pitch);
        app.command(json!({"command":"pointer","phase":"down","x":p.0,"y":p.1,"shift":true}));
        let released =
            app.command(json!({"command":"pointer","phase":"up","x":p.0,"y":p.1,"shift":false}));
        assert_eq!(released["project"], before["project"]);
        assert_eq!(released["history"], before["history"]);
        let mut expected = if already_selected {
            vec![]
        } else {
            vec![first, second]
        };
        expected.sort_unstable();
        // Pointer-up applies the toggle after notes have painted for that
        // frame; inspect the next frame for the resulting selection styling.
        assert_eq!(selected(&app.inspect()), expected);

        // The shifted gesture cannot seed an unmodified double-click either.
        let plain = click_note(&mut app, beat, pitch, false);
        assert_eq!(plain["project"], before["project"]);
        // A real pair of ordinary clicks still performs the documented delete.
        let double = click_note(&mut app, beat, pitch, false);
        assert_eq!(notes(&double).len(), 2);
        assert_eq!(undo(&mut app, false)["project"], before["project"]);
        app.quit();
    }
}

#[test]
fn native_copy_paste_and_cut_preserve_notes_velocities_and_one_step_history() {
    let mut app = open();
    let before = app.inspect();
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let copied = app.command(json!({"command":"clipboard","action":"copy"}));
    assert!(
        copied["clipboard_text"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "Native paste needs a nonempty system clipboard marker"
    );
    assert_eq!(copied["project"], before["project"]);
    cursor(&mut app, 8.0);
    let pasted = app.command(json!({"command":"clipboard","action":"paste","with_key":true}));
    assert_eq!(
        notes(&pasted).len(),
        6,
        "Native event and key fallback paste once"
    );
    for (pitch, start, velocity) in [(60, 8.0, 70), (64, 10.0, 90), (67, 12.0, 110)] {
        assert!(notes(&pasted).iter().any(|note| note["pitch"] == pitch
            && note["start"] == start
            && note["length"] == 1.0
            && note["velocity"] == velocity));
    }
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    let redone = undo(&mut app, true);
    assert_eq!(redone["project"], pasted["project"]);
    assert_eq!(
        selected(&redone),
        selected(&pasted),
        "Redo restores the pasted phrase selection so Cut can continue immediately"
    );
    let cut = app.command(json!({"command":"clipboard","action":"cut","with_key":true}));
    assert_eq!(notes(&cut).len(), 3);
    assert_eq!(cut["project"], before["project"]);
    assert_eq!(undo(&mut app, false)["project"], pasted["project"]);
    assert_eq!(undo(&mut app, true)["project"], cut["project"]);
    let restored = app.command(json!({"command":"clipboard","action":"paste"}));
    assert_eq!(notes(&restored).len(), 6);
    let path = app.directory.path().join("clipboard.rill");
    app.command(json!({"command":"save","path":path}));
    assert_eq!(
        Project::load(path).unwrap().tracks[0].clips[0].content,
        serde_json::from_value(restored["project"]["tracks"][0]["clips"][0]["content"].clone())
            .unwrap()
    );
    app.quit();
}

#[test]
fn copy_notes_menu_publishes_native_marker_and_text_fields_keep_native_paste() {
    let mut app = open();
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.click("Notes");
    let copied = app.click("Copy notes");
    assert!(
        copied["clipboard_text"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    app.click("Notes");
    assert!(notes(&app.click("Cut notes")).is_empty());
    undo(&mut app, false);
    app.click("Window");
    let properties = app.click("Properties");
    let rect = &properties["clip_name_rect"];
    app.command(json!({"command":"pointer","phase":"click","x":(rect[0].as_f64().unwrap()+rect[2].as_f64().unwrap())*0.5,"y":(rect[1].as_f64().unwrap()+rect[3].as_f64().unwrap())*0.5}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let pasted =
        app.command(json!({"command":"clipboard","action":"paste","text":"Renamed phrase"}));
    assert_eq!(
        pasted["project"]["tracks"][0]["clips"][0]["name"],
        "Renamed phrase"
    );
    assert_eq!(notes(&pasted).len(), 3);
    app.quit();
}
