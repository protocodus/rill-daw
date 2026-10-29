//! Clip properties and channel mixing remain distinct through the production UI.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Note, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|text| *text == label)
        .count()
        .checked_sub(1)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]));
    app.command(json!({"command":"click", "label":label, "occurrence":occurrence}))
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

fn center_y(rect: &Value) -> f64 {
    (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0
}

fn fixture(app: &mut Harness) -> Value {
    let source = app.directory.path().join("drums.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 96_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let midi = project.add_midi_track("Synth");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Pattern",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let audio = project.add_audio_track("Drums");
    let track = project.track_mut(audio).unwrap();
    track.gain = 0.5;
    track.pan = 0.2;
    let mut clip = Clip::audio("Take", 0.0, 4.0, source.to_string_lossy());
    if let ClipContent::Audio { gain, .. } = &mut clip.content {
        *gain = 0.75;
    }
    track.clips.push(clip);
    let path = app.directory.path().join("channels.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}))["project"].clone()
}

#[test]
fn clips_and_channels_have_separate_control_scopes_and_edits() {
    let mut app = Harness::new();
    let original = fixture(&mut app);
    for dark in [false, true] {
        for density in [1.0, 2.0] {
            app.command(json!({"command":"system_theme", "dark":dark}));
            app.command(json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}));
            menu(&mut app, "Window", "Arrangement");
            app.click("Pattern");
            let midi = menu(&mut app, "Window", "Properties");
            assert!(visible(&midi, "Properties of Pattern"));
            for channel_control in ["Gain", "Clip gain", "Pan", "Mute", "Solo"] {
                assert!(
                    !visible(&midi, channel_control),
                    "MIDI clip exposes {channel_control}"
                );
            }
            let track = front(&mut app, "Track");
            assert!(visible(&track, "Track · Synth"));
            assert!(visible(&track, "Color"));
            for channel_control in ["Gain", "Pan", "Mute", "Solo"] {
                assert!(
                    !visible(&track, channel_control),
                    "Track metadata exposes {channel_control}"
                );
            }
            let channel = front(&mut app, "Channel");
            assert!(visible(&channel, "Channel · Synth"));
            assert!(visible(&channel, "Gain"));
            assert!(visible(&channel, "Pan"));
            assert!(!visible(&channel, "Clip gain"));
            assert_eq!(channel["project"], original);
            let muted = front(&mut app, "Mute");
            assert_eq!(muted["project"]["tracks"][0]["mute"], true);
            assert_eq!(
                muted["project"]["tracks"][0]["clips"],
                original["tracks"][0]["clips"]
            );
            let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
            assert_eq!(undone["project"], original);
            let pan_value = rect(&undone, "0.0%");
            let x = (pan_value["min"][0].as_f64().unwrap() + pan_value["max"][0].as_f64().unwrap())
                / 2.0;
            let y = center_y(pan_value);
            app.command(json!({"command":"pointer", "phase":"double_click", "x":x, "y":y}));
            app.command(json!({"command":"key", "key":"A", "cmd":true}));
            app.command(json!({"command":"text", "text":"45"}));
            let panned = app.key("Enter");
            assert!((panned["project"]["tracks"][0]["pan"].as_f64().unwrap() - 0.45).abs() < 1e-6);
            assert_eq!(
                panned["project"]["tracks"][0]["clips"],
                original["tracks"][0]["clips"]
            );
            assert_eq!(
                app.command(json!({"command":"key", "key":"Z", "cmd":true}))["project"],
                original
            );

            menu(&mut app, "Window", "Arrangement");
            app.click("Take");
            let audio = menu(&mut app, "Window", "Properties");
            assert!(visible(&audio, "Properties of Take"));
            assert!(visible(&audio, "Clip gain"));
            assert!(!visible(&audio, "Pan"));
            let left = audio["clip_name_rect"][0].as_f64().unwrap();
            let right = rect(&audio, "-2.5 dB")["min"][0].as_f64().unwrap()
                - f64::from(daw_ui::tokens::SPACE_SM);
            let y = center_y(rect(&audio, "Clip gain"));
            let changed = app.command(
                json!({"command":"pointer", "phase":"click", "x":left + (right-left)*0.7, "y":y}),
            );
            assert!(
                changed["project"]["tracks"][1]["clips"][0]["content"]["Audio"]["gain"]
                    .as_f64()
                    .unwrap()
                    > 1.0
            );
            assert_eq!(
                changed["project"]["tracks"][1]["gain"],
                original["tracks"][1]["gain"]
            );
            assert_eq!(
                changed["project"]["tracks"][1]["pan"],
                original["tracks"][1]["pan"]
            );
            assert_eq!(
                app.command(json!({"command":"key", "key":"Z", "cmd":true}))["project"],
                original
            );
        }
    }
    app.quit();
}

#[test]
fn mixer_names_open_the_correct_channel_and_connect_to_related_views() {
    let mut app = Harness::new();
    let original = fixture(&mut app);
    menu(&mut app, "Window", "Mixer");
    let channel = app.click("Synth");
    assert!(visible(&channel, "Channel · Synth"));
    assert_eq!(channel["popups"]["properties"], true);
    assert_eq!(channel["selected_track"], original["tracks"][0]["id"]);
    let effects = front(&mut app, "Effects");
    assert!(visible(&effects, "Effects of Synth"));
    let channel = front(&mut app, "Channel");
    assert!(visible(&channel, "Channel · Synth"));
    let instrument = front(&mut app, "Instrument");
    assert_eq!(instrument["popups"]["instrument"], true);
    front(&mut app, "Channel");
    let routing = front(&mut app, "Routing");
    assert_eq!(routing["popups"]["routing"], true);
    let mixer = front(&mut app, "Mixer");
    assert_eq!(mixer["view"], "Mixer");
    let drums = app.click("Drums");
    assert!(visible(&drums, "Channel · Drums"));
    assert_eq!(drums["selected_track"], original["tracks"][1]["id"]);
    assert!(
        !visible(&drums, "Instrument"),
        "Audio channels have no instrument action"
    );
    assert_eq!(
        drums["project"], original,
        "Navigation does not change the project"
    );
    let returned = app.key("Escape");
    assert_eq!(returned["view"], "Mixer");
    assert_eq!(returned["project"], original);
    app.quit();
}
