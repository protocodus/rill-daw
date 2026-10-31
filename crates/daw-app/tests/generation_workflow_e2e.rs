//! Repeatable whole-bar phrases and continued generation through the real app.
mod common;

use common::Harness;
use daw_core::{Clip, MAX_BEAT, Project};
use serde_json::{Value, json};

fn fixture(start: f64, offset: f64, rate: f64, ghost: bool) -> (Harness, u64, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1440,"height":1000}));
    let mut project = Project::new();
    let track = project.add_midi_track("Phrase channel");
    let clip = Clip::midi("Phrase", start, 4.0, vec![]);
    let original = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let selected = if ghost {
        project.clone_clip(track, original).unwrap()
    } else {
        original
    };
    let clip = project.clip_mut(track, selected).unwrap();
    clip.offset = offset;
    clip.source_rate = rate;
    let path = app.directory.path().join("phrase-workflow.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let state = app.click("Fit");
    let rect = state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == selected)
        .unwrap()["rect"]
        .clone();
    click_rect(&mut app, &rect);
    app.click("Window");
    app.click("Clip Editor");
    (app, original, selected)
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

fn generator(state: &Value) -> &Value {
    let result = &state["midi_editor"]["generate"];
    assert!(
        !result.is_null(),
        "Generator stays open: {}",
        state["visible_labels"]
    );
    result
}

fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}

fn notes(state: &Value, id: u64) -> &[Value] {
    clip(state, id)["content"]["Midi"]["notes"]
        .as_array()
        .unwrap()
}

fn control(app: &mut Harness, field: &str) -> Value {
    for _ in 0..16 {
        let state = app.inspect();
        let controls = &generator(&state)["controls"];
        let rect = &controls[field];
        assert!(!rect.is_null(), "Missing {field}: {controls}");
        if ["start_bar", "length_bars"].contains(&field) {
            return rect.clone();
        }
        let options = &controls["options"];
        let (_, y) = center(rect);
        let top = options["min"][1].as_f64().unwrap();
        let bottom = options["max"][1].as_f64().unwrap();
        if y >= top + 2.0 && y <= bottom - 2.0 {
            return rect.clone();
        }
        let (x, y_anchor) = center(options);
        app.command(
            json!({"command":"scroll","x":x,"y":y_anchor,"delta":if y < top {160.0} else {-160.0}}),
        );
    }
    panic!("Could not reveal {field}");
}

fn number(app: &mut Harness, field: &str, value: &str) -> Value {
    let (x, y) = center(&control(app, field));
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":value}));
    app.click("Start bar")
}

fn choice(app: &mut Harness, field: &str, name: &str) -> Value {
    let rect = control(app, field);
    let mut state = click_rect(app, &rect);
    for _ in 0..12 {
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == name)
        {
            break;
        }
        let last = state["label_rects"].as_array().unwrap().last().unwrap();
        let (x, y) = center(last);
        state = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-160.0}));
    }
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == name)
        .count()
        .checked_sub(1)
        .expect("choice visible");
    app.command(json!({"command":"click","label":name,"occurrence":occurrence}))
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":redo}))
}

#[test]
fn short_arpeggio_notes_keep_whole_bars_and_next_range_adds_without_reopening() {
    let (mut app, _, id) = fixture(0.0, 0.0, 1.0, false);
    let before = app.inspect();
    app.click("Generate…");
    app.click("Arpeggio");
    number(&mut app, "start_bar", "1");
    number(&mut app, "length_bars", "4");
    choice(&mut app, "speed_multiplier", "2×");
    let preview = number(&mut app, "gate", "70");
    let expected = generator(&preview)["notes"].as_array().unwrap().len();
    let add_button = preview["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == "Add notes")
        .unwrap();
    let (x, y) = center(add_button);
    let first = app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    assert_eq!(generator(&first)["added_notes"], expected);
    assert_eq!(generator(&first)["preview_visible"], false);
    assert_eq!(clip(&first, id)["length"], 16.0);
    assert_eq!(notes(&first, id).len(), expected);
    assert!(
        notes(&first, id)
            .iter()
            .all(|note| note["start"].as_f64().unwrap() + note["length"].as_f64().unwrap() < 16.0)
    );
    let repeated = app.click("Add notes");
    assert_eq!(repeated["project"], first["project"]);
    assert_eq!(repeated["history"], first["history"]);
    let advanced = app.click("Next range");
    assert_eq!(generator(&advanced)["start_bar"], 5);
    choice(&mut app, "root", "B");
    let second = app.click("Add notes");
    assert_eq!(generator(&second)["added_notes"], expected * 2);
    assert_eq!(clip(&second, id)["length"], 32.0);
    assert_eq!(&notes(&second, id)[..expected], notes(&first, id));
    assert_eq!(notes(&second, id).len(), expected * 2);
    assert_eq!(
        second["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 2
    );
    let done = app.click("Done");
    assert!(done["midi_editor"]["generate"].is_null());
    assert_eq!(done["project"], second["project"]);
    assert_eq!(undo(&mut app, false)["project"], first["project"]);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    undo(&mut app, true);
    assert_eq!(undo(&mut app, true)["project"], second["project"]);

    // Repetition must begin on the requested bar boundary, not after the last
    // shortened note. Duplicate the accepted eight-bar phrase through the UI.
    app.click("Arranger");
    let fitted = app.click("Fit");
    let rect = fitted["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["rect"]
        .clone();
    app.click("Select");
    app.click("Duplicate");
    let copied = click_rect(&mut app, &rect);
    let copy = copied["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] != id)
        .expect("duplicate exists");
    assert_eq!(copy["start"], 32.0);
    assert_eq!(copy["length"], 32.0);
    app.quit();
}

#[test]
fn full_requested_range_extends_only_the_trimmed_stretched_ghost_occurrence() {
    let (mut app, original, id) = fixture(20.0, 1.5, 0.75, true);
    let before = app.inspect();
    app.click("Generate…");
    app.click("Arpeggio");
    number(&mut app, "start_bar", "3");
    number(&mut app, "length_bars", "4");
    let preview = number(&mut app, "gate", "70");
    let expected = generator(&preview)["notes"].as_array().unwrap().clone();
    let added = app.click("Add notes");
    generator(&added);
    assert_eq!(clip(&added, id)["length"], 24.0);
    for field in ["start", "length", "offset", "source_rate"] {
        assert_eq!(
            clip(&added, original)[field],
            clip(&before, original)[field]
        );
    }
    for (actual, expected) in notes(&added, id).iter().zip(expected) {
        assert!(
            (actual["start"].as_f64().unwrap() - 1.5 - expected["start"].as_f64().unwrap() * 0.75)
                .abs()
                < 1e-9
        );
        assert!(
            (actual["length"].as_f64().unwrap() - expected["length"].as_f64().unwrap() * 0.75)
                .abs()
                < 1e-9
        );
    }
    assert_eq!(notes(&added, id), notes(&added, original));
    // Closing through Escape after a successful Add retains committed notes.
    let closed = app.key("Escape");
    assert!(closed["midi_editor"]["generate"].is_null());
    assert_eq!(closed["project"], added["project"]);
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    app.quit();
}

#[test]
fn requested_ranges_respect_both_timeline_and_stretched_source_limits() {
    for (start, offset, rate) in [(MAX_BEAT - 4.0, 0.0, 1.0), (0.0, MAX_BEAT - 8.0, 2.0)] {
        let (mut app, _, id) = fixture(start, offset, rate, false);
        app.click("Generate…");
        number(&mut app, "start_bar", "1");
        number(&mut app, "length_bars", "1");
        let valid = app.click("Add notes");
        assert_eq!(clip(&valid, id)["length"], 4.0);
        let advanced = app.click("Next range");
        assert_eq!(generator(&advanced)["start_bar"], 1);
        let invalid = number(&mut app, "length_bars", "2");
        assert!(!generator(&invalid)["error"].is_null());
        assert!(generator(&invalid)["notes"].as_array().unwrap().is_empty());
        let blocked = app.click("Add notes");
        assert_eq!(blocked["project"], valid["project"]);
        let done = app.click("Done");
        assert_eq!(done["project"], valid["project"]);
        app.quit();
    }
}

#[test]
fn all_roots_are_visible_and_scale_sync_is_explicit_and_view_only() {
    let (mut app, _, _) = fixture(0.0, 0.0, 1.0, false);
    app.command(json!({"command":"viewport","width":1100,"height":720}));
    app.click("Generate…");
    let rect = control(&mut app, "root");
    let opened = click_rect(&mut app, &rect);
    for name in [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ] {
        assert!(
            opened["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label == name),
            "All roots visible without scrolling: {name}"
        );
    }
    app.click("B");
    choice(&mut app, "harmony", "Scale");
    choice(&mut app, "scale", "Dorian");
    let before_sync = choice(&mut app, "root", "D");
    for key in before_sync["midi_editor"]["keys"].as_array().unwrap() {
        let pitch = key["pitch"].as_u64().unwrap() % 12;
        assert_eq!(
            key["scale"],
            [0, 2, 3, 5, 7, 8, 10].contains(&pitch),
            "Generator choices must not silently change the C minor grid"
        );
    }
    let synced = app.click("Use scale for grid");
    assert_eq!(synced["project"], before_sync["project"]);
    assert_eq!(synced["history"], before_sync["history"]);
    for key in synced["midi_editor"]["keys"].as_array().unwrap() {
        let pitch = key["pitch"].as_u64().unwrap() % 12;
        assert_eq!(
            key["scale"],
            [0, 2, 4, 5, 7, 9, 11].contains(&pitch),
            "Explicit scale sync updates the visible grid to D Dorian"
        );
    }
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], before_sync["project"]);
    let drawing = app.click("Draw settings");
    assert!(
        drawing["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "D · Dorian")
    );
    assert_eq!(drawing["project"], before_sync["project"]);
    assert_eq!(drawing["history"], before_sync["history"]);
    app.quit();
}
