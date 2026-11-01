//! Mixer processing, output metering and toolbar routing through the running app.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn edge(rect: &Value, corner: &str, axis: usize) -> f64 {
    rect[corner][axis].as_f64().unwrap()
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (edge(rect, "min", 0) + edge(rect, "max", 0)) * 0.5,
        (edge(rect, "min", 1) + edge(rect, "max", 1)) * 0.5,
    )
}

fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn click_rect(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

fn unchanged(state: &Value, baseline: &Value) {
    for key in ["project", "history", "modified"] {
        assert_eq!(state[key], baseline[key], "Viewing Mixer changed {key}");
    }
}

#[test]
fn mixer_measures_after_processing_and_exposes_full_size_mute_solo_controls() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_audio_track("Source");
        let other = project.add_midi_track("Other");
        let bus = project.add_bus("Return");
        project.add_send(source, bus, 0.25, true).unwrap();
        let path = app.directory.path().join("mixer-signal-flow.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":900,
            "pixels_per_point":density}));
        let baseline = app.click("Mixer");
        let state = app.command(json!({"command":"inspect", "paint":true}));
        unchanged(&state, &baseline);
        for id in [source, other, bus] {
            let channel = channel(&state, id);
            for pair in ["input", "processing", "measuring", "output"].windows(2) {
                assert!(
                    edge(&channel["stages"][pair[0]], "max", 1)
                        <= edge(&channel["stages"][pair[1]], "min", 1) + 0.1,
                    "Signal stages {pair:?} overlap or run backwards: {channel}"
                );
            }
            for processing in [
                "effects",
                "insert_summary",
                "gain",
                "fader",
                "pan",
                "mute",
                "solo",
            ] {
                assert!(
                    edge(&channel[processing], "max", 1) <= edge(&channel["meter"], "min", 1),
                    "The measured output must follow {processing}"
                );
            }
            assert!(edge(&channel["meter"], "max", 1) <= edge(&channel["target"], "min", 1));
            assert!(edge(&channel["target"], "max", 1) <= edge(&channel["sends"], "min", 1));
            assert!(edge(&channel["sends"], "max", 1) <= 900.0);
            for control in ["mute", "solo"] {
                let rect = &channel[control];
                assert!(edge(rect, "max", 0) - edge(rect, "min", 0) >= 60.0);
                assert!(
                    edge(rect, "max", 1) - edge(rect, "min", 1)
                        >= f64::from(daw_ui::tokens::CONTROL_HEIGHT) - 0.1
                );
            }
        }
        // These actions belong to their own channel even when another channel
        // is selected, and each remains one mouse-reachable Undo operation.
        let selected = state["selected_track"].clone();
        for (id, control) in [(other, "mute"), (bus, "solo")] {
            let current = app.inspect();
            let edited = click_rect(&mut app, &channel(&current, id)[control]);
            assert_eq!(track(&edited, id)[control], true);
            assert_eq!(track(&edited, source)[control], false);
            assert_eq!(edited["selected_track"], selected);
            assert_eq!(undo(&mut app)["project"], baseline["project"]);
        }
        app.quit();
    }
}

#[test]
fn adjacent_mixer_toolbar_actions_create_sends_and_buses_for_the_selected_channel() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    let other = project.add_midi_track("Other");
    let bus = project.add_bus("Return");
    let path = app.directory.path().join("mixer-toolbar-routing.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1100, "height":900}));
    let baseline = app.click("Mixer");
    let send_action = label(&baseline, "Add send…");
    let bus_action = label(&baseline, "Add bus");
    assert!((center(send_action).1 - center(bus_action).1).abs() <= 1.0);
    let gap = (edge(send_action, "min", 0) - edge(bus_action, "max", 0))
        .max(edge(bus_action, "min", 0) - edge(send_action, "max", 0));
    assert!(
        (0.0..=70.0).contains(&gap),
        "Toolbar actions must be adjacent: {gap}"
    );

    // Select the second channel through its normal mouse navigation.
    app.click("Other");
    app.click("Mixer");
    let selected = app.inspect();
    assert_eq!(selected["selected_track"], other);
    unchanged(&selected, &baseline);
    let opened = app.click("Add send…");
    unchanged(&opened, &baseline);
    let sent = app.click("Return");
    assert_eq!(sent["selected_track"], other);
    assert_eq!(track(&sent, source)["sends"], json!([]));
    assert_eq!(
        track(&sent, other)["sends"],
        json!([{
            "target_bus":bus, "gain":0.25, "pre_fader":false
        }])
    );
    let duplicate_choices = app.click("Add send…");
    assert!(
        !duplicate_choices["menu_label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label["text"] == "Return"),
        "Existing sends must be excluded"
    );
    app.key("Escape");
    assert_eq!(undo(&mut app)["project"], baseline["project"]);

    app.click("Add send…");
    let created = app.click("New bus and send");
    assert_eq!(created["selected_track"], other);
    let tracks = created["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 4);
    assert_eq!(tracks[3]["name"], "Bus 1");
    assert_eq!(
        track(&created, other)["sends"][0]["target_bus"],
        tracks[3]["id"]
    );
    assert_eq!(
        undo(&mut app)["project"],
        baseline["project"],
        "Bus plus send is one edit"
    );

    let added = app.click("Add bus");
    assert_eq!(added["project"]["tracks"].as_array().unwrap().len(), 4);
    assert_eq!(added["project"]["tracks"][3]["name"], "Bus 1");
    assert_eq!(undo(&mut app)["project"], baseline["project"]);

    // A folder has no channel signal and must not create an arbitrary send.
    let mut folder_project = Project::new();
    folder_project.add_folder("Folder");
    folder_project.add_bus("Destination");
    let path = app.directory.path().join("mixer-no-send-source.rill");
    folder_project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let folder = app.click("Mixer");
    let clicked = click_rect(&mut app, label(&folder, "Add send…"));
    unchanged(&clicked, &folder);
    assert!(
        !clicked["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "New bus and send")
    );
    app.quit();
}
