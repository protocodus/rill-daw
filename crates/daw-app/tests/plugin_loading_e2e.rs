//! The production app and isolation worker load a real native VST3 without an editor.
//! The fixture exits its process if createView is called, so this also guards metadata
//! reads and offline rendering against accidentally constructing plugin GUI objects.
mod common;
#[allow(dead_code)] // The direct plugin tests also exercise the immediate lifecycle variant.
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::Harness;
use daw_core::{AudioData, Clip, Instrument, Note, Project};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn production_helper() -> PathBuf {
    // Use the matching application distribution, never an ambient protocol substitute.
    let directory = if let Some(binary) = std::env::var_os("RILL_E2E_BINARY") {
        PathBuf::from(binary).parent().unwrap().to_owned()
    } else {
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned()
    };
    let helper = directory.join(if cfg!(windows) {
        "daw-plugin-host.exe"
    } else {
        "daw-plugin-host"
    });
    assert!(
        helper.is_file(),
        "Build workspace binaries before this E2E test: {}",
        helper.display()
    );
    helper
}

fn window(app: &mut Harness, item: &str) -> Value {
    app.click("Window");
    app.click(item)
}

fn wait_parameters(app: &mut Harness) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        assert_eq!(state["device_open"], false);
        assert_eq!(state["native_editor"]["open"], false);
        assert_eq!(state["native_editor"]["opening"], false);
        assert!(
            state["native_editor"]["error"].is_null(),
            "Native parameter loading failed: {}",
            state["native_editor"]
        );
        if state["native_editor"]["parameter_count"] == 1 {
            assert_eq!(state["native_editor"]["selected"], 7);
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Native parameter loading timed out: {}",
            state["native_editor"]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn export(app: &mut Harness, path: &Path) -> AudioData {
    app.command(json!({"command":"export", "path":path}));
    let state = app.command(json!({"command":"wait_export"}));
    assert_eq!(state["export_pending"], false);
    assert_eq!(state["error"], false, "{}", state["status"]);
    assert_eq!(state["device_open"], false);
    assert_eq!(state["native_editor"]["open"], false);
    assert!(state["status"].as_str().unwrap().starts_with("Exported "));
    let audio = AudioData::load_wav(path).unwrap();
    assert_eq!(audio.channels, 2);
    assert_eq!(audio.sample_rate, 48_000);
    assert!(audio.duration_seconds() >= 2.0);
    assert!(audio.samples.iter().all(|sample| sample.is_finite()));
    assert!(
        peak(&audio) > 0.001,
        "A crashed worker must not pass as silence"
    );
    // The note lasts one second. Its note-off must also reach the native processor.
    assert!(
        audio.samples[48_000 * 2 + 1024..]
            .iter()
            .all(|sample| sample.abs() < 0.0001),
        "The fixture must stop sounding after the MIDI note-off"
    );
    audio
}

fn peak(audio: &AudioData) -> f32 {
    audio
        .samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0, f32::max)
}

#[test]
fn native_instrument_metadata_parameter_edit_export_and_recall_never_construct_an_editor() {
    // Native instrument setup is allowed to exceed the audio-block IPC deadline.
    let fixture = EditorProbeFixture::build_with_lifecycle_delay(Duration::from_millis(200));
    let mut app = Harness::with_plugin_host(fixture.path.parent().unwrap(), &production_helper());
    window(&mut app, "Library");
    app.click("Search instruments");
    app.command(json!({"command":"text", "text":"No Editor Probe"}));
    common::wait_library_device(&mut app, "No Editor Probe");
    let catalog = app.inspect();
    let scanned = catalog["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["name"] == "No Editor Probe")
        .expect("The real bundle must be discoverable as an instrument");
    assert_eq!(scanned["class_id"], CLASS_ID);

    // Opening a project exercises assignment without the device browser's intentional
    // native-editor autolaunch. This test must never create a desktop window.
    let mut project = Project::new();
    let track = project.add_midi_track("Native fixture");
    project
        .set_instrument(
            track,
            Instrument::Vst3 {
                path: fixture.path.to_string_lossy().into_owned(),
                class_id: Some(CLASS_ID.to_owned()),
            },
        )
        .unwrap();
    project.track_mut(track).unwrap().clips.push(Clip::midi(
        "Native phrase",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 2.0, 100)],
    ));
    let session = app.directory.path().join("native-instrument.rill");
    project.save(&session).unwrap();
    app.command(json!({"command":"open", "path":session}));
    app.click("Native phrase");
    let baseline_path = app.directory.path().join("baseline.wav");
    let baseline = export(&mut app, &baseline_path);

    window(&mut app, "Instrument");
    app.click("Automate…");
    let parameters = wait_parameters(&mut app);
    assert_eq!(parameters["native_editor"]["selected_value"], 0.5);
    let rect = &parameters["instrument_parameter_rect"];
    let left = rect["min"][0].as_f64().unwrap();
    let right = rect["max"][0].as_f64().unwrap();
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    let edited = app.command(json!({"command":"pointer", "phase":"click",
        "x":left + (right - left) * 0.2, "y":y}));
    let value = edited["native_editor"]["selected_value"].as_f64().unwrap();
    assert!(
        (0.05..0.45).contains(&value),
        "Slider did not attenuate Level: {value}"
    );
    assert_eq!(edited["native_editor"]["last_touched"], 7);
    let changed_path = app.directory.path().join("changed.wav");
    let changed = export(&mut app, &changed_path);
    let ratio = f64::from(peak(&changed) / peak(&baseline));
    assert!(
        (ratio - value / 0.5).abs() < 0.02,
        "The native processor must receive the edited parameter: ratio={ratio}, value={value}"
    );

    app.command(json!({"command":"save", "path":session}));
    let saved = Project::load(&session).unwrap();
    let parameter = saved.tracks[0]
        .instrument_parameters
        .iter()
        .find(|parameter| parameter.id == 7)
        .expect("The parameter override must be persisted in the archive");
    assert!((parameter.value - value).abs() < 1e-6);
    let mut app = app.restart();
    let restored = app.command(json!({"command":"open", "path":session}));
    assert_eq!(
        restored["project"]["tracks"][0]["instrument"]["Vst3"]["class_id"],
        CLASS_ID
    );
    let recalled_path = app.directory.path().join("recalled.wav");
    let recalled = export(&mut app, &recalled_path);
    assert_eq!(recalled.samples.len(), changed.samples.len());
    assert!(
        recalled
            .samples
            .iter()
            .zip(&changed.samples)
            .all(|(actual, expected)| (actual - expected).abs() < 0.0001),
        "A new app and native worker must render the same saved instrument settings"
    );
    app.quit();
}
