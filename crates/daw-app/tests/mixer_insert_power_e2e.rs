//! Mixer insert rows: a lit lamp bypasses the insert in place, and built-in
//! filters, equalizers and dynamics show their static curve in miniature.
mod common;

use common::Harness;
use daw_core::{Effect, EffectSlot, Project};
use serde_json::{Value, json};

fn builtin(name: &str, device: &str) -> EffectSlot {
    EffectSlot::new(
        name,
        Effect::Builtin {
            device: device.into(),
        },
    )
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn device<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["mixer_devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["name"] == name)
        .unwrap_or_else(|| panic!("Missing insert {name}: {}", state["mixer_devices"]))
}

fn click_power(app: &mut Harness, name: &str) -> Value {
    let state = app.inspect();
    let (x, y) = center(&device(&state, name)["power"]);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

#[test]
fn insert_lamps_bypass_in_place_and_builtins_show_their_curves() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1400,"height":1000}));
    let mut project = Project::new();
    let drums = project.add_audio_track("Drums");
    let inserts = &mut project.track_mut(drums).unwrap().inserts;
    inserts.push(builtin("Tone", "r1-equalizer"));
    inserts.push(builtin("Glue", "r1-compressor"));
    inserts.push(EffectSlot::new(
        "Hosted",
        Effect::Vst3 {
            path: "/missing/insert-power.vst3".into(),
            class_id: None,
        },
    ));
    project
        .master
        .inserts
        .push(builtin("Ceiling", "r1-compressor"));
    let path = app.directory.path().join("insert-power.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));

    let mixer = app.click("Mixer");
    assert_eq!(device(&mixer, "Tone")["curve"], "Response");
    assert_eq!(device(&mixer, "Glue")["curve"], "Transfer");
    assert_eq!(device(&mixer, "Hosted")["curve"], Value::Null);
    assert_eq!(device(&mixer, "Ceiling")["curve"], "Transfer");
    for name in ["Tone", "Glue", "Hosted", "Ceiling"] {
        let row = device(&mixer, name);
        assert!(
            row["power"]["max"][0].as_f64().unwrap() <= row["rect"]["min"][0].as_f64().unwrap(),
            "The lamp sits before the row that opens the device: {row}"
        );
    }
    assert!(
        mixer["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "3/3")
    );
    let undo_before = mixer["history"]["undo"].as_array().unwrap().len();

    // The lamp bypasses only its own insert, without opening the device.
    let bypassed = click_power(&mut app, "Tone");
    let inserts = &bypassed["project"]["tracks"][0]["inserts"];
    assert_eq!(inserts[0]["enabled"], false);
    assert_eq!(inserts[1]["enabled"], true);
    assert_eq!(inserts[2]["enabled"], true);
    assert_eq!(
        bypassed["history"]["undo"].as_array().unwrap().len(),
        undo_before + 1
    );
    assert!(
        bypassed["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Effects"),
        "Bypassing must leave the mixer in place"
    );
    let counted = app.inspect();
    assert!(
        counted["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "2/3"),
        "The Effects count follows the lamp: {}",
        counted["visible_labels"]
    );

    let master = click_power(&mut app, "Ceiling");
    assert_eq!(master["project"]["master"]["inserts"][0]["enabled"], false);

    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"]["master"]["inserts"][0]["enabled"], true);
    assert_eq!(
        undone["project"]["tracks"][0]["inserts"][0]["enabled"],
        false
    );
    app.click("Edit");
    let restored = app.click("Undo");
    assert_eq!(
        restored["project"]["tracks"][0]["inserts"][0]["enabled"],
        true
    );

    // A bypassed insert is turned back on by the same lamp.
    click_power(&mut app, "Glue");
    let on = click_power(&mut app, "Glue");
    assert_eq!(on["project"]["tracks"][0]["inserts"][1]["enabled"], true);
    app.quit();
}
