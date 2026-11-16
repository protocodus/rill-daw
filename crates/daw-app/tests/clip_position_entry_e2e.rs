//! Bar/beat placement is a mouse-accessible alternative to exact scalar beats.
mod common;

use common::Harness;
use daw_core::{Clip, ClipContent, Note, Project};
use serde_json::{Value, json};

fn fixture(meter: u8, start: f64, width: u32) -> (Harness, u64, u64) {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport", "width":width, "height":800}));
    let mut project = Project::new();
    project.beats_per_bar = meter;
    let track = project.add_midi_track("Position instrument");
    let mut clip = Clip::midi(
        "Position phrase",
        start,
        4.25,
        vec![Note::new(36, 0.0, 0.75, 90), Note::new(43, 2.25, 0.5, 85)],
    );
    clip.offset = 0.125;
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let ghost = project.clone_clip(track, id).unwrap();
    let path = app.directory.path().join("position.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let state = app.inspect();
    let rect = state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()["visible_rect"]
        .clone();
    click_rect(&mut app, &rect);
    app.click("Window");
    app.click("Properties");
    (app, id, ghost)
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

fn notes_without_ids(state: &Value, id: u64) -> Vec<Note> {
    let clip: Clip = serde_json::from_value(clip(state, id).clone()).unwrap();
    let ClipContent::Midi { mut notes } = clip.content else {
        panic!("Expected MIDI")
    };
    for note in &mut notes {
        note.id = 0;
    }
    notes
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn front(app: &mut Harness, text: &str) -> Value {
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == text)
        .count()
        - 1;
    app.command(json!({"command":"click", "label":text, "occurrence":occurrence}))
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "modified",
        "selected_track",
        "selected_clip",
    ] {
        assert_eq!(state[field], before[field], "Unexpected change to {field}");
    }
}

fn open_position(app: &mut Harness, id: u64, meter: u8) -> Value {
    let before = app.inspect();
    let caption = daw_ui::duration::musical_position(
        clip(&before, id)["start"].as_f64().unwrap(),
        &daw_ui::MeterMap::uniform(meter),
    );
    let opener = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == caption)
        .expect("Visible bar/beat position button");
    let window = before["window_rects"].as_array().unwrap().last().unwrap();
    for axis in 0..2 {
        assert!(opener["min"][axis].as_f64().unwrap() >= window["min"][axis].as_f64().unwrap());
        assert!(opener["max"][axis].as_f64().unwrap() <= window["max"][axis].as_f64().unwrap());
    }
    app.click(&caption);
    let opened = app.inspect();
    unchanged(&opened, &before);
    assert!(
        opened["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Apply position")
    );
    opened
}

fn row_control(state: &Value, row: &str, symbol: Option<&str>) -> Value {
    let labels = state["label_rects"].as_array().unwrap();
    let heading = labels
        .iter()
        .rev()
        .find(|label| label["text"] == row)
        .unwrap();
    labels
        .iter()
        .filter(|label| {
            let text = label["text"].as_str().unwrap();
            (center(label).1 - center(heading).1).abs() < 12.0
                && label["min"][0].as_f64().unwrap() > heading["max"][0].as_f64().unwrap()
                && symbol.map_or_else(|| text.parse::<f64>().is_ok(), |symbol| text == symbol)
        })
        .min_by(|a, b| center(a).0.total_cmp(&center(b).0))
        .unwrap()
        .clone()
}

fn type_field(app: &mut Harness, row: &str, value: &str) -> Value {
    let state = app.inspect();
    let (x, y) = center(&row_control(&state, row, None));
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter")
}

#[test]
fn bar_steps_keep_fractional_beats_and_commit_with_apply_position() {
    let start = 1.234_567_891_234;
    let (mut app, id, _) = fixture(7, start, 800);
    let before = app.inspect();
    open_position(&mut app, id, 7);
    for _ in 0..4 {
        let state = app.inspect();
        unchanged(
            &click_rect(&mut app, &row_control(&state, "Bar", Some("+"))),
            &before,
        );
    }
    let staged = app.inspect();
    click_rect(&mut app, &row_control(&staged, "Beat in bar", None));
    unchanged(&front(&mut app, "Start position"), &before);
    let applied = front(&mut app, "Apply position");
    assert_eq!(clip(&applied, id)["start"], start + 28.0);
    assert_eq!(
        applied["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    app.quit();
}

#[test]
fn rounded_position_caption_carries_without_retiming_the_untouched_draft() {
    let start = 3.999_f64;
    let (mut app, id, _) = fixture(4, start, 800);
    let before = app.inspect();
    let labels = before["visible_labels"].as_array().unwrap();
    assert!(labels.iter().any(|label| label == "Bar 2 · Beat 1.00"));
    assert!(!labels.iter().any(|label| label == "Bar 1 · Beat 5.00"));
    app.click("Bar 2 · Beat 1.00");
    let opened = app.inspect();
    unchanged(&opened, &before);
    assert_eq!(row_control(&opened, "Beat in bar", None)["text"], "4.999");
    click_rect(&mut app, &row_control(&opened, "Beat in bar", None));
    let focused = app.inspect();
    unchanged(&focused, &before);
    assert_eq!(
        row_control(&focused, "Beat in bar", None)["text"],
        "4.9990000000000006",
        "The focused field must expose its exact staged value"
    );
    unchanged(&front(&mut app, "Start position"), &before);
    let blurred = app.inspect();
    assert_eq!(row_control(&blurred, "Beat in bar", None)["text"], "4.999");
    assert!(
        !blurred["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "123")
    );
    unchanged(&front(&mut app, "Apply position"), &before);
    assert_eq!(
        clip(&app.inspect(), id)["start"]
            .as_f64()
            .unwrap()
            .to_bits(),
        start.to_bits()
    );
    app.quit();
}

#[test]
fn untouched_cancelled_and_invalid_bar_beat_drafts_do_not_edit_exact_positions() {
    for (meter, start, width) in [
        (3, 1e-20, 800),
        (4, 1.123_456_789_123, 1100),
        (7, 6.999_999_999_99, 800),
    ] {
        let (mut app, id, _) = fixture(meter, start, width);
        let before = app.inspect();
        let opened = open_position(&mut app, id, meter);
        if meter == 7 {
            assert_eq!(
                row_control(&opened, "Beat in bar", None)["text"],
                "7.99999999999",
                "Idle formatting must not suggest a beat in the next bar"
            );
        }
        click_rect(&mut app, &row_control(&opened, "Beat in bar", None));
        unchanged(&front(&mut app, "Start position"), &before);
        unchanged(&front(&mut app, "Apply position"), &before);
        assert_eq!(
            clip(&app.inspect(), id)["start"]
                .as_f64()
                .unwrap()
                .to_bits(),
            start.to_bits()
        );

        open_position(&mut app, id, meter);
        unchanged(&type_field(&mut app, "Bar", "2.5"), &before);
        unchanged(&front(&mut app, "Apply position"), &before);
        unchanged(&front(&mut app, "Cancel"), &before);

        open_position(&mut app, id, meter);
        unchanged(&type_field(&mut app, "Beat in bar", "0"), &before);
        unchanged(&front(&mut app, "Apply position"), &before);
        unchanged(&app.key("Escape"), &before);

        open_position(&mut app, id, meter);
        unchanged(&type_field(&mut app, "Bar", "5"), &before);
        unchanged(&front(&mut app, "Cancel"), &before);
        app.quit();
    }
}

#[test]
fn exact_bar_beat_placement_is_one_undoable_move_and_duplicate_still_appends_the_phrase() {
    for meter in [3, 7] {
        let (mut app, id, ghost) = fixture(meter, 1.234_567_891_234, 1100);
        let before = app.inspect();
        open_position(&mut app, id, meter);
        unchanged(&type_field(&mut app, "Bar", "5"), &before);
        unchanged(&type_field(&mut app, "Beat in bar", "1.125"), &before);
        let applied = front(&mut app, "Apply position");
        assert_eq!(clip(&applied, id)["start"], f64::from(meter) * 4.0 + 0.125);
        for field in ["length", "offset", "source_rate", "content", "link_group"] {
            assert_eq!(clip(&applied, id)[field], clip(&before, id)[field]);
        }
        assert_eq!(clip(&applied, ghost), clip(&before, ghost));
        assert_eq!(
            applied["history"]["undo"].as_array().unwrap().len(),
            before["history"]["undo"].as_array().unwrap().len() + 1
        );
        let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
        assert_eq!(undone["project"], before["project"]);
        let redone = app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":true}));
        assert_eq!(redone["project"], applied["project"]);
        app.key("Escape");
        app.click("Edit");
        let duplicated = app.click("Duplicate");
        let copy_id = duplicated["selected_clip"].as_u64().unwrap();
        assert_ne!(copy_id, id);
        assert_eq!(
            clip(&duplicated, copy_id)["start"].as_f64().unwrap(),
            clip(&applied, id)["start"].as_f64().unwrap()
                + clip(&applied, id)["length"].as_f64().unwrap()
        );
        assert_eq!(
            notes_without_ids(&duplicated, copy_id),
            notes_without_ids(&applied, id)
        );
        assert_eq!(clip(&duplicated, id), clip(&applied, id));
        app.quit();
    }
}
