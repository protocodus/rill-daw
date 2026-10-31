//! Export feedback follows the chosen format, with explicit tail duration and
//! serialized native pickers so an earlier WAV cannot obscure a MIDI result.
mod common;
#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::Harness;
use daw_core::{AudioData, Clip, Instrument, Note, Project};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

fn has(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}

fn menu(app: &mut Harness, label: &str) -> Value {
    app.click("File");
    app.click(label)
}

fn picker(app: &mut Harness, kind: &str, path: Option<&Path>) {
    app.command(json!({"command":"file_dialog", "kind":kind, "path":path}));
}

fn seed(app: &mut Harness, length: f64) {
    let source = app.directory.path().join("Source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..4800).map(|i| (i as f32 * 0.07).sin() * 0.2).collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Rhythm");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Phrase",
        0.0,
        length,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("Melody");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Notes",
        0.0,
        length,
        vec![Note::new(60, 0.0, 0.125, 100)],
    ));
    let path = app.directory.path().join("Song.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
}

fn export_wav(app: &mut Harness, path: &Path) -> Value {
    picker(app, "export", Some(path));
    app.click("Export…");
    app.command(json!({"command":"wait_export"}))
}

fn receipt(kind: &str, path: &Path) -> Value {
    json!({"kind":kind, "path":path})
}

fn copy_receipt(app: &mut Harness, kind: &str, path: &Path) -> Value {
    menu(app, "Recent exports");
    app.click(&format!(
        "{kind} · {}",
        path.file_name().unwrap().to_string_lossy()
    ));
    let copied = app.click("Copy path");
    assert_eq!(copied["clipboard_text"], path.to_string_lossy().as_ref());
    copied
}

#[test]
fn tail_summary_changes_with_the_checkbox_and_matches_the_rendered_boundary() {
    let mut app = Harness::new();
    seed(&mut app, 60.0);
    let before = app.inspect();
    let options = menu(&mut app, "Export WAV…");
    assert_eq!(options["export_include_effect_tails"], true);
    assert!(has(&options, "Arrangement · 0:30 + tails"));
    assert!(has(&options, "48 kHz · Stereo · 32-bit float"));
    let tails = app.directory.path().join("With tails.wav");
    export_wav(&mut app, &tails);
    assert_eq!(
        AudioData::load_wav(tails).unwrap().frames(),
        48_000 * 30 + 9600
    );

    app.click("Window");
    app.click("Setup");
    app.click("Audio");
    app.click("48000 Hz");
    app.click("44100 Hz");
    let settings = app.click("Apply");
    assert_eq!(settings["settings"]["sample_rate"], 44_100);
    let options = menu(&mut app, "Export WAV…");
    assert!(has(&options, "44.1 kHz · Stereo · 32-bit float"));
    let exact_options = app.click("Include effect tails");
    assert_eq!(exact_options["export_include_effect_tails"], false);
    assert!(has(&exact_options, "Arrangement · 0:30"));
    assert!(!has(&exact_options, "Arrangement · 0:30 + tails"));
    let exact = app.directory.path().join("Exact.wav");
    let done = export_wav(&mut app, &exact);
    let rendered = AudioData::load_wav(exact).unwrap();
    assert_eq!(rendered.sample_rate, 44_100);
    assert_eq!(rendered.channels, 2);
    assert_eq!(rendered.frames(), 44_100 * 30);
    assert!(rendered.samples.iter().any(|s| s.abs() > 0.01));
    assert_eq!(done["project"], before["project"]);
    assert_eq!(done["history"], before["history"]);
    app.quit();
}

#[test]
fn midi_results_replace_wav_feedback_without_losing_previous_success_on_failure() {
    let mut app = Harness::new();
    seed(&mut app, 0.2);
    menu(&mut app, "Export WAV…");
    app.click("Include effect tails");
    let wav = app.directory.path().join("Mix.wav");
    let done = export_wav(&mut app, &wav);
    assert!(has(&done, "Exported · Mix.wav"));
    let wav_target = done["finishing"]["last_export"].clone();
    let wav_receipts = json!([receipt("WAV", &wav)]);
    assert_eq!(done["finishing"]["recent_exports"], wav_receipts);

    // A failed MIDI write leaves the previous delivered file identifiable and
    // usable, while the error gets its own primary notice.
    let invalid = app.directory.path().join("Missing folder/Notes.mid");
    picker(&mut app, "export_midi", Some(&invalid));
    let failed = menu(&mut app, "Export MIDI…");
    assert_eq!(failed["error"], true);
    assert!(
        failed["status"]
            .as_str()
            .unwrap()
            .contains("MIDI export failed")
    );
    assert_eq!(failed["exported_file"], json!(wav));
    assert_eq!(failed["finishing"]["recent_exports"], wav_receipts);
    assert!(has(&failed, "Exported · Mix.wav"));
    assert_eq!(
        app.click("Copy path")["clipboard_text"],
        wav.to_string_lossy().as_ref()
    );
    let dismissed = app.click("Dismiss");
    assert_eq!(dismissed["error"], false);
    assert!(has(&dismissed, "Exported · Mix.wav"));
    assert!(!invalid.exists());

    picker(&mut app, "export_midi", None);
    let cancelled = menu(&mut app, "Export MIDI…");
    assert_eq!(cancelled["exported_file"], json!(wav));
    assert_eq!(cancelled["finishing"]["recent_exports"], wav_receipts);

    let midi = app.directory.path().join("Notes.mid");
    picker(&mut app, "export_midi", Some(&midi));
    let done = menu(&mut app, "Export MIDI…");
    assert_eq!(done["error"], false);
    assert_eq!(done["exported_file"], json!(midi));
    assert!(has(&done, "Exported · Notes.mid"));
    assert!(!has(&done, "Exported · Mix.wav"));
    assert_eq!(
        app.click("Copy path")["clipboard_text"],
        midi.to_string_lossy().as_ref()
    );
    assert_eq!(done["finishing"]["last_export"], wav_target);
    let both_receipts = json!([receipt("MIDI", &midi), receipt("WAV", &wav)]);
    assert_eq!(done["finishing"]["recent_exports"], both_receipts);
    let recent = menu(&mut app, "Recent exports");
    assert!(has(&recent, "Completed exports"));
    assert!(has(&recent, "MIDI · Notes.mid"));
    assert!(has(&recent, "WAV · Mix.wav"));
    // Close the menu before exercising each complete mouse-accessible path.
    app.click("File");
    let copied = copy_receipt(&mut app, "WAV", &wav);
    assert_eq!(copied["exported_file"], json!(midi));
    assert!(has(&copied, "Exported · Notes.mid"));
    copy_receipt(&mut app, "MIDI", &midi);

    // Neither a failure nor a cancelled picker invents a completion or erases
    // either successful output after the latest primary result became MIDI.
    picker(&mut app, "export_midi", Some(&invalid));
    let failed = menu(&mut app, "Export MIDI…");
    assert_eq!(failed["error"], true);
    assert_eq!(failed["exported_file"], json!(midi));
    assert_eq!(failed["finishing"]["recent_exports"], both_receipts);
    assert!(has(&failed, "Exported · Notes.mid"));
    app.click("Dismiss");
    picker(&mut app, "export_midi", None);
    let cancelled = menu(&mut app, "Export MIDI…");
    assert_eq!(cancelled["exported_file"], json!(midi));
    assert_eq!(cancelled["finishing"]["recent_exports"], both_receipts);
    let midi_bytes = std::fs::read(&midi).unwrap();
    let mut imported = Project::new();
    imported.import_midi(&midi, 0.0).unwrap();
    assert!(imported.tracks.iter().flat_map(|t| &t.clips).any(|clip| {
        matches!(&clip.content, daw_core::ClipContent::Midi { notes } if !notes.is_empty())
    }));

    // The explicitly named WAV command still targets its last successful WAV,
    // never the more recent MIDI file or the failed MIDI destination.
    std::fs::write(&wav, b"replace this mix").unwrap();
    menu(&mut app, "Re-export last WAV (replace)");
    let reexported = app.command(json!({"command":"wait_export"}));
    assert_eq!(reexported["exported_file"], json!(wav));
    assert!(has(&reexported, "Exported · Mix.wav"));
    assert_eq!(AudioData::load_wav(&wav).unwrap().frames(), 4800);
    assert_eq!(std::fs::read(midi).unwrap(), midi_bytes);
    assert_eq!(reexported["finishing"]["last_export"], wav_target);
    assert_eq!(
        reexported["finishing"]["recent_exports"],
        json!([receipt("WAV", &wav), both_receipts[0]])
    );

    // Opening even the same saved document starts a fresh receipt scope.
    let project_path = app.directory.path().join("Song.rill");
    picker(&mut app, "open", Some(&project_path));
    menu(&mut app, "Open…");
    let opened = app.command(json!({"command":"wait_project"}));
    assert_eq!(opened["finishing"]["recent_exports"], json!([]));
    assert!(opened["finishing"]["last_export"].is_null());
    let empty = menu(&mut app, "Recent exports");
    assert!(has(&empty, "No completed exports in this project"));
    assert!(!has(&empty, "WAV · Mix.wav"));
    app.click("File");

    let fresh = app.directory.path().join("Fresh.mid");
    picker(&mut app, "export_midi", Some(&fresh));
    let done = menu(&mut app, "Export MIDI…");
    assert_eq!(
        done["finishing"]["recent_exports"],
        json!([receipt("MIDI", &fresh)])
    );
    menu(&mut app, "New");
    let new = app.command(json!({"command":"wait_project"}));
    assert_eq!(new["finishing"]["recent_exports"], json!([]));
    let empty = menu(&mut app, "Recent exports");
    assert!(has(&empty, "No completed exports in this project"));
    assert!(!has(&empty, "MIDI · Fresh.mid"));
    app.quit();
}

#[test]
fn recent_exports_keep_five_completed_outputs_and_move_repeated_paths_to_the_front() {
    let mut app = Harness::new();
    seed(&mut app, 0.2);
    menu(&mut app, "Export WAV…");
    let wav = app.directory.path().join("Original.wav");
    let done = export_wav(&mut app, &wav);
    let wav_target = done["finishing"]["last_export"].clone();
    let paths: Vec<_> = (1..=6)
        .map(|number| app.directory.path().join(format!("Take {number}.mid")))
        .collect();
    for path in &paths[..5] {
        picker(&mut app, "export_midi", Some(path));
        menu(&mut app, "Export MIDI…");
    }
    let expected: Vec<_> = paths[..5]
        .iter()
        .rev()
        .map(|path| receipt("MIDI", path))
        .collect();
    let capped = app.inspect();
    assert_eq!(capped["finishing"]["recent_exports"], json!(expected));
    // Receipt eviction must not lose the independently retained WAV target.
    assert_eq!(capped["finishing"]["last_export"], wav_target);

    picker(&mut app, "export_midi", Some(&paths[2]));
    let repeated = menu(&mut app, "Export MIDI…");
    assert_eq!(
        repeated["finishing"]["recent_exports"],
        json!([
            expected[2],
            expected[0],
            expected[1],
            expected[3],
            expected[4]
        ])
    );
    picker(&mut app, "export_midi", Some(&paths[5]));
    let newest = menu(&mut app, "Export MIDI…");
    assert_eq!(
        newest["finishing"]["recent_exports"],
        json!([
            receipt("MIDI", &paths[5]),
            expected[2],
            expected[0],
            expected[1],
            expected[3]
        ])
    );
    assert_eq!(newest["finishing"]["last_export"], wav_target);
    let recent = menu(&mut app, "Recent exports");
    for number in 2..=6 {
        assert!(has(&recent, &format!("MIDI · Take {number}.mid")));
    }
    assert!(!has(&recent, "MIDI · Take 1.mid"));
    assert!(!has(&recent, "WAV · Original.wav"));
    app.click("File");
    copy_receipt(&mut app, "MIDI", &paths[2]);
    app.quit();
}

#[test]
fn pending_wav_disables_midi_picker_and_reexport_without_consuming_the_next_destination() {
    // A native instrument holds the worker in its first offline processing
    // block, so the menu is exercised while a real export remains pending.
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_secs(2));
    let mut app = Harness::with_vst_path(plugin.path.parent().unwrap());
    let mut project = Project::new();
    let track = project.add_midi_track("Patient synth");
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
        "Phrase",
        0.0,
        0.25,
        vec![Note::new(60, 0.0, 0.125, 100)],
    ));
    let project_path = app.directory.path().join("Patient.rill");
    project.save(&project_path).unwrap();
    app.command(json!({"command":"open", "path":project_path}));
    let wav = app.directory.path().join("Patient.wav");
    picker(&mut app, "export", Some(&wav));
    menu(&mut app, "Export WAV…");
    let pending = app.click("Export…");
    assert_eq!(pending["export_pending"], true);
    let midi = app.directory.path().join("Patient.mid");
    picker(&mut app, "export_midi", Some(&midi));
    app.click("File");
    let guarded = app.click("Export MIDI…");
    assert_eq!(guarded["export_pending"], true);
    assert_eq!(guarded["file_dialog_pending"], true);
    assert!(!midi.exists());
    // Clicking a disabled menu row dismisses the popup; reopen it to exercise
    // the independent re-export action while the same worker is still busy.
    app.click("File");
    let guarded = app.click("Re-export last WAV (replace)");
    assert_eq!(guarded["export_pending"], true);
    assert_eq!(guarded["file_dialog_pending"], true);
    assert_eq!(guarded["export_run"]["path"], json!(wav));
    app.command(json!({"command":"wait_export", "timeout_ms":20000}));

    let done = menu(&mut app, "Export MIDI…");
    assert_eq!(done["file_dialog_pending"], false);
    assert_eq!(done["exported_file"], json!(midi));
    assert!(has(&done, "Exported · Patient.mid"));
    assert!(!has(&done, "Exported · Patient.wav"));
    assert_eq!(
        app.click("Copy path")["clipboard_text"],
        midi.to_string_lossy().as_ref()
    );
    assert!(
        AudioData::load_wav(wav)
            .unwrap()
            .samples
            .iter()
            .any(|s| s.abs() > 0.001)
    );
    app.quit();
}
