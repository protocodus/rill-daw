//! Integrated instruments and effects explain preset actions before a click.
mod common;

use common::{Harness, wait_visible};
use daw_core::{Clip, Effect, EffectSlot, Instrument, Note, Project};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn fixture(effect: bool) -> Harness {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Device tooltip channel");
    let channel = project.track_mut(track).unwrap();
    channel.instrument = Instrument::builtin("r1-mini");
    channel.clips.push(Clip::midi(
        "Device tooltip phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 90)],
    ));
    channel.inserts.push(EffectSlot::new(
        "R1 Filter",
        Effect::Builtin {
            device: "r1-filter".into(),
        },
    ));
    let path = app.directory.path().join("device-toolbar-tooltips.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.command(json!({"command":"viewport", "width":1480, "height":1000,
        "pixels_per_point":if effect { 2.0 } else { 1.0 }}));
    app.command(json!({"command":"system_theme", "dark":effect}));
    app.click("Device tooltip phrase");
    app.click("Window");
    app.click(if effect { "Effects" } else { "Instrument" });
    if effect {
        app.click("Open editor");
    }
    wait_visible(&mut app, "PRESET");
    app
}

fn unchanged(state: &Value, before: &Value, action: &str) {
    for field in [
        "project",
        "history",
        "modified",
        "selected_track",
        "selected_clip",
        "selected_notes",
        "view",
        "popups",
    ] {
        assert_eq!(
            state[field], before[field],
            "Hovering {action} changed {field}"
        );
    }
}

fn explains(text: &str, fragments: &[&str]) -> bool {
    // Painted glyph rows include line breaks at wrapping points; compare the
    // same prose regardless of where this device's tooltip wraps.
    let text = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    fragments
        .iter()
        .all(|fragment| text.contains(&fragment.to_lowercase()))
}

fn hover(app: &mut Harness, action: &str, fragments: &[&str]) -> Value {
    let state = app.inspect();
    app.command(json!({"command":"pointer", "phase":"move",
        "x":state["viewport"][0].as_f64().unwrap()-2.0,
        "y":state["viewport"][1].as_f64().unwrap()-2.0}));
    let before = app.inspect();
    // Icon-only preset arrows publish their accessible names and actual hit
    // rectangles in the same label diagnostics as the visible text controls.
    let rect = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["text"] == action)
        .min_by(|a, b| {
            a["min"][1]
                .as_f64()
                .unwrap()
                .total_cmp(&b["min"][1].as_f64().unwrap())
        })
        .unwrap_or_else(|| panic!("Missing {action}: {}", before["visible_labels"]));
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let mut last = app.command(json!({"command":"pointer", "phase":"move", "x":x, "y":y}));
    unchanged(&last, &before, action);
    // Advance ordinary frames through the real hover delay, including disabled
    // Save. The test does not inject a tooltip or override its timing.
    for _ in 0..90 {
        last = app.command(json!({"command":"inspect", "paint":true}));
        unchanged(&last, &before, action);
        let visible = last["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .any(|text| text != action && explains(text, fragments));
        let painted = last["painted_text"].as_array().unwrap().iter().any(|text| {
            // Require one actual tooltip text shape to contain both the
            // explanation and its rendered glyphs. Unrelated page labels
            // cannot collectively satisfy the expected fragments.
            text["text"]
                .as_str()
                .is_some_and(|text| text != action && explains(text, fragments))
                && text["rendered_text"]
                    .as_str()
                    .is_some_and(|text| text != action && explains(text, fragments))
        });
        if visible && painted {
            return last;
        }
    }
    panic!(
        "{action} never painted an explanation containing {fragments:?}: {}",
        last["visible_labels"]
    );
}

fn preset_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if root.is_dir() {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(preset_files(&path));
            } else if path
                .extension()
                .is_some_and(|extension| extension == "rillpreset")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn check_device(effect: bool) {
    let mut app = fixture(effect);
    let initial = app.inspect();
    let presets = app.directory.path().join("config/presets");
    let saved_before = preset_files(&presets);
    for (action, fragments) in [
        (
            "Previous preset",
            &["load", "previous", "parameter values"][..],
        ),
        ("Custom", &["choose", "factory or saved preset", "replace"]),
        ("Next preset", &["load", "next", "parameter values"]),
        (
            "Save as…",
            &[
                "name and save",
                "current parameter values",
                "reusable preset",
            ],
        ),
        ("Reset", &["every parameter", "default"]),
        (
            "1%",
            &["random variation", "1%", "output, volume", "unchanged"],
        ),
        (
            "10%",
            &["random variation", "10%", "output, volume", "unchanged"],
        ),
        (
            "100%",
            &["random variation", "100%", "output, volume", "unchanged"],
        ),
    ] {
        let shown = hover(&mut app, action, fragments);
        unchanged(&shown, &initial, action);
        assert_eq!(
            preset_files(&presets),
            saved_before,
            "Hover must not save a preset"
        );
    }

    let naming = app.click("Save as…");
    unchanged(&naming, &initial, "Save as… opens naming");
    hover(
        &mut app,
        "Preset name",
        &["name this preset", "preset chooser"],
    );
    hover(&mut app, "Save", &["enter a preset name", "before saving"]);
    hover(
        &mut app,
        "Cancel",
        &["without saving", "keep the current sound"],
    );
    let cancelled = app.click("Cancel");
    unchanged(&cancelled, &initial, "Cancel preset naming");
    assert!(
        cancelled["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Save as…")
    );
    assert_eq!(
        preset_files(&presets),
        saved_before,
        "Hovering Save and cancelling create no preset files"
    );
    app.quit();
}

#[test]
fn integrated_instrument_preset_toolbar_explains_actions_without_changing_sound() {
    check_device(false);
}

#[test]
fn integrated_effect_preset_toolbar_explains_actions_without_changing_sound() {
    check_device(true);
}
