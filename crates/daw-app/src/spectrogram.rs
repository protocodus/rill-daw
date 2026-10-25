//! Bounded source previews. File reads, FFT work and image conversion stay on
//! one background worker; the UI only uploads completed, small textures.
use daw_core::{AudioData, Clip, ClipContent};
use eframe::egui;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime},
};

const KEY: &str = "source_spectrogram_cache";
const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(2);
const RETRY_INTERVAL: Duration = Duration::from_secs(15);
type Shared = Arc<Mutex<Cache>>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Hint {
    rate: u32,
    frames: usize,
    samples: [u32; 3],
}
impl Hint {
    fn new(audio: &AudioData) -> Self {
        let n = audio.samples.len();
        Self {
            rate: audio.sample_rate,
            frames: audio.frames(),
            samples: [0, n / 2, n.saturating_sub(1)]
                .map(|i| audio.samples.get(i).copied().unwrap_or(0.0).to_bits()),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified: Option<SystemTime>,
}
impl FileStamp {
    fn read(path: &str) -> anyhow::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}
#[derive(Clone)]
struct Summary {
    sample_rate: u32,
    source_frames: usize,
    columns: usize,
    bands: usize,
    min_frequency: f32,
    max_frequency: f32,
    peak_hz: [f32; 2],
}
struct Prepared {
    summary: Summary,
    image: egui::ColorImage,
    stamp: FileStamp,
}
enum Outcome {
    Ready(Prepared),
    Unchanged,
    Stale,
    Failed(String),
}
struct Job {
    serial: u64,
    generation: u64,
    revision: u64,
    path: String,
    stamp: Option<FileStamp>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
}
struct Completion {
    serial: u64,
    generation: u64,
    revision: u64,
    path: String,
    outcome: Outcome,
}
#[derive(Clone)]
struct Ready {
    path: Arc<str>,
    summary: Summary,
    texture: egui::TextureHandle,
    stamp: FileStamp,
    bytes: usize,
}
struct Painted {
    view: &'static str,
    clip: u64,
    path: Arc<str>,
    source_left: f64,
    source_right: f64,
    transpose: f64,
    texture: egui::TextureId,
    opacity: f32,
    tint_alpha: u8,
    rect: egui::Rect,
    uv: egui::Rect,
}
struct Entry {
    revision: u64,
    hint: Option<Hint>,
    last_seen: u64,
    priority: u8,
    queued: bool,
    loading: bool,
    ready: Option<Ready>,
    error: Option<String>,
    checked: Instant,
}
struct Cache {
    generation: u64,
    cancel_generation: Arc<AtomicU64>,
    frame: u64,
    serial: u64,
    revision: u64,
    sender: mpsc::SyncSender<Job>,
    receiver: mpsc::Receiver<Completion>,
    inflight: Option<u64>,
    inflight_cancel: Option<Arc<AtomicBool>>,
    entries: HashMap<String, Entry>,
    painted: Vec<Painted>,
    bytes: usize,
    jobs_started: u64,
    completed: u64,
    discarded: u64,
    cache_hits: u64,
}

fn prepare(job: &Job, generation: &AtomicU64) -> Outcome {
    let cancelled = || {
        generation.load(Ordering::Acquire) != job.generation || job.cancel.load(Ordering::Acquire)
    };
    if cancelled() {
        return Outcome::Stale;
    }
    let result = (|| -> anyhow::Result<Option<Prepared>> {
        let stamp = FileStamp::read(&job.path)?;
        if job.stamp == Some(stamp) {
            return Ok(None);
        }
        // Match the existing waveform loader's admitted source size. Only one
        // such temporary decode can exist, and it is dropped by this worker.
        let audio =
            AudioData::load_wav_with_limit(&job.path, crate::project_io::WAVEFORM_FILE_BYTES)?;
        anyhow::ensure!(audio.frames() > 0, "Audio source is empty");
        if cancelled() {
            return Ok(None);
        }
        let spectrum = daw_core::compute_audio_spectrogram_cancellable(&audio, &job.cancel)?;
        let peak_hz = [0.25, 0.75].map(|fraction| {
            let column = ((spectrum.columns as f64 * fraction) as usize)
                .min(spectrum.columns.saturating_sub(1));
            let levels = &spectrum.levels[column * spectrum.bands..(column + 1) * spectrum.bands];
            let (band, level) = levels
                .iter()
                .enumerate()
                .max_by_key(|(_, level)| *level)
                .map_or((0, 0), |(band, level)| (band, *level));
            if level == 0 {
                0.0
            } else {
                spectrum.band_frequency(band).unwrap_or(0.0)
            }
        });
        let summary = Summary {
            sample_rate: spectrum.sample_rate,
            source_frames: spectrum.source_frames,
            columns: spectrum.columns,
            bands: spectrum.bands,
            min_frequency: spectrum.min_frequency,
            max_frequency: spectrum.max_frequency,
            peak_hz,
        };
        let image = daw_ui::spectrogram_image(spectrum.columns, spectrum.bands, &spectrum.levels)
            .ok_or_else(|| anyhow::anyhow!("Invalid spectrogram image"))?;
        anyhow::ensure!(
            FileStamp::read(&job.path)? == stamp,
            "Audio source changed during analysis"
        );
        Ok(Some(Prepared {
            summary,
            image,
            stamp,
        }))
    })();
    if cancelled() {
        return Outcome::Stale;
    }
    match result {
        Ok(Some(ready)) => Outcome::Ready(ready),
        Ok(None) => Outcome::Unchanged,
        Err(error) => Outcome::Failed(format!("{error:#}")),
    }
}

impl Cache {
    fn new() -> Self {
        let (sender, jobs) = mpsc::sync_channel::<Job>(1);
        let (results, receiver) = mpsc::sync_channel(1);
        let cancel_generation = Arc::new(AtomicU64::new(1));
        let generation = Arc::clone(&cancel_generation);
        // The thread owns no persistent Context/Cache reference. Dropping the
        // context closes its job channel; no joining or waiting happens on UI.
        let _ = std::thread::Builder::new()
            .name("rill-spectrogram".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        prepare(&job, &generation)
                    }))
                    .unwrap_or_else(|_| Outcome::Failed("Spectral analysis failed".into()));
                    let result = Completion {
                        serial: job.serial,
                        generation: job.generation,
                        revision: job.revision,
                        path: job.path,
                        outcome,
                    };
                    if results.send(result).is_err() {
                        break;
                    }
                    job.ctx.request_repaint();
                }
            });
        Self {
            generation: 1,
            cancel_generation,
            frame: 0,
            serial: 0,
            revision: 0,
            sender,
            receiver,
            inflight: None,
            inflight_cancel: None,
            entries: HashMap::new(),
            painted: Vec::new(),
            bytes: 0,
            jobs_started: 0,
            completed: 0,
            discarded: 0,
            cache_hits: 0,
        }
    }
    fn evict_one(&mut self, keep: Option<&str>) -> bool {
        let oldest = self
            .entries
            .iter()
            .filter(|(path, entry)| {
                Some(path.as_str()) != keep && !entry.loading && entry.last_seen < self.frame
            })
            .min_by_key(|(_, entry)| (entry.last_seen, entry.priority))
            .map(|(path, _)| path.clone());
        if let Some(path) = oldest {
            if let Some(entry) = self.entries.remove(&path) {
                self.bytes -= entry.ready.map_or(0, |ready| ready.bytes);
            }
            true
        } else {
            false
        }
    }
    fn request(&mut self, path: &str, hint: Option<Hint>, priority: u8) -> Option<Ready> {
        if !self.entries.contains_key(path) {
            while self.entries.len() >= MAX_ENTRIES {
                if !self.evict_one(None) {
                    return None;
                }
            }
            self.revision = self.revision.wrapping_add(1);
            self.entries.insert(
                path.to_owned(),
                Entry {
                    revision: self.revision,
                    hint,
                    last_seen: self.frame,
                    priority,
                    queued: true,
                    loading: false,
                    ready: None,
                    error: None,
                    checked: Instant::now(),
                },
            );
        }
        let entry = self.entries.get_mut(path).unwrap();
        if hint.is_some() && entry.hint.is_some() && hint != entry.hint {
            if entry.loading
                && let Some(cancel) = &self.inflight_cancel
            {
                cancel.store(true, Ordering::Release);
            }
            self.bytes -= entry.ready.take().map_or(0, |ready| ready.bytes);
            self.revision = self.revision.wrapping_add(1);
            entry.revision = self.revision;
            entry.queued = true;
            entry.loading = false;
            entry.error = None;
        }
        if hint.is_some() {
            entry.hint = hint;
        }
        entry.priority = if entry.last_seen == self.frame {
            entry.priority.max(priority)
        } else {
            priority
        };
        entry.last_seen = self.frame;
        if !entry.loading
            && entry.checked.elapsed()
                >= if entry.error.is_some() {
                    RETRY_INTERVAL
                } else {
                    CHECK_INTERVAL
                }
        {
            entry.queued = true;
        }
        if entry.ready.is_some() {
            self.cache_hits += 1;
        }
        entry.ready.clone()
    }
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.receiver.try_recv() {
            if self.inflight == Some(result.serial) {
                self.inflight = None;
                self.inflight_cancel = None;
            }
            if result.generation != self.generation
                || self
                    .entries
                    .get(&result.path)
                    .is_none_or(|entry| entry.revision != result.revision)
            {
                self.discarded += 1;
                continue;
            }
            let prepared = match result.outcome {
                Outcome::Ready(ready) => Some(ready),
                other => {
                    let entry = self.entries.get_mut(&result.path).unwrap();
                    entry.loading = false;
                    entry.checked = Instant::now();
                    match other {
                        Outcome::Failed(error) => entry.error = Some(error),
                        Outcome::Stale => {
                            entry.queued = true;
                            self.discarded += 1;
                        }
                        _ => {}
                    }
                    None
                }
            };
            if let Some(prepared) = prepared {
                let bytes = prepared.image.pixels.len() * std::mem::size_of::<egui::Color32>();
                let previous = self.entries.get_mut(&result.path).unwrap().ready.take();
                self.bytes -= previous.map_or(0, |ready| ready.bytes);
                while self.bytes + bytes > MAX_BYTES {
                    if !self.evict_one(Some(&result.path)) {
                        break;
                    }
                }
                let texture = ctx.load_texture(
                    format!("source-spectrum-{}-{}", self.generation, result.revision),
                    prepared.image,
                    egui::TextureOptions::LINEAR,
                );
                self.bytes += bytes;
                let entry = self.entries.get_mut(&result.path).unwrap();
                entry.ready = Some(Ready {
                    path: Arc::from(result.path.as_str()),
                    summary: prepared.summary,
                    stamp: prepared.stamp,
                    texture,
                    bytes,
                });
                entry.loading = false;
                entry.queued = false;
                entry.error = None;
                entry.checked = Instant::now();
                self.completed += 1;
            }
        }
    }
    fn dispatch(&mut self, ctx: &egui::Context) {
        if self.inflight.is_some() {
            return;
        }
        let path = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.queued && entry.last_seen == self.frame)
            .max_by_key(|(_, entry)| {
                (
                    entry.ready.is_none(),
                    entry.priority,
                    std::cmp::Reverse(entry.revision),
                )
            })
            .map(|(path, _)| path.clone());
        let Some(path) = path else {
            return;
        };
        self.serial = self.serial.wrapping_add(1);
        let entry = self.entries.get_mut(&path).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job {
            serial: self.serial,
            generation: self.generation,
            revision: entry.revision,
            stamp: entry.ready.as_ref().map(|ready| ready.stamp),
            path,
            ctx: ctx.clone(),
            cancel: Arc::clone(&cancel),
        };
        match self.sender.try_send(job) {
            Ok(()) => {
                self.inflight = Some(self.serial);
                self.inflight_cancel = Some(cancel);
                entry.loading = true;
                entry.queued = false;
                self.jobs_started += 1;
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                entry.error = Some("Spectrogram worker unavailable".into());
                entry.queued = false;
                entry.checked = Instant::now();
            }
            Err(mpsc::TrySendError::Full(_)) => {}
        }
    }
}

fn shared(ctx: &egui::Context) -> Shared {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_insert_with(egui::Id::new(KEY), || Arc::new(Mutex::new(Cache::new())))
            .clone()
    })
}
pub(crate) fn reset(ctx: &egui::Context) {
    let cache = shared(ctx);
    if let Ok(mut cache) = cache.lock() {
        cache.generation = cache.generation.wrapping_add(1);
        cache
            .cancel_generation
            .store(cache.generation, Ordering::Release);
        if let Some(cancel) = &cache.inflight_cancel {
            cancel.store(true, Ordering::Release);
        }
        cache.entries.clear();
        cache.painted.clear();
        cache.bytes = 0;
    }
}
pub(crate) fn begin_frame(ctx: &egui::Context) {
    let cache = shared(ctx);
    if let Ok(mut cache) = cache.lock() {
        cache.frame += 1;
        cache.painted.clear();
        cache.poll(ctx);
    }
}
pub(crate) fn end_frame(ctx: &egui::Context) {
    let cache = shared(ctx);
    if let Ok(mut cache) = cache.lock() {
        cache.dispatch(ctx);
        if cache.inflight.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
}

/// Prime the focused clip before neighbour drawing so a large session renders
/// its current task first. The data hint inspects three samples, never clones PCM.
pub(crate) fn prioritize(ctx: &egui::Context, path: &str, audio: Option<&AudioData>) {
    if let Ok(mut cache) = shared(ctx).lock() {
        cache.request(path, audio.map(Hint::new), 3);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    painter: &egui::Painter,
    rect: egui::Rect,
    clip: &Clip,
    tempo: f64,
    beats: (f64, f64),
    tint: egui::Color32,
    view: &'static str,
    audio: Option<&AudioData>,
) {
    let ClipContent::Audio {
        path, processing, ..
    } = &clip.content
    else {
        return;
    };
    let visible = rect.intersect(painter.clip_rect());
    if !visible.is_positive() || rect.width() <= 0.0 {
        return;
    }
    let shared = shared(painter.ctx());
    let ready = if let Ok(mut cache) = shared.lock() {
        cache.request(
            path,
            audio.map(Hint::new),
            if view == "audio_editor" { 3 } else { 1 },
        )
    } else {
        None
    };
    let Some(ready) = ready else {
        return;
    };
    let at = |x: f32| beats.0 + (beats.1 - beats.0) * f64::from((x - rect.left()) / rect.width());
    let source_left = clip.audio_source_frame(
        at(visible.left()) - clip.start,
        tempo,
        ready.summary.sample_rate,
    );
    let source_right = clip.audio_source_frame(
        at(visible.right()) - clip.start,
        tempo,
        ready.summary.sample_rate,
    );
    let tint_scale = if painter.ctx().style().visuals.dark_mode {
        0.80
    } else {
        0.65
    };
    let opacity = 72.0 / 255.0 * tint_scale;
    let tint = tint.gamma_multiply(tint_scale);
    let source = daw_ui::SpectrogramSource {
        texture: ready.texture.id(),
        frames: ready.summary.source_frames,
        min_frequency: ready.summary.min_frequency,
        max_frequency: ready.summary.max_frequency,
    };
    if let Some(painted) = daw_ui::paint_audio_spectrogram(
        painter,
        visible,
        source,
        (source_left, source_right),
        processing.transpose_semitones as f32,
        tint,
    ) && let Ok(mut cache) = shared.lock()
    {
        cache.painted.push(Painted {
            view,
            clip: clip.id,
            path: Arc::clone(&ready.path),
            source_left,
            source_right,
            transpose: processing.transpose_semitones,
            texture: ready.texture.id(),
            opacity,
            tint_alpha: tint.a(),
            rect: painted.rect,
            uv: painted.uv,
        });
    }
}

pub(crate) fn diagnostics(ctx: &egui::Context) -> serde_json::Value {
    let shared = shared(ctx);
    let Ok(cache) = shared.lock() else {
        return serde_json::Value::Null;
    };
    let mut sources: Vec<_> = cache.entries.iter().map(|(path, entry)| {
        let mut value = serde_json::json!({"path":path,"revision":entry.revision,"status":if entry.ready.is_some() { "ready" } else if entry.loading { "pending" } else if entry.error.is_some() { "failed" } else { "queued" },"error":entry.error});
        if let Some(ready) = &entry.ready {
            value["columns"] = ready.summary.columns.into(); value["bands"] = ready.summary.bands.into();
            value["source_frames"] = ready.summary.source_frames.into(); value["sample_rate"] = ready.summary.sample_rate.into();
            value["peak_hz"] = serde_json::json!(ready.summary.peak_hz); value["texture_id"] = format!("{:?}", ready.texture.id()).into();
        }
        value
    }).collect();
    sources.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let painted: Vec<_> = cache.painted.iter().map(|paint| {
        let r = paint.rect; let uv = paint.uv;
        serde_json::json!({"view":paint.view,"clip":paint.clip,"path":paint.path.as_ref(),"source_left":paint.source_left,"source_right":paint.source_right,
            "transpose":paint.transpose,"texture_id":format!("{:?}",paint.texture),"opacity":paint.opacity,"tint_alpha":paint.tint_alpha,
            "rect":[r.left(),r.top(),r.right(),r.bottom()],"uv":[uv.left(),uv.top(),uv.right(),uv.bottom()]})
    }).collect();
    serde_json::json!({"generation":cache.generation,"jobs_started":cache.jobs_started,"completed":cache.completed,"discarded":cache.discarded,"cache_hits":cache.cache_hits,
        "cache_bytes":cache.bytes,"queued_count":cache.entries.values().filter(|entry| entry.queued).count(),"pending_count":usize::from(cache.inflight.is_some()),
        "ready_count":cache.entries.values().filter(|entry| entry.ready.is_some()).count(),"sources":sources,"painted":painted,
        "limits":{"entries":MAX_ENTRIES,"bytes":MAX_BYTES,"workers":1,"decode_bytes":crate::project_io::WAVEFORM_FILE_BYTES}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared() -> Outcome {
        Outcome::Ready(Prepared {
            summary: Summary {
                sample_rate: 48000,
                source_frames: 1000,
                columns: 2,
                bands: 2,
                min_frequency: 40.0,
                max_frequency: 24000.0,
                peak_hz: [220.0, 880.0],
            },
            image: egui::ColorImage::new([2, 2], vec![egui::Color32::TRANSPARENT; 4]),
            stamp: FileStamp {
                length: 1000,
                modified: None,
            },
        })
    }

    #[test]
    fn reload_rejects_late_completion_even_when_the_same_source_is_visible_again() {
        let ctx = egui::Context::default();
        let shared = shared(&ctx);
        let (sender, receiver) = mpsc::sync_channel(1);
        let (generation, revision) = {
            let mut cache = shared.lock().unwrap();
            cache.request("same.wav", None, 1);
            cache.receiver = receiver;
            cache.inflight = Some(1);
            (cache.generation, cache.entries["same.wav"].revision)
        };
        reset(&ctx);
        let mut cache = shared.lock().unwrap();
        cache.request("same.wav", None, 1);
        sender
            .send(Completion {
                serial: 1,
                generation,
                revision,
                path: "same.wav".into(),
                outcome: prepared(),
            })
            .unwrap();
        cache.poll(&ctx);
        assert_eq!(cache.discarded, 1);
        assert_eq!(cache.completed, 0);
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries["same.wav"].ready.is_none());
        assert!(cache.entries["same.wav"].queued);
        assert!(cache.inflight.is_none());
    }

    #[test]
    fn source_hint_change_cancels_and_rejects_an_older_revision() {
        let ctx = egui::Context::default();
        let mut cache = Cache::new();
        let first = Hint {
            rate: 48000,
            frames: 1000,
            samples: [0, 1, 2],
        };
        cache.request("edited.wav", Some(first), 1);
        let revision = cache.entries["edited.wav"].revision;
        let cancel = Arc::new(AtomicBool::new(false));
        cache.inflight_cancel = Some(Arc::clone(&cancel));
        cache.inflight = Some(1);
        cache.entries.get_mut("edited.wav").unwrap().loading = true;
        cache.request(
            "edited.wav",
            Some(Hint {
                samples: [3, 4, 5],
                ..first
            }),
            1,
        );
        assert!(cancel.load(Ordering::Acquire));
        let (sender, receiver) = mpsc::sync_channel(1);
        cache.receiver = receiver;
        sender
            .send(Completion {
                serial: 1,
                generation: cache.generation,
                revision,
                path: "edited.wav".into(),
                outcome: prepared(),
            })
            .unwrap();
        cache.poll(&ctx);
        assert_eq!(cache.discarded, 1);
        assert!(cache.entries["edited.wav"].ready.is_none());
        assert!(cache.entries["edited.wav"].queued);
    }

    #[test]
    fn cache_is_bounded_and_visible_entries_survive_lru_eviction() {
        let mut cache = Cache::new();
        cache.frame = 1;
        for index in 0..MAX_ENTRIES + 10 {
            cache.request(&format!("source-{index}"), None, 1);
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        cache.frame = 2;
        cache.request("source-0", None, 3);
        cache.request("new-source", None, 1);
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.entries.contains_key("source-0"));
        assert!(cache.entries.contains_key("new-source"));
    }

    #[test]
    fn empty_source_failure_does_not_kill_the_worker_for_the_next_valid_source() {
        let ctx = egui::Context::default();
        let cache = Cache::new();
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.wav");
        AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: vec![],
        }
        .write_wav(&empty)
        .unwrap();
        let valid = dir.path().join("valid.wav");
        AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: (0..2400)
                .map(|i| (i as f32 * std::f32::consts::TAU * 440.0 / 48000.0).sin() * 0.2)
                .collect(),
        }
        .write_wav(&valid)
        .unwrap();
        for (serial, path) in [(1, empty), (2, valid)] {
            cache
                .sender
                .send(Job {
                    serial,
                    generation: cache.generation,
                    revision: serial,
                    path: path.to_string_lossy().into_owned(),
                    stamp: None,
                    ctx: ctx.clone(),
                    cancel: Arc::new(AtomicBool::new(false)),
                })
                .unwrap();
            let completion = cache
                .receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            if serial == 1 {
                assert!(matches!(completion.outcome, Outcome::Failed(_)));
            } else {
                assert!(matches!(completion.outcome, Outcome::Ready(_)));
            }
        }
    }
}
