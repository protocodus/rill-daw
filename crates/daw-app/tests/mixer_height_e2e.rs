//! Mixer height follows the window while its channel controls remain usable.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug)]
struct Bounds {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Bounds {
    fn from(rect: &Value) -> Self {
        Self {
            left: rect["min"][0].as_f64().unwrap(),
            top: rect["min"][1].as_f64().unwrap(),
            right: rect["max"][0].as_f64().unwrap(),
            bottom: rect["max"][1].as_f64().unwrap(),
        }
    }

    fn width(self) -> f64 {
        self.right - self.left
    }

    fn height(self) -> f64 {
        self.bottom - self.top
    }
}

fn labels(state: &Value, text: &str) -> Vec<Bounds> {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| label["text"] == text)
        .map(Bounds::from)
        .collect()
}

fn painted(app: &mut Harness) -> Value {
    app.command(json!({"command":"inspect", "paint":true}))
}

fn mixer_strips(state: &Value) -> Vec<Bounds> {
    let columns: Vec<_> = state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|channel| Bounds::from(&channel["strip"]))
        .chain(std::iter::once(Bounds::from(
            &state["mixer"]["master_strip"],
        )))
        .collect();
    let mut strips: Vec<_> = state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|shape| shape["fill"][3].as_u64().unwrap_or(0) > 0)
        .map(|shape| Bounds::from(&shape["rect"]))
        .filter(|rect| {
            rect.height() > 250.0
                && columns.iter().any(|column| {
                    // The upper edge can be clipped after a short window scrolls.
                    rect.top >= column.top - 0.1
                        && (rect.bottom - column.bottom).abs() < 0.1
                        && rect.left >= column.left
                        && rect.right <= column.right
                        && rect.width() >= column.width() * 0.5
                })
        })
        .collect();
    strips.sort_by(|a, b| a.left.total_cmp(&b.left));
    assert_eq!(
        strips.len(),
        4,
        "three channels plus Master: {strips:?}; columns: {columns:?}"
    );
    strips
}

fn assert_layout(state: &Value, height: f64) -> f64 {
    assert_eq!(state["view"], "Mixer");
    let strips = mixer_strips(state);
    let routes = state["mixer"]["routes"].as_array().unwrap();
    assert!(
        !routes.is_empty(),
        "the active channel must have an output lane"
    );
    let routing_top = routes[0]["lane"]["min"][1].as_f64().unwrap();
    for strip in &strips {
        assert!(
            strip.bottom <= routing_top + 0.1
                && routing_top - strip.bottom <= f64::from(daw_ui::tokens::SPACE_SM),
            "channel controls must end immediately above the separate routing lanes at {height}: {strips:?}"
        );
        assert!(strip.bottom <= height);
        assert!((strip.top - strips[0].top).abs() < 0.1);
        assert!((strip.bottom - strips[0].bottom).abs() < 0.1);
    }
    let channels = state["mixer"]["channels"].as_array().unwrap();
    let faders: Vec<_> = channels
        .iter()
        .map(|channel| Bounds::from(&channel["fader_scale"]["rail"]))
        .collect();
    for (channel, fader) in channels.iter().zip(&faders) {
        let strip = Bounds::from(&channel["strip"]);
        let pan = Bounds::from(&channel["pan"]);
        let mute_solo = Bounds::from(&channel["mute_solo"]);
        let meter = Bounds::from(&channel["meter"]);
        let target = Bounds::from(&channel["target"]);
        assert!(
            fader.height() >= 80.0,
            "A fader must retain usable travel: {fader:?}"
        );
        assert!((fader.top - faders[0].top).abs() < 1.0);
        assert!((fader.bottom - faders[0].bottom).abs() < 1.0);
        assert!(fader.bottom <= pan.top && pan.bottom <= mute_solo.top);
        assert!(
            mute_solo.bottom <= meter.top && meter.bottom <= target.top,
            "Metering follows processing and precedes output: {channel}"
        );
        assert!(
            meter.width() > meter.height() * 2.0,
            "The compact stereo meter must use horizontal rails: {meter:?}"
        );
        assert!(meter.left > strip.left && meter.right < strip.right);
        assert!(meter.bottom <= height && target.bottom <= height);
    }
    for (label, minimum_height) in [
        ("Pan C", 12.0),
        ("Mute", 17.0),
        ("Solo", 17.0),
        ("Channel", 17.0),
    ] {
        let controls = labels(state, label);
        assert_eq!(controls.len(), 3, "all {label} controls must be visible");
        for control in &controls {
            assert!(control.bottom <= height, "{label}: {control:?}");
            assert!(
                control.height() >= minimum_height,
                "{label} must not be clipped"
            );
            assert!((control.top - controls[0].top).abs() < 1.0);
        }
    }
    assert_aligned_targets(state, height);
    assert_eq!(labels(state, "Back").len(), 1);
    faders[0].height()
}

fn assert_aligned_targets(state: &Value, height: f64) {
    let targets = labels(state, "Output");
    assert_eq!(
        targets.len(),
        4,
        "three channel targets plus the output sink"
    );
    for target in &targets {
        assert!(
            target.bottom <= height && target.height() >= f64::from(daw_ui::tokens::FONT_CAPTION),
            "Output captions must remain readable and visible: {targets:?}"
        );
        assert!(
            (target.top - targets[0].top).abs() < 1.0,
            "Channel and Master output rows must align: {targets:?}"
        );
    }
}

#[test]
fn mixer_fills_height_and_resizes_faders_without_losing_channel_controls() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        project.add_audio_track("Drums");
        project.add_midi_track("Synth");
        project.add_bus("Music bus");
        let path = app.directory.path().join("mixer-height.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        // The shared control rows need more than 720 px before every channel
        // action and the routing lanes fit without vertical scrolling.
        app.command(
            json!({"command":"viewport", "width":1100, "height":900, "pixels_per_point":density}),
        );
        app.click("Mixer");
        let initial = painted(&mut app);
        let initial_height = assert_layout(&initial, 900.0);
        let original_project = initial["project"].clone();

        app.command(json!({"command":"viewport", "width":1440, "height":1300}));
        let tall = painted(&mut app);
        let tall_height = assert_layout(&tall, 1300.0);
        assert!(
            tall_height > initial_height + 300.0,
            "extra window height must extend faders and meters, not create empty space"
        );

        // Short windows may scroll once the minimum usable fader size is reached.
        for height in [720.0, 600.0] {
            app.command(json!({"command":"viewport", "width":800, "height":height}));
            let before_scroll = painted(&mut app);
            let back = labels(&before_scroll, "Back")[0];
            app.command(json!({"command":"scroll", "x":90.0, "y":350.0, "delta":-1000.0}));
            for _ in 0..20 {
                app.inspect();
            }
            let compact = painted(&mut app);
            assert_aligned_targets(&compact, height);
            assert!((labels(&compact, "Back")[0].top - back.top).abs() < 0.1);
            app.command(json!({"command":"scroll", "x":90.0, "y":350.0, "delta":1000.0}));
            for _ in 0..20 {
                app.inspect();
            }
        }
        app.click("Channel");
        let effects = app.click("Effects");
        assert_eq!(effects["popups"]["effects"], true);
        assert_eq!(effects["selected_track"], project.tracks[0].id);
        app.key("Escape");

        app.command(json!({"command":"viewport", "width":1100, "height":900}));
        let resized = painted(&mut app);
        assert!((assert_layout(&resized, 900.0) - initial_height).abs() < 1.0);
        assert_eq!(resized["project"], original_project);
        let fader = Bounds::from(&resized["mixer"]["channels"][0]["fader_scale"]["rail"]);
        let x = (fader.left + fader.right) * 0.5;
        let edited = app.command(json!({"command":"pointer", "phase":"drag",
            "x":x, "y":fader.top + fader.height() * 0.5,
            "to_x":x, "to_y":fader.top + fader.height() * 0.2}));
        assert!(
            edited["project"]["tracks"][0]["gain"].as_f64().unwrap()
                > original_project["tracks"][0]["gain"].as_f64().unwrap() + 0.1,
            "the resized fader must still edit its channel"
        );
        let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
        assert_eq!(undone["project"], original_project);
        app.quit();
    }
}
