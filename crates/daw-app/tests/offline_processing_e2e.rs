//! Offline exports allow expensive native blocks without relaxing live playback.
//! The native fixture also rejects process calls unless VST3 kOffline is sent.
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
    time::{Duration, Instant},
};

fn helper() -> PathBuf {
    let directory = std::env::var_os("RILL_E2E_BINARY")
        .map(|path| PathBuf::from(path).parent().unwrap().to_owned())
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_owned()
        });
    let path = directory.join(if cfg!(windows) {
        "daw-plugin-host.exe"
    } else {
        "daw-plugin-host"
    });
    assert!(
        path.is_file(),
        "Build the workspace binaries before this test"
    );
    path
}

fn open_native_song(app: &mut Harness, plugin: &EditorProbeFixture) -> Value {
    let mut project = Project::new();
    let track = project.add_midi_track("Offline synth");
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
        "Short phrase",
        0.0,
        0.25,
        vec![Note::new(60, 0.0, 0.125, 100)],
    ));
    let path = app.directory.path().join("Offline song.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open", "path":path}))
}

fn start_export(app: &mut Harness, path: &Path) {
    app.command(json!({"command":"file_dialog", "kind":"export", "path":path}));
    app.click("File");
    app.click("Export WAV…");
    app.click("Export…");
}

#[test]
fn offline_export_accepts_a_slow_first_block_and_advertises_offline_processing() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_millis(400));
    let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &helper());
    let original = open_native_song(&mut app, &plugin);
    let path = app.directory.path().join("Slow first block.wav");
    start_export(&mut app, &path);
    let completed = app.command(json!({"command":"wait_export", "timeout_ms":15000}));
    assert_eq!(completed["error"], false, "{}", completed["status"]);
    assert_eq!(completed["export_pending"], false);
    assert_eq!(completed["project"], original["project"]);
    assert_eq!(completed["history"], original["history"]);
    let audio = AudioData::load_wav(path).unwrap();
    assert_eq!(audio.sample_rate, 48_000);
    assert_eq!(audio.channels, 2);
    assert!(audio.samples.iter().all(|sample| sample.is_finite()));
    assert!(
        audio.samples.iter().any(|sample| sample.abs() > 0.01),
        "Native offline processor must produce its note, not a silent fallback"
    );
    app.quit();
}

#[test]
fn offline_hung_block_still_fails_with_a_bounded_deadline_and_preserves_the_old_wav() {
    let plugin = EditorProbeFixture::build_for_offline(Duration::from_secs(10));
    let mut app = Harness::with_plugin_host(plugin.path.parent().unwrap(), &helper());
    let original = open_native_song(&mut app, &plugin);
    let path = app.directory.path().join("Existing output.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![0.23; 64],
    }
    .write_wav(&path)
    .unwrap();
    let old_wav = std::fs::read(&path).unwrap();
    let started = Instant::now();
    start_export(&mut app, &path);
    app.raw_command(json!({"command":"wait_export", "timeout_ms":15000}));
    let failed = app.inspect();
    assert_eq!(failed["error"], true, "{failed}");
    assert_eq!(failed["export_pending"], false);
    assert!(
        started.elapsed() >= Duration::from_secs(5),
        "Offline work must get its own budget"
    );
    assert!(
        started.elapsed() < Duration::from_secs(12),
        "Hung native work must stay bounded"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        old_wav,
        "Failed export must not replace an existing file"
    );
    assert_eq!(failed["project"], original["project"]);
    assert_eq!(failed["history"], original["history"]);
    let details = app.click("Details");
    assert!(
        details["error_details"]["details"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
    app.key("Escape");
    app.quit();
}
