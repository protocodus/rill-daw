//! Reusable bounded windows. File operations run only on graph/preparation workers.
use anyhow::{Context, Result, ensure};
use daw_core::{AudioFile, AudioFileReader};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
pub(crate) const WINDOW_FRAMES: usize = 8192;
pub(crate) const WINDOW_BYTES: usize = WINDOW_FRAMES * 2 * size_of::<f32>() + 64 * 1024;
pub(crate) const MAX_WINDOW_BYTES: usize = 64 * 1024 * 1024;
const ALIGN_FRAMES: usize = 4096;
#[derive(Default)]
pub(crate) struct ReadMetrics {
    pub operations: AtomicU64,
    pub frames: AtomicU64,
}
pub(crate) struct StreamWindow {
    file: Option<AudioFile>,
    reader: Option<AudioFileReader>,
    samples: Vec<f32>,
    start: usize,
    frames: usize,
    pub metrics: Arc<ReadMetrics>,
}
impl StreamWindow {
    pub fn new() -> Self {
        Self {
            file: None,
            reader: None,
            samples: vec![0.0; WINDOW_FRAMES * 2],
            start: 0,
            frames: 0,
            metrics: Arc::new(ReadMetrics::default()),
        }
    }
    pub fn prepare(&mut self, file: &AudioFile, first: usize, last: usize) -> Result<()> {
        file.check_unchanged()?;
        let info = file.info();
        ensure!(
            (1..=2).contains(&info.channels),
            "Streamed playback requires prepared mono or stereo PCM"
        );
        ensure!(
            first <= last && last < info.frames,
            "Invalid streamed source window"
        );
        let same = self.file.as_ref().is_some_and(|old| old.same_source(file));
        if same && first >= self.start && last < self.start + self.frames {
            return Ok(());
        }
        if !same {
            self.reader = None;
            self.file = Some(file.clone());
            self.reader = Some(file.reader()?);
            self.frames = 0;
        }
        ensure!(
            last - first < WINDOW_FRAMES,
            "Audio source window exceeds the streaming bound"
        );
        let aligned = first / ALIGN_FRAMES * ALIGN_FRAMES;
        let start = if last - aligned < WINDOW_FRAMES {
            aligned
        } else {
            first
        };
        let frames = WINDOW_FRAMES.min(info.frames - start);
        let actual = self
            .reader
            .as_mut()
            .expect("stream reader")
            .read_frames(start, &mut self.samples[..frames * info.channels])
            .context("Read streamed audio window")?;
        ensure!(actual == frames, "Streamed audio source was truncated");
        self.start = start;
        self.frames = actual;
        self.metrics.operations.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .frames
            .fetch_add(actual as u64, Ordering::Relaxed);
        Ok(())
    }
    #[inline]
    pub fn sample(&self, frame: usize, channel: usize) -> f32 {
        debug_assert!(frame >= self.start && frame < self.start + self.frames);
        let channels = self.file.as_ref().expect("prepared source").info().channels;
        self.samples[(frame - self.start) * channels + channel.min(channels - 1)]
    }
}
static WINDOW_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
pub(crate) struct WindowPair([StreamWindow; 2]);
impl std::ops::Deref for WindowPair {
    type Target = [StreamWindow; 2];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for WindowPair {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for WindowPair {
    fn drop(&mut self) {
        WINDOW_ALLOCATED.fetch_sub(2 * WINDOW_BYTES, Ordering::AcqRel);
    }
}
pub(crate) fn windows() -> Result<Box<WindowPair>> {
    // `try_update` replaces this from Rust 1.95, above the workspace MSRV.
    #[allow(deprecated)]
    let reserved = WINDOW_ALLOCATED.fetch_update(Ordering::AcqRel, Ordering::Acquire, |bytes| {
        bytes
            .checked_add(2 * WINDOW_BYTES)
            .filter(|next| *next <= MAX_WINDOW_BYTES)
    });
    ensure!(
        reserved.is_ok(),
        "Streamed audio working buffers exceed the 64 MiB process budget"
    );
    Ok(Box::new(WindowPair([
        StreamWindow::new(),
        StreamWindow::new(),
    ])))
}
pub(crate) fn window_bytes() -> u64 {
    WINDOW_ALLOCATED.load(Ordering::Relaxed) as u64
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioStreamingSnapshot {
    pub streamed_clips: u32,
    pub resident_bytes: u64,
    /// Reserved streaming window/reader memory across current and pending graphs.
    pub window_bytes: u64,
    pub disk_read_operations: u64,
    pub disk_frames_read: u64,
}
pub(crate) struct SharedStreaming {
    values: [AtomicU64; 5],
}
impl Default for SharedStreaming {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}
impl SharedStreaming {
    pub fn publish(&self, snapshot: AudioStreamingSnapshot) {
        for (value, next) in self.values.iter().zip([
            u64::from(snapshot.streamed_clips),
            snapshot.resident_bytes,
            snapshot.window_bytes,
            snapshot.disk_read_operations,
            snapshot.disk_frames_read,
        ]) {
            value.store(next, Ordering::Relaxed);
        }
    }
    pub fn snapshot(&self) -> AudioStreamingSnapshot {
        AudioStreamingSnapshot {
            streamed_clips: self.values[0].load(Ordering::Relaxed) as u32,
            resident_bytes: self.values[1].load(Ordering::Relaxed),
            window_bytes: self.values[2].load(Ordering::Relaxed),
            disk_read_operations: self.values[3].load(Ordering::Relaxed),
            disk_frames_read: self.values[4].load(Ordering::Relaxed),
        }
    }
}
