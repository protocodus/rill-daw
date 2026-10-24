//! User document transitions are decisions above the ordered I/O pipeline.
//! Low-level requests remain available to controlled automation and completions.
use crate::app::DawApp;
use eframe::egui;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentDialog {
    Save,
    Open,
    Export,
}

#[derive(Clone, Debug)]
pub(crate) enum DocumentTransition {
    New,
    Open { path: PathBuf, recovery: bool },
    Quit,
}

#[derive(Clone)]
enum Stage {
    Waiting,
    Checking,
    Confirm,
    PreparingSave(PathBuf),
    PreparingTransition,
    Saving,
    Ready,
    Executing,
}

pub(crate) struct DocumentGuard {
    transition: DocumentTransition,
    stage: Stage,
    error: Option<String>,
}

impl DawApp {
    pub(crate) fn pick_document_path(&mut self, kind: DocumentDialog) -> Option<PathBuf> {
        if self
            .next_document_dialog
            .as_ref()
            .is_some_and(|(next, _)| *next == kind)
        {
            return self.next_document_dialog.take().and_then(|(_, path)| path);
        }
        let dialog = if kind == DocumentDialog::Export {
            rfd::FileDialog::new().add_filter("Wave audio", &["wav"])
        } else {
            rfd::FileDialog::new()
                .add_filter("Rill project", &["rill"])
                .add_filter("Legacy Rill session", &["json"])
        };
        let dialog = if let Some(parent) = self.project_path.as_ref().and_then(|path| path.parent())
        {
            dialog.set_directory(parent)
        } else {
            dialog
        };
        daw_ui::with_native_dialog_menu(|| match kind {
            DocumentDialog::Open => dialog.pick_file(),
            DocumentDialog::Export => dialog.set_file_name("mixdown.wav").save_file(),
            DocumentDialog::Save => {
                let filename = self
                    .project_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("{}.rill", self.project.name));
                dialog.set_file_name(filename).save_file()
            }
        })
    }

    pub(crate) fn request_document_transition(&mut self, transition: DocumentTransition) {
        if self.document_guard.is_some() {
            return;
        }
        self.flush_pending_edit();
        self.audition_request = None;
        if let Some(engine) = &self.engine
            && let Err(error) = engine.audition_note(None)
        {
            self.message(Err(error), "");
        }
        self.document_guard = Some(DocumentGuard {
            transition,
            stage: Stage::Waiting,
            error: None,
        });
    }

    pub(crate) fn poll_document_guard(&mut self, ctx: &egui::Context) {
        let Some(stage) = self
            .document_guard
            .as_ref()
            .map(|guard| guard.stage.clone())
        else {
            return;
        };
        match stage {
            Stage::Waiting
                if !self.project_io_pending()
                    && !self.state_sync_pending()
                    && self.export_job.is_none()
                    && !self.join_ui.pending() =>
            {
                self.document_guard.as_mut().unwrap().stage = Stage::Checking;
                self.request_document_snapshot(false);
            }
            Stage::Executing if !self.project_io_pending() && !self.state_sync_pending() => {
                self.document_guard = None
            }
            _ => {}
        }
        if self.document_guard.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }

    /// Native snapshots may reveal changes not yet reported as parameter events.
    /// Only a final acknowledged close can prevent more edits during disk saving.
    pub(crate) fn document_snapshot_ready(&mut self) {
        let Some(stage) = self
            .document_guard
            .as_ref()
            .map(|guard| guard.stage.clone())
        else {
            return;
        };
        self.flush_pending_edit();
        match stage {
            Stage::Checking => {
                if self.modified {
                    self.document_guard.as_mut().unwrap().stage = Stage::Confirm;
                } else {
                    self.document_guard.as_mut().unwrap().stage = Stage::PreparingTransition;
                    self.request_document_snapshot(true);
                }
            }
            Stage::PreparingTransition => {
                self.document_guard.as_mut().unwrap().stage = if self.modified {
                    Stage::Confirm
                } else {
                    Stage::Ready
                };
            }
            Stage::PreparingSave(path) => {
                self.document_guard.as_mut().unwrap().stage = Stage::Saving;
                self.save_to(path);
            }
            _ => {}
        }
    }

    pub(crate) fn document_snapshot_failed(&mut self, error: String) {
        if let Some(guard) = self.document_guard.as_mut() {
            guard.stage = Stage::Confirm;
            guard.error = Some(error);
        }
    }

    pub(crate) fn document_save_finished(&mut self, succeeded: bool) {
        let Some(guard) = self
            .document_guard
            .as_mut()
            .filter(|guard| matches!(guard.stage, Stage::Saving))
        else {
            return;
        };
        if succeeded && !self.modified {
            guard.stage = Stage::Ready;
        } else {
            guard.stage = Stage::Confirm;
            guard.error = Some(if succeeded {
                "New changes arrived while saving. Save again to continue.".into()
            } else {
                self.status.clone()
            });
        }
    }

    fn finish_document_transition(&mut self, ctx: &egui::Context) {
        let Some(guard) = self.document_guard.as_mut() else {
            return;
        };
        let transition = guard.transition.clone();
        guard.stage = Stage::Executing;
        match transition {
            DocumentTransition::New => self.new_session(),
            DocumentTransition::Open { path, recovery } => {
                self.request_open_project(path, recovery)
            }
            DocumentTransition::Quit => self.request_quit(ctx),
        }
    }

    pub(crate) fn document_guard_diagnostics(&self) -> serde_json::Value {
        self.document_guard.as_ref().map_or(serde_json::Value::Null, |guard| serde_json::json!({
            "intent": match &guard.transition {
                DocumentTransition::New => "new",
                DocumentTransition::Open { recovery: true, .. } => "recover",
                DocumentTransition::Open { .. } => "open",
                DocumentTransition::Quit => "quit",
            },
            "phase": match guard.stage {
                Stage::Waiting => "waiting",
                Stage::Checking | Stage::PreparingSave(_) | Stage::PreparingTransition => "checking",
                Stage::Confirm => "confirm",
                Stage::Saving => "saving",
                Stage::Ready => "ready",
                Stage::Executing => "opening",
            },
            "error": guard.error,
        }))
    }

    /// The guard owns this frame, so raw editor shortcuts and dropped files are
    /// not processed under the modal, including the frame that opens it.
    pub(crate) fn document_guard_frame(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(daw_ui::inset(ctx).inner_margin(0).corner_radius(0))
            .show(ctx, |ui| {
                daw_ui::paint_ambient(ui.painter(), ui.max_rect());
            });
        let Some(guard) = self.document_guard.as_ref() else {
            return;
        };
        let confirmation = matches!(guard.stage, Stage::Confirm);
        let cancelable = !matches!(guard.stage, Stage::Executing);
        let progress = match guard.stage {
            Stage::Waiting => "Finishing the current operation…",
            Stage::Saving => "Saving project…",
            Stage::Executing => "Opening project…",
            _ => "Checking plug-in state…",
        };
        let action = match &guard.transition {
            DocumentTransition::New => "starting a new project",
            DocumentTransition::Open { recovery: true, .. } => "recovering another project",
            DocumentTransition::Open { .. } => "opening another project",
            DocumentTransition::Quit => "quitting Rill",
        };
        let error = guard.error.clone();
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        let response = daw_ui::modal(
            ctx,
            egui::Id::new("unsaved_document"),
            if confirmation {
                "Save changes?"
            } else {
                "Please wait"
            },
            |ui| {
                daw_ui::text::bounded_block(
                    ui,
                    daw_ui::text::TextRole::Subtitle,
                    &self.project.name,
                    2,
                );
                ui.add_space(daw_ui::tokens::SPACE_SM);
                if confirmation {
                    daw_ui::text::body(ui, format!("Save your changes before {action}?"));
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        daw_ui::text::body(ui, progress);
                    });
                }
                if let Some(error) = &error {
                    ui.add_space(daw_ui::tokens::SPACE_MD);
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Body, error, 4);
                }
                ui.add_space(daw_ui::tokens::SPACE_XL);
                daw_ui::action_row(
                    ui,
                    |ui| {
                        if confirmation {
                            save = daw_ui::primary_button(ui, "Save").clicked();
                            discard = daw_ui::button(ui, "Don't save").clicked();
                        }
                        if cancelable {
                            cancel = daw_ui::button(ui, "Cancel").clicked();
                        }
                    },
                    |_| {},
                );
            },
        );
        let escape =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancelable && (cancel || (response.is_top_modal && escape)) {
            self.document_guard = None;
        } else if discard {
            self.finish_document_transition(ctx);
        } else if save {
            let path = self
                .project_path
                .clone()
                .or_else(|| self.pick_document_path(DocumentDialog::Save));
            if let Some(path) = path {
                let guard = self.document_guard.as_mut().unwrap();
                guard.error = None;
                guard.stage = Stage::PreparingSave(path);
                self.request_document_snapshot(true);
            }
        } else if self
            .document_guard
            .as_ref()
            .is_some_and(|guard| matches!(guard.stage, Stage::Ready))
            && !ctx.input(|input| input.pointer.any_down())
        {
            // Render and handle the existing Cancel target before committing.
            // A completion on either its press or release frame must allow the
            // in-flight click to finish before replacing the document.
            self.finish_document_transition(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::Project;

    #[test]
    fn final_native_snapshot_can_cancel_a_previously_clean_transition() {
        let ctx = egui::Context::default();
        let mut project = Project::new();
        let track = project.add_audio_track("Keep this project");
        let mut app = DawApp::with_context(&ctx, project, None);
        app.request_document_transition(DocumentTransition::New);
        app.document_guard.as_mut().unwrap().stage = Stage::PreparingTransition;
        // A native close acknowledgement can carry an opaque preset update
        // that was absent from the first state snapshot.
        app.project.track_mut(track).unwrap().name = "Late native change".into();
        app.modified = true;
        app.document_snapshot_ready();
        app.poll_document_guard(&ctx);
        assert_eq!(app.document_guard_diagnostics()["phase"], "confirm");
        assert_eq!(app.project.tracks[0].name, "Late native change");
    }

    #[test]
    fn changes_after_saved_snapshot_require_another_decision() {
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        app.request_document_transition(DocumentTransition::Quit);
        app.document_guard.as_mut().unwrap().stage = Stage::Saving;
        app.modified = true;
        app.document_save_finished(true);
        app.poll_document_guard(&ctx);
        assert_eq!(app.document_guard_diagnostics()["phase"], "confirm");
        assert!(!app.close_ready);
        assert!(
            app.document_guard_diagnostics()["error"]
                .as_str()
                .unwrap()
                .contains("New changes")
        );
    }
    fn draw(app: &mut DawApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 720.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.ui_frame(ctx),
        )
    }

    #[test]
    fn cancel_input_wins_over_a_save_completion_in_the_same_frame() {
        for pointer_phase in [None, Some(true), Some(false)] {
            let ctx = egui::Context::default();
            let mut project = Project::new();
            project.add_audio_track("Keep unsaved changes");
            let mut app = DawApp::with_context(&ctx, project.clone(), None);
            app.audio_device_enabled = false;
            app.modified = true;
            app.request_document_transition(DocumentTransition::New);
            app.document_guard.as_mut().unwrap().stage = Stage::Saving;
            for _ in 0..3 {
                draw(&mut app, &ctx, vec![]);
            }
            let output = draw(&mut app, &ctx, vec![]);
            let cancel = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "Cancel" => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .unwrap();
            let pointer_event = |pressed| {
                vec![
                    egui::Event::PointerMoved(cancel),
                    egui::Event::PointerButton {
                        pos: cancel,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            if pointer_phase == Some(false) {
                draw(&mut app, &ctx, pointer_event(true));
            }
            app.modified = false;
            app.document_save_finished(true);
            assert_eq!(app.document_guard_diagnostics()["phase"], "ready");
            let events = if let Some(pressed) = pointer_phase {
                pointer_event(pressed)
            } else {
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            };
            draw(&mut app, &ctx, events);
            if pointer_phase == Some(true) {
                assert!(app.document_guard.is_some());
                assert_eq!(app.project, project);
                draw(&mut app, &ctx, pointer_event(false));
            }
            assert!(app.document_guard.is_none());
            assert_eq!(app.project, project);
            assert!(!app.close_ready);
        }
    }

    #[test]
    fn modal_drops_never_import_audio_and_request_releases_audition() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("must-not-import.wav");
        daw_core::AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.1; 480],
        }
        .write_wav(&path)
        .unwrap();
        let ctx = egui::Context::default();
        let mut project = Project::new();
        let track = project.add_audio_track("Current project");
        let mut app = DawApp::with_context(&ctx, project.clone(), None);
        app.audio_device_enabled = false;
        app.modified = true;
        app.audition_request = Some((track, 60, 100));
        app.request_document_transition(DocumentTransition::New);
        assert!(app.audition_request.is_none());
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 720.0),
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
        assert_eq!(app.document_guard_diagnostics()["phase"], "confirm");
        assert!(!app.error);
    }
}
