//! A focused export decision; rendering and native plug-in snapshots stay asynchronous.
use crate::{
    app::{DawApp, PopupKind},
    document_ui::DocumentDialog,
};
use daw_core::{ClipContent, Instrument, TrackKind};
use eframe::egui;
use std::{path::PathBuf, sync::Arc, time::Instant};

#[derive(Clone)]
struct ExportRequest {
    project: Arc<daw_core::Project>,
    path: PathBuf,
    sample_rate: u32,
    options: daw_engine::ExportOptions,
    identity: String,
}

/// Retain the exact mix and destination so a retry cannot silently export later edits.
pub(crate) struct ExportRun {
    request: ExportRequest,
    started: Instant,
    failure: Option<crate::error_ui::ErrorReport>,
}

impl DawApp {
    pub(crate) fn start_export(&mut self, path: PathBuf, include_effect_tails: bool) {
        if self.export_job.is_some() {
            self.message(Err(anyhow::anyhow!("An export is already running")), "");
            return;
        }
        let request = ExportRequest {
            project: Arc::new(self.project.clone()),
            path,
            sample_rate: self.settings.sample_rate,
            options: daw_engine::ExportOptions {
                render: daw_engine::RenderOptions {
                    resample_quality: self.settings.resample_quality,
                },
                include_effect_tails,
            },
            identity: self.document_identity(),
        };
        self.begin_export_request(request);
    }

    fn begin_export_request(&mut self, request: ExportRequest) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.export_job = Some(rx);
        self.export_run = Some(ExportRun {
            request: request.clone(),
            started: Instant::now(),
            failure: None,
        });
        self.exported_file = None;
        self.status = "Rendering stereo mixdown in background…".into();
        self.error = false;
        std::thread::spawn(move || {
            use anyhow::Context;
            let result = daw_engine::export_project_wav_with_options(
                &request.project,
                &request.path,
                request.sample_rate,
                request.options,
            )
            .with_context(|| format!("Rendering {}", request.identity))
            .with_context(|| format!("Exporting WAV to {}", request.path.display()))
            .context("WAV export failed")
            .map(|_| request.path);
            let _ = tx.send(result);
        });
    }

    pub(crate) fn export_progress_message(&self) -> Option<String> {
        self.export_job.as_ref()?;
        Some(self.export_run.as_ref().map_or_else(
            || "Exporting WAV · wait for completion".into(),
            |run| {
                let path = &run.request.path;
                let name = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy();
                format!(
                    "Exporting · {name} · {} s elapsed",
                    run.started.elapsed().as_secs()
                )
            },
        ))
    }

    pub(crate) fn export_failed(&self) -> bool {
        self.export_run
            .as_ref()
            .is_some_and(|run| run.failure.is_some())
    }

    pub(crate) fn export_error_report(&self) -> Option<&crate::error_ui::ErrorReport> {
        self.export_run.as_ref()?.failure.as_ref()
    }

    pub(crate) fn export_failed_notice(&mut self, error: anyhow::Error) {
        self.message(Err(error), "");
        if let Some(run) = &mut self.export_run {
            run.failure = self.last_error_report.clone();
        }
        self.document_export_failed(self.status.clone());
    }

    pub(crate) fn retry_export(&mut self) {
        if self.export_job.is_some() || self.project_io_pending() || self.state_sync_pending() {
            return;
        }
        if let Some(run) = self.export_run.as_ref().filter(|_| self.export_failed()) {
            self.begin_export_request(run.request.clone());
        }
    }

    pub(crate) fn export_diagnostics(&self) -> serde_json::Value {
        self.export_run
            .as_ref()
            .map_or(serde_json::Value::Null, |run| {
                serde_json::json!({
                    "path": run.request.path,
                    "identity": run.request.identity,
                    "include_effect_tails": run.request.options.include_effect_tails,
                    "sample_rate": run.request.sample_rate,
                    "failed": run.failure.is_some(),
                    "elapsed_seconds": run.started.elapsed().as_secs_f64(),
                })
            })
    }

    /// Source notes outside a trimmed occurrence are retained, but never rendered.
    pub(crate) fn export_missing_instruments(&self) -> Vec<&str> {
        let audible = daw_engine::audible_track_ids(&self.project).unwrap_or_default();
        self.project
            .tracks
            .iter()
            .filter(|track| {
                audible.contains(&track.id)
                    && track.kind == TrackKind::Midi
                    && track.instrument == Instrument::None
            })
            .filter(|track| {
                track.clips.iter().any(|clip| {
                    clip.length > 0.0
                        && matches!(&clip.content, ClipContent::Midi { notes }
                        if notes.iter().any(|note| note.velocity > 0
                            && note.start.max(clip.offset)
                                < (note.start + note.length).min(clip.source_end())))
                })
            })
            .map(|track| track.name.as_str())
            .collect()
    }

    pub(crate) fn export_midi_dialog(&mut self) {
        if let Some(path) = self.pick_document_path(DocumentDialog::ExportMidi) {
            self.exported_file = None;
            let result = self.project.export_midi(&path);
            match result {
                Ok(()) => {
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    self.message(Ok(()), &format!("Exported {name}"));
                    self.exported_file = Some(path);
                }
                Err(error) => self.message(Err(error.context("MIDI export failed")), ""),
            }
        }
    }

    pub(crate) fn export(&mut self) {
        if self.export_job.is_some() {
            self.status = "A mixdown is already rendering".into();
            return;
        }
        self.open_popup(PopupKind::Export);
    }

    pub(crate) fn export_popup(&mut self, ctx: &egui::Context) {
        let mut open = self.export_open;
        if !open {
            return;
        }
        let duration = self.project.end_beat() * 60.0 / self.project.tempo;
        let milliseconds = (duration * 1000.0).round() as u64;
        let duration = format!(
            "Arrangement · {}:{:02}.{:03}",
            milliseconds / 60_000,
            milliseconds / 1000 % 60,
            milliseconds % 1000,
        );
        let mut export = false;
        let mut cancel = false;
        let missing_instruments = self.export_missing_instruments().join(", ");
        daw_ui::popup("Export WAV", ctx)
            .id(egui::Id::new("export_options"))
            .default_width(360.0)
            .vscroll(false)
            .open(&mut open)
            .show(ctx, |ui| {
                daw_ui::text::body(ui, duration);
                if !missing_instruments.is_empty() {
                    ui.add_space(daw_ui::tokens::SPACE_MD);
                    daw_ui::notice(ui, daw_ui::NoticeKind::Warning, "MIDI notes need an instrument", |_| {});
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Body,
                        format!("No instrument: {missing_instruments}"), 3);
                    daw_ui::text::body(ui, "These notes will be silent in the WAV. Choose instruments on their tracks, or continue without them.");
                }
                ui.add_space(daw_ui::tokens::SPACE_SM);
                daw_ui::checkbox(ui, &mut self.export_include_effect_tails, "Include effect tails")
                    .on_hover_text("Keep instrument releases and effect tails after the arrangement. Turn off to end exactly at the arrangement boundary.");
                ui.add_space(daw_ui::tokens::SPACE_MD);
                ui.horizontal(|ui| {
                    export = daw_ui::primary_button(ui, "Export…").clicked();
                    cancel = daw_ui::button(ui, "Cancel").clicked();
                });
            });
        self.export_open = open && !cancel;
        if export {
            let include_effect_tails = self.export_include_effect_tails;
            self.close_popup();
            if let Some(path) = self.pick_document_path(DocumentDialog::Export) {
                self.export_to_with_tails(path, include_effect_tails);
            }
        }
    }
}
