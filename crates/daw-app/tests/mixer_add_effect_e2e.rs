//! Every mixer strip ends its Effects section with one add row: "Insert…"
//! opens the effect library, "EQ" and "Comp" append the built-in equalizer and
//! compressor in one Undo step and open their editor.
mod common;

use common::Harness;
use daw_core::{MASTER_CHANNEL_ID, Project};
use serde_json::{Value, json};

fn click(app: &mut Harness, rect: &Value) -> Value {
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

fn inserts(state: &Value, track: u64) -> Vec<Value> {
    let track = if track == MASTER_CHANNEL_ID {
        &state["project"]["master"]
    } else {
        state["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == track)
            .unwrap()
    };
    track["inserts"].as_array().unwrap().clone()
}

fn strip(state: &Value, track: u64) -> Value {
    if track == MASTER_CHANNEL_ID {
        return state["mixer"]["master"].clone();
    }
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == track)
        .unwrap()
        .clone()
}

fn undo(app: &mut Harness) -> Value {
    app.click("Edit");
    app.click("Undo")
}

#[test]
fn add_row_offers_insert_eq_and_comp_on_every_strip() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let vocal = project.add_audio_track("Vocal");
        let path = app.directory.path().join("add-effect.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(json!({"command":"viewport","width":1280,"height":800,
            "pixels_per_point":density}));
        let baseline = app.click("Mixer");
        let state = app.command(json!({"command":"inspect","paint":true}));
        for track in [vocal, MASTER_CHANNEL_ID] {
            let strip = strip(&state, track);
            let row = strip["add_effect"].as_array().unwrap();
            assert_eq!(row.len(), 3);
            // One row under Effects, the three targets side by side, edge to edge.
            assert!(
                row[0]["min"][1].as_f64().unwrap() >= strip["effects"]["max"][1].as_f64().unwrap()
            );
            for pair in row.windows(2) {
                assert_eq!(pair[0]["min"][1], pair[1]["min"][1]);
                assert!(
                    (pair[0]["max"][0].as_f64().unwrap() - pair[1]["min"][0].as_f64().unwrap())
                        .abs()
                        < 0.01
                );
            }
            for (label, cell) in ["Insert…", "EQ", "Comp"].iter().zip(row) {
                assert!(
                    state["label_rects"].as_array().unwrap().iter().any(|text| {
                        text["text"] == *label
                            && text["min"][0].as_f64().unwrap()
                                >= cell["min"][0].as_f64().unwrap() - 0.5
                            && text["max"][0].as_f64().unwrap()
                                <= cell["max"][0].as_f64().unwrap() + 0.5
                    }),
                    "{label} is painted inside its own target"
                );
            }
        }

        for (track, cell, device, name) in [
            (vocal, 1, "r1-equalizer", "R1 Equalizer"),
            (vocal, 2, "r1-compressor", "R1 Compressor"),
            (MASTER_CHANNEL_ID, 1, "r1-equalizer", "R1 Equalizer"),
        ] {
            let before = app.click("Mixer");
            let count = inserts(&before, track).len();
            let added = click(&mut app, &strip(&before, track)["add_effect"][cell]);
            let chain = inserts(&added, track);
            assert_eq!(chain.len(), count + 1, "{name} appended");
            let slot = chain.last().unwrap();
            assert_eq!(slot["effect"]["Builtin"]["device"], device);
            assert_eq!(slot["name"], name);
            // Its editor opens on the new insert.
            assert_eq!(added["popups"]["instrument"], true);
            assert_eq!(added["native_editor"]["effect"], slot["id"]);
            assert_eq!(
                added["history"]["undo"].as_array().unwrap().len(),
                before["history"]["undo"].as_array().unwrap().len() + 1,
                "one Undo step"
            );
            app.key("Escape");
        }
        // Undo removes the three additions one at a time.
        for _ in 0..3 {
            undo(&mut app);
        }
        let restored = app.click("Mixer");
        assert_eq!(restored["project"], baseline["project"]);

        let library = click(&mut app, &strip(&restored, vocal)["add_effect"][0]);
        assert_eq!(library["popups"]["library"], true);
        assert_eq!(library["project"], baseline["project"]);
        app.key("Escape");
        app.quit();
    }
}
