//! Exact short-form time targets remain mouse-accessible at any song tempo.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Note, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn duration(app: &mut Harness, seconds: u32) -> Value {
    let state = app.click("Range");
    let label = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|label| label.starts_with("Duration · "))
        .unwrap();
    app.click(label);
    app.click(&format!("{seconds} seconds"))
}

#[test]
fn time_target_repeats_audio_and_midi_to_an_exact_thirty_second_export() {
    let mut app = Harness::new();
    let source = app.directory.path().join("Drum phrase.wav");
    AudioData {
        channels: 1,
        sample_rate: 48_000,
        samples: (0..240_000)
            .map(|i| (i as f32 * 0.05).sin() * 0.1)
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 112.0;
    project.loop_start = 0.0;
    project.loop_end = 8.0;
    project.loop_enabled = false;
    let audio = project.add_audio_track("Drums");
    project.track_mut(audio).unwrap().clips.push(Clip::audio(
        "Drum phrase",
        0.0,
        8.0,
        source.to_string_lossy(),
    ));
    let midi = project.add_midi_track("Melody");
    project.track_mut(midi).unwrap().clips.push(Clip::midi(
        "Melody phrase",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    ));
    let path = app.directory.path().join("Time target.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}));
    let before = menu(&mut app, "Edit", "Select all clips");
    let target = duration(&mut app, 30);
    assert_eq!(target["project"]["loop_start"], 0.0);
    assert_eq!(target["project"]["loop_end"], 56.0);
    assert_eq!(target["project"]["loop_enabled"], false);
    assert_eq!(target["project"]["tracks"], before["project"]["tracks"]);
    assert_eq!(target["selected_clips"], before["selected_clips"]);
    assert_eq!(target["history"]["undo"], json!(["Loop duration"]));
    assert_eq!(duration(&mut app, 30)["history"], target["history"]);
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before["project"]);
    assert_eq!(menu(&mut app, "Edit", "Redo")["project"], target["project"]);
    let repeated = menu(&mut app, "Edit", "Repeat to loop end");
    for track in repeated["project"]["tracks"].as_array().unwrap() {
        let clips = track["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 7);
        let last = clips.last().unwrap();
        assert_eq!(
            last["start"].as_f64().unwrap() + last["length"].as_f64().unwrap(),
            56.0
        );
    }
    let options = menu(&mut app, "File", "Export WAV…");
    if options["export_include_effect_tails"] == true {
        app.click("Include effect tails");
    }
    let output = app.directory.path().join("Thirty seconds.wav");
    app.command(json!({"command":"file_dialog", "kind":"export", "path":output}));
    app.click("Export…");
    app.command(json!({"command":"wait_export"}));
    let rendered = AudioData::load_wav(output).unwrap();
    assert_eq!(rendered.frames(), 30 * 48_000);
    assert!(rendered.samples.iter().all(|sample| sample.is_finite()));
    assert!(rendered.samples.iter().any(|sample| sample.abs() > 0.01));
    let saved = app.command(json!({"command":"save", "path":path}));
    app = app.restart();
    let reopened = app.command(json!({"command":"open", "path":path}));
    // Portable projects extract their assets to a fresh scratch directory.
    // Compare retained musical state and the actual source bytes independently.
    let mut saved_project = saved["project"].clone();
    let mut reopened_project = reopened["project"].clone();
    let source_bytes = std::fs::read(&source).unwrap();
    for (state, validate_asset) in [(&mut saved_project, false), (&mut reopened_project, true)] {
        for track in state["tracks"].as_array_mut().unwrap() {
            for clip in track["clips"].as_array_mut().unwrap() {
                if let Some(audio) = clip["content"].get_mut("Audio") {
                    if validate_asset {
                        assert_eq!(
                            std::fs::read(audio["path"].as_str().unwrap()).unwrap(),
                            source_bytes
                        );
                    }
                    audio["path"] = json!("portable audio asset");
                }
            }
        }
    }
    assert_eq!(reopened_project, saved_project);
    app.quit();
}

#[test]
fn time_target_preserves_fractional_start_at_narrow_widths_and_both_appearances() {
    for dark in [false, true] {
        let mut app = Harness::new();
        app.command(json!({"command":"system_theme", "dark":dark}));
        let mut project = Project::new();
        project.tempo = 93.5;
        project.loop_start = 1.375;
        project.loop_end = 4.25;
        let path = app.directory.path().join("Fractional range.rill");
        project.save(&path).unwrap();
        for width in [800, 1100, 1440] {
            app.command(json!({"command":"open", "path":path}));
            app.command(json!({"command":"viewport", "width":width, "height":800}));
            let state = duration(&mut app, 15);
            assert_eq!(state["project"]["loop_start"], 1.375);
            let end = state["project"]["loop_end"].as_f64().unwrap();
            assert!((end - (1.375 + 15.0 * 93.5 / 60.0)).abs() < 1e-10);
            assert_eq!(state["project"]["tracks"], json!([]));
            assert_eq!(state["native_menu"]["open"], false);
        }
        app.quit();
    }
}
