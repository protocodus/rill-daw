//! One ordered document pipeline. Filesystem work never runs on the UI thread;
//! native plugin snapshots are still acknowledged before saving or swapping a project.
use crate::{app::DawApp, settings::Settings};
use anyhow::Result;
use daw_core::{AudioData, ClipContent, Project};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

pub(crate) const WAVEFORM_FILE_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const WAVEFORM_CACHE_BYTES: usize = 512 * 1024 * 1024;

pub(crate) struct LoadedProject {
    pub project: Project,
    pub waveforms: HashMap<String, AudioData>,
    pub waveform_warning: Option<String>,
}

impl From<Project> for LoadedProject {
    fn from(project: Project) -> Self {
        let mut waveforms = HashMap::new();
        let mut visited = HashSet::new();
        let mut decoded_bytes = 0;
        let mut skipped = 0;
        for track in &project.tracks {
            for clip in &track.clips {
                if let ClipContent::Audio { path, .. } = &clip.content
                    && visited.insert(path.clone())
                {
                    match AudioData::load_wav_with_limit(
                        path,
                        WAVEFORM_FILE_BYTES.min(WAVEFORM_CACHE_BYTES - decoded_bytes),
                    ) {
                        Ok(audio) => {
                            decoded_bytes += audio.samples.len() * std::mem::size_of::<f32>();
                            waveforms.insert(path.clone(), audio);
                        }
                        Err(_) => skipped += 1,
                    }
                }
            }
        }
        let waveform_warning = (skipped > 0).then(|| format!("{skipped} waveform preview(s) unavailable · Missing, invalid, or too large · Audio files remain linked"));
        Self {
            project,
            waveforms,
            waveform_warning,
        }
    }
}

pub(crate) enum DocumentRequest {
    Save(PathBuf),
    Open(PathBuf, bool),
    New,
    Quit,
    Export(PathBuf),
    Settings(Settings),
}

enum Completion {
    Saved {
        snapshot: Project,
        path: PathBuf,
        history_state: u64,
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
}

impl DawApp {
    pub(crate) fn project_io_pending(&self) -> bool {
        self.project_io.job.is_some() || !self.project_io.queue.is_empty()
    }

    /// Serialize public document requests, including requests made while an editor
    /// snapshot is outstanding. Internal completions bypass this queue exactly once.
    pub(crate) fn defer_document_request(&mut self, request: DocumentRequest) -> bool {
        if self.project_io.dispatching
            || (!self.project_io_pending()
                && !self.state_sync_pending()
                && self.export_job.is_none())
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
    pub(crate) fn finish_project_save(&mut self, path: PathBuf) {
        self.flush_pending_edit();
        self.history.seal_group();
        let snapshot = self.project.clone();
        let history_state = self.history.state_id();
        self.start_project_job("Saving project…", move || {
            snapshot.save(&path)?;
            Ok(Completion::Saved {
                snapshot,
                path,
                history_state,
            })
        });
    }

    pub(crate) fn start_project_load(&mut self, path: PathBuf, recovery: bool) {
        let scratch = self.settings.scratch_dir.clone();
        self.start_project_job("Opening project…", move || {
            let project = Project::load_with_assets(&path, scratch)?;
            Ok(Completion::Loaded {
                loaded: project.into(),
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
        std::thread::spawn(move || {
            let _ = sender.send(operation());
        });
    }

    pub(crate) fn poll_project_io(&mut self, ctx: &egui::Context) {
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
                    }) => {
                        // Audio graph revision excludes names/colors. Compare the
                        // complete saved document so edits made during I/O stay dirty.
                        self.history.mark_saved_at(history_state);
                        self.modified = self.project != snapshot;
                        self.project_path = Some(path);
                        self.message(
                            Ok(()),
                            if self.modified {
                                "Saved · New changes remain"
                            } else {
                                "Session saved"
                            },
                        );
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
            }
            _ => {}
        }
        if self.project_io.job.is_none() && !self.state_sync_pending() && self.export_job.is_none()
        {
            if self.error {
                // A failed snapshot/save must never be followed by a queued New,
                // Open or Quit that would discard the document the user was saving.
                self.project_io.queue.clear();
            } else if let Some(request) = self.project_io.queue.pop_front() {
                self.project_io.dispatching = true;
                match request {
                    DocumentRequest::Save(path) => self.save_to(path),
                    DocumentRequest::Open(path, recovery) => {
                        self.request_open_project(path, recovery)
                    }
                    DocumentRequest::New => self.new_session(),
                    DocumentRequest::Quit => self.request_quit(ctx),
                    DocumentRequest::Export(path) => self.export_to(path),
                    DocumentRequest::Settings(settings) => self.apply_settings_snapshot(settings),
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
            }))
            .unwrap();
        app.poll_project_io(&ctx);
        assert!(app.modified);
        assert!(!app.history.is_saved());
        assert_eq!(app.project_path, Some(PathBuf::from("saved.rill")));
        assert_eq!(app.project.tracks[0].name, "Renamed while saving");
    }
}
