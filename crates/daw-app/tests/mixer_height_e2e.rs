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
    let mut strips: Vec<_> = state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|shape| shape["fill"][3].as_u64().unwrap_or(0) > 0)
        .map(|shape| Bounds::from(&shape["rect"]))
        .filter(|rect| {
            (rect.width() - (180.0 - f64::from(daw_ui::tokens::SPACE_XS))).abs() < 0.1
                && rect.height() > 250.0
        })
        .collect();
    strips.sort_by(|a, b| a.left.total_cmp(&b.left));
    assert_eq!(strips.len(), 4, "three channels plus Master: {strips:?}");
    strips
}

fn rails(state: &Value, strip: Bounds) -> Vec<Bounds> {
    let mut rails: Vec<_> = state["painted_rects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shape| Bounds::from(&shape["rect"]))
        .filter(|rect| {
            rect.left > strip.left
                && rect.right < strip.right
                && rect.width() >= 3.0
                && rect.width() <= 30.0
                && rect.height() >= 80.0
        })
        .collect();
    rails.sort_by(|a, b| a.left.total_cmp(&b.left));
    rails
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
    let mut all_rails = Vec::new();
    for (index, strip) in strips.into_iter().enumerate() {
        let channel_rails = rails(state, strip);
        assert_eq!(
            channel_rails.len(),
            if index == 3 { 2 } else { 3 },
            "each channel has its fader and stereo meter; Master has a stereo meter"
        );
        all_rails.extend(channel_rails);
    }
    for rail in &all_rails {
        assert!((rail.top - all_rails[0].top).abs() < 1.0);
        assert!((rail.bottom - all_rails[0].bottom).abs() < 1.0);
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
    assert_eq!(labels(state, "Back").len(), 1);
    all_rails[0].height()
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
        app.command(
            json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":density}),
        );
        app.click("Mixer");
        let initial = painted(&mut app);
        let initial_height = assert_layout(&initial, 720.0);
        let original_project = initial["project"].clone();

        app.command(json!({"command":"viewport", "width":1440, "height":1100}));
        let tall = painted(&mut app);
        let tall_height = assert_layout(&tall, 1100.0);
        assert!(
            tall_height > initial_height + 300.0,
            "extra window height must extend faders and meters, not create empty space"
        );

        // Short windows may scroll once the minimum usable fader size is reached.
        app.command(json!({"command":"viewport", "width":800, "height":600}));
        let before_scroll = painted(&mut app);
        let back = labels(&before_scroll, "Back")[0];
        app.command(json!({"command":"scroll", "x":90.0, "y":350.0, "delta":-1000.0}));
        for _ in 0..20 {
            app.inspect();
        }
        let compact = painted(&mut app);
        let actions = labels(&compact, "Target");
        assert_eq!(
            actions.len(),
            4,
            "three channel targets plus the output sink"
        );
        assert!(
            actions
                .iter()
                .all(|action| action.bottom <= 600.0 && action.height() >= 17.0)
        );
        assert!((labels(&compact, "Back")[0].top - back.top).abs() < 0.1);
        app.command(json!({"command":"scroll", "x":90.0, "y":350.0, "delta":1000.0}));
        for _ in 0..20 {
            app.inspect();
        }
        app.click("Channel");
        let effects = app.click("Effects");
        assert_eq!(effects["popups"]["effects"], true);
        assert_eq!(effects["selected_track"], project.tracks[0].id);
        app.key("Escape");

        app.command(json!({"command":"viewport", "width":1100, "height":720}));
        let resized = painted(&mut app);
        assert!((assert_layout(&resized, 720.0) - initial_height).abs() < 1.0);
        assert_eq!(resized["project"], original_project);
        let fader = rails(&resized, mixer_strips(&resized)[0])[0];
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
