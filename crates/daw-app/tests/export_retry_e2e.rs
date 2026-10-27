//! Export retry preserves the rendered snapshot, and document switches wait for an outcome.
mod common;
#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::Harness;
use daw_core::{AudioData, Clip, Instrument, Note, Project};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn start(app: &mut Harness, path: &Path, without_tails: bool) {
    app.command(json!({"command":"file_dialog","kind":"export","path":path}));
    app.click("File");
    app.click("Export WAV…");
    if without_tails {
        app.click("Include effect tails");
    }
    app.click("Export…");
}

fn native_helper() -> PathBuf {
    let directory = std::env::var_os("RILL_E2E_BINARY")
        .map(|p| PathBuf::from(p).parent().unwrap().to_owned())
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_owned()
        });
    directory.join(if cfg!(windows) {
        "daw-plugin-host.exe"
    } else {
        "daw-plugin-host"
    })
}

fn native_song(app: &mut Harness, plugin: &EditorProbeFixture) -> Value {
    let mut project = Project::new();
    let track = project.add_midi_track("Patient instrument");
    project
        .set_instrument(
            track,
            Instrument::Vst3 {
                path: plugin.path.to_string_lossy().into_owned(),
                class_id: Some(CLASS_ID.into()),
            },
        )
        .unwrap();
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Patient phrase",
        0.0,
        0.25,
        vec![Note::new(60, 0.0, 0.125, 100)],
    ));
    let path = app.directory.path().join("Patient song.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}))
}

#[test]
fn retry_uses_the_failed_mix_destination_and_options_after_later_edits() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..4800).map(|i| (i as f32 * 0.1).sin() * 0.2).collect(),
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop","paths":[source],"beat":0}));
    let project_path = app.directory.path().join("Retry song.rill");
    app.command(json!({"command":"save","path":project_path}));
    let parent = app.directory.path().join("Exports not created yet");
    let output = parent.join("Retry song.wav");
    start(&mut app, &output, true);
    app.raw_command(json!({"command":"wait_export"}));
    let failed = app.inspect();
    assert!(has(&failed, "Retry export"));
    assert_eq!(failed["export_run"]["include_effect_tails"], false);
    assert_eq!(failed["export_run"]["path"], json!(output));
    assert!(!output.exists());

    // A retained WAV error must not hide another successfully delivered file.
    let midi = app.directory.path().join("Notes.mid");
    app.command(json!({"command":"file_dialog","kind":"export_midi","path":midi}));
    app.click("File");
    let midi_done = app.click("Export MIDI…");
    assert!(has(&midi_done, "Exported · Notes.mid"));
    assert!(has(&midi_done, "Retry export"));
    assert_eq!(
        app.click("Copy path")["clipboard_text"],
        midi.to_string_lossy().as_ref()
    );
    let dismissed = app.click("Dismiss result");
    assert!(has(&dismissed, "Retry export"));
    assert!(!has(&dismissed, "Exported · Notes.mid"));

    // Replace tempo with a mouse gesture and typing, then commit by clicking the ruler.
    let tempo = failed["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["text"].as_str().is_some_and(|s| s.ends_with(" BPM")))
        .unwrap();
    let x = (tempo["min"][0].as_f64().unwrap() + tempo["max"][0].as_f64().unwrap()) / 2.0;
    let y = (tempo["min"][1].as_f64().unwrap() + tempo["max"][1].as_f64().unwrap()) / 2.0;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":"60"}));
    let canvas = &app.inspect()["timeline"]["canvas"];
    let edited = app.command(json!({"command":"pointer","phase":"click",
        "x":canvas["max"][0].as_f64().unwrap()-12.0,
        "y":canvas["max"][1].as_f64().unwrap()-12.0}));
    assert_eq!(edited["project"]["tempo"], 60.0);
    assert!(
        has(&edited, "Retry export"),
        "failure remains visible after editing"
    );

    // Even changing the preference for future exports must not change this retry.
    app.click("File");
    app.click("Export WAV…");
    app.click("Include effect tails");
    app.click("Cancel");
    std::fs::create_dir(&parent).unwrap();
    app.command(json!({"command":"file_dialog","kind":"export","path":null}));
    app.click("Retry export");
    let completed = app.command(json!({"command":"wait_export"}));
    assert_eq!(completed["project"]["tempo"], 60.0);
    assert_eq!(completed["exported_file"], json!(output));
    assert!(completed["export_run"].is_null());
    assert_eq!(
        completed["file_dialog_pending"], true,
        "Retry does not reopen a picker"
    );
    let wav = AudioData::load_wav(output).unwrap();
    assert_eq!(wav.frames(), 4800, "original 120 BPM mix without tails");
    assert!(wav.samples.iter().any(|s| s.abs() > 0.01));
    app.quit();
}

#[test]
fn new_session_waits_for_export_completion_and_shows_the_file_being_rendered() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_millis(800));
    let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &native_helper());
    let original = native_song(&mut app, &plugin);
    let output = app.directory.path().join("Patient mix.wav");
    start(&mut app, &output, true);
    app.click("File");
    let waiting = app.click("New");
    assert_eq!(waiting["document_guard"]["phase"], "waiting");
    assert_eq!(waiting["project"], original["project"]);
    assert!(has(
        &waiting,
        "The export will finish before switching sessions."
    ));
    assert!(
        waiting["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| {
                text.as_str()
                    .is_some_and(|s| s.contains("Exporting · Patient mix.wav"))
            })
    );
    let fresh = app.command(json!({"command":"wait_project","timeout_ms":15000}));
    assert!(fresh["document_guard"].is_null());
    assert!(fresh["project"]["tracks"].as_array().unwrap().is_empty());
    assert!(
        AudioData::load_wav(output)
            .unwrap()
            .samples
            .iter()
            .any(|s| s.abs() > 0.01)
    );
    app.quit();
}

#[test]
fn failed_export_stops_a_queued_document_switch_and_can_return_to_retry() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_millis(1200));
    let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &native_helper());
    native_song(&mut app, &plugin);
    let output = app.directory.path().join("Destination.wav");
    std::fs::create_dir(&output).unwrap();
    start(&mut app, &output, true);
    app.click("Track");
    let edited = app.click("Add audio track");
    assert_eq!(edited["modified"], true);
    app.click("File");
    app.click("New");
    app.raw_command(json!({"command":"wait_project","timeout_ms":15000}));
    let stopped = app.inspect();
    assert_eq!(stopped["document_guard"]["phase"], "export_failed");
    assert_eq!(stopped["project"], edited["project"]);
    assert!(has(&stopped, "Continue"));
    assert!(has(&stopped, "Cancel"));
    app.click("Continue");
    app.raw_command(json!({"command":"wait_project","timeout_ms":15000}));
    let confirmation = app.inspect();
    assert_eq!(confirmation["document_guard"]["phase"], "confirm");
    assert!(has(&confirmation, "Save changes?"));
    let returned = app.click("Cancel");
    assert_eq!(returned["project"], edited["project"]);
    assert!(returned["document_guard"].is_null());
    assert!(has(&returned, "Retry export"));
    std::fs::remove_dir(&output).unwrap();
    app.click("Retry export");
    let succeeded = app.command(json!({"command":"wait_export","timeout_ms":15000}));
    assert_eq!(succeeded["project"], edited["project"]);
    assert_eq!(succeeded["exported_file"], json!(output));
    app.quit();
}
