//! A native LV2 instrument and effect through the running app (Linux): discovery,
//! format badges, insertion, the generic parameter browser, audible export with
//! latency compensation, undo/redo and save/reopen.
#![cfg(target_os = "linux")]
mod common;
#[path = "../../daw-plugins/tests/common/lv2_probe_fixture.rs"]
mod lv2_probe_fixture;

use common::Harness;
use daw_core::{Clip, Note, Project};
use lv2_probe_fixture::{GAIN_URI, Lv2ProbeFixture, SYNTH_URI};
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
    assert_eq!(card["format"], "LV2", "{card}");
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
fn lv2_instrument_and_effect_scan_insert_render_save_reopen_and_undo() {
    let fixture = Lv2ProbeFixture::build();
    let mut app = Harness::with_vst_path(fixture.path.parent().unwrap());
    let mut project = Project::new();
    let track = project.add_midi_track("LV2 synth");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Phrase",
        1.0,
        2.0,
        vec![Note::new(69, 0.0, 1.0, 127)],
    ));
    let source = app.directory.path().join("lv2-source.rill");
    project.save(&source).unwrap();
    app.command(json!({"command":"open","path":source}));

    menu(&mut app, "Window", "Instrument");
    app.click("Choose instrument");
    choose(&mut app, "Search instruments", "Rill LV2 Synth");
    let assigned = app.click("Assign instrument");
    let instrument = &assigned["project"]["tracks"][0]["instrument"]["Vst3"];
    assert_eq!(instrument["class_id"], SYNTH_URI, "{assigned}");
    assert!(
        instrument["path"]
            .as_str()
            .unwrap()
            .ends_with("RillProbe.lv2")
    );
    let (dry, dry_start) = export(&mut app, "synth.wav");
    assert!(dry > 0.05, "LV2 synth must be audible: {dry}");

    // LV2 has no embedded native editor here; its controls use the generic browser.
    menu(&mut app, "Window", "Instrument");
    common::wait_visible(&mut app, "Parameters…");
    app.click("Parameters…");
    let browser = common::wait_until(&mut app, |state| {
        state["native_editor"]["parameter_count"] == 2
    });
    assert!(
        browser["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Level"),
        "{}",
        browser["visible_labels"]
    );
    app.key("Escape");

    menu(&mut app, "Window", "Effects");
    app.click("+ Insert");
    choose(&mut app, "Search effects", "Rill LV2 Gain");
    let inserted = app.click("Add to effect chain");
    let effect = &inserted["project"]["tracks"][0]["inserts"][0]["effect"]["Vst3"];
    assert_eq!(effect["class_id"], GAIN_URI, "{inserted}");
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

    let saved = app.directory.path().join("lv2-session.rill");
    app.command(json!({"command":"save","path":saved}));
    let reopened = app.command(json!({"command":"open","path":saved}));
    assert_eq!(reopened["project"]["tracks"], inserted["project"]["tracks"]);
    let (recalled, _) = export(&mut app, "reopened.wav");
    assert!((recalled - wet).abs() < 1e-6);
    app.quit();
}
