//! One-shot sample audition, independent of the song and its transport.
//! Only the selected source is decoded, in the existing isolated decoder.
use crate::app::DawApp;
use daw_devices::SampleData;
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

struct Pending {
    generation: u64,
    receiver: Receiver<Result<Arc<SampleData>, String>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(crate) struct SamplePreview {
    session: u64,
    generation: u64,
    queued: Option<PathBuf>,
    pending: Option<Pending>,
    ready: Option<Arc<SampleData>>,
    path: Option<PathBuf>,
    playing: bool,
    requested_engine: bool,
    failed: Option<(PathBuf, String)>,
    last_error_status: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SamplePreviewStatus<'a> {
    Idle,
    Loading,
    Playing,
    Failed(&'a str),
}

impl DawApp {
    pub(crate) fn preview_sample(&mut self, path: PathBuf) {
        if self.sample_preview.last_error_status.take().as_deref() == Some(self.status.as_str()) {
            self.message(Ok(()), "");
        }
        self.stop_sample_preview();
        self.sample_preview.session = self.song_session.id;
        self.sample_preview.path = Some(path.clone());
        if !self.audio_device_enabled {
            self.fail_sample_preview("Enable an audio output to preview samples".into());
            return;
        }
        self.sample_preview.queued = Some(path);
        if self.sample_preview.pending.is_none() {
            self.start_sample_preview_decode();
        }
        if self.engine.is_none() && self.loading.is_none() {
            self.prepare_engine(self.desired_playing);
        }
        self.sample_preview.requested_engine = true;
    }

    fn start_sample_preview_decode(&mut self) {
        let Some(source) = self.sample_preview.queued.take() else {
            return;
        };
        let generation = self.sample_preview.generation;
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let scratch = self.settings.scratch_dir.clone();
        let worker = std::thread::Builder::new()
            .name("rill-sample-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| -> anyhow::Result<Arc<SampleData>> {
                    let audio = crate::audio_import::decode_audio_preview(
                        &source,
                        &scratch,
                        SampleData::MAX_BYTES,
                        &worker_cancel,
                    )?;
                    let channels: Vec<Vec<f32>> = (0..audio.channels)
                        .map(|channel| {
                            audio
                                .samples
                                .iter()
                                .skip(channel)
                                .step_by(audio.channels)
                                .copied()
                                .collect()
                        })
                        .collect();
                    let slices: Vec<_> = channels.iter().map(Vec::as_slice).collect();
                    anyhow::ensure!(
                        !worker_cancel.load(Ordering::Acquire),
                        "Sample preview cancelled"
                    );
                    let sample = Arc::new(SampleData::new(audio.sample_rate as f32, &slices));
                    anyhow::ensure!(
                        !worker_cancel.load(Ordering::Acquire),
                        "Sample preview cancelled"
                    );
                    Ok(sample)
                })
                .map_err(|_| "Sample preview decoder stopped unexpectedly".to_owned())
                .and_then(|result| result.map_err(|error| format!("{error:#}")));
                let _ = sender.send(result);
            });
        if let Err(error) = worker {
            self.fail_sample_preview(format!("Could not start sample preview: {error}"));
            return;
        }
        self.sample_preview.pending = Some(Pending {
            generation,
            receiver,
            cancel,
        });
    }

    pub(crate) fn stop_sample_preview(&mut self) {
        self.sample_preview.generation = self.sample_preview.generation.wrapping_add(1);
        self.sample_preview.queued = None;
        self.sample_preview.ready = None;
        self.sample_preview.path = None;
        self.sample_preview.playing = false;
        self.sample_preview.requested_engine = false;
        self.sample_preview.failed = None;
        if let Some(pending) = &self.sample_preview.pending {
            pending.cancel.store(true, Ordering::Release);
        }
        if let Some(engine) = &self.engine {
            let _ = engine.audition_sample(None);
        }
    }

    pub(crate) fn sample_preview_active(&self) -> bool {
        self.sample_preview.path.is_some()
            && (self.sample_preview.pending.is_some()
                || self.sample_preview.queued.is_some()
                || self.sample_preview.ready.is_some()
                || (self.sample_preview.playing
                    && self
                        .engine
                        .as_ref()
                        .is_some_and(|engine| engine.sample_audition_active())))
    }

    /// Status belongs to the requested source, independent of current selection.
    /// Errors persist for that source until Stop, selection change, or retry.
    pub(crate) fn sample_preview_status(&self, path: &Path) -> SamplePreviewStatus<'_> {
        if self.sample_preview.session != self.song_session.id {
            return SamplePreviewStatus::Idle;
        }
        if let Some((failed, error)) = &self.sample_preview.failed
            && failed == path
        {
            return SamplePreviewStatus::Failed(error);
        }
        if self.sample_preview.path.as_deref() != Some(path) {
            return SamplePreviewStatus::Idle;
        }
        if self.sample_preview.playing
            && self
                .engine
                .as_ref()
                .is_some_and(|engine| engine.sample_audition_active())
        {
            SamplePreviewStatus::Playing
        } else if self.sample_preview.queued.is_some()
            || self.sample_preview.ready.is_some()
            || self.sample_preview.pending.as_ref().is_some_and(|pending| {
                pending.generation == self.sample_preview.generation
                    && !pending.cancel.load(Ordering::Acquire)
            })
        {
            SamplePreviewStatus::Loading
        } else {
            SamplePreviewStatus::Idle
        }
    }

    fn fail_sample_preview(&mut self, error: String) {
        let path = self.sample_preview.path.clone();
        self.stop_sample_preview();
        if let Some(path) = path {
            self.sample_preview.failed = Some((path, error.clone()));
        }
        self.message(Err(anyhow::anyhow!(error)), "");
        self.sample_preview.last_error_status = Some(self.status.clone());
    }

    pub(crate) fn poll_sample_preview(&mut self, ctx: &egui::Context) {
        if self.sample_preview.session != self.song_session.id {
            self.sample_preview.failed = None;
        }
        if self.sample_preview.path.is_none() && self.sample_preview.pending.is_none() {
            return;
        }
        if self.sample_preview.path.is_some()
            && (self.sample_preview.session != self.song_session.id
                || (!self.library_open && !self.sample_player_editor.open))
        {
            self.stop_sample_preview();
        }
        if self.sample_preview.path.is_some()
            && self.sample_preview.requested_engine
            && self.engine.is_none()
            && self.loading.is_none()
        {
            self.fail_sample_preview("Audio output is unavailable for sample preview".into());
        }
        let completed = self.sample_preview.pending.as_ref().and_then(|pending| {
            match pending.receiver.try_recv() {
                Ok(result) => Some((pending.generation, result)),
                Err(TryRecvError::Disconnected) => Some((
                    pending.generation,
                    Err("Sample preview worker disconnected".into()),
                )),
                Err(TryRecvError::Empty) => None,
            }
        });
        if let Some((generation, result)) = completed {
            self.sample_preview.pending = None;
            if generation == self.sample_preview.generation {
                match result {
                    Ok(sample) => self.sample_preview.ready = Some(sample),
                    Err(error) => {
                        self.fail_sample_preview(error);
                        return;
                    }
                }
            }
        }
        if self.sample_preview.pending.is_none() && self.sample_preview.queued.is_some() {
            self.start_sample_preview_decode();
        }
        if let (Some(sample), Some(engine)) = (&self.sample_preview.ready, &self.engine) {
            match engine.audition_sample(Some(Arc::clone(sample))) {
                Ok(()) => {
                    self.sample_preview.ready = None;
                    self.sample_preview.playing = true;
                }
                Err(error) => {
                    self.fail_sample_preview(format!("{error:#}"));
                    return;
                }
            }
        }
        if self.sample_preview.playing
            && self
                .engine
                .as_ref()
                .is_some_and(|engine| !engine.sample_audition_active())
        {
            self.sample_preview.playing = false;
            self.sample_preview.path = None;
        }
        if self.sample_preview.path.is_some() || self.sample_preview.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}
