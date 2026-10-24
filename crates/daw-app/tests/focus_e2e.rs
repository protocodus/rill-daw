//! Focused workspaces exercise the actual application without a window server.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn labels(state: &Value, text: &str) -> usize {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == text)
        .count()
}

fn window(app: &mut Harness, name: &str) -> Value {
    app.click("Window");
    app.click(name)
}

#[test]
fn one_workspace_at_a_time_and_back_preserve_the_project_and_editor_view() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Focused track");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Focused phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 2.0, 100)],
    ));
    let path = app.directory.path().join("focus.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Focused phrase");
    let original = app.inspect()["project"].clone();

    for dark in [true, false] {
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(json!({"command":"viewport","width":1100,"height":760}));
        let editor = window(&mut app, "Clip Editor");
        assert_eq!(editor["view"], "Clip");
        assert_eq!(
            labels(&editor, "Fit"),
            1,
            "Only the active editor has tools"
        );
        assert_eq!(labels(&editor, "TRACKS"), 0);
        assert_eq!(labels(&editor, "Back"), 1);
        assert_eq!(labels(&editor, "Focused track"), 0);
        for action in ["Back", "Properties", "Play", "File", "Window"] {
            let rect = editor["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == action)
                .unwrap_or_else(|| panic!("Missing {action}"));
            assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{rect}");
            assert!(rect["max"][0].as_f64().unwrap() <= 1100.0, "{rect}");
            assert!(rect["max"][1].as_f64().unwrap() <= 760.0, "{rect}");
        }
        let selected = editor["selected_clip"].clone();
        app.click("Fit");
        let before = app.inspect()["midi_editor"].clone();
        let properties = app.click("Properties");
        assert_eq!(properties["popups"]["properties"], true);
        assert_eq!(labels(&properties, "Fit"), 0);
        assert_eq!(labels(&properties, "TRACKS"), 0);
        assert_eq!(
            properties["popup_context"]["source"],
            "Clip Editor · Focused phrase"
        );
        assert!(properties["popup_context"]["shapes"].as_u64().unwrap() > 0);
        assert_eq!(properties["popup_context"]["interactive"], false);

        // The caller is painted, but its previous Back target cannot navigate
        // beneath the utility or change the editor selection/view state.
        let back = editor["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rect| rect["text"] == "Back")
            .unwrap();
        let clicked_background = app.command(json!({
            "command":"pointer", "phase":"click",
            "x":(back["min"][0].as_f64().unwrap()+back["max"][0].as_f64().unwrap())*0.5,
            "y":(back["min"][1].as_f64().unwrap()+back["max"][1].as_f64().unwrap())*0.5,
        }));
        assert_eq!(clicked_background["popups"]["properties"], true);
        assert_eq!(clicked_background["selected_clip"], selected);
        assert_eq!(clicked_background["project"], original);

        let nested = window(&mut app, "Routing");
        assert!(
            nested["popup_context"]["source"]
                .as_str()
                .unwrap()
                .contains("Properties")
        );
        assert!(nested["popup_context"]["shapes"].as_u64().unwrap() > 0);
        assert!(nested["popup_context"]["depth"].as_u64().unwrap() >= 2);
        let caller = app.key("Escape");
        assert_eq!(caller["popups"]["properties"], true);
        assert_eq!(
            caller["popup_context"]["source"],
            "Clip Editor · Focused phrase"
        );
        let restored = app.key("Escape");
        assert_eq!(restored["view"], "Clip");
        assert_eq!(restored["selected_clip"], selected);
        assert_eq!(restored["midi_editor"], before);

        let arrangement = app.click("Back");
        assert_eq!(arrangement["view"], "Arrangement");
        assert_eq!(labels(&arrangement, "Fit"), 1);
        assert_eq!(labels(&arrangement, "TRACKS"), 1);
        assert_eq!(labels(&arrangement, "Back"), 0);
        let mixer = window(&mut app, "Mixer");
        assert_eq!(mixer["view"], "Mixer");
        assert_eq!(labels(&mixer, "TRACKS"), 0);
        assert_eq!(labels(&mixer, "Fit"), 0);
        assert_eq!(labels(&mixer, "Back"), 1);
        let restored = app.click("Back");
        assert_eq!(restored["view"], "Arrangement");
        assert_eq!(restored["project"], original);
        assert_eq!(restored["modified"], false, "Navigation is not an edit");
    }
    app.quit();
}
