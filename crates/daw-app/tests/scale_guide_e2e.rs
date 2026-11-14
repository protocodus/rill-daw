//! The scale is an optional visual guide, independent of selection and source notes.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

const PHRASE: &str = "Scale guide phrase";

fn fixture() -> Harness {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":900}));
    let mut project = Project::new();
    let track = project.add_midi_track("Guide instrument");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        PHRASE,
        0.0,
        8.0,
        vec![
            Note::new(60, 0.0, 0.5, 92),
            Note::new(61, 1.0, 0.5, 92),
            Note::new(64, 2.0, 0.5, 92),
        ],
    ));
    let path = app.directory.path().join("scale-guide.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click(PHRASE);
    app.click("Window");
    app.click("Clip Editor");
    app
}

fn note(state: &Value, pitch: u8) -> &Value {
    state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["pitch"] == pitch)
        .unwrap()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "song_session"] {
        assert_eq!(
            state[field], before[field],
            "Guide controls changed {field}"
        );
    }
}

fn close_menu(app: &mut Harness) -> Value {
    app.click(&format!("Clip Editor · {PHRASE}"))
}

#[test]
fn fresh_major_guide_matches_the_initial_chord_and_explicit_harmony_choices_stay_independent() {
    let mut app = fixture();
    let before = app.inspect();
    assert_eq!(before["midi_editor"]["root"], 0);
    assert_eq!(before["midi_editor"]["scale"], "Major");
    assert_eq!(before["midi_editor"]["chord"], "Major triad");
    assert!(
        before["visible_labels"]
            .as_array()
            .unwrap()
            .contains(&json!("Scale guide C · Major"))
    );
    let draft = app.click("Generate…");
    assert_eq!(
        draft["midi_editor"]["generate"]["settings"]["chord"],
        "Major triad"
    );
    assert!(
        draft["midi_editor"]["generate"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|note| { [0, 4, 7].contains(&(note["pitch"].as_u64().unwrap() % 12)) })
    );
    app.click("Cancel");

    app.click("Scale guide C · Major");
    app.click("Major");
    app.click("Natural minor");
    let minor = close_menu(&mut app);
    assert_eq!(minor["midi_editor"]["scale"], "Natural minor");
    assert_eq!(
        minor["midi_editor"]["chord"], "Major triad",
        "Scale does not silently replace an explicit chord"
    );
    assert_eq!(note(&minor, 64)["out_of_scale"], true);
    unchanged(&minor, &before);

    app.click("Scale guide C · Natural minor");
    app.click("Major triad");
    app.click("Minor triad");
    let selected = close_menu(&mut app);
    assert_eq!(selected["midi_editor"]["scale"], "Natural minor");
    assert_eq!(selected["midi_editor"]["chord"], "Minor triad");
    unchanged(&selected, &before);
    app.click("Scale guide C · Natural minor");
    app.click("Natural minor");
    app.click("Major");
    let major = close_menu(&mut app);
    assert_eq!(major["midi_editor"]["scale"], "Major");
    assert_eq!(major["midi_editor"]["chord"], "Minor triad");
    unchanged(&major, &before);
    app.quit();
}

#[test]
fn outside_scale_marker_preserves_neutral_velocity_fill_and_selected_outline_in_both_themes() {
    let mut app = fixture();
    let before = app.inspect();
    for dark in [false, true] {
        let state = app.command(json!({"command":"system_theme","dark":dark}));
        let inside = note(&state, 60);
        let outside = note(&state, 61);
        assert_eq!(
            outside["fill_color"], inside["fill_color"],
            "Same velocity uses the same fill"
        );
        assert!(inside["scale_marker"].is_null());
        assert!(
            !outside["scale_marker"].is_null(),
            "Only the outside note has a guide marker"
        );
        let rect = &outside["rect"];
        let marker = &outside["scale_marker"];
        for axis in 0..2 {
            assert!(marker["min"][axis].as_f64().unwrap() > rect["min"][axis].as_f64().unwrap());
            assert!(marker["max"][axis].as_f64().unwrap() < rect["max"][axis].as_f64().unwrap());
        }
        let (x, y) = center(rect);
        app.command(json!({"command":"pointer","phase":"move","x":x,"y":y}));
        let explanation = "Outside the selected scale guide";
        let mut hovered = app.inspect();
        for _ in 0..40 {
            if hovered["visible_labels"]
                .as_array()
                .unwrap()
                .contains(&json!(explanation))
            {
                break;
            }
            hovered = app.inspect();
        }
        assert!(
            hovered["visible_labels"]
                .as_array()
                .unwrap()
                .contains(&json!(explanation)),
            "Hovering the marked note explains the visual guide"
        );
        unchanged(&hovered, &before);
        let selected = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
        assert_eq!(note(&selected, 61)["selected"], true);
        assert!(note(&selected, 61)["stroke_width"].as_f64().unwrap() > 0.0);
        assert_eq!(note(&selected, 61)["scale_marker"], outside["scale_marker"]);
        assert_eq!(note(&selected, 61)["fill_color"], outside["fill_color"]);
        unchanged(&selected, &before);

        app.click("View");
        app.click("Mark out-of-scale notes");
        let hidden = close_menu(&mut app);
        assert!(note(&hidden, 61)["scale_marker"].is_null());
        assert_eq!(note(&hidden, 61)["fill_color"], outside["fill_color"]);
        assert_eq!(note(&hidden, 61)["selected"], true);
        app.click("View");
        app.click("Mark out-of-scale notes");
        let restored = close_menu(&mut app);
        assert!(!note(&restored, 61)["scale_marker"].is_null());
        unchanged(&restored, &before);
        app.click("Edit");
        app.click("Clear note selection");
    }
    app.quit();
}
