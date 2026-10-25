//! Inspectable error reports, retained independently of transient status messages.
use crate::app::DawApp;
use eframe::egui;

#[derive(Clone)]
pub(crate) struct ErrorReport {
    pub summary: String,
    pub details: String,
    pub status: String,
}

pub(crate) struct ErrorDetails {
    title: String,
    report: ErrorReport,
    backdrop: Option<daw_ui::ViewBackdrop>,
}

impl DawApp {
    pub(crate) fn make_error_report(&self, summary: String, diagnostic: String) -> ErrorReport {
        let selected = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .map(|track| format!("\nSelected channel: {} (#{})", track.name, track.id))
            .unwrap_or_default();
        let details = format!(
            "{diagnostic}\n\nRILL {}\nPlatform: {} / {}\nProject: {}{selected}\nAudio: {} Hz, {} frames\nOutput device: {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.project.name,
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
