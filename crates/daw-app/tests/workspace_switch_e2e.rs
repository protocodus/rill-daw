//! Persistent workspace switching remains available above contextual utilities.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn toolbar(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    // Related-view links and native menu items may repeat Mixer. The transport
    // choice is the uppermost painted matching control.
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["text"] == label)
        .min_by(|left, right| {
            left["min"][1]
                .as_f64()
                .unwrap()
                .total_cmp(&right["min"][1].as_f64().unwrap())
        })
        .unwrap_or_else(|| panic!("Missing toolbar {label}: {}", state["visible_labels"]));
    let width = state["viewport"][0].as_f64().unwrap();
    assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
    assert!(rect["max"][0].as_f64().unwrap() <= width);
    assert!(rect["max"][1].as_f64().unwrap() < 160.0);
    app.command(json!({
        "command":"pointer", "phase":"click",
        "x":(rect["min"][0].as_f64().unwrap()+rect["max"][0].as_f64().unwrap())*0.5,
        "y":(rect["min"][1].as_f64().unwrap()+rect["max"][1].as_f64().unwrap())*0.5,
    }))
}

fn window(app: &mut Harness, label: &str) -> Value {
    app.click("Window");
    app.click(label)
}

fn workspace(state: &Value, view: &str, original: &Value) {
    assert_eq!(state["view"], view);
    assert!(
        state["popups"]
            .as_object()
            .unwrap()
            .values()
            .all(|open| open == false)
    );
    assert_eq!(state["project"], original["project"]);
    assert_eq!(state["history"], original["history"]);
    assert_eq!(state["selected_track"], original["selected_track"]);
    assert_eq!(state["selected_clip"], original["selected_clip"]);
    assert_eq!(state["modified"], false);
}

#[test]
fn arranger_mixer_toolbar_switches_from_editors_and_utilities_without_editing() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Switch synth");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Switch phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 2.0, 100)],
    ));
    let path = app.directory.path().join("workspace-switch.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let original = app.click("Switch phrase");

    for (width, height, density) in [(800, 600, 1.0), (1100, 720, 2.0)] {
        for dark in [false, true] {
            app.command(json!({"command":"viewport", "width":width, "height":height,
                "pixels_per_point":density}));
            app.command(json!({"command":"system_theme", "dark":dark}));
            workspace(&toolbar(&mut app, "Mixer"), "Mixer", &original);
            let channel = app.click("Switch synth");
            assert_eq!(channel["popups"]["properties"], true);
            // Selecting the already active caller still dismisses its utility.
            workspace(&toolbar(&mut app, "Mixer"), "Mixer", &original);
            assert_eq!(window(&mut app, "Effects")["popups"]["effects"], true);
            workspace(&toolbar(&mut app, "Arranger"), "Arrangement", &original);
            assert_eq!(window(&mut app, "Effects")["popups"]["effects"], true);
            workspace(&toolbar(&mut app, "Arranger"), "Arrangement", &original);
            assert_eq!(window(&mut app, "Clip Editor")["view"], "Clip");
            workspace(&toolbar(&mut app, "Mixer"), "Mixer", &original);
            workspace(&toolbar(&mut app, "Arranger"), "Arrangement", &original);
            // The Window menu remains a separate working route despite the
            // persistent toolbar having the same Mixer label.
            workspace(&window(&mut app, "Mixer"), "Mixer", &original);
            workspace(&toolbar(&mut app, "Arranger"), "Arrangement", &original);
        }
    }
    app.quit();
}
