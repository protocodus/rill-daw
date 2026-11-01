//! Colors communicate purpose through the running production UI.
mod common;
use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use daw_ui::theme;
use serde_json::{Value, json};

fn text_color(state: &Value, label: &str, color: eframe::egui::Color32) {
    assert!(
        state["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| text["text"] == label
                && text["colors"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(color.to_array()))),
        "Missing {label} in {color:?}"
    );
}
#[test]
fn neutral_defaults_and_semantic_roles_survive_theme_changes_and_saved_user_colors() {
    let mut app = Harness::new();
    app.click("Track");
    let created = app.click("Add audio track");
    assert_eq!(
        created["project"]["tracks"][0]["color"],
        json!([148, 153, 162])
    );
    let mut project = Project::new();
    let source = project.add_audio_track("Neutral source");
    project.add_bus("Neutral return");
    let lane = project
        .add_automation_track(
            "Volume envelope",
            AutomationTarget {
                track: source,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.automation_lane_mut(lane).unwrap().points = vec![
        AutomationPoint::new(0.0, 0.2),
        AutomationPoint::new(4.0, 0.8),
    ];
    let path = app.directory.path().join("semantic-colors.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1440,"height":1100,"pixels_per_point":density}),
        );
        app.click("Arranger");
        app.click("Fit");
        let colors = if dark { theme::DARK } else { theme::LIGHT };
        let before = app.inspect();
        let state = app.command(json!({"command":"inspect","paint":true}));
        text_color(&state, "Play", colors.action);
        text_color(&state, "Mixer", colors.text);
        text_color(&state, "AI", colors.ai);
        assert_eq!(colors.cursor, colors.guide);
        assert!(
            state["painted_paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path["stroke"] == json!(colors.automation.to_array()))
        );
        assert_eq!(state["project"], before["project"]);
        app.click("Loop");
        let looped = app.command(json!({"command":"inspect","paint":true}));
        assert!(
            looped["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|shape| shape["fill"] == json!(colors.guide.gamma_multiply(0.6).to_array()))
        );
        app.click("Loop");
    }
    // Custom identity colors persist as part of the song across appearances.
    let mut custom = project.clone();
    custom.track_mut(source).unwrap().color = [135, 110, 185];
    let path = app.directory.path().join("custom-colors.rill");
    custom.save(&path).unwrap();
    let opened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        opened["project"]["tracks"][0]["color"],
        json!([135, 110, 185])
    );
    let mixer = app.click("Mixer");
    assert_eq!(mixer["project"], opened["project"]);
    let painted = app.command(json!({"command":"inspect","paint":true}));
    let send = &painted["mixer"]["channels"][0]["sends"];
    assert!(
        painted["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| {
                text["text"] == "Add send…"
                    && text["colors"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(theme::DARK.action.to_array()))
                    && text["rect"]["min"][0].as_f64().unwrap() >= send["min"][0].as_f64().unwrap()
                    && text["rect"]["min"][1].as_f64().unwrap() >= send["min"][1].as_f64().unwrap()
                    && text["rect"]["max"][0].as_f64().unwrap() <= send["max"][0].as_f64().unwrap()
                    && text["rect"]["max"][1].as_f64().unwrap() <= send["max"][1].as_f64().unwrap()
            })
    );
    app.quit();
}
