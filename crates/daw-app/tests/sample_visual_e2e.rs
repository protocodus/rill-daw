//! Selected-source analysis paints the inspector without loading or playing audio.
mod common;

use common::Harness;
use daw_core::AudioData;
use serde_json::{Value, json};
use std::path::Path;

fn tone(path: &Path, frequency: f32, amplitude: f32, frames: usize) {
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..frames)
            .map(|frame| {
                let envelope = 0.1 + 0.9 * frame as f32 / frames as f32;
                amplitude
                    * envelope
                    * (std::f32::consts::TAU * frequency * frame as f32 / 48_000.0).sin()
            })
            .collect(),
    }
    .write_wav(path)
    .unwrap();
}

fn index(app: &mut Harness, root: &Path, count: usize) -> Value {
    app.click("Window");
    app.click("Device browser");
    app.click("Samples");
    app.click("Folders…");
    app.click("Sample folder path");
    app.command(json!({"command":"text","text":root}));
    app.click("Add folder");
    app.click("Done");
    app.click("Scan folders");
    common::wait_until(app, |state| {
        state["library"]["indexed"] == count && state["library"]["scanning"] == false
    })
}

fn ready(app: &mut Harness, path: &Path) -> Value {
    common::wait_until(app, |state| {
        let visual = &state["library"]["visual_analysis"];
        visual["state"] == "ready"
            && visual["path"] == path.to_string_lossy().as_ref()
            && visual["waveform_rect"].is_object()
            && visual["spectrum_rect"].is_object()
    })
}

fn unchanged(state: &Value, before: &Value) {
    for field in [
        "project",
        "history",
        "revision",
        "modified",
        "selected_track",
        "error",
    ] {
        assert_eq!(
            state[field], before[field],
            "Visual inspection changed {field}"
        );
    }
    assert_ne!(state["engine"]["playing"], true);
    assert_ne!(state["engine"]["sample_audition_active"], true);
    assert_eq!(state["audio_import"]["busy"], false);
    assert_eq!(state["audio_import"]["queued"], 0);
}

fn dominant_frequency(visual: &Value) -> f64 {
    let spectrum = visual["spectrum"].as_array().unwrap();
    let index = spectrum
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.as_f64().unwrap().total_cmp(&b.as_f64().unwrap()))
        .unwrap()
        .0;
    let low = visual["min_frequency"].as_f64().unwrap();
    let high = visual["max_frequency"].as_f64().unwrap();
    low * (high / low).powf((index as f64 + 0.5) / spectrum.len() as f64)
}

fn contains(outer: &Value, inner: &Value) -> bool {
    (0..2).all(|axis| {
        inner["min"][axis].as_f64().unwrap() >= outer["min"][axis].as_f64().unwrap() - 0.1
            && inner["max"][axis].as_f64().unwrap() <= outer["max"][axis].as_f64().unwrap() + 0.1
    })
}

#[test]
fn selected_audio_paints_actual_waveform_and_spectrum_behind_metadata_only() {
    let mut app = Harness::with_virtual_audio(None, None);
    app.command(json!({"command":"viewport","width":1100,"height":720}));
    let root = app.directory.path().join("Visual library");
    std::fs::create_dir(&root).unwrap();
    let low = root.join("Low tone.wav");
    let high = root.join("High tone.wav");
    tone(&low, 400.0, 0.4, 48_000);
    tone(&high, 1_600.0, 0.15, 24_000);
    let before = index(&mut app, &root, 2);
    assert!(
        before["library"]["visual_analysis"].is_null(),
        "indexing alone does not analyze files"
    );
    app.click("Low tone.wav");
    let first = ready(&mut app, &low);
    let visual = &first["library"]["visual_analysis"];
    assert_eq!(visual["sample_rate"], 48_000);
    assert_eq!(visual["source_frames"], 48_000);
    assert_eq!(visual["waveform"].as_array().unwrap().len(), 256);
    assert_eq!(visual["spectrum"].as_array().unwrap().len(), 48);
    assert!(
        (dominant_frequency(visual) - 400.0).abs() < 80.0,
        "{visual}"
    );
    let wave = visual["waveform"].as_array().unwrap();
    assert!(
        wave.last().unwrap()[1].as_f64().unwrap() > wave[0][1].as_f64().unwrap() * 5.0,
        "the waveform preserves the source envelope"
    );
    assert!(contains(&visual["bounds"], &visual["waveform_rect"]));
    assert!(contains(&visual["bounds"], &visual["spectrum_rect"]));
    for row in first["library"]["samples"].as_array().unwrap() {
        assert!(
            row["rect"]["max"][0].as_f64().unwrap() < visual["bounds"]["min"][0].as_f64().unwrap(),
            "visuals belong to the right inspector, not result rows"
        );
    }
    // Inspect the actual submitted paint as well as the service result.
    let painted = app.command(json!({"command":"inspect","paint":true}));
    assert!(
        painted["painted_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| {
                path["points"].as_array().is_some_and(|points| {
                    points.len() >= 48
                        && points.iter().all(|point| {
                            (0..2).all(|axis| {
                                point[axis].as_f64().unwrap()
                                    >= visual["bounds"]["min"][axis].as_f64().unwrap()
                                    && point[axis].as_f64().unwrap()
                                        <= visual["bounds"]["max"][axis].as_f64().unwrap()
                            })
                        })
                })
            }),
        "the inspector submits the analyzed geometry"
    );
    // Backdrop painting must not steal metadata controls or source selection.
    app.click("File details…");
    let copied = app.click("Copy path");
    assert_eq!(copied["clipboard_text"], low.to_string_lossy().as_ref());
    app.click("High tone.wav");
    app.click("Low tone.wav");
    app.click("High tone.wav");
    let second = ready(&mut app, &high);
    let other = &second["library"]["visual_analysis"];
    assert_eq!(other["source_frames"], 24_000);
    assert!(
        (dominant_frequency(other) - 1_600.0).abs() < 320.0,
        "{other}"
    );
    assert_ne!(other["waveform"], visual["waveform"]);
    assert_ne!(other["spectrum"], visual["spectrum"]);
    unchanged(&second, &before);
    app.click("Search samples");
    let hidden = app.command(json!({"command":"text","text":"no match"}));
    assert!(hidden["library"]["selected"].is_null());
    assert!(hidden["library"]["visual_analysis"].is_null());
    unchanged(&hidden, &before);
    app.quit();
}

#[test]
fn failed_visual_analysis_is_local_retryable_and_does_not_publish_sample_edits() {
    let mut app = Harness::with_virtual_audio(None, None);
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    let root = app.directory.path().join("Repair library");
    std::fs::create_dir(&root).unwrap();
    let source = root.join("Repair.wav");
    std::fs::write(&source, b"broken audio").unwrap();
    let before = index(&mut app, &root, 1);
    app.click("Repair.wav");
    let failed = common::wait_until(&mut app, |state| {
        state["library"]["visual_analysis"]["state"] == "failed"
            && state["library"]["metadata"]["state"] == "failed"
    });
    let selected = failed["library"]["selected"].clone();
    assert!(failed["library"]["visual_analysis"]["waveform_rect"].is_null());
    assert!(failed["library"]["visual_analysis"]["spectrum_rect"].is_null());
    unchanged(&failed, &before);
    app.click("File details…");
    tone(&source, 800.0, 0.3, 24_000);
    app.click("Retry visual analysis");
    let repaired = ready(&mut app, &source);
    assert_eq!(repaired["library"]["selected"], selected);
    assert_eq!(
        repaired["library"]["indexed"], 1,
        "retry does not scan folders"
    );
    assert!((dominant_frequency(&repaired["library"]["visual_analysis"]) - 800.0).abs() < 160.0);
    unchanged(&repaired, &before);
    assert_eq!(
        repaired["library"]["metadata"]["state"], "failed",
        "visual retry remains independent of the metadata cache"
    );
    app.click("Add to Project Samples");
    let added = common::wait_until(&mut app, |state| {
        state["project"]["samples"]
            .as_array()
            .is_some_and(|samples| samples.len() == 1)
            && state["library"]["pending_import"].is_null()
    });
    assert_eq!(added["project"]["tracks"], before["project"]["tracks"]);
    assert_eq!(
        added["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1,
        "only explicit sample admission edits the project"
    );
    app.quit();
}
