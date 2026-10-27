//! Source identities stay protected when a mixdown destination aliases original audio.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::json;
use std::{path::Path, process::Command};

fn fixture(app: &Harness) -> (Project, std::path::PathBuf, Vec<std::path::PathBuf>) {
    let sources: Vec<_> = ["Original.wav", "Inactive take.wav", "Muted source.wav"]
        .iter()
        .map(|name| app.directory.path().join(name))
        .collect();
    for (index, path) in sources.iter().enumerate() {
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: (0..24_000)
                .map(|frame| (frame as f32 * 0.07).sin() * (index + 1) as f32 * 0.1)
                .collect(),
        }
        .write_wav(path)
        .unwrap();
    }
    let mut project = Project::new();
    let track = project.add_audio_track("Takes");
    let ids: Vec<_> = sources[..2]
        .iter()
        .map(|path| {
            let clip = Clip::audio("Take", 0.0, 1.0, path.to_string_lossy());
            let id = clip.id;
            project.track_mut(track).unwrap().clips.push(clip);
            id
        })
        .collect();
    project.group_audio_takes(track, &ids).unwrap();
    let muted = project.add_audio_track("Muted");
    let muted = project.track_mut(muted).unwrap();
    muted.mute = true;
    muted.clips.push(Clip::audio(
        "Muted audio",
        0.0,
        1.0,
        sources[2].to_string_lossy(),
    ));
    // Legacy JSON deliberately retains the real imported paths instead of
    // extracting private bundle copies, as importing a WAV does in the app.
    let path = app.directory.path().join("Sources.daw.json");
    project.save(&path).unwrap();
    (project, path, sources)
}

fn export_options(app: &mut Harness, path: &Path) {
    app.click("File");
    app.click("Export WAV…");
    app.command(json!({"command":"file_dialog", "kind":"export", "path":path}));
}

fn assert_sources_unchanged(sources: &[std::path::PathBuf], bytes: &[Vec<u8>]) {
    for (source, before) in sources.iter().zip(bytes) {
        assert_eq!(
            &std::fs::read(source).unwrap(),
            before,
            "{}",
            source.display()
        );
    }
}

#[test]
fn export_picker_rejects_source_aliases_including_inactive_takes_and_allows_a_new_mix() {
    let mut app = Harness::new();
    let (_, project_path, sources) = fixture(&app);
    let bytes: Vec<_> = sources
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    app.command(json!({"command":"open", "path":project_path}));
    let before = app.inspect();
    let mut destinations = sources.clone();
    let child = app.directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    destinations.push(child.join("../Original.wav"));
    let hardlink = app.directory.path().join("Original hard link.wav");
    std::fs::hard_link(&sources[0], &hardlink).unwrap();
    destinations.push(hardlink);
    #[cfg(unix)]
    {
        let symlink = app.directory.path().join("Original symlink.wav");
        std::os::unix::fs::symlink(&sources[0], &symlink).unwrap();
        destinations.push(symlink);
        let directory_alias = app.directory.path().join("directory alias");
        std::os::unix::fs::symlink(app.directory.path(), &directory_alias).unwrap();
        destinations.push(directory_alias.join("Original.wav"));
    }
    for destination in destinations {
        export_options(&mut app, &destination);
        let result = app.raw_command(json!({"command":"click", "label":"Export…"}));
        let failed = &result["state"];
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(failed["error"], true);
        assert!(
            failed["status"]
                .as_str()
                .unwrap()
                .contains("Choose a different output file")
        );
        assert_eq!(failed["export_pending"], false);
        assert!(failed["exported_file"].is_null());
        assert_eq!(failed["project"], before["project"]);
        assert_eq!(failed["history"], before["history"]);
        assert_sources_unchanged(&sources, &bytes);
        app.click("Dismiss");
    }
    let mix = app.directory.path().join("Mix.wav");
    export_options(&mut app, &mix);
    app.click("Export…");
    app.command(json!({"command":"wait_export"}));
    let exported = app.inspect();
    assert_eq!(exported["error"], false);
    assert_eq!(exported["exported_file"], json!(mix));
    let audio = AudioData::load_wav(mix).unwrap();
    assert_eq!(audio.channels, 2);
    assert!(audio.samples.iter().any(|sample| sample.abs() > 0.01));
    assert_sources_unchanged(&sources, &bytes);
    app.quit();
}

#[test]
fn batch_render_wav_and_midi_exports_cannot_replace_source_audio() {
    let app = Harness::new();
    let (project, project_path, sources) = fixture(&app);
    let bytes: Vec<_> = sources
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let binary =
        std::env::var_os("RILL_E2E_BINARY").unwrap_or_else(|| env!("CARGO_BIN_EXE_rill").into());
    let output = Command::new(binary)
        .arg("--render")
        .arg(project_path)
        .arg(&sources[0])
        .env("RILL_CONFIG_DIR", app.directory.path().join("cli-config"))
        .env("RILL_CACHE_DIR", app.directory.path().join("cli-cache"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Choose a different output file"));
    let track = project.tracks[1].id;
    let clip = project.tracks[1].clips[0].id;
    for result in [
        daw_engine::export_wav(&project, &sources[0], 48_000, 1.0),
        daw_engine::export_project_wav(&project, &sources[1], 48_000),
        daw_core::export_midi(&project, &sources[0]),
        project.export_midi(&sources[1]),
        daw_engine::export_audio_join(
            &project,
            track,
            &[clip],
            &sources[2],
            48_000,
            daw_engine::RenderOptions::default(),
        ),
    ] {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Choose a different output file")
        );
    }
    assert_sources_unchanged(&sources, &bytes);
    assert!(
        std::fs::read_dir(app.directory.path())
            .unwrap()
            .all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
    );
    app.quit();
}

#[test]
fn save_as_preserves_imported_and_bundled_sources_and_portable_save_keeps_complete_audio() {
    let mut app = Harness::new();
    let (project, project_path, sources) = fixture(&app);
    let bytes: Vec<_> = sources
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    app.command(json!({"command":"open", "path":project_path}));
    let before = app.inspect();
    let bundle_alias = app.directory.path().join("Source alias.rill");
    std::fs::hard_link(&sources[0], &bundle_alias).unwrap();
    for destination in [&sources[0], &bundle_alias] {
        app.command(json!({"command":"file_dialog", "kind":"save", "path":destination}));
        app.click("File");
        app.click("Save as…");
        app.raw_command(json!({"command":"wait_project"}));
        let failed = app.inspect();
        assert_eq!(failed["error"], true);
        assert!(
            failed["status"]
                .as_str()
                .unwrap()
                .contains("Choose a different output file")
        );
        assert_eq!(failed["project"], before["project"]);
        assert_eq!(failed["history"], before["history"]);
        assert_eq!(failed["project_path"], before["project_path"]);
        assert_sources_unchanged(&sources, &bytes);
        app.click("Dismiss");
    }
    // The core API applies the same protection to both formats too.
    assert!(project.save(&sources[0]).is_err());
    assert!(project.save(&bundle_alias).is_err());
    let bundle = app.directory.path().join("Portable.rill");
    app.command(json!({"command":"file_dialog", "kind":"save", "path":bundle}));
    app.click("File");
    app.click("Save as…");
    app.command(json!({"command":"wait_project"}));
    app = app.restart();
    let reopened = app.command(json!({"command":"open", "path":bundle}));
    let portable: Project = serde_json::from_value(reopened["project"].clone()).unwrap();
    let mut extracted = Vec::new();
    for track in &portable.tracks {
        for clip in &track.clips {
            let daw_core::ClipContent::Audio { path, .. } = &clip.content else {
                continue;
            };
            let content = std::fs::read(path).unwrap();
            assert!(
                bytes.contains(&content),
                "Bundle must retain complete, unchanged WAVs"
            );
            if !extracted.contains(path) {
                extracted.push(path.clone());
            }
        }
    }
    assert_eq!(
        extracted.len(),
        3,
        "Inactive and muted originals must travel too"
    );
    app.command(json!({"command":"file_dialog", "kind":"save", "path":extracted[0]}));
    app.click("File");
    app.click("Save as…");
    app.raw_command(json!({"command":"wait_project"}));
    let failed = app.inspect();
    assert_eq!(failed["error"], true);
    assert!(
        failed["status"]
            .as_str()
            .unwrap()
            .contains("Choose a different output file")
    );
    assert_eq!(failed["project_path"], json!(bundle));
    assert_sources_unchanged(&sources, &bytes);
    assert_eq!(std::fs::read(&extracted[0]).unwrap(), bytes[0]);
    app.click("Dismiss");
    app.quit();
}
