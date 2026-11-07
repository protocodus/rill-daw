//! Public saves are self-contained, including legacy document migration and
//! explicit application paths with a nonstandard extension.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, ClipContent, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn source_project(source: &std::path::Path) -> Project {
    AudioData {
        sample_rate: 8_000,
        channels: 2,
        samples: [0.25, -0.125].repeat(8_000),
    }
    .write_wav(source)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Portable audio");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Source",
        1.0,
        2.0,
        source.to_string_lossy(),
    ));
    let mut muted = Clip::audio(
        "Retained muted original",
        4.0,
        2.0,
        source.to_string_lossy(),
    );
    muted.muted = true;
    project.track_mut(track).unwrap().clips.push(muted);
    project
}

fn check_samples(project: &Project) {
    assert_eq!(project.tracks[0].clips.len(), 2);
    assert!(project.tracks[0].clips[1].muted);
    for clip in &project.tracks[0].clips {
        let ClipContent::Audio { path, .. } = &clip.content else {
            unreachable!()
        };
        assert_eq!(
            AudioData::load_wav(path).unwrap().samples,
            [0.25, -0.125].repeat(8_000)
        );
    }
}

#[test]
fn gui_save_migrates_a_legacy_document_without_replacing_its_json_or_losing_media() {
    let mut app = Harness::new();
    let original = app.directory.path().join("external.wav");
    let project = source_project(&original);
    let legacy = app.directory.path().join("Legacy session.daw.json");
    project.save_legacy_json(&legacy).unwrap();
    let legacy_bytes = std::fs::read(&legacy).unwrap();
    let opened = app.command(json!({"command":"open","path":legacy}));
    assert_eq!(
        opened["file_dialog_defaults"]["save"],
        "Legacy session.daw.rill"
    );
    let chosen = app.directory.path().join("Self contained");
    app.command(json!({"command":"file_dialog","kind":"save","path":chosen}));
    menu(&mut app, "File", "Save");
    let saved = app.command(json!({"command":"wait_project"}));
    let portable = chosen.with_extension("rill");
    assert_eq!(saved["project_path"], json!(portable));
    assert_eq!(saved["modified"], false);
    assert_eq!(std::fs::read(&legacy).unwrap(), legacy_bytes);
    assert!(std::fs::read(&portable).unwrap().starts_with(b"PK\x03\x04"));
    std::fs::remove_file(original).unwrap();
    app = app.restart();
    let moved = app.directory.path().join("Moved project.rill");
    std::fs::rename(portable, &moved).unwrap();
    let reopened = app.command(json!({"command":"open","path":moved}));
    check_samples(&serde_json::from_value(reopened["project"].clone()).unwrap());
    let rendered = app.directory.path().join("portable.wav");
    app.command(json!({"command":"export","path":rendered}));
    app.command(json!({"command":"wait_export"}));
    let audio = AudioData::load_wav(rendered).unwrap();
    let frame = audio.sample_rate as usize;
    assert!((audio.samples[frame * 2] - 0.25).abs() < 1e-5);
    assert!((audio.samples[frame * 2 + 1] + 0.125).abs() < 1e-5);
    app.quit();
}

#[test]
fn explicit_save_path_keeps_its_filename_but_contains_portable_audio() {
    let mut app = Harness::new();
    let original = app.directory.path().join("external.wav");
    let project = source_project(&original);
    let legacy = app.directory.path().join("legacy.json");
    project.save_legacy_json(&legacy).unwrap();
    app.command(json!({"command":"open","path":legacy}));
    let path = app.directory.path().join("explicit.json");
    let saved = app.command(json!({"command":"save","path":path}));
    assert_eq!(saved["project_path"], json!(path));
    assert!(std::fs::read(&path).unwrap().starts_with(b"PK\x03\x04"));
    std::fs::remove_file(original).unwrap();
    let loaded = app.command(json!({"command":"open","path":path}));
    check_samples(&serde_json::from_value(loaded["project"].clone()).unwrap());
    app.quit();
}
