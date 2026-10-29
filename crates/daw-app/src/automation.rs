//! Display-free driver for the production application, used by subprocess E2E tests.
//!
//! Input and output are newline-delimited JSON. UI actions travel through egui's
//! ordinary pointer and file-drop events; export shares the native UI's worker.

use crate::{app::DawApp, settings::Settings};
use anyhow::{Context, Result, bail, ensure};
use daw_core::Project;
use eframe::egui::{self, Event, PointerButton, Pos2, RawInput, Rect, Vec2};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

const SIZE: Vec2 = Vec2::new(1600.0, 1200.0);
fn default_pixels_per_point() -> f32 {
    1.0
}
fn one_key_press() -> usize {
    1
}

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    SongTool {
        name: String,
        arguments: Value,
    },
    ApplicationTool {
        name: String,
        arguments: Value,
    },
    McpRequest {
        request: Value,
        #[serde(default)]
        application: bool,
    },
    AssistantScript {
        turns: Vec<daw_ai::ScriptedTurn>,
    },
    WaitAssistant {
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
    /// Consume the real engine queue through its callback with no audio device.
    PumpAudio {
        frames: usize,
    },
    PumpInput {
        channels: u16,
        samples: Vec<f32>,
    },
    InputError,
    MidiInput {
        bytes: Vec<u8>,
        #[serde(default)]
        offset_frames: u32,
        #[serde(default)]
        delay_frames: u32,
    },
    MidiError,
    OutputError,
    FeedInput {
        channels: u16,
        samples: Vec<f32>,
    },
    DummyAudioDevice {
        input_channels: Option<u16>,
        input_latency_frames: Option<u32>,
        output_latency_frames: Option<u32>,
    },
    Inspect {
        #[serde(default)]
        paint: bool,
    },
    /// Supply only the next native file picker's result; application menus still
    /// execute the complete document guard and background I/O path.
    FileDialog {
        kind: TestDocumentDialog,
        path: Option<PathBuf>,
        replace_existing: Option<bool>,
    },
    AudioTakeFiles {
        paths: Option<Vec<PathBuf>>,
    },
    CloseWindow,
    Focused {
        focused: bool,
    },
    SystemTheme {
        dark: bool,
    },
    Viewport {
        width: f32,
        height: f32,
        #[serde(default = "default_pixels_per_point")]
        pixels_per_point: f32,
    },
    Click {
        label: String,
        #[serde(default)]
        occurrence: usize,
    },
    Pointer {
        x: f32,
        y: f32,
        phase: PointerPhase,
        to_x: Option<f32>,
        to_y: Option<f32>,
        #[serde(default)]
        button: TestPointerButton,
        #[serde(default)]
        shift: bool,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        cmd: bool,
    },
    Scroll {
        x: f32,
        y: f32,
        delta: f32,
        #[serde(default)]
        delta_x: f32,
        #[serde(default)]
        shift: bool,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        cmd: bool,
    },
    Key {
        key: String,
        #[serde(default)]
        shift: bool,
        #[serde(default)]
        ctrl: bool,
        #[serde(default)]
        alt: bool,
        #[serde(default)]
        cmd: bool,
        /// Deliver the chord and its release before the same UI frame.
        #[serde(default)]
        release_modifiers: bool,
        #[serde(default = "one_key_press")]
        repetitions: usize,
    },
    Text {
        text: String,
    },
    /// Deliver the clipboard events emitted by the native window integration.
    Clipboard {
        action: TestClipboardAction,
        text: Option<String>,
        /// Some integrations send the key alongside the native event.
        #[serde(default)]
        with_key: bool,
    },
    Drop {
        paths: Vec<PathBuf>,
        /// Zero-based project track index. Omit to use the empty arrangement area.
        track: Option<usize>,
        #[serde(default)]
        beat: f64,
    },
    Open {
        path: PathBuf,
        #[serde(default = "wait_by_default")]
        wait: bool,
    },
    Save {
        path: PathBuf,
        #[serde(default = "wait_by_default")]
        wait: bool,
    },
    WaitProject {
        #[serde(default = "project_timeout")]
        timeout_ms: u64,
    },
    Export {
        path: PathBuf,
    },
    WaitExport {
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
    Quit,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TestDocumentDialog {
    Save,
    Open,
    Export,
    ExportMidi,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TestClipboardAction {
    Copy,
    Cut,
    Paste,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PointerPhase {
    Click,
    DoubleClick,
    Drag,
    Down,
    Move,
    Up,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TestPointerButton {
    #[default]
    Primary,
    Middle,
    Secondary,
}

impl From<TestPointerButton> for PointerButton {
    fn from(button: TestPointerButton) -> Self {
        match button {
            TestPointerButton::Primary => Self::Primary,
            TestPointerButton::Middle => Self::Middle,
            TestPointerButton::Secondary => Self::Secondary,
        }
    }
}

fn input_modifiers(shift: bool, ctrl: bool, alt: bool, cmd: bool) -> egui::Modifiers {
    egui::Modifiers {
        shift,
        ctrl,
        alt,
        mac_cmd: cmd && cfg!(target_os = "macos"),
        command: cmd || ctrl,
    }
}

fn wait_by_default() -> bool {
    true
}
fn project_timeout() -> u64 {
    120_000
}

fn default_timeout() -> u64 {
    30_000
}

struct Harness {
    tool_result: Option<Value>,
    mcp_result: Option<Value>,
    mcp_server: daw_mcp::McpServer,
    ctx: egui::Context,
    app: DawApp,
    time: f64,
    labels: Vec<(String, Rect)>,
    menu_labels: Vec<(String, Rect)>,
    window_labels: Vec<(String, Rect)>,
    inspect_paint: bool,
    painted_rects: Vec<Value>,
    painted_lines: Vec<Value>,
    painted_paths: Vec<Value>,
    painted_circles: Vec<Value>,
    painted_text: Vec<Value>,
    painted_images: Vec<Value>,
    painted_meshes: Vec<Value>,
    audio_output: Option<Vec<[f32; 2]>>,
    clipboard_text: Option<String>,
    close_requested: bool,
    focused: bool,
    system_theme: egui::Theme,
    size: Vec2,
}

impl Harness {
    fn new() -> Result<Self> {
        let directory = PathBuf::from(
            std::env::var_os("RILL_AUTOMATION_DIR")
                .context("--headless requires RILL_AUTOMATION_DIR")?,
        );
        let config = PathBuf::from(
            std::env::var_os("RILL_CONFIG_DIR")
                .context("--headless requires RILL_CONFIG_DIR inside RILL_AUTOMATION_DIR")?,
        );
        ensure!(
            directory.is_absolute() && config.is_absolute(),
            "Automation directories must be absolute"
        );
        std::fs::create_dir_all(&directory)?;
        let directory = directory.canonicalize()?;
        let config_parent = config
            .parent()
            .context("Configuration directory needs a parent")?
            .canonicalize()?;
        ensure!(
            config_parent.starts_with(&directory),
            "RILL_CONFIG_DIR parent must be inside RILL_AUTOMATION_DIR"
        );
        std::fs::create_dir_all(&config)?;
        ensure!(
            config.canonicalize()?.starts_with(&directory),
            "RILL_CONFIG_DIR must be inside RILL_AUTOMATION_DIR"
        );
        let scratch_dir = directory.join("scratch");
        std::fs::create_dir_all(&scratch_dir)?;
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        app.virtual_audio = std::env::var_os("RILL_E2E_VIRTUAL_AUDIO").is_some();
        app.audio_device_enabled =
            app.virtual_audio || std::env::var_os("RILL_E2E_AUDIO_DEVICE").is_some();
        // Native editor capture stays opt-in for tests, which supply an
        // isolated helper through DAW_PLUGIN_HOST.
        app.automatic_plugin_previews = std::env::var_os("RILL_E2E_PLUGIN_PREVIEWS").is_some();
        app.settings = Settings {
            scratch_dir,
            ..app.settings.clone()
        };
        app.setup_draft = app.settings.clone();
        app.refresh_plugins();
        let mut harness = Self {
            ctx,
            app,
            time: 0.0,
            tool_result: None,
            mcp_result: None,
            mcp_server: Default::default(),
            labels: Vec::new(),
            menu_labels: Vec::new(),
            window_labels: Vec::new(),
            inspect_paint: false,
            painted_rects: Vec::new(),
            painted_lines: Vec::new(),
            painted_paths: Vec::new(),
            painted_circles: Vec::new(),
            painted_text: Vec::new(),
            painted_images: Vec::new(),
            painted_meshes: Vec::new(),
            audio_output: None,
            clipboard_text: None,
            close_requested: false,
            focused: true,
            system_theme: egui::Theme::Dark,
            size: SIZE,
        };
        harness.settle();
        Ok(harness)
    }

    fn frame(&mut self, mut input: RawInput) {
        self.time += 1.0 / 60.0;
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, self.size));
        input.time = Some(self.time);
        input.focused = self.focused;
        input.system_theme = Some(self.system_theme);
        // Periodic recovery is wall-clock driven in the native app. Closing still
        // exercises recovery normally, within the isolated automation directory.
        self.app.last_recovery = Instant::now();
        self.menu_labels.clear();
        self.window_labels.clear();
        let app = &mut self.app;
        let menu_labels = &mut self.menu_labels;
        let window_labels = &mut self.window_labels;
        let output = self.ctx.run(input, |ctx| {
            app.ui_frame(ctx);
            // A menu can repeat a persistent toolbar destination. Resolve the
            // default click to the actual foreground menu, without changing
            // explicit occurrence indexing for repeated channel/library rows.
            {
                let layers = ctx.memory(|memory| {
                    memory
                        .layer_ids()
                        .filter(|layer| {
                            matches!(layer.order, egui::Order::Foreground | egui::Order::Middle)
                                && memory.areas().is_visible(layer)
                        })
                        .collect::<Vec<_>>()
                });
                for layer in layers {
                    let mut layer_labels = Vec::new();
                    let transform = ctx.layer_transform_to_global(layer);
                    ctx.graphics(|graphics| {
                        if let Some(list) = graphics.get(layer) {
                            for original in list.all_entries() {
                                let mut shape = original.clone();
                                if let Some(transform) = transform {
                                    shape.transform(transform);
                                }
                                collect_labels(&shape.shape, shape.clip_rect, &mut layer_labels);
                            }
                        }
                    });
                    if layer.order == egui::Order::Foreground {
                        // Retained backdrops can repeat toolbar text on another
                        // foreground layer. Only the actual pointer-owning area
                        // is an interactive menu target.
                        menu_labels.extend(
                            layer_labels
                                .into_iter()
                                .filter(|(_, rect)| ctx.layer_id_at(rect.center()) == Some(layer)),
                        );
                    } else {
                        window_labels.extend(layer_labels);
                    }
                }
            }
        });
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                self.clipboard_text = Some(text.clone());
            }
        }
        self.close_requested |= output.viewport_output.values().any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        });
        self.labels.clear();
        self.painted_rects.clear();
        self.painted_lines.clear();
        self.painted_paths.clear();
        self.painted_circles.clear();
        self.painted_text.clear();
        self.painted_images.clear();
        self.painted_meshes.clear();
        for clipped in output.shapes {
            collect_labels(&clipped.shape, clipped.clip_rect, &mut self.labels);
            if self.inspect_paint {
                collect_painted_rects(&clipped.shape, clipped.clip_rect, &mut self.painted_rects);
                collect_painted_lines(&clipped.shape, clipped.clip_rect, &mut self.painted_lines);
                collect_painted_details(
                    &clipped.shape,
                    clipped.clip_rect,
                    &mut self.painted_paths,
                    &mut self.painted_circles,
                    &mut self.painted_text,
                );
                collect_painted_images(&clipped.shape, clipped.clip_rect, &mut self.painted_images);
                collect_painted_meshes(&clipped.shape, clipped.clip_rect, &mut self.painted_meshes);
            }
        }
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(RawInput::default());
        }
    }

    fn label_rect(&self, label: &str, occurrence: usize) -> Result<Rect> {
        if occurrence == 0
            && let Some((_, rect)) = self
                .menu_labels
                .iter()
                .rev()
                .find(|(text, _)| text.trim() == label)
        {
            return Ok(*rect);
        }
        let first = self.labels.iter().find(|(text, _)| text.trim() == label);
        if occurrence == 0
            && let Some((_, rect)) = self
                .window_labels
                .iter()
                .find(|(text, _)| text.trim() == label)
            && !first.is_some_and(|first| self.window_labels.contains(first))
        {
            // Prefer the active utility over the retained, disabled workspace.
            // Repeated labels within that utility keep their original indexing.
            return Ok(*rect);
        }
        self.labels
            .iter()
            .filter(|(text, _)| text.trim() == label)
            .nth(occurrence)
            .map(|(_, rect)| *rect)
            .with_context(|| {
                format!("Visible label {label:?}, occurrence {occurrence}, was not found")
            })
    }

    fn click(&mut self, label: &str, occurrence: usize) -> Result<()> {
        // Semantic clicks reveal a command using real wheel input, just as a
        // mouse user does when the menu is taller than the application window.
        let foreground_has_label = |this: &Self| {
            this.menu_labels
                .iter()
                .any(|(text, _)| text.trim() == label)
        };
        if egui::Popup::is_any_open(&self.ctx)
            && let Some(rect) = daw_ui::command_menu_scroll_rect(&self.ctx)
            && !foreground_has_label(self)
            && self
                .label_rect(label, occurrence)
                .map_or(true, |target| target.center().y >= rect.top())
        {
            self.frame(RawInput {
                events: vec![
                    Event::PointerMoved(rect.center()),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, 10_000.0),
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            });
            self.settle();
            for _ in 0..12 {
                if foreground_has_label(self) {
                    break;
                }
                self.frame(RawInput {
                    events: vec![
                        Event::PointerMoved(rect.center()),
                        Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -180.0),
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                });
                self.settle();
                if foreground_has_label(self) {
                    break;
                }
            }
            ensure!(
                foreground_has_label(self),
                "Command {label:?} was not visible in the active scrolling menu"
            );
        }
        let pos = self.label_rect(label, occurrence)?.center();
        for pressed in [true, false] {
            self.frame(RawInput {
                events: vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            });
        }
        self.settle();
        Ok(())
    }

    fn pointer(
        &mut self,
        from: Pos2,
        to: Pos2,
        phase: PointerPhase,
        button: PointerButton,
        modifiers: egui::Modifiers,
    ) -> Result<()> {
        ensure!(
            [from.x, from.y, to.x, to.y]
                .iter()
                .all(|value| value.is_finite())
                && Rect::from_min_size(Pos2::ZERO, self.size).contains(from)
                && Rect::from_min_size(Pos2::ZERO, self.size).contains(to),
            "Pointer coordinates must be finite and within the viewport"
        );
        let send = |this: &mut Self, pos: Pos2, pressed: Option<bool>| {
            let mut events = vec![Event::PointerMoved(pos)];
            if let Some(pressed) = pressed {
                events.push(Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                });
            }
            this.frame(RawInput {
                events,
                modifiers,
                ..Default::default()
            });
        };
        match phase {
            PointerPhase::Down => send(self, from, Some(true)),
            PointerPhase::Move => send(self, from, None),
            PointerPhase::Up => send(self, from, Some(false)),
            PointerPhase::Click | PointerPhase::DoubleClick | PointerPhase::Drag => {
                let count = if matches!(phase, PointerPhase::DoubleClick) {
                    2
                } else {
                    1
                };
                for _ in 0..count {
                    send(self, from, None);
                    send(self, from, Some(true));
                    if matches!(phase, PointerPhase::Drag) {
                        for step in 1..=8 {
                            send(self, from.lerp(to, step as f32 / 8.0), None);
                        }
                    }
                    send(self, to, Some(false));
                }
                self.settle();
            }
        }
        Ok(())
    }

    fn drop_files(&mut self, paths: Vec<PathBuf>, track: Option<usize>, beat: f64) -> Result<()> {
        ensure!(!paths.is_empty(), "Provide at least one audio file");
        ensure!(
            beat.is_finite() && beat >= 0.0,
            "Drop beat must be nonnegative and finite"
        );
        let (target, y) = if let Some(index) = track {
            let track = self
                .app
                .project
                .tracks
                .get(index)
                .context("Track index is out of bounds")?;
            ensure!(
                track.kind == daw_core::TrackKind::Audio,
                "Target must be an audio track"
            );
            let occurrence = crate::routing::visible_tracks(&self.app.project)
                .iter()
                .take_while(|(candidate, _)| *candidate != index)
                .filter(|(candidate, _)| self.app.project.tracks[*candidate].name == track.name)
                .count();
            (
                Some(track.id),
                self.label_rect(&track.name, occurrence)?.center().y,
            )
        } else {
            (
                None,
                self.label_rect("Drop audio here · Double-click for MIDI", 0)?
                    .center()
                    .y,
            )
        };
        // Locate the requested beat using the arrangement's own hit testing. This
        // avoids duplicating its header, scroll, zoom, and snap geometry here.
        let (mut low, mut high) = (0.0, self.size.x - 1.0);
        let mut position = None;
        for _ in 0..24 {
            let pos = Pos2::new((low + high) * 0.5, y);
            self.frame(RawInput {
                events: vec![Event::PointerMoved(pos)],
                ..Default::default()
            });
            match self.app.drop_target {
                Some((found, found_beat)) if found == target => {
                    if (found_beat - beat).abs() < 1e-6 {
                        position = Some(pos);
                        break;
                    }
                    if found_beat < beat {
                        low = pos.x;
                    } else {
                        high = pos.x;
                    }
                }
                _ => low = pos.x,
            }
        }
        let pos = position.context(
            "Requested drop position is not visible or does not align to the current snap",
        )?;
        self.frame(RawInput {
            events: vec![Event::PointerMoved(pos)],
            hovered_files: paths
                .iter()
                .map(|path| egui::HoveredFile {
                    path: Some(path.clone()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        self.frame(RawInput {
            events: vec![Event::PointerMoved(pos)],
            dropped_files: paths
                .into_iter()
                .map(|path| egui::DroppedFile {
                    path: Some(path),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        self.settle();
        ensure!(!self.app.error, "{}", self.app.status);
        Ok(())
    }

    fn execute(&mut self, command: Command) -> Result<bool> {
        self.audio_output = None;
        self.inspect_paint = false;
        self.painted_rects.clear();
        self.painted_lines.clear();
        self.painted_paths.clear();
        self.painted_circles.clear();
        self.painted_text.clear();
        self.painted_images.clear();
        match command {
            Command::SongTool { name, arguments } => {
                let result = self.execute_tool_wait(&name, arguments);
                self.tool_result = Some(match result {
                    Ok(result) => json!({"ok":true,"result":result}),
                    Err(error) => json!({"ok":false,"error":error}),
                });
                self.settle();
            }
            Command::ApplicationTool { name, arguments } => {
                let mut result = self.execute_tool_wait_profile(
                    daw_tools::ToolProfile::Application,
                    &name,
                    arguments,
                );
                if result.as_ref().is_ok_and(|value| value["accepted"] == true) {
                    let deadline = Instant::now() + Duration::from_secs(120);
                    while self.app.project_io_pending()
                        || self.app.state_sync_pending()
                        || self.app.document_guard.is_some()
                        || self.app.export_job.is_some()
                        || self.app.loading.is_some()
                    {
                        if Instant::now() >= deadline {
                            result = Err(daw_tools::ToolError::new(
                                "timeout",
                                "Application operation timed out",
                            ));
                            break;
                        }
                        self.frame(RawInput::default());
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    if self.app.error {
                        result = Err(daw_tools::ToolError::new(
                            "operation_failed",
                            self.app.status.clone(),
                        ));
                    }
                }
                self.tool_result = Some(match result {
                    Ok(value) => json!({"ok":true,"result":value}),
                    Err(error) => json!({"ok":false,"error":error}),
                });
                self.settle();
            }
            Command::McpRequest {
                request,
                application,
            } => {
                let profile = if application {
                    daw_tools::ToolProfile::Application
                } else {
                    daw_tools::ToolProfile::CurrentSong
                };
                struct DriverHost<'a>(&'a mut Harness, daw_tools::ToolProfile);
                impl daw_tools::ToolHost for DriverHost<'_> {
                    fn tool_specs(&self) -> Vec<daw_tools::ToolSpec> {
                        daw_tools::tool_specs_for(self.1)
                    }
                    fn call_tool(
                        &mut self,
                        name: &str,
                        args: Value,
                    ) -> std::result::Result<Value, daw_tools::ToolError> {
                        self.0.execute_tool_wait_profile(self.1, name, args)
                    }
                }
                let mut server = std::mem::take(&mut self.mcp_server);
                self.mcp_result = server.handle_request(&mut DriverHost(self, profile), request);
                self.mcp_server = server;
                self.settle();
            }
            Command::AssistantScript { turns } => {
                self.app.assistant.set_script(turns);
                self.settle();
            }
            Command::WaitAssistant { timeout_ms } => {
                ensure!(
                    (1..=120000).contains(&timeout_ms),
                    "Assistant timeout must be between 1 and 120000 ms"
                );
                let deadline = Instant::now() + Duration::from_millis(timeout_ms);
                while self.app.assistant.busy() {
                    ensure!(Instant::now() < deadline, "Assistant timed out");
                    self.frame(RawInput::default());
                    std::thread::sleep(Duration::from_millis(1));
                }
                self.settle();
            }
            Command::PumpAudio { frames } => {
                ensure!(self.app.virtual_audio, "Virtual audio is not enabled");
                ensure!(
                    (1..=8192).contains(&frames),
                    "Audio pump size must be 1..=8192"
                );
                self.audio_output = Some(
                    self.app
                        .engine
                        .as_mut()
                        .context("Audio engine is not ready")?
                        .pump_virtual(frames)?,
                );
                self.settle();
            }
            Command::PumpInput { channels, samples } => {
                ensure!(
                    self.app.virtual_audio,
                    "Virtual input requires virtual audio mode"
                );
                ensure!(
                    samples.len() <= 1_048_576,
                    "Input injection exceeds bounded request"
                );
                self.audio_output = Some(
                    self.app
                        .engine
                        .as_mut()
                        .context("Audio engine is not ready")?
                        .pump_virtual_input(&samples, channels)?,
                );
                self.settle();
            }
            Command::DummyAudioDevice {
                input_channels,
                input_latency_frames,
                output_latency_frames,
            } => {
                ensure!(
                    self.app.virtual_audio,
                    "Dummy device configuration requires virtual audio mode"
                );
                ensure!(
                    !self.app.recording_busy(),
                    "Finish recording before changing dummy device parameters"
                );
                let mut config = self.app.virtual_io;
                if let Some(value) = input_channels {
                    config.input_channels = value;
                }
                if let Some(value) = input_latency_frames {
                    config.input_latency_frames = value;
                }
                if let Some(value) = output_latency_frames {
                    config.output_latency_frames = value;
                }
                ensure!(
                    (1..=32).contains(&config.input_channels),
                    "Dummy input requires 1..32 channels"
                );
                ensure!(
                    config.input_latency_frames <= self.app.settings.sample_rate
                        && config.output_latency_frames <= self.app.settings.sample_rate,
                    "Dummy device latency may not exceed one second"
                );
                if let Some(engine) = self.app.engine.as_mut() {
                    engine.configure_virtual_io(config)?;
                }
                self.app.virtual_io = config;
                self.app.refresh_devices();
                self.settle();
            }
            Command::FeedInput { channels, samples } => {
                ensure!(
                    self.app.virtual_audio && samples.len() <= 1_048_576,
                    "Input injection requires bounded virtual audio mode"
                );
                self.app
                    .engine
                    .as_mut()
                    .context("Audio engine is not ready")?
                    .feed_virtual_input(&samples, channels)?;
                self.settle();
            }
            Command::MidiInput {
                bytes,
                offset_frames,
                delay_frames,
            } => {
                ensure!(
                    self.app.virtual_audio,
                    "MIDI injection requires virtual audio mode"
                );
                ensure!(
                    offset_frames == 0 || delay_frames == 0,
                    "Choose either a MIDI offset or a callback delivery delay"
                );
                let engine = self
                    .app
                    .engine
                    .as_mut()
                    .context("Audio engine is not ready")?;
                if delay_frames == 0 {
                    engine.inject_virtual_midi(&bytes, offset_frames)?;
                } else {
                    engine.inject_virtual_midi_delayed(&bytes, delay_frames)?;
                }
                self.settle();
            }
            Command::MidiError => {
                ensure!(
                    self.app.virtual_audio,
                    "MIDI fault injection requires virtual audio mode"
                );
                self.app
                    .engine
                    .as_mut()
                    .context("Audio engine is not ready")?
                    .fail_virtual_midi()?;
                self.settle();
            }
            Command::OutputError => {
                ensure!(
                    self.app.virtual_audio,
                    "Output fault injection requires virtual audio mode"
                );
                self.app
                    .engine
                    .as_mut()
                    .context("Audio engine is not ready")?
                    .fail_virtual_output()?;
                self.settle();
            }
            Command::InputError => {
                ensure!(
                    self.app.virtual_audio,
                    "Input fault injection requires virtual audio mode"
                );
                self.app
                    .engine
                    .as_mut()
                    .context("Audio engine is not ready")?
                    .fail_virtual_input()?;
                self.settle();
            }
            Command::FileDialog {
                kind,
                path,
                replace_existing,
            } => {
                use crate::document_ui::{DocumentDialog, DocumentDialogSelection};
                self.app.next_document_dialog = Some(DocumentDialogSelection {
                    kind: match kind {
                        TestDocumentDialog::Save => DocumentDialog::Save,
                        TestDocumentDialog::Open => DocumentDialog::Open,
                        TestDocumentDialog::Export => DocumentDialog::Export,
                        TestDocumentDialog::ExportMidi => DocumentDialog::ExportMidi,
                    },
                    path,
                    replace_existing,
                });
            }
            Command::AudioTakeFiles { paths } => {
                self.app.audio_takes_ui.next_files = Some(paths);
            }
            Command::CloseWindow => {
                let mut input = RawInput::default();
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .context("Missing root viewport")?
                    .events
                    .push(egui::ViewportEvent::Close);
                self.frame(input);
                self.settle();
            }
            Command::SystemTheme { dark } => {
                self.system_theme = if dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                };
                self.settle();
            }
            Command::Viewport {
                width,
                height,
                pixels_per_point,
            } => {
                ensure!(
                    width.is_finite()
                        && height.is_finite()
                        && (800.0..=4096.0).contains(&width)
                        && (600.0..=4096.0).contains(&height)
                        && (0.75..=3.0).contains(&pixels_per_point),
                    "Invalid test viewport"
                );
                self.size = Vec2::new(width, height);
                self.ctx.set_pixels_per_point(pixels_per_point);
                self.settle();
            }
            Command::Inspect { paint } => {
                self.inspect_paint = paint;
                self.settle();
            }
            Command::Focused { focused } => {
                self.focused = focused;
                self.settle();
            }
            Command::Click { label, occurrence } => self.click(&label, occurrence)?,
            Command::Pointer {
                x,
                y,
                phase,
                to_x,
                to_y,
                button,
                shift,
                ctrl,
                alt,
                cmd,
            } => {
                let from = Pos2::new(x, y);
                let drag = matches!(phase, PointerPhase::Drag);
                let to = if drag {
                    Pos2::new(
                        to_x.context("Drag requires to_x")?,
                        to_y.context("Drag requires to_y")?,
                    )
                } else {
                    from
                };
                self.pointer(
                    from,
                    to,
                    phase,
                    button.into(),
                    input_modifiers(shift, ctrl, alt, cmd),
                )?;
            }
            Command::Scroll {
                x,
                y,
                delta,
                delta_x,
                shift,
                ctrl,
                alt,
                cmd,
            } => {
                let modifiers = input_modifiers(shift, ctrl, alt, cmd);
                let pos = Pos2::new(x, y);
                ensure!(
                    [x, y, delta, delta_x].iter().all(|value| value.is_finite())
                        && Rect::from_min_size(Pos2::ZERO, self.size).contains(pos),
                    "Scroll coordinates must be finite and within the viewport"
                );
                self.frame(RawInput {
                    events: vec![
                        Event::PointerMoved(pos),
                        Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: Vec2::new(delta_x, delta),
                            modifiers,
                        },
                    ],
                    modifiers,
                    ..Default::default()
                });
                self.settle();
            }
            Command::Key {
                key,
                shift,
                ctrl,
                alt,
                cmd,
                release_modifiers,
                repetitions,
            } => {
                ensure!(
                    (1..=32).contains(&repetitions),
                    "Key repetitions must be between 1 and 32"
                );
                let key = egui::Key::from_name(&key).context("Unknown keyboard key")?;
                let modifiers = egui::Modifiers {
                    alt,
                    ctrl,
                    shift,
                    mac_cmd: cmd && cfg!(target_os = "macos"),
                    command: cmd || ctrl,
                };
                let event = |pressed| Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                };
                if release_modifiers || repetitions > 1 {
                    self.frame(RawInput {
                        modifiers: if release_modifiers {
                            egui::Modifiers::NONE
                        } else {
                            modifiers
                        },
                        events: (0..repetitions)
                            .flat_map(|_| [event(true), event(false)])
                            .collect(),
                        ..Default::default()
                    });
                } else {
                    for pressed in [true, false] {
                        self.frame(RawInput {
                            modifiers,
                            events: vec![event(pressed)],
                            ..Default::default()
                        });
                    }
                }
                self.settle();
            }
            Command::Text { text } => {
                self.frame(RawInput {
                    events: vec![Event::Text(text)],
                    ..Default::default()
                });
                self.settle();
            }
            Command::Clipboard {
                action,
                text,
                with_key,
            } => {
                let (event, key) = match action {
                    TestClipboardAction::Copy => (Event::Copy, egui::Key::C),
                    TestClipboardAction::Cut => (Event::Cut, egui::Key::X),
                    TestClipboardAction::Paste => (
                        Event::Paste(
                            text.or_else(|| self.clipboard_text.clone())
                                .unwrap_or_default(),
                        ),
                        egui::Key::V,
                    ),
                };
                let mut events = vec![event];
                if with_key {
                    for pressed in [true, false] {
                        events.push(Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        });
                    }
                }
                self.frame(RawInput {
                    events,
                    ..Default::default()
                });
                self.settle();
            }
            Command::Drop { paths, track, beat } => self.drop_files(paths, track, beat)?,
            Command::Open { path, wait } => {
                self.app.request_open_project(path, false);
                if wait {
                    self.wait_project(project_timeout())?;
                    self.settle();
                }
            }
            Command::Save { path, wait } => {
                self.app.save_to(path);
                if wait {
                    self.wait_project(project_timeout())?;
                }
            }
            Command::WaitProject { timeout_ms } => self.wait_project(timeout_ms)?,
            Command::Export { path } => {
                ensure!(
                    self.app.export_job.is_none(),
                    "An export is already running"
                );
                self.app.export_to(path);
                self.settle();
                ensure!(!self.app.error, "{}", self.app.status);
            }
            Command::WaitExport { timeout_ms } => {
                ensure!(
                    (1..=60_000).contains(&timeout_ms),
                    "Export timeout must be between 1 and 60000 ms"
                );
                let deadline = Instant::now() + Duration::from_millis(timeout_ms);
                while self.app.export_job.is_some()
                    || self.app.state_sync_pending()
                    || self.app.project_io_pending()
                {
                    ensure!(Instant::now() < deadline, "Export timed out");
                    self.frame(RawInput::default());
                    std::thread::sleep(Duration::from_millis(1));
                }
                ensure!(!self.app.error, "{}", self.app.status);
            }
            Command::Quit => {
                ensure!(
                    self.app.export_job.is_none(),
                    "Wait for the running export before quitting"
                );
                self.close()?;
                return Ok(true);
            }
        }
        if self.close_requested {
            self.close()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn wait_project(&mut self, timeout_ms: u64) -> Result<()> {
        ensure!(
            (1..=120_000).contains(&timeout_ms),
            "Project timeout must be between 1 and 120000 ms"
        );
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while self.app.recording_busy()
            || self.app.state_sync_pending()
            || self.app.project_io_pending()
            || self.app.document_guard_diagnostics()["phase"]
                .as_str()
                .is_some_and(|phase| !matches!(phase, "confirm" | "export_failed"))
        {
            ensure!(Instant::now() < deadline, "Project operation timed out");
            self.frame(RawInput::default());
            std::thread::sleep(Duration::from_millis(1));
        }
        ensure!(!self.app.error, "{}", self.app.status);
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        // Explicit protocol shutdown has already elected to leave the test
        // document. `close_window` separately exercises the user's close event
        // and unsaved-change choice without silently accepting the prompt.
        if !self.app.close_ready {
            self.app.request_quit(&self.ctx);
        }
        self.wait_project(project_timeout())?;
        ensure!(self.app.close_ready, "Project shutdown did not complete");
        Ok(())
    }

    fn execute_tool_wait(
        &mut self,
        name: &str,
        args: Value,
    ) -> std::result::Result<Value, daw_tools::ToolError> {
        self.execute_tool_wait_profile(daw_tools::ToolProfile::CurrentSong, name, args)
    }

    fn execute_tool_wait_profile(
        &mut self,
        profile: daw_tools::ToolProfile,
        name: &str,
        args: Value,
    ) -> std::result::Result<Value, daw_tools::ToolError> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.app
            .dispatch_profile_tool(profile, name.into(), args, sender, cancel.clone());
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            match receiver.try_recv() {
                Ok(result) => return result,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err(daw_tools::ToolError::new(
                        "unavailable",
                        "Tool dispatcher closed",
                    ));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            if Instant::now() >= deadline {
                cancel.store(true, std::sync::atomic::Ordering::Release);
                return Err(daw_tools::ToolError::new("timeout", "Tool timed out"));
            }
            self.frame(RawInput::default());
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn state(&self) -> Value {
        let mut state = json!({
            "effective_theme": format!("{:?}", self.ctx.theme()),
            "theme_colors": {
                "background": daw_ui::theme::palette(&self.ctx).bg.to_array(),
                "text": daw_ui::theme::palette(&self.ctx).text.to_array(),
                "panel": self.ctx.style().visuals.panel_fill.to_array(),
            },
            "project": self.app.project,
            "project_pending": self.app.project_io_pending() || self.app.state_sync_pending(),
            "modified": self.app.modified,
            "history": { "undo": self.app.history.undo_labels().collect::<Vec<_>>(),
                "redo": self.app.history.redo_labels().collect::<Vec<_>>(),
                "bytes": self.app.history.memory_bytes(), "saved": self.app.history.is_saved() },
            "project_path": self.app.project_path,
            "waveform_count": self.app.waveforms.len(),
            "waveform_warning": self.app.waveform_warning,
            "native_editor": self.app.instrument_diagnostics(),
            "audition_request": self.app.audition_request,
            "join_pending": self.app.join_ui.pending(),
            "plugin_catalog": self.app.vst_catalog.iter().map(|entry| json!({
                "name": entry.plugin.name, "class_id": entry.plugin.class_id,
                "thumbnail": entry.thumbnail, "status": format!("{:?}", entry.status),
                "added_unix_ms": entry.added_unix_ms, "version": entry.plugin.version,
                "architectures": entry.plugin.architectures,
                "favorite": self.app.plugin_favorite(&entry.plugin),
                "categories": self.app.plugin_categories(&entry.plugin),
            })).collect::<Vec<_>>(),
            "plugin_scanning": self.app.plugin_sync.scanning(),
            "instrument_parameter_rect": self.ctx.data(|data| data.get_temp::<Rect>(egui::Id::new("instrument_parameter_rect"))).map(|rect| json!({"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]})),
            "selected_track": self.app.selected_track,
            "selected_clip": self.app.selected_clip,
            "view": format!("{:?}", self.app.view),
            "popups": {
                "library": self.app.library_open, "setup": self.app.setup_open,
                "properties": self.app.inspector_open, "instrument": self.app.instrument_open,
                "effects": self.app.effects_open, "routing": self.app.routing_open,
                "automation": self.app.automation_open, "history": self.app.history_open,
                "crossfade": self.app.crossfade_open, "join": self.app.join_open,
            },
            "automation_plot_rect": self.ctx.data(|data| data.get_temp::<Rect>(egui::Id::new("automation_plot_rect"))).map(|rect| json!({"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]})),
            "settings": self.app.settings,
            "setup_draft": self.app.setup_draft,
            "status": self.app.status,
            "error": self.app.error,
            "export_pending": self.app.export_job.is_some() || self.app.state_sync_pending() || self.app.project_io_pending(),
            "device_open": self.app.engine.is_some(),
            "scratch_dir": self.app.settings.scratch_dir,
            "drop_target": self.app.drop_target,
            "pointer_position": self.ctx.input(|input| input.pointer.hover_pos().map(|point| [point.x, point.y])),
            "viewport": [self.size.x, self.size.y],
            "text_sizes": { "body": daw_ui::tokens::FONT_BODY, "note": daw_ui::tokens::FONT_SMALL, "canvas": daw_ui::tokens::FONT_CANVAS },
            "visible_labels": self.labels.iter().map(|(label, _)| label.trim()).collect::<Vec<_>>(),
            "label_rects": self.labels.iter().map(|(label, rect)| json!({
                "text": label.trim(), "min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y],
            })).collect::<Vec<_>>(),
        });
        for (field, diagnostic) in [
            ("midi_editor", "midi_editor_diagnostics"),
            ("audio_editor", "audio_editor_diagnostics"),
            ("timeline", "timeline_diagnostics"),
            ("mixer", "mixer_diagnostics"),
            ("crossfade", "crossfade_diagnostics"),
        ] {
            state[field] = self
                .ctx
                .data(|data| data.get_temp::<Value>(egui::Id::new(diagnostic)))
                .unwrap_or(Value::Null);
        }
        state["last_error_report"] = self.app.last_error_report.as_ref().map_or(
            Value::Null,
            |report| {
                json!({
                    "summary": report.summary, "details": report.details, "status": report.status,
                })
            },
        );
        state["tool_join_pending"] = json!(self.app.tool_joins.pending());
        state["song_session"] = json!(self.app.song_session);
        state["assistant"] = self.app.assistant_diagnostics(&self.ctx);
        state["tool_result"] = json!(self.tool_result);
        state["mcp_result"] = json!(self.mcp_result);
        state["revision"] = json!(self.app.revision);
        state["audio_silence"] = crate::audio_silence::diagnostics(&self.ctx);
        state["popups"]["about"] = json!(self.app.about_open);
        state["audio_transpose_rect"] = json!(
            self.ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("audio_transpose_rect")))
                .map(|rect| [rect.left(), rect.top(), rect.right(), rect.bottom()])
        );
        state["playback_loading"] = json!(self.app.loading.is_some());
        state["engine"] = self.app.engine.as_ref().map_or(Value::Null, |engine| {
            let s = engine.snapshot();
            json!({
                "engine_id": s.engine_id, "playing": s.playing,
                "position_beats": s.position_beats,
                "peak_left": s.peak_left, "peak_right": s.peak_right,
                "underruns": s.underruns, "render_errors": s.render_errors,
                "requested_revision": s.requested_revision,
                "applied_revision": s.applied_revision, "edit_errors": s.edit_errors,
                "rendered_frames": s.rendered_frames, "consumed_frames": s.consumed_frames,
                "generation": s.generation, "refining_audio": s.refining_audio,
            })
        });
        state["audio_output"] = json!(self.audio_output);
        state["window_rects"] = self
            .ctx
            .memory(|memory| {
                memory
                    .layer_ids()
                    .filter(|layer| {
                        layer.order == egui::Order::Middle && memory.areas().is_visible(layer)
                    })
                    .filter_map(|layer| memory.area_rect(layer.id))
                    .map(|rect| {
                        json!({"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]})
                    })
                    .collect::<Vec<_>>()
            })
            .into();
        state["exported_file"] = json!(self.app.exported_file);
        state["export_run"] = self.app.export_diagnostics();
        state["selected_clips"] = json!(self.app.arranger_selection_ids());
        state["selection_span"] = json!(self.app.arranger_phrase_span());
        state["boundary_fades_dialog"] = crate::audio_batch::diagnostics(&self.ctx);
        state["audio_batch_fades"] = state["boundary_fades_dialog"].clone();
        state["finishing"] = self.app.finishing_diagnostics();
        state["menu_label_rects"] =
            json!(self.menu_labels.iter().map(|(label,rect)| json!({
            "text":label,"min":[rect.left(),rect.top()],"max":[rect.right(),rect.bottom()]
        })).collect::<Vec<_>>());
        state["native_menu"] = json!({
            "open": egui::Popup::is_any_open(&self.ctx),
            "scroll_rect": daw_ui::command_menu_scroll_rect(&self.ctx).map(|rect| [rect.left(),rect.top(),rect.right(),rect.bottom()]),
            "foreground_areas": self.ctx.memory(|memory| memory.layer_ids()
                .filter(|layer| layer.order == egui::Order::Foreground && memory.areas().is_visible(layer))
                .filter_map(|layer| memory.area_rect(layer.id).map(|rect| [rect.left(),rect.top(),rect.right(),rect.bottom()]))
                .collect::<Vec<_>>()),
        });
        if self.inspect_paint {
            state["painted_rects"] = json!(self.painted_rects);
            state["painted_lines"] = json!(self.painted_lines);
            state["painted_paths"] = json!(self.painted_paths);
            state["painted_circles"] = json!(self.painted_circles);
            state["painted_text"] = json!(self.painted_text);
            state["painted_images"] = json!(self.painted_images);
            state["painted_meshes"] = json!(self.painted_meshes);
        }
        state["inspector_page"] = json!(format!("{:?}", self.app.inspector_page));
        state["popup_context"] = self.app.popup_context_diagnostics();
        state["backdrop_blur"] = json!(daw_ui::backdrop_blur_info(&self.ctx).iter().map(|blur| json!({
            "rect": {"min": [blur.bounds.min.x, blur.bounds.min.y], "max": [blur.bounds.max.x, blur.bounds.max.y]},
            "target_size": blur.target_size,
            "cached_source": blur.cached_source,
            "gpu_available": blur.gpu_available,
        })).collect::<Vec<_>>());
        state["popups"]["export"] = json!(self.app.export_open);
        state["export_include_effect_tails"] = json!(self.app.export_include_effect_tails);
        state["clipboard_text"] = json!(self.clipboard_text);
        state["recording"] = self.app.recording_diagnostics(&self.ctx);
        state["popups"]["recording"] = json!(self.app.recording_open);
        state["input_device_name"] = json!(self.app.settings.input_device_name);
        state["audio_takes"] = self.app.audio_takes_diagnostics();
        state["spectrogram"] = crate::spectrogram::diagnostics(&self.ctx);
        state["waveform"] = crate::spectrogram::waveform_diagnostics(&self.ctx);
        state["popups"]["audio_takes"] = json!(self.app.audio_takes_open);
        state["error_details"] = self.app.error_details_diagnostics();
        state["document_guard"] = self.app.document_guard_diagnostics();
        state["file_dialog_pending"] = json!(self.app.next_document_dialog.is_some());
        state["project_title"] = json!(self.app.document_title());
        state["file_dialog_defaults"] = json!({
            "directory": self.app.document_dialog_directory(),
            "save": self.app.document_dialog_filename(crate::document_ui::DocumentDialog::Save),
            "wav": self.app.document_dialog_filename(crate::document_ui::DocumentDialog::Export),
            "midi": self.app.document_dialog_filename(crate::document_ui::DocumentDialog::ExportMidi),
        });
        state["export_missing_instruments"] = json!(self.app.export_missing_instruments());
        state["clip_name_rect"] = json!(
            self.ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::new("clip_name_rect")))
                .map(|rect| [rect.left(), rect.top(), rect.right(), rect.bottom()])
        );
        state["library"] = if self.app.library_open {
            self.ctx
                .data(|data| data.get_temp::<Value>(egui::Id::new("library_diagnostics")))
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        state["automation_editor"] = if self.app.automation_open {
            self.ctx
                .data(|data| data.get_temp::<Value>(egui::Id::new("automation_editor_diagnostics")))
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        state
    }
}

/// Raw line geometry is collected only for explicit paint inspection. This lets
/// running-app tests compare scale marks with the rendered handle, independently
/// of the widget's layout diagnostics.
fn collect_painted_lines(shape: &egui::Shape, clip: Rect, output: &mut Vec<Value>) {
    match shape {
        egui::Shape::LineSegment { points, stroke }
            if stroke.width > 0.0
                && stroke.color.a() > 0
                && Rect::from_two_pos(points[0], points[1])
                    .expand(stroke.width * 0.5)
                    .intersects(clip) =>
        {
            output.push(json!({
                "points": [[points[0].x, points[0].y], [points[1].x, points[1].y]],
                "stroke": stroke.color.to_array(), "stroke_width": stroke.width,
                "clip": {"min":[clip.min.x,clip.min.y], "max":[clip.max.x,clip.max.y]},
            }));
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_painted_lines(shape, clip, output);
            }
        }
        _ => {}
    }
}

/// Observe the actual canvas and text paint only for explicit inspections.
/// Shapes retain their geometry so tests can distinguish a control or curve
/// from an unrelated use of the same theme color elsewhere on screen.
fn collect_painted_details(
    shape: &egui::Shape,
    clip: Rect,
    paths: &mut Vec<Value>,
    circles: &mut Vec<Value>,
    text: &mut Vec<Value>,
) {
    let rect_json =
        |rect: Rect| json!({"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]});
    match shape {
        egui::Shape::Path(path) if path.visual_bounding_rect().intersects(clip) => {
            let stroke = match &path.stroke.color {
                egui::epaint::ColorMode::Solid(color) => Some(color.to_array()),
                egui::epaint::ColorMode::UV(_) => None,
            };
            paths.push(json!({
                "points":path.points.iter().map(|point| [point.x,point.y]).collect::<Vec<_>>(),
                "closed":path.closed,"fill":path.fill.to_array(),"stroke":stroke,
                "stroke_width":path.stroke.width,"clip":rect_json(clip),
            }));
        }
        egui::Shape::Circle(circle) if circle.visual_bounding_rect().intersects(clip) => {
            circles.push(json!({
                "center":[circle.center.x,circle.center.y],"radius":circle.radius,
                "fill":circle.fill.to_array(),"stroke":circle.stroke.color.to_array(),
                "stroke_width":circle.stroke.width,"clip":rect_json(clip),
            }));
        }
        egui::Shape::Text(shape) => {
            let rect = shape
                .galley
                .rect
                .translate(shape.pos.to_vec2())
                .intersect(clip);
            if rect.is_positive() {
                let mut colors = Vec::new();
                for section in &shape.galley.job.sections {
                    let color = shape
                        .override_text_color
                        .unwrap_or_else(|| {
                            if section.format.color == egui::Color32::PLACEHOLDER {
                                shape.fallback_color
                            } else {
                                section.format.color
                            }
                        })
                        .gamma_multiply(shape.opacity_factor)
                        .to_array();
                    if !colors.contains(&color) {
                        colors.push(color);
                    }
                }
                text.push(
                    json!({"text":shape.galley.job.text,"rect":rect_json(rect),"colors":colors}),
                );
            }
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_painted_details(shape, clip, paths, circles, text);
            }
        }
        _ => {}
    }
}

fn collect_painted_rects(shape: &egui::Shape, clip: Rect, output: &mut Vec<Value>) {
    match shape {
        egui::Shape::Rect(shape) if shape.rect.intersects(clip) => {
            let rect = shape.rect.intersect(clip);
            output.push(json!({
                "rect": {"min":[rect.min.x,rect.min.y], "max":[rect.max.x,rect.max.y]},
                "fill":shape.fill.to_array(), "stroke":shape.stroke.color.to_array(),
                "stroke_width":shape.stroke.width,
            }));
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_painted_rects(shape, clip, output);
            }
        }
        _ => {}
    }
}

fn collect_painted_images(shape: &egui::Shape, clip: Rect, output: &mut Vec<Value>) {
    match shape {
        egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
            let rect = mesh.calc_bounds().intersect(clip);
            if rect.is_positive() {
                output.push(json!({
                    "rect": {"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]},
                    "texture_id": format!("{:?}", mesh.texture_id),
                    "vertex_colors": mesh.vertices.iter().map(|vertex| vertex.color.to_array()).collect::<Vec<_>>(),
                }));
            }
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_painted_images(shape, clip, output);
            }
        }
        _ => {}
    }
}

/// Inspect raw untextured geometry only when requested. This observes the mesh
/// submitted by the renderer rather than trusting a visualization-stage flag.
fn collect_painted_meshes(shape: &egui::Shape, clip: Rect, output: &mut Vec<Value>) {
    match shape {
        egui::Shape::Mesh(mesh) if mesh.texture_id == egui::TextureId::default() => {
            let bounds = mesh.calc_bounds();
            let rect = bounds.intersect(clip);
            if rect.is_positive() {
                output.push(json!({
                    "rect": {"min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y]},
                    "bounds": [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()],
                    "vertices": mesh.vertices.len(), "indices": mesh.indices.len(),
                    "transparent_vertices": mesh.vertices.iter().filter(|vertex| vertex.color.a() == 0).count(),
                    "max_alpha": mesh.vertices.iter().map(|vertex| vertex.color.a()).max().unwrap_or(0),
                    "valid": mesh.is_valid(),
                    // Small UI meshes expose actual submitted geometry for
                    // gradient/edge regressions; keep opt-in diagnostics bounded
                    // for large imported or tessellated content.
                    "vertex_data": mesh.vertices.iter().take(512).map(|vertex| json!({
                        "position": [vertex.pos.x, vertex.pos.y],
                        "color": vertex.color.to_array(),
                    })).collect::<Vec<_>>(),
                }));
            }
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_painted_meshes(shape, clip, output);
            }
        }
        _ => {}
    }
}

fn collect_labels(shape: &egui::Shape, clip: Rect, labels: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Text(text) => {
            let rect = text
                .galley
                .rect
                .translate(text.pos.to_vec2())
                .intersect(clip);
            if rect.is_positive() {
                labels.push((text.galley.job.text.clone(), rect));
            }
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_labels(shape, clip, labels);
            }
        }
        _ => {}
    }
}

pub fn run() -> Result<()> {
    let mut harness = Harness::new()?;
    let mut output = std::io::stdout().lock();
    write_response(
        &mut output,
        &json!({"ok": true, "event": "ready", "state": harness.state()}),
    )?;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let result = serde_json::from_str::<Command>(&line)
            .map_err(anyhow::Error::from)
            .and_then(|command| harness.execute(command));
        let quit = matches!(result, Ok(true));
        let response = match result {
            Ok(_) => {
                json!({"ok": true, "event": if quit { "exiting" } else { "result" }, "state": harness.state()})
            }
            Err(error) => {
                json!({"ok": false, "error": format!("{error:#}"), "state": harness.state()})
            }
        };
        write_response(&mut output, &response)?;
        if quit {
            return Ok(());
        }
    }
    if harness.app.export_job.is_some() || harness.app.project_io_pending() {
        bail!("Input closed while a project operation was running");
    }
    Ok(())
}

fn write_response(output: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod label_geometry_tests {
    use super::*;

    #[test]
    fn right_aligned_labels_keep_their_full_visible_hit_rectangle() {
        let ctx = egui::Context::default();
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let output = ctx.run(
            RawInput {
                screen_rect: Some(bounds),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new("Right aligned project title").truncate());
                    });
                });
            },
        );
        let mut labels = Vec::new();
        for shape in output.shapes {
            collect_labels(&shape.shape, shape.clip_rect, &mut labels);
        }
        let (_, rect) = labels
            .iter()
            .find(|(text, _)| text == "Right aligned project title")
            .unwrap();
        assert!(
            rect.width() > 100.0,
            "Full text must be clickable: {rect:?}"
        );
        assert!(bounds.contains_rect(*rect));
    }
}
