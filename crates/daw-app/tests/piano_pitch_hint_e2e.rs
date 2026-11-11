//! Pitches remain readable at the actual pointer before drawing and on tiny notes.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn fixture(dark: bool, density: f64, width: u32, height: u32) -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"system_theme","dark":dark}));
    app.command(json!({"command":"viewport","width":width,"height":height,
        "pixels_per_point":density}));
    let mut project = Project::new();
    let track = project.add_midi_track("Pitch hint instrument");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Pitch hint phrase",
        0.0,
        16.0,
        vec![
            Note::new(60, 0.0, 1.0, 90),
            Note::new(61, 6.0, 0.0625, 90),
            Note::new(72, 12.0, 1.0, 90),
        ],
    ));
    let path = app.directory.path().join("pitch-hint.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Pitch hint phrase");
    app.click("Window");
    app.click("Clip Editor");
    app
}

fn rect(value: &Value) -> [f64; 4] {
    [
        value["min"][0].as_f64().unwrap(),
        value["min"][1].as_f64().unwrap(),
        value["max"][0].as_f64().unwrap(),
        value["max"][1].as_f64().unwrap(),
    ]
}

fn center(rect: [f64; 4]) -> (f64, f64) {
    ((rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5)
}

fn contains(outer: [f64; 4], inner: [f64; 4]) -> bool {
    inner[0] >= outer[0] && inner[1] >= outer[1] && inner[2] <= outer[2] && inner[3] <= outer[3]
}

fn point(state: &Value, pitch: u8) -> (f64, f64) {
    let plot = rect(&state["midi_editor"]["plot"]);
    let key = state["midi_editor"]["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["pitch"] == pitch)
        .unwrap();
    (
        plot[0] + (plot[2] - plot[0]) * 0.65,
        center(rect(&key["rect"])).1,
    )
}

fn pointer(app: &mut Harness, phase: &str, at: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":at.0,"y":at.1}));
    painted(app)
}

fn painted(app: &mut Harness) -> Value {
    // Paint diagnostics are opt-in per command, not retained from an earlier
    // inspection. Observe the held/hovered state without sending more input.
    app.command(json!({"command":"inspect","paint":true}))
}

fn pitch_at(state: &Value, at: (f64, f64)) -> u8 {
    let editor = &state["midi_editor"];
    let row = (editor["row_offset"].as_f64().unwrap()
        + (at.1 - editor["plot"]["min"][1].as_f64().unwrap())
            / editor["row_height"].as_f64().unwrap())
    .floor() as usize;
    editor["visible_pitches"][row].as_u64().unwrap() as u8
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "song_session",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(
            state[field], before[field],
            "Pitch feedback changed {field}"
        );
    }
    let selected = |state: &Value| {
        state["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|note| note["selected"] == true)
            .map(|note| note["id"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(selected(state), selected(before));
}

fn toolbar(state: &Value) -> Vec<Value> {
    [
        "Snap grid",
        "Length last",
        "Key C · Natural minor",
        "Notes",
        "Generate…",
    ]
    .iter()
    .map(|label| {
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == *label)
            .unwrap()
            .clone()
    })
    .collect()
}

fn assert_hint(state: &Value, pitch: u8, pointer: (f64, f64)) -> [f64; 4] {
    let expected = daw_core::note_name(pitch);
    let plot = rect(&state["midi_editor"]["plot"]);
    let hint = &state["midi_editor"]["pitch_hint"];
    assert_eq!(hint["pitch"], pitch);
    assert_eq!(hint["text"], expected);
    let bounds = rect(&hint["rect"]);
    assert!(contains(plot, bounds));
    let painted = state["painted_text"]
        .as_array()
        .expect("Pitch assertions require an inspect command with paint enabled")
        .iter()
        .find(|text| {
            text["text"] == expected
                && text["rendered_text"] == expected
                && contains(bounds, rect(&text["rect"]))
        })
        .unwrap_or_else(|| {
            panic!("Pitch hint must actually paint {expected} near {pointer:?}: {hint}")
        });
    let painted = rect(&painted["rect"]);
    let middle = center(painted);
    assert!(
        contains(plot, painted),
        "A gutter label cannot satisfy this test"
    );
    assert!((middle.0 - pointer.0).hypot(middle.1 - pointer.1) < 100.0);
    assert!(painted[2] > painted[0] && painted[3] > painted[1]);
    painted
}

#[test]
fn pointer_pitch_is_painted_before_entry_and_on_short_dense_and_folded_notes() {
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            let mut app = fixture(dark, density, 1100, 760);
            let before = app.click("Pencil");
            let controls = toolbar(&before);
            let at = point(&before, 61);
            let hovered = pointer(&mut app, "move", at);
            assert_hint(&hovered, 61, at);
            unchanged(&hovered, &before);
            assert_eq!(toolbar(&hovered), controls);

            app.click("Select");
            let selecting = painted(&mut app);
            let short = selecting["midi_editor"]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|note| note["pitch"] == 61)
                .unwrap();
            let note_bounds = rect(&short["rect"]);
            assert!(note_bounds[2] - note_bounds[0] < 10.0);
            assert!(
                !selecting["painted_text"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|text| {
                        text["text"] == "C♯4" && contains(note_bounds, rect(&text["rect"]))
                    })
            );
            let at = center(note_bounds);
            let inspected = pointer(&mut app, "move", at);
            let painted = assert_hint(&inspected, 61, at);
            assert!(!contains(note_bounds, painted));
            unchanged(&inspected, &before);
            assert_eq!(toolbar(&inspected), controls);

            let dense = app.command(json!({"command":"scroll","x":at.0,"y":at.1,
                "delta":-125.0,"ctrl":true,"shift":true}));
            assert!(dense["midi_editor"]["row_height"].as_f64().unwrap() < 13.0);
            let at = point(&dense, 61);
            let dense = pointer(&mut app, "move", at);
            assert_hint(&dense, 61, at);
            unchanged(&dense, &before);
            assert_eq!(toolbar(&dense), controls);

            app.click("View");
            app.click("Only scale keys");
            let folded = app.key("Escape");
            assert!(
                !folded["midi_editor"]["visible_pitches"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(61))
            );
            let at = point(&folded, 63);
            let hovered = pointer(&mut app, "move", at);
            assert_hint(&hovered, 63, at);
            unchanged(&hovered, &before);
            app.quit();
        }
    }
}

#[test]
fn pitch_hint_stays_in_the_plot_and_disappears_over_other_surfaces_and_menus() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = fixture(dark, density, 800, 600);
        let before = app.click("Pencil");
        let controls = toolbar(&before);
        let plot = rect(&before["midi_editor"]["plot"]);
        for at in [
            (plot[0] + 1.0, plot[1] + 1.0),
            (plot[2] - 1.0, plot[1] + 1.0),
            (plot[2] - 1.0, plot[3] - 1.0),
            (plot[0] + 1.0, plot[3] - 1.0),
        ] {
            let hovered = pointer(&mut app, "move", at);
            assert_hint(&hovered, pitch_at(&before, at), at);
            unchanged(&hovered, &before);
            assert_eq!(toolbar(&hovered), controls);
        }
        for field in ["ruler", "velocity"] {
            let at = center(rect(&before["midi_editor"][field]));
            let outside = pointer(&mut app, "move", at);
            assert!(outside["midi_editor"]["pitch_hint"].is_null());
            unchanged(&outside, &before);
        }
        app.click("View");
        let covered = pointer(&mut app, "move", center(plot));
        assert!(covered["midi_editor"]["pitch_hint"].is_null());
        unchanged(&covered, &before);
        app.key("Escape");
        app.quit();
    }
}

#[test]
fn drawing_through_the_previous_hint_keeps_the_pressed_pitch_and_one_undo_step() {
    let mut app = fixture(true, 2.0, 1100, 760);
    let before = app.click("Pencil");
    let at = point(&before, 63);
    let hovered = pointer(&mut app, "move", at);
    assert_hint(&hovered, 63, at);
    let start = center(rect(&hovered["midi_editor"]["pitch_hint"]["rect"]));
    let pitch = pitch_at(&hovered, start);
    let editor = &hovered["midi_editor"];
    let beat = ((editor["start_beat"].as_f64().unwrap()
        + (start.0 - editor["plot"]["min"][0].as_f64().unwrap())
            / editor["pixels_per_beat"].as_f64().unwrap())
        * 4.0)
        .round()
        / 4.0;
    // Move and press arrive together over the previous hint's painted surface.
    let pressed = pointer(&mut app, "down", start);
    assert_hint(&pressed, pitch, start);
    let end = (start.0 + 78.0, start.1 + 18.0);
    let held = pointer(&mut app, "move", end);
    assert_hint(&held, pitch, end);
    let released = pointer(&mut app, "up", end);
    let notes = &released["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"];
    assert_eq!(notes.as_array().unwrap().len(), 4);
    assert!(notes.as_array().unwrap().iter().any(|note| {
        note["pitch"] == pitch && (note["start"].as_f64().unwrap() - beat).abs() < 1e-8
    }));
    assert_eq!(released["history"]["undo"].as_array().unwrap().len(), 1);
    app.click("Edit");
    assert_eq!(app.click("Undo")["project"], before["project"]);

    let selecting = app.click("Select");
    let short = selecting["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["pitch"] == 61)
        .unwrap();
    let at = center(rect(&short["rect"]));
    let inspected = pointer(&mut app, "move", at);
    assert_hint(&inspected, 61, at);
    let pressed = pointer(&mut app, "down", at);
    assert!(pressed["midi_editor"]["pitch_hint"].is_null());
    let selected = pointer(&mut app, "up", at);
    assert_hint(&selected, 61, at);
    assert_eq!(selected["project"], before["project"]);
    let hovered = pointer(&mut app, "move", point(&selected, 63));
    unchanged(&hovered, &selected);
    app.quit();
}

#[test]
fn held_multi_draw_keeps_the_pressed_pitch_for_chord_and_scale_stacks() {
    for scale_stack in [false, true] {
        let mut app = fixture(true, 2.0, 1100, 760);
        app.click("Multi-draw");
        let before = if scale_stack {
            app.click("Chord stack");
            app.click("Scale stack")
        } else {
            app.click("Major triad");
            app.click("Minor triad")
        };
        let original_notes = before["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
            .as_array()
            .unwrap();
        let at = point(&before, 67);
        let hovered = pointer(&mut app, "move", at);
        assert_hint(&hovered, 67, at);
        // Both stacks begin at C4 despite the G4 press. The cue must not jump
        // to the first generated tone while the pointer remains stationary.
        let pressed = pointer(&mut app, "down", at);
        assert_hint(&pressed, 67, at);
        let end = (
            at.0 + 78.0,
            at.1 + 2.0 * before["midi_editor"]["row_height"].as_f64().unwrap(),
        );
        assert_ne!(pitch_at(&before, end), 67);
        let held = pointer(&mut app, "move", end);
        assert_hint(&held, 67, end);
        let released = pointer(&mut app, "up", end);
        let notes = released["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
            .as_array()
            .unwrap();
        for original in original_notes {
            assert!(
                notes.contains(original),
                "Pitch feedback must preserve existing notes"
            );
        }
        let created: Vec<_> = notes
            .iter()
            .filter(|note| {
                !original_notes
                    .iter()
                    .any(|original| original["id"] == note["id"])
            })
            .map(|note| note["pitch"].as_u64().unwrap())
            .collect();
        assert_eq!(
            created,
            if scale_stack {
                vec![60, 62, 63, 65, 67, 68, 70]
            } else {
                vec![60, 63, 67]
            },
            "The cue must not change stack generation or ordering"
        );
        assert_eq!(released["history"]["undo"].as_array().unwrap().len(), 1);
        app.click("Edit");
        assert_eq!(app.click("Undo")["project"], before["project"]);
        app.quit();
    }
}
