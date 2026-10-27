//! Exact source extrema for progressive waveform drawing. Build away from the UI;
//! each visible pixel then visits logarithmically many peak bins and at most two
//! short source edges, regardless of the source's duration.
use crate::AudioData;
use anyhow::{Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};

const BLOCK_FRAMES: usize = 64;

#[derive(Clone, Copy, Debug)]
struct Peak {
    minimum: f32,
    maximum: f32,
}
impl Peak {
    const EMPTY: Self = Self {
        minimum: f32::INFINITY,
        maximum: f32::NEG_INFINITY,
    };
    fn add(&mut self, minimum: f32, maximum: f32) {
        self.minimum = self.minimum.min(minimum);
        self.maximum = self.maximum.max(maximum);
    }
}

#[derive(Debug)]
pub struct AudioWaveformPeaks {
    pub sample_rate: u32,
    pub source_frames: usize,
    /// Stereo display mapping, preserving each of the first two source channels.
    pub channels: usize,
    source_channels: usize,
    levels: Vec<Vec<Peak>>,
}

impl AudioWaveformPeaks {
    pub fn build(audio: &AudioData) -> Result<Self> {
        Self::build_cancellable(audio, &AtomicBool::new(false))
    }

    pub fn build_cancellable(audio: &AudioData, cancel: &AtomicBool) -> Result<Self> {
        ensure!(
            !cancel.load(Ordering::Relaxed),
            "Waveform refinement cancelled"
        );
        ensure!(
            audio.samples.len() <= 512 * 1024 * 1024 / size_of::<f32>(),
            "Waveform source exceeds the 512 MiB limit"
        );
        audio.validate()?;
        let channels = audio.channels.min(2);
        let source_frames = audio.frames();
        let mut base = Vec::with_capacity(source_frames.div_ceil(BLOCK_FRAMES) * channels);
        for start in (0..source_frames).step_by(BLOCK_FRAMES) {
            if start % 8192 == 0 {
                ensure!(
                    !cancel.load(Ordering::Relaxed),
                    "Waveform refinement cancelled"
                );
            }
            let end = (start + BLOCK_FRAMES).min(source_frames);
            for channel in 0..channels {
                let mut peak = Peak::EMPTY;
                for frame in start..end {
                    let value = audio.samples[frame * audio.channels + channel];
                    peak.add(value, value);
                }
                base.push(peak);
            }
        }
        let mut levels = vec![base];
        while levels.last().unwrap().len() > channels {
            ensure!(
                !cancel.load(Ordering::Relaxed),
                "Waveform refinement cancelled"
            );
            let previous = levels.last().unwrap();
            let bins = previous.len() / channels;
            let mut next = Vec::with_capacity(bins.div_ceil(2) * channels);
            for bin in (0..bins).step_by(2) {
                for channel in 0..channels {
                    let mut peak = previous[bin * channels + channel];
                    if bin + 1 < bins {
                        let other = previous[(bin + 1) * channels + channel];
                        peak.add(other.minimum, other.maximum);
                    }
                    next.push(peak);
                }
            }
            levels.push(next);
        }
        ensure!(
            !cancel.load(Ordering::Relaxed),
            "Waveform refinement cancelled"
        );
        Ok(Self {
            sample_rate: audio.sample_rate,
            source_frames,
            channels,
            source_channels: audio.channels,
            levels,
        })
    }

    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    pub fn byte_size(&self) -> usize {
        self.levels
            .iter()
            .map(|level| level.capacity() * size_of::<Peak>())
            .sum::<usize>()
            + self.levels.capacity() * size_of::<Vec<Peak>>()
    }

    /// Exact half-open source-frame range, clamped to the available source.
    /// PCM is borrowed only for the two partial edge blocks; it is never retained.
    pub fn range(
        &self,
        audio: &AudioData,
        start: usize,
        end: usize,
        channel: usize,
    ) -> Option<(f32, f32)> {
        if channel >= self.channels
            || audio.channels != self.source_channels
            || audio.sample_rate != self.sample_rate
            || audio.frames() != self.source_frames
        {
            return None;
        }
        let start = start.min(self.source_frames);
        let end = end.min(self.source_frames);
        if start >= end {
            return None;
        }
        let mut peak = Peak::EMPTY;
        let mut cursor = start;
        let first_full = start
            .div_ceil(BLOCK_FRAMES)
            .saturating_mul(BLOCK_FRAMES)
            .min(end);
        while cursor < first_full {
            let value = audio.samples[cursor * audio.channels + channel];
            peak.add(value, value);
            cursor += 1;
        }
        let end_full = end / BLOCK_FRAMES;
        let mut bin = cursor / BLOCK_FRAMES;
        while cursor.is_multiple_of(BLOCK_FRAMES) && bin < end_full {
            let remaining = end_full - bin;
            let largest = (usize::BITS - 1 - remaining.leading_zeros()) as usize;
            let aligned = if bin == 0 {
                largest
            } else {
                bin.trailing_zeros() as usize
            };
            let level = largest.min(aligned).min(self.levels.len() - 1);
            let value = self.levels[level][(bin >> level) * self.channels + channel];
            peak.add(value.minimum, value.maximum);
            bin += 1 << level;
            cursor = bin * BLOCK_FRAMES;
        }
        while cursor < end {
            let value = audio.samples[cursor * audio.channels + channel];
            peak.add(value, value);
            cursor += 1;
        }
        Some((peak.minimum, peak.maximum))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(frames: usize) -> AudioData {
        AudioData {
            sample_rate: 48000,
            channels: 2,
            samples: (0..frames)
                .flat_map(|frame| {
                    let value = ((frame * 73 % 997) as f32 - 498.0) / 499.0;
                    [value, value * -0.3]
                })
                .collect(),
        }
    }

    #[test]
    fn exact_ranges_match_every_sample_across_unaligned_edges_and_pyramid_levels() {
        let audio = source(8197);
        let peaks = AudioWaveformPeaks::build(&audio).unwrap();
        for start in (0..audio.frames()).step_by(37) {
            for length in [1, 2, 31, 63, 64, 65, 127, 997, 4097, 10000] {
                for channel in 0..2 {
                    let end = (start + length).min(audio.frames());
                    let mut reference = Peak::EMPTY;
                    for frame in start..end {
                        let value = audio.samples[frame * 2 + channel];
                        reference.add(value, value);
                    }
                    assert_eq!(
                        peaks.range(&audio, start, end, channel),
                        Some((reference.minimum, reference.maximum)),
                        "{start}..{end} channel {channel}"
                    );
                }
            }
        }
        assert!(peaks.range(&audio, 10, 10, 0).is_none());
        assert!(peaks.range(&audio, 0, 10, 2).is_none());
        assert!(peaks.range(&source(1), 0, 1, 0).is_none());
    }

    #[test]
    fn isolated_impulses_and_antiphase_channels_survive_whole_source_overview() {
        let mut audio = source(65539);
        audio.samples.fill(0.0);
        audio.samples[32771 * 2] = 1.0;
        audio.samples[32771 * 2 + 1] = -1.0;
        let peaks = AudioWaveformPeaks::build(&audio).unwrap();
        assert_eq!(peaks.range(&audio, 0, audio.frames(), 0), Some((0.0, 1.0)));
        assert_eq!(peaks.range(&audio, 0, audio.frames(), 1), Some((-1.0, 0.0)));
        assert!(peaks.byte_size() < audio.samples.len() * size_of::<f32>() / 8);
        assert!(AudioWaveformPeaks::build_cancellable(&audio, &AtomicBool::new(true)).is_err());
    }

    #[test]
    fn empty_and_short_sources_have_valid_bounded_queries() {
        for frames in [0, 1, 7, 63, 64, 65] {
            let audio = source(frames);
            let peaks = AudioWaveformPeaks::build(&audio).unwrap();
            assert_eq!(peaks.source_frames, frames);
            assert!(peaks.level_count() <= 2);
            assert_eq!(peaks.range(&audio, 0, 1, 0).is_some(), frames > 0);
        }
    }
}
