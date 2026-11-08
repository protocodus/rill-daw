//! Bounded source-window analysis. PCM reads retain absolute source coordinates,
//! including a neighbouring frame for the resident sampler's interpolation.
use crate::{
    AudioFile, AudioSilenceSettings, Clip, TransientSettings,
    audio_analysis::{AudioAnalysis, interpolate_sample, validate_clip},
};
use anyhow::{Context, Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};

const CACHE_BYTES: usize = 256 * 1024;

fn check_cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "Audio analysis cancelled");
    Ok(())
}

/// A small source-coordinate cache. Both directions use the same interleaved
/// channel frames; reversal changes lookup positions, never channel order.
struct Samples<'a, R> {
    read: R,
    channels: usize,
    frames: usize,
    block: usize,
    first: usize,
    loaded: usize,
    data: Vec<f32>,
    cancel: &'a AtomicBool,
    visits: usize,
}

impl<'a, R: FnMut(usize, &mut [f32]) -> Result<usize>> Samples<'a, R> {
    fn new(channels: usize, frames: usize, cancel: &'a AtomicBool, read: R) -> Result<Self> {
        ensure!(channels > 0, "Audio source has no channels");
        let frame_bytes = channels
            .checked_mul(4)
            .context("Audio frame size overflow")?;
        let block = (CACHE_BYTES / frame_bytes).max(1);
        let count = (block + 1)
            .checked_mul(channels)
            .context("Audio read size overflow")?;
        let mut data = Vec::new();
        data.try_reserve_exact(count)
            .context("Reserve audio analysis window")?;
        data.resize(count, 0.0);
        Ok(Self {
            read,
            channels,
            frames,
            block,
            first: 0,
            loaded: 0,
            data,
            cancel,
            visits: 0,
        })
    }

    fn integer(&mut self, frame: usize, channel: usize) -> Result<f32> {
        if frame < self.first || frame >= self.first + self.loaded {
            check_cancelled(self.cancel)?;
            self.first = frame / self.block * self.block;
            let count = (self.block + 1).min(self.frames - self.first);
            self.loaded = (self.read)(self.first, &mut self.data[..count * self.channels])?;
            ensure!(self.loaded == count, "Audio source changed during analysis");
        }
        Ok(self.data[(frame - self.first) * self.channels + channel])
    }

    fn sample(&mut self, frame: f64, channel: usize) -> Result<f32> {
        self.visits = self.visits.wrapping_add(1);
        if self.visits & 4095 == 0 {
            check_cancelled(self.cancel)?;
        }
        if !frame.is_finite() || frame < -1e-8 || self.frames == 0 {
            return Ok(0.0);
        }
        let frame = frame.max(0.0);
        let index = frame.floor() as usize;
        if index >= self.frames {
            return Ok(0.0);
        }
        let next = (index + 1).min(self.frames - 1);
        let a = self.integer(index, channel)?;
        let b = self.integer(next, channel)?;
        Ok(interpolate_sample(a, b, (frame - index as f64) as f32))
    }
}

/// The entire current clip source window determines normalization, independently
/// of a smaller transient/silence selection. No complete source is decoded.
pub fn normalization_gain_file(
    clip: &Clip,
    file: &AudioFile,
    tempo: f64,
    target_db: f32,
    cancel: &AtomicBool,
) -> Result<f32> {
    validate_clip(clip, tempo)?;
    ensure!(
        target_db.is_finite() && (-60.0..=0.0).contains(&target_db),
        "Invalid normalization target"
    );
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    let info = file.info();
    let frames_per_beat = f64::from(info.sample_rate) * 60.0 / tempo;
    let start = (clip.offset * frames_per_beat).floor().max(0.0) as usize;
    let end = ((clip.source_end() * frames_per_beat).ceil().max(0.0) as usize).min(info.frames);
    let mut reader = file.reader()?;
    let mut samples = Samples::new(info.channels, info.frames, cancel, |start, output| {
        reader.read_frames(start, output)
    })?;
    let mut peak = 0.0_f32;
    for frame in start..end {
        if frame & 4095 == 0 {
            check_cancelled(cancel)?;
        }
        for channel in 0..info.channels {
            peak = peak.max(samples.integer(frame, channel)?.abs());
        }
    }
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    ensure!(peak > 1e-12, "Cannot normalize a silent clip");
    Ok(10.0_f32.powf(target_db / 20.0) / peak)
}

/// The same 5 ms source-frame envelope and onset decisions as the resident
/// implementation, with bounded PCM and at most one million envelope values.
pub fn detect_transients_file(
    clip: &Clip,
    file: &AudioFile,
    tempo: f64,
    settings: TransientSettings,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    validate_clip(clip, tempo)?;
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    let info = file.info();
    let mut reader = file.reader()?;
    let mut samples = Samples::new(info.channels, info.frames, cancel, |start, output| {
        reader.read_frames(start, output)
    })?;
    let result = AudioAnalysis::new(
        clip,
        tempo,
        info.sample_rate,
        info.channels,
        |frame, channel| samples.sample(frame, channel),
    )?
    .transients(settings, || check_cancelled(cancel))?;
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    Ok(result)
}

/// Silence decisions retain playback-time thresholds, reverse/stretch lookup,
/// arrangement coordinates and the resident implementation's bounded results.
pub fn detect_audio_silence_file(
    clip: &Clip,
    file: &AudioFile,
    tempo: f64,
    range: (f64, f64),
    settings: AudioSilenceSettings,
    cancel: &AtomicBool,
) -> Result<Vec<(f64, f64)>> {
    validate_clip(clip, tempo)?;
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    let info = file.info();
    let mut reader = file.reader()?;
    let mut samples = Samples::new(info.channels, info.frames, cancel, |start, output| {
        reader.read_frames(start, output)
    })?;
    let result = AudioAnalysis::new(
        clip,
        tempo,
        info.sample_rate,
        info.channels,
        |frame, channel| samples.sample(frame, channel),
    )?
    .silence(range, settings, || check_cancelled(cancel))?;
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_samples_cancel_even_before_the_next_read_and_keep_neighbor_storage_bounded() {
        let cancel = AtomicBool::new(false);
        let mut reads = 0;
        let mut samples = Samples::new(2, 1_000_000, &cancel, |_, output: &mut [f32]| {
            reads += 1;
            output.fill(0.25);
            cancel.store(true, Ordering::Release);
            Ok(output.len() / 2)
        })
        .unwrap();
        assert!(samples.data.len() * size_of::<f32>() <= CACHE_BYTES + 2 * size_of::<f32>());
        assert_eq!(samples.sample(0.5, 0).unwrap(), 0.25);
        let failure = (0..4096).find_map(|_| samples.sample(0.5, 0).err());
        assert!(failure.unwrap().to_string().contains("cancelled"));
        assert_eq!(reads, 1, "A cached block must still observe cancellation");
    }
}
