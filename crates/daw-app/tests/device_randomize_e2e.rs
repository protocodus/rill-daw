//! A device randomization is one complete, independent Undo transaction.
mod common;
use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn parameters(state: &Value, track: u64) -> Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == track)
        .unwrap()["instrument_parameters"]
        .clone()
}

#[test]
fn randomize_clicks_are_independent_whole_device_undo_steps() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_instrument_track("Randomize test");
    let path = app.directory.path().join("randomize.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1600,"height":1400}));
    app.click("Randomize test");
    app.click("Window");
    app.click("Instrument");
    common::wait_visible(&mut app, "RANDOMIZE");
    let before = app.inspect();
    let first = app.click("100%");
    let second = app.click("100%");
    assert_ne!(parameters(&first, track), parameters(&before, track));
    assert_ne!(parameters(&second, track), parameters(&first, track));
    let initial = before["history"]["undo"].as_array().unwrap().len();
    assert_eq!(
        first["history"]["undo"].as_array().unwrap().len(),
        initial + 1
    );
    assert_eq!(
        second["history"]["undo"].as_array().unwrap().len(),
        initial + 2
    );
    assert_eq!(second["history"]["undo"][0], "Randomize device");
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(
        parameters(&undone, track),
        parameters(&first, track),
        "One Undo restores every value from the preceding randomization"
    );
    let original = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(parameters(&original, track), parameters(&before, track));
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(parameters(&redone, track), parameters(&first, track));
    app.quit();
}
