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

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Inspect,
    Click {
        label: String,
        #[serde(default)]
        occurrence: usize,
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
    Export {
        path: PathBuf,
    },
    WaitExport {
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
    },
    Quit,
}

fn default_timeout() -> u64 {
    30_000
}

struct Harness {
    ctx: egui::Context,
    app: DawApp,
    time: f64,
    labels: Vec<(String, Rect)>,
    close_requested: bool,
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
        app.settings = Settings {
            scratch_dir,
            ..Settings::default()
        };
        app.setup_draft = app.settings.clone();
        let mut harness = Self {
            ctx,
            app,
            time: 0.0,
            labels: Vec::new(),
            close_requested: false,
        };
        harness.settle();
        Ok(harness)
    }

    fn frame(&mut self, mut input: RawInput) {
        self.time += 1.0 / 60.0;
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, SIZE));
        input.time = Some(self.time);
        // Periodic recovery is wall-clock driven in the native app. Closing still
        // exercises recovery normally, within the isolated automation directory.
        self.app.last_recovery = Instant::now();
        let output = self.ctx.run(input, |ctx| self.app.ui_frame(ctx));
        self.close_requested |= output.viewport_output.values().any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        });
        self.labels.clear();
        for clipped in output.shapes {
            collect_labels(&clipped.shape, clipped.clip_rect, &mut self.labels);
        }
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(RawInput::default());
        }
    }

    fn label_rect(&self, label: &str, occurrence: usize) -> Result<Rect> {
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
        let (mut low, mut high) = (0.0, SIZE.x - 1.0);
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
        match command {
            Command::Inspect => self.settle(),
            Command::Click { label, occurrence } => self.click(&label, occurrence)?,
            Command::Key {
                key,
                shift,
                ctrl,
                alt,
                cmd,
            } => {
                let key = egui::Key::from_name(&key).context("Unknown keyboard key")?;
                let modifiers = egui::Modifiers {
                    alt,
                    ctrl,
                    shift,
                    mac_cmd: cmd && cfg!(target_os = "macos"),
                    command: cmd || ctrl,
                };
                for pressed in [true, false] {
                    self.frame(RawInput {
                        modifiers,
                        events: vec![Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers,
                        }],
                        ..Default::default()
                    });
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
                while self.app.export_job.is_some() {
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

    fn close(&mut self) -> Result<()> {
        let mut input = RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .context("Missing root viewport")?
            .events
            .push(egui::ViewportEvent::Close);
        self.frame(input);
        ensure!(!self.app.error, "{}", self.app.status);
        Ok(())
    }

    fn state(&self) -> Value {
        json!({
            "project": self.app.project,
            "selected_track": self.app.selected_track,
            "selected_clip": self.app.selected_clip,
            "view": format!("{:?}", self.app.view),
            "popups": {
                "library": self.app.library_open, "setup": self.app.setup_open,
                "properties": self.app.inspector_open, "instrument": self.app.instrument_open,
                "effects": self.app.effects_open, "routing": self.app.routing_open,
            },
            "settings": self.app.settings,
            "setup_draft": self.app.setup_draft,
            "status": self.app.status,
            "error": self.app.error,
            "export_pending": self.app.export_job.is_some(),
            "device_open": self.app.engine.is_some(),
            "scratch_dir": self.app.settings.scratch_dir,
            "drop_target": self.app.drop_target,
            "pointer_position": self.ctx.input(|input| input.pointer.hover_pos().map(|point| [point.x, point.y])),
            "visible_labels": self.labels.iter().map(|(label, _)| label.trim()).collect::<Vec<_>>(),
            "label_rects": self.labels.iter().map(|(label, rect)| json!({
                "text": label.trim(), "min": [rect.min.x, rect.min.y], "max": [rect.max.x, rect.max.y],
            })).collect::<Vec<_>>(),
        })
    }
}

fn collect_labels(shape: &egui::Shape, clip: Rect, labels: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Text(text) => {
            let rect = Rect::from_min_size(text.pos, text.galley.size()).intersect(clip);
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
    if harness.app.export_job.is_some() {
        bail!("Input closed while an export was running");
    }
    Ok(())
}

fn write_response(output: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
