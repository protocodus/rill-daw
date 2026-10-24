//! Export completion is visible, non-modal, and distinct from failure.
mod common;
use common::Harness;
use daw_core::AudioData;
use serde_json::{Value, json};

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

fn seed(app: &mut Harness) {
    let source = app.directory.path().join("Phrase.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: (0..4800).map(|i| (i as f32 * 0.057).sin() * 0.2).collect(),
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop","paths":[source],"beat":0.0}));
}

#[test]
fn export_notice_stays_visible_copies_full_path_and_dismisses_without_editing() {
    for dark in [false, true] {
        let mut app = Harness::new();
        app.command(json!({"command":"viewport","width":1100,"height":720,"pixels_per_point":if dark {2.0} else {1.0}}));
        app.command(json!({"command":"system_theme","dark":dark}));
        seed(&mut app);
        let before = app.inspect();
        let directory = app
            .directory
            .path()
            .join("Exports 🎹/Long descriptive collection name/Final mixes with source notes");
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join(format!("{} mix.wav", "Synthpop 🎹 ".repeat(9)));
        app.command(json!({"command":"export","path":output}));
        app.command(json!({"command":"wait_export"}));
        let notice = format!(
            "Exported · {}",
            output.file_name().unwrap().to_string_lossy()
        );
        let state = app.inspect();
        assert_eq!(state["exported_file"], json!(output));
        assert!(has(&state, &notice));
        for label in ["Copy path", "Dismiss"] {
            let rect = state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == label)
                .unwrap();
            assert!(rect["min"][0].as_f64().unwrap() >= 0.0);
            assert!(rect["max"][0].as_f64().unwrap() <= 1100.0);
            assert!(rect["max"][1].as_f64().unwrap() <= 720.0);
        }
        let audio = AudioData::load_wav(&output).unwrap();
        assert_eq!(audio.channels, 2);
        assert!(audio.samples.iter().any(|sample| sample.abs() > 0.01));
        let copied = app.click("Copy path");
        assert_eq!(copied["clipboard_text"], output.to_string_lossy().as_ref());
        // Copy remains usable after another application changes the clipboard.
        assert!(has(&copied, "Copy path"));
        app.click("Window");
        let mixer = app.click("Mixer");
        assert_eq!(mixer["view"], "Mixer");
        assert!(
            has(&mixer, &notice),
            "completion must not block or disappear on navigation"
        );
        let dismissed = app.click("Dismiss");
        assert!(dismissed["exported_file"].is_null());
        assert!(!has(&dismissed, "Copy path"));
        assert_eq!(dismissed["project"], before["project"]);
        assert_eq!(dismissed["history"], before["history"]);
        app.quit();
    }
}

#[test]
fn failed_export_replaces_old_completion_and_retry_shows_the_new_file() {
    let mut app = Harness::new();
    seed(&mut app);
    let first = app.directory.path().join("First.wav");
    app.command(json!({"command":"export","path":first}));
    app.command(json!({"command":"wait_export"}));
    assert!(has(&app.inspect(), "Exported · First.wav"));

    // An existing directory is an invalid WAV destination on every platform.
    let invalid = app.directory.path().join("Cannot be a wave file");
    std::fs::create_dir(&invalid).unwrap();
    app.raw_command(json!({"command":"export","path":invalid}));
    app.raw_command(json!({"command":"wait_export"}));
    let failed = app.inspect();
    assert_eq!(failed["error"], true);
    assert!(failed["exported_file"].is_null());
    assert!(!has(&failed, "Copy path"));
    assert!(!has(&failed, "Exported · First.wav"));
    let dismissed = app.click("Dismiss");
    assert_eq!(dismissed["error"], false);

    let retry = app.directory.path().join("Retry.wav");
    app.command(json!({"command":"export","path":retry}));
    app.command(json!({"command":"wait_export"}));
    let finished = app.inspect();
    assert_eq!(finished["error"], false);
    assert!(has(&finished, "Exported · Retry.wav"));
    assert_eq!(finished["exported_file"], json!(retry));
    app.click("File");
    app.click("New");
    let new = app.click("Don't save");
    assert!(new["exported_file"].is_null());
    assert!(!has(&new, "Copy path"));
    app.quit();
}
