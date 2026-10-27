//! Bounded source previews. File reads, FFT work and image conversion stay on
//! one background worker; the UI only uploads completed, small textures.
use daw_core::{AudioData, AudioWaveformPeaks, Clip, ClipContent};
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
    channels: usize,
    pcm: usize,
    samples: [u32; 3],
}
impl Hint {
    fn new(audio: &AudioData) -> Self {
        let n = audio.samples.len();
        Self {
            rate: audio.sample_rate,
            frames: audio.frames(),
            channels: audio.channels,
            pcm: audio.samples.as_ptr() as usize,
            samples: [0, n / 2, n.saturating_sub(1)]
                .map(|i| audio.samples.get(i).copied().unwrap_or(0.0).to_bits()),
        }
    }
    fn matches_audio(self, audio: &AudioData) -> bool {
        let actual = Self::new(audio);
        self.rate == actual.rate
            && self.frames == actual.frames
            && self.channels == actual.channels
            && self.samples == actual.samples
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
    images: [egui::ColorImage; 2],
    colors: [[[u8; 4]; 3]; 2],
    stamp: FileStamp,
}
enum Outcome {
    Peaks(Arc<AudioWaveformPeaks>, FileStamp),
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
    has_peaks: bool,
    has_spectrum: bool,
    blocked: bool,
    hint: Option<Hint>,
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
    textures: [egui::TextureHandle; 2],
    colors: [[[u8; 4]; 3]; 2],
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
    palette: daw_ui::SpectrumPalette,
    rect: egui::Rect,
    uv: egui::Rect,
}

#[derive(Clone)]
pub(crate) struct WaveformPaint {
    pub view: &'static str,
    pub clip: u64,
    pub path: Arc<str>,
    pub stage: &'static str,
    pub columns: usize,
    pub channels: usize,
    pub minimum: f32,
    pub maximum: f32,
    pub source_left: f64,
    pub source_right: f64,
    pub rect: egui::Rect,
    pub vertices: usize,
    pub indices: usize,
    pub fringe_vertices: usize,
    pub sample_reads: usize,
    pub cached: bool,
    pub amplitude_reference: f64,
}
struct Entry {
    revision: u64,
    hint: Option<Hint>,
    last_seen: u64,
    priority: u8,
    queued: bool,
    loading: bool,
    ready: Option<Ready>,
    peaks: Option<Arc<AudioWaveformPeaks>>,
    stamp: Option<FileStamp>,
    blocked_bytes: Option<usize>,
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
    analyses_started: u64,
    completed: u64,
    discarded: u64,
    cache_hits: u64,
    peaks_completed: u64,
    peak_cache_hits: u64,
    raw_paints: u64,
    refined_paints: u64,
    waveform_painted: Vec<WaveformPaint>,
    geometry_builds: u64,
    geometry_cache_hits: u64,
}

fn palette_colors(images: &[egui::ColorImage; 2]) -> [[[u8; 4]; 3]; 2] {
    images.each_ref().map(|image| {
        [64_u8, 128, 255].map(|alpha| {
            image
                .pixels
                .iter()
                .filter(|color| color.a() > 0)
                .min_by_key(|color| color.a().abs_diff(alpha))
                .copied()
                .unwrap_or(egui::Color32::TRANSPARENT)
                .to_array()
        })
    })
}

fn prepare(job: &Job, generation: &AtomicU64, results: &mpsc::SyncSender<Completion>) -> Outcome {
    let cancelled = || {
        generation.load(Ordering::Acquire) != job.generation || job.cancel.load(Ordering::Acquire)
    };
    if cancelled() {
        return Outcome::Stale;
    }
    let result = (|| -> anyhow::Result<Option<Prepared>> {
        let stamp = FileStamp::read(&job.path)?;
        // A file replaced behind the current in-memory PCM must not contribute
        // interior peaks to the old PCM's edge samples. Reimport/open supplies
        // a new PCM identity (or generation), which clears the bound stamp.
        if job.hint.is_some() && job.stamp.is_some_and(|previous| previous != stamp) {
            return Ok(None);
        }
        if job.stamp == Some(stamp) && (job.blocked || job.has_peaks && job.has_spectrum) {
            return Ok(None);
        }
        // Match the existing waveform loader's admitted source size. Only one
        // such temporary decode can exist, and it is dropped by this worker.
        let audio =
            AudioData::load_wav_with_limit(&job.path, crate::project_io::WAVEFORM_FILE_BYTES)?;
        anyhow::ensure!(audio.frames() > 0, "Audio source is empty");
        anyhow::ensure!(
            job.hint.is_none_or(|hint| hint.matches_audio(&audio)),
            "Audio source changed before waveform refinement; reimport or reopen it"
        );
        if cancelled() {
            return Ok(None);
        }
        // Publish exact source extrema first, then retain this same decode for
        // the more expensive FFT. Neither stage clones PCM on the UI thread.
        let peaks = Arc::new(AudioWaveformPeaks::build_cancellable(&audio, &job.cancel)?);
        anyhow::ensure!(
            FileStamp::read(&job.path)? == stamp,
            "Audio source changed during waveform refinement"
        );
        if cancelled() {
            return Ok(None);
        }
        results
            .send(Completion {
                serial: job.serial,
                generation: job.generation,
                revision: job.revision,
                path: job.path.clone(),
                outcome: Outcome::Peaks(peaks, stamp),
            })
            .map_err(|_| anyhow::anyhow!("Source preview cache closed"))?;
        job.ctx.request_repaint();
        if job.stamp == Some(stamp) && job.has_spectrum {
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
        let images = [
            daw_ui::SpectrumPalette::Monochrome,
            daw_ui::SpectrumPalette::BlueYellow,
        ]
        .map(|palette| {
            daw_ui::spectrogram_palette_image(
                spectrum.columns,
                spectrum.bands,
                &spectrum.levels,
                palette,
            )
            .ok_or_else(|| anyhow::anyhow!("Invalid spectrogram image"))
        });
        let [mono, color] = images;
        let images = [mono?, color?];
        let colors = palette_colors(&images);
        anyhow::ensure!(
            FileStamp::read(&job.path)? == stamp,
            "Audio source changed during analysis"
        );
        Ok(Some(Prepared {
            summary,
            images,
            colors,
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
        // Exactly one dispatched job may publish Peaks and then its final
        // outcome. Both must fit even after the last UI frame: Job owns Context,
        // which owns this cache/receiver, so blocking final send would retain a
        // whole closed window until another frame drained the queue.
        let (results, receiver) = mpsc::sync_channel(2);
        let cancel_generation = Arc::new(AtomicU64::new(1));
        let generation = Arc::clone(&cancel_generation);
        // The thread owns no persistent Context/Cache reference. Dropping the
        // context closes its job channel; no joining or waiting happens on UI.
        let _ = std::thread::Builder::new()
            .name("rill-spectrogram".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        prepare(&job, &generation, &results)
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
            analyses_started: 0,
            completed: 0,
            discarded: 0,
            cache_hits: 0,
            peaks_completed: 0,
            peak_cache_hits: 0,
            raw_paints: 0,
            refined_paints: 0,
            waveform_painted: Vec::new(),
            geometry_builds: 0,
            geometry_cache_hits: 0,
        }
    }
    fn evict_one(&mut self, keep: Option<&str>) -> bool {
        let oldest = self
            .entries
            .iter()
            .filter(|(path, entry)| {
                Some(path.as_str()) != keep
                    && !entry.loading
                    && entry.last_seen.saturating_add(1) < self.frame
            })
            .min_by_key(|(_, entry)| (entry.last_seen, entry.priority))
            .map(|(path, _)| path.clone());
        if let Some(path) = oldest {
            if let Some(entry) = self.entries.remove(&path) {
                self.bytes -= entry.ready.map_or(0, |ready| ready.bytes);
                self.bytes -= entry.peaks.map_or(0, |peaks| peaks.byte_size());
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
                    peaks: None,
                    stamp: None,
                    blocked_bytes: None,
                    error: None,
                    checked: Instant::now(),
                },
            );
        }
        let can_reclaim = self
            .entries
            .get(path)
            .is_some_and(|entry| entry.blocked_bytes.is_some())
            && self.entries.iter().any(|(other, entry)| {
                other != path && !entry.loading && entry.last_seen.saturating_add(1) < self.frame
            });
        let entry = self.entries.get_mut(path).unwrap();
        if hint.is_some() && hint != entry.hint {
            if entry.loading
                && let Some(cancel) = &self.inflight_cancel
            {
                cancel.store(true, Ordering::Release);
            }
            self.bytes -= entry.ready.take().map_or(0, |ready| ready.bytes);
            self.bytes -= entry.peaks.take().map_or(0, |peaks| peaks.byte_size());
            self.revision = self.revision.wrapping_add(1);
            entry.revision = self.revision;
            entry.queued = true;
            entry.loading = false;
            entry.error = None;
            entry.stamp = None;
            entry.blocked_bytes = None;
        }
        if entry
            .blocked_bytes
            .is_some_and(|bytes| self.bytes + bytes <= MAX_BYTES || can_reclaim)
        {
            entry.blocked_bytes = None;
            entry.queued = true;
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
            let intermediate = matches!(&result.outcome, Outcome::Peaks(..));
            if !intermediate && self.inflight == Some(result.serial) {
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
            if let Outcome::Peaks(peaks, stamp) = result.outcome {
                let bytes = peaks.byte_size();
                let previous = self.entries.get_mut(&result.path).unwrap().peaks.take();
                self.bytes -= previous.map_or(0, |peaks| peaks.byte_size());
                while self.bytes + bytes > MAX_BYTES && self.evict_one(Some(&result.path)) {}
                if self.bytes + bytes <= MAX_BYTES {
                    self.bytes += bytes;
                    self.entries.get_mut(&result.path).unwrap().peaks = Some(peaks);
                    self.peaks_completed += 1;
                    self.entries.get_mut(&result.path).unwrap().blocked_bytes = None;
                } else {
                    self.entries.get_mut(&result.path).unwrap().blocked_bytes = Some(bytes);
                }
                self.entries.get_mut(&result.path).unwrap().stamp = Some(stamp);
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
                let bytes = prepared
                    .images
                    .iter()
                    .map(|image| image.pixels.len() * std::mem::size_of::<egui::Color32>())
                    .sum();
                let previous = self.entries.get_mut(&result.path).unwrap().ready.take();
                self.bytes -= previous.map_or(0, |ready| ready.bytes);
                while self.bytes + bytes > MAX_BYTES {
                    if !self.evict_one(Some(&result.path)) {
                        break;
                    }
                }
                if self.bytes + bytes > MAX_BYTES {
                    let entry = self.entries.get_mut(&result.path).unwrap();
                    entry.loading = false;
                    entry.queued = false;
                    entry.checked = Instant::now();
                    entry.blocked_bytes = Some(bytes);
                    continue;
                }
                let mut index = 0;
                let textures = prepared.images.map(|image| {
                    let texture = ctx.load_texture(
                        format!(
                            "source-spectrum-{}-{}-{index}",
                            self.generation, result.revision
                        ),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    index += 1;
                    texture
                });
                self.bytes += bytes;
                let entry = self.entries.get_mut(&result.path).unwrap();
                entry.ready = Some(Ready {
                    path: Arc::from(result.path.as_str()),
                    summary: prepared.summary,
                    textures,
                    colors: prepared.colors,
                    bytes,
                });
                entry.loading = false;
                entry.queued = false;
                entry.error = None;
                entry.checked = Instant::now();
                entry.stamp = Some(prepared.stamp);
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
            stamp: entry.stamp,
            has_peaks: entry.peaks.is_some(),
            has_spectrum: entry.ready.is_some(),
            blocked: entry.blocked_bytes.is_some(),
            hint: entry.hint,
            path,
            ctx: ctx.clone(),
            cancel: Arc::clone(&cancel),
        };
        let analyzes_spectrum = !job.has_spectrum && !job.blocked;
        match self.sender.try_send(job) {
            Ok(()) => {
                self.inflight = Some(self.serial);
                self.inflight_cancel = Some(cancel);
                entry.loading = true;
                entry.queued = false;
                self.jobs_started += 1;
                self.analyses_started += u64::from(analyzes_spectrum);
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

pub(crate) fn view_settings(ctx: &egui::Context) -> daw_ui::SpectrumViewOptions {
    let mut settings: daw_ui::SpectrumViewOptions = ctx.data(|data| {
        data.get_temp(egui::Id::new("spectrum_view_options"))
            .unwrap_or_default()
    });
    settings.opacity = if settings.opacity.is_finite() {
        settings.opacity.clamp(0.0, 1.0)
    } else {
        daw_ui::SpectrumViewOptions::default().opacity
    };
    settings
}

pub(crate) fn view_options(ui: &mut egui::Ui) {
    let mut options = view_settings(ui.ctx());
    ui.separator();
    let response = daw_ui::spectrum_view_controls(ui, &mut options);
    ui.ctx().data_mut(|data| {
        data.insert_temp(egui::Id::new("spectrum_view_options"), options);
        let rect = |rect: egui::Rect| [rect.left(), rect.top(), rect.right(), rect.bottom()];
        data.insert_temp(
            egui::Id::new("spectrum_view_controls"),
            serde_json::json!({"intensity":rect(response.intensity.rect),"palette":rect(response.palette.rect)}),
        );
    });
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
        cache.waveform_painted.clear();
        cache.raw_paints = 0;
        cache.refined_paints = 0;
        cache.geometry_builds = 0;
        cache.geometry_cache_hits = 0;
        cache.bytes = 0;
    }
}
pub(crate) fn begin_frame(ctx: &egui::Context) {
    let cache = shared(ctx);
    if let Ok(mut cache) = cache.lock() {
        cache.frame += 1;
        cache.painted.clear();
        cache.waveform_painted.clear();
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

/// Request refinement without waiting for work or copying decoded samples.
pub(crate) fn waveform_peaks(
    ctx: &egui::Context,
    path: &str,
    audio: Option<&AudioData>,
    priority: u8,
) -> Option<Arc<AudioWaveformPeaks>> {
    let shared = shared(ctx);
    let mut cache = shared.lock().ok()?;
    cache.request(path, audio.map(Hint::new), priority);
    let peaks = cache.entries.get(path)?.peaks.clone();
    if peaks.is_some() {
        cache.peak_cache_hits += 1;
    }
    peaks
}

pub(crate) fn waveform_generation(ctx: &egui::Context) -> u64 {
    shared(ctx).lock().map_or(0, |cache| cache.generation)
}

/// Screenshot capture can wait for visible previews without blocking a frame.
pub(crate) fn has_pending_analysis(ctx: &egui::Context) -> bool {
    shared(ctx).lock().is_ok_and(|cache| {
        cache.inflight.is_some()
            || cache
                .entries
                .values()
                .any(|entry| entry.queued && entry.last_seen == cache.frame)
    })
}

pub(crate) fn record_waveform_paint(ctx: &egui::Context, paint: WaveformPaint) {
    if let Ok(mut cache) = shared(ctx).lock() {
        if paint.stage == "refined" {
            cache.refined_paints += 1;
        } else {
            cache.raw_paints += 1;
        }
        if paint.cached {
            cache.geometry_cache_hits += 1;
        } else {
            cache.geometry_builds += 1;
        }
        // Diagnostics retain only the visible frame and cannot grow with song
        // duration or play time, even for many overlapping clip instances.
        if cache.waveform_painted.len() < 512 {
            cache.waveform_painted.push(paint);
        }
    }
}

pub(crate) fn waveform_diagnostics(ctx: &egui::Context) -> serde_json::Value {
    let shared = shared(ctx);
    let Ok(cache) = shared.lock() else {
        return serde_json::Value::Null;
    };
    let mut sources: Vec<_> = cache.entries.iter().map(|(path, entry)| {
        let mut value = serde_json::json!({"path":path,"revision":entry.revision,"status":if entry.peaks.is_some() { "ready" } else if entry.loading { "pending" } else if entry.error.is_some() { "failed" } else { "queued" },"error":entry.error});
        if let Some(peaks) = &entry.peaks {
            value["frames"] = peaks.source_frames.into();
            value["channels"] = peaks.channels.into();
            value["sample_rate"] = peaks.sample_rate.into();
            value["levels"] = peaks.level_count().into();
            value["bytes"] = peaks.byte_size().into();
        }
        value
    }).collect();
    sources.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let painted: Vec<_> = cache.waveform_painted.iter().map(|paint| {
        let rect = paint.rect;
        serde_json::json!({"view":paint.view,"clip":paint.clip,"path":paint.path.as_ref(),"stage":paint.stage,
            "columns":paint.columns,"channels":paint.channels,"minimum":paint.minimum,"maximum":paint.maximum,
            "source_left":paint.source_left,"source_right":paint.source_right,"rect":[rect.left(),rect.top(),rect.right(),rect.bottom()],
            "vertices":paint.vertices,"indices":paint.indices,"fringe_vertices":paint.fringe_vertices,"sample_reads":paint.sample_reads,"cached":paint.cached,
            "amplitude_reference":paint.amplitude_reference})
    }).collect();
    serde_json::json!({"generation":cache.generation,"jobs_started":cache.jobs_started,"completed":cache.peaks_completed,"discarded":cache.discarded,"cache_hits":cache.peak_cache_hits,
        "cache_bytes":cache.bytes,"peak_bytes":cache.entries.values().filter_map(|entry| entry.peaks.as_ref()).map(|peaks| peaks.byte_size()).sum::<usize>(),
        "queued_count":cache.entries.values().filter(|entry| entry.queued).count(),"pending_count":usize::from(cache.inflight.is_some()),
        "ready_count":cache.entries.values().filter(|entry| entry.peaks.is_some()).count(),"sources":sources,"painted":painted,
        "raw_paints":cache.raw_paints,"refined_paints":cache.refined_paints,"geometry_builds":cache.geometry_builds,"geometry_cache_hits":cache.geometry_cache_hits,
        "limits":{"entries":MAX_ENTRIES,"bytes":MAX_BYTES,"workers":1,"decode_bytes":crate::project_io::WAVEFORM_FILE_BYTES}})
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
    let settings = view_settings(painter.ctx());
    let opacity = settings.opacity.clamp(0.0, 1.0) * tint_scale;
    if opacity <= 0.0 {
        return;
    }
    let tint = match settings.palette {
        daw_ui::SpectrumPalette::Monochrome => tint,
        daw_ui::SpectrumPalette::BlueYellow => egui::Color32::WHITE,
    }
    .gamma_multiply(opacity);
    let texture = ready.textures[settings.palette.index()].id();
    let source = daw_ui::SpectrogramSource {
        texture,
        frames: ready.summary.source_frames,
        min_frequency: ready.summary.min_frequency,
        max_frequency: ready.summary.max_frequency,
    };
    // Horizontal cropping changes the source endpoints above. Keep the full
    // frequency axis vertically, so a partially visible row clips its bands
    // instead of squeezing the entire spectrum into the visible row fragment.
    let mapping_rect = egui::Rect::from_min_max(
        egui::pos2(visible.left(), rect.top()),
        egui::pos2(visible.right(), rect.bottom()),
    );
    if let Some(painted) = daw_ui::paint_audio_spectrogram(
        painter,
        mapping_rect,
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
            texture,
            opacity,
            tint_alpha: tint.a(),
            palette: settings.palette,
            rect: painted.rect,
            uv: painted.uv,
        });
    }
}

pub(crate) fn diagnostics(ctx: &egui::Context) -> serde_json::Value {
    let settings = view_settings(ctx);
    let shared = shared(ctx);
    let Ok(cache) = shared.lock() else {
        return serde_json::Value::Null;
    };
    let mut sources: Vec<_> = cache.entries.iter().map(|(path, entry)| {
        let mut value = serde_json::json!({"path":path,"revision":entry.revision,"status":if entry.ready.is_some() { "ready" } else if entry.loading { "pending" } else if entry.error.is_some() { "failed" } else { "queued" },"error":entry.error});
        if let Some(ready) = &entry.ready {
            value["columns"] = ready.summary.columns.into(); value["bands"] = ready.summary.bands.into();
            value["source_frames"] = ready.summary.source_frames.into(); value["sample_rate"] = ready.summary.sample_rate.into();
            value["peak_hz"] = serde_json::json!(ready.summary.peak_hz); value["texture_id"] = format!("{:?}", ready.textures[settings.palette.index()].id()).into();
            value["texture_ids"] = serde_json::json!(ready.textures.iter().map(|texture| format!("{:?}",texture.id())).collect::<Vec<_>>());
            value["texture_colors"] = serde_json::json!(ready.colors);
        }
        value
    }).collect();
    sources.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let painted: Vec<_> = cache.painted.iter().map(|paint| {
        let r = paint.rect; let uv = paint.uv;
        serde_json::json!({"view":paint.view,"clip":paint.clip,"path":paint.path.as_ref(),"source_left":paint.source_left,"source_right":paint.source_right,
            "transpose":paint.transpose,"texture_id":format!("{:?}",paint.texture),"opacity":paint.opacity,"tint_alpha":paint.tint_alpha,"palette":paint.palette.label(),
            "rect":[r.left(),r.top(),r.right(),r.bottom()],"uv":[uv.left(),uv.top(),uv.right(),uv.bottom()]})
    }).collect();
    serde_json::json!({"generation":cache.generation,"jobs_started":cache.jobs_started,"analyses_started":cache.analyses_started,"completed":cache.completed,"discarded":cache.discarded,"cache_hits":cache.cache_hits,
        "cache_bytes":cache.bytes,"queued_count":cache.entries.values().filter(|entry| entry.queued).count(),"pending_count":usize::from(cache.inflight.is_some()),
        "ready_count":cache.entries.values().filter(|entry| entry.ready.is_some()).count(),"sources":sources,"painted":painted,
        "view":{"intensity":settings.opacity,"palette":settings.palette.label()},
        "controls":ctx.data(|data| data.get_temp::<serde_json::Value>(egui::Id::new("spectrum_view_controls"))),
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
            images: std::array::from_fn(|_| {
                egui::ColorImage::new([2, 2], vec![egui::Color32::TRANSPARENT; 4])
            }),
            colors: [[[0; 4]; 3]; 2],
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
            channels: 1,
            pcm: 0,
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
        cache.frame = 3;
        cache.request("source-0", None, 3);
        cache.request("new-source", None, 1);
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.entries.contains_key("source-0"));
        assert!(cache.entries.contains_key("new-source"));
    }

    #[test]
    fn vertically_clipped_waveforms_preserve_their_original_frequency_axis() {
        let ctx = egui::Context::default();
        let shared = shared(&ctx);
        let (sender, receiver) = mpsc::sync_channel(1);
        {
            let mut cache = shared.lock().unwrap();
            cache.request("clipped.wav", None, 1);
            cache.receiver = receiver;
            sender
                .send(Completion {
                    serial: 1,
                    generation: cache.generation,
                    revision: cache.entries["clipped.wav"].revision,
                    path: "clipped.wav".into(),
                    outcome: prepared(),
                })
                .unwrap();
            cache.poll(&ctx);
        }
        let clip = Clip::audio("Clipped", 0.0, 1000.0 / 24000.0, "clipped.wav");
        let plot = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 120.0));
        let _ = ctx.run(Default::default(), |ctx| {
            for (top, bottom) in [(60.0, 120.0), (0.0, 60.0)] {
                let painter = ctx
                    .layer_painter(egui::LayerId::background())
                    .with_clip_rect(egui::Rect::from_min_max(
                        egui::pos2(0.0, top),
                        egui::pos2(200.0, bottom),
                    ));
                paint(
                    &painter,
                    plot,
                    &clip,
                    120.0,
                    (clip.start, clip.end()),
                    egui::Color32::WHITE,
                    "takes",
                    None,
                );
            }
        });
        let cache = shared.lock().unwrap();
        assert_eq!(cache.painted.len(), 2);
        assert_eq!(
            (cache.painted[0].rect.top(), cache.painted[0].rect.bottom()),
            (60.0, 120.0)
        );
        assert_eq!(
            (cache.painted[0].uv.top(), cache.painted[0].uv.bottom()),
            (0.5, 1.0)
        );
        assert_eq!(
            (cache.painted[1].uv.top(), cache.painted[1].uv.bottom()),
            (0.0, 0.5)
        );
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
                    has_peaks: false,
                    has_spectrum: false,
                    blocked: false,
                    hint: None,
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
                assert!(matches!(completion.outcome, Outcome::Peaks(..)));
                let completion = cache
                    .receiver
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
                assert!(matches!(completion.outcome, Outcome::Ready(_)));
            }
        }
    }

    #[test]
    fn progressive_peaks_publish_before_spectrum_without_finishing_the_job() {
        let ctx = egui::Context::default();
        let mut cache = Cache::new();
        cache.request("progressive.wav", None, 3);
        cache.inflight = Some(7);
        cache.entries.get_mut("progressive.wav").unwrap().loading = true;
        let (sender, receiver) = mpsc::sync_channel(2);
        cache.receiver = receiver;
        let peaks = Arc::new(
            AudioWaveformPeaks::build(&AudioData {
                sample_rate: 48000,
                channels: 1,
                samples: vec![0.0, 0.75, -0.25, 0.0],
            })
            .unwrap(),
        );
        sender
            .send(Completion {
                serial: 7,
                generation: cache.generation,
                revision: cache.entries["progressive.wav"].revision,
                path: "progressive.wav".into(),
                outcome: Outcome::Peaks(
                    Arc::clone(&peaks),
                    FileStamp {
                        length: 1000,
                        modified: None,
                    },
                ),
            })
            .unwrap();
        cache.poll(&ctx);
        assert_eq!(cache.inflight, Some(7));
        assert!(cache.entries["progressive.wav"].loading);
        assert!(cache.entries["progressive.wav"].ready.is_none());
        assert!(Arc::ptr_eq(
            cache.entries["progressive.wav"].peaks.as_ref().unwrap(),
            &peaks
        ));
        assert_eq!(cache.peaks_completed, 1);
        assert_eq!(cache.completed, 0);
        assert_eq!(cache.bytes, peaks.byte_size());
        sender
            .send(Completion {
                serial: 7,
                generation: cache.generation,
                revision: cache.entries["progressive.wav"].revision,
                path: "progressive.wav".into(),
                outcome: prepared(),
            })
            .unwrap();
        cache.poll(&ctx);
        assert!(cache.inflight.is_none());
        assert_eq!(cache.completed, 1);
        assert_eq!(cache.bytes, peaks.byte_size() + 32);
    }

    #[test]
    fn late_peak_publication_is_rejected_after_reload() {
        let ctx = egui::Context::default();
        let shared = shared(&ctx);
        let (sender, receiver) = mpsc::sync_channel(1);
        let (generation, revision) = {
            let mut cache = shared.lock().unwrap();
            cache.request("old.wav", None, 1);
            cache.receiver = receiver;
            (cache.generation, cache.entries["old.wav"].revision)
        };
        reset(&ctx);
        let peaks = AudioWaveformPeaks::build(&AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: vec![0.1; 128],
        })
        .unwrap();
        sender
            .send(Completion {
                serial: 1,
                generation,
                revision,
                path: "old.wav".into(),
                outcome: Outcome::Peaks(
                    Arc::new(peaks),
                    FileStamp {
                        length: 1000,
                        modified: None,
                    },
                ),
            })
            .unwrap();
        let mut cache = shared.lock().unwrap();
        cache.poll(&ctx);
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries.is_empty());
        assert_eq!(cache.discarded, 1);
    }

    #[test]
    fn admission_blocked_sources_use_metadata_checks_without_redecoding() {
        let ctx = egui::Context::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metadata-only.wav");
        // Deliberately undecodable: reaching WAV decode would make the result
        // Failed, so this directly verifies the memory-pressure fast path.
        std::fs::write(&path, b"not a WAV").unwrap();
        let path = path.to_string_lossy().into_owned();
        let stamp = FileStamp::read(&path).unwrap();
        let job = Job {
            serial: 1,
            generation: 1,
            revision: 1,
            path,
            stamp: Some(stamp),
            has_peaks: false,
            has_spectrum: false,
            blocked: true,
            hint: None,
            ctx,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        assert!(matches!(
            prepare(&job, &AtomicU64::new(1), &sender),
            Outcome::Unchanged
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn external_file_replacement_cannot_mix_new_peaks_with_old_ui_pcm() {
        use std::io::Write;
        let ctx = egui::Context::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("replaced.wav");
        let audio = AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: vec![0.2; 128],
        };
        audio.write_wav(&path).unwrap();
        let path = path.to_string_lossy().into_owned();
        let stamp = FileStamp::read(&path).unwrap();
        AudioData {
            samples: vec![0.8; 128],
            ..audio.clone()
        }
        .write_wav(&path)
        .unwrap();
        // Trailing data keeps the WAV's declared PCM shape intact while making
        // the replacement distinguishable even on coarse timestamp filesystems.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[0])
            .unwrap();
        let job = Job {
            serial: 1,
            generation: 1,
            revision: 1,
            path,
            stamp: Some(stamp),
            has_peaks: true,
            has_spectrum: true,
            blocked: false,
            hint: Some(Hint::new(&audio)),
            ctx,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        assert!(matches!(
            prepare(&job, &AtomicU64::new(1), &sender),
            Outcome::Unchanged
        ));
        assert!(receiver.try_recv().is_err());
        let initial = Job { stamp: None, ..job };
        assert!(matches!(
            prepare(&initial, &AtomicU64::new(1), &sender),
            Outcome::Failed(_)
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn rejected_peaks_stay_blocked_after_smaller_spectrum_is_admitted() {
        let ctx = egui::Context::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("near-full.wav");
        // Matching metadata can be checked without WAV decoding. Any accidental
        // new preparation after the rejected peak would fail on this body.
        std::fs::write(&path, b"unchanged metadata only").unwrap();
        let path = path.to_string_lossy().into_owned();
        let stamp = FileStamp::read(&path).unwrap();
        let mut cache = Cache::new();
        cache.request(&path, None, 1);
        let revision = cache.entries[&path].revision;
        cache.inflight = Some(1);
        cache.entries.get_mut(&path).unwrap().loading = true;
        // Account for other currently visible residents without allocating a
        // full cache just to exercise its admission policy.
        cache.bytes = MAX_BYTES - 32;
        let (sender, receiver) = mpsc::sync_channel(2);
        cache.receiver = receiver;
        let peaks = Arc::new(
            AudioWaveformPeaks::build(&AudioData {
                sample_rate: 48000,
                channels: 1,
                samples: vec![0.2; 4096],
            })
            .unwrap(),
        );
        let peak_bytes = peaks.byte_size();
        sender
            .send(Completion {
                serial: 1,
                generation: cache.generation,
                revision,
                path: path.clone(),
                outcome: Outcome::Peaks(peaks, stamp),
            })
            .unwrap();
        cache.poll(&ctx);
        assert!(cache.entries[&path].peaks.is_none());
        assert_eq!(cache.entries[&path].blocked_bytes, Some(peak_bytes));
        let Outcome::Ready(mut small) = prepared() else {
            panic!()
        };
        small.stamp = stamp;
        sender
            .send(Completion {
                serial: 1,
                generation: cache.generation,
                revision,
                path: path.clone(),
                outcome: Outcome::Ready(small),
            })
            .unwrap();
        cache.poll(&ctx);
        assert!(cache.entries[&path].ready.is_some());
        assert_eq!(cache.entries[&path].blocked_bytes, Some(peak_bytes));
        cache.entries.get_mut(&path).unwrap().checked = Instant::now() - CHECK_INTERVAL;
        cache.request(&path, None, 1);
        let entry = &cache.entries[&path];
        assert!(entry.queued);
        let job = Job {
            serial: 2,
            generation: cache.generation,
            revision,
            path,
            stamp: entry.stamp,
            has_peaks: entry.peaks.is_some(),
            has_spectrum: entry.ready.is_some(),
            blocked: entry.blocked_bytes.is_some(),
            hint: entry.hint,
            ctx: ctx.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let (sender, receiver) = mpsc::sync_channel(2);
        assert!(matches!(
            prepare(&job, &AtomicU64::new(cache.generation), &sender),
            Outcome::Unchanged
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn dropping_the_window_does_not_leave_a_two_stage_completion_cycle() {
        let ctx = egui::Context::default();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("closing-window.wav");
        AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: vec![0.1; 2400],
        }
        .write_wav(&path)
        .unwrap();
        let cache = shared(&ctx);
        let weak_cache = Arc::downgrade(&cache);
        let job = Job {
            serial: 1,
            generation: 1,
            revision: 1,
            path: path.to_string_lossy().into_owned(),
            stamp: None,
            has_peaks: false,
            has_spectrum: false,
            blocked: false,
            hint: None,
            ctx: ctx.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        cache.lock().unwrap().sender.send(job).unwrap();
        // Do not poll either completion. The worker must finish/drop its Context
        // reference so Context -> Cache -> receiver can be released naturally.
        drop(cache);
        drop(ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while weak_cache.strong_count() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            weak_cache.strong_count(),
            0,
            "Closed preview context was retained by a blocked worker"
        );
    }
}
