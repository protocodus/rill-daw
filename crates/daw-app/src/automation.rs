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
    Inspect {
        #[serde(default)]
        paint: bool,
    },
    /// Supply only the next native file picker's result; application menus still
    /// execute the complete document guard and background I/O path.
    FileDialog {
        kind: TestDocumentDialog,
        path: Option<PathBuf>,
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
    ctx: egui::Context,
    app: DawApp,
    time: f64,
    labels: Vec<(String, Rect)>,
    menu_labels: Vec<(String, Rect)>,
    inspect_paint: bool,
    painted_rects: Vec<Value>,
    painted_images: Vec<Value>,
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
        app.audio_device_enabled = std::env::var_os("RILL_E2E_AUDIO_DEVICE").is_some();
        app.automatic_plugin_previews = false;
        app.settings = Settings {
            scratch_dir,
            ..app.settings.clone()
        };
        app.setup_draft = app.settings.clone();
        let mut harness = Self {
            ctx,
            app,
            time: 0.0,
            labels: Vec::new(),
            menu_labels: Vec::new(),
            inspect_paint: false,
            painted_rects: Vec::new(),
            painted_images: Vec::new(),
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
        let app = &mut self.app;
        let menu_labels = &mut self.menu_labels;
        let output = self.ctx.run(input, |ctx| {
            app.ui_frame(ctx);
            // A menu can repeat a persistent toolbar destination. Resolve the
            // default click to the actual foreground menu, without changing
            // explicit occurrence indexing for repeated channel/library rows.
            if egui::Popup::is_any_open(ctx) {
                let layers = ctx.memory(|memory| {
                    memory
                        .layer_ids()
                        .filter(|layer| {
                            layer.order == egui::Order::Foreground
                                && memory.areas().is_visible(layer)
                        })
                        .collect::<Vec<_>>()
                });
                for layer in layers {
                    let transform = ctx.layer_transform_to_global(layer);
                    ctx.graphics(|graphics| {
                        if let Some(list) = graphics.get(layer) {
                            for original in list.all_entries() {
                                let mut shape = original.clone();
                                if let Some(transform) = transform {
                                    shape.transform(transform);
                                }
                                collect_labels(&shape.shape, shape.clip_rect, menu_labels);
                            }
                        }
                    });
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
        self.painted_images.clear();
        for clipped in output.shapes {
            collect_labels(&clipped.shape, clipped.clip_rect, &mut self.labels);
            if self.inspect_paint {
                collect_painted_rects(&clipped.shape, clipped.clip_rect, &mut self.painted_rects);
                collect_painted_images(&clipped.shape, clipped.clip_rect, &mut self.painted_images);
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
                self.label_rect("+   Drop audio here to create a track", 0)?
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
        self.inspect_paint = false;
        self.painted_rects.clear();
        self.painted_images.clear();
        match command {
            Command::FileDialog { kind, path } => {
                use crate::document_ui::DocumentDialog;
                self.app.next_document_dialog = Some((
                    match kind {
                        TestDocumentDialog::Save => DocumentDialog::Save,
                        TestDocumentDialog::Open => DocumentDialog::Open,
                        TestDocumentDialog::Export => DocumentDialog::Export,
                    },
                    path,
                ));
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
        while self.app.state_sync_pending()
            || self.app.project_io_pending()
            || self.app.document_guard_diagnostics()["phase"]
                .as_str()
                .is_some_and(|phase| phase != "confirm")
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
            "midi_editor": self.ctx.data(|data| data.get_temp::<Value>(egui::Id::new("midi_editor_diagnostics"))),
            "audio_editor": self.ctx.data(|data| data.get_temp::<Value>(egui::Id::new("audio_editor_diagnostics"))),
            "timeline": self.ctx.data(|data| data.get_temp::<Value>(egui::Id::new("timeline_diagnostics"))),
            "crossfade": self.ctx.data(|data| data.get_temp::<Value>(egui::Id::new("crossfade_diagnostics"))),
            "join_pending": self.app.join_ui.pending(),
            "plugin_catalog": self.app.vst_catalog.iter().map(|entry| json!({
                "name": entry.plugin.name, "class_id": entry.plugin.class_id,
                "thumbnail": entry.thumbnail, "status": format!("{:?}", entry.status),
            })).collect::<Vec<_>>(),
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
        if self.inspect_paint {
            state["painted_rects"] = json!(self.painted_rects);
            state["painted_images"] = json!(self.painted_images);
        }
        state["inspector_page"] = json!(format!("{:?}", self.app.inspector_page));
        state["popup_context"] = self.app.popup_context_diagnostics();
        state["popups"]["export"] = json!(self.app.export_open);
        state["export_include_effect_tails"] = json!(self.app.export_include_effect_tails);
        state["clipboard_text"] = json!(self.clipboard_text);
        state["document_guard"] = self.app.document_guard_diagnostics();
        state["file_dialog_pending"] = json!(self.app.next_document_dialog.is_some());
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
