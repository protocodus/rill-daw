//! One ordered document pipeline. Filesystem work never runs on the UI thread;
//! native plugin snapshots are still acknowledged before saving or swapping a project.
use crate::{app::DawApp, settings::Settings};
use anyhow::Result;
use daw_core::{AudioData, AudioFile, Project};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

pub(crate) const WAVEFORM_FILE_BYTES: usize = daw_core::AUDIO_STREAM_THRESHOLD_BYTES as usize - 1;
pub(crate) const WAVEFORM_CACHE_BYTES: usize = 128 * 1024 * 1024;

pub(crate) struct LoadedProject {
    pub project: Project,
    pub waveforms: HashMap<String, AudioData>,
    pub audio_files: HashMap<String, AudioFile>,
    pub waveform_warning: Option<String>,
    pub previous_recovery: bool,
}

impl From<Project> for LoadedProject {
    fn from(project: Project) -> Self {
        let paths: HashSet<_> = project
            .tracks
            .iter()
            .flat_map(daw_core::Track::audio_paths)
            .map(str::to_owned)
            .collect();
        let mut waveforms = HashMap::new();
        let mut audio_files = HashMap::new();
        let mut skipped = 0;
        for (path, result) in
            crate::app::audio_sources::load_paths(paths.into_iter().collect(), WAVEFORM_CACHE_BYTES)
        {
            match result {
                Ok((file, pcm)) => {
                    audio_files.insert(path.clone(), file);
                    if let Some(pcm) = pcm {
                        waveforms.insert(path, pcm);
                    }
                }
                Err(_) => skipped += 1,
            }
        }
        let waveform_warning = (skipped > 0).then(|| {
            format!(
                "{skipped} audio source(s) unavailable · Missing or invalid · Files remain linked"
            )
        });
        Self {
            project,
            waveforms,
            audio_files,
            waveform_warning,
            previous_recovery: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveMode {
    Normal,
    Optimized,
}

pub(crate) enum DocumentRequest {
    Save(PathBuf, SaveMode),
    Open(PathBuf, bool),
    New,
    Quit,
    Export(PathBuf, crate::export_ui::ExportSpecification),
    Settings(Box<Settings>),
}

enum Completion {
    Saved {
        snapshot: Project,
        path: PathBuf,
        history_state: u64,
        mode: SaveMode,
    },
    Loaded {
        loaded: LoadedProject,
        path: PathBuf,
        recovery: bool,
    },
}

struct Job {
    receiver: mpsc::Receiver<Result<Completion>>,
}

#[derive(Default)]
pub(crate) struct ProjectIo {
    job: Option<Job>,
    queue: VecDeque<DocumentRequest>,
    dispatching: bool,
    recovery: RecoveryIo,
}

/// Recovery is independent of document operations and native plug-in barriers.
/// Only a running write and the newest pending snapshot per song are retained:
/// dragging a control cannot create an unbounded backlog. Document requests
/// wait for recovery, while a load completing during edits can cross one song
/// boundary; that outgoing checkpoint must survive the first new-song edit.
#[derive(Default)]
struct RecoveryIo {
    job: Option<mpsc::Receiver<Result<()>>>,
    queued: VecDeque<(u64, Project, PathBuf)>,
    requested: Option<(u64, u64)>,
    quitting: bool,
}

fn write_recovery(snapshot: &Project, directory: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let latest = directory.join("recovery.daw.json");
    // Preserve the preceding complete snapshot before atomically replacing the
    // latest one. A crash/failure during either write leaves a complete copy.
    if latest.exists() && Project::load(&latest).is_ok() {
        let temporary = directory.join(format!(
            ".recovery-previous-{}-{}.tmp",
            std::process::id(),
            daw_core::new_id()
        ));
        let result = (|| -> Result<()> {
            std::fs::copy(&latest, &temporary)?;
            // Windows flushes only through a writable handle; a read-only one
            // fails with "Access is denied".
            std::fs::OpenOptions::new()
                .write(true)
                .open(&temporary)?
                .sync_all()?;
            std::fs::rename(&temporary, directory.join("recovery.previous.daw.json"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result?;
    }
    snapshot.save_legacy_json(latest)
}

impl DawApp {
    /// GUI Save migrates legacy filenames through the portable project picker.
    /// Explicit tool/API paths are honored and still contain portable ZIP bytes.
    pub(crate) fn portable_save_path(&self) -> Option<PathBuf> {
        self.project_path
            .as_ref()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("rill"))
            })
            .cloned()
    }

    pub(crate) fn recovery_pending(&self) -> bool {
        self.project_io.recovery.job.is_some() || !self.project_io.recovery.queued.is_empty()
    }

    pub(crate) fn project_io_pending(&self) -> bool {
        self.project_io.job.is_some()
            || !self.project_io.queue.is_empty()
            || self.project_io.recovery.quitting
    }

    /// Checkpoint each committed song revision, without making UI edits wait on
    /// disk I/O or on a plug-in that may have stopped responding.
    pub(crate) fn checkpoint_committed_work(&mut self) {
        let key = (self.song_session.id, self.song_session.revision);
        if self.modified && self.project_io.recovery.requested != Some(key) {
            self.queue_recovery_snapshot();
        }
    }

    pub(crate) fn queue_recovery_snapshot(&mut self) {
        if !self.modified {
            return;
        }
        self.project_io.recovery.requested =
            Some((self.song_session.id, self.song_session.revision));
        let pending = (
            self.song_session.id,
            self.project.clone(),
            self.settings.scratch_dir.clone(),
        );
        if self
            .project_io
            .recovery
            .queued
            .back()
            .is_some_and(|(id, _, _)| *id == pending.0)
        {
            *self.project_io.recovery.queued.back_mut().unwrap() = pending;
        } else {
            self.project_io.recovery.queued.push_back(pending);
        }
        self.start_recovery_write();
    }

    fn start_recovery_write(&mut self) {
        if self.project_io.recovery.job.is_some() {
            return;
        }
        let Some((_, snapshot, directory)) = self.project_io.recovery.queued.pop_front() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.project_io.recovery.job = Some(receiver);
        let startup = sender.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("rill-recovery".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| write_recovery(&snapshot, &directory))
                    .unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("Recovery worker stopped unexpectedly"))
                    });
                let _ = sender.send(result);
            })
        {
            let _ = startup.send(Err(anyhow::anyhow!(
                "Recovery worker could not start: {error}"
            )));
        }
    }

    pub(crate) fn finish_recovery_before_quit(&mut self) {
        self.project_io.recovery.quitting = true;
    }

    fn poll_recovery(&mut self, ctx: &egui::Context) {
        let result =
            self.project_io
                .recovery
                .job
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err(anyhow::anyhow!("Recovery worker stopped unexpectedly")))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        if let Some(result) = result {
            self.project_io.recovery.job = None;
            if let Err(error) = result {
                // Keep editing and retain the previous successful checkpoint.
                // A failed shutdown checkpoint must never close this document.
                self.project_io.recovery.quitting = false;
                self.project_io.queue.clear();
                self.message(Err(error.context("Recovery could not be saved; your work remains open. Check the recovery folder and save the project")), "");
            }
        }
        self.start_recovery_write();
        if self.project_io.recovery.quitting
            && self.project_io.recovery.job.is_none()
            && self.project_io.recovery.queued.is_empty()
        {
            self.project_io.recovery.quitting = false;
            self.retire_engine();
            self.close_ready = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.project_io.recovery.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// Serialize public document requests, including requests made while an editor
    /// snapshot is outstanding. Internal completions bypass this queue exactly once.
    pub(crate) fn defer_document_request(&mut self, request: DocumentRequest) -> bool {
        if matches!(
            request,
            DocumentRequest::New | DocumentRequest::Open(..) | DocumentRequest::Quit
        ) {
            self.cancel_audio_import();
            self.cancel_freeze();
        }
        if self.recording_busy() {
            self.finish_recording(crate::recording_ui::AfterRecording::Pause);
        }
        if self.project_io.dispatching
            || (!self.recording_busy()
                && !self.project_io_pending()
                && !self.recovery_pending()
                && !self.state_sync_pending()
                && self.export_job.is_none())
                && !self.audio_import_busy()
                && !self.freeze_busy()
        {
            return false;
        }
        if matches!(request, DocumentRequest::Quit)
            && self
                .project_io
                .queue
                .iter()
                .any(|item| matches!(item, DocumentRequest::Quit))
        {
            return true;
        }
        if self.project_io.queue.len() >= 16 {
            self.message(
                Err(anyhow::anyhow!("Wait for pending project operations")),
                "",
            );
            return true;
        }
        self.project_io.queue.push_back(request);
        self.status = "Finishing project operation…".into();
        self.error = false;
        true
    }

    /// Called only after the matching native-editor save acknowledgement.
    pub(crate) fn finish_project_save(&mut self, path: PathBuf, mode: SaveMode) {
        self.flush_pending_edit();
        self.history.seal_group();
        let snapshot = self.project.clone();
        let history_state = self.history.state_id();
        let status = match mode {
            SaveMode::Normal => "Saving project…",
            SaveMode::Optimized => "Saving and optimizing project…",
        };
        self.start_project_job(status, move || {
            match mode {
                SaveMode::Normal => snapshot.save(&path)?,
                SaveMode::Optimized => snapshot.save_optimized(&path)?,
            }
            Ok(Completion::Saved {
                snapshot,
                path,
                history_state,
                mode,
            })
        });
    }

    pub(crate) fn start_project_load(&mut self, path: PathBuf, recovery: bool) {
        let scratch = self.settings.scratch_dir.clone();
        self.start_project_job("Opening project…", move || {
            let (project, previous_recovery) = match Project::load_with_assets(&path, &scratch) {
                Ok(project) => (project, false),
                Err(error) if recovery => {
                    let previous = path.with_file_name("recovery.previous.daw.json");
                    let project = Project::load_with_assets(previous, &scratch).map_err(|_| {
                        error.context("Latest and previous recovery snapshots are unavailable")
                    })?;
                    (project, true)
                }
                Err(error) => return Err(error),
            };
            let mut loaded = LoadedProject::from(project);
            loaded.previous_recovery = previous_recovery;
            Ok(Completion::Loaded {
                loaded,
                path,
                recovery,
            })
        });
    }

    fn start_project_job(
        &mut self,
        status: &str,
        operation: impl FnOnce() -> Result<Completion> + Send + 'static,
    ) {
        debug_assert!(self.project_io.job.is_none());
        let (sender, receiver) = mpsc::channel();
        self.project_io.job = Some(Job { receiver });
        self.status = status.into();
        self.error = false;
        let startup = sender.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("rill-project-io".into())
            .spawn(move || {
                let _ = sender.send(operation());
            })
        {
            let _ = startup.send(Err(anyhow::anyhow!(
                "Project worker could not start: {error}"
            )));
        }
    }

    pub(crate) fn poll_project_io(&mut self, ctx: &egui::Context) {
        self.poll_recovery(ctx);
        // Covers Undo/Redo and native state updates as well as UI/tool commits.
        if !self.edit_dirty && !self.project_io.recovery.quitting && !self.close_ready {
            self.checkpoint_committed_work();
        }
        let result = self
            .project_io
            .job
            .as_ref()
            .map(|job| job.receiver.try_recv());
        match result {
            Some(Ok(result)) => {
                self.project_io.job = None;
                match result {
                    Ok(Completion::Saved {
                        snapshot,
                        path,
                        history_state,
                        mode,
                    }) => {
                        // Audio graph revision excludes names/colors. Compare the
                        // complete saved document so edits made during I/O stay dirty.
                        self.history.mark_saved_at(history_state);
                        self.modified = self.project != snapshot;
                        self.remember_project_directory(&path);
                        self.project_path = Some(path);
                        self.message(
                            Ok(()),
                            match (mode, self.modified) {
                                (SaveMode::Normal, true) => "Saved · New changes remain",
                                (SaveMode::Normal, false) => "Session saved",
                                (SaveMode::Optimized, true) => {
                                    "Saved and optimized · New changes remain"
                                }
                                (SaveMode::Optimized, false) => "Project saved and optimized",
                            },
                        );
                        self.document_save_finished(true);
                    }
                    Ok(Completion::Loaded {
                        loaded,
                        path,
                        recovery,
                    }) => {
                        // The outgoing editor may have changed while files loaded.
                        // Flush and close that exact session before replacing it.
                        self.accept_loaded_project(loaded, path, recovery);
                    }
                    Err(error) => {
                        self.project_io.queue.clear();
                        self.message(Err(error.context("Project operation failed")), "");
                        self.document_save_finished(false);
                    }
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.project_io.job = None;
                self.project_io.queue.clear();
                self.message(
                    Err(anyhow::anyhow!("Project worker stopped unexpectedly")),
                    "",
                );
                self.document_save_finished(false);
            }
            _ => {}
        }
        if !self.recording_busy()
            && !self.audio_import_busy()
            && !self.freeze_busy()
            && self.project_io.job.is_none()
            && !self.recovery_pending()
            && !self.state_sync_pending()
            && self.export_job.is_none()
        {
            if self.error {
                // A failed snapshot/save must never be followed by a queued New,
                // Open or Quit that would discard the document the user was saving.
                self.project_io.queue.clear();
            } else if let Some(request) = self.project_io.queue.pop_front() {
                self.project_io.dispatching = true;
                match request {
                    DocumentRequest::Save(path, mode) => self.save_to_with_mode(path, mode),
                    DocumentRequest::Open(path, recovery) => {
                        self.request_open_project(path, recovery)
                    }
                    DocumentRequest::New => self.new_session(),
                    DocumentRequest::Quit => self.request_quit(ctx),
                    DocumentRequest::Export(path, specification) => {
                        self.export_to_with_specification(path, specification)
                    }
                    DocumentRequest::Settings(settings) => self.apply_settings_snapshot(*settings),
                }
                self.project_io.dispatching = false;
            }
        }
        if self.project_io_pending() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_coalesces_edits_but_preserves_outgoing_song_checkpoint() {
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        let (_sender, receiver) = mpsc::channel();
        app.project_io.recovery.job = Some(receiver);
        app.modified = true;
        app.project.add_audio_track("First edit");
        app.queue_recovery_snapshot();
        app.project.add_audio_track("Last outgoing edit");
        app.queue_recovery_snapshot();
        assert_eq!(app.project_io.recovery.queued.len(), 1);
        app.song_session.reset();
        app.project = Project::new();
        app.project.add_audio_track("New song");
        app.queue_recovery_snapshot();
        assert_eq!(app.project_io.recovery.queued.len(), 2);
        assert_eq!(app.project_io.recovery.queued[0].1.tracks.len(), 2);
        assert_eq!(
            app.project_io.recovery.queued[1].1.tracks[0].name,
            "New song"
        );
    }

    #[test]
    fn recovery_failure_does_not_spin_or_close_the_document() {
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, Project::new(), None);
        app.modified = true;
        let (sender, receiver) = mpsc::channel();
        app.project_io.recovery.requested = Some((app.song_session.id, app.song_session.revision));
        app.project_io.recovery.job = Some(receiver);
        app.project_io.recovery.quitting = true;
        sender.send(Err(anyhow::anyhow!("Disk full"))).unwrap();
        app.poll_project_io(&ctx);
        assert!(!app.recovery_pending());
        assert!(!app.project_io_pending());
        assert!(!app.close_ready);
        for _ in 0..10 {
            app.poll_project_io(&ctx);
            assert!(
                !app.recovery_pending(),
                "Retry requires a new edit or periodic refresh"
            );
        }
        assert!(app.error);
    }

    #[test]
    fn failed_worker_cancels_queued_document_replacement_and_quit() {
        let ctx = egui::Context::default();
        let mut project = Project::new();
        project.add_audio_track("Keep unsaved work");
        let mut app = DawApp::with_context(&ctx, project.clone(), None);
        let (sender, receiver) = mpsc::channel();
        app.project_io.job = Some(Job { receiver });
        assert!(app.defer_document_request(DocumentRequest::New));
        assert!(app.defer_document_request(DocumentRequest::Quit));
        sender
            .send(Err(anyhow::anyhow!("Disk write failed")))
            .unwrap();
        app.poll_project_io(&ctx);
        assert!(!app.project_io_pending());
        assert_eq!(app.project, project);
        assert!(!app.close_ready);
        assert!(app.error);
    }

    #[test]
    fn completion_after_cosmetic_edit_marks_only_the_saved_history_checkpoint() {
        let ctx = egui::Context::default();
        let mut project = Project::new();
        project.add_audio_track("Original name");
        let mut app = DawApp::with_context(&ctx, project.clone(), None);
        let saved_state = app.history.state_id();
        app.begin_edit();
        app.project.tracks[0].name = "Renamed while saving".into();
        app.changed();
        app.flush_pending_edit();
        let (sender, receiver) = mpsc::channel();
        app.project_io.job = Some(Job { receiver });
        sender
            .send(Ok(Completion::Saved {
                snapshot: project,
                path: PathBuf::from("saved.rill"),
                history_state: saved_state,
                mode: SaveMode::Optimized,
            }))
            .unwrap();
        app.poll_project_io(&ctx);
        assert!(app.modified);
        assert!(!app.history.is_saved());
        assert_eq!(app.project_path, Some(PathBuf::from("saved.rill")));
        assert_eq!(app.project.tracks[0].name, "Renamed while saving");
    }
}
