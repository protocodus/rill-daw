//! A rejected native view cannot hold saving/rendering hostage before it opens.
//! A running editor's failed state capture must still cancel the operation.
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
    process::Command,
    time::{Duration, Instant},
};

fn production_helper() -> PathBuf {
    let directory = std::env::var_os("RILL_E2E_BINARY")
        .map(PathBuf::from)
        .map(|binary| binary.parent().unwrap().to_owned())
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
        "Build workspace binaries first: {}",
        path.display()
    );
    path
}

struct Fixture {
    plugin: EditorProbeFixture,
    workers: tempfile::TempDir,
    helper: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let plugin = EditorProbeFixture::build();
        let workers = tempfile::tempdir().unwrap();
        let source = workers.path().join("editor-unavailable.rs");
        std::fs::write(&source, r###"
use std::{io::{BufRead, Write}, time::{Duration, Instant}};
fn main() {
    if !std::env::args().any(|arg| arg == "--editor") {
        let status = std::process::Command::new(__REAL_HELPER__)
            .args(std::env::args().skip(1)).status().unwrap();
        std::process::exit(status.code().unwrap_or(1));
    }
    let directory = std::env::current_exe().unwrap().parent().unwrap().to_owned();
    let mut input = std::io::stdin().lock().lines();
    assert!(input.next().unwrap().unwrap().contains("\"command\":\"open\""));
    let mut output = std::io::stdout().lock();
    if directory.join("running-editor").exists() {
        writeln!(output, r##"{{"event":"ready","parameters":[{{"id":7,"name":"Level","value":0.5,"default":0.5,"unit":"","steps":0,"read_only":false}}]}}"##).unwrap();
        writeln!(output, r##"{{"event":"parameter_changed","id":7,"value":0.75}}"##).unwrap();
        output.flush().unwrap();
        for command in input {
            let command = command.unwrap();
            if command.contains("\"command\":\"save_state\"") || command.contains("\"command\":\"close\"") {
                // Identical words after Ready are a genuine state-capture failure.
                writeln!(output, r##"{{"event":"error","message":"Failed to create editor view"}}"##).unwrap();
                output.flush().unwrap();
                return;
            }
        }
    } else {
        std::fs::write(directory.join("opening"), "opening").unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !directory.join("release").exists() {
            assert!(Instant::now() < deadline, "test did not release startup");
            std::thread::sleep(Duration::from_millis(5));
        }
        let message = std::fs::read_to_string(directory.join("startup-error")).unwrap();
        // The fixed test messages contain no JSON escapes.
        writeln!(output, "{{\"event\":\"error\",\"message\":\"{message}\"}}").unwrap();
        output.flush().unwrap();
    }
}
"###.replace("__REAL_HELPER__", &format!("{:?}", production_helper()))).unwrap();
        let helper = workers.path().join(if cfg!(windows) {
            "editor-unavailable.exe"
        } else {
            "editor-unavailable"
        });
        let result = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "-C", "opt-level=1"])
            .arg(&source)
            .arg("-o")
            .arg(&helper)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Self {
            plugin,
            workers,
            helper,
        }
    }

    fn app(&self) -> Harness {
        let mut app = Harness::with_plugin_host(self.plugin.path.parent().unwrap(), &self.helper);
        let mut project = Project::new();
        let track = project.add_midi_track("Editorless instrument");
        project
            .set_instrument(
                track,
                Instrument::Vst3 {
                    path: self.plugin.path.to_string_lossy().into_owned(),
                    class_id: Some(CLASS_ID.into()),
                },
            )
            .unwrap();
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            "Audible phrase",
            0.0,
            4.0,
            vec![Note::new(60, 0.0, 2.0, 100)],
        ));
        let path = app.directory.path().join("editorless.rill");
        project.save(&path).unwrap();
        app.command(json!({"command":"open","path":path}));
        app
    }
}

fn wait(app: &mut Harness, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if ready(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Editor transition timed out: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn open_editor(app: &mut Harness) -> Value {
    app.click("Window");
    app.click("Instrument");
    app.click("Open editor")
}

fn save_as(app: &mut Harness, path: &Path) -> Value {
    app.command(json!({"command":"file_dialog","kind":"save","path":path}));
    app.click("File");
    app.click("Save as…")
}

fn export(app: &mut Harness, path: &Path) -> Value {
    app.command(json!({"command":"file_dialog","kind":"export","path":path}));
    app.click("File");
    app.click("Export WAV…");
    app.click("Export…")
}

#[test]
fn unavailable_native_views_resume_pending_save_and_export_with_real_audible_dsp() {
    let fixture = Fixture::new();
    for (operation, message) in [
        ("save", "Failed to create editor view"),
        ("export", "This plug-in has no native editor"),
    ] {
        let _ = std::fs::remove_file(fixture.workers.path().join("release"));
        std::fs::write(fixture.workers.path().join("startup-error"), message).unwrap();
        let mut app = fixture.app();
        let baseline = app.inspect();
        assert_eq!(open_editor(&mut app)["native_editor"]["opening"], true);
        let saved = app.directory.path().join("saved-without-editor.rill");
        let wav = app.directory.path().join("audible-without-editor.wav");
        let pending = if operation == "save" {
            save_as(&mut app, &saved)
        } else {
            export(&mut app, &wav)
        };
        assert_eq!(pending["project_pending"], true);
        assert_eq!(pending["native_editor"]["opening"], true);
        std::fs::write(fixture.workers.path().join("release"), "release").unwrap();
        let completed = if operation == "save" {
            app.command(json!({"command":"wait_project"}))
        } else {
            app.command(json!({"command":"wait_export"}))
        };
        assert_eq!(completed["error"], false, "{}", completed["status"]);
        assert_eq!(completed["native_editor"]["open"], false);
        assert_eq!(completed["native_editor"]["opening"], false);
        assert!(
            completed["native_editor"]["error"]
                .as_str()
                .unwrap()
                .contains("Save and export remain available")
        );
        assert_eq!(completed["project"], baseline["project"]);
        if operation == "save" {
            export(&mut app, &wav);
            app.command(json!({"command":"wait_export"}));
        } else {
            save_as(&mut app, &saved);
            app.command(json!({"command":"wait_project"}));
        }
        let recalled = Project::load(&saved).unwrap();
        assert_eq!(serde_json::to_value(recalled).unwrap(), baseline["project"]);
        let audio = AudioData::load_wav(&wav).unwrap();
        assert!(audio.samples.iter().all(|sample| sample.is_finite()));
        assert!(
            audio.samples.iter().any(|sample| sample.abs() > 0.01),
            "real native DSP must remain audible"
        );
        assert!(
            audio.samples[48_000 * 2 + 1024..]
                .iter()
                .all(|sample| sample.abs() < 0.0001)
        );
        assert_eq!(app.inspect()["device_open"], false);
        app.quit();
    }
}

#[test]
fn running_editor_capture_failure_still_cancels_save_and_preserves_known_changes() {
    let fixture = Fixture::new();
    std::fs::write(fixture.workers.path().join("running-editor"), "running").unwrap();
    let mut app = fixture.app();
    open_editor(&mut app);
    let edited = wait(&mut app, |state| {
        state["native_editor"]["open"] == true
            && state["project"]["tracks"][0]["instrument_parameters"]
                == json!([{"id":7,"value":0.75}])
    });
    let saved = app.directory.path().join("must-not-save.rill");
    save_as(&mut app, &saved);
    let failed = wait(&mut app, |state| {
        state["project_pending"] == false && state["error"] == true
    });
    assert!(!saved.exists());
    assert!(
        failed["status"]
            .as_str()
            .unwrap()
            .contains("Plugin state could not be saved")
    );
    assert_eq!(failed["project"], edited["project"]);
    assert_eq!(failed["modified"], true);
    let recovery = Project::load(app.directory.path().join("scratch/recovery.daw.json")).unwrap();
    assert_eq!(serde_json::to_value(recovery).unwrap(), edited["project"]);
    app.quit();
}
