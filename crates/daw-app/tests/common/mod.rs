//! Shared driver for the real `rill --headless` application process.

#![allow(dead_code)] // Integration targets exercise different helpers.

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use tempfile::TempDir;

pub struct Harness {
    pub directory: TempDir,
    vst_path: Option<std::path::PathBuf>,
    plugin_host: Option<std::path::PathBuf>,
    plugin_previews: bool,
    virtual_audio: bool,
    child: Child,
    input: ChildStdin,
    output: Receiver<String>,
}

impl Harness {
    pub fn new() -> Self {
        Self::launch(None)
    }

    pub fn with_vst_path(path: &std::path::Path) -> Self {
        Self::launch(Some(path))
    }

    /// Supply a test-only isolation worker to this app child, without changing
    /// process-global environment or the production helper executable.
    pub fn with_plugin_host(path: &std::path::Path, host: &std::path::Path) -> Self {
        let directory = tempfile::tempdir().expect("create isolated E2E directory");
        Self::launch_configured(Some(path), directory, None, Some(host), false, false)
    }

    /// Like `with_plugin_host`, with the desktop's automatic background
    /// capture of missing previews. Only supply a test editor worker here.
    pub fn with_plugin_previews(path: &std::path::Path, host: &std::path::Path) -> Self {
        let directory = tempfile::tempdir().expect("create isolated E2E directory");
        Self::launch_configured(Some(path), directory, None, Some(host), true, false)
    }

    /// A deterministic device-free sink sharing the production render worker,
    /// bounded audio queue and callback consumer. Only explicit pump commands
    /// consume audio; no host device or desktop is touched.
    pub fn with_virtual_audio(
        vst_path: Option<&std::path::Path>,
        host: Option<&std::path::Path>,
    ) -> Self {
        let directory = tempfile::tempdir().expect("create isolated virtual-audio directory");
        Self::launch_configured(vst_path, directory, None, host, false, true)
    }

    /// A real CPAL/ALSA stream that discards all samples. The child sees only
    /// this isolated null PCM configuration, never a host sound device.
    #[cfg(target_os = "linux")]
    pub fn with_null_audio() -> Self {
        let directory = tempfile::tempdir().expect("create isolated audio E2E directory");
        let config = directory.path().join("alsa-null.conf");
        std::fs::write(
            &config,
            "pcm.!default { type null hint { show on description \"Rill null output\" } }\n",
        )
        .expect("write isolated ALSA configuration");
        Self::launch_configured(None, directory, Some(&config), None, false, false)
    }

    /// Keep ALSA confined to a null PCM while requesting a disconnected output.
    #[cfg(target_os = "linux")]
    pub fn with_missing_output() -> Self {
        let directory = tempfile::tempdir().expect("create isolated audio failure directory");
        let config = directory.path().join("alsa-null.conf");
        std::fs::write(
            &config,
            "pcm.!default { type null hint { show on description \"Rill null output\" } }\n",
        )
        .unwrap();
        std::fs::create_dir_all(directory.path().join("config")).unwrap();
        std::fs::write(
            directory.path().join("config/settings.json"),
            r#"{"device_name":"Disconnected Rill E2E output"}"#,
        )
        .unwrap();
        Self::launch_configured(None, directory, Some(&config), None, false, false)
    }

    fn launch(vst_path: Option<&std::path::Path>) -> Self {
        let directory = tempfile::tempdir().expect("create isolated E2E directory");
        Self::launch_in(vst_path, directory)
    }

    fn launch_in(vst_path: Option<&std::path::Path>, directory: TempDir) -> Self {
        Self::launch_configured(vst_path, directory, None, None, false, false)
    }

    fn launch_configured(
        vst_path: Option<&std::path::Path>,
        directory: TempDir,
        null_audio: Option<&std::path::Path>,
        plugin_host: Option<&std::path::Path>,
        plugin_previews: bool,
        virtual_audio: bool,
    ) -> Self {
        let binary = std::env::var_os("RILL_E2E_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_rill").into());
        let mut command = Command::new(binary);
        command
            .arg("--headless")
            .env("RILL_AUTOMATION_DIR", directory.path())
            .env("RILL_CONFIG_DIR", directory.path().join("config"))
            .env("RILL_CACHE_DIR", directory.path().join("cache"))
            .env_remove("RILL_E2E_VIRTUAL_AUDIO")
            .env_remove("RILL_E2E_AUDIO_DEVICE")
            .env_remove("RILL_E2E_PLUGIN_PREVIEWS")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if virtual_audio {
            command.env("RILL_E2E_VIRTUAL_AUDIO", "1");
        }
        if plugin_previews {
            command.env("RILL_E2E_PLUGIN_PREVIEWS", "1");
        }
        if let Some(path) = vst_path {
            command.env("VST3_PATH", path);
        }
        if let Some(path) = plugin_host {
            command.env("DAW_PLUGIN_HOST", path);
        }
        if let Some(path) = null_audio {
            command
                .env("ALSA_CONFIG_PATH", path)
                .env("RILL_E2E_AUDIO_DEVICE", "1");
        }
        let mut child = command
            .spawn()
            .expect("launch the production Rill executable");
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let harness = Self {
            directory,
            vst_path: vst_path.map(std::path::Path::to_owned),
            plugin_host: plugin_host.map(std::path::Path::to_owned),
            plugin_previews,
            virtual_audio,
            child,
            input,
            output: rx,
        };
        let ready = harness.response();
        assert_eq!(ready["event"], "ready", "{ready}");
        assert_eq!(ready["ok"], true, "{ready}");
        assert_eq!(ready["state"]["device_open"], false);
        assert!(
            ready["state"]["project"]["tracks"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        harness
    }

    /// Restart the production process with the same isolated settings and cache.
    pub fn restart(mut self) -> Self {
        let result = self.raw_command(json!({"command":"quit"}));
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["event"], "exiting");
        let directory = std::mem::replace(&mut self.directory, tempfile::tempdir().unwrap());
        let vst_path = self.vst_path.clone();
        let plugin_host = self.plugin_host.clone();
        let (plugin_previews, virtual_audio) = (self.plugin_previews, self.virtual_audio);
        self.wait_exit();
        Self::launch_configured(
            vst_path.as_deref(),
            directory,
            None,
            plugin_host.as_deref(),
            plugin_previews,
            virtual_audio,
        )
    }

    fn response(&self) -> Value {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(125))
            .expect("application did not return JSON within 125 seconds");
        serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("Invalid application output {line:?}: {error}"))
    }

    pub fn raw_command(&mut self, command: Value) -> Value {
        serde_json::to_writer(&mut self.input, &command).unwrap();
        writeln!(&mut self.input).unwrap();
        self.input.flush().unwrap();
        self.response()
    }

    pub fn command(&mut self, command: Value) -> Value {
        let response = self.raw_command(command.clone());
        assert_eq!(response["ok"], true, "Command {command} failed: {response}");
        response["state"].clone()
    }

    pub fn click(&mut self, label: &str) -> Value {
        self.command(json!({"command": "click", "label": label}))
    }

    pub fn inspect(&mut self) -> Value {
        self.command(json!({"command": "inspect"}))
    }

    pub fn key(&mut self, key: &str) -> Value {
        self.command(json!({"command": "key", "key": key}))
    }

    pub fn quit(mut self) {
        let result = self.raw_command(json!({"command": "quit"}));
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["event"], "exiting");
        assert_eq!(result["state"]["device_open"], false);
        self.wait_exit();
    }

    pub fn wait_exit(mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "Application exited with {status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Application failed to exit after quit"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Failed assertions must not leave an application process or temp session.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Metadata-only multi-class bundle. It contains no executable plug-in code.
pub fn plugin_fixture() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    let resources = root.path().join("RillFixture.vst3/Contents/Resources");
    std::fs::create_dir_all(&resources).unwrap();
    let classes = [("Rill Fixture A", "Instrument", "11111111111111111111111111111111"),
                   ("Rill Fixture B", "Instrument", "22222222222222222222222222222222"),
                   ("Rill Fixture FX", "Fx", "33333333333333333333333333333333")]
        .into_iter().map(|(name,category,id)| json!({"CID":id,"Category":"Audio Module Class","Name":name,"Sub Categories":[category],"Version":"1.0","SDKVersion":"VST3.7.9","Cardinality":2147483647})).collect::<Vec<_>>();
    std::fs::write(resources.join("moduleinfo.json"),json!({"Name":"Fixture","Version":"1.0","Factory Info":{"Vendor":"Rill tests"},"Classes":classes}).to_string()).unwrap();
    root
}

pub fn fixture_library(app: &mut Harness) {
    app.click("Window");
    app.click("Library");
    app.click("Instruments");
    let state = app.inspect();
    if state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "Search instruments")
    {
        app.click("Search instruments");
        app.command(json!({"command":"text","text":"Rill Fixture"}));
    }
    wait_visible(app, "Rill Fixture A");
}

pub fn wait_visible(app: &mut Harness, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let state = app.inspect();
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == label)
        {
            return;
        }
        assert!(Instant::now() < deadline, "Missing {label}: {state}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait for a rendered device card rather than its always-present disabled action.
/// A search field may have exactly the same visible text as the requested device.
pub fn wait_library_device(app: &mut Harness, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let state = app.inspect();
        if state["library"]["cards"]
            .as_array()
            .is_some_and(|cards| cards.iter().any(|card| card["name"] == name))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Missing device card {name}: {}",
            state["library"]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_until(app: &mut Harness, condition: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let state = app.inspect();
        if condition(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Condition not reached: {}",
            state["library"]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Rescan with the library's Refresh button and wait for the catalog, e.g.
/// after a test seeds the preview cache. The library stays open.
pub fn refresh_library(app: &mut Harness) -> Value {
    wait_until(app, |state| state["plugin_scanning"] == false);
    app.click("Window");
    app.click("Library");
    app.click("Refresh");
    wait_until(app, |state| state["plugin_scanning"] == false)
}

/// Edit the real plug-in knob's visible number, with mouse focus/commit and no
/// select-all or Enter shortcut. The shared Number selects the complete value.
pub fn edit_plugin_parameter_value(app: &mut Harness, state: &Value, percent: &str) -> Value {
    let rect = &state["native_editor"]["parameter_control"]["number"];
    let min_x = rect["min"][0].as_f64().expect("plug-in numeric field");
    let min_y = rect["min"][1].as_f64().unwrap();
    let max_x = rect["max"][0].as_f64().unwrap();
    let max_y = rect["max"][1].as_f64().unwrap();
    let clip = &state["native_editor"]["parameter_control"]["clip"];
    assert!(
        min_x >= clip["min"][0].as_f64().unwrap()
            && min_y >= clip["min"][1].as_f64().unwrap()
            && max_x <= clip["max"][0].as_f64().unwrap()
            && max_y <= clip["max"][1].as_f64().unwrap(),
        "Exact parameter entry must be fully visible: {rect}, clipped to {clip}"
    );
    let x = (min_x + max_x) * 0.5;
    let y = (min_y + max_y) * 0.5;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"text","text":percent}));
    app.click("Normalized value")
}
