//! Appearance is applied through the real Setup controls and survives a process restart.
mod common;
use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn setup(app: &mut Harness) {
    app.click("Window");
    app.click("Setup");
    app.click("Appearance");
}
fn choose(app: &mut Harness, theme: &str) {
    let current = app.inspect()["setup_draft"]["appearance"]
        .as_str()
        .unwrap()
        .to_owned();
    app.click(&current);
    app.click(theme);
}
fn luma(rgb: &Value) -> f64 {
    rgb[0].as_f64().unwrap() * 0.2126
        + rgb[1].as_f64().unwrap() * 0.7152
        + rgb[2].as_f64().unwrap() * 0.0722
}
#[test]
fn appearance_applies_cancels_persists_and_system_follows_live_os_events() {
    let mut app = Harness::new();
    assert_eq!(app.inspect()["settings"]["appearance"], "System");
    let dark = app.command(json!({"command":"system_theme","dark":true}));
    assert_eq!(dark["effective_theme"], "Dark");
    let light = app.command(json!({"command":"system_theme","dark":false}));
    assert_eq!(light["effective_theme"], "Light");
    assert!(
        luma(&light["theme_colors"]["background"])
            > luma(&dark["theme_colors"]["background"]) + 150.0
    );
    setup(&mut app);
    choose(&mut app, "Dark");
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["settings"]["appearance"], "System");
    assert_eq!(cancelled["effective_theme"], "Light");
    setup(&mut app);
    choose(&mut app, "Dark");
    let saved = app.click("Apply");
    assert_eq!(saved["effective_theme"], "Dark");
    assert!(!saved["device_open"].as_bool().unwrap());
    let config = app.directory.path().join("config/settings.json");
    let persisted: Value = serde_json::from_slice(&std::fs::read(config).unwrap()).unwrap();
    assert_eq!(persisted["appearance"], "Dark");
    app = app.restart();
    assert_eq!(app.inspect()["settings"]["appearance"], "Dark");
    assert_eq!(
        app.command(json!({"command":"system_theme","dark":false}))["effective_theme"],
        "Dark"
    );
    setup(&mut app);
    choose(&mut app, "Light");
    assert_eq!(app.click("Apply")["effective_theme"], "Light");
    assert_eq!(
        app.command(json!({"command":"system_theme","dark":true}))["effective_theme"],
        "Light"
    );
    setup(&mut app);
    choose(&mut app, "System");
    assert_eq!(app.click("Apply")["effective_theme"], "Dark");
    assert_eq!(
        app.command(json!({"command":"system_theme","dark":false}))["effective_theme"],
        "Light"
    );
    app.quit();
}

#[test]
fn piano_rows_preserve_black_white_contrast_with_blue_scale_and_chord_tints_in_both_modes() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Theme piano");
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Color test",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("colors.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Color test");
    app.click("Window");
    app.click("Clip Editor");
    for dark in [true, false, true] {
        if !dark {
            app.click("C · Natural minor");
            app.click("C");
            app.click("C♯");
            app.click("C♯ · Natural minor");
            app.click("Major triad");
            app.click("Minor triad");
        }
        let state = app.command(json!({"command":"system_theme","dark":dark}));
        let keys = state["midi_editor"]["keys"].as_array().unwrap();
        let black: Vec<_> = keys.iter().filter(|key| key["black"] == true).collect();
        let white: Vec<_> = keys.iter().filter(|key| key["black"] == false).collect();
        assert!(!black.is_empty() && !white.is_empty());
        for black in &black {
            for white in &white {
                assert!(
                    luma(&black["row_color"]) + 8.0 < luma(&white["row_color"]),
                    "{black} must stay darker than {white}"
                );
                assert!(
                    luma(&black["outside_color"]) + 6.0 < luma(&white["outside_color"]),
                    "Pitch contrast must survive the outside-clip tint"
                );
            }
        }
        assert!(keys.iter().any(|key| key["chord"] == true));
        if !dark {
            for pitch in [61, 64, 68] {
                assert!(
                    keys.iter()
                        .any(|key| key["pitch"] == pitch && key["chord"] == true)
                );
            }
        }
        assert!(
            keys.iter()
                .any(|key| key["scale"] == true && key["chord"] == false)
        );
        let tail = &state["midi_editor"]["outside_clip"];
        let plot = &state["midi_editor"]["plot"];
        assert!(tail["min"][0].as_f64().unwrap() > plot["min"][0].as_f64().unwrap());
        assert_eq!(tail["max"], plot["max"]);
        for key in keys {
            let original = luma(&key["row_color"]);
            let outside = luma(&key["outside_color"]);
            assert!(
                (original - outside).abs() < 25.0,
                "Clip tint must remain subtle"
            );
            if !dark {
                assert!(
                    outside >= original - 1.0,
                    "Light mode must not darken into a gray block"
                );
            }
        }
        assert!(
            keys.iter().any(|key| {
                (luma(&key["outside_color"]) - luma(&key["row_color"])).abs() >= 3.0
            }),
            "Outside-clip rows must remain visually distinct"
        );
        for key in keys
            .iter()
            .filter(|key| key["scale"] == true || key["chord"] == true)
        {
            let color = &key["row_color"];
            assert!(
                color[2].as_u64().unwrap() > color[0].as_u64().unwrap() + 10,
                "highlight is blue: {key}"
            );
        }
    }
    app.quit();
}
