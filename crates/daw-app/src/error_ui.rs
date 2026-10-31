//! Inspectable error reports, retained independently of transient status messages.
use crate::app::DawApp;
use eframe::egui;

#[derive(Clone)]
pub(crate) struct ErrorReport {
    pub summary: String,
    pub details: String,
    pub status: String,
    pub audio_setup: bool,
}

pub(crate) struct ErrorDetails {
    title: String,
    report: ErrorReport,
    backdrop: Option<daw_ui::ViewBackdrop>,
}

impl DawApp {
    pub(crate) fn has_failed_output_notice(&self) -> bool {
        // Only our terminal output diagnostics qualify. Input failures and
        // unrelated reports can contain output-device metadata or old causes.
        self.error
            && self
                .engine
                .as_ref()
                .is_some_and(|engine| engine.snapshot().output_failed)
            && self.last_error_report.as_ref().is_some_and(|report| {
                report.status == self.status
                    && (matches!(
                        report.summary.as_str(),
                        "Audio output stream failed or disconnected"
                            | "Audio output dropout interrupted recording"
                            | "Audio output failed during MIDI recording; captured notes were retained"
                    ) || report.summary.starts_with("Audio output stream failed;"))
            })
    }

    pub(crate) fn can_retry_playback(&self) -> bool {
        self.has_failed_output_notice()
            && self.loading.is_none()
            && self.export_job.is_none()
            && !self.project_io_pending()
            && !self.state_sync_pending()
            && !self.recording_busy()
            && !self.recording_finalizing()
            && self.document_guard.is_none()
    }

    pub(crate) fn retry_failed_playback(&mut self) {
        // A retained notice or a click from an earlier frame must never toggle
        // a healthy transport or retire a stream still finalizing a capture.
        if !self.can_retry_playback() {
            return;
        }
        self.cursor = self.engine.as_ref().unwrap().position_beats();
        self.prepare_engine(true);
    }

    pub(crate) fn make_error_report(&self, summary: String, diagnostic: String) -> ErrorReport {
        // Classify the original cause before appending the standard device
        // metadata, which appears in reports for unrelated errors too.
        let cause = diagnostic.to_lowercase();
        let audio_setup = [
            "audio device",
            "audio output device",
            "audio output stream",
            "audio output dropout",
            "output device '",
            "device does not support the requested sample rate",
            "device buffer size",
        ]
        .iter()
        .any(|phrase| cause.contains(phrase));
        let selected = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .map(|track| format!("\nSelected channel: {} (#{})", track.name, track.id))
            .unwrap_or_default();
        let identity = self.document_identity();
        let document = self
            .project_path
            .as_ref()
            .map(|path| format!("\nProject file: {}", path.display()))
            .unwrap_or_default();
        let session = if self.project.name != identity {
            format!("\nSession title: {}", self.project.name)
        } else {
            String::new()
        };
        let details = format!(
            "{diagnostic}\n\nRILL {}\nPlatform: {} / {}\nProject: {}{document}{session}{selected}\nAudio: {} Hz, {} frames\nOutput device: {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            identity,
            self.settings.sample_rate,
            self.settings.buffer_frames,
            self.settings
                .device_name
                .as_deref()
                .unwrap_or("System default"),
        );
        ErrorReport {
            summary,
            details,
            status: self.status.clone(),
            audio_setup,
        }
    }

    pub(crate) fn remember_error(&mut self, error: &anyhow::Error) {
        let mut causes = error.chain();
        let summary = causes.next().unwrap().to_string();
        let mut diagnostic = summary.clone();
        for (index, cause) in causes.enumerate() {
            if index == 0 {
                diagnostic.push_str("\n\nCaused by:");
            }
            diagnostic.push_str(&format!("\n{}. {cause}", index + 1));
        }
        let summary = summary.lines().next().unwrap_or(&summary).to_owned();
        self.last_error_report = Some(self.make_error_report(summary, diagnostic));
    }

    pub(crate) fn show_error_details(
        &mut self,
        title: impl Into<String>,
        summary: impl Into<String>,
        diagnostic: impl Into<String>,
    ) {
        self.error_details = Some(ErrorDetails {
            title: title.into(),
            report: self.make_error_report(summary.into(), diagnostic.into()),
            backdrop: None,
        });
    }

    pub(crate) fn show_status_error_details(&mut self) {
        let report = self
            .last_error_report
            .as_ref()
            .filter(|report| report.status == self.status)
            .cloned()
            .unwrap_or_else(|| self.make_error_report(self.status.clone(), self.status.clone()));
        self.show_error_report_details(report);
    }

    pub(crate) fn show_error_report_details(&mut self, report: ErrorReport) {
        self.error_details = Some(ErrorDetails {
            title: "Error details".into(),
            report,
            backdrop: None,
        });
    }

    /// On opening, capture the completed caller including its menus and any popup.
    /// Later frames paint only those retained pixels: raw editor shortcuts and drops
    /// must not reach the underlying project while the diagnostics modal is active.
    pub(crate) fn error_details_frame(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.error_details else {
            return;
        };
        if let Some(backdrop) = &dialog.backdrop {
            egui::CentralPanel::default()
                .frame(daw_ui::inset(ctx).inner_margin(0).corner_radius(0))
                .show(ctx, |ui| backdrop.paint(ui));
        } else {
            dialog.backdrop = Some(daw_ui::ViewBackdrop::capture_dialog_context(
                ctx,
                ctx.content_rect(),
            ));
        }
        if daw_ui::error_details(
            ctx,
            &dialog.title,
            &dialog.report.summary,
            &dialog.report.details,
        )
        .closed
        {
            self.error_details = None;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    pub(crate) fn error_details_diagnostics(&self) -> serde_json::Value {
        self.error_details
            .as_ref()
            .map_or(serde_json::Value::Null, |dialog| {
                serde_json::json!({
                    "title": dialog.title,
                    "summary": dialog.report.summary,
                    "details": dialog.report.details,
                    "background_shapes": dialog.backdrop.as_ref().map_or(0, |b| b.shape_count()),
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::app::DawApp;
    use daw_core::Project;
    use daw_engine::{AudioEngine, DeviceSettings};
    use std::sync::{Arc, atomic::AtomicBool, mpsc};

    fn failed_output_app() -> DawApp {
        let ctx = eframe::egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        app.virtual_audio = true;
        let mut engine = AudioEngine::start_virtual_cancellable(
            &app.project,
            DeviceSettings::default(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        engine.seek(2.0);
        engine.fail_virtual_output().unwrap();
        let diagnostic = engine.take_input_error().unwrap();
        app.engine = Some(engine);
        app.message(Err(anyhow::anyhow!(diagnostic)), "");
        assert!(app.can_retry_playback());
        app
    }

    #[test]
    fn retry_revalidates_the_current_notice_and_never_pauses_a_healthy_engine() {
        let mut app = failed_output_app();
        let report = app.last_error_report.clone();
        app.status = "A newer operation failed".into();
        app.retry_failed_playback();
        assert!(app.engine.as_ref().unwrap().snapshot().output_failed);
        assert!(app.loading.is_none());
        app.message(
            Err(anyhow::anyhow!("Audio input stream failed or disconnected")),
            "",
        );
        assert!(!app.has_failed_output_notice());
        app.retry_failed_playback();
        assert!(app.loading.is_none());

        app.last_error_report = report;
        app.status = app.last_error_report.as_ref().unwrap().status.clone();
        let healthy = AudioEngine::start_virtual_cancellable(
            &app.project,
            DeviceSettings::default(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        healthy.set_playing(true);
        let identity = healthy.snapshot().engine_id;
        app.engine = Some(healthy);
        let project = app.project.clone();
        let revision = app.revision;
        app.retry_failed_playback();
        assert!(app.engine.as_ref().unwrap().snapshot().playing);
        assert_eq!(app.engine.as_ref().unwrap().snapshot().engine_id, identity);
        assert_eq!(app.project, project);
        assert_eq!(app.revision, revision);
        assert!(app.loading.is_none());
        assert!(!app.has_failed_output_notice());
    }

    #[test]
    fn retry_waits_for_audio_preparation_export_and_queued_document_work() {
        let mut app = failed_output_app();
        let identity = app.engine.as_ref().unwrap().snapshot().engine_id;
        let diagnostic = app.status.clone();
        let (loading_sender, loading_receiver) = mpsc::channel();
        app.loading = Some(loading_receiver);
        assert!(app.has_failed_output_notice());
        assert!(!app.can_retry_playback());
        app.retry_failed_playback();
        assert_eq!(app.engine.as_ref().unwrap().snapshot().engine_id, identity);
        assert!(app.loading.is_some());
        app.loading = None;
        drop(loading_sender);

        let (export_sender, export_receiver) = mpsc::channel();
        app.export_job = Some(export_receiver);
        assert!(!app.can_retry_playback());
        app.retry_failed_playback();
        assert_eq!(app.engine.as_ref().unwrap().snapshot().engine_id, identity);
        assert!(app.export_job.is_some());

        assert!(app.defer_document_request(crate::project_io::DocumentRequest::New));
        app.export_job = None;
        drop(export_sender);
        app.message(Err(anyhow::anyhow!(diagnostic)), "");
        assert!(app.project_io_pending());
        assert!(app.has_failed_output_notice());
        assert!(!app.can_retry_playback());
        app.retry_failed_playback();
        assert_eq!(app.engine.as_ref().unwrap().snapshot().engine_id, identity);
        assert!(app.loading.is_none());
    }

    #[test]
    fn report_preserves_all_causes_and_freezes_context() {
        let ctx = eframe::egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        app.settings.sample_rate = 48_000;
        let error = anyhow::anyhow!("Worker exited with code 86 · 音楽 🎹")
            .context("Loading /tmp/Organ.vst3")
            .context("Preparing the instrument");
        app.message(Err(error), "");
        app.show_status_error_details();
        let before = app.error_details_diagnostics();
        assert_eq!(before["summary"], "Preparing the instrument");
        let details = before["details"].as_str().unwrap();
        assert!(details.contains("1. Loading /tmp/Organ.vst3"));
        assert!(details.contains("2. Worker exited with code 86 · 音楽 🎹"));
        assert!(details.contains("48000 Hz"));
        app.message(Ok(()), "Operation complete");
        assert_eq!(app.error_details_diagnostics(), before);
    }

    #[test]
    fn raw_audio_drop_behind_error_details_cannot_change_the_project() {
        use eframe::egui::{self, Context, RawInput};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("do-not-import.wav");
        daw_core::AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.1; 480],
        }
        .write_wav(&path)
        .unwrap();
        let ctx = Context::default();
        let project = Project::new();
        let mut app = DawApp::with_context(&ctx, project.clone(), None);
        app.audio_device_enabled = false;
        app.show_error_details("Error details", "Could not export", "Permission denied");
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 650.0),
                )),
                dropped_files: vec![egui::DroppedFile {
                    path: Some(path),
                    ..Default::default()
                }],
                ..Default::default()
            },
            |ctx| app.ui_frame(ctx),
        );
        assert_eq!(app.project, project);
        assert!(!app.modified);
        assert!(app.error_details.is_some());
    }

    #[test]
    fn closing_the_app_from_details_preserves_the_unsaved_project_guard() {
        use eframe::egui::{Context, RawInput, ViewportCommand, ViewportEvent, ViewportId};
        let ctx = Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        app.audio_device_enabled = false;
        app.modified = true;
        app.show_error_details("Error details", "Could not save", "Disk is unavailable");
        let mut input = RawInput::default();
        input
            .viewports
            .get_mut(&ViewportId::ROOT)
            .unwrap()
            .events
            .push(ViewportEvent::Close);
        let output = ctx.run(input, |ctx| app.ui_frame(ctx));
        assert!(app.error_details.is_none());
        assert!(!app.close_ready);
        assert_eq!(app.document_guard_diagnostics()["phase"], "confirm");
        assert!(
            output.viewport_output[&ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, ViewportCommand::CancelClose))
        );
    }
}
