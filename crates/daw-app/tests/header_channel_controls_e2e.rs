//! Graphic track-header controls edit the same mixer channels used by rendering.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

fn channel(state: &Value, id: u64) -> &Value {
    state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn drag(app: &mut Harness, track: u64, control: &str, target: f64) -> Value {
    let state = app.inspect();
    let rect = &channel(&state, track)[control];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let to_x = rect["min"][0].as_f64().unwrap()
        + (rect["max"][0].as_f64().unwrap() - rect["min"][0].as_f64().unwrap()) * target;
    app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y, "to_x":to_x, "to_y":y}))
}

#[test]
fn header_sliders_share_mixer_values_undo_and_render_while_compact_channels_keep_controls() {
    let mut app = Harness::new();
    let source = app.directory.path().join("tone.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.2; 48_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Tone",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("Synth");
    let bus = project.add_bus("Bus");
    project.add_folder("Folder");
    project.track_mut(audio).unwrap().output_bus = Some(bus);
    let path = app.directory.path().join("header-channels.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open", "path":path}));
    for dark in [false, true] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        let state = app.inspect();
        assert_eq!(state["timeline"]["channels"].as_array().unwrap().len(), 3);
        for track in [audio, midi, bus] {
            let controls = channel(&state, track);
            assert_eq!(
                controls["peaks"],
                json!([0.0, 0.0]),
                "Stopped channels cannot fabricate a level"
            );
            assert!(
                controls["gain"]["max"][0].as_f64().unwrap()
                    < controls["meter"]["min"][0].as_f64().unwrap()
            );
            assert!(
                controls["pan"]["max"][1].as_f64().unwrap()
                    <= controls["meter"]["max"][1].as_f64().unwrap()
            );
        }
    }
    let changed = drag(&mut app, audio, "gain", 0.8);
    let gain = changed["project"]["tracks"][0]["gain"].as_f64().unwrap();
    assert!(gain > 0.0 && gain < original["project"]["tracks"][0]["gain"].as_f64().unwrap());
    assert_eq!(
        changed["project"]["tracks"][0]["clips"],
        original["project"]["tracks"][0]["clips"]
    );
    assert_eq!(
        app.command(json!({"command":"key","key":"Z","cmd":true}))["project"],
        original["project"]
    );
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    let panned = drag(&mut app, audio, "pan", 0.85);
    assert!(panned["project"]["tracks"][0]["pan"].as_f64().unwrap() > 0.4);
    assert_eq!(
        panned["project"]["tracks"][0]["clips"],
        original["project"]["tracks"][0]["clips"]
    );
    let mixer = menu(&mut app, "Window", "Mixer");
    assert_eq!(mixer["project"], panned["project"]);
    let expected_db = 20.0 * gain.log10();
    assert!(
        mixer["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|label| label.strip_suffix(" dB"))
            .filter_map(|value| value.parse::<f64>().ok())
            .any(|db| (db - expected_db).abs() < 0.0001),
        "The precise mixer field must reflect the edited header gain: {}",
        mixer["visible_labels"]
    );
    menu(&mut app, "Window", "Arrangement");
    app.click("Audio");
    menu(&mut app, "Window", "Properties");
    app.click("Compact track");
    let compact = app.key("Escape");
    assert_eq!(channel(&compact, audio)["compact"], true);
    let repanned = drag(&mut app, audio, "pan", 0.75);
    assert!(repanned["project"]["tracks"][0]["pan"].as_f64().unwrap() > 0.2);
    let saved = app.directory.path().join("edited.rill");
    app.command(json!({"command":"save", "path":saved}));
    let loaded = Project::load(&saved).unwrap();
    assert!((loaded.track(audio).unwrap().gain as f64 - gain).abs() < 1e-6);
    assert_eq!(loaded.track(audio).unwrap().clips[0].name, "Tone");
    assert_eq!(loaded.track(audio).unwrap().clips[0].length, 2.0);
    assert!(matches!(
        loaded.track(audio).unwrap().clips[0].content,
        daw_core::ClipContent::Audio { gain: 1.0, .. }
    ));
    let output = app.directory.path().join("mixed.wav");
    app.command(json!({"command":"export", "path":output}));
    app.command(json!({"command":"wait_export"}));
    let output = AudioData::load_wav(&output).unwrap();
    let mut energy = [0.0_f64; 2];
    for frame in output.samples.as_chunks::<2>().0 {
        for index in 0..2 {
            energy[index] += f64::from(frame[index]).powi(2);
        }
    }
    assert!(
        energy[1] > energy[0] * 1.5 && energy[1] > 0.01,
        "Header channel edits affect exported audio: {energy:?}"
    );
    app.quit();
}
