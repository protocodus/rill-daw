//! Disk-backed waveform views: bounded overview extrema and a small LRU of
//! detailed PCM tiles. Paint never opens a file or waits for a reader.
use daw_core::{AudioFile, AudioWaveformPeaks, Clip, ClipContent};
use eframe::egui::{self, Rect};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, mpsc},
};
const TILE_FRAMES: usize = 16384;
const DETAIL_WINDOW: usize = 524288;
const DETAIL_BYTES: usize = 16 * 1024 * 1024;
const GEOMETRY_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;

struct Tile {
    path: String,
    start: usize,
    channels: usize,
    samples: Vec<f32>,
}
impl Tile {
    fn end(&self) -> usize {
        self.start + self.samples.len() / self.channels
    }
    fn bytes(&self) -> usize {
        self.samples.capacity() * size_of::<f32>()
    }
}
fn read_tile(file: &AudioFile, start: usize) -> anyhow::Result<Arc<Tile>> {
    let info = file.info();
    let frames = TILE_FRAMES.min(info.frames.saturating_sub(start));
    let mut raw = vec![0.0; frames * info.channels];
    let mut reader = file.reader()?;
    let count = reader.read_frames(start, &mut raw)?;
    anyhow::ensure!(count == frames, "Incomplete waveform detail");
    file.check_unchanged()?;
    let channels = info.channels.min(2);
    let samples = if channels == info.channels {
        raw
    } else {
        raw.chunks_exact(info.channels)
            .flat_map(|frame| frame[..channels].iter().copied())
            .collect()
    };
    Ok(Arc::new(Tile {
        path: file.path().to_string_lossy().into_owned(),
        start,
        channels,
        samples,
    }))
}

struct Request {
    file: AudioFile,
    start: usize,
    generation: u64,
    ctx: egui::Context,
}
struct Completion {
    path: String,
    start: usize,
    generation: u64,
    tile: anyhow::Result<Arc<Tile>>,
}
struct Detail {
    generation: u64,
    frame: u64,
    bytes: usize,
    entries: VecDeque<(u64, Arc<Tile>)>,
    failed: VecDeque<(String, usize)>,
    queue: VecDeque<(AudioFile, usize)>,
    inflight: bool,
    active: Option<(String, usize)>,
    sender: mpsc::SyncSender<Request>,
    receiver: mpsc::Receiver<Completion>,
    revision: u64,
}
impl Detail {
    fn new() -> Self {
        let (sender, jobs) = mpsc::sync_channel::<Request>(1);
        let (results, receiver) = mpsc::sync_channel(1);
        let _ = std::thread::Builder::new()
            .name("rill-waveform-detail".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let path = job.file.path().to_string_lossy().into_owned();
                    let tile = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        read_tile(&job.file, job.start)
                    }))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("Waveform detail failed")));
                    if results
                        .send(Completion {
                            path,
                            start: job.start,
                            generation: job.generation,
                            tile,
                        })
                        .is_err()
                    {
                        break;
                    }
                    job.ctx.request_repaint();
                }
            });
        Self {
            generation: 0,
            frame: 0,
            bytes: 0,
            entries: VecDeque::new(),
            failed: VecDeque::new(),
            queue: VecDeque::new(),
            inflight: false,
            active: None,
            sender,
            receiver,
            revision: 0,
        }
    }
    fn poll(&mut self) {
        while let Ok(result) = self.receiver.try_recv() {
            self.inflight = false;
            self.active = None;
            if result.generation != self.generation {
                continue;
            }
            match result.tile {
                Ok(tile) => {
                    if let Some(index) = self
                        .entries
                        .iter()
                        .position(|(_, old)| old.path == tile.path && old.start == tile.start)
                        && let Some((_, old)) = self.entries.remove(index)
                    {
                        self.bytes -= old.bytes();
                    }
                    while self.bytes + tile.bytes() > DETAIL_BYTES
                        || self.entries.len() >= MAX_ENTRIES
                    {
                        if let Some((_, old)) = self.entries.pop_back() {
                            self.bytes -= old.bytes();
                        } else {
                            break;
                        }
                    }
                    if self.bytes + tile.bytes() <= DETAIL_BYTES {
                        self.bytes += tile.bytes();
                        self.entries.push_front((self.frame, tile));
                        self.revision = self.revision.wrapping_add(1);
                    }
                }
                Err(_) => {
                    self.failed.push_back((result.path, result.start));
                    while self.failed.len() > MAX_ENTRIES {
                        self.failed.pop_front();
                    }
                }
            }
        }
    }
    fn window(
        &mut self,
        ctx: &egui::Context,
        file: &AudioFile,
        start: usize,
        end: usize,
    ) -> Vec<Arc<Tile>> {
        let generation = super::waveform_generation(ctx);
        if self.generation != generation {
            self.generation = generation;
            self.entries.clear();
            self.bytes = 0;
            self.queue.clear();
            self.failed.clear();
            self.revision = self.revision.wrapping_add(1);
        }
        self.frame = self.frame.wrapping_add(1);
        self.poll();
        let path = file.path().to_string_lossy();
        let mut tiles = Vec::new();
        if end.saturating_sub(start) <= DETAIL_WINDOW {
            for first in
                (start / TILE_FRAMES..end.div_ceil(TILE_FRAMES)).map(|bin| bin * TILE_FRAMES)
            {
                if let Some(index) = self
                    .entries
                    .iter()
                    .position(|(_, tile)| tile.path == path && tile.start == first)
                {
                    let (_, tile) = self.entries.remove(index).unwrap();
                    tiles.push(tile.clone());
                    self.entries.push_front((self.frame, tile));
                } else if !self
                    .failed
                    .iter()
                    .any(|(p, s)| p == path.as_ref() && *s == first)
                    && !self
                        .active
                        .as_ref()
                        .is_some_and(|(p, s)| p == path.as_ref() && *s == first)
                    && !self
                        .queue
                        .iter()
                        .any(|(f, s)| f.path() == file.path() && *s == first)
                    && self.queue.len() < 64
                {
                    self.queue.push_back((file.clone(), first));
                }
            }
        }
        if !self.inflight
            && let Some((file, start)) = self.queue.pop_front()
        {
            let active = (file.path().to_string_lossy().into_owned(), start);
            let job = Request {
                file,
                start,
                generation: self.generation,
                ctx: ctx.clone(),
            };
            match self.sender.try_send(job) {
                Ok(()) => {
                    self.inflight = true;
                    self.active = Some(active);
                }
                Err(mpsc::TrySendError::Full(job)) => self.queue.push_front((job.file, job.start)),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.failed.push_back(active);
                    while self.failed.len() > MAX_ENTRIES {
                        self.failed.pop_front();
                    }
                }
            }
        }
        if self.inflight || !self.queue.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        tiles
    }
}
fn detail(ctx: &egui::Context) -> Arc<Mutex<Detail>> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_insert_with(egui::Id::new("streamed_waveform_detail"), || {
            Arc::new(Mutex::new(Detail::new()))
        })
        .clone()
    })
}
pub(crate) fn diagnostics(ctx: &egui::Context) -> serde_json::Value {
    let detail = ctx.data(|data| {
        data.get_temp::<Arc<Mutex<Detail>>>(egui::Id::new("streamed_waveform_detail"))
    });
    let details = detail.as_ref().and_then(|detail| detail.lock().ok());
    let geometry = ctx.data(|data| {
        data.get_temp::<Arc<Mutex<Geometries>>>(egui::Id::new("streamed_waveform_geometry"))
    });
    let meshes = geometry.as_ref().and_then(|geometry| geometry.lock().ok());
    serde_json::json!({"resident_bytes":details.as_ref().map_or(0,|cache|cache.bytes),"tiles":details.as_ref().map_or(0,|cache|cache.entries.len()),"pending":details.as_ref().is_some_and(|cache|cache.inflight||!cache.queue.is_empty()),"byte_limit":DETAIL_BYTES,"tile_frames":TILE_FRAMES,"geometry_bytes":meshes.as_ref().map_or(0,|cache|cache.bytes),"geometry_limit":GEOMETRY_BYTES})
}
pub(crate) fn reset(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<Arc<Mutex<Detail>>>(egui::Id::new("streamed_waveform_detail"));
        data.remove::<Arc<Mutex<Geometries>>>(egui::Id::new("streamed_waveform_geometry"));
    });
}

fn sample(tiles: &[Arc<Tile>], frame: f64, channel: usize, frames: usize) -> Option<f32> {
    if frames == 0 {
        return None;
    }
    let frame = frame.clamp(0.0, (frames - 1) as f64);
    let first = frame.floor() as usize;
    let last = (first + 1).min(frames - 1);
    let value = |frame| {
        tiles
            .iter()
            .find(|tile| frame >= tile.start && frame < tile.end())
            .map(|tile| tile.samples[(frame - tile.start) * tile.channels + channel])
    };
    let a = f64::from(value(first)?);
    let b = f64::from(value(last)?);
    Some(
        (a + (b - a) * (frame - first as f64)).clamp(-f64::from(f32::MAX), f64::from(f32::MAX))
            as f32,
    )
}
fn extrema(
    tiles: &[Arc<Tile>],
    a: f64,
    b: f64,
    channel: usize,
    frames: usize,
) -> Option<(f32, f32, usize)> {
    let first = sample(tiles, a, channel, frames)?;
    let last = sample(tiles, b, channel, frames)?;
    let mut lo = first.min(last);
    let mut hi = first.max(last);
    let mut reads = 2;
    let start = a.min(b).ceil().max(0.0) as usize;
    let end = (a.max(b).floor().max(0.0) as usize)
        .min(frames.saturating_sub(1))
        .saturating_add(1);
    let mut visited = 0;
    for tile in tiles {
        let first = start.max(tile.start);
        let last = end.min(tile.end());
        if first >= last {
            continue;
        }
        for frame in first..last {
            let value = tile.samples[(frame - tile.start) * tile.channels + channel];
            lo = lo.min(value);
            hi = hi.max(value);
            reads += 1;
            visited += 1;
        }
    }
    if visited != end.saturating_sub(start) {
        return None;
    }
    Some((lo, hi, reads))
}

fn fit_reference_with(
    peaks: Option<&AudioWaveformPeaks>,
    file: &AudioFile,
    clip: &Clip,
    tempo: f64,
) -> f64 {
    let ClipContent::Audio {
        gain,
        fade_in,
        fade_out,
        boundary_fades,
        ..
    } = &clip.content
    else {
        return 1.0;
    };
    let Some(peaks) = peaks else {
        return 1.0;
    };
    let bins = if *fade_in > 0.0 || *fade_out > 0.0 || boundary_fades.is_some() {
        128
    } else {
        1
    };
    let mut peak = 0.0_f64;
    for bin in 0..bins {
        let start = clip.length * bin as f64 / bins as f64;
        let end = clip.length * (bin + 1) as f64 / bins as f64;
        let a = clip.audio_source_frame(start, tempo, file.info().sample_rate);
        let b = clip.audio_source_frame(end, tempo, file.info().sample_rate);
        let scale =
            f64::from(gain.abs()) * f64::from(clip.audio_envelope_gain((start + end) * 0.5));
        for channel in 0..file.info().channels.min(2) {
            if let Some((lo, hi)) = peaks.overview_range(
                a.min(b).floor().max(0.0) as usize,
                (a.max(b).ceil().max(0.0) as usize).saturating_add(1),
                channel,
            ) {
                peak = peak.max(f64::from(lo.abs().max(hi.abs())) * scale);
            }
        }
    }
    if peak > 0.0 { peak.max(0.001) } else { 1.0 }
}
pub(crate) fn fit_reference(ctx: &egui::Context, file: &AudioFile, clip: &Clip, tempo: f64) -> f32 {
    let path = file.path().to_string_lossy();
    let peaks = super::waveform_file_peaks(ctx, &path, file, 3);
    fit_reference_with(peaks.as_deref(), file, clip, tempo).min(f64::from(f32::MAX)) as f32
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ClipKey {
    timing: [f64; 4],
    gain: f32,
    fades: [f64; 2],
    curves: [daw_core::FadeCurve; 2],
    reverse: bool,
    processing: daw_core::AudioProcessing,
    envelope: Option<daw_core::AudioEnvelope>,
    boundary: Option<daw_core::AudioBoundaryFades>,
    crossfade: Option<daw_core::AudioCrossfade>,
    /// Hash of the warp markers; the drawn geometry follows them.
    warp: u64,
}
impl ClipKey {
    pub(crate) fn new(clip: &Clip) -> Self {
        let ClipContent::Audio {
            gain,
            fade_in,
            fade_out,
            fade_in_curve,
            fade_out_curve,
            reverse,
            processing,
            envelope,
            boundary_fades,
            crossfade,
            ..
        } = &clip.content
        else {
            unreachable!("Audio drawing requires an audio clip");
        };
        Self {
            timing: [clip.start, clip.length, clip.offset, clip.source_rate],
            gain: *gain,
            fades: [*fade_in, *fade_out],
            curves: [*fade_in_curve, *fade_out_curve],
            reverse: *reverse,
            processing: *processing,
            envelope: *envelope,
            boundary: *boundary_fades,
            crossfade: *crossfade,
            warp: warp_hash(&clip.warp_markers),
        }
    }
}

fn warp_hash(markers: &[daw_core::WarpMarker]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for marker in markers {
        marker.source.to_bits().hash(&mut hasher);
        marker.beat.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}
#[derive(Clone)]
struct SourceKey(AudioFile);
impl PartialEq for SourceKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.same_source(&other.0)
    }
}
fn source_edges(clip: &Clip, tempo: f64, info: daw_core::AudioFileInfo) -> (f64, f64) {
    let frames_per_beat = f64::from(info.sample_rate) * 60.0 / tempo;
    let start = (clip.offset * frames_per_beat).max(0.0);
    let end = (crate::audio_editor::source_frame_ceil(clip.source_end() * frames_per_beat) - 1.0)
        .max(start)
        .min(info.frames.saturating_sub(1) as f64);
    (start.min(end), end)
}

#[derive(PartialEq)]
struct Key {
    clip: ClipKey,
    source: SourceKey,
    rect: Rect,
    visible: Rect,
    view: daw_ui::TimelineViewport,
    tempo: f64,
    color: egui::Color32,
    background: egui::Color32,
    density: f32,
    headroom: f32,
    amplitude: daw_ui::WaveformAmplitudeScale,
    colored: bool,
    peaks: usize,
    detail: u64,
    generation: u64,
    mono: bool,
}
struct Geometry {
    prepared: daw_ui::PreparedAudioWaveform,
    minimum: f32,
    maximum: f32,
    columns: usize,
    reads: usize,
    reference: f64,
    stage: &'static str,
    bytes: usize,
}
#[derive(Default)]
struct Geometries {
    entries: VecDeque<(Key, Arc<Geometry>)>,
    bytes: usize,
}
fn geometry(ctx: &egui::Context) -> Arc<Mutex<Geometries>> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_insert_with(egui::Id::new("streamed_waveform_geometry"), || {
            Arc::new(Mutex::new(Geometries::default()))
        })
        .clone()
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    painter: &egui::Painter,
    rect: Rect,
    view: daw_ui::TimelineViewport,
    file: &AudioFile,
    clip: &Clip,
    tempo: f64,
    headroom: f32,
    color: egui::Color32,
    view_name: &'static str,
    amplitude: daw_ui::WaveformAmplitudeScale,
) {
    let ClipContent::Audio { path, gain, .. } = &clip.content else {
        return;
    };
    let info = file.info();
    if !rect.is_positive() || !rect.is_finite() || info.frames == 0 || tempo <= 0.0 {
        return;
    }
    let clipped = rect.intersect(painter.clip_rect());
    let visible = Rect::from_min_max(
        egui::pos2(
            view.x_at(rect, clip.start).max(clipped.left()),
            clipped.top(),
        ),
        egui::pos2(
            view.x_at(rect, clip.end()).min(clipped.right()),
            clipped.bottom(),
        ),
    );
    if !visible.is_positive() {
        return;
    }
    let start = view.beat_at(rect, visible.left()).max(clip.start);
    let end = view.beat_at(rect, visible.right()).min(clip.end());
    if end <= start {
        return;
    }
    if view_name == "audio_editor" {
        super::prioritize(painter.ctx(), path, None);
    }
    let peaks = super::waveform_file_peaks(
        painter.ctx(),
        path,
        file,
        if view_name == "audio_editor" { 3 } else { 1 },
    );
    let a = clip.audio_source_frame(start - clip.start, tempo, info.sample_rate);
    let b = clip.audio_source_frame(end - clip.start, tempo, info.sample_rate);
    let (source_first, source_last) = source_edges(clip, tempo, info);
    let edge = |frame: f64| frame.clamp(source_first, source_last);
    let tiles = detail(painter.ctx());
    let (tiles, revision) = if let Ok(mut cache) = tiles.lock() {
        let tiles = cache.window(
            painter.ctx(),
            file,
            (a.min(b).floor().max(0.0) as usize).saturating_sub(2),
            (a.max(b).ceil().max(0.0) as usize)
                .saturating_add(3)
                .min(info.frames),
        );
        (tiles, cache.revision)
    } else {
        (Vec::new(), 0)
    };
    let mono = view_name == "takes";
    let channels = if mono { 1 } else { info.channels.min(2) };
    let palette = daw_ui::theme::palette(painter.ctx());
    let settings = super::view_settings(painter.ctx());
    super::paint(
        painter,
        visible,
        clip,
        tempo,
        (start, end),
        color,
        view_name,
        None,
    );
    let key = Key {
        clip: ClipKey::new(clip),
        source: SourceKey(file.clone()),
        rect,
        visible,
        view,
        tempo,
        color,
        background: palette.bg,
        density: painter.pixels_per_point(),
        headroom,
        amplitude,
        colored: settings.waveform_coloring,
        peaks: peaks.as_ref().map_or(0, |p| Arc::as_ptr(p) as usize),
        detail: revision,
        generation: super::waveform_generation(painter.ctx()),
        mono,
    };
    let shared = geometry(painter.ctx());
    let cached = shared.lock().ok().and_then(|mut cache| {
        let index = cache.entries.iter().position(|(old, _)| old == &key)?;
        let entry = cache.entries.remove(index)?;
        let result = entry.1.clone();
        cache.entries.push_front(entry);
        Some(result)
    });
    let warm = cached.is_some();
    let prepared = cached.unwrap_or_else(|| {
        let reference = if amplitude == daw_ui::WaveformAmplitudeScale::Fit {
            fit_reference_with(peaks.as_deref(), file, clip, tempo)
        } else {
            1.0
        };
        let bins = (visible.width() * painter.pixels_per_point())
            .ceil()
            .clamp(1.0, 4096.0) as usize;
        let mut columns = Vec::with_capacity(bins * channels);
        let (mut minimum, mut maximum) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut reads = 0;
        let mut detailed = false;
        for x in 0..bins {
            let left = visible.left() + visible.width() * x as f32 / bins as f32;
            let right = visible.left() + visible.width() * (x + 1) as f32 / bins as f32;
            let start = view.beat_at(rect, left).max(clip.start);
            let end = view.beat_at(rect, right).min(clip.end());
            let a = edge(clip.audio_source_frame(start - clip.start, tempo, info.sample_rate));
            let b = edge(clip.audio_source_frame(end - clip.start, tempo, info.sample_rate));
            for channel in 0..info.channels.min(2) {
                let values = extrema(&tiles, a, b, channel, info.frames)
                    .map(|(lo, hi, n)| {
                        reads += n;
                        detailed = true;
                        (lo, hi)
                    })
                    .or_else(|| {
                        peaks.as_ref().and_then(|peaks| {
                            peaks.overview_range(
                                a.min(b).floor().max(0.0) as usize,
                                (a.max(b).ceil().max(0.0) as usize).saturating_add(1),
                                channel,
                            )
                        })
                    })
                    .unwrap_or((0.0, 0.0));
                let scale = f64::from(*gain)
                    * f64::from(clip.audio_envelope_gain((start + end) * 0.5 - clip.start));
                let lo = f64::from(values.0) * scale;
                let hi = f64::from(values.1) * scale;
                minimum = minimum.min(lo.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32);
                maximum = maximum.max(hi.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32);
                let lo = (lo / reference).clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
                let hi = (hi / reference).clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
                if mono && channel > 0 {
                    if let Some(last) = columns.last_mut() {
                        let last: &mut daw_ui::WaveformColumn = last;
                        last.minimum = last.minimum.min(lo);
                        last.maximum = last.maximum.max(hi);
                    }
                } else {
                    columns.push(daw_ui::WaveformColumn {
                        beat: (start + end) * 0.5,
                        channel,
                        minimum: lo,
                        maximum: hi,
                    });
                }
            }
        }
        let prepared = daw_ui::prepare_audio_waveform_styled(
            rect,
            view,
            &columns,
            channels,
            headroom,
            painter.pixels_per_point(),
            daw_ui::WaveformColorStyle {
                peak: daw_ui::waveform_peak_color(color, palette),
                quiet: daw_ui::waveform_quiet_color(color, palette.muted),
                center: palette.line,
                amplitude_reference: reference,
                amplitude_coloring: settings.waveform_coloring,
            },
        );
        let result = Arc::new(Geometry {
            bytes: prepared.bytes()
                + size_of::<Key>()
                + size_of::<Geometry>()
                + file.path().as_os_str().as_encoded_bytes().len()
                + 512,
            prepared,
            minimum,
            maximum,
            columns: columns.len(),
            reads,
            reference,
            stage: if detailed {
                "disk_detail"
            } else if peaks.is_some() {
                "disk_overview"
            } else {
                "disk_pending"
            },
        });
        if let Ok(mut cache) = shared.lock() {
            while cache.bytes + result.bytes > GEOMETRY_BYTES || cache.entries.len() >= 512 {
                if let Some((_, old)) = cache.entries.pop_back() {
                    cache.bytes -= old.bytes;
                } else {
                    break;
                }
            }
            if cache.bytes + result.bytes <= GEOMETRY_BYTES {
                cache.bytes += result.bytes;
                cache.entries.push_front((key, result.clone()));
            }
        }
        result
    });
    prepared.prepared.paint(painter);
    if prepared.stage == "disk_pending" && visible.width() > 160.0 && visible.height() > 30.0 {
        daw_ui::text::paint_row(
            painter,
            visible.shrink(4.0),
            daw_ui::text::TextRole::Note,
            if super::waveform_error(painter.ctx(), path).is_some() {
                "Waveform unavailable"
            } else {
                "Preparing waveform…"
            },
        );
    }
    super::record_waveform_paint(
        painter.ctx(),
        super::WaveformPaint {
            view: view_name,
            clip: clip.id,
            path: Arc::from(path.as_str()),
            stage: prepared.stage,
            columns: prepared.columns,
            channels,
            minimum: prepared.minimum,
            maximum: prepared.maximum,
            source_left: a,
            source_right: b,
            rect: visible,
            vertices: prepared.prepared.stats.vertices,
            indices: prepared.prepared.stats.indices,
            fringe_vertices: prepared.prepared.stats.fringe_vertices,
            sample_reads: if warm { 0 } else { prepared.reads },
            cached: warm,
            amplitude_reference: prepared.reference,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detailed_tiles_preserve_fractional_edges_and_stereo_peaks_across_boundaries() {
        let extreme = vec![Arc::new(Tile {
            path: "headroom.wav".into(),
            start: 0,
            channels: 1,
            samples: vec![-f32::MAX, f32::MAX],
        })];
        assert_eq!(sample(&extreme, 0.5, 0, 2), Some(0.0));

        let mut first = vec![0.0; TILE_FRAMES * 2];
        let mut second = vec![0.0; TILE_FRAMES * 2];
        first[(TILE_FRAMES - 1) * 2] = 0.8;
        first[(TILE_FRAMES - 1) * 2 + 1] = -0.4;
        second[0] = -0.6;
        second[1] = 0.3;
        second[2] = 1.0;
        second[3] = -0.9;
        let tiles = vec![
            Arc::new(Tile {
                path: "signal.wav".into(),
                start: 0,
                channels: 2,
                samples: first,
            }),
            Arc::new(Tile {
                path: "signal.wav".into(),
                start: TILE_FRAMES,
                channels: 2,
                samples: second,
            }),
        ];
        let frame = TILE_FRAMES as f64 - 0.5;
        assert!((sample(&tiles, frame, 0, TILE_FRAMES * 2).unwrap() - 0.1).abs() < 1e-6);
        assert!((sample(&tiles, frame, 1, TILE_FRAMES * 2).unwrap() + 0.05).abs() < 1e-6);
        assert_eq!(
            extrema(&tiles, frame, TILE_FRAMES as f64 + 1.0, 0, TILE_FRAMES * 2)
                .map(|(lo, hi, _)| (lo, hi)),
            Some((-0.6, 1.0))
        );
        assert_eq!(
            extrema(&tiles, frame, TILE_FRAMES as f64 + 1.0, 1, TILE_FRAMES * 2)
                .map(|(lo, hi, _)| (lo, hi)),
            Some((-0.9, 0.3))
        );
        assert!(
            extrema(
                &tiles[..1],
                frame,
                TILE_FRAMES as f64 + 1.0,
                0,
                TILE_FRAMES * 2
            )
            .is_none(),
            "Missing detail must refine from overview, never invent silent samples"
        );
    }
}

#[cfg(test)]
mod cache_regressions {
    use super::*;
    #[test]
    fn fractional_clip_edges_and_fixed_size_keys_preserve_signal_without_retaining_names() {
        let mut clip = Clip::audio("Source", 0.0, 2.0 / 24000.0, "source.wav");
        clip.offset = 0.5 / 24000.0;
        let info = daw_core::AudioFileInfo {
            sample_rate: 48000,
            channels: 1,
            frames: 3,
            decoded_bytes: 12,
        };
        let (start, end) = source_edges(&clip, 120.0, info);
        assert!((start - 0.5).abs() < 1e-12);
        assert_eq!(end, 2.0);
        let tiles = vec![Arc::new(Tile {
            path: "source.wav".into(),
            start: 0,
            channels: 1,
            samples: vec![1.0, 0.0, 0.0],
        })];
        assert_eq!(sample(&tiles, start, 0, 3), Some(0.5));
        let key = ClipKey::new(&clip);
        clip.name = "long name ".repeat(131072);
        assert!(ClipKey::new(&clip) == key);
        assert!(!std::mem::needs_drop::<ClipKey>());
        assert!(size_of::<ClipKey>() <= 256);
        if let ClipContent::Audio { gain, .. } = &mut clip.content {
            *gain = 0.5;
        }
        assert!(
            ClipKey::new(&clip) != key,
            "Signal edits still invalidate geometry"
        );
        let key = ClipKey::new(&clip);
        clip.warp_markers = vec![daw_core::WarpMarker {
            source: clip.offset,
            beat: 0.0,
        }];
        assert!(ClipKey::new(&clip) != key, "Warp edits redraw the waveform");
    }
    #[test]
    fn multichannel_detail_retains_only_displayed_legs_and_one_window_fits_the_cache() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sixteen.wav");
        AudioDataForTest::write(&path);
        let file = AudioFile::open(&path).unwrap();
        let tile = read_tile(&file, 0).unwrap();
        assert_eq!(tile.channels, 2);
        assert_eq!(tile.samples.len(), TILE_FRAMES * 2);
        assert_eq!(&tile.samples[..2], &[0.7, -0.3]);
        assert!(tile.bytes() * DETAIL_WINDOW.div_ceil(TILE_FRAMES) <= DETAIL_BYTES);
    }
    struct AudioDataForTest;
    impl AudioDataForTest {
        fn write(path: &std::path::Path) {
            daw_core::AudioData {
                sample_rate: 48000,
                channels: 16,
                samples: (0..TILE_FRAMES)
                    .flat_map(|_| {
                        [
                            0.7, -0.3, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9,
                            0.9, 0.9,
                        ]
                    })
                    .collect(),
            }
            .write_wav(path)
            .unwrap();
        }
    }
    #[test]
    fn widening_visible_clip_rebuilds_the_submitted_waveform_mesh() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wide.wav");
        let data = daw_core::AudioData {
            sample_rate: 48000,
            channels: 1,
            samples: (0..1_200_000)
                .map(|frame| (frame as f32 * 0.07).sin() * 0.4)
                .collect(),
        };
        data.write_wav(&path).unwrap();
        let file = AudioFile::open(&path).unwrap();
        let ctx = egui::Context::default();
        daw_ui::theme::apply(&ctx);
        let clip = Clip::audio("Wide source", 0.0, 50.0, path.to_string_lossy());
        let shared = super::super::shared(&ctx);
        {
            let mut cache = shared.lock().unwrap();
            let path = path.to_string_lossy();
            cache.request(&path, None, 1);
            let entry = cache.entries.get_mut(path.as_ref()).unwrap();
            entry.peaks = Some(Arc::new(AudioWaveformPeaks::build(&data).unwrap()));
            entry.queued = false;
        }
        let frame = |width: f32| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(2000.0, 200.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    let painter = ctx
                        .layer_painter(egui::LayerId::background())
                        .with_clip_rect(Rect::from_min_size(
                            egui::pos2(20.0, 20.0),
                            egui::vec2(width, 120.0),
                        ));
                    paint(
                        &painter,
                        Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(2400.0, 120.0)),
                        daw_ui::TimelineViewport {
                            pixels_per_beat: 48.0,
                            row_height: 60.0,
                            ..Default::default()
                        },
                        &file,
                        &clip,
                        120.0,
                        1.0,
                        egui::Color32::LIGHT_BLUE,
                        "arranger",
                        daw_ui::WaveformAmplitudeScale::Default,
                    );
                },
            )
        };
        frame(1100.0);
        let output = frame(1500.0);
        let maximum = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) => Some(
                    mesh.vertices
                        .iter()
                        .map(|vertex| vertex.pos.x)
                        .fold(f32::NEG_INFINITY, f32::max),
                ),
                _ => None,
            })
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            maximum > 1490.0,
            "A resized viewport must submit newly visible waveform columns, got {maximum}"
        );
        assert_eq!(geometry(&ctx).lock().unwrap().entries.len(), 2);
    }
}
