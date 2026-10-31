//! The arranger's channel sections retain independent, bounded mouse targets.
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
    fn from(value: &Value) -> Self {
        Self {
            left: value["min"][0].as_f64().unwrap(),
            top: value["min"][1].as_f64().unwrap(),
            right: value["max"][0].as_f64().unwrap(),
            bottom: value["max"][1].as_f64().unwrap(),
        }
    }

    fn center(self) -> (f64, f64) {
        (
            (self.left + self.right) * 0.5,
            (self.top + self.bottom) * 0.5,
        )
    }

    fn contains(self, other: Self) -> bool {
        other.left >= self.left - 0.1
            && other.right <= self.right + 0.1
            && other.top >= self.top - 0.1
            && other.bottom <= self.bottom + 0.1
    }

    fn overlaps(self, other: Self) -> bool {
        self.left < other.right - 0.1
            && self.right > other.left + 0.1
            && self.top < other.bottom - 0.1
            && self.bottom > other.top + 0.1
    }
}

fn item<'a>(state: &'a Value, list: &str, track: u64) -> &'a Value {
    state["timeline"][list]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["track"] == track)
        .unwrap_or_else(|| panic!("Missing {list} for {track}: {}", state["timeline"]))
}

fn model(state: &Value, track: u64) -> &Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == track)
        .unwrap()
}

fn unchanged(state: &Value, before: &Value) {
    for field in ["project", "history", "modified"] {
        assert_eq!(
            state[field], before[field],
            "Layout/navigation changed {field}"
        );
    }
}

fn click(app: &mut Harness, bounds: &Value) -> Value {
    let (x, y) = Bounds::from(bounds).center();
    app.command(json!({"command":"pointer", "phase":"click", "x":x, "y":y}))
}

fn fixture(app: &mut Harness) -> [u64; 3] {
    let mut project = Project::new();
    let audio = project.add_audio_track("Circuit drums · room microphones");
    let midi = project.add_midi_track("Glass keys · 音楽 and a longer instrument name");
    let bus = project.add_bus("Rhythm bus");
    let path = app.directory.path().join("track-sections.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    [audio, midi, bus]
}

fn resize(app: &mut Harness, width: f64, height: f64) -> Value {
    let state = app.inspect();
    let (x, _) = Bounds::from(&state["timeline"]["header_resize"]).center();
    let y = state["timeline"]["ruler"]["min"][1].as_f64().unwrap() + 10.0;
    let old_width = state["timeline"]["header_width"].as_f64().unwrap();
    let state = app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y,
        "to_x":x + width - old_width, "to_y":y}));
    let (x, y) = Bounds::from(&state["timeline"]["row_resize"][0]["rect"]).center();
    let old_height = state["timeline"]["row_height"].as_f64().unwrap();
    let state = app.command(json!({"command":"pointer", "phase":"drag", "x":x, "y":y,
        "to_x":x, "to_y":y + height - old_height}));
    assert!((state["timeline"]["header_width"].as_f64().unwrap() - width).abs() < 0.1);
    assert!((state["timeline"]["row_height"].as_f64().unwrap() - height).abs() < 0.1);
    state
}

#[test]
fn section_bounds_stay_separate_and_compact_headers_keep_channel_access() {
    let mut app = Harness::new();
    let tracks = fixture(&mut app);
    let original = app.inspect();
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":1100,
            "pixels_per_point":density}));
        for (width, height) in [
            (224.0, 56.0),
            (224.0, 144.0),
            (288.0, 89.0),
            (288.0, 144.0),
            (400.0, 233.0),
        ] {
            let state = resize(&mut app, width, height);
            unchanged(&state, &original);
            for track in tracks {
                let header = item(&state, "tracks", track);
                let bounds = Bounds::from(&header["header"]);
                let sections = &header["sections"];
                let mut occupied: Vec<(&str, Bounds)> = Vec::new();
                for name in ["identity", "mix", "routing", "device", "meter"] {
                    if !sections[name].is_null() {
                        let section = Bounds::from(&sections[name]);
                        assert!(
                            bounds.contains(section),
                            "{name} leaves its header at {width}×{height}: {section:?}"
                        );
                        assert!(section.right > section.left && section.bottom > section.top);
                        for (other_name, other) in &occupied {
                            assert!(
                                !section.overlaps(*other),
                                "{name} overlaps {other_name} at {width}×{height}"
                            );
                        }
                        occupied.push((name, section));
                    }
                }
                assert!(!sections["mix"].is_null());
                assert!(!sections["meter"].is_null());
                if height >= 144.0 {
                    assert!(!sections["routing"].is_null());
                    if track != tracks[2] {
                        assert!(!sections["device"].is_null());
                    }
                } else {
                    assert!(!sections["details"].is_null());
                }
                let mix = Bounds::from(&sections["mix"]);
                let channel = item(&state, "channels", track);
                let mut controls: Vec<(&str, Bounds)> = Vec::new();
                let controls_to_check: &[&str] = if channel["gain_scale"].is_null() {
                    assert_eq!(channel["gain"], channel["gain_readout"]);
                    assert_eq!(channel["pan"], channel["pan_readout"]);
                    &["gain", "pan", "mute", "solo"]
                } else {
                    &["gain", "pan", "gain_readout", "pan_readout", "mute", "solo"]
                };
                for &name in controls_to_check {
                    let rect = Bounds::from(&channel[name]);
                    assert!(
                        mix.contains(rect),
                        "{name} leaves mix section at {width}×{height}: {rect:?}, {mix:?}"
                    );
                    for (other_name, other) in &controls {
                        assert!(
                            !rect.overlaps(*other),
                            "{name} overlaps {other_name} at {width}×{height}"
                        );
                    }
                    controls.push((name, rect));
                }
                assert_eq!(channel["peaks"], json!([0.0, 0.0]));
            }
            if height < 144.0 {
                let details = item(&state, "tracks", tracks[0])["sections"]["details"].clone();
                let opened = click(&mut app, &details);
                assert_eq!(opened["popups"]["properties"], true);
                assert_eq!(opened["selected_track"], tracks[0]);
                assert!(
                    opened["visible_labels"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|label| label == "Gain")
                );
                unchanged(&opened, &original);
                unchanged(&app.key("Escape"), &original);
            }
        }
    }
    app.quit();
}

#[test]
fn channel_sections_route_actions_to_the_correct_track_and_preserve_undo() {
    let mut app = Harness::new();
    let [audio, midi, bus] = fixture(&mut app);
    let original = app.inspect();
    for (control, field) in [("mute", "mute"), ("solo", "solo")] {
        let state = app.inspect();
        let rect = item(&state, "channels", audio)[control].clone();
        let edited = click(&mut app, &rect);
        assert_eq!(model(&edited, audio)[field], true);
        assert_eq!(model(&edited, midi), model(&original, midi));
        assert_eq!(model(&edited, bus), model(&original, bus));
        assert_eq!(edited["history"]["undo"].as_array().unwrap().len(), 1);
        let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
        assert_eq!(undone["project"], original["project"]);
    }
    let state = app.inspect();
    let output = item(&state, "tracks", audio)["sections"]["routing"].clone();
    let opened = click(&mut app, &output);
    unchanged(&opened, &state);
    let occurrence = opened["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|label| *label == "Rhythm bus")
        .count()
        - 1;
    let routed =
        app.command(json!({"command":"click", "label":"Rhythm bus", "occurrence":occurrence}));
    assert_eq!(model(&routed, audio)["output_bus"], bus);
    assert_eq!(model(&routed, midi), model(&original, midi));
    let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
    assert_eq!(undone["project"], original["project"]);

    for (id, popup) in [(audio, "recording"), (midi, "library")] {
        let before = app.inspect();
        let source = item(&before, "tracks", id)["sections"]["device"].clone();
        let opened = click(&mut app, &source);
        assert_eq!(opened["popups"][popup], true);
        unchanged(&opened, &before);
        unchanged(&app.key("Escape"), &before);
    }
    app.quit();
}

#[test]
fn short_track_rows_keep_exact_mix_entry_deliberate_and_undoable() {
    let mut app = Harness::new();
    let [audio, midi, bus] = fixture(&mut app);
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(json!({"command":"viewport", "width":1100, "height":720,
            "pixels_per_point":density}));
        resize(&mut app, 224.0, 56.0);
        for (id, control, value, expected) in [
            (audio, "gain", "-12.34567", 10.0_f64.powf(-12.34567 / 20.0)),
            (midi, "pan", "-23.45678", -0.2345678),
        ] {
            let before = app.inspect();
            let channel = item(&before, "channels", id);
            assert!(channel[format!("{control}_scale")].is_null());
            let target = channel[control].clone();
            let bounds = Bounds::from(&target);
            assert!(bounds.bottom - bounds.top >= 21.0);
            assert_eq!(channel[control], channel[format!("{control}_readout")]);

            let opened = click(&mut app, &target);
            unchanged(&opened, &before);
            // Native popups measure their first frame before painting contents.
            let opened = app.inspect();
            assert!(
                opened["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| label == "Apply value"),
                "{control} should open exact entry at {bounds:?}: {}",
                opened["visible_labels"]
            );
            let drafted = app.command(json!({"command":"text", "text":value}));
            unchanged(&drafted, &before);
            unchanged(&app.click("Cancel"), &before);

            // Exact-entry clicks and automation context menus share this field
            // but must retain independent popup state.
            let (x, y) = bounds.center();
            app.command(
                json!({"command":"pointer", "phase":"click", "button":"secondary", "x":x, "y":y}),
            );
            let context = app.inspect();
            unchanged(&context, &before);
            for action in ["Create automation", "Apply value"] {
                assert!(
                    context["visible_labels"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|label| label == action)
                );
            }
            unchanged(&app.click("Cancel"), &before);

            unchanged(&click(&mut app, &target), &before);
            unchanged(
                &app.command(json!({"command":"text", "text":value})),
                &before,
            );
            let applied = app.click("Apply value");
            let actual = model(&applied, id)[control].as_f64().unwrap();
            assert!(
                (actual - expected).abs() < 0.0000002,
                "The short-row {control} field must apply its full precision: {actual}"
            );
            for other in [audio, midi, bus].into_iter().filter(|track| *track != id) {
                assert_eq!(model(&applied, other), model(&before, other));
            }
            assert_eq!(
                applied["history"]["undo"].as_array().unwrap().len(),
                before["history"]["undo"].as_array().unwrap().len() + 1
            );
            let undone = app.command(json!({"command":"key", "key":"Z", "cmd":true}));
            assert_eq!(undone["project"], before["project"]);
            assert_eq!(undone["modified"], before["modified"]);
        }
    }
    app.quit();
}
