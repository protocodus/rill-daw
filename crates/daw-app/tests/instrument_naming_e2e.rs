//! Default instrument identity follows an explicit creation flow in the real app.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

fn tracks(state: &Value) -> &[Value] {
    state["project"]["tracks"].as_array().unwrap()
}

fn assign(app: &mut Harness, name: &str) -> Value {
    common::wait_library_device(app, name);
    app.click(name);
    app.click("Assign instrument")
}

#[test]
fn initial_assignment_and_explicit_add_use_unique_names_with_atomic_undo() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let created = menu(&mut app, "Track", "Add MIDI track");
    assert_eq!(tracks(&created)[0]["name"], "New instrument");
    let assigned = assign(&mut app, "Rill Fixture A");
    assert_eq!(tracks(&assigned)[0]["name"], "Rill Fixture A");
    assert_eq!(tracks(&assigned)[0]["clips"], tracks(&created)[0]["clips"]);
    assert_eq!(
        assigned["history"]["undo"].as_array().unwrap().len(),
        created["history"]["undo"].as_array().unwrap().len() + 1
    );
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        created["project"]
    );
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        assigned["project"]
    );

    menu(&mut app, "Track", "Add MIDI track");
    let second = assign(&mut app, "Rill Fixture A");
    assert_eq!(tracks(&second)[1]["name"], "Rill Fixture A 2");

    // The selected track must still count when explicitly adding another one.
    menu(&mut app, "Track", "Add instrument…");
    common::wait_library_device(&mut app, "Rill Fixture A");
    app.click("Rill Fixture A");
    let third = app.click("Add instrument track");
    assert_eq!(tracks(&third)[2]["name"], "Rill Fixture A 3");
    app.key("Escape");
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], second["project"]);
    assert_eq!(menu(&mut app, "Edit", "Redo")["project"], third["project"]);
    app.quit();
}

#[test]
fn blank_arranger_creation_names_its_initial_instrument_assignment() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let mut project = Project::new();
    project.add_bus("Reference");
    let path = app.directory.path().join("blank-arranger.rill");
    project.save(&path).unwrap();
    let initial = app.command(json!({"command":"open", "path":path}));
    let canvas = &initial["timeline"]["canvas"];
    let x = canvas["min"][0].as_f64().unwrap() + 100.0;
    let y = canvas["max"][1].as_f64().unwrap() - 24.0;
    let created = app.command(json!({
        "command":"pointer", "phase":"double_click", "x":x, "y":y
    }));
    assert_eq!(tracks(&created).len(), 2);
    assert_eq!(tracks(&created)[1]["name"], "New instrument");
    assert_eq!(created["popups"]["library"], true);
    let assigned = assign(&mut app, "Rill Fixture A");
    assert_eq!(tracks(&assigned)[1]["name"], "Rill Fixture A");
    assert_eq!(tracks(&assigned)[1]["clips"], tracks(&created)[1]["clips"]);
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        created["project"]
    );
    assert_eq!(
        menu(&mut app, "Edit", "Undo")["project"],
        initial["project"]
    );
    app.quit();
}

#[test]
fn a_mouse_renamed_track_keeps_its_name_through_assignment_save_and_replacement() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Track", "Add MIDI track");
    app.key("Escape");
    menu(&mut app, "Window", "Mixer");
    menu(&mut app, "Edit", "Rename…");
    app.click("Keyboard");
    for letter in ["l", "e", "a", "d"] {
        app.click(letter);
    }
    let renamed = app.click("Apply name");
    assert_eq!(tracks(&renamed)[0]["name"], "lead");
    menu(&mut app, "Window", "Instrument");
    app.click("Choose instrument");
    let assigned = assign(&mut app, "Rill Fixture A");
    assert_eq!(tracks(&assigned)[0]["name"], "lead");

    let path = app.directory.path().join("named-instrument.rill");
    app.command(json!({"command":"save", "path":path}));
    app = app.restart();
    let loaded = app.command(json!({"command":"open", "path":path}));
    assert_eq!(tracks(&loaded)[0]["name"], "lead");
    app.click("lead");
    menu(&mut app, "Window", "Instrument");
    app.click("Change instrument");
    let replaced = assign(&mut app, "Rill Fixture B");
    assert_eq!(tracks(&replaced)[0]["name"], "lead");
    assert_eq!(
        tracks(&replaced)[0]["instrument"]["Vst3"]["class_id"],
        "22222222222222222222222222222222"
    );
    app.key("Escape");
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], loaded["project"]);
    app.quit();
}

#[test]
fn a_loaded_default_looking_name_is_preserved_when_assigning_an_instrument() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let mut project = Project::new();
    project.add_midi_track("New instrument");
    let path = app.directory.path().join("user-named-instrument.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("New instrument");
    menu(&mut app, "Window", "Instrument");
    app.click("Choose instrument");
    let assigned = assign(&mut app, "Rill Fixture A");
    assert_eq!(tracks(&assigned)[0]["name"], "New instrument");
    assert_eq!(
        tracks(&assigned)[0]["instrument"]["Vst3"]["class_id"],
        "11111111111111111111111111111111"
    );
    app.quit();
}
