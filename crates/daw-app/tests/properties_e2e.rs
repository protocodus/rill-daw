//! Focused Properties pages retain real editing and fit the smallest desktop.
mod common;

use common::Harness;
use daw_core::{
    AudioData, AutomationParameter, AutomationTarget, Clip, ClipContent, Note, Project,
};
use serde_json::{Value, json};

fn window(app: &mut Harness, name: &str) -> Value {
    app.click("Window");
    app.click(name)
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn rect<'a>(state: &'a Value, label: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|rect| rect["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn accessible(state: &Value, labels: &[&str]) {
    for label in labels {
        let rect = rect(state, label);
        assert!(rect["min"][0].as_f64().unwrap() >= 24.0, "{label}: {rect}");
        assert!(
            rect["max"][0].as_f64().unwrap() <= 1076.0,
            "{label}: {rect}"
        );
        assert!(rect["min"][1].as_f64().unwrap() >= 120.0, "{label}: {rect}");
        assert!(rect["max"][1].as_f64().unwrap() <= 696.0, "{label}: {rect}");
        assert!(
            rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap() >= 13.0,
            "Clipped {label}: {rect}"
        );
    }
}

fn front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|text| *text == label)
        .count()
        - 1;
    app.command(json!({"command":"click", "label":label, "occurrence":occurrence}))
}

fn select_track(app: &mut Harness, name: &str) {
    let arrangement = window(app, "Arrangement");
    if name == "Track folder" {
        // The folder header toggles collapse; its empty timeline row only selects.
        let y = center(rect(&arrangement, "Track folder")).1;
        let x = arrangement["timeline"]["canvas"]["min"][0]
            .as_f64()
            .unwrap()
            + 24.0;
        app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    } else {
        app.click(name);
    }
}

fn undo(app: &mut Harness, redo: bool) -> Value {
    app.command(json!({"command":"key", "key":"Z", "cmd":true, "shift":redo}))
}

fn selected_clip(app: &mut Harness, id: u64) -> Value {
    window(app, "Arrangement");
    app.key("Q");
    let state = app.click("Fit");
    let clip = state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap();
    let (x, y) = center(&clip["rect"]);
    let selected = app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}));
    assert_eq!(selected["selected_clip"], id);
    window(app, "Properties")
}

fn fixture(app: &mut Harness) -> (u64, u64) {
    let source = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 48000,
        channels: 1,
        samples: vec![0.1; 144000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio channel");
    let mut clip = Clip::audio("Linked take", 2.0, 4.0, source.to_string_lossy());
    clip.offset = 0.5;
    if let ClipContent::Audio {
        gain,
        fade_in,
        fade_out,
        ..
    } = &mut clip.content
    {
        *gain = 0.75;
        *fade_in = 0.25;
        *fade_out = 0.5;
    }
    let first = clip.id;
    project.track_mut(audio).unwrap().clips.push(clip);
    let second = project.clone_clip(audio, first).unwrap();
    let midi = project.add_midi_track("MIDI channel");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Phrase",
        0.0,
        2.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    project.add_bus("Bus channel");
    project.add_folder("Track folder");
    project
        .add_automation_track(
            "Gain automation",
            AutomationTarget {
                track: audio,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let path = app.directory.path().join("properties.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    (first, second)
}

fn clip(state: &Value, id: u64) -> &Value {
    state["project"]["tracks"][0]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == id)
        .unwrap()
}

#[test]
fn properties_pages_fit_and_expose_only_controls_for_the_selected_item() {
    let mut app = Harness::new();
    let (first, _) = fixture(&mut app);
    let original = app.inspect()["project"].clone();
    for density in [1.0, 2.0] {
        for dark in [false, true] {
            app.command(json!({"command":"viewport", "width":1100, "height":720,
                "pixels_per_point":density}));
            app.command(json!({"command":"system_theme", "dark":dark}));
            let selected = selected_clip(&mut app, first);
            accessible(
                &selected,
                &[
                    "Clip",
                    "Track",
                    "Name",
                    "Beat 3.00",
                    "4.00 beats",
                    "0.50 beats",
                    "Clip gain",
                    "-2.5 dB",
                    "Fade in",
                    "Fade out",
                    "Make independent",
                ],
            );
            assert!(!visible(&selected, "Compact track"));
            assert!(!visible(&selected, "Path latency"));
            let track = front(&mut app, "Track");
            accessible(&track, &["Color", "Compact track", "Folder"]);
            for channel_control in ["Gain", "Pan", "Mute", "Solo", "Path latency"] {
                assert!(!visible(&track, channel_control));
            }
            let channel = front(&mut app, "Channel");
            accessible(&channel, &["Gain", "Pan", "Mute", "Solo", "Path latency"]);
            assert!(!visible(&channel, "Source offset"));
            assert!(!visible(&channel, "Fade in"));
            accessible(
                &app.click("Clip"),
                &["Source offset", "Fade out", "Make independent"],
            );

            for name in [
                "Audio channel",
                "MIDI channel",
                "Bus channel",
                "Track folder",
                "Gain automation",
            ] {
                select_track(&mut app, name);
                let track = window(&mut app, "Properties");
                accessible(&track, &["Name", "Color", "Folder", "Top level"]);
                assert!(!visible(&track, "Source offset"));
                match name {
                    "Track folder" => {
                        accessible(&track, &["Collapse folder", "Mute", "Solo"]);
                        for unused in ["Gain", "Pan", "Path latency", "Bypass automation"] {
                            assert!(!visible(&track, unused), "Folder exposes {unused}");
                        }
                    }
                    "Gain automation" => {
                        accessible(&track, &["Bypass automation"]);
                        for unused in ["Gain", "Pan", "Mute", "Solo", "Path latency"] {
                            assert!(!visible(&track, unused), "Automation exposes {unused}");
                        }
                    }
                    _ => {
                        for channel_control in ["Gain", "Pan", "Mute", "Solo", "Path latency"] {
                            assert!(!visible(&track, channel_control));
                        }
                        accessible(
                            &front(&mut app, "Channel"),
                            &["Gain", "Pan", "Mute", "Solo", "Path latency"],
                        );
                    }
                }
                assert_eq!(track["project"], original);
            }
        }
    }
    let bypassed = app.click("Bypass automation");
    assert_eq!(bypassed["project"]["tracks"][4]["mute"], true);
    assert_eq!(undo(&mut app, false)["project"], original);
    select_track(&mut app, "Track folder");
    window(&mut app, "Properties");
    assert_eq!(app.click("Mute")["project"]["tracks"][3]["mute"], true);
    assert_eq!(undo(&mut app, false)["project"], original);
    app.quit();
}

fn edit_fade(app: &mut Harness, row: &str, value: &str) -> Value {
    let state = app.inspect();
    let label = rect(&state, row);
    let y = center(label).1;
    let number = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| {
            candidate["text"]
                .as_str()
                .is_some_and(|text| text.ends_with(" beats"))
                && (center(candidate).1 - y).abs() < 12.0
                && candidate["min"][0].as_f64().unwrap() > label["max"][0].as_f64().unwrap()
        })
        .unwrap_or_else(|| panic!("Missing value for {row}: {}", state["visible_labels"]));
    let (x, y) = center(number);
    app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":value}));
    app.key("Enter")
}

#[test]
fn clip_gain_and_fades_edit_linked_instances_and_support_undo_then_independence() {
    let mut app = Harness::new();
    let (first, second) = fixture(&mut app);
    app.command(json!({"command":"viewport", "width":1100, "height":720}));
    let before = selected_clip(&mut app, first);
    let left = before["clip_name_rect"][0].as_f64().unwrap();
    let right =
        rect(&before, "-2.5 dB")["min"][0].as_f64().unwrap() - f64::from(daw_ui::tokens::SPACE_SM);
    let y = center(rect(&before, "Clip gain")).1;
    let changed = app.command(json!({"command":"pointer", "phase":"click",
        "x":left + (right - left) * 0.7, "y":y}));
    let gain = clip(&changed, first)["content"]["Audio"]["gain"]
        .as_f64()
        .unwrap();
    assert!(
        gain > 1.0 && gain <= 2.0,
        "Gain slider did not respond: {gain}"
    );
    assert_eq!(
        clip(&changed, first)["content"],
        clip(&changed, second)["content"]
    );
    assert_eq!(undo(&mut app, false)["project"], before["project"]);
    assert_eq!(undo(&mut app, true)["project"], changed["project"]);
    let faded_in = edit_fade(&mut app, "Fade in", "1.25");
    for id in [first, second] {
        assert_eq!(clip(&faded_in, id)["content"]["Audio"]["fade_in"], 1.25);
    }
    assert_eq!(undo(&mut app, false)["project"], changed["project"]);
    assert_eq!(undo(&mut app, true)["project"], faded_in["project"]);
    let faded_out = edit_fade(&mut app, "Fade out", "0.75");
    for id in [first, second] {
        assert_eq!(clip(&faded_out, id)["content"]["Audio"]["fade_out"], 0.75);
    }
    assert_eq!(undo(&mut app, false)["project"], faded_in["project"]);
    assert_eq!(undo(&mut app, true)["project"], faded_out["project"]);
    let detached = app.click("Make independent");
    assert!(clip(&detached, first)["link_group"].is_null());
    let independent = edit_fade(&mut app, "Fade out", "1.5");
    assert_eq!(
        clip(&independent, first)["content"]["Audio"]["fade_out"],
        1.5
    );
    assert_eq!(
        clip(&independent, second)["content"]["Audio"]["fade_out"],
        0.75
    );
    for state in [&faded_in, &faded_out, &independent] {
        for id in [first, second] {
            for field in ["start", "length", "offset"] {
                assert_eq!(clip(state, id)[field], clip(&before, id)[field]);
            }
        }
    }
    app.quit();
}

#[test]
fn inspecting_existing_high_gain_does_not_change_or_dirty_the_project() {
    let mut app = Harness::new();
    let (first, _) = fixture(&mut app);
    let mut project: Project = serde_json::from_value(app.inspect()["project"].clone()).unwrap();
    project.tracks[0].gain = 2.0;
    project.tracks[0].latency_samples = 220_000;
    // Inspect the stored manual gain. Active lanes now correctly show their
    // sounding value; bypass the fixture's lane for this range-preservation test.
    for track in &mut project.tracks {
        if track.automation.is_some() {
            track.mute = true;
        }
    }
    for clip in &mut project.tracks[0].clips {
        if let ClipContent::Audio { gain, .. } = &mut clip.content {
            *gain = 100.0;
        }
    }
    let path = app.directory.path().join("high-gain.daw.json");
    project.save(&path).unwrap();
    let opened = app.command(json!({"command":"open", "path":path}));
    assert_eq!(
        opened["modified"],
        false,
        "Opened: {}",
        json!({"history":opened["history"], "gain":opened["project"]["tracks"][0]["gain"]})
    );
    let properties = selected_clip(&mut app, first);
    assert!(visible(&properties, "40.0 dB"));
    assert_eq!(properties["project"], opened["project"]);
    assert_eq!(properties["modified"], false);
    let track = front(&mut app, "Channel");
    assert!(visible(&track, "6.0 dB"));
    assert!(visible(&track, "220000 samples"));
    assert_eq!(track["project"], opened["project"]);
    assert_eq!(track["modified"], false);
    assert!(track["history"]["undo"].as_array().unwrap().is_empty());
    let editor = window(&mut app, "Clip Editor");
    assert_eq!(editor["project"], opened["project"]);
    assert_eq!(editor["modified"], false);
    assert!(editor["history"]["undo"].as_array().unwrap().is_empty());
    let mixer = window(&mut app, "Mixer");
    assert_eq!(mixer["project"], opened["project"]);
    assert_eq!(mixer["modified"], false);
    assert!(mixer["history"]["undo"].as_array().unwrap().is_empty());
    window(&mut app, "Properties");
    let track = front(&mut app, "Channel");
    let right =
        rect(&track, "6.0 dB")["min"][0].as_f64().unwrap() - f64::from(daw_ui::tokens::SPACE_SM);
    let y = center(rect(&track, "Gain")).1;
    let edited = app.command(json!({"command":"pointer", "phase":"click",
        "x":right - 60.0, "y":y}));
    let gain = edited["project"]["tracks"][0]["gain"].as_f64().unwrap();
    assert!(
        gain > 0.0 && gain < 1.5,
        "Deliberate gain edit failed: {gain}"
    );
    assert_eq!(undo(&mut app, false)["project"], opened["project"]);
    app.quit();
}
