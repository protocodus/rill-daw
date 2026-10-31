//! The sound stays reachable while composing, without losing the edited phrase.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}

fn close_popup(app: &mut Harness, state: &Value, title: &str) -> Value {
    let title = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == title)
        .expect("popup title is visible");
    let (x, y) = center(title);
    let bounds = state["window_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| {
            rect["min"][0].as_f64().unwrap() < x
                && rect["max"][0].as_f64().unwrap() > x
                && rect["min"][1].as_f64().unwrap() < y
                && rect["max"][1].as_f64().unwrap() > y
        })
        .expect("popup contains its title");
    let x = bounds["max"][0].as_f64().unwrap() - (y - bounds["min"][1].as_f64().unwrap());
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn editing_context(state: &Value) -> Value {
    let mut selected: Vec<_> = state["midi_editor"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|note| note["selected"] == true)
        .map(|note| note["id"].as_u64().unwrap())
        .collect();
    selected.sort_unstable();
    json!({
        "view":state["view"],
        "track":state["selected_track"],
        "clip":state["selected_clip"],
        "notes":selected,
        "start":state["midi_editor"]["start_beat"],
        "zoom":state["midi_editor"]["pixels_per_beat"],
        "row":state["midi_editor"]["row_offset"],
        "height":state["midi_editor"]["row_height"],
    })
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified", "revision"] {
        assert_eq!(state[field], before[field], "navigation changed {field}");
    }
    assert_eq!(editing_context(state), editing_context(before));
}

#[test]
fn clip_instrument_action_survives_assignment_and_restores_notes_and_view_on_return() {
    let fixture = common::plugin_fixture();
    for (dark, width, density) in [(false, 1100, 1.0), (true, 800, 2.0)] {
        let mut app = Harness::with_vst_path(fixture.path());
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":width, "height":720, "pixels_per_point":density}),
        );
        let mut project = Project::new();
        let track = project.add_midi_track("Evening lead");
        let late = Note::new(67, 12.0, 1.0, 100);
        let note_id = late.id;
        let phrase = Clip::midi(
            "Second phrase",
            4.0,
            16.0,
            vec![Note::new(60, 0.0, 1.0, 90), late],
        );
        let clip_id = phrase.id;
        project
            .track_mut(track)
            .unwrap()
            .clips
            .extend([Clip::midi("Opening", 0.0, 4.0, vec![]), phrase]);
        let path = app.directory.path().join("sound-and-phrase.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.click("Fit");
        app.click("Second phrase");
        app.click("Window");
        app.click("Clip Editor");
        let fitted = app.click("Fit");
        let note = fitted["midi_editor"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|note| note["id"] == note_id)
            .unwrap();
        let (x, y) = center(&note["rect"]);
        app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
        app.click("View");
        let before = app.click("Fit selection");
        assert_eq!(before["selected_clip"], clip_id);
        assert_eq!(editing_context(&before)["notes"], json!([note_id]));
        assert!(before["midi_editor"]["start_beat"].as_f64().unwrap() > 0.0);
        assert!(visible(&before, "Choose instrument…"));
        assert!(!visible(&before, "Instrument"));

        let chooser = app.click("Choose instrument…");
        assert_eq!(chooser["popups"]["library"], true);
        let cancelled = close_popup(&mut app, &chooser, "Device library");
        unchanged(&cancelled, &before);

        app.click("Choose instrument…");
        common::wait_library_device(&mut app, "Rill Fixture A");
        let assigned = app.click("Assign instrument");
        assert_eq!(editing_context(&assigned), editing_context(&before));
        assert_eq!(
            assigned["project"]["tracks"][0]["clips"],
            before["project"]["tracks"][0]["clips"]
        );
        assert!(visible(&assigned, "Instrument"));
        assert!(!visible(&assigned, "Choose instrument…"));
        assert_eq!(assigned["native_editor"]["open"], false);
        assert_eq!(assigned["popups"]["instrument"], false);

        for mouse_close in [true, false] {
            let overview = app.click("Instrument");
            assert_eq!(overview["popups"]["instrument"], true);
            assert_eq!(overview["selected_track"], track);
            assert_eq!(
                overview["popup_context"]["source"],
                "Clip Editor · Second phrase"
            );
            assert_eq!(overview["native_editor"]["open"], false);
            assert!(visible(&overview, "Open editor"));
            assert!(visible(&overview, "Change instrument"));
            let returned = if mouse_close {
                close_popup(&mut app, &overview, "Instrument of Evening lead")
            } else {
                app.key("Escape")
            };
            assert_eq!(returned["popups"]["instrument"], false);
            unchanged(&returned, &assigned);
            assert!(visible(&returned, "Instrument"));
        }
        app.quit();
    }
}
