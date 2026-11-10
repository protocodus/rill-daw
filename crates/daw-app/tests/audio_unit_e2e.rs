//! Apple's system Audio Units through the running app: discovery, library
//! badges, insertion, audible export, save/reopen and undo. Every Mac ships
//! these components, so the suite needs no fixture.
#![cfg(target_os = "macos")]
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use daw_plugins::audio_unit::AU_LOCATOR;
use serde_json::{Value, json};
use std::path::Path;

const DELAY: &str = "aufx:dely:appl";
const DLS: &str = "aumu:dls :appl";

fn menu(app: &mut Harness, name: &str, item: &str) -> Value {
    app.click(name);
    app.click(item)
}

fn choose(app: &mut Harness, search: &str, name: &str) -> Value {
    common::wait_visible(app, search);
    app.click(search);
    app.command(json!({"command":"text","text":name}));
    common::wait_library_device(app, name);
    let state = app.inspect();
    let card = state["library"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["name"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["library"]))
        .clone();
    assert_eq!(card["format"], "AU", "{card}");
    app.command(json!({"command":"click","label":name,"occurrence":1}))
}

fn export(app: &mut Harness, name: &str) -> Vec<f32> {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    let audio = daw_core::AudioData::load_wav(path).unwrap();
    assert!(audio.samples.iter().all(|x| x.is_finite()));
    audio.samples
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |peak, x| peak.max(x.abs()))
}

fn difference(a: &[f32], b: &[f32]) -> f32 {
    // A longer export (the delay's tail) counts as different too.
    let shared: f32 = a.iter().zip(b).map(|(a, b)| (a - b).abs()).sum();
    shared + a.len().abs_diff(b.len()) as f32
}

#[test]
fn apple_audio_units_scan_insert_render_save_reopen_and_undo() {
    let mut app = Harness::with_vst_path(Path::new(AU_LOCATOR));
    let mut project = Project::new();
    let track = project.add_midi_track("AU synth");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Phrase",
        1.0,
        2.0,
        vec![Note::new(60, 0.0, 1.0, 127)],
    ));
    let source = app.directory.path().join("au-source.rill");
    project.save(&source).unwrap();
    app.command(json!({"command":"open","path":source}));

    menu(&mut app, "Window", "Instrument");
    app.click("Choose instrument");
    choose(&mut app, "Search instruments", "DLSMusicDevice");
    let assigned = app.click("Assign instrument");
    let instrument = &assigned["project"]["tracks"][0]["instrument"]["Vst3"];
    assert_eq!(instrument["class_id"], DLS, "{assigned}");
    assert_eq!(instrument["path"], AU_LOCATOR);
    let dry = export(&mut app, "synth.wav");
    assert!(
        peak(&dry) > 0.01,
        "DLS synth must be audible: {}",
        peak(&dry)
    );

    menu(&mut app, "Window", "Effects");
    app.click("+ Insert");
    choose(&mut app, "Search effects", "AUDelay");
    let inserted = app.click("Add to effect chain");
    let effect = &inserted["project"]["tracks"][0]["inserts"][0]["effect"]["Vst3"];
    assert_eq!(effect["class_id"], DELAY, "{inserted}");
    let wet = export(&mut app, "delay.wav");
    assert!(
        difference(&wet, &dry) > 1.0,
        "the delay must change the export"
    );

    let undone = menu(&mut app, "Edit", "Undo");
    assert!(
        undone["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{undone}"
    );
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(
        redone["project"]["tracks"][0]["inserts"][0]["effect"],
        inserted["project"]["tracks"][0]["inserts"][0]["effect"]
    );

    let saved = app.directory.path().join("au-session.rill");
    app.command(json!({"command":"save","path":saved}));
    let before = app.inspect()["project"]["tracks"].clone();
    let reopened = app.command(json!({"command":"open","path":saved}));
    assert_eq!(reopened["project"]["tracks"], before);
    let recalled = export(&mut app, "reopened.wav");
    assert!((peak(&recalled) - peak(&wet)).abs() < 1e-3);

    let listing = loop {
        let listing = app.command(json!({"command":"song_tool","name":"list_plugins",
            "arguments":{"format":"AU"}}));
        let result = listing["tool_result"]["result"].clone();
        if result["scanning"] != true {
            break result;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let plugins = listing["plugins"].as_array().unwrap();
    assert!(plugins.iter().any(|plugin| plugin["name"] == "AUDelay"));
    assert!(plugins.iter().all(|plugin| plugin["format"] == "AU"));
    app.quit();
}
