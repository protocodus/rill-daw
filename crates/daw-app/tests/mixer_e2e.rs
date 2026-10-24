//! Arranger controls exercise the production UI and the exported audio graph.
mod common;

use common::Harness;
use daw_core::{AudioData, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn drag(app: &mut Harness, label: &str, distance: f64) -> Value {
    let state = app.inspect();
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {state}"));
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","x":x,"y":y,"phase":"drag","to_x":x+distance,"to_y":y}))
}

#[test]
fn sequencer_mixer_controls_route_undo_persist_and_change_exported_audio() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add audio track");
    let source = app.directory.path().join("tone.wav");
    let tone = AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..24_000)
            .map(|frame| 0.2 * (std::f32::consts::TAU * 440.0 * frame as f32 / 48_000.0).sin())
            .collect(),
    };
    tone.write_wav(&source).unwrap();
    app.command(json!({"command":"drop","paths":[source],"track":0}));
    menu(&mut app, "Track", "Add folder");
    let state = menu(&mut app, "Track", "Add bus");
    assert!(visible(&state, "Ch 1 > Master"));
    assert!(visible(&state, "Ch 2 > Master"));
    assert!(
        !visible(&state, "Ch 3 > Master"),
        "Folders must not consume mixer channels"
    );

    app.click("New audio");
    menu(&mut app, "Window", "Routing");
    app.click("Master output");
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == "New bus")
        .count()
        - 1;
    app.command(json!({"command":"click","label":"New bus","occurrence":occurrence}));
    let state = app.key("Escape");
    assert!(visible(&state, "Ch 1 > New bus"));

    let original_gain = state["project"]["tracks"][0]["gain"].as_f64().unwrap();
    let db_label = format!("{:.1} dB", 20.0 * original_gain.log10());
    let state = drag(&mut app, &db_label, -30.0);
    let gain = state["project"]["tracks"][0]["gain"].as_f64().unwrap();
    assert!(
        gain < original_gain * 0.9 && gain > 0.0,
        "Volume drag must change the channel: {state}"
    );
    let state = drag(&mut app, "Pan 0", 50.0);
    let pan = state["project"]["tracks"][0]["pan"].as_f64().unwrap();
    assert!(pan > 0.2 && pan <= 1.0);
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"]["tracks"][0]["pan"], 0.0);
    assert_eq!(
        state["project"]["tracks"][0]["gain"].as_f64().unwrap(),
        gain
    );
    let state = menu(&mut app, "Edit", "Redo");
    assert_eq!(state["project"]["tracks"][0]["pan"].as_f64().unwrap(), pan);
    let state = app.click("M");
    assert_eq!(state["project"]["tracks"][0]["mute"], true);
    app.click("M");
    let state = app.click("S");
    assert_eq!(state["project"]["tracks"][0]["solo"], true);
    app.click("S");

    let state = menu(&mut app, "Window", "Mixer");
    assert!(visible(&state, "Ch 1 > New bus"));
    assert!(visible(&state, "Ch 2 > Master"));
    assert!(visible(&state, &format!("{:.1} dB", 20.0 * gain.log10())));
    let saved = app.directory.path().join("mix.daw.json");
    app.command(json!({"command":"save","path":saved}));
    let project = Project::load(&saved).unwrap();
    assert!((f64::from(project.tracks[0].gain) - gain).abs() < 1e-6);
    assert!((f64::from(project.tracks[0].pan) - pan).abs() < 1e-6);
    app.command(json!({"command":"open","path":saved}));
    let output = app.directory.path().join("mixed.wav");
    app.command(json!({"command":"export","path":output}));
    let state = app.command(json!({"command":"wait_export"}));
    assert_eq!(state["error"], false);
    let audio = AudioData::load_wav(&output).unwrap();
    let mut energy = [0.0_f64; 2];
    for frame in audio.samples.as_chunks::<2>().0 {
        for channel in 0..2 {
            energy[channel] += f64::from(frame[channel]).powi(2);
        }
    }
    assert!(energy[1] > 1.0, "Routed audio must reach the master");
    assert!(
        energy[1] > energy[0] * 1.5,
        "Pan must affect the rendered channel balance: {energy:?}"
    );
    let peak = audio
        .samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    assert!(
        f64::from(peak) <= 0.2 * gain + 0.001,
        "The arranger gain must affect the export"
    );
    app.quit();
}
