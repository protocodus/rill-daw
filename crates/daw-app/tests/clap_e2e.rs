//! A native CLAP instrument and effect through the running app: discovery,
//! library badges, insertion, audible export, save/reopen and undo.
#[path = "../../daw-plugins/tests/common/clap_probe_fixture.rs"]
mod clap_probe_fixture;
mod common;

use clap_probe_fixture::{ClapProbeFixture, GAIN_ID, SYNTH_ID};
use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

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
    assert_eq!(card["format"], "CLAP", "{card}");
    app.command(json!({"command":"click","label":name,"occurrence":1}))
}

fn export(app: &mut Harness, name: &str) -> (f64, usize) {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    let audio = daw_core::AudioData::load_wav(path).unwrap();
    assert!(audio.samples.iter().all(|x| x.is_finite()));
    let peak = audio
        .samples
        .iter()
        .fold(0.0_f64, |peak, x| peak.max(f64::from(x.abs())));
    let first = audio.samples.iter().position(|x| x.abs() > 1e-4);
    (peak, first.unwrap_or(usize::MAX) / 2)
}

#[test]
fn clap_instrument_and_effect_scan_insert_render_save_reopen_and_undo() {
    let fixture = ClapProbeFixture::build();
    let mut app = Harness::with_vst_path(fixture.path.parent().unwrap());
    let mut project = Project::new();
    let track = project.add_midi_track("CLAP synth");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Phrase",
        1.0,
        2.0,
        vec![Note::new(69, 0.0, 1.0, 127)],
    ));
    let source = app.directory.path().join("clap-source.rill");
    project.save(&source).unwrap();
    app.command(json!({"command":"open","path":source}));
    let (silent, _) = export(&mut app, "unassigned.wav");
    assert_eq!(silent, 0.0);

    menu(&mut app, "Window", "Instrument");
    app.click("Choose instrument");
    choose(&mut app, "Search instruments", "Rill CLAP Synth");
    let assigned = app.click("Assign instrument");
    let instrument = &assigned["project"]["tracks"][0]["instrument"]["Vst3"];
    assert_eq!(instrument["class_id"], SYNTH_ID, "{assigned}");
    assert!(
        instrument["path"]
            .as_str()
            .unwrap()
            .ends_with("RillProbe.clap")
    );
    let (dry, dry_start) = export(&mut app, "synth.wav");
    assert!(dry > 0.05, "CLAP synth must be audible: {dry}");

    menu(&mut app, "Window", "Effects");
    app.click("+ Insert");
    choose(&mut app, "Search effects", "Rill CLAP Gain");
    let inserted = app.click("Add to effect chain");
    let effect = &inserted["project"]["tracks"][0]["inserts"][0]["effect"]["Vst3"];
    assert_eq!(effect["class_id"], GAIN_ID, "{inserted}");
    let (wet, wet_start) = export(&mut app, "gain.wav");
    // Unity gain; the 32-sample plug-in latency is compensated on export.
    assert!((wet - dry).abs() < 1e-3, "{wet} vs {dry}");
    assert_eq!(wet_start, dry_start);

    let undone = menu(&mut app, "Edit", "Undo");
    assert!(
        undone["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{undone}"
    );
    let redone = menu(&mut app, "Edit", "Redo");
    assert_eq!(redone["project"]["tracks"], inserted["project"]["tracks"]);

    let saved = app.directory.path().join("clap-session.rill");
    app.command(json!({"command":"save","path":saved}));
    let project = inserted["project"].clone();
    let reopened = app.command(json!({"command":"open","path":saved}));
    assert_eq!(reopened["project"]["tracks"], project["tracks"]);
    let (recalled, _) = export(&mut app, "reopened.wav");
    assert!((recalled - wet).abs() < 1e-6);

    app.quit();
}

fn song_tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"song_tool","name":name,"arguments":arguments}))
}

#[test]
fn plugin_tools_report_filter_and_check_the_clap_format() {
    let fixture = ClapProbeFixture::build();
    let mut app = Harness::with_vst_path(fixture.path.parent().unwrap());
    let listing = loop {
        let listing = song_tool(&mut app, "list_plugins", json!({"format":"clap"}));
        let result = &listing["tool_result"]["result"];
        if result["plugins"].as_array().unwrap().len() == 2 {
            break result.clone();
        }
        assert_eq!(result["scanning"], true, "{listing}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let plugins = listing["plugins"].as_array().unwrap();
    assert!(plugins.iter().all(|plugin| plugin["format"] == "CLAP"));
    let none = song_tool(&mut app, "list_plugins", json!({"format":"VST3"}));
    assert_eq!(none["tool_result"]["result"]["plugins"], json!([]));
    let synth = plugins
        .iter()
        .find(|plugin| plugin["name"] == "Rill CLAP Synth")
        .unwrap()["id"]
        .clone();
    menu(&mut app, "Track", "Add MIDI track");
    let state = app.inspect();
    let request = |format: &str| {
        json!({"session_id":state["song_session"]["id"],
            "expected_revision":state["song_session"]["revision"],
            "track_id":state["project"]["tracks"][0]["id"],"plugin_id":synth,"format":format})
    };
    let wrong = song_tool(&mut app, "assign_plugin", request("VST3"));
    assert_eq!(wrong["tool_result"]["error"]["code"], "invalid_arguments");
    assert_eq!(wrong["project"], state["project"]);
    let assigned = song_tool(&mut app, "assign_plugin", request("CLAP"));
    assert_eq!(
        assigned["tool_result"]["result"]["format"], "CLAP",
        "{assigned}"
    );
    assert_eq!(
        assigned["project"]["tracks"][0]["instrument"]["Vst3"]["class_id"],
        SYNTH_ID
    );
    app.quit();
}
