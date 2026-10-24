use crate::app::DawApp;
use daw_core::{AudioData, Clip, TrackKind};
use eframe::egui;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};

#[derive(Default)]
pub(crate) struct JoinUi {
    selected: HashSet<u64>,
    job: Option<JoinJob>,
}

impl JoinUi {
    pub(crate) fn pending(&self) -> bool {
        self.job.is_some()
    }

    /// Request cancellation without detaching the worker. Its slot remains busy
    /// until `poll_join` observes termination and disposes of its result.
    pub(crate) fn cancel(&self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Release);
        }
    }

    fn canceling(&self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|job| job.cancel.load(Ordering::Acquire))
    }
}
struct JoinJob {
    receiver: mpsc::Receiver<anyhow::Result<JoinedSource>>,
    worker: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    revision: u64,
    track: u64,
    ids: Vec<u64>,
    clips: Vec<Clip>,
    tempo: f64,
}
impl Drop for JoinJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
// Also cleans queued results when their receiver is dropped at app shutdown.
struct JoinedSource {
    path: PathBuf,
    committed: bool,
    waveform: Option<AudioData>,
}
impl Drop for JoinedSource {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
}

impl DawApp {
    pub(crate) fn prepare_join(&mut self) {
        self.join_ui.cancel();
        self.join_ui.selected.clear();
        let Some(track) = self.selected_track.and_then(|id| self.project.track(id)) else {
            return;
        };
        let Some(current) = self
            .selected_clip
            .and_then(|id| track.clips.iter().find(|clip| clip.id == id))
        else {
            return;
        };
        self.join_ui.selected.insert(current.id);
        if let Some(neighbor) = track
            .clips
            .iter()
            .filter(|clip| clip.id != current.id)
            .min_by(|a, b| {
                (a.start - current.start)
                    .abs()
                    .total_cmp(&(b.start - current.start).abs())
            })
        {
            self.join_ui.selected.insert(neighbor.id);
        }
        if track.kind == TrackKind::Audio {
            loop {
                let before = self.join_ui.selected.len();
                let start = track
                    .clips
                    .iter()
                    .filter(|clip| self.join_ui.selected.contains(&clip.id))
                    .map(|clip| clip.start)
                    .fold(f64::INFINITY, f64::min);
                let end = track
                    .clips
                    .iter()
                    .filter(|clip| self.join_ui.selected.contains(&clip.id))
                    .map(Clip::end)
                    .fold(0.0, f64::max);
                for clip in &track.clips {
                    if clip.start < end && clip.end() > start {
                        self.join_ui.selected.insert(clip.id);
                    }
                }
                if before == self.join_ui.selected.len() {
                    break;
                }
            }
        }
    }

    pub(crate) fn join_popup(&mut self, ctx: &egui::Context) {
        if !self.join_open {
            return;
        }
        let mut open = true;
        let mut submit = None;
        let mut cancel = false;
        daw_ui::popup("Join clips", ctx)
            .open(&mut open)
            .default_width(440.0)
            .default_height(0.0)
            .vscroll(false)
            .show(ctx, |ui| {
                ui.set_min_width(400.0);
                let Some(track) = self.selected_track.and_then(|id| self.project.track(id)) else {
                    daw_ui::text::note(ui, "Select an audio or MIDI track.");
                    return;
                };
                let busy = self.join_ui.pending();
                daw_ui::text::subtitle(ui, &track.name);
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            let mut clips: Vec<_> = track.clips.iter().collect();
                            clips.sort_by(|a, b| a.start.total_cmp(&b.start));
                            for clip in clips {
                                ui.push_id(clip.id, |ui| {
                                    let mut selected = self.join_ui.selected.contains(&clip.id);
                                    if daw_ui::checkbox(ui, &mut selected, &clip.name)
                                        .on_hover_text(format!(
                                            "Beat {:.2} → {:.2}",
                                            clip.start + 1.0,
                                            clip.end() + 1.0
                                        ))
                                        .changed()
                                    {
                                        if selected {
                                            self.join_ui.selected.insert(clip.id);
                                        } else {
                                            self.join_ui.selected.remove(&clip.id);
                                        }
                                    }
                                });
                            }
                        });
                });
                let ids: Vec<_> = track
                    .clips
                    .iter()
                    .filter(|clip| self.join_ui.selected.contains(&clip.id))
                    .map(|clip| clip.id)
                    .collect();
                let error = if ids.len() < 2 {
                    Some("Select at least two clips".into())
                } else if track.kind == TrackKind::Audio {
                    self.project
                        .audio_join_range(track.id, &ids)
                        .err()
                        .map(|error| error.to_string())
                } else if track.kind != TrackKind::Midi {
                    Some("Select an audio or MIDI track".into())
                } else {
                    None
                };
                if let Some(error) = &error {
                    daw_ui::text::note(ui, error);
                } else {
                    daw_ui::text::note(ui, format!("{} clips → one editable clip", ids.len()));
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy && error.is_none(), daw_ui::button_widget("Join"))
                        .clicked()
                    {
                        submit = Some((track.id, ids, track.kind));
                    }
                    cancel = daw_ui::button(ui, "Cancel").clicked();
                });
                // Reserve the activity row so asynchronous completion cannot
                // move Join/Cancel between pointer-down and pointer-up.
                ui.horizontal(|ui| {
                    ui.set_min_height(daw_ui::tokens::COMPACT_HEIGHT);
                    if busy {
                        ui.spinner();
                        daw_ui::text::note(
                            ui,
                            if self.join_ui.canceling() {
                                "Canceling…"
                            } else {
                                "Rendering audio…"
                            },
                        );
                    }
                });
            });
        self.join_open &= open && !cancel;
        if !self.join_open {
            self.join_ui.cancel();
        }
        if let Some((track, ids, kind)) =
            submit.filter(|_| !self.join_ui.pending() && self.join_open)
        {
            self.flush_pending_edit();
            if kind == TrackKind::Midi {
                self.begin_edit();
                match self.project.join_midi_clips(track, &ids) {
                    Ok(id) => self.finish_join(track, id),
                    Err(error) => self.message(Err(error), ""),
                }
            } else {
                let project = self.project.clone();
                let clips = project.track(track).unwrap().clips.clone();
                let tempo = project.tempo;
                let scratch = self.settings.scratch_dir.clone();
                let rate = self.settings.sample_rate;
                let options = daw_engine::RenderOptions {
                    resample_quality: self.settings.resample_quality,
                };
                let (sender, receiver) = mpsc::channel();
                let render_ids = ids.clone();
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = Arc::clone(&cancel);
                let used = self
                    .waveforms
                    .values()
                    .map(|audio| audio.samples.len() * size_of::<f32>())
                    .sum::<usize>();
                let preview_limit = crate::project_io::WAVEFORM_FILE_BYTES
                    .min(crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(used));
                let worker =
                    std::thread::Builder::new()
                        .name("rill-join".into())
                        .spawn(move || {
                            let result = (|| {
                                anyhow::ensure!(
                                    !worker_cancel.load(Ordering::Acquire),
                                    "Audio join canceled"
                                );
                                let stamp = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)?
                                    .as_nanos();
                                let directory = scratch.join("joins").join(format!(
                                    "{}-{stamp}-{}",
                                    std::process::id(),
                                    daw_core::new_id()
                                ));
                                std::fs::create_dir_all(directory.parent().unwrap())?;
                                std::fs::create_dir(&directory)?;
                                let mut source = JoinedSource {
                                    path: directory.join("joined.wav"),
                                    committed: false,
                                    waveform: None,
                                };
                                daw_engine::export_audio_join_cancelled(
                                    &project,
                                    track,
                                    &render_ids,
                                    &source.path,
                                    rate,
                                    options,
                                    &worker_cancel,
                                )?;
                                anyhow::ensure!(
                                    !worker_cancel.load(Ordering::Acquire),
                                    "Audio join canceled"
                                );
                                source.waveform =
                                    AudioData::load_wav_with_limit(&source.path, preview_limit)
                                        .ok();
                                anyhow::ensure!(
                                    !worker_cancel.load(Ordering::Acquire),
                                    "Audio join canceled"
                                );
                                Ok(source)
                            })();
                            let _ = sender.send(result);
                        });
                match worker {
                    Ok(worker) => {
                        self.join_ui.job = Some(JoinJob {
                            receiver,
                            worker: Some(worker),
                            cancel,
                            revision: self.revision,
                            track,
                            ids,
                            clips,
                            tempo,
                        })
                    }
                    Err(error) => self.message(Err(error.into()), ""),
                }
            }
        }
    }

    pub(crate) fn poll_join(&mut self, ctx: &egui::Context) {
        if !self.join_open {
            self.join_ui.cancel();
        }
        let Some(job) = &self.join_ui.job else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
        if job
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return;
        }
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("Audio join worker stopped"))
            }
        };
        let mut job = self.join_ui.job.take().unwrap();
        if let Some(worker) = job.worker.take() {
            // is_finished above guarantees this never blocks the UI.
            let _ = worker.join();
        }
        if job.cancel.load(Ordering::Acquire) {
            drop(result);
            return;
        }
        let mut preview_limited = false;
        let result = result.and_then(|mut source| {
            anyhow::ensure!(
                job.revision == self.revision
                    && job.tempo == self.project.tempo
                    && self
                        .project
                        .track(job.track)
                        .is_some_and(|track| track.clips == job.clips),
                "Project changed; join the clips again"
            );
            self.begin_edit();
            let id = self.project.replace_audio_clips_with_joined(
                job.track,
                &job.ids,
                source.path.to_string_lossy().into_owned(),
            )?;
            source.committed = true;
            self.finish_join(job.track, id);
            let used = self
                .waveforms
                .values()
                .map(|audio| audio.samples.len() * size_of::<f32>())
                .sum::<usize>();
            if let Some(waveform) = source.waveform.take().filter(|audio| {
                audio.samples.len() * size_of::<f32>()
                    <= crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(used)
            }) {
                self.waveforms
                    .insert(source.path.to_string_lossy().into_owned(), waveform);
            } else {
                preview_limited = true;
            }
            Ok(())
        });
        if result.is_ok() && preview_limited {
            self.waveform_warning =
                Some("Joined audio waveform preview unavailable · Audio remains linked".into());
        }
        self.message(
            result,
            if preview_limited {
                "Clips joined; waveform preview limited"
            } else {
                "Clips joined"
            },
        );
    }

    fn finish_join(&mut self, track: u64, clip: u64) {
        self.close_popups();
        self.selected_track = Some(track);
        self.selected_clip = Some(clip);
        self.last_editor_clip = None;
        self.edit_label = Some("Join clips".into());
        self.changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::{AudioData, ClipContent, Project};
    use std::path::Path;

    fn fixture(ctx: &egui::Context, directory: &Path) -> DawApp {
        let source = directory.join("original.wav");
        AudioData {
            sample_rate: 8_000,
            channels: 1,
            samples: vec![0.2; 8_000],
        }
        .write_wav(&source)
        .unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Audio");
        project.track_mut(track).unwrap().clips = vec![
            Clip::audio("First", 0.0, 1.0, source.to_string_lossy()),
            Clip::audio("Second", 1.0, 1.0, source.to_string_lossy()),
        ];
        let mut app = DawApp::with_context(ctx, project, None);
        app.audio_device_enabled = false;
        app.settings.scratch_dir = directory.to_path_buf();
        app.join_open = true;
        app
    }

    fn queue(app: &mut DawApp) -> mpsc::Sender<anyhow::Result<JoinedSource>> {
        let (sender, receiver) = mpsc::channel();
        let track = &app.project.tracks[0];
        app.join_ui.job = Some(JoinJob {
            receiver,
            worker: None,
            cancel: Arc::new(AtomicBool::new(false)),
            revision: app.revision,
            track: track.id,
            ids: track.clips.iter().map(|clip| clip.id).collect(),
            clips: track.clips.clone(),
            tempo: app.project.tempo,
        });
        sender
    }

    fn rendered_source(directory: &Path, name: &str) -> JoinedSource {
        let directory = directory.join(name);
        std::fs::create_dir(&directory).unwrap();
        let source = JoinedSource {
            path: directory.join("joined.wav"),
            committed: false,
            waveform: Some(AudioData {
                sample_rate: 8_000,
                channels: 1,
                samples: vec![0.3; 8_000],
            }),
        };
        AudioData {
            sample_rate: 8_000,
            channels: 1,
            samples: vec![0.3; 8_000],
        }
        .write_wav(&source.path)
        .unwrap();
        source
    }

    #[test]
    fn cancelled_join_cleans_queued_and_late_results_without_changing_project() {
        for queued_before_cancel in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let ctx = egui::Context::default();
            let mut app = fixture(&ctx, directory.path());
            let original = app.project.clone();
            let sender = queue(&mut app);
            let source = rendered_source(directory.path(), "cancelled");
            let path = source.path.clone();
            if queued_before_cancel {
                assert!(sender.send(Ok(source)).is_ok());
                app.join_open = false;
                app.poll_join(&ctx);
            } else {
                app.join_open = false;
                app.poll_join(&ctx);
                assert!(app.join_ui.pending());
                assert!(app.join_ui.canceling());
                // The tracked worker can still return its owned source. A
                // cancelled completion is discarded even if rendering won the
                // race and produced an otherwise successful result.
                assert!(sender.send(Ok(source)).is_ok());
                app.poll_join(&ctx);
            }
            assert!(!path.exists());
            assert!(!path.parent().unwrap().exists());
            assert!(!app.join_ui.pending());
            assert_eq!(app.project, original);
            app.flush_pending_edit();
            assert!(!app.history.can_undo());
            assert!(!app.modified);
        }
    }

    #[test]
    fn cancelled_join_retains_single_worker_slot_until_the_thread_has_exited() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, directory.path());
        let original = app.project.clone();
        let sender = queue(&mut app);
        let cancel = Arc::clone(&app.join_ui.job.as_ref().unwrap().cancel);
        let source = rendered_source(directory.path(), "retiring");
        let path = source.path.clone();
        let (sent, received) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        app.join_ui.job.as_mut().unwrap().worker = Some(std::thread::spawn(move || {
            assert!(sender.send(Ok(source)).is_ok());
            sent.send(()).unwrap();
            wait.recv().unwrap();
        }));
        received
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        app.join_open = false;
        app.poll_join(&ctx);
        assert!(cancel.load(Ordering::Acquire));
        assert!(app.join_ui.pending());
        assert!(path.exists(), "Result is retained until its worker exits");
        for _ in 0..4 {
            app.prepare_join();
            app.join_open = true;
            app.poll_join(&ctx);
            assert!(app.join_ui.pending() && app.join_ui.canceling());
            assert!(Arc::ptr_eq(
                &app.join_ui.job.as_ref().unwrap().cancel,
                &cancel
            ));
        }
        assert_eq!(app.project, original);
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while app.join_ui.pending() {
            app.poll_join(&ctx);
            assert!(
                std::time::Instant::now() < deadline,
                "Worker did not retire"
            );
            std::thread::yield_now();
        }
        assert!(!path.exists());
        assert_eq!(app.project, original);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn dropping_join_ui_cancels_worker_token_and_disposes_late_result() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, directory.path());
        let sender = queue(&mut app);
        let cancel = Arc::clone(&app.join_ui.job.as_ref().unwrap().cancel);
        let source = rendered_source(directory.path(), "shutdown");
        let path = source.path.clone();
        drop(app);
        assert!(cancel.load(Ordering::Acquire));
        assert!(sender.send(Ok(source)).is_err());
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn stale_join_results_and_worker_disconnect_never_replace_current_clips() {
        for change in ["revision", "tempo", "clips", "disconnect"] {
            let directory = tempfile::tempdir().unwrap();
            let ctx = egui::Context::default();
            let mut app = fixture(&ctx, directory.path());
            let sender = queue(&mut app);
            let source = rendered_source(directory.path(), "stale");
            let path = source.path.clone();
            match change {
                "revision" => app.revision = app.revision.wrapping_add(1),
                "tempo" => app.project.tempo = 130.0,
                "clips" => app.project.tracks[0].clips[0].start = 0.25,
                "disconnect" => {}
                _ => unreachable!(),
            }
            let current = app.project.clone();
            if change == "disconnect" {
                drop(sender);
                drop(source);
            } else {
                assert!(sender.send(Ok(source)).is_ok());
            }
            app.poll_join(&ctx);
            assert!(app.error);
            assert!(app.status.contains(if change == "disconnect" {
                "worker stopped"
            } else {
                "Project changed"
            }));
            assert_eq!(app.project, current);
            assert!(!path.exists());
            assert!(!path.parent().unwrap().exists());
            assert!(!app.join_ui.pending());
            app.flush_pending_edit();
            assert!(!app.history.can_undo());
            assert!(!app.modified);
        }
    }

    #[test]
    fn successful_join_commits_one_undo_and_retains_wav_through_undo_and_redo() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = fixture(&ctx, directory.path());
        let original = app.project.clone();
        let sender = queue(&mut app);
        let source = rendered_source(directory.path(), "retained");
        let path = source.path.clone();
        assert!(sender.send(Ok(source)).is_ok());
        app.poll_join(&ctx);
        assert!(!app.error, "{}", app.status);
        assert!(!app.join_open && !app.join_ui.pending());
        app.flush_pending_edit();
        assert_eq!(
            app.history.undo_labels().collect::<Vec<_>>(),
            ["Join clips"]
        );
        assert!(app.modified);
        assert_eq!(app.project.tracks[0].clips.len(), 1);
        let joined = &app.project.tracks[0].clips[0];
        assert_eq!((joined.start, joined.length), (0.0, 2.0));
        assert_eq!(app.selected_clip, Some(joined.id));
        assert!(
            matches!(&joined.content, ClipContent::Audio { path: actual, .. } if Path::new(actual) == path)
        );
        assert!(path.is_file());
        assert_eq!(AudioData::load_wav(&path).unwrap().frames(), 8_000);
        assert_eq!(
            app.waveforms[&path.to_string_lossy().into_owned()].frames(),
            8_000
        );
        app.project.validate().unwrap();
        let joined_project = app.project.clone();
        assert!(app.history.undo(&mut app.project));
        assert_eq!(app.project, original);
        assert!(path.is_file(), "Redo still needs the rendered source");
        assert!(app.history.redo(&mut app.project));
        assert_eq!(app.project, joined_project);
        drop(app);
        assert!(
            path.is_file(),
            "Committed audio outlives the background job"
        );
    }
}
