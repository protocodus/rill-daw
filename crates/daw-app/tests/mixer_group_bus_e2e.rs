//! Group selected channel outputs through the running mixer, using only mouse controls.
mod common;

use common::{Harness, wait_until};
use daw_core::Project;
use serde_json::{Value, json};

fn track(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap()
}

fn select(app: &mut Harness, id: u64, action: &str) -> Value {
    let state = app.inspect();
    let channel = state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap();
    let rect = &channel["name"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0;
    app.command(json!({"command":"pointer", "phase":"click", "button":"secondary", "x":x,"y":y}));
    app.click(action)
}

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

#[test]
fn selected_channels_create_a_group_bus_preserving_their_shared_downstream_and_sends() {
    for (dark, width) in [(false, 800), (true, 1400)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let a = project.add_audio_track("Kick");
        let b = project.add_midi_track("Snare");
        let downstream = project.add_bus("Bus 1");
        let room = project.add_bus("Room");
        project.set_output(a, Some(downstream)).unwrap();
        project.set_output(b, Some(downstream)).unwrap();
        project.add_send(a, room, 0.125, true).unwrap();
        project.add_send(b, room, 0.35, false).unwrap();
        project.track_mut(a).unwrap().gain = 0.7;
        project.track_mut(downstream).unwrap().gain = 0.6;
        let path = app.directory.path().join("groups.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"viewport","width":width,"height":900}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.click("Mixer");
        select(&mut app, a, "Select channel");
        let before = select(&mut app, b, "Add channel to selection");
        let opened = app.click("Route selected channels");
        assert_eq!(opened["project"], before["project"]);
        assert_eq!(opened["history"], before["history"]);
        let cancelled = app.key("Escape");
        assert_eq!(cancelled["project"], before["project"]);
        let grouped = menu(&mut app, "Route selected channels", "New bus and route");
        let created = grouped["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["name"] == "Bus 2")
            .unwrap();
        let bus = created["id"].as_u64().unwrap();
        assert_eq!(
            created["output_bus"], downstream,
            "Keep existing downstream processing"
        );
        assert_eq!(created["gain"], 1.0);
        for id in [a, b] {
            assert_eq!(track(&grouped, id)["output_bus"], bus);
            for field in ["gain", "pan", "sends", "inserts", "clips"] {
                assert_eq!(track(&grouped, id)[field], track(&before, id)[field]);
            }
        }
        assert_eq!(grouped["selected_track"], before["selected_track"]);
        assert_eq!(
            grouped["finishing"]["selected_channels"],
            before["finishing"]["selected_channels"]
        );
        assert_eq!(grouped["view"], "Mixer");
        assert_eq!(grouped["project"]["tracks"].as_array().unwrap().len(), 5);
        let undone = menu(&mut app, "Edit", "Undo");
        assert_eq!(
            undone["project"], before["project"],
            "Creation and all routes are one Undo"
        );
        let redone = menu(&mut app, "Edit", "Redo");
        assert_eq!(redone["project"], grouped["project"]);
        app.quit();
    }
}

#[test]
fn selected_channels_with_different_outputs_create_a_group_routed_to_master() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let a = project.add_audio_track("Dry");
    let b = project.add_audio_track("Processed");
    let downstream = project.add_bus("Music");
    project.set_output(b, Some(downstream)).unwrap();
    let path = app.directory.path().join("different-outputs.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Mixer");
    select(&mut app, a, "Select channel");
    let before = select(&mut app, b, "Add channel to selection");
    let grouped = menu(&mut app, "Route selected channels", "New bus and route");
    let bus = grouped["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["name"] == "Bus 1")
        .unwrap();
    assert!(bus["output_bus"].is_null());
    for id in [a, b] {
        assert_eq!(track(&grouped, id)["output_bus"], bus["id"]);
    }
    assert_eq!(track(&grouped, downstream), track(&before, downstream));
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    app.quit();
}

#[test]
fn adding_a_bus_reveals_its_selected_strip_in_a_long_mixer() {
    let mut app = Harness::new();
    let mut project = Project::new();
    for index in 0..16 {
        project.add_audio_track(format!("Source {}", index + 1));
    }
    let path = app.directory.path().join("long-mixer.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":800,"height":900}));
    let before = app.click("Mixer");
    let added = app.click("Add bus");
    let bus = added["selected_track"].as_u64().unwrap();
    assert_eq!(track(&added, bus)["name"], "Bus 1");
    let revealed = wait_until(&mut app, |state| {
        let channel = state["mixer"]["channels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|channel| channel["track"] == bus)
            .unwrap();
        let name = &channel["name"];
        let viewport = &state["mixer"]["reorder_viewport"];
        (0..2).all(|axis| {
            name["min"][axis].as_f64().unwrap() >= viewport["min"][axis].as_f64().unwrap()
                && name["max"][axis].as_f64().unwrap() <= viewport["max"][axis].as_f64().unwrap()
        })
    });
    assert_eq!(revealed["selected_track"], bus);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    app.quit();
}
