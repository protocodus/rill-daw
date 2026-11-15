//! Sample audition through the real browser, isolated decoder and output bus.
mod common;
use common::Harness;
use daw_core::{AudioData, Project};
use serde_json::{Value, json};
use std::path::Path;

fn tone(path: &Path, frequency: f32) {
    AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: (0..44_100)
            .map(|frame| 0.25 * (std::f32::consts::TAU * frequency * frame as f32 / 44_100.0).sin())
            .collect(),
    }
    .write_wav(path)
    .unwrap();
}

fn browser(app: &mut Harness) {
    app.click("Window");
    app.click("Device browser");
    app.click("Samples");
}

fn samples(state: &Value) -> Vec<f64> {
    state["audio_output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| frame[0].as_f64().unwrap())
        .collect()
}

fn peak(signal: &[f64]) -> f64 {
    signal.iter().copied().map(f64::abs).fold(0.0, f64::max)
}

fn frequency(signal: &[f64], rate: f64) -> f64 {
    signal
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count() as f64
        * rate
        / signal.len() as f64
}

#[test]
fn preview_switch_stop_and_document_change_preserve_song_and_transport() {
    let mut app = Harness::with_virtual_audio(None, None);
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    let first = app.directory.path().join("first.wav");
    let second = app.directory.path().join("second.wav");
    tone(&first, 440.0);
    tone(&second, 880.0);
    let mut project = Project::new();
    project
        .add_project_sample("First tone", first.to_string_lossy())
        .unwrap();
    project
        .add_project_sample("Second tone", second.to_string_lossy())
        .unwrap();
    let path = app.directory.path().join("audition.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(
        json!({"command":"song_tool","name":"transport","arguments":{"action":"seek","beat":7.5}}),
    );
    browser(&mut app);
    let before = app.inspect();
    app.click("First tone");
    app.click("Preview");
    common::wait_until(&mut app, |state| {
        state["engine"]["sample_audition_active"] == true
    });
    let playing = app.command(json!({"command":"pump_audio","frames":8192}));
    let output = samples(&playing);
    assert!(peak(&output) > 0.2);
    assert!((frequency(&output, 48_000.0) - 440.0).abs() < 15.0);
    assert_eq!(playing["engine"]["playing"], false);
    assert_eq!(playing["project"], before["project"]);
    assert_eq!(playing["history"], before["history"]);
    assert_eq!(playing["engine"]["position_beats"], 7.5);

    // Selection changes stop the old source; selecting alone never autoplays.
    app.click("Second tone");
    let stopped = app.command(json!({"command":"pump_audio","frames":1024}));
    assert_eq!(
        peak(&samples(&stopped)[128..]),
        0.0,
        "only the short stop ramp may remain"
    );
    assert_eq!(stopped["engine"]["sample_audition_active"], false);
    app.click("Preview");
    common::wait_until(&mut app, |state| {
        state["engine"]["sample_audition_active"] == true
    });
    let second = app.command(json!({"command":"pump_audio","frames":8192}));
    assert!((frequency(&samples(&second), 48_000.0) - 880.0).abs() < 15.0);
    app.click("Stop preview");
    let stopped = app.command(json!({"command":"pump_audio","frames":1024}));
    assert_eq!(
        peak(&samples(&stopped)[128..]),
        0.0,
        "only the short stop ramp may remain"
    );
    assert_eq!(stopped["project"], before["project"]);
    assert_eq!(stopped["history"], before["history"]);

    // The document replacement cancels pending decoding as well as playback.
    app.click("Preview");
    let empty = app.directory.path().join("empty.rill");
    Project::new().save(&empty).unwrap();
    app.command(json!({"command":"open","path":empty}));
    let replaced = app.inspect();
    assert_ne!(replaced["song_session"]["id"], before["song_session"]["id"]);
    assert_ne!(replaced["engine"]["sample_audition_active"], true);
    app.quit();
}

#[test]
fn audio_formats_preview_and_probe_on_demand_without_importing_media() {
    let mut app = Harness::with_virtual_audio(None, None);
    app.command(json!({"command":"viewport","width":1280,"height":900}));
    let library = app.directory.path().join("Library");
    std::fs::create_dir(&library).unwrap();
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../daw-core/tests/fixtures/audio-import");
    let names = [
        "tone.wav",
        "tone.mp3",
        "tone.flac",
        "tone.ogg",
        "tone.aiff",
        "tone.aifc",
        "tone.aac",
        "tone-aac.m4a",
        "tone-alac.m4a",
        "tone.caf",
        "tone-alac.caf",
    ];
    for name in names {
        std::fs::copy(fixtures.join(name), library.join(name)).unwrap();
    }
    browser(&mut app);
    app.click("Folders…");
    app.click("Sample folder path");
    app.command(json!({"command":"text","text":library}));
    app.click("Add folder");
    app.click("Done");
    app.click("Scan folders");
    common::wait_until(&mut app, |state| {
        state["library"]["indexed"] == names.len() && state["library"]["scanning"] == false
    });
    let before = app.inspect();
    for (index, name) in names.into_iter().enumerate() {
        if index > 0 {
            app.click("Clear search");
        }
        app.click("Search samples");
        // Keep the query distinct from the row label: automation can also
        // discover the editable search field by its current text.
        app.command(json!({"command":"text","text":name.trim_start_matches("tone")}));
        app.click(name);
        let metadata = common::wait_until(&mut app, |state| {
            state["library"]["metadata"]["state"] == "ready"
        });
        let metadata = &metadata["library"]["metadata"];
        assert_eq!(metadata["sample_rate"], 48_000, "{name}");
        assert_eq!(metadata["channels"], 2, "{name}");
        assert_eq!(
            metadata["file_bytes"],
            std::fs::metadata(fixtures.join(name)).unwrap().len(),
            "{name}"
        );
        assert!(!metadata["codec"].as_str().unwrap().is_empty(), "{name}");
        if name != "tone.aac" {
            assert!(
                metadata["duration_seconds"].as_f64().unwrap() > 0.1,
                "{name}"
            );
        }
        app.click("Preview");
        common::wait_until(&mut app, |state| {
            state["engine"]["sample_audition_active"] == true
        });
        let played = app.command(json!({"command":"pump_audio","frames":4096}));
        assert!(peak(&samples(&played)) > 0.01, "{name}");
        assert_eq!(played["project"], before["project"]);
        assert_eq!(played["history"], before["history"]);
        app.click("Stop preview");
    }
    app.quit();
}

#[test]
fn leaving_sample_views_stops_preview_without_a_transport_stop() {
    let mut app = Harness::with_virtual_audio(None, None);
    let source = app.directory.path().join("tone.wav");
    tone(&source, 440.0);
    let mut project = Project::new();
    project.add_instrument_track("Preview destination");
    project
        .add_project_sample("Preview tone", source.to_string_lossy())
        .unwrap();
    let song = app.directory.path().join("navigation.rill");
    project.save(&song).unwrap();
    app.command(json!({"command":"open","path":song}));
    app.command(
        json!({"command":"song_tool","name":"transport","arguments":{"action":"seek","beat":7.5}}),
    );
    browser(&mut app);
    app.click("Preview tone");
    let before = app.inspect();
    app.click("Preview");
    common::wait_until(&mut app, |state| {
        state["engine"]["sample_audition_active"] == true
    });
    app.command(json!({"command":"pump_audio","frames":1024}));
    app.click("Window");
    app.click("Instrument");
    let stopped = app.command(json!({"command":"pump_audio","frames":1024}));
    assert_eq!(stopped["popups"]["instrument"], true);
    assert_eq!(stopped["engine"]["sample_audition_active"], false);
    assert_eq!(peak(&samples(&stopped)[128..]), 0.0);
    assert_eq!(stopped["engine"]["position_beats"], 7.5);
    assert_eq!(stopped["engine"]["playing"], false);
    assert_eq!(stopped["project"], before["project"]);
    assert_eq!(stopped["history"], before["history"]);
    app.quit();
}
