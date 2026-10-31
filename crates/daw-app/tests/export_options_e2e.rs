//! Export length is chosen through the real popup before the native destination picker.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Instrument, Note, Project};
use serde_json::{Value, json};
use std::path::Path;

fn options(app: &mut Harness) -> Value {
    app.click("File");
    let state = app.click("Export WAV…");
    assert_eq!(state["popups"]["export"], true);
    state
}

fn picker(app: &mut Harness, path: Option<&Path>) {
    app.command(json!({"command":"file_dialog", "kind":"export", "path":path}));
}

fn finish(app: &mut Harness, path: &Path) -> AudioData {
    picker(app, Some(path));
    let state = app.click("Export…");
    assert_eq!(state["popups"]["export"], false);
    app.command(json!({"command":"wait_export"}));
    assert_eq!(app.inspect()["exported_file"], json!(path));
    AudioData::load_wav(path).unwrap()
}

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

#[test]
fn minute_export_can_exclude_tails_and_cancellation_preserves_the_editor_and_project() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Hit.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: (0..4800).map(|i| (i as f32 * 0.07).sin() * 0.2).collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Minute arrangement");
    for (name, start, length) in [("Opening", 0.0, 4.0), ("Closing", 119.8, 0.2)] {
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            name,
            start,
            length,
            source.to_string_lossy().into_owned(),
        ));
    }
    let path = app.directory.path().join("Minute.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    app.click("Opening");
    app.click("Window");
    let before = app.click("Clip Editor");

    let state = options(&mut app);
    assert_eq!(state["export_include_effect_tails"], true);
    assert!(has(&state, "Arrangement · 1:00"));
    let full_path = app.directory.path().join("With tails.wav");
    let full = finish(&mut app, &full_path);
    assert_eq!(full.frames(), 48_000 * 60 + 48_000 / 5);
    assert_eq!(app.inspect()["view"], "Clip");

    // The same arrangement exports to exactly one minute when tails are off.
    options(&mut app);
    let state = app.click("Include effect tails");
    assert_eq!(state["export_include_effect_tails"], false);
    // Background clip shortcuts cannot edit through the export window.
    app.key("Delete");
    app.command(json!({"command":"key", "key":"D", "cmd":true}));
    let exact_path = app.directory.path().join("Exact minute.wav");
    let exact = finish(&mut app, &exact_path);
    assert_eq!(exact.frames(), 48_000 * 60);
    assert_eq!(exact.channels, 2);
    assert_eq!(exact.sample_rate, 48_000);
    assert!(exact.samples.iter().all(|sample| sample.is_finite()));
    assert!(
        exact.samples[exact.samples.len() - 4800..]
            .iter()
            .any(|sample| sample.abs() > 0.01)
    );

    // Cancel and Escape never consume the queued picker result or write a file.
    let cancelled_path = app.directory.path().join("Cancelled.wav");
    for dismiss in ["Cancel", "Escape"] {
        picker(&mut app, Some(&cancelled_path));
        options(&mut app);
        let state = if dismiss == "Escape" {
            app.key("Escape")
        } else {
            app.click(dismiss)
        };
        assert_eq!(state["popups"]["export"], false);
        assert_eq!(state["file_dialog_pending"], true);
        assert_eq!(state["export_pending"], false);
        assert_eq!(state["view"], "Clip");
        assert!(!cancelled_path.exists());
    }
    // Cancelling the native picker also preserves the previous completed export.
    picker(&mut app, None);
    options(&mut app);
    let cancelled = app.click("Export…");
    assert_eq!(cancelled["file_dialog_pending"], false);
    assert_eq!(cancelled["export_pending"], false);
    assert_eq!(cancelled["exported_file"], json!(exact_path));
    assert_eq!(cancelled["view"], "Clip");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    assert_eq!(cancelled["selected_clip"], before["selected_clip"]);
    app.quit();
}

#[test]
#[ignore = "requires a real instrument with a reported tail in RILL_TEST_VST_INSTRUMENT (Surge XT)"]
fn native_instrument_export_choice_keeps_or_trims_the_real_plugin_tail() {
    let plugin = std::env::var("RILL_TEST_VST_INSTRUMENT").unwrap();
    let probe = daw_plugins::VstInstrument::load(Path::new(&plugin), 48_000.0, 256).unwrap();
    let tail = (probe.tail_samples() as usize).min(48_000 * 30);
    assert!(
        tail > 48_000 / 5,
        "fixture must report an effect/release tail"
    );
    drop(probe);
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Native synth");
    let track = project.track_mut(track).unwrap();
    track.instrument = Instrument::Vst3 {
        path: plugin,
        class_id: None,
    };
    track.clips.push(Clip::midi(
        "Phrase",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("Native.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    options(&mut app);
    let full_path = app.directory.path().join("Native tails.wav");
    let full = finish(&mut app, &full_path);
    assert_eq!(full.frames(), 24_000 + tail);
    assert!(
        full.samples[48_000..]
            .iter()
            .any(|sample| sample.abs() > 0.0001)
    );
    options(&mut app);
    app.click("Include effect tails");
    let exact_path = app.directory.path().join("Native exact.wav");
    let exact = finish(&mut app, &exact_path);
    assert_eq!(exact.frames(), 24_000);
    assert!(exact.samples.iter().any(|sample| sample.abs() > 0.01));
    assert!(exact.samples.iter().all(|sample| sample.is_finite()));
    app.quit();
}
