//! Document identity and export decisions use the production menus and workers.
mod common;
use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};
use std::path::Path;

fn menu(app: &mut Harness, group: &str, item: &str) -> Value {
    app.click(group);
    app.click(item)
}

fn picker(app: &mut Harness, kind: &str, path: Option<&Path>) {
    app.command(json!({"command":"file_dialog", "kind":kind, "path":path}));
}

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

fn finished(app: &mut Harness) -> Value {
    app.command(json!({"command":"wait_project"}));
    app.inspect()
}

#[test]
fn save_as_and_reopen_show_the_saved_filename_and_seed_export_destinations() {
    let mut app = Harness::new();
    let initial = app.inspect();
    assert_eq!(
        initial["file_dialog_defaults"]["wav"],
        "Untitled session.wav"
    );
    assert_eq!(
        initial["file_dialog_defaults"]["midi"],
        "Untitled session.mid"
    );
    assert!(initial["file_dialog_defaults"]["directory"].is_null());
    menu(&mut app, "Track", "Add audio track");
    let folder = app.directory.path().join("My songs");
    std::fs::create_dir_all(&folder).unwrap();
    let chosen = folder.join("My first song");
    let saved = folder.join("My first song.rill");
    picker(&mut app, "save", Some(&chosen));
    menu(&mut app, "File", "Save as…");
    let state = finished(&mut app);
    assert!(saved.exists());
    assert_eq!(state["project_path"], json!(saved));
    assert_eq!(state["project_title"], "My first song");
    assert!(has(&state, "My first song"));
    assert_eq!(state["project"]["name"], "Untitled session");
    assert_eq!(state["file_dialog_defaults"]["directory"], json!(folder));
    assert_eq!(state["file_dialog_defaults"]["save"], "My first song.rill");
    assert_eq!(state["file_dialog_defaults"]["wav"], "My first song.wav");
    assert_eq!(state["file_dialog_defaults"]["midi"], "My first song.mid");

    let changed = menu(&mut app, "Track", "Add audio track");
    assert_eq!(changed["project_title"], "My first song •");
    assert!(has(&changed, "My first song •"));
    menu(&mut app, "File", "Save");
    assert_eq!(finished(&mut app)["project_title"], "My first song");
    menu(&mut app, "File", "New");
    assert_eq!(finished(&mut app)["project_title"], "Untitled session");
    picker(&mut app, "open", Some(&saved));
    menu(&mut app, "File", "Open…");
    let reopened = finished(&mut app);
    assert_eq!(reopened["project_title"], "My first song");
    assert!(has(&reopened, "My first song"));
    assert_eq!(reopened["project"]["tracks"].as_array().unwrap().len(), 2);
    app.quit();
}

#[test]
fn save_picker_normalizes_missing_extensions_and_preserves_recognized_extensions() {
    let mut app = Harness::new();
    menu(&mut app, "Track", "Add audio track");
    for (chosen, expected) in [
        ("First song", "First song.rill"),
        ("Version 1.2", "Version 1.2.rill"),
        ("Capital.RILL", "Capital.RILL"),
        ("Legacy.JSON", "Legacy.JSON"),
    ] {
        let chosen = app.directory.path().join(chosen);
        let expected = app.directory.path().join(expected);
        picker(&mut app, "save", Some(&chosen));
        menu(&mut app, "File", "Save as…");
        let state = finished(&mut app);
        assert_eq!(state["project_path"], json!(expected));
        assert!(expected.exists());
        assert_eq!(Project::load(&expected).unwrap().tracks.len(), 1);
        if expected.extension().unwrap().eq_ignore_ascii_case("rill") {
            assert!(std::fs::read(&expected).unwrap().starts_with(b"PK"));
        }
    }
    app.quit();
}

fn unassigned_project() -> Project {
    let mut project = Project::new();
    let bass = project.add_midi_track("Bass needs instrument");
    project.track_mut(bass).unwrap().clips.push(Clip::midi(
        "Bass phrase",
        0.0,
        4.0,
        vec![Note::new(36, 0.0, 2.0, 100)],
    ));
    project.add_midi_track("Empty MIDI");
    let hidden = project.add_midi_track("Trimmed notes only");
    let mut clip = Clip::midi(
        "Hidden source",
        0.0,
        2.0,
        vec![Note::new(60, 0.0, 1.0, 100), Note::new(72, 5.0, 1.0, 100)],
    );
    clip.offset = 2.0;
    project.track_mut(hidden).unwrap().clips.push(clip);
    project
}

#[test]
fn missing_instrument_warning_follows_track_folder_and_solo_bus_eligibility() {
    let mut app = Harness::new();
    for case in [
        "muted",
        "muted folder",
        "solo other",
        "solo folder",
        "solo bus",
    ] {
        let mut project = unassigned_project();
        let bass = project.tracks[0].id;
        let should_warn = match case {
            "muted" => {
                project.track_mut(bass).unwrap().mute = true;
                false
            }
            "muted folder" | "solo folder" => {
                let folder = project.add_folder("Group");
                project.set_parent_folder(bass, Some(folder)).unwrap();
                project.track_mut(folder).unwrap().mute = case == "muted folder";
                project.track_mut(folder).unwrap().solo = case == "solo folder";
                case == "solo folder"
            }
            "solo other" => {
                let other = project.add_audio_track("Intentionally soloed audio");
                project.track_mut(other).unwrap().solo = true;
                false
            }
            "solo bus" => {
                let bus = project.add_bus("Soloed destination");
                project.set_output(bass, Some(bus)).unwrap();
                project.track_mut(bus).unwrap().solo = true;
                true
            }
            _ => unreachable!(),
        };
        let saved = app.directory.path().join(format!("{case}.rill"));
        project.save(&saved).unwrap();
        app.command(json!({"command":"open", "path":saved}));
        let state = menu(&mut app, "File", "Export WAV…");
        assert_eq!(
            has(&state, "MIDI notes need an instrument"),
            should_warn,
            "{case}"
        );
        assert_eq!(
            state["export_missing_instruments"],
            if should_warn {
                json!(["Bass needs instrument"])
            } else {
                json!([])
            },
            "{case}"
        );
        app.click("Cancel");
    }
    app.quit();
}

#[test]
fn wav_export_warns_for_visible_unassigned_notes_and_allows_cancel_or_explicit_export() {
    for include_audio in [false, true] {
        let mut app = Harness::new();
        let mut project = unassigned_project();
        if include_audio {
            let source = app.directory.path().join("Backing.wav");
            AudioData {
                channels: 1,
                sample_rate: 48_000,
                samples: (0..48_000)
                    .map(|i| (i as f32 * 0.057).sin() * 0.2)
                    .collect(),
            }
            .write_wav(&source)
            .unwrap();
            let track = project.add_audio_track("Backing audio");
            project.track_mut(track).unwrap().clips.push(Clip::audio(
                "Backing phrase",
                0.0,
                2.0,
                source.to_string_lossy(),
            ));
        }
        let saved = app.directory.path().join("Unassigned.rill");
        project.save(&saved).unwrap();
        app.command(json!({"command":"open", "path":saved}));
        let before = app.inspect();
        let output = app.directory.path().join("Chosen mix.wav");
        picker(&mut app, "export", Some(&output));
        let warning = menu(&mut app, "File", "Export WAV…");
        assert_eq!(
            warning["export_missing_instruments"],
            json!(["Bass needs instrument"])
        );
        assert!(has(&warning, "MIDI notes need an instrument"));
        assert!(has(&warning, "No instrument: Bass needs instrument"));
        assert!(
            warning["visible_labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|text| text
                    .as_str()
                    .unwrap_or_default()
                    .contains("These notes will be silent in the WAV"))
        );
        let cancelled = app.click("Cancel");
        assert_eq!(cancelled["file_dialog_pending"], true);
        assert_eq!(cancelled["export_pending"], false);
        assert_eq!(cancelled["project"], before["project"]);
        assert_eq!(cancelled["history"], before["history"]);
        assert!(!output.exists());

        menu(&mut app, "File", "Export WAV…");
        let exporting = app.click("Export…");
        assert_eq!(exporting["popups"]["export"], false);
        app.command(json!({"command":"wait_export"}));
        let rendered = AudioData::load_wav(&output).unwrap();
        assert_eq!(rendered.channels, 2);
        assert!(rendered.samples.iter().all(|sample| sample.is_finite()));
        let peak = rendered
            .samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);
        if include_audio {
            assert!(peak > 0.05, "audio tracks still export: {peak}");
        } else {
            assert!(peak < 0.000001, "unassigned MIDI remains silent: {peak}");
        }
        let finished = app.inspect();
        assert_eq!(finished["exported_file"], json!(output));
        assert_eq!(finished["project"], before["project"]);
        assert_eq!(finished["history"], before["history"]);
        app.quit();
    }
}

#[test]
fn ordinary_audio_export_has_no_instrument_warning_and_midi_completion_names_the_file() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Source.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: (0..4800).map(|i| (i as f32 * 0.07).sin() * 0.2).collect(),
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop", "paths":[source], "beat":0.0}));
    let output = app.directory.path().join("Audio.wav");
    picker(&mut app, "export", Some(&output));
    let state = menu(&mut app, "File", "Export WAV…");
    assert_eq!(state["export_missing_instruments"], json!([]));
    assert!(!has(&state, "MIDI notes need an instrument"));
    app.click("Export…");
    app.command(json!({"command":"wait_export"}));
    assert!(
        AudioData::load_wav(&output)
            .unwrap()
            .samples
            .iter()
            .any(|sample| sample.abs() > 0.05)
    );

    // MIDI export preserves notes without requiring a sound-generating device.
    let saved = app.directory.path().join("Song version 2.rill");
    unassigned_project().save(&saved).unwrap();
    app.command(json!({"command":"open", "path":saved}));
    assert_eq!(
        app.inspect()["file_dialog_defaults"]["midi"],
        "Song version 2.mid"
    );
    let picked = app.directory.path().join("My MIDI arrangement");
    let midi = app.directory.path().join("My MIDI arrangement.mid");
    picker(&mut app, "export_midi", Some(&picked));
    let completed = menu(&mut app, "File", "Export MIDI…");
    assert_eq!(completed["exported_file"], json!(midi));
    assert!(has(&completed, "Exported · My MIDI arrangement.mid"));
    assert_eq!(completed["popups"]["export"], false);
    let copied = app.click("Copy path");
    assert_eq!(copied["clipboard_text"], json!(midi.to_string_lossy()));
    let mut imported = Project::new();
    imported.import_midi(&midi, 0.0).unwrap();
    assert!(imported.tracks.iter().flat_map(|track| &track.clips).any(
        |clip| matches!(&clip.content, daw_core::ClipContent::Midi { notes } if !notes.is_empty())
    ));
    app.quit();
}
