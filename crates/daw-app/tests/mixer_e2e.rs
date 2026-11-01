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

fn track_header<'a>(state: &'a Value, track: &Value) -> &'a Value {
    state["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|header| header["track"] == *track)
        .expect("visible track header")
}

fn click_header_section(app: &mut Harness, track: &Value, section: &str) -> Value {
    let state = app.inspect();
    let header = track_header(&state, track);
    let rect = if section == "name" {
        &header["name"]
    } else {
        &header["sections"][section]
    };
    assert!(
        !rect.is_null(),
        "Missing {section} on track {track}: {header}"
    );
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn assert_header_output(state: &Value, track: &Value, destination: &str) {
    let bounds = &track_header(state, track)["sections"]["routing"];
    assert!(
        state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| {
                label["text"] == destination
                    && (0..2).all(|axis| {
                        label["min"][axis].as_f64().unwrap()
                            >= bounds["min"][axis].as_f64().unwrap()
                            && label["max"][axis].as_f64().unwrap()
                                <= bounds["max"][axis].as_f64().unwrap()
                    })
            }),
        "The track's Routing section must show {destination}"
    );
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
    let source_track = state["project"]["tracks"][0]["id"].clone();
    let folder = state["project"]["tracks"][1]["id"].clone();
    let bus = state["project"]["tracks"][2]["id"].clone();
    assert_header_output(&state, &source_track, "Master");
    assert_header_output(&state, &bus, "Master");
    assert_eq!(state["timeline"]["channels"].as_array().unwrap().len(), 2);
    assert!(
        track_header(&state, &folder)["sections"]["routing"].is_null(),
        "Folders must not consume mixer channels"
    );

    let before_routing = state["project"].clone();
    click_header_section(&mut app, &source_track, "routing");
    let state = app.key("Escape");
    assert_eq!(
        state["project"], before_routing,
        "Cancel must preserve routing"
    );
    click_header_section(&mut app, &source_track, "routing");
    let state = click_front(&mut app, "Bus 1");
    assert_eq!(state["project"]["tracks"][0]["output_bus"], bus);
    assert_header_output(&state, &source_track, "Bus 1");
    let state = menu(&mut app, "Edit", "Undo");
    assert!(state["project"]["tracks"][0]["output_bus"].is_null());
    let state = menu(&mut app, "Edit", "Redo");
    assert_eq!(state["project"]["tracks"][0]["output_bus"], bus);
    click_header_section(&mut app, &source_track, "routing");
    let state = click_front(&mut app, "Master");
    assert!(state["project"]["tracks"][0]["output_bus"].is_null());
    let state = menu(&mut app, "Edit", "Undo");
    assert_eq!(state["project"]["tracks"][0]["output_bus"], bus);

    let original_gain = state["project"]["tracks"][0]["gain"].as_f64().unwrap();
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
    assert!(visible(&state, "Ch 1 > Bus 1"));
    assert!(visible(&state, "Ch 2 > Master"));
    let expected_db = format!("{:.1} dB", daw_ui::channel_gain_to_db(gain as f32));
    assert!(
        visible(&state, &expected_db),
        "Mixer must display the channel gain as a concise dB caption"
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
    app.command(json!({"command":"viewport","width":1100,"height":900}));
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
            .all(|row| row["max"][1].as_f64().unwrap() < 900.0)
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
    assert!(visible(&scrolled, "Output"));
    assert!(visible(&scrolled, "Back"));
    assert!(
        scrolled["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["text"] == "Output")
            .all(|row| row["max"][1].as_f64().unwrap() < 600.0)
    );
    // Hover the actual fader after scrolling settles; shared padding can move
    // it without changing the control. Its stacked value/volume/automation
    // hints must let the first press reach the output target underneath.
    // Leave a full control-height portion of the fader visible above Output;
    // the raw diagnostic rectangle can otherwise sit behind the fixed header.
    let viewport = &scrolled["mixer"]["reorder_viewport"];
    let delta = (viewport["min"][1].as_f64().unwrap() + f64::from(daw_ui::tokens::CONTROL_HEIGHT)
        - scrolled["mixer"]["channels"][0]["fader"]["max"][1]
            .as_f64()
            .unwrap())
    .max(0.0);
    if delta > 0.0 {
        app.command(
            json!({"command":"scroll", "x":viewport["max"][0].as_f64().unwrap() - 5.0,
            "y":(viewport["min"][1].as_f64().unwrap() + viewport["max"][1].as_f64().unwrap()) * 0.5,
            "delta":delta}),
        );
        for _ in 0..20 {
            scrolled = app.inspect();
        }
    }
    let fader = &scrolled["mixer"]["channels"][0]["fader"];
    let viewport = &scrolled["mixer"]["reorder_viewport"];
    let x = (fader["min"][0].as_f64().unwrap() + fader["max"][0].as_f64().unwrap()) * 0.5;
    let visible_top = fader["min"][1]
        .as_f64()
        .unwrap()
        .max(viewport["min"][1].as_f64().unwrap());
    let visible_bottom = fader["max"][1]
        .as_f64()
        .unwrap()
        .min(viewport["max"][1].as_f64().unwrap());
    assert!(visible_bottom - visible_top >= f64::from(daw_ui::tokens::CONTROL_HEIGHT) - 1.0);
    let y = (visible_top + visible_bottom) * 0.5;
    let output = &scrolled["mixer"]["channels"][0]["target"];
    assert!(output["min"][1].as_f64().unwrap() >= viewport["min"][1].as_f64().unwrap());
    assert!(output["max"][1].as_f64().unwrap() <= viewport["max"][1].as_f64().unwrap());
    assert!((0.0..800.0).contains(&x) && (0.0..600.0).contains(&y));
    app.command(json!({"command":"pointer","phase":"move","x":x,"y":y}));
    for _ in 0..20 {
        scrolled = app.inspect();
    }
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
        rename_selected_track(&mut app, "Bus 1", name);
    }
    menu(&mut app, "Track", "Add audio track");
    let before = app.inspect();
    let bus_a = before["project"]["tracks"][0]["id"].clone();
    let bus_b = before["project"]["tracks"][1]["id"].clone();
    let bus_c = before["project"]["tracks"][2]["id"].clone();
    let opened = click_header_section(&mut app, &bus_a, "routing");
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
    click_header_section(&mut app, &bus_b, "routing");
    let state = click_front(&mut app, "Bus C");
    assert_eq!(
        state["project"]["tracks"][1]["output_bus"],
        state["project"]["tracks"][2]["id"]
    );

    let before = app.inspect();
    let opened = click_header_section(&mut app, &bus_c, "routing");
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

    let selected = click_header_section(&mut app, &bus_b, "name");
    assert_eq!(selected["selected_track"], bus_b);
    menu(&mut app, "Window", "Properties");
    app.click("Compact track");
    app.key("Escape");
    let state = click_header_section(&mut app, &bus_b, "details");
    assert_eq!(state["selected_track"], bus_b);
    assert_eq!(state["popups"]["properties"], true);
    let state = click_front(&mut app, "Routing");
    assert_eq!(state["popups"]["routing"], true);
    click_front(&mut app, "Bus C");
    let state = click_front(&mut app, "Master output");
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
