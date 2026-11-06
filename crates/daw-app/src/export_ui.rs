//! A focused export decision; rendering and native plug-in snapshots stay asynchronous.
use crate::{
    app::{DawApp, PopupKind},
    document_ui::DocumentDialog,
    settings::ExportRangeMode,
};
use anyhow::{Result, ensure};
use daw_core::{ClipContent, Instrument, TrackKind};
use eframe::egui;
use std::{path::PathBuf, sync::Arc, time::Instant};

/// Freeze every delivery choice before native editor synchronization or queued
/// document work. A retry keeps this specification and its original project;
/// re-export applies it to the current acknowledged project snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ExportSpecification {
    pub sample_rate: u32,
    pub options: daw_engine::ExportOptions,
}

impl ExportSpecification {
    pub(crate) fn diagnostics(self) -> serde_json::Value {
        serde_json::json!({
            "sample_rate":self.sample_rate,
            "encoding":self.options.encoding,
            "dither":self.options.dither && self.options.encoding != daw_engine::WavEncoding::Float32,
            "include_effect_tails":self.options.include_effect_tails,
            "range":self.options.range.map(|range| serde_json::json!({"start_beat":range.start_beat,"end_beat":range.end_beat})),
        })
    }

    pub(crate) fn summary(self) -> String {
        format!(
            "{} kHz · Stereo · {}{}",
            f64::from(self.sample_rate) / 1000.0,
            encoding_label(self.options.encoding),
            if self.options.dither && self.options.encoding != daw_engine::WavEncoding::Float32 {
                " · TPDF dither"
            } else {
                ""
            }
        )
    }
}

pub(crate) struct ExportCompletion {
    pub path: PathBuf,
    pub report: daw_engine::ExportReport,
}

pub(crate) fn encoding_label(encoding: daw_engine::WavEncoding) -> &'static str {
    match encoding {
        daw_engine::WavEncoding::Pcm16 => "16-bit PCM",
        daw_engine::WavEncoding::Pcm24 => "24-bit PCM",
        daw_engine::WavEncoding::Float32 => "32-bit float",
    }
}

#[derive(Clone)]
struct ExportRequest {
    project: Arc<daw_core::Project>,
    path: PathBuf,
    specification: ExportSpecification,
    identity: String,
}

/// Retain the exact mix and destination so a retry cannot silently export later edits.
pub(crate) struct ExportRun {
    request: ExportRequest,
    started: Instant,
    failure: Option<crate::error_ui::ErrorReport>,
}

impl DawApp {
    pub(crate) fn legacy_export_specification(
        &self,
        include_effect_tails: bool,
    ) -> ExportSpecification {
        ExportSpecification {
            sample_rate: self.settings.sample_rate,
            options: daw_engine::ExportOptions {
                render: daw_engine::RenderOptions {
                    resample_quality: self.settings.resample_quality,
                },
                include_effect_tails,
                ..Default::default()
            },
        }
    }

    pub(crate) fn start_export(&mut self, path: PathBuf, specification: ExportSpecification) {
        if self.export_job.is_some() {
            self.message(Err(anyhow::anyhow!("An export is already running")), "");
            return;
        }
        let request = ExportRequest {
            project: Arc::new(self.project.clone()),
            path,
            specification,
            identity: self.document_identity(),
        };
        self.begin_export_request(request);
    }

    fn begin_export_request(&mut self, request: ExportRequest) {
        self.export_run = Some(ExportRun {
            request: request.clone(),
            started: Instant::now(),
            failure: None,
        });
        self.exported_file = None;
        // Retry protects both its retained mix sources and any new source that
        // the user imported into the current document after the first attempt.
        if let Err(error) = request
            .project
            .validate_audio_destination(&request.path)
            .and_then(|_| self.project.validate_audio_destination(&request.path))
        {
            self.export_failed_notice(error);
            return;
        }
        self.finishing
            .export_started(request.path.clone(), request.specification);
        let (tx, rx) = std::sync::mpsc::channel();
        self.export_job = Some(rx);
        self.status = "Rendering stereo mixdown in background…".into();
        self.error = false;
        std::thread::spawn(move || {
            use anyhow::Context;
            let result = daw_engine::export_project_wav_with_report(
                &request.project,
                &request.path,
                request.specification.sample_rate,
                request.specification.options,
            )
            .with_context(|| format!("Rendering {}", request.identity))
            .with_context(|| format!("Exporting WAV to {}", request.path.display()))
            .context("WAV export failed")
            .map(|report| ExportCompletion {
                path: request.path,
                report,
            });
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
                let mut diagnostic = run.request.specification.diagnostics();
                let details = serde_json::json!({
                    "path": run.request.path,
                    "identity": run.request.identity,
                    "failed": run.failure.is_some(),
                    "elapsed_seconds": run.started.elapsed().as_secs_f64(),
                });
                diagnostic
                    .as_object_mut()
                    .unwrap()
                    .extend(details.as_object().unwrap().clone());
                diagnostic
            })
    }

    /// Source notes outside a trimmed occurrence are retained, but never rendered.
    pub(crate) fn export_missing_instruments(&self) -> Vec<&str> {
        self.export_missing_instruments_in_range(None)
    }

    fn export_missing_instruments_in_range(
        &self,
        range: Option<daw_engine::ExportRange>,
    ) -> Vec<&str> {
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
                        if notes.iter().any(|note| {
                            let start = note.start.max(clip.offset);
                            let end = (note.start + note.length).min(clip.source_end());
                            note.velocity > 0 && start < end && range.is_none_or(|range|
                                clip.start + clip.source_to_local(start) < range.end_beat
                                && clip.start + clip.source_to_local(end) > range.start_beat)
                        }))
                })
            })
            .map(|track| track.name.as_str())
            .collect()
    }

    pub(crate) fn export_midi_ready(&self) -> bool {
        self.export_job.is_none() && !self.state_sync_pending() && !self.project_io_pending()
    }

    pub(crate) fn export_midi_dialog(&mut self) {
        // A native picker blocks polling. Let an earlier mixdown finish before
        // starting another export so its result cannot replace this MIDI result.
        if !self.export_midi_ready() {
            return;
        }
        if let Some(path) = self.pick_document_path(DocumentDialog::ExportMidi) {
            let result = self.project.export_midi(&path);
            match result {
                Ok(()) => {
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    self.message(Ok(()), &format!("Exported {name}"));
                    self.finishing.midi_export_finished(&path);
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

    fn preferred_export_specification(&self) -> Result<ExportSpecification> {
        let preferences = &self.settings.export;
        let range = match preferences.range_mode {
            ExportRangeMode::Arrangement => None,
            ExportRangeMode::Loop => {
                ensure!(
                    self.project.loop_start.is_finite()
                        && self.project.loop_end.is_finite()
                        && self.project.loop_start >= 0.0
                        && self.project.loop_end > self.project.loop_start
                        && self.project.loop_end <= daw_core::MAX_BEAT,
                    "Set increasing loop markers before exporting a loop range"
                );
                Some(daw_engine::ExportRange {
                    start_beat: self.project.loop_start,
                    end_beat: self.project.loop_end,
                })
            }
        };
        let sample_rate = preferences.sample_rate.unwrap_or(self.settings.sample_rate);
        ensure!(
            (8_000..=384_000).contains(&sample_rate),
            "Export sample rate must be between 8000 and 384000 Hz"
        );
        Ok(ExportSpecification {
            sample_rate,
            options: daw_engine::ExportOptions {
                render: daw_engine::RenderOptions {
                    resample_quality: self.settings.resample_quality,
                },
                include_effect_tails: preferences.include_effect_tails && range.is_none(),
                encoding: preferences.encoding,
                dither: preferences.dither,
                range,
            },
        })
    }

    pub(crate) fn export_popup(&mut self, ctx: &egui::Context) {
        let mut open = self.export_open;
        if !open {
            return;
        }
        let previous_preferences = self.settings.export.clone();
        let specification = self.preferred_export_specification();
        let loop_mode = self.settings.export.range_mode == ExportRangeMode::Loop;
        let duration_beats = if loop_mode {
            self.project.loop_end - self.project.loop_start
        } else {
            self.project.end_beat()
        };
        let duration = duration_beats * 60.0 / self.project.tempo;
        let duration = format!(
            "{} · {}{}",
            if loop_mode {
                "Loop range"
            } else {
                "Arrangement"
            },
            daw_ui::format_clock_time(duration, 3),
            if self.settings.export.include_effect_tails && !loop_mode {
                " + tails"
            } else {
                ""
            },
        );
        let mut export = false;
        let mut cancel = false;
        let missing_instruments = self
            .export_missing_instruments_in_range(
                specification
                    .as_ref()
                    .ok()
                    .and_then(|spec| spec.options.range),
            )
            .join(", ");
        daw_ui::popup("Export WAV", ctx)
            .id(egui::Id::new("export_options"))
            .default_width(daw_ui::tokens::POPUP_WIDTH_SM)
            .vscroll(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let mut footer = daw_ui::popup_footer(ui, "export_actions");
                let window_ui = ui;
                let ui = footer.body();
                daw_ui::popup_scroll_body(ui, "export_summary", 0.0, |ui| {
                daw_ui::text::body(ui, duration).on_hover_text(
                    if self.settings.export.include_effect_tails && !loop_mode {
                        "The file includes releases and effect tails beyond this arrangement duration."
                    } else {
                        "The file ends exactly at this arrangement duration."
                    }
                );
                if let Ok(specification) = &specification {
                    daw_ui::text::note(ui, specification.summary());
                }
                ui.add_space(daw_ui::tokens::SPACE_MD);
                daw_ui::form_grid(ui, "wav_delivery_choices", |ui| {
                    daw_ui::form_label(ui, "Format");
                    daw_ui::choice_menu(ui, encoding_label(self.settings.export.encoding), |ui| {
                        for encoding in [daw_engine::WavEncoding::Pcm16, daw_engine::WavEncoding::Pcm24, daw_engine::WavEncoding::Float32] {
                            if daw_ui::selectable_value(ui, &mut self.settings.export.encoding, encoding, encoding_label(encoding)).clicked() { ui.close(); }
                        }
                    });
                    ui.end_row();
                    daw_ui::form_label(ui, "Rate");
                    let rate_label = self.settings.export.sample_rate.map_or_else(|| "Follow audio setup".to_owned(), |rate| format!("{} kHz", f64::from(rate) / 1000.0));
                    daw_ui::choice_menu(ui, rate_label, |ui| {
                        if daw_ui::selectable_value(ui, &mut self.settings.export.sample_rate, None, "Follow audio setup").clicked() { ui.close(); }
                        for rate in [44_100,48_000,88_200,96_000,176_400,192_000] {
                            if daw_ui::selectable_value(ui, &mut self.settings.export.sample_rate, Some(rate), format!("{} kHz", f64::from(rate) / 1000.0)).clicked() { ui.close(); }
                        }
                    });
                    ui.end_row();
                    daw_ui::form_label(ui, "Range");
                    daw_ui::choice_menu(ui, if loop_mode { "Loop range" } else { "Arrangement" }, |ui| {
                        for (mode, label) in [(ExportRangeMode::Arrangement,"Arrangement"),(ExportRangeMode::Loop,"Loop range")] {
                            if daw_ui::selectable_value(ui, &mut self.settings.export.range_mode, mode, label).clicked() { ui.close(); }
                        }
                    });
                    ui.end_row();
                });
                ui.add_space(daw_ui::tokens::SPACE_MD);
                ui.add_enabled_ui(!loop_mode, |ui| daw_ui::checkbox(ui, &mut self.settings.export.include_effect_tails, "Include effect tails")
                    .on_hover_text("Keep instrument releases and effect tails after the arrangement. Turn off to end exactly at the arrangement boundary."));
                let integer = self.settings.export.encoding != daw_engine::WavEncoding::Float32;
                ui.add_enabled_ui(integer, |ui| daw_ui::checkbox(ui, &mut self.settings.export.dither, "Dither (TPDF)"));
                if loop_mode {
                    daw_ui::text::note(ui, format!("Loop markers · Beats {}–{} · exact length",
                        daw_ui::parameter_value_text(self.project.loop_start+1.0), daw_ui::parameter_value_text(self.project.loop_end+1.0)));
                }
                if integer {
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note,
                        "PCM levels above 0 dBFS are clipped and reported after export.", 2);
                }
                if let Err(error) = &specification { daw_ui::text::body(ui, error.to_string()); }
                if !missing_instruments.is_empty() {
                    ui.add_space(daw_ui::tokens::SPACE_LG);
                    daw_ui::notice(ui, daw_ui::NoticeKind::Warning, "MIDI notes need an instrument", |_| {});
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Body,
                        format!("No instrument: {missing_instruments}"), 3);
                    daw_ui::text::body(ui, "These notes will be silent in the WAV. Choose instruments on their tracks, or continue without them.");
                }
                });
                footer.show(window_ui, |ui| daw_ui::dialog_actions(ui, |ui| {
                    export = ui.add_enabled_ui(specification.is_ok(), |ui| daw_ui::dialog_primary(ui, "Export…")).inner.clicked();
                    cancel = daw_ui::dialog_button(ui, "Cancel").clicked();
                }, |_| {}));
            });
        self.export_open = open && !cancel;
        if self.settings.export != previous_preferences
            && let Err(error) = self.save_independent_preferences()
        {
            self.message(Err(error.context("Saving export preferences")), "");
        }
        if export {
            let specification = match self.preferred_export_specification() {
                Ok(specification) => specification,
                Err(error) => {
                    self.message(Err(error), "");
                    return;
                }
            };
            self.close_popup();
            if let Some(path) = self.pick_document_path(DocumentDialog::Export) {
                self.export_to_with_specification(path, specification);
            }
        }
    }
}
