//! Utility footers stay reachable as generous spacing meets compact viewports.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, menu: &str, action: &str) -> Value {
    app.click(menu);
    app.click(action)
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text}: {}", state["visible_labels"]))
}

fn visible(state: &Value, labels: &[&str], width: u32, height: u32) {
    for text in labels {
        let bounds = label(state, text);
        for (axis, maximum) in [(0, width), (1, height)] {
            assert!(
                bounds["min"][axis].as_f64().unwrap() >= 0.0,
                "{text}: {bounds}"
            );
            assert!(
                bounds["max"][axis].as_f64().unwrap() <= f64::from(maximum),
                "{text}: {bounds}"
            );
        }
    }
}

fn popup_fits(state: &Value, title: &str, width: u32, height: u32) {
    let title = label(state, title);
    let x = (title["min"][0].as_f64().unwrap() + title["max"][0].as_f64().unwrap()) * 0.5;
    let y = (title["min"][1].as_f64().unwrap() + title["max"][1].as_f64().unwrap()) * 0.5;
    let window = state["window_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|window| {
            window["min"][0].as_f64().unwrap() <= x
                && x <= window["max"][0].as_f64().unwrap()
                && window["min"][1].as_f64().unwrap() <= y
                && y <= window["max"][1].as_f64().unwrap()
        })
        .expect("Popup contains its title");
    assert!(window["min"][0].as_f64().unwrap() >= 0.0, "{window}");
    assert!(window["min"][1].as_f64().unwrap() >= 0.0, "{window}");
    assert!(
        window["max"][0].as_f64().unwrap() <= f64::from(width),
        "{window}"
    );
    assert!(
        window["max"][1].as_f64().unwrap() <= f64::from(height),
        "{window}"
    );
}

#[test]
fn setup_recording_export_warning_history_and_about_keep_their_actions_in_view() {
    let mut app = Harness::new();
    let mut project = Project::new();
    for index in 0..5 {
        let name = if index == 0 {
            "Spacing instrument".to_owned()
        } else {
            format!(
                "Missing expressive instrument {index} · 夜の音楽 🎹 with a long descriptive name"
            )
        };
        let track = project.add_midi_track(name);
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            if index == 0 {
                "Spacing pattern"
            } else {
                "Missing instrument pattern"
            },
            0.0,
            60.0,
            vec![Note::new(60, 0.0, 1.0, 90)],
        ));
    }
    let path = app.directory.path().join("dialog-spacing.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Spacing pattern");
    for dark in [false, true] {
        for (width, height) in [(800, 600), (1440, 1000)] {
            for density in [1.0, 1.25, 2.0] {
                app.command(json!({"command":"viewport", "width":width, "height":height, "pixels_per_point":density}));
                app.command(json!({"command":"system_theme", "dark":dark}));
                menu(&mut app, "Window", "Setup");
                for page in ["Audio", "Appearance", "Storage", "Plug-ins", "AI"] {
                    let state = app.click(page);
                    popup_fits(&state, "Setup", width, height);
                    visible(
                        &state,
                        &[
                            "Audio",
                            "Appearance",
                            "Storage",
                            "Plug-ins",
                            "AI",
                            "Apply",
                            "Cancel",
                        ],
                        width,
                        height,
                    );
                }
                app.click("Cancel");

                let recording = menu(&mut app, "Window", "Recording");
                popup_fits(&recording, "Recording · Spacing instrument", width, height);
                visible(
                    &recording,
                    &["Pre-count", "No pre-count", "Click level", "Record"],
                    width,
                    height,
                );
                app.key("Escape");

                let export = menu(&mut app, "File", "Export WAV…");
                popup_fits(&export, "Export WAV", width, height);
                visible(
                    &export,
                    &[
                        "MIDI notes need an instrument",
                        "Include effect tails",
                        "Export…",
                        "Cancel",
                    ],
                    width,
                    height,
                );
                app.click("Cancel");

                let history = menu(&mut app, "Edit", "History…");
                popup_fits(&history, "History", width, height);
                let metadata = history["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find_map(|label| {
                        label
                            .as_str()
                            .filter(|text| text.starts_with("Up to 128 steps"))
                    })
                    .expect("History storage footer remains visible");
                visible(&history, &[metadata], width, height);
                app.key("Escape");

                let about = menu(&mut app, "Help", "About RILL");
                popup_fits(&about, "About RILL", width, height);
                visible(&about, &["Audio workstation", "Close"], width, height);
                app.click("Close");
            }
        }
    }
    app.quit();
}
