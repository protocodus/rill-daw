//! Long user paths must never displace settings navigation or commit controls.
mod common;

use common::Harness;
use serde_json::{Value, json};

fn visible_rect<'a>(state: &'a Value, label: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rect| rect["text"] == label)
        .unwrap_or_else(|| panic!("Missing {label}: {}", state["visible_labels"]))
}

fn accessible(state: &Value, labels: &[&str]) {
    let width = state["viewport"][0].as_f64().unwrap();
    let height = state["viewport"][1].as_f64().unwrap();
    for label in labels {
        let rect = visible_rect(state, label);
        assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{label}: {rect}");
        assert!(rect["max"][0].as_f64().unwrap() <= width, "{label}: {rect}");
        assert!(rect["min"][1].as_f64().unwrap() >= 120.0, "{label}: {rect}");
        assert!(
            rect["max"][1].as_f64().unwrap() <= height - f64::from(daw_ui::tokens::POPUP_MARGIN),
            "{label}: {rect}"
        );
    }
}

fn setup(app: &mut Harness) {
    app.click("Window");
    app.click("Setup");
}

fn add_folder(app: &mut Harness, folder: &std::path::Path) -> Value {
    app.click("Plug-ins");
    app.click("Folder path");
    app.command(json!({"command":"text","text":folder.to_string_lossy()}));
    app.click("Add folder")
}

#[test]
fn long_unicode_scan_paths_keep_tabs_actions_and_aligned_audio_settings_in_view() {
    let mut app = Harness::new();
    let original = app.inspect()["settings"].clone();
    let mut folder = app.directory.path().to_path_buf();
    for index in 0..3 {
        folder.push(format!(
            "Instrument collection 東京 🎹 {index} warm expressive sounds"
        ));
    }
    std::fs::create_dir_all(&folder).unwrap();
    for (density, dark) in [(1.0, false), (2.0, true)] {
        app.command(
            json!({"command":"viewport","width":1100,"height":720,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        setup(&mut app);
        let added = add_folder(&mut app, &folder);
        assert_eq!(added["setup_draft"]["vst_dirs"], json!([folder]));
        assert_eq!(added["settings"], original);
        accessible(
            &added,
            &[
                "Audio",
                "Appearance",
                "Storage",
                "Plug-ins",
                "Remove",
                "Add folder",
                "Browse…",
                "Apply",
                "Cancel",
            ],
        );
        let audio = app.click("Audio");
        accessible(
            &audio,
            &[
                "System default",
                "48000 Hz",
                "High quality",
                "Apply",
                "Cancel",
            ],
        );
        let label_x = visible_rect(&audio, "Output")["min"][0].as_f64().unwrap();
        for label in ["Sample rate", "Buffer", "Sample conversion"] {
            assert!(
                (visible_rect(&audio, label)["min"][0].as_f64().unwrap() - label_x).abs() < 1.0
            );
        }
        let control_x = visible_rect(&audio, "System default")["min"][0]
            .as_f64()
            .unwrap();
        for value in ["48000 Hz", "256 samples", "High quality"] {
            assert!(
                (visible_rect(&audio, value)["min"][0].as_f64().unwrap() - control_x).abs() < 1.0
            );
        }
        assert!(label_x < control_x);
        accessible(&app.click("Appearance"), &["System", "Apply", "Cancel"]);
        accessible(
            &app.click("Storage"),
            &["Choose…", "Recover session", "Apply", "Cancel"],
        );
        app.click("Plug-ins");
        let removed = app.click("Remove");
        assert_eq!(removed["setup_draft"]["vst_dirs"], json!([]));
        add_folder(&mut app, &folder);
        let cancelled = app.click("Cancel");
        assert_eq!(cancelled["settings"], original);
        assert_eq!(cancelled["popups"]["setup"], false);
    }

    setup(&mut app);
    add_folder(&mut app, &folder);
    let applied = app.click("Apply");
    assert_eq!(applied["settings"]["vst_dirs"], json!([folder]));
    setup(&mut app);
    let recalled = app.click("Plug-ins");
    accessible(&recalled, &["Remove", "Apply", "Cancel"]);
    app.click("Remove");
    assert_eq!(app.click("Apply")["settings"]["vst_dirs"], json!([]));
    app.quit();
}

#[test]
fn many_long_scan_folders_keep_apply_and_cancel_visible_without_scrolling() {
    let mut app = Harness::new();
    let original = app.inspect()["settings"].clone();
    let folders: Vec<_> = (0..8)
        .map(|index| {
            let folder = app
                .directory
                .path()
                .join("Instrument collection 東京 🎹 warm expressive sounds")
                .join("Third party plug-ins with extended library names")
                .join(format!("Additional instruments 日本語 {index}"));
            std::fs::create_dir_all(&folder).unwrap();
            folder
        })
        .collect();
    for (width, height, density, dark) in [
        (800, 600, 1.0, false),
        (800, 600, 1.25, false),
        (1440, 1000, 2.0, true),
    ] {
        app.command(
            json!({"command":"viewport","width":width,"height":height,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        setup(&mut app);
        for (index, folder) in folders.iter().enumerate() {
            let added = add_folder(&mut app, folder);
            assert_eq!(added["setup_draft"]["vst_dirs"], json!(&folders[..=index]));
            accessible(
                &added,
                &[
                    "Audio",
                    "Appearance",
                    "Storage",
                    "Plug-ins",
                    "Remove",
                    "Add folder",
                    "Browse…",
                    "Apply",
                    "Cancel",
                ],
            );
        }
        if dark {
            let applied = app.click("Apply");
            assert_eq!(applied["settings"]["vst_dirs"], json!(folders));
            setup(&mut app);
            accessible(&app.click("Plug-ins"), &["Remove", "Apply", "Cancel"]);
            app.click("Cancel");
        } else {
            let cancelled = app.click("Cancel");
            assert_eq!(cancelled["settings"], original);
            assert_eq!(cancelled["popups"]["setup"], false);
        }
    }
    app.quit();
}
