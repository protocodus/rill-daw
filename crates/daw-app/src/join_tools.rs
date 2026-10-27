//! Song-scoped consolidation. File destinations belong to the host; callers
//! supply only existing clip IDs. Uncommitted rendered sources are RAII-owned.
use crate::app::DawApp;
use daw_core::{AudioData, Project, SongSession, TrackKind};
use daw_tools::{ToolError, parse_arguments};
use eframe::egui;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Default)]
pub(crate) struct JoinTools {
    job: Option<JoinJob>,
}

impl JoinTools {
    pub(crate) fn pending(&self) -> bool {
        self.job.is_some()
    }
    pub(crate) fn cancel(&self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Release);
        }
    }
}

impl Drop for JoinTools {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct JoinJob {
    receiver: Receiver<Result<JoinedSource, String>>,
    worker: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    owner_cancel: Arc<AtomicBool>,
    reply: Option<Sender<Result<Value, ToolError>>>,
    session: SongSession,
    candidate: Option<Project>,
    track: u64,
    ids: Vec<u64>,
    name: Option<String>,
}

impl Drop for JoinJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(ToolError::new("cancelled", "Join cancelled")));
        }
    }
}

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    session_id: u64,
    expected_revision: u64,
    track_id: u64,
    clip_ids: Vec<u64>,
    name: Option<String>,
}

fn failure(error: impl std::fmt::Display) -> ToolError {
    ToolError::new("edit_failed", error.to_string())
}

impl DawApp {
    pub(crate) fn start_join_tool(
        &mut self,
        arguments: Value,
        reply: Sender<Result<Value, ToolError>>,
        owner_cancel: Arc<AtomicBool>,
    ) {
        let result = (|| -> Result<Option<JoinJob>, ToolError> {
            if owner_cancel.load(Ordering::Acquire) {
                return Err(ToolError::new("cancelled", "Join cancelled"));
            }
            self.tool_ready()?;
            if self.tool_joins.pending() || self.join_ui.pending() {
                return Err(ToolError::new("busy", "Another clip join is still running"));
            }
            let args: Args = parse_arguments(arguments)?;
            self.check_tool_revision(args.session_id, args.expected_revision)?;
            if !(2..=128).contains(&args.clip_ids.len()) {
                return Err(ToolError::new(
                    "invalid_arguments",
                    "Join between 2 and 128 distinct clips on one track",
                ));
            }
            if args
                .name
                .as_ref()
                .is_some_and(|name| name.chars().count() > 1024 || name.contains('\0'))
            {
                return Err(ToolError::new(
                    "invalid_arguments",
                    "Clip name must contain at most 1024 characters and no NUL",
                ));
            }
            let kind = self
                .project
                .track(args.track_id)
                .ok_or_else(|| ToolError::new("not_found", "Track not found"))?
                .kind;
            let mut candidate = self.project.clone();
            if kind == TrackKind::Midi {
                let id = candidate
                    .join_midi_clips(args.track_id, &args.clip_ids)
                    .map_err(failure)?;
                if let Some(name) = args.name {
                    candidate.clip_mut(args.track_id, id).unwrap().name = name;
                }
                if owner_cancel.load(Ordering::Acquire) {
                    return Err(ToolError::new("cancelled", "Join cancelled"));
                }
                self.commit_song_candidate(candidate, "Join clips")?;
                self.selected_track = Some(args.track_id);
                self.selected_clip = Some(id);
                self.last_editor_clip = None;
                let _ = reply.send(Ok(join_result(
                    self.song_session,
                    args.track_id,
                    id,
                    "midi",
                    args.clip_ids.len(),
                )));
                return Ok(None);
            }
            if kind != TrackKind::Audio {
                return Err(ToolError::new(
                    "invalid_arguments",
                    "Join clips on an audio or MIDI track",
                ));
            }
            candidate
                .audio_join_selection(args.track_id, &args.clip_ids)
                .map_err(failure)?;
            let project = candidate.clone();
            let ids = args.clip_ids.clone();
            let track = args.track_id;
            let scratch = self.settings.scratch_dir.clone();
            let rate = self.settings.sample_rate;
            let options = daw_engine::RenderOptions {
                resample_quality: self.settings.resample_quality,
            };
            let used = self
                .waveforms
                .values()
                .map(|data| data.samples.len() * size_of::<f32>())
                .sum::<usize>();
            let preview_limit = crate::project_io::WAVEFORM_FILE_BYTES
                .min(crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(used));
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let (sender, receiver) = mpsc::channel();
            let worker=std::thread::Builder::new().name("rill-tool-join".into()).spawn(move || {
                let result=(||->anyhow::Result<JoinedSource> {
                    anyhow::ensure!(!worker_cancel.load(Ordering::Acquire),"Join cancelled");
                    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
                    let directory=scratch.join("joins").join(format!("tool-{}-{stamp}-{}",std::process::id(),daw_core::new_id()));
                    std::fs::create_dir_all(directory.parent().unwrap())?;
                    std::fs::create_dir(&directory)?;
                    let mut source=JoinedSource {path:directory.join("joined.wav"),committed:false,waveform:None};
                    daw_engine::export_audio_join_cancelled(&project,track,&ids,&source.path,rate,options,&worker_cancel)?;
                    anyhow::ensure!(!worker_cancel.load(Ordering::Acquire),"Join cancelled");
                    if preview_limit>0 {source.waveform=AudioData::load_wav_with_limit(&source.path,preview_limit).ok();}
                    anyhow::ensure!(!worker_cancel.load(Ordering::Acquire),"Join cancelled");
                    Ok(source)
                })().map_err(|_|"Unable to render joined audio; check source files and available scratch storage".to_string());
                let _=sender.send(result);
            }).map_err(|_|ToolError::new("unavailable","Unable to start audio join worker"))?;
            Ok(Some(JoinJob {
                receiver,
                worker: Some(worker),
                cancel,
                owner_cancel,
                reply: Some(reply.clone()),
                session: self.song_session,
                candidate: Some(candidate),
                track,
                ids: args.clip_ids,
                name: args.name,
            }))
        })();
        match result {
            Ok(Some(job)) => self.tool_joins.job = Some(job),
            Ok(None) => {}
            Err(error) => {
                let _ = reply.send(Err(error));
            }
        }
    }

    pub(crate) fn poll_join_tools(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.tool_joins.job else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(20));
        let invalid =
            if job.cancel.load(Ordering::Acquire) || job.owner_cancel.load(Ordering::Acquire) {
                Some(ToolError::new("cancelled", "Join cancelled"))
            } else if job.session != self.song_session {
                Some(ToolError::new(
                    "conflict",
                    "Song changed during join; read fresh context before retrying",
                ))
            } else {
                None
            };
        if let Some(error) = invalid {
            job.cancel.store(true, Ordering::Release);
            if let Some(reply) = job.reply.take() {
                let _ = reply.send(Err(error));
            }
        }
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
            Err(mpsc::TryRecvError::Disconnected) => Err("Audio join worker stopped".into()),
        };
        let mut job = self.tool_joins.job.take().unwrap();
        if let Some(worker) = job.worker.take() {
            let _ = worker.join();
        }
        let Some(reply) = job.reply.take() else {
            drop(result);
            return;
        };
        let result = result
            .map_err(|message| ToolError::new("tool_failed", message))
            .and_then(|mut source| {
                if job.cancel.load(Ordering::Acquire) || job.owner_cancel.load(Ordering::Acquire) {
                    return Err(ToolError::new("cancelled", "Join cancelled"));
                }
                self.tool_ready()?;
                self.check_tool_revision(job.session.id, job.session.revision)?;
                let mut candidate = job.candidate.take().unwrap();
                let id = candidate
                    .replace_audio_clips_with_joined(
                        job.track,
                        &job.ids,
                        source.path.to_string_lossy().into_owned(),
                    )
                    .map_err(failure)?;
                if let Some(name) = job.name.take() {
                    candidate.clip_mut(job.track, id).unwrap().name = name;
                }
                self.commit_song_candidate(candidate, "Join clips")?;
                source.committed = true;
                let used = self
                    .waveforms
                    .values()
                    .map(|data| data.samples.len() * size_of::<f32>())
                    .sum::<usize>();
                if let Some(waveform) = source.waveform.take().filter(|data| {
                    data.samples.len() * size_of::<f32>()
                        <= crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(used)
                }) {
                    self.waveforms
                        .insert(source.path.to_string_lossy().into_owned(), waveform);
                }
                self.selected_track = Some(job.track);
                self.selected_clip = Some(id);
                self.last_editor_clip = None;
                Ok(join_result(
                    self.song_session,
                    job.track,
                    id,
                    "audio",
                    job.ids.len(),
                ))
            });
        let _ = reply.send(result);
    }
}

fn join_result(session: SongSession, track: u64, clip: u64, kind: &str, count: usize) -> Value {
    json!({"session_id":session.id,"revision":session.revision,"track_id":track,"clip_id":clip,
        "kind":kind,"joined_count":count,"changed":true,"created":[{"kind":"clip","id":clip}]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::Clip;

    fn fixture(directory: &std::path::Path) -> (DawApp, u64, Vec<u64>) {
        let mut project = Project::new();
        let track = project.add_audio_track("Join");
        let clips = vec![
            Clip::audio("A", 0.0, 1.0, "missing.wav"),
            Clip::audio("B", 1.0, 1.0, "missing.wav"),
        ];
        let ids = clips.iter().map(|clip| clip.id).collect();
        project.track_mut(track).unwrap().clips = clips;
        let mut app = DawApp::with_context(&egui::Context::default(), project, None);
        app.audio_device_enabled = false;
        app.settings.scratch_dir = directory.to_owned();
        (app, track, ids)
    }

    type QueuedJob = (
        Sender<Result<JoinedSource, String>>,
        Receiver<Result<Value, ToolError>>,
        Arc<AtomicBool>,
    );

    fn queue(app: &mut DawApp, track: u64, ids: Vec<u64>) -> QueuedJob {
        let (sender, receiver) = mpsc::channel();
        let (reply, response) = mpsc::channel();
        let owner_cancel = Arc::new(AtomicBool::new(false));
        app.tool_joins.job = Some(JoinJob {
            receiver,
            worker: None,
            cancel: Arc::new(AtomicBool::new(false)),
            owner_cancel: owner_cancel.clone(),
            reply: Some(reply),
            session: app.song_session,
            candidate: Some(app.project.clone()),
            track,
            ids,
            name: None,
        });
        (sender, response, owner_cancel)
    }

    fn source(directory: &std::path::Path) -> JoinedSource {
        let directory = directory.join("derived");
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("joined.wav");
        AudioData {
            sample_rate: 8_000,
            channels: 1,
            samples: vec![0.2; 8_000],
        }
        .write_wav(&path)
        .unwrap();
        JoinedSource {
            path,
            committed: false,
            waveform: None,
        }
    }

    #[test]
    fn stale_and_cancelled_queued_or_late_sources_are_cleaned_without_commits() {
        for stale in [true, false] {
            for early in [true, false] {
                let directory = tempfile::tempdir().unwrap();
                let (mut app, track, ids) = fixture(directory.path());
                let original = app.project.clone();
                let (sender, response, owner) = queue(&mut app, track, ids);
                let source = source(directory.path());
                let path = source.path.clone();
                if stale {
                    app.song_session.bump();
                } else {
                    owner.store(true, Ordering::Release);
                }
                if early {
                    assert!(sender.send(Ok(source)).is_ok());
                    app.poll_join_tools(&egui::Context::default());
                } else {
                    app.poll_join_tools(&egui::Context::default());
                    assert!(app.tool_joins.pending());
                    assert!(sender.send(Ok(source)).is_ok());
                    app.poll_join_tools(&egui::Context::default());
                }
                let error = response.recv().unwrap().unwrap_err();
                assert_eq!(error.code, if stale { "conflict" } else { "cancelled" });
                assert!(!app.tool_joins.pending());
                assert!(!path.exists());
                assert!(!path.parent().unwrap().exists());
                assert_eq!(app.project, original);
                assert!(!app.history.can_undo());
            }
        }
    }

    #[test]
    fn cancellation_keeps_the_worker_slot_until_its_thread_exits() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, track, ids) = fixture(directory.path());
        let (sender, response, owner) = queue(&mut app, track, ids.clone());
        let source = source(directory.path());
        let path = source.path.clone();
        let (ready, received) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        app.tool_joins.job.as_mut().unwrap().worker = Some(std::thread::spawn(move || {
            assert!(sender.send(Ok(source)).is_ok());
            ready.send(()).unwrap();
            wait.recv().unwrap();
        }));
        received.recv_timeout(Duration::from_secs(3)).unwrap();
        owner.store(true, Ordering::Release);
        app.poll_join_tools(&egui::Context::default());
        assert_eq!(response.recv().unwrap().unwrap_err().code, "cancelled");
        assert!(app.tool_joins.pending());
        let (reply, result) = mpsc::channel();
        app.start_join_tool(json!({"session_id":app.song_session.id,"expected_revision":app.song_session.revision,"track_id":track,"clip_ids":ids}),reply,Arc::new(AtomicBool::new(false)));
        assert_eq!(result.recv().unwrap().unwrap_err().code, "busy");
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while app.tool_joins.pending() {
            assert!(std::time::Instant::now() < deadline);
            app.poll_join_tools(&egui::Context::default());
            std::thread::yield_now();
        }
        assert!(!path.exists());
        assert!(!app.history.can_undo());
    }

    #[test]
    fn renderer_failure_cleans_host_scratch_and_redacts_asset_paths() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, track, ids) = fixture(directory.path());
        let original = app.project.clone();
        let (reply, response) = mpsc::channel();
        app.start_join_tool(json!({"session_id":app.song_session.id,"expected_revision":app.song_session.revision,"track_id":track,"clip_ids":ids}),reply,Arc::new(AtomicBool::new(false)));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.tool_joins.pending() {
            assert!(std::time::Instant::now() < deadline);
            app.poll_join_tools(&egui::Context::default());
            std::thread::yield_now();
        }
        let error = response.recv().unwrap().unwrap_err();
        assert!(!error.message.contains("missing.wav"));
        assert!(
            std::fs::read_dir(directory.path().join("joins"))
                .unwrap()
                .next()
                .is_none()
        );
        assert_eq!(app.project, original);
        assert!(!app.history.can_undo());
    }
}
