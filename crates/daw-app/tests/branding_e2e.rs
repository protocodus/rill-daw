//! Brand artwork and native-font lettering through the real headless app.
mod common;
use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn labels(state: &Value, text: &str) -> usize {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| **value == text)
        .count()
}

#[test]
fn identity_and_about_work_in_both_themes_and_return_to_the_previous_workspace() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Brand test");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("branding.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Phrase");
    for dark in [false, true] {
        for (width, height, density) in [(800, 600, 1.0), (1100, 720, 2.0)] {
            app.command(json!({"command":"viewport", "width":width, "height":height, "pixels_per_point":density}));
            let state = app.command(json!({"command":"system_theme", "dark":dark}));
            assert_eq!(
                labels(&state, "RILL"),
                1,
                "one unobtrusive identity in the menu bar"
            );
            let menus: Vec<_> = ["RILL", "File", "Edit", "Track", "Window", "Help"]
                .into_iter()
                .map(|text| {
                    let rect = state["label_rects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|rect| rect["text"] == text)
                        .unwrap();
                    (
                        rect["min"][0].as_f64().unwrap(),
                        rect["max"][0].as_f64().unwrap(),
                    )
                })
                .collect();
            assert!(
                menus.windows(2).all(|pair| pair[0].1 < pair[1].0),
                "brand and menus never overlap: {menus:?}"
            );
            app.click("Mixer");
            let before = app.inspect();
            app.click("Help");
            let about = app.click("About RILL");
            assert_eq!(about["popups"]["about"], true);
            assert_eq!(about["popup_context"]["source"], "Mixer");
            assert!(about["popup_context"]["shapes"].as_u64().unwrap() > 0);
            assert!(
                labels(&about, "RILL") >= 2,
                "About shows the large native-font logo too"
            );
            assert!(labels(&about, "Audio workstation") > 0);
            assert!(labels(&about, &format!("Version {}", env!("CARGO_PKG_VERSION"))) > 0);
            assert_eq!(about["project"], before["project"]);
            assert_eq!(about["history"], before["history"]);
            let blocked = app.key("Delete");
            assert_eq!(
                blocked["project"], before["project"],
                "About owns editing keys"
            );
            let back = app.click("Close");
            assert_eq!(back["popups"]["about"], false);
            assert_eq!(back["view"], "Mixer");
            assert_eq!(back["project"], before["project"]);
            app.click("Help");
            app.click("About RILL");
            let escaped = app.key("Escape");
            assert_eq!(escaped["popups"]["about"], false);
            assert_eq!(escaped["view"], "Mixer");
            app.click("Arranger");
        }
    }
    app.quit();
}
