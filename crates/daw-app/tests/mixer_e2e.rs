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

fn label_count(state: &Value, text: &str) -> usize {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == text)
        .count()
}

fn click_front(app: &mut Harness, label: &str) -> Value {
    let occurrence = label_count(&app.inspect(), label)
        .checked_sub(1)
        .expect("visible picker item");
    app.command(json!({"command":"click","label":label,"occurrence":occurrence}))
}

fn rename_selected_track(app: &mut Harness, current: &str, name: &str) {
    menu(app, "Window", "Properties");
    click_front(app, current);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":name}));
    app.key("Escape");
}

fn drag(app: &mut Harness, track: &Value, control: &str, distance: f64) -> Value {
    let state = app.inspect();
    let channel = state["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == *track)
        .expect("visible channel");
    let handle = &channel[format!("{control}_scale")]["handle_center"];
    let x = handle[0].as_f64().unwrap();
    let y = handle[1].as_f64().unwrap();
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

    let before_routing = state["project"].clone();
    app.click("Ch 1 > Master");
    let state = app.key("Escape");
    assert_eq!(
        state["project"], before_routing,
        "Cancel must preserve routing"
    );
    app.click("Ch 1 > Master");
    let state = click_front(&mut app, "New bus");
    let bus = state["project"]["tracks"][2]["id"].clone();
    assert_eq!(state["project"]["tracks"][0]["output_bus"], bus);
    assert!(visible(&state, "Ch 1 > New bus"));
    let state = menu(&mut app, "Edit", "Undo");
    assert!(state["project"]["tracks"][0]["output_bus"].is_null());
    let state = menu(&mut app, "Edit", "Redo");
    assert_eq!(state["project"]["tracks"][0]["output_bus"], bus);
    app.click("Ch 1 > New bus");
    let state = click_front(&mut app, "Master");
    assert!(state["project"]["tracks"][0]["output_bus"].is_null());
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"]["tracks"][0]["output_bus"], bus);

    let original_gain = state["project"]["tracks"][0]["gain"].as_f64().unwrap();
    let source_track = state["project"]["tracks"][0]["id"].clone();
    let state = drag(&mut app, &source_track, "gain", -6.0);
    let gain = state["project"]["tracks"][0]["gain"].as_f64().unwrap();
    assert!(
        gain < original_gain * 0.9 && gain > 0.0,
        "Volume drag must change the channel: {state}"
    );
    let state = drag(&mut app, &source_track, "pan", 50.0);
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
    let expected_db = 20.0 * gain.log10();
    assert!(
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|label| label.as_str()?.strip_suffix(" dB")?.parse::<f64>().ok())
            .any(|value| (value - expected_db).abs() < 1e-4),
        "Mixer must display the actual channel gain in dB"
    );
    let saved = app.directory.path().join("mix.daw.json");
    app.command(json!({"command":"save","path":saved}));
    let project = Project::load(&saved).unwrap();
    assert_eq!(project.tracks[0].output_bus, Some(project.tracks[2].id));
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

#[test]
fn aligned_mixer_strips_keep_channel_selection_routing_and_editors_on_demand() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let bass = project.add_midi_track("Bass");
    let drums = project.add_audio_track("Drums");
    project.add_bus("Aux");
    let path = app.directory.path().join("mixer.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1100,"height":720}));
    let state = menu(&mut app, "Window", "Mixer");
    assert_eq!(label_count(&state, "Channel"), 3);
    assert_eq!(
        label_count(&state, "Effects"),
        3,
        "inserts precede channel volume"
    );
    assert!(!visible(&state, "Instrument"));
    let route_rows: Vec<_> = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["text"].as_str().unwrap().starts_with("Ch "))
        .collect();
    assert_eq!(route_rows.len(), 3);
    assert!(route_rows.windows(2).all(|rows| {
        (rows[0]["min"][1].as_f64().unwrap() - rows[1]["min"][1].as_f64().unwrap()).abs() <= 1.0
    }));
    let levels: Vec<_> = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        // Calibrated fader endpoints carry a separate fixed +3.5 dB caption
        // below each channel's editable level readout.
        .filter(|row| {
            let text = row["text"].as_str().unwrap();
            text.ends_with(" dB") && text != "+3.5 dB"
        })
        .collect();
    assert_eq!(levels.len(), 4, "three channel gains and the master peak");
    assert!(
        levels.windows(2).all(|rows| {
            (rows[0]["min"][1].as_f64().unwrap() - rows[1]["min"][1].as_f64().unwrap()).abs() <= 1.0
        }),
        "channel gain readouts and master peak must align"
    );
    assert!(
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["text"] == "Channel")
            .all(|row| row["max"][1].as_f64().unwrap() < 720.0)
    );
    let channel = app.click("Drums");
    assert_eq!(channel["selected_track"], drums);
    assert_eq!(channel["popups"]["properties"], true);
    app.key("Escape");
    let routing = app.click("Ch 1 > Master");
    assert_eq!(routing["selected_track"], bass);
    assert_eq!(routing["popups"]["routing"], true);
    menu(&mut app, "Window", "Mixer");
    app.click("Channel");
    let effects = app.click("Effects");
    assert_eq!(effects["selected_track"], bass);
    assert_eq!(effects["popups"]["effects"], true);
    menu(&mut app, "Window", "Mixer");
    app.command(json!({"command":"click","label":"Channel","occurrence":1}));
    let menu_state = app.inspect();
    assert!(
        !visible(&menu_state, "Instrument"),
        "audio channels have no instrument editor"
    );
    app.key("Escape");
    let muted = app.command(json!({"command":"click","label":"Mute","occurrence":1}));
    assert_eq!(muted["project"]["tracks"][1]["mute"], true);
    assert_eq!(muted["project"]["tracks"][0]["mute"], false);
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(undone["project"]["tracks"][1]["mute"], false);
    // Retain usable fader height on shorter displays and let the channel area
    // scroll; the workspace Back action remains fixed above it.
    app.command(json!({"command":"viewport","width":800,"height":600}));
    app.command(json!({"command":"scroll","x":90.0,"y":350.0,"delta":-1000.0}));
    let mut scrolled = app.inspect();
    for _ in 0..20 {
        scrolled = app.inspect();
    }
    assert!(visible(&scrolled, "Target"));
    assert!(visible(&scrolled, "Back"));
    assert!(
        scrolled["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["text"] == "Target")
            .all(|row| row["max"][1].as_f64().unwrap() < 600.0)
    );
    // The pointer stayed over the fader while scrolling settled. Its stacked
    // value/volume/automation hints cover the output target in this compact
    // layout. Move and press immediately; read-only hints must let that first
    // click reach the target underneath instead of consuming the press.
    assert!(visible(&scrolled, "Channel volume"));
    assert!(visible(
        &scrolled,
        "Right-click for exact value and automation"
    ));
    let routing = app.click("Ch 1 > Master");
    assert_eq!(
        routing["popups"]["routing"], true,
        "the output target remains usable at the end of the signal chain"
    );
    assert_eq!(routing["project"], scrolled["project"]);
    assert_eq!(routing["history"], scrolled["history"]);
    app.quit();
}

#[test]
fn output_header_picker_excludes_feedback_and_remains_available_on_compact_tracks() {
    let mut app = Harness::new();
    for name in ["Bus A", "Bus B", "Bus C"] {
        menu(&mut app, "Track", "Add bus");
        rename_selected_track(&mut app, "New bus", name);
    }
    menu(&mut app, "Track", "Add audio track");
    let before = app.inspect();
    let opened = app.click("Ch 1 > Master");
    assert_eq!(
        label_count(&opened, "Bus A"),
        label_count(&before, "Bus A"),
        "A bus cannot feed itself"
    );
    assert_eq!(
        label_count(&opened, "New audio"),
        label_count(&before, "New audio"),
        "Audio tracks cannot be bus destinations"
    );
    assert_eq!(
        label_count(&opened, "Bus B"),
        label_count(&before, "Bus B") + 1
    );
    let state = click_front(&mut app, "Bus B");
    assert_eq!(
        state["project"]["tracks"][0]["output_bus"],
        state["project"]["tracks"][1]["id"]
    );
    app.click("Ch 2 > Master");
    let state = click_front(&mut app, "Bus C");
    assert_eq!(
        state["project"]["tracks"][1]["output_bus"],
        state["project"]["tracks"][2]["id"]
    );

    let before = app.inspect();
    let opened = app.click("Ch 3 > Master");
    for name in ["Bus A", "Bus B", "Bus C"] {
        assert_eq!(
            label_count(&opened, name),
            label_count(&before, name),
            "{name} would create feedback"
        );
    }
    assert!(visible(&opened, "Master"));
    let state = app.key("Escape");
    assert_eq!(state["project"], before["project"]);

    app.click("Bus B");
    menu(&mut app, "Window", "Properties");
    app.click("Compact track");
    app.key("Escape");
    let state = app.click("Ch 2 > Bus C");
    assert!(visible(&state, "Master"));
    let state = click_front(&mut app, "Master");
    assert!(state["project"]["tracks"][1]["output_bus"].is_null());
    assert_eq!(state["project"]["tracks"][1]["collapsed"], true);
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(
        state["project"]["tracks"][1]["output_bus"],
        state["project"]["tracks"][2]["id"]
    );
    assert_eq!(state["project"]["tracks"][1]["collapsed"], true);
    app.quit();
}
