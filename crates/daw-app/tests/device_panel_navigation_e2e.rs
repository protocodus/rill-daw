//! Device panels stay floating; their choices open; the arranger opens an
//! instrument in one click; other utilities are pages with Back.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Effect, EffectSlot, Instrument, Note, Project};
use serde_json::{Value, json};

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}

fn filter(id: u64) -> EffectSlot {
    EffectSlot {
        id,
        name: "R1 Filter".into(),
        enabled: true,
        effect: Effect::Builtin {
            device: "r1-filter".into(),
        },
        state: Vec::new(),
        parameters: Vec::new(),
        inputs: Vec::new(),
    }
}

fn open_project(app: &mut Harness) -> u64 {
    let source = app.directory.path().join("silence.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0.0; 9_600],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let synth = project.add_midi_track("Synth");
    let track = project.track_mut(synth).unwrap();
    track.instrument = Instrument::Builtin {
        device: "r1-mini".into(),
    };
    track.clips.push(Clip::midi(
        "Pattern",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let drums = project.add_audio_track("Drums");
    let track = project.track_mut(drums).unwrap();
    track
        .clips
        .push(Clip::audio("Drum loop", 0.0, 2.0, source.to_string_lossy()));
    track.inserts.push(filter(900));
    let path = app.directory.path().join("devices.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1400,"height":1000,"pixels_per_point":1.0}));
    synth
}

#[test]
fn automatable_choices_open_and_change_their_parameter() {
    let mut app = Harness::new();
    open_project(&mut app);
    app.click("Drum loop");
    app.click("Window");
    app.click("Effects");
    app.click("Open editor");
    common::wait_visible(&mut app, "Cutoff");
    let before = app.inspect();
    let opened = app.click("Clean");
    assert_eq!(
        opened["native_menu"]["open"], true,
        "the model list stays open"
    );
    let chosen = app.click("Sallen-Key");
    let model = chosen["project"]["tracks"][1]["inserts"][0]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["id"] == 1)
        .cloned()
        .unwrap_or_else(|| panic!("Model unchanged: {}", chosen["project"]));
    assert_eq!(model["value"], 0.25);
    assert_eq!(chosen["history"]["undo"][0], "Effect parameter");
    assert_ne!(chosen["project"], before["project"]);
    app.quit();
}

#[test]
fn the_arranger_opens_an_assigned_instrument_in_one_click() {
    let mut app = Harness::new();
    let synth = open_project(&mut app);
    let state = app.set_track_density("Detailed");
    let row = state["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["track"] == synth)
        .unwrap()
        .clone();
    let (x, y) = center(&row["sections"]["device"]);
    let opened = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(opened["popups"]["instrument"], true);
    assert_eq!(opened["popups"]["library"], false);
    assert_eq!(opened["selected_track"], synth);
    assert_eq!(opened["history"], state["history"]);
    common::wait_visible(&mut app, "Osc 1");
    // Changing the instrument stays one right-click away.
    app.key("Escape");
    app.command(json!({"command":"pointer","phase":"click","button":"secondary","x":x,"y":y}));
    let chooser = app.click("Change instrument…");
    assert_eq!(chooser["popups"]["library"], true);
    app.quit();
}

#[test]
fn effects_and_instruments_open_in_one_click_from_the_arranger_and_the_mixer() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let empty = project.add_midi_track("Empty");
    let single = project.add_audio_track("Single");
    project.track_mut(single).unwrap().inserts.push(filter(900));
    let chain = project.add_bus("Chain");
    project
        .track_mut(chain)
        .unwrap()
        .inserts
        .extend([filter(901), filter(902)]);
    let path = app.directory.path().join("one-click.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let state = app
        .command(json!({"command":"viewport","width":1400,"height":1000,"pixels_per_point":1.0}));
    let effects = |track: u64| {
        let row = state["timeline"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["track"] == track)
            .unwrap();
        // The slot shares the identity line with the name at every density.
        assert!(
            row["sections"]["effects"]["min"][0].as_f64().unwrap()
                >= row["name"]["max"][0].as_f64().unwrap(),
            "{row}"
        );
        center(&row["sections"]["effects"])
    };

    // The only insert opens its editor directly.
    let (x, y) = effects(single);
    let editor = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(editor["popups"]["instrument"], true);
    assert_eq!(editor["selected_track"], single);
    assert_eq!(editor["history"], state["history"]);
    common::wait_visible(&mut app, "Cutoff");
    app.key("Escape");

    // Several inserts open the rack to choose from.
    let (x, y) = effects(chain);
    let rack = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(rack["popups"]["effects"], true);
    assert_eq!(rack["selected_track"], chain);
    app.key("Escape");

    // An empty chain goes straight to the device browser.
    let (x, y) = effects(empty);
    let browser = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    assert_eq!(browser["popups"]["library"], true);
    assert_eq!(browser["project"], state["project"]);
    app.click("Back");

    // In the mixer, a MIDI channel without an instrument offers the chooser
    // where the instrument name would be.
    app.click("Mixer");
    let chooser = app.click("MIDI · Choose…");
    assert_eq!(chooser["popups"]["library"], true);
    assert_eq!(chooser["selected_track"], empty);
    app.quit();
}

#[test]
fn full_screens_have_back_and_action_dialogs_float_above_the_blurred_workspace() {
    let mut app = Harness::new();
    open_project(&mut app);
    app.click("Pattern");
    // The device browser is a full screen: it fills the workspace below the
    // transport, and Back returns to the caller.
    app.click("Window");
    let browser = app.click("Device browser");
    assert_eq!(browser["popups"]["library"], true);
    let page = &browser["window_rects"][0];
    let viewport = &browser["viewport"];
    assert_eq!(page["max"][0], viewport[0]);
    assert!(page["min"][0].as_f64().unwrap() <= 1.0, "{page}");
    assert!(browser["backdrop_blur"].as_array().unwrap().is_empty());
    let back = app.click("Back");
    assert_eq!(back["popups"]["library"], false);
    assert_eq!(back["view"], "Arrangement");
    assert_eq!(back["selected_clip"], browser["selected_clip"]);

    // Properties is an action dialog: a bounded window over one blur, with
    // no Back of its own, closed by its close button.
    app.click("Window");
    let properties = app.click("Properties");
    assert_eq!(properties["popups"]["properties"], true);
    let dialog = properties["window_rects"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert!(dialog["min"][0].as_f64().unwrap() > 100.0, "{dialog}");
    assert!(
        dialog["max"][0].as_f64().unwrap() < viewport[0].as_f64().unwrap() - 100.0,
        "{dialog}"
    );
    assert_eq!(properties["backdrop_blur"].as_array().unwrap().len(), 1);
    assert!(
        !properties["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label["text"] == "Back")
    );
    let closed = common::close_utility(&mut app, &properties, "Properties · Pattern");
    assert_eq!(closed["popups"]["properties"], false);
    assert_eq!(closed["view"], "Arrangement");
    assert_eq!(closed["selected_clip"], properties["selected_clip"]);
    app.quit();
}
