//! Extension normalization cannot bypass a native picker's overwrite decision.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};
use std::path::Path;

fn menu(app: &mut Harness, group: &str, item: &str) -> Value {
    app.click(group);
    app.click(item)
}

fn seed(app: &mut Harness) -> Value {
    let source = app.directory.path().join("Source.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: vec![0.1; 4800],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio to preserve");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Audio phrase",
        0.0,
        0.2,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("MIDI to preserve");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "MIDI phrase",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 0.5, 100)],
    ));
    let path = app.directory.path().join("Original.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"file_dialog","kind":"open","path":path}));
    menu(app, "File", "Open…");
    app.command(json!({"command":"wait_project"}));
    menu(app, "Track", "Add audio track")
}

fn choose(app: &mut Harness, kind: &str, chosen: &Path, replace: Option<bool>) {
    let mut request = json!({"command":"file_dialog","kind":kind,"path":chosen});
    if let Some(replace) = replace {
        request["replace_existing"] = json!(replace);
    }
    app.command(request);
}

fn invoke(app: &mut Harness, kind: &str) -> Value {
    match kind {
        "save" => {
            menu(app, "File", "Save as…");
            app.command(json!({"command":"wait_project"}))
        }
        "export" => {
            menu(app, "File", "Export WAV…");
            app.click("Export…")
        }
        "export_midi" => menu(app, "File", "Export MIDI…"),
        _ => unreachable!(),
    }
}

fn assert_content(path: &Path, kind: &str) {
    match kind {
        "save" => assert_eq!(Project::load(path).unwrap().tracks.len(), 3),
        "export" => assert!(
            AudioData::load_wav(path)
                .unwrap()
                .samples
                .iter()
                .any(|sample| sample.abs() > 0.01)
        ),
        "export_midi" => {
            let mut project = Project::new();
            project.import_midi(path, 0.0).unwrap();
            assert!(project.tracks.iter().flat_map(|track| &track.clips).any(
                |clip| matches!(&clip.content, daw_core::ClipContent::Midi { notes } if !notes.is_empty())
            ));
        }
        _ => unreachable!(),
    }
}

fn replacement_case(kind: &str, extension: &str) {
    let mut app = Harness::new();
    let before = seed(&mut app);
    let chosen = app.directory.path().join("Chosen name 音楽");
    let normalized = chosen.with_extension(extension);
    let sentinel = b"Existing normalized destination must survive cancellation";
    let bare = b"The extensionless entry is not the output destination";
    std::fs::write(&chosen, bare).unwrap();
    std::fs::write(&normalized, sentinel).unwrap();

    // The normal hook path cannot silently approve an overwrite merely by
    // returning an extensionless name, nor can an explicit rejection.
    for replacement in [None, Some(false)] {
        choose(&mut app, kind, &chosen, replacement);
        let cancelled = invoke(&mut app, kind);
        assert_eq!(std::fs::read(&normalized).unwrap(), sentinel);
        assert_eq!(std::fs::read(&chosen).unwrap(), bare);
        assert_eq!(cancelled["project"], before["project"]);
        assert_eq!(cancelled["history"], before["history"]);
        assert_eq!(cancelled["project_path"], before["project_path"]);
        assert_eq!(cancelled["modified"], before["modified"]);
        assert_eq!(cancelled["export_pending"], false);
    }

    choose(&mut app, kind, &chosen, Some(true));
    let accepted = invoke(&mut app, kind);
    if kind == "export" {
        app.command(json!({"command":"wait_export"}));
    }
    assert_ne!(std::fs::read(&normalized).unwrap(), sentinel);
    assert_eq!(std::fs::read(&chosen).unwrap(), bare);
    assert_content(&normalized, kind);
    if kind == "save" {
        assert_eq!(accepted["project_path"], json!(normalized));
    } else {
        assert_eq!(app.inspect()["exported_file"], json!(normalized));
    }

    // A native picker already confirmed this exact recognized-extension path;
    // the post-picker normalization layer must not ask or reject it again.
    let explicit = app.directory.path().join(format!("Explicit.{extension}"));
    std::fs::write(&explicit, sentinel).unwrap();
    choose(&mut app, kind, &explicit, Some(false));
    invoke(&mut app, kind);
    if kind == "export" {
        app.command(json!({"command":"wait_export"}));
    }
    assert_content(&explicit, kind);
    assert_eq!(std::fs::read(&chosen).unwrap(), bare);
    app.quit();
}

#[test]
fn save_normalized_existing_destination_requires_affirmative_replacement() {
    replacement_case("save", "rill");
}

#[test]
fn wav_normalized_existing_destination_requires_affirmative_replacement() {
    replacement_case("export", "wav");
}

#[test]
fn midi_normalized_existing_destination_requires_affirmative_replacement() {
    replacement_case("export_midi", "mid");
}

#[test]
fn declining_normalized_replacement_keeps_the_unsaved_document_guard_and_current_song() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Unsaved source.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: vec![0.1; 4800],
    }
    .write_wav(&source)
    .unwrap();
    let before = app.command(json!({"command":"drop","paths":[source],"beat":0.0}));
    let chosen = app.directory.path().join("Protected song");
    let normalized = chosen.with_extension("rill");
    let sentinel = b"Existing project to preserve";
    std::fs::write(&normalized, sentinel).unwrap();
    let pending = menu(&mut app, "File", "New");
    assert_eq!(pending["document_guard"]["phase"], "confirm");
    choose(&mut app, "save", &chosen, Some(false));
    let cancelled = app.click("Save");
    assert_eq!(cancelled["document_guard"]["phase"], "confirm");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["history"], before["history"]);
    assert!(cancelled["project_path"].is_null());
    assert_eq!(std::fs::read(&normalized).unwrap(), sentinel);

    choose(&mut app, "save", &chosen, Some(true));
    app.click("Save");
    let fresh = app.command(json!({"command":"wait_project"}));
    assert!(fresh["document_guard"].is_null());
    assert!(fresh["project"]["tracks"].as_array().unwrap().is_empty());
    assert_eq!(Project::load(&normalized).unwrap().tracks.len(), 1);
    app.quit();
}

#[test]
fn raw_and_normalized_source_aliases_are_rejected_before_replacement_approval() {
    for (kind, extension) in [("save", "rill"), ("export", "wav"), ("export_midi", "mid")] {
        let mut app = Harness::new();
        let before = seed(&mut app);
        let source = Path::new(
            before["project"]["tracks"][0]["clips"][0]["content"]["Audio"]["path"]
                .as_str()
                .unwrap(),
        );
        let bytes = std::fs::read(source).unwrap();
        let chosen = app.directory.path().join("Normalized source alias");
        let normalized = chosen.with_extension(extension);
        std::fs::hard_link(source, &normalized).unwrap();
        for destination in [source, chosen.as_path()] {
            choose(&mut app, kind, destination, Some(true));
            // Invoke without waiting through a source-protection error: no
            // worker or editor-state barrier should have been dispatched.
            match kind {
                "save" => {
                    menu(&mut app, "File", "Save as…");
                }
                "export" => {
                    menu(&mut app, "File", "Export WAV…");
                    app.click("Export…");
                }
                "export_midi" => {
                    menu(&mut app, "File", "Export MIDI…");
                }
                _ => unreachable!(),
            }
            let failed = app.inspect();
            assert_eq!(failed["error"], true, "{failed}");
            assert!(
                failed["status"]
                    .as_str()
                    .unwrap()
                    .contains("Choose a different output file")
            );
            assert_eq!(failed["project"], before["project"]);
            assert_eq!(failed["history"], before["history"]);
            assert_eq!(failed["project_path"], before["project_path"]);
            assert_eq!(failed["export_pending"], false);
            assert_eq!(std::fs::read(source).unwrap(), bytes);
            assert_eq!(std::fs::read(&normalized).unwrap(), bytes);
            assert!(!chosen.exists());
            app.click("Dismiss");
        }
        app.quit();
    }
}
