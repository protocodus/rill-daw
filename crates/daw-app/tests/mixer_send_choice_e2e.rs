//! An empty channel's Add send command asks for its destination immediately.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

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

fn center(rect: &Value) -> [f64; 2] {
    [
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    ]
}

fn settled(app: &mut Harness) -> Value {
    for _ in 0..12 {
        app.inspect();
    }
    app.inspect()
}

fn unchanged(state: &Value, before: &Value) {
    for key in ["project", "history", "modified", "selected_track", "view"] {
        assert_eq!(
            state[key], before[key],
            "Opening or cancelling changed {key}"
        );
    }
}

fn menu_has(state: &Value, text: &str) -> bool {
    state["menu_label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label["text"] == text)
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn click_position(app: &mut Harness, position: [f64; 2]) -> Value {
    app.command(json!({"command":"pointer", "phase":"click",
        "x":position[0], "y":position[1]}))
}

fn empty_body_position(state: &Value) -> [f64; 2] {
    let viewport = &state["mixer"]["reorder_viewport"];
    let left = state["mixer"]["master_strip"]["max"][0].as_f64().unwrap();
    let right = viewport["max"][0].as_f64().unwrap();
    assert!(right - left > 8.0, "No empty body beside the master strip");
    let position = [(left + right) * 0.5, center(viewport)[1]];
    for area in state["native_menu"]["foreground_areas"].as_array().unwrap() {
        assert!(
            position[0] < area[0].as_f64().unwrap()
                || position[0] > area[2].as_f64().unwrap()
                || position[1] < area[1].as_f64().unwrap()
                || position[1] > area[3].as_f64().unwrap(),
            "Cancellation point {position:?} is inside the menu {area}"
        );
    }
    position
}

fn send_position(app: &mut Harness, source: u64) -> [f64; 2] {
    for _ in 0..12 {
        let state = settled(app);
        let rect = &channel(&state, source)["sends"];
        let viewport = &state["mixer"]["reorder_viewport"];
        if rect["min"][1].as_f64().unwrap() >= viewport["min"][1].as_f64().unwrap()
            && rect["max"][1].as_f64().unwrap() <= viewport["max"][1].as_f64().unwrap()
        {
            return center(rect);
        }
        let [x, y] = center(viewport);
        let delta = if rect["max"][1].as_f64().unwrap() > viewport["max"][1].as_f64().unwrap() {
            -80.0
        } else {
            80.0
        };
        app.command(json!({"command":"scroll", "x":x, "y":y, "delta":delta}));
    }
    panic!("The channel's send control was not reachable by mouse scrolling");
}

fn open_send_choices(app: &mut Harness, source: u64) -> Value {
    let position = send_position(app, source);
    click_position(app, position);
    settled(app)
}

fn open_project(app: &mut Harness, project: &Project, dark: bool, width: f64, height: f64) {
    let path = app.directory.path().join("send-choice.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"system_theme", "dark":dark}));
    app.command(json!({"command":"viewport", "width":width, "height":height,
        "pixels_per_point":if dark {2.0} else {1.0}}));
    app.click("Mixer");
}

fn fixture(app: &mut Harness, dark: bool, width: f64, height: f64) -> (u64, u64, u64) {
    let mut project = Project::new();
    let selected = project.add_audio_track("Selected source");
    let clicked = project.add_midi_track("Clicked source");
    let destination = project.add_bus("Room");
    open_project(app, &project, dark, width, height);
    (selected, clicked, destination)
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

#[test]
fn empty_channel_send_opens_choices_and_mouse_cancellation_changes_nothing() {
    for (dark, width, height) in [(false, 1100.0, 900.0), (true, 800.0, 600.0)] {
        let mut app = Harness::new();
        let (selected, clicked, _) = fixture(&mut app, dark, width, height);
        let position = send_position(&mut app, clicked);
        let before = app.inspect();
        assert_eq!(before["selected_track"], selected);
        click_position(&mut app, position);
        let opened = settled(&mut app);
        assert_eq!(opened["popups"]["routing"], false);
        unchanged(&opened, &before);
        assert!(visible(&opened, "Send from Clicked source"));
        assert!(menu_has(&opened, "Room"));
        assert!(menu_has(&opened, "New bus and send"));
        for choice in ["Room", "New bus and send"] {
            let rect = opened["menu_label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|label| label["text"] == choice)
                .unwrap();
            for axis in 0..2 {
                assert!(rect["min"][axis].as_f64().unwrap() >= 0.0);
                assert!(
                    rect["max"][axis].as_f64().unwrap()
                        <= opened["viewport"][axis].as_f64().unwrap()
                );
            }
        }

        // A second click on the same command dismisses its menu without
        // treating the currently selected track as the intended source.
        click_position(&mut app, position);
        let cancelled = settled(&mut app);
        assert!(!menu_has(&cancelled, "New bus and send"));
        unchanged(&cancelled, &before);

        let reopened = open_send_choices(&mut app, clicked);
        // Dismiss from empty mixer body, below its navigation header and
        // outside the menu, including after scrolling a short viewport.
        click_position(&mut app, empty_body_position(&reopened));
        let outside = settled(&mut app);
        assert!(!menu_has(&outside, "New bus and send"));
        unchanged(&outside, &before);
        app.quit();
    }
}

#[test]
fn channel_destination_choice_targets_the_clicked_source_and_is_one_undo() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let (selected, clicked, destination) = fixture(&mut app, dark, 1100.0, 900.0);
        let before = app.inspect();
        let choices = open_send_choices(&mut app, clicked);
        unchanged(&choices, &before);
        assert_eq!(choices["popups"]["routing"], false);
        let sent = app.click("Room");
        assert_eq!(sent["view"], "Mixer");
        assert_eq!(sent["selected_track"], selected);
        assert_eq!(track(&sent, selected)["sends"], json!([]));
        assert_eq!(
            track(&sent, clicked)["sends"],
            json!([{
                "target_bus":destination, "gain":0.25, "pre_fader":false
            }])
        );
        assert_eq!(sent["project"]["tracks"].as_array().unwrap().len(), 3);
        assert_eq!(undo(&mut app)["project"], before["project"]);
        app.click("Edit");
        assert_eq!(app.click("Redo")["project"], sent["project"]);

        // Once a send exists, its summary retains the established editing
        // route so level, Pre/Post, removal and automation remain available.
        let before_editing = settled(&mut app);
        let position = send_position(&mut app, clicked);
        let routing = click_position(&mut app, position);
        assert_eq!(routing["popups"]["routing"], true);
        assert_eq!(routing["selected_track"], clicked);
        assert!(visible(&routing, "Routing of Clicked source"));
        assert!(visible(&routing, "Pre") && visible(&routing, "Post"));
        assert_eq!(routing["project"], before_editing["project"]);
        assert_eq!(routing["history"], before_editing["history"]);
        app.quit();
    }
}

#[test]
fn explicit_new_bus_choice_is_one_undoable_channel_send() {
    let mut app = Harness::new();
    let (selected, clicked, _) = fixture(&mut app, false, 1100.0, 900.0);
    let before = app.inspect();
    let choices = open_send_choices(&mut app, clicked);
    unchanged(&choices, &before);
    assert_eq!(choices["popups"]["routing"], false);
    let sent = app.click("New bus and send");
    let tracks = sent["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 4);
    let new_bus = &tracks[3];
    assert_eq!(new_bus["kind"], "Bus");
    assert_eq!(new_bus["name"], "Bus 1");
    assert!(new_bus["output_bus"].is_null());
    assert_eq!(sent["selected_track"], selected);
    assert_eq!(track(&sent, selected)["sends"], json!([]));
    assert_eq!(
        track(&sent, clicked)["sends"],
        json!([{
            "target_bus":new_bus["id"], "gain":0.25, "pre_fader":false
        }])
    );
    assert_eq!(undo(&mut app)["project"], before["project"]);
    app.quit();
}

#[test]
fn channel_send_choices_exclude_self_and_all_feedback_paths() {
    for has_destination in [false, true] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_bus("Source bus");
        let output_upstream = project.add_bus("Output upstream");
        project.set_output(output_upstream, Some(source)).unwrap();
        let send_upstream = project.add_bus("Send upstream");
        project.add_send(send_upstream, source, 0.5, true).unwrap();
        let destination = has_destination.then(|| project.add_bus("Safe return"));
        open_project(&mut app, &project, false, 1100.0, 900.0);
        let before = app.inspect();
        let choices = open_send_choices(&mut app, source);
        unchanged(&choices, &before);
        assert_eq!(choices["popups"]["routing"], false);
        for excluded in ["Source bus", "Output upstream", "Send upstream"] {
            assert!(
                !menu_has(&choices, excluded),
                "Unsafe send destination {excluded}"
            );
        }
        assert!(menu_has(&choices, "New bus and send"));
        if let Some(destination) = destination {
            assert!(menu_has(&choices, "Safe return"));
            let sent = app.click("Safe return");
            assert_eq!(track(&sent, source)["sends"][0]["target_bus"], destination);
            assert_eq!(
                track(&sent, output_upstream),
                track(&before, output_upstream)
            );
            assert_eq!(track(&sent, send_upstream), track(&before, send_upstream));
            assert_eq!(undo(&mut app)["project"], before["project"]);
        } else {
            assert!(menu_has(&choices, "No eligible buses"));
            unchanged(&choices, &before);
        }
        app.quit();
    }
}
