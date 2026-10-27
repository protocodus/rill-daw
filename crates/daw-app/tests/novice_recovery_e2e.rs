//! Mouse-accessible recovery without changing the composition.
mod common;

use common::Harness;
use daw_core::AudioData;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

#[test]
fn unassigned_clip_offers_instrument_setup_without_leaving_the_editor_first() {
    let mut app = Harness::new();
    app.click("Track");
    app.click("Add MIDI track");
    app.click("Window");
    let before = app.click("Clip Editor");
    assert!(visible(&before, "Choose instrument…"));
    let opened = app.click("Choose instrument…");
    assert_eq!(opened["popups"]["library"], true);
    assert!(visible(&opened, "Assign instrument"));
    app.click("Window");
    let restored = app.click("Clip Editor");
    assert_eq!(restored["view"], "Clip");
    assert_eq!(restored["project"], before["project"]);
    assert_eq!(restored["history"], before["history"]);
    assert_eq!(restored["selected_clip"], before["selected_clip"]);
    app.quit();
}

#[test]
#[cfg(target_os = "linux")]
fn disconnected_output_links_to_audio_setup_and_offline_export_still_works() {
    let mut app = Harness::with_missing_output();
    let source = app.directory.path().join("Phrase.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.125; 4800],
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop","paths":[source],"beat":0.0}));
    let before = app.inspect();
    app.click("Play");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if state["error"] == true && visible(&state, "Audio setup…") {
            break;
        }
        assert!(Instant::now() < deadline, "missing audio recovery: {state}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let setup = app.click("Audio setup…");
    assert_eq!(setup["popups"]["setup"], true);
    assert!(visible(&setup, "Output"));
    let restored = app.click("Cancel");
    assert_eq!(restored["project"], before["project"]);
    assert_eq!(restored["history"], before["history"]);
    assert_eq!(restored["device_open"], false);

    let destination = app.directory.path().join("offline.wav");
    app.command(json!({"command":"export","path":destination}));
    app.command(json!({"command":"wait_export"}));
    let audio = AudioData::load_wav(&destination).unwrap();
    assert!(audio.samples.iter().all(|sample| sample.is_finite()));
    assert!(audio.samples.iter().any(|sample| sample.abs() > 0.01));
    app.quit();
}

#[test]
fn unrelated_export_error_does_not_offer_audio_setup_from_report_metadata() {
    let mut app = Harness::new();
    let destination = app.directory.path().join("directory.wav");
    std::fs::create_dir(&destination).unwrap();
    app.raw_command(json!({"command":"export","path":destination}));
    app.raw_command(json!({"command":"wait_export"}));
    let failed = app.inspect();
    assert_eq!(failed["error"], true);
    assert!(!visible(&failed, "Audio setup…"));
    assert!(visible(&failed, "Details"));
    app.quit();
}
