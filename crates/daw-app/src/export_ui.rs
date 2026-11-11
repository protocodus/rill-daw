//! A focused export decision; rendering and native plug-in snapshots stay asynchronous.
use crate::{
    app::{DawApp, PopupKind},
    document_ui::DocumentDialog,
    settings::{ExportRangeMode, ExportWhat},
};
use anyhow::{Context, Result, ensure};
use daw_core::{ClipContent, Instrument, TrackKind};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Freeze every delivery choice before native editor synchronization or queued
/// document work. A retry keeps this specification and its original project;
/// re-export applies it to the current acknowledged project snapshot.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExportSpecification {
    pub sample_rate: u32,
    pub options: daw_engine::ExportOptions,
    /// None exports one mix file; otherwise the path is a folder of stems.
    pub stems: Option<StemSelection>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StemSelection {
    /// Channels in project order.
    pub channels: Vec<u64>,
    pub include_mix: bool,
    pub through_master: bool,
    /// Base of every file name.
    pub song: String,
    pub replace_existing: bool,
}

impl ExportSpecification {
    pub(crate) fn diagnostics(&self) -> serde_json::Value {
        let integer = self.options.format.is_integer(self.options.encoding);
        serde_json::json!({
            "sample_rate":self.sample_rate,
            "encoding":self.options.encoding,
            "format":self.options.format,
            "dither":self.options.dither && integer,
            "include_effect_tails":self.options.include_effect_tails,
            "range":self.options.range.map(|range| serde_json::json!({"start_beat":range.start_beat,"end_beat":range.end_beat})),
            "stems":self.stems.as_ref().map(|stems| serde_json::json!({
                "channels":stems.channels,"include_mix":stems.include_mix,"through_master":stems.through_master,
            })),
        })
    }

    pub(crate) fn summary(&self) -> String {
        let options = &self.options;
        let dither = if options.dither && options.format.is_integer(options.encoding) {
            " · TPDF dither"
        } else {
            ""
        };
        let format = match options.format {
            daw_engine::ExportFormat::Wav => encoding_label(options.encoding).to_owned(),
            daw_engine::ExportFormat::Flac => {
                format!("{}-bit FLAC", options.encoding.bits_per_sample())
            }
            daw_engine::ExportFormat::Vorbis { quality } => {
                format!("Ogg Vorbis · quality {quality}")
            }
            daw_engine::ExportFormat::Mp3 { kbps } => format!("MP3 · {kbps} kbit/s"),
        };
        let stems = self.stems.as_ref().map_or(String::new(), |stems| {
            format!(
                "{} stems · ",
                stems.channels.len() + usize::from(stems.include_mix)
            )
        });
        format!(
            "{stems}{} kHz · Stereo · {format}{dither}",
            f64::from(self.sample_rate) / 1000.0,
        )
    }
}

pub(crate) struct ExportCompletion {
    /// The file, or for stems the folder.
    pub path: PathBuf,
    /// For stems, the loudest peak and every overload across the files.
    pub report: daw_engine::ExportReport,
    /// Number of stem files written; zero for a single file.
    pub stems: usize,
    pub files: Vec<PathBuf>,
}

pub(crate) fn encoding_label(encoding: daw_engine::WavEncoding) -> &'static str {
    match encoding {
        daw_engine::WavEncoding::Pcm16 => "16-bit PCM",
        daw_engine::WavEncoding::Pcm24 => "24-bit PCM",
        daw_engine::WavEncoding::Float32 => "32-bit float",
    }
}

/// Typical bit rate of each Vorbis quality, for the quality choice.
fn vorbis_kbps(quality: u8) -> u16 {
    [64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 500][usize::from(quality.min(10))]
}

/// Folders and instrument outputs are listed beside tracks; instrument
/// outputs start selected because they are what the instrument sounds like.
fn stem_choices(project: &daw_core::Project) -> Vec<(u64, String)> {
    project
        .tracks
        .iter()
        .filter(|track| track.kind != TrackKind::Automation)
        .map(|track| {
            let suffix = match track.kind {
                _ if track.instrument_output.is_some() => " (output)",
                TrackKind::Bus => " (bus)",
                TrackKind::Folder => " (folder)",
                _ => "",
            };
            (track.id, format!("{}{suffix}", track.name))
        })
        .collect()
}

pub(crate) fn default_stem_channels(project: &daw_core::Project) -> Vec<u64> {
    project
        .tracks
        .iter()
        .filter(|track| {
            matches!(track.kind, TrackKind::Audio | TrackKind::Midi)
                || track.instrument_output.is_some()
        })
        .map(|track| track.id)
        .collect()
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
            stems: None,
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
        self.exported_stems = 0;
        self.exported_files.clear();
        // Retry protects both its retained mix sources and any new source that
        // the user imported into the current document after the first attempt.
        // Stem destinations are checked file by file by the exporter.
        if request.specification.stems.is_none()
            && let Err(error) = request
                .project
                .validate_audio_destination(&request.path)
                .and_then(|_| self.project.validate_audio_destination(&request.path))
        {
            self.export_failed_notice(error);
            return;
        }
        self.finishing
            .export_started(request.path.clone(), request.specification.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.export_job = Some(rx);
        let cancel = Arc::new(AtomicBool::new(false));
        self.export_cancel = Some(Arc::clone(&cancel));
        self.status = if request.specification.stems.is_some() {
            "Rendering stems in background…".into()
        } else {
            "Rendering stereo mixdown in background…".into()
        };
        self.error = false;
        std::thread::spawn(move || {
            let _ = tx.send(run_export(&request, &cancel));
        });
    }

    /// Stop the running export; no file of it is written or replaced.
    pub(crate) fn cancel_export(&mut self) {
        if let Some(cancel) = &self.export_cancel {
            cancel.store(true, Ordering::Release);
        }
    }

    pub(crate) fn export_progress_message(&self) -> Option<String> {
        self.export_job.as_ref()?;
        Some(self.export_run.as_ref().map_or_else(
            || "Exporting audio · wait for completion".into(),
            |run| {
                let path = &run.request.path;
                let name = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy();
                format!(
                    "Exporting {}· {name} · {} s elapsed",
                    run.request.specification.stems.as_ref().map_or(
                        String::new(),
                        |stems| format!(
                            "{} stems ",
                            stems.channels.len() + usize::from(stems.include_mix)
                        )
                    ),
                    run.started.elapsed().as_secs()
                )
            },
        ))
    }

    /// The completion notice's subject: the file, or the stem folder.
    pub(crate) fn exported_name(&self, path: &Path) -> String {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        if self.exported_stems > 0 {
            format!("{} stems · {name}", self.exported_stems)
        } else {
            name.into_owned()
        }
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
        if self.project.contains_midi2() {
            self.open_popup(PopupKind::ExportMidi);
            return;
        }
        self.finish_midi_export_dialog();
    }

    fn finish_midi_export_dialog(&mut self) {
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

    fn midi_export_confirmation(&mut self, ctx: &egui::Context) {
        let mut open = self.midi_export_open;
        if !open {
            return;
        }
        let mut export = false;
        let mut cancel = false;
        daw_ui::page("Export MIDI 1.0 copy")
            .id(egui::Id::new("midi_export_confirmation"))
            .width(daw_ui::tokens::POPUP_WIDTH_SM)
            .open(&mut open).show(ctx, |ui| {
                let mut footer = daw_ui::popup_footer(ui, "midi_export_confirmation_actions");
                let window_ui = ui;
                daw_ui::popup_scroll_body(footer.body(), "midi_export_confirmation_body", 0.0, |ui| {
                    daw_ui::text::body(ui, "Standard MIDI (.mid) exports MIDI 1.0 notes. High-resolution pitch and velocity are reduced; UMP messages and per-note expression are omitted.");
                    ui.add_space(daw_ui::tokens::SPACE_MD);
                    daw_ui::text::note(ui, "Your RILL project keeps the original performance data. Save a .rill project to preserve it.");
                });
                footer.show(window_ui, |ui| {
                    daw_ui::dialog_actions(ui, |ui| {
                        export = ui.add_enabled(self.export_midi_ready(), daw_ui::primary_button_widget(ui, "Export MIDI 1.0 copy…")).clicked();
                    }, |ui| { cancel = daw_ui::button(ui, "Cancel").clicked(); });
                });
            });
        self.midi_export_open = open && !cancel && !export;
        if export {
            self.finish_midi_export_dialog();
        }
    }

    pub(crate) fn export(&mut self) {
        if self.export_job.is_some() {
            self.status = "A mixdown is already rendering".into();
            return;
        }
        self.export_stem_selection = default_stem_channels(&self.project).into_iter().collect();
        self.open_popup(PopupKind::Export);
    }

    /// Selected stem channels in project order; deleted channels drop out.
    fn selected_stem_channels(&self) -> Vec<u64> {
        self.project
            .tracks
            .iter()
            .filter(|track| self.export_stem_selection.contains(&track.id))
            .filter(|track| track.kind != TrackKind::Automation)
            .map(|track| track.id)
            .collect()
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
        preferences
            .format
            .validate(preferences.encoding, sample_rate)?;
        let stems = (preferences.what == ExportWhat::Stems)
            .then(|| {
                let channels = self.selected_stem_channels();
                ensure!(
                    !channels.is_empty() || preferences.stems_include_mix,
                    "Choose at least one channel or include the full mix"
                );
                Ok(StemSelection {
                    channels,
                    include_mix: preferences.stems_include_mix,
                    through_master: preferences.stems_through_master,
                    song: self.document_file_stem(),
                    replace_existing: false,
                })
            })
            .transpose()?;
        Ok(ExportSpecification {
            sample_rate,
            options: daw_engine::ExportOptions {
                render: daw_engine::RenderOptions {
                    resample_quality: self.settings.resample_quality,
                },
                include_effect_tails: preferences.include_effect_tails && range.is_none(),
                encoding: preferences.encoding,
                format: preferences.format,
                dither: preferences.dither,
                range,
            },
            stems,
        })
    }

    /// File names a stem export of `specification` writes.
    fn planned_stem_files(&self, specification: &ExportSpecification) -> Vec<String> {
        let Some(stems) = &specification.stems else {
            return Vec::new();
        };
        daw_engine::plan_stems(
            &self.project,
            &stems.song,
            &stems.channels,
            stems.include_mix,
            specification.options.format,
        )
        .map(|plans| plans.into_iter().map(|plan| plan.file_name).collect())
        .unwrap_or_default()
    }

    pub(crate) fn export_popup(&mut self, ctx: &egui::Context) {
        self.midi_export_confirmation(ctx);
        let mut open = self.export_open;
        if !open {
            return;
        }
        let previous_preferences = self.settings.export.clone();
        let specification = self.preferred_export_specification();
        let loop_mode = self.settings.export.range_mode == ExportRangeMode::Loop;
        let stems_mode = self.settings.export.what == ExportWhat::Stems;
        let format = self.settings.export.format;
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
        let choices = stem_choices(&self.project);
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
        daw_ui::page("Export audio")
            .id(egui::Id::new("export_options"))
            .width(daw_ui::tokens::FORM_WIDTH)
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
                ui.add_space(daw_ui::tokens::SPACE_SM);
                let integer = format.is_integer(self.settings.export.encoding);
                daw_ui::form_grid(ui, "wav_delivery_choices", |ui| {
                    let label = daw_ui::form_label(ui, "What");
                    ui.horizontal(|ui| {
                        for (what, name) in [(ExportWhat::Mix, "Mix"), (ExportWhat::Stems, "Stems")] {
                            if daw_ui::tab_choice(ui, self.settings.export.what == what, name).clicked() {
                                self.settings.export.what = what;
                            }
                        }
                    }).response.labelled_by(label.id);
                    ui.end_row();
                    let label = daw_ui::form_label(ui, "Type");
                    daw_ui::form_select("export_type", format.label()).show_ui(ui, |ui| {
                        for choice in [
                            daw_engine::ExportFormat::Wav,
                            daw_engine::ExportFormat::Flac,
                            daw_engine::ExportFormat::VORBIS_DEFAULT,
                            daw_engine::ExportFormat::MP3_DEFAULT,
                        ] {
                            let selected = std::mem::discriminant(&choice) == std::mem::discriminant(&format);
                            if daw_ui::selectable(ui, selected, choice.label()).clicked() && !selected {
                                self.settings.export.format = choice;
                                // FLAC has no floating-point depth.
                                if choice == daw_engine::ExportFormat::Flac
                                    && self.settings.export.encoding == daw_engine::WavEncoding::Float32
                                {
                                    self.settings.export.encoding = daw_engine::WavEncoding::Pcm24;
                                }
                            }
                        }
                    }).response.labelled_by(label.id);
                    ui.end_row();
                    match format {
                        daw_engine::ExportFormat::Wav | daw_engine::ExportFormat::Flac => {
                            let label = daw_ui::form_label(ui, "Format");
                            daw_ui::form_select("wav_format", encoding_label(self.settings.export.encoding)).show_ui(ui, |ui| {
                                let float = format == daw_engine::ExportFormat::Wav;
                                for encoding in [daw_engine::WavEncoding::Pcm16, daw_engine::WavEncoding::Pcm24, daw_engine::WavEncoding::Float32] {
                                    if encoding != daw_engine::WavEncoding::Float32 || float {
                                        daw_ui::selectable_value(ui, &mut self.settings.export.encoding, encoding, encoding_label(encoding));
                                    }
                                }
                            }).response.labelled_by(label.id);
                            ui.end_row();
                        }
                        daw_engine::ExportFormat::Vorbis { quality } => {
                            let label = daw_ui::form_label(ui, "Quality");
                            daw_ui::form_select("vorbis_quality", format!("{quality} · ≈{} kbit/s", vorbis_kbps(quality))).show_ui(ui, |ui| {
                                for choice in 0..=10 {
                                    daw_ui::selectable_value(ui, &mut self.settings.export.format,
                                        daw_engine::ExportFormat::Vorbis { quality: choice },
                                        format!("{choice} · ≈{} kbit/s", vorbis_kbps(choice)));
                                }
                            }).response.labelled_by(label.id);
                            ui.end_row();
                        }
                        daw_engine::ExportFormat::Mp3 { kbps } => {
                            let label = daw_ui::form_label(ui, "Bitrate");
                            daw_ui::form_select("mp3_bitrate", format!("{kbps} kbit/s")).show_ui(ui, |ui| {
                                for choice in daw_engine::ExportFormat::MP3_BITRATES {
                                    daw_ui::selectable_value(ui, &mut self.settings.export.format,
                                        daw_engine::ExportFormat::Mp3 { kbps: choice },
                                        format!("{choice} kbit/s"));
                                }
                            }).response.labelled_by(label.id);
                            ui.end_row();
                        }
                    }
                    let label = daw_ui::form_label(ui, "Rate");
                    let rate_label = self.settings.export.sample_rate.map_or_else(|| "Follow audio setup".to_owned(), |rate| format!("{} kHz", f64::from(rate) / 1000.0));
                    daw_ui::form_select("wav_rate", rate_label).show_ui(ui, |ui| {
                        daw_ui::selectable_value(ui, &mut self.settings.export.sample_rate, None, "Follow audio setup");
                        for rate in [44_100,48_000,88_200,96_000,176_400,192_000] {
                            daw_ui::selectable_value(ui, &mut self.settings.export.sample_rate, Some(rate), format!("{} kHz", f64::from(rate) / 1000.0));
                        }
                    }).response.labelled_by(label.id);
                    ui.end_row();
                    let label = daw_ui::form_label(ui, "Range");
                    daw_ui::form_select("wav_range", if loop_mode { "Loop range" } else { "Arrangement" }).show_ui(ui, |ui| {
                        for (mode, label) in [(ExportRangeMode::Arrangement,"Arrangement"),(ExportRangeMode::Loop,"Loop range")] {
                            daw_ui::selectable_value(ui, &mut self.settings.export.range_mode, mode, label);
                        }
                    }).response.labelled_by(label.id);
                    ui.end_row();
                    // Options sit in the field column, under the choices they qualify.
                    ui.label("");
                    ui.add_enabled_ui(!loop_mode, |ui| daw_ui::checkbox(ui, &mut self.settings.export.include_effect_tails, "Include effect tails")
                        .on_hover_text("Keep instrument releases and effect tails after the arrangement. Turn off to end exactly at the arrangement boundary."));
                    ui.end_row();
                    ui.label("");
                    ui.add_enabled_ui(integer, |ui| daw_ui::checkbox(ui, &mut self.settings.export.dither, "Dither (TPDF)"));
                    ui.end_row();
                    if stems_mode {
                        ui.label("");
                        daw_ui::checkbox(ui, &mut self.settings.export.stems_include_mix, "Include full mix")
                            .on_hover_text("Also write the complete mix, with Master processing, aligned with the stems.");
                        ui.end_row();
                        ui.label("");
                        daw_ui::checkbox(ui, &mut self.settings.export.stems_through_master, "Through Master processing")
                            .on_hover_text("Off: stems are taken before Master inserts, fader and pan, so they can be mixed elsewhere. On: each stem also passes through Master.");
                        ui.end_row();
                    }
                });
                if stems_mode {
                    ui.add_space(daw_ui::tokens::SPACE_SM);
                    daw_ui::text::note(ui, "Each stem is the song with only that channel audible, including its sends. All files share one length.");
                    daw_ui::checklist(ui, "export_stem_channels", &choices, &mut self.export_stem_selection, 200.0);
                }
                if loop_mode {
                    daw_ui::text::note(ui, format!("Loop markers · Beats {}–{} · exact length",
                        daw_ui::parameter_value_text(self.project.loop_start+1.0), daw_ui::parameter_value_text(self.project.loop_end+1.0)));
                }
                if format == daw_engine::ExportFormat::Wav && integer {
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note,
                        "PCM levels above 0 dBFS are clipped and reported after export.", 2);
                } else if format != daw_engine::ExportFormat::Wav {
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note,
                        "Levels above 0 dBFS are limited and reported after export.", 2);
                }
                if format.is_lossy() {
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Note,
                        "Lossy files can decode slightly above 0 dBFS; leave some headroom.", 2);
                }
                if let Err(error) = &specification { daw_ui::text::body(ui, error.to_string()); }
                if !missing_instruments.is_empty() {
                    ui.add_space(daw_ui::tokens::SPACE_LG);
                    daw_ui::notice(ui, daw_ui::NoticeKind::Warning, "MIDI notes need an instrument", |_| {});
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Body,
                        format!("No instrument: {missing_instruments}"), 3);
                    daw_ui::text::body(ui, "These notes will be silent in the exported audio. Choose instruments on their tracks, or continue without them.");
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
            let mut specification = match self.preferred_export_specification() {
                Ok(specification) => specification,
                Err(error) => {
                    self.message(Err(error), "");
                    return;
                }
            };
            self.close_popup();
            if specification.stems.is_some() {
                let planned = self.planned_stem_files(&specification);
                let picked = self.pick_stem_folder(|folder| {
                    planned
                        .into_iter()
                        .filter(|name| std::fs::symlink_metadata(folder.join(name)).is_ok())
                        .collect()
                });
                if let Some((folder, replace)) = picked {
                    if let Some(stems) = &mut specification.stems {
                        stems.replace_existing = replace;
                    }
                    self.export_to_with_specification(folder, specification);
                }
            } else if let Some(path) = self.pick_document_path(DocumentDialog::Export) {
                self.export_to_with_specification(path, specification);
            }
        }
    }
}

/// The export worker. A child-scoped delay makes cancellation E2Es
/// deterministic; ordinary launches never consult the test-only setting.
fn run_export(request: &ExportRequest, cancel: &AtomicBool) -> Result<ExportCompletion> {
    if std::env::var_os("RILL_AUTOMATION_DIR").is_some() {
        let delay = std::env::var("RILL_E2E_EXPORT_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            .min(5_000);
        let deadline = Instant::now() + Duration::from_millis(delay);
        while Instant::now() < deadline && !cancel.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let specification = &request.specification;
    let label = specification.options.format.label();
    let completion = if let Some(stems) = &specification.stems {
        daw_engine::export_stems(
            &request.project,
            &daw_engine::StemRequest {
                directory: request.path.clone(),
                song: stems.song.clone(),
                channels: stems.channels.clone(),
                include_mix: stems.include_mix,
                through_master: stems.through_master,
                replace_existing: stems.replace_existing,
            },
            specification.sample_rate,
            specification.options,
            cancel,
        )
        .map(|files| {
            let mut report = files[0].report;
            for file in &files[1..] {
                report.sample_peak = report.sample_peak.max(file.report.sample_peak);
                report.clipped_samples += file.report.clipped_samples;
                report.unsupported_midi_events = report
                    .unsupported_midi_events
                    .max(file.report.unsupported_midi_events);
                report.unsupported_midi_groups = report
                    .unsupported_midi_groups
                    .max(file.report.unsupported_midi_groups);
            }
            ExportCompletion {
                path: request.path.clone(),
                report,
                stems: files.len(),
                files: files.into_iter().map(|file| file.path).collect(),
            }
        })
    } else {
        daw_engine::export_project_cancellable(
            &request.project,
            &request.path,
            specification.sample_rate,
            specification.options,
            cancel,
        )
        .map(|report| ExportCompletion {
            path: request.path.clone(),
            report,
            stems: 0,
            files: vec![request.path.clone()],
        })
    };
    completion
        .with_context(|| format!("Rendering {}", request.identity))
        .with_context(|| format!("Exporting {label} to {}", request.path.display()))
        .with_context(|| format!("{label} export failed"))
}
