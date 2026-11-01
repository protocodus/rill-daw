//! User document transitions are decisions above the ordered I/O pipeline.
//! Low-level requests remain available to controlled automation and completions.
use crate::app::DawApp;
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentDialog {
    Save,
    Open,
    Import,
    Export,
    ExportMidi,
}

/// A headless picker result still passes through the production destination
/// checks. Missing replacement decisions are cancellations, never approval.
pub(crate) struct DocumentDialogSelection {
    pub kind: DocumentDialog,
    pub path: Option<PathBuf>,
    pub replace_existing: Option<bool>,
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
    ExportFailed,
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
    tool_discard: Option<bool>,
}

impl DawApp {
    /// The document's disk identity stays distinct from its musical project name.
    pub(crate) fn document_identity(&self) -> String {
        self.project_path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.project.name.clone())
    }

    pub(crate) fn document_title(&self) -> String {
        format!(
            "{}{}",
            self.document_identity(),
            if self.modified { " •" } else { "" }
        )
    }

    pub(crate) fn document_dialog_filename(&self, kind: DocumentDialog) -> Option<String> {
        if matches!(kind, DocumentDialog::Open | DocumentDialog::Import) {
            return None;
        }
        if kind == DocumentDialog::Save
            && let Some(name) = self.project_path.as_ref().and_then(|path| path.file_name())
        {
            return Some(name.to_string_lossy().into_owned());
        }
        let stem = self
            .project_path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                let name = self.project.name.replace(['/', '\\'], "-");
                if name.trim().is_empty() {
                    "Untitled".into()
                } else {
                    name
                }
            });
        let extension = match kind {
            DocumentDialog::Save => "rill",
            DocumentDialog::Export => "wav",
            DocumentDialog::ExportMidi => "mid",
            DocumentDialog::Open | DocumentDialog::Import => unreachable!(),
        };
        Some(format!("{stem}.{extension}"))
    }

    pub(crate) fn document_dialog_directory(&self) -> Option<&Path> {
        self.project_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|directory| {
                if directory.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    directory
                }
            })
            .filter(|directory| directory.is_dir())
            .or_else(|| {
                self.settings
                    .last_project_directory
                    .as_deref()
                    .filter(|directory| directory.is_dir())
            })
    }

    pub(crate) fn remember_project_directory(&mut self, path: &Path) {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let Some(directory) = parent.canonicalize().ok().filter(|path| path.is_dir()) else {
            return;
        };
        self.setup_draft.last_project_directory = Some(directory.clone());
        if self.settings.last_project_directory.as_ref() != Some(&directory) {
            self.settings.last_project_directory = Some(directory);
            // A preference-write failure must not turn a completed project
            // save into a failed document operation. The session still keeps it.
            let _ = self.settings.save();
        }
    }

    pub(crate) fn pick_document_path(&mut self, kind: DocumentDialog) -> Option<PathBuf> {
        let injected = self
            .next_document_dialog
            .as_ref()
            .is_some_and(|selection| selection.kind == kind);
        let (chosen, replacement_decision) = if injected {
            let selection = self.next_document_dialog.take().unwrap();
            (
                selection.path?,
                Some(selection.replace_existing.unwrap_or(false)),
            )
        } else {
            let dialog = match kind {
                DocumentDialog::Import => {
                    rfd::FileDialog::new().add_filter("Audio / MIDI", &["wav", "mid", "midi"])
                }
                DocumentDialog::Export => rfd::FileDialog::new().add_filter("Wave audio", &["wav"]),
                DocumentDialog::ExportMidi => {
                    rfd::FileDialog::new().add_filter("MIDI", &["mid", "midi"])
                }
                _ => rfd::FileDialog::new()
                    .add_filter("Rill project", &["rill"])
                    .add_filter("Legacy Rill session", &["json"]),
            };
            let dialog = if let Some(parent) = self.document_dialog_directory() {
                dialog.set_directory(parent)
            } else {
                dialog
            };
            let dialog = if let Some(filename) = self.document_dialog_filename(kind) {
                dialog.set_file_name(filename)
            } else {
                dialog
            };
            let chosen = daw_ui::with_native_dialog_menu(|| match kind {
                DocumentDialog::Open | DocumentDialog::Import => dialog.pick_file_result(),
                _ => dialog.save_file_result(),
            });
            let chosen = match chosen {
                Ok(selection) => selection?,
                Err(error) => {
                    self.message(
                        Err(anyhow::Error::new(error).context("Could not open the file picker")),
                        "",
                    );
                    if let Some(guard) = self.document_guard.as_mut() {
                        guard.error = Some(self.status.clone());
                    }
                    return None;
                }
            };
            (chosen, None)
        };
        if !matches!(kind, DocumentDialog::Open | DocumentDialog::Import)
            && let Err(error) = self
                .project
                .validate_audio_destination(&chosen)
                .and_then(|_| {
                    self.project
                        .validate_audio_destination(&normalize_document_path(chosen.clone(), kind))
                })
        {
            // Selecting active source media remains an error even when a file
            // suffix would redirect the write. Check the final name as well,
            // before offering to replace any existing entry at that path.
            self.message(Err(error), "");
            if let Some(guard) = self.document_guard.as_mut() {
                guard.error = Some(self.status.clone());
            }
            return None;
        }
        normalize_and_confirm_document_path(chosen, kind, |destination| {
            replacement_decision.unwrap_or_else(|| confirm_normalized_replacement(destination))
        })
    }

    pub(crate) fn request_document_transition(&mut self, transition: DocumentTransition) {
        if self.document_guard.is_some() {
            return;
        }
        self.assistant.stop();
        self.tool_joins.cancel();
        if self.recording_busy() {
            self.finish_recording(crate::recording_ui::AfterRecording::Pause);
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
            tool_discard: None,
        });
    }

    /// Application-profile transitions never require a GUI confirmation. Dirty
    /// state discovered in acknowledged native snapshots is checked again.
    pub(crate) fn request_tool_document_transition(
        &mut self,
        transition: DocumentTransition,
        discard_unsaved: bool,
    ) -> Result<(), daw_tools::ToolError> {
        if self.document_guard.is_some()
            || self.recording_busy()
            || self.project_io_pending()
            || self.state_sync_pending()
            || self.edit_dirty
            || self.drag.is_some()
        {
            return Err(daw_tools::ToolError::new(
                "busy",
                "Finish the current edit or document operation first",
            ));
        }
        if self.modified && !discard_unsaved {
            return Err(daw_tools::ToolError::new(
                "unsaved_changes",
                "Save the song or explicitly set discard_unsaved before replacing it",
            ));
        }
        self.request_document_transition(transition);
        if let Some(guard) = &mut self.document_guard {
            guard.tool_discard = Some(discard_unsaved);
        }
        Ok(())
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
                if !self.recording_busy()
                    && !self.project_io_pending()
                    && !self.state_sync_pending()
                    && self.export_job.is_none()
                    && !self.join_ui.pending()
                    && !self.tool_joins.pending() =>
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
        let tool_discard = self
            .document_guard
            .as_ref()
            .and_then(|guard| guard.tool_discard);
        if let Some(discard) = tool_discard {
            if self.modified
                && !discard
                && matches!(stage, Stage::Checking | Stage::PreparingTransition)
            {
                self.document_guard = None;
                self.message(Err(anyhow::anyhow!("Unsaved changes were discovered; save or explicitly discard them before replacing the song")),"");
                return;
            }
            if matches!(stage, Stage::Checking) {
                self.document_guard.as_mut().unwrap().stage = Stage::PreparingTransition;
                self.request_document_snapshot(true);
                return;
            }
            if matches!(stage, Stage::PreparingTransition) {
                self.document_guard.as_mut().unwrap().stage = Stage::Ready;
                return;
            }
        }
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
        if self
            .document_guard
            .as_ref()
            .is_some_and(|guard| guard.tool_discard.is_some())
        {
            self.document_guard = None;
            self.message(Err(anyhow::anyhow!(error)), "");
            return;
        }
        if let Some(guard) = self.document_guard.as_mut() {
            guard.stage = Stage::Confirm;
            guard.error = Some(error);
        }
    }

    /// A document switch must not hide a WAV failure that it was waiting for.
    pub(crate) fn document_export_failed(&mut self, error: String) {
        if self.document_guard.as_ref().is_some_and(|guard| {
            matches!(guard.stage, Stage::Waiting) && guard.tool_discard.is_some()
        }) {
            // Application tools cannot answer a GUI-only Continue/Cancel
            // decision. Abort their transition and retain the already captured
            // export report, including its full cause chain, for the caller.
            self.document_guard = None;
            self.status = error;
            self.error = true;
            return;
        }
        if let Some(guard) = self.document_guard.as_mut()
            && matches!(guard.stage, Stage::Waiting)
        {
            guard.stage = Stage::ExportFailed;
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
                Stage::ExportFailed => "export_failed",
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
                if self.last_scene.shape_count() > 0 {
                    self.last_scene.paint(ui);
                } else {
                    daw_ui::paint_ambient(ui.painter(), ui.max_rect());
                }
            });
        let Some(guard) = self.document_guard.as_ref() else {
            return;
        };
        let confirmation = matches!(guard.stage, Stage::Confirm);
        let export_failed = matches!(guard.stage, Stage::ExportFailed);
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
        let export_progress = self.export_progress_message();
        let identity = self.document_identity();
        let mut save = false;
        let mut discard = false;
        let mut continue_after_export = false;
        let mut cancel = false;
        let response = daw_ui::modal(
            ctx,
            egui::Id::new("unsaved_document"),
            if confirmation {
                "Save changes?"
            } else if export_failed {
                "WAV export failed"
            } else {
                "Please wait"
            },
            |ui| {
                daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Subtitle, &identity, 2);
                ui.add_space(daw_ui::tokens::SPACE_SM);
                if confirmation {
                    daw_ui::text::body(ui, format!("Save your changes before {action}?"));
                } else if export_failed {
                    daw_ui::text::body(
                        ui,
                        "Cancel to return and retry, or continue without this export.",
                    );
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        daw_ui::text::body(ui, export_progress.as_deref().unwrap_or(progress));
                    });
                    if export_progress.is_some() {
                        daw_ui::text::body(ui, "The export will finish before switching sessions.");
                    }
                }
                if let Some(error) = &error {
                    ui.add_space(daw_ui::tokens::SPACE_MD);
                    daw_ui::text::bounded_block(ui, daw_ui::text::TextRole::Body, error, 4);
                    if daw_ui::button(ui, "Details").clicked() {
                        if export_failed {
                            self.show_status_error_details();
                        } else {
                            self.show_error_details(
                                "Project error",
                                error.lines().next().unwrap_or(error),
                                error,
                            );
                        }
                    }
                }
                ui.add_space(daw_ui::tokens::SPACE_XL);
                daw_ui::action_row(
                    ui,
                    |ui| {
                        if confirmation {
                            save = daw_ui::primary_button(ui, "Save").clicked();
                            discard = daw_ui::button(ui, "Don't save").clicked();
                        }
                        if export_failed {
                            continue_after_export = daw_ui::button(ui, "Continue").clicked();
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
        } else if continue_after_export {
            let guard = self.document_guard.as_mut().unwrap();
            guard.error = None;
            guard.stage = Stage::Checking;
            self.request_document_snapshot(false);
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

/// Keep explicit legacy saves, and make extensionless native saves portable.
/// Appending preserves dots in names without producing `Song.RILL.rill`.
fn normalize_document_path(mut path: PathBuf, kind: DocumentDialog) -> PathBuf {
    let (extension, accepted): (&str, &[&str]) = match kind {
        DocumentDialog::Open | DocumentDialog::Import => return path,
        DocumentDialog::Save => ("rill", &["rill", "json"]),
        DocumentDialog::Export => ("wav", &["wav"]),
        DocumentDialog::ExportMidi => ("mid", &["mid", "midi"]),
    };
    if !path.extension().is_some_and(|actual| {
        accepted
            .iter()
            .any(|expected| actual.eq_ignore_ascii_case(expected))
    }) {
        let mut filename = path.file_name().unwrap_or_default().to_os_string();
        filename.push(format!(".{extension}"));
        path.set_file_name(filename);
    }
    path
}

/// A native picker only confirms replacement of the exact name it returned.
/// Appending an extension can target a different existing entry, so protect
/// that destination before starting any save, export, or editor-state barrier.
fn normalize_and_confirm_document_path(
    chosen: PathBuf,
    kind: DocumentDialog,
    confirm_replacement: impl FnOnce(&Path) -> bool,
) -> Option<PathBuf> {
    let destination = normalize_document_path(chosen.clone(), kind);
    let entry_exists = match std::fs::symlink_metadata(&destination) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    };
    if destination != chosen && entry_exists && !confirm_replacement(&destination) {
        return None;
    }
    Some(destination)
}

fn confirm_normalized_replacement(destination: &Path) -> bool {
    let result = daw_ui::with_native_dialog_menu(|| {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Replace existing file?")
            .set_description(format!(
                "The filename will be saved as:\n{}\n\nAn entry already exists at this path. Replace it?",
                destination.display()
            ))
            .set_buttons(rfd::MessageButtons::OkCancelCustom(
                "Replace".into(),
                "Cancel".into(),
            ))
            .show()
    });
    // Older Windows common controls display standard OK/Cancel for custom
    // labels; both APIs still require an explicit affirmative button.
    result == rfd::MessageDialogResult::Ok
        || result == rfd::MessageDialogResult::Custom("Replace".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::Project;

    #[test]
    fn normalized_destination_confirmation_receives_the_actual_changed_path_only() {
        let directory = tempfile::tempdir().unwrap();
        for (kind, extension) in [
            (DocumentDialog::Save, "rill"),
            (DocumentDialog::Export, "wav"),
            (DocumentDialog::ExportMidi, "mid"),
        ] {
            let chosen = directory.path().join(format!("Song.{extension}.other"));
            let normalized = directory
                .path()
                .join(format!("Song.{extension}.other.{extension}"));
            std::fs::write(&normalized, b"preserve me").unwrap();
            let mut asked = false;
            let cancelled = normalize_and_confirm_document_path(chosen.clone(), kind, |path| {
                assert_eq!(path, normalized);
                asked = true;
                false
            });
            assert!(asked);
            assert!(cancelled.is_none());
            assert_eq!(std::fs::read(&normalized).unwrap(), b"preserve me");
            let accepted = normalize_and_confirm_document_path(chosen, kind, |path| {
                assert_eq!(path, normalized);
                true
            });
            assert_eq!(accepted, Some(normalized.clone()));

            // The picker already confirmed an existing exact filename, while
            // an absent normalized filename has nothing to replace.
            assert_eq!(
                normalize_and_confirm_document_path(normalized.clone(), kind, |_| panic!(
                    "second overwrite prompt"
                )),
                Some(normalized)
            );
            let absent = directory.path().join(format!("New {extension}"));
            assert!(
                normalize_and_confirm_document_path(absent, kind, |_| panic!("new file prompt"))
                    .is_some()
            );
        }
        let existing = directory.path().join("Existing without extension");
        std::fs::write(&existing, b"open me").unwrap();
        assert_eq!(
            normalize_and_confirm_document_path(
                existing.clone(),
                DocumentDialog::Open,
                |_| panic!("open replacement prompt")
            ),
            Some(existing)
        );
    }

    #[test]
    fn normalized_directory_entries_require_replacement_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let chosen = directory.path().join("Directory");
        let normalized = chosen.with_extension("rill");
        std::fs::create_dir(&normalized).unwrap();
        let mut asked = false;
        assert!(
            normalize_and_confirm_document_path(chosen, DocumentDialog::Save, |path| {
                assert_eq!(path, normalized);
                asked = true;
                false
            })
            .is_none()
        );
        assert!(asked);
        assert!(normalized.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlinks_are_existing_destinations_for_normalization_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let chosen = directory.path().join("Linked song");
        let normalized = chosen.with_extension("rill");
        let target = directory.path().join("Missing target");
        std::os::unix::fs::symlink(&target, &normalized).unwrap();
        assert!(!normalized.exists());
        let mut asked = false;
        assert!(
            normalize_and_confirm_document_path(chosen, DocumentDialog::Save, |path| {
                assert_eq!(path, normalized);
                asked = true;
                false
            })
            .is_none()
        );
        assert!(asked);
        assert_eq!(std::fs::read_link(&normalized).unwrap(), target);
    }

    #[test]
    fn picker_uses_existing_current_parent_then_remembered_folder_and_accepts_relative_files() {
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("current");
        let remembered = directory.path().join("remembered");
        std::fs::create_dir(&current).unwrap();
        std::fs::create_dir(&remembered).unwrap();
        app.settings.last_project_directory = Some(remembered.clone());
        app.project_path = Some(current.join("song.rill"));
        assert_eq!(app.document_dialog_directory(), Some(current.as_path()));
        std::fs::remove_dir(&current).unwrap();
        assert_eq!(app.document_dialog_directory(), Some(remembered.as_path()));
        app.project_path = Some(PathBuf::from("relative-song.rill"));
        assert_eq!(app.document_dialog_directory(), Some(Path::new(".")));
    }

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
