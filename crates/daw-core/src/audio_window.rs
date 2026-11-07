//! Bounded source-window analysis. PCM reads retain absolute source coordinates,
//! including a neighbouring frame for the resident sampler's interpolation.
use crate::{
    AudioFile, AudioSilenceSettings, Clip, ClipContent, MAX_BEAT, MAX_SOURCE_RATE, MIN_SOURCE_RATE,
    TransientAlgorithm, TransientSettings,
};
use anyhow::{Context, Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};

const CACHE_BYTES: usize = 256 * 1024;

fn check_cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "Audio analysis cancelled");
    Ok(())
}

fn validate_clip(clip: &Clip, tempo: f64) -> Result<()> {
    ensure!(
        matches!(clip.content, ClipContent::Audio { .. }),
        "Select an audio clip"
    );
    ensure!(
        tempo.is_finite()
            && tempo > 0.0
            && clip.start.is_finite()
            && clip.start >= 0.0
            && clip.length.is_finite()
            && clip.length > 0.0
            && clip.end() > clip.start
            && clip.end() <= MAX_BEAT
            && clip.offset.is_finite()
            && clip.offset >= 0.0
            && clip.source_rate.is_finite()
            && (MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&clip.source_rate)
            && clip.source_end() > clip.offset
            && clip.source_end() <= MAX_BEAT,
        "Invalid audio timing"
    );
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
        Ok(a + (b - a) * (frame - index as f64) as f32)
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
    ensure!(
        settings.sensitivity.is_finite()
            && (0.0..=1.0).contains(&settings.sensitivity)
            && settings.minimum_gap_ms.is_finite()
            && (20.0..=2000.0).contains(&settings.minimum_gap_ms),
        "Invalid transient settings"
    );
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    let info = file.info();
    let frames_per_beat = f64::from(info.sample_rate) * 60.0 / tempo;
    let frames = (clip.source_length() * frames_per_beat).ceil() as usize;
    let window = (f64::from(info.sample_rate) * 0.005 * clip.source_rate)
        .round()
        .max(1.0) as usize;
    let count = frames.div_ceil(window);
    ensure!(
        count <= 1_000_000,
        "Clip is too long for transient analysis"
    );
    let mut reader = file.reader()?;
    let mut samples = Samples::new(info.channels, info.frames, cancel, |start, output| {
        reader.read_frames(start, output)
    })?;
    let mut levels = Vec::new();
    levels
        .try_reserve_exact(count)
        .context("Reserve transient envelope")?;
    for block in 0..count {
        check_cancelled(cancel)?;
        let mut sum = 0.0_f64;
        let mut peak = 0.0_f32;
        let length = window.min(frames - block * window);
        for frame in 0..length {
            let beat = (block * window + frame) as f64 / frames_per_beat / clip.source_rate;
            let source = clip.audio_source_frame(beat, tempo, info.sample_rate);
            let mut level = 0.0_f32;
            for channel in 0..info.channels {
                level = level.max(samples.sample(source, channel)?.abs());
            }
            peak = peak.max(level);
            sum += f64::from(level) * f64::from(level);
        }
        levels.push(match settings.algorithm {
            TransientAlgorithm::EnergyRise => (sum / length.max(1) as f64).sqrt() as f32,
            TransientAlgorithm::AmplitudeGate => peak,
        });
    }
    file.check_unchanged()?;
    let peak = levels.iter().copied().fold(0.0_f32, f32::max);
    if peak <= 1e-8 {
        return Ok(Vec::new());
    }
    let floor = peak * (0.04 + 0.46 * (1.0 - settings.sensitivity));
    let minimum_gap = settings.minimum_gap_ms as f64 * 0.001 * tempo / 60.0;
    let mut result = Vec::new();
    let mut previous = 0.0_f32;
    let mut baseline = 0.0_f32;
    let mut armed = true;
    for (index, level) in levels.iter().copied().enumerate() {
        if index & 4095 == 0 {
            check_cancelled(cancel)?;
        }
        let attack = match settings.algorithm {
            TransientAlgorithm::AmplitudeGate => armed && level >= floor,
            TransientAlgorithm::EnergyRise => {
                level >= floor
                    && level > previous * (1.15 + (1.0 - settings.sensitivity) * 1.2)
                    && level > baseline * (1.1 + (1.0 - settings.sensitivity))
            }
        };
        let beat = (index * window) as f64 / frames_per_beat / clip.source_rate;
        if attack
            && beat > 1e-9
            && beat < clip.length - 1e-9
            && result
                .last()
                .is_none_or(|last: &f64| beat - *last >= minimum_gap)
        {
            ensure!(
                result.len() < 2048,
                "Too many transients; increase the minimum gap"
            );
            result.push(beat);
        }
        if level >= floor {
            armed = false;
        }
        if level < floor * 0.4 {
            armed = true;
        }
        baseline = baseline * 0.9 + level * 0.1;
        previous = level;
    }
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
    clip.audio_region(range.0, range.1)?;
    validate_clip(clip, tempo)?;
    ensure!(
        settings.threshold_db.is_finite() && (-96.0..=0.0).contains(&settings.threshold_db),
        "Threshold must be between -96 and 0 dBFS"
    );
    ensure!(
        settings.minimum_silence_ms.is_finite()
            && (1.0..=10000.0).contains(&settings.minimum_silence_ms),
        "Minimum silence must be between 1 and 10000 ms"
    );
    ensure!(
        settings.padding_ms.is_finite() && (0.0..=1000.0).contains(&settings.padding_ms),
        "Padding must be between 0 and 1000 ms"
    );
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    let info = file.info();
    let source_frames_per_beat = f64::from(info.sample_rate) * 60.0 / tempo * clip.source_rate;
    let frames = ((range.1 - range.0) * source_frames_per_beat).ceil();
    ensure!(
        frames <= 134_217_728.0,
        "Audio selection is too long for silence analysis"
    );
    let frames = frames as usize;
    let minimum = (f64::from(settings.minimum_silence_ms)
        * 0.001
        * f64::from(info.sample_rate)
        * clip.source_rate)
        .ceil()
        .max(1.0) as usize;
    let padding = f64::from(settings.padding_ms) * 0.001 * tempo / 60.0;
    let threshold = 10_f32.powf(settings.threshold_db / 20.0);
    let mut reader = file.reader()?;
    let mut samples = Samples::new(info.channels, info.frames, cancel, |start, output| {
        reader.read_frames(start, output)
    })?;
    let mut quiet = None;
    let mut gaps = Vec::new();
    for frame in 0..=frames {
        let below = if frame == frames {
            false
        } else {
            let beat = range.0 - clip.start + frame as f64 / source_frames_per_beat;
            let source = clip.audio_source_frame(beat, tempo, info.sample_rate);
            let mut below = true;
            for channel in 0..info.channels {
                let value = samples.sample(source, channel)?.abs();
                if value.is_nan() || value >= threshold {
                    below = false;
                    break;
                }
            }
            below
        };
        if below {
            quiet.get_or_insert(frame);
        } else if let Some(start) = quiet.take()
            && frame - start >= minimum
        {
            let left = if start == 0 {
                range.0
            } else {
                range.0 + start as f64 / source_frames_per_beat + padding
            };
            let right = if frame == frames {
                range.1
            } else {
                range.0 + frame as f64 / source_frames_per_beat - padding
            };
            if right > left {
                ensure!(
                    gaps.len() < 2048,
                    "Too many silent regions; increase Minimum silence"
                );
                gaps.push((left, right));
            }
        }
    }
    check_cancelled(cancel)?;
    file.check_unchanged()?;
    let mut retained = Vec::new();
    let mut start = range.0;
    for (left, right) in gaps {
        if left > start {
            retained.push((start, left));
        }
        start = right;
    }
    if start < range.1 {
        retained.push((start, range.1));
    }
    Ok(retained)
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
