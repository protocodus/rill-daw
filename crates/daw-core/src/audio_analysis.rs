//! Shared source analysis for resident and streamed audio. Storage adapters
//! provide interpolated samples; timing, bounds and detection decisions live here.
use crate::{
    AudioSilenceSettings, Clip, ClipContent, MAX_BEAT, MAX_SOURCE_RATE, MIN_SOURCE_RATE,
    TransientAlgorithm, TransientSettings,
};
use anyhow::{Context, Result, ensure};

pub(crate) fn validate_clip(clip: &Clip, tempo: f64) -> Result<()> {
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

/// Keep ordinary PCM interpolation unchanged while preventing the difference
/// between opposite finite extremes from overflowing before interpolation.
pub(crate) fn interpolate_sample(a: f32, b: f32, fraction: f32) -> f32 {
    let difference = b - a;
    if difference.is_finite() {
        a + difference * fraction
    } else {
        (f64::from(a) + (f64::from(b) - f64::from(a)) * f64::from(fraction)) as f32
    }
}

pub(crate) struct AudioAnalysis<'a, S> {
    clip: &'a Clip,
    tempo: f64,
    sample_rate: u32,
    channels: usize,
    sample: S,
}

impl<'a, S: FnMut(f64, usize) -> Result<f32>> AudioAnalysis<'a, S> {
    pub(crate) fn new(
        clip: &'a Clip,
        tempo: f64,
        sample_rate: u32,
        channels: usize,
        sample: S,
    ) -> Result<Self> {
        validate_clip(clip, tempo)?;
        ensure!(sample_rate > 0 && channels > 0, "Invalid audio format");
        ensure!(
            (f64::from(sample_rate) * 60.0 / tempo * clip.source_rate).is_finite(),
            "Invalid audio timing"
        );
        Ok(Self {
            clip,
            tempo,
            sample_rate,
            channels,
            sample,
        })
    }

    pub(crate) fn transients(
        &mut self,
        settings: TransientSettings,
        mut check_cancelled: impl FnMut() -> Result<()>,
    ) -> Result<Vec<f64>> {
        ensure!(
            settings.sensitivity.is_finite()
                && (0.0..=1.0).contains(&settings.sensitivity)
                && settings.minimum_gap_ms.is_finite()
                && (20.0..=2000.0).contains(&settings.minimum_gap_ms),
            "Invalid transient settings"
        );
        let frames_per_beat = f64::from(self.sample_rate) * 60.0 / self.tempo;
        // Inspect every source frame even for a compressed clip: output-rate
        // sampling would skip narrow transients when source_rate exceeds one.
        let frames = (self.clip.source_length() * frames_per_beat).ceil() as usize;
        let window = (f64::from(self.sample_rate) * 0.005 * self.clip.source_rate)
            .round()
            .max(1.0) as usize;
        let count = frames.div_ceil(window);
        ensure!(
            count <= 1_000_000,
            "Clip is too long for transient analysis"
        );
        let mut levels = Vec::new();
        levels
            .try_reserve_exact(count)
            .context("Reserve transient envelope")?;
        for block in 0..count {
            check_cancelled()?;
            let mut sum = 0.0_f64;
            let mut peak = 0.0_f32;
            let length = window.min(frames - block * window);
            for frame in 0..length {
                let beat =
                    (block * window + frame) as f64 / frames_per_beat / self.clip.source_rate;
                let source = self
                    .clip
                    .audio_source_frame(beat, self.tempo, self.sample_rate);
                let mut level = 0.0_f32;
                for channel in 0..self.channels {
                    level = level.max((self.sample)(source, channel)?.abs());
                }
                peak = peak.max(level);
                sum += f64::from(level) * f64::from(level);
            }
            levels.push(match settings.algorithm {
                TransientAlgorithm::EnergyRise => (sum / length.max(1) as f64).sqrt() as f32,
                TransientAlgorithm::AmplitudeGate => peak,
            });
        }
        let peak = levels.iter().copied().fold(0.0_f32, f32::max);
        if peak <= 1e-8 {
            return Ok(Vec::new());
        }
        let floor = peak * (0.04 + 0.46 * (1.0 - settings.sensitivity));
        let minimum_gap = settings.minimum_gap_ms as f64 * 0.001 * self.tempo / 60.0;
        let mut result = Vec::new();
        let mut previous = 0.0_f32;
        let mut baseline = 0.0_f32;
        let mut armed = true;
        for (index, level) in levels.iter().copied().enumerate() {
            if index & 4095 == 0 {
                check_cancelled()?;
            }
            let attack = match settings.algorithm {
                TransientAlgorithm::AmplitudeGate => armed && level >= floor,
                TransientAlgorithm::EnergyRise => {
                    level >= floor
                        && level > previous * (1.15 + (1.0 - settings.sensitivity) * 1.2)
                        && level > baseline * (1.1 + (1.0 - settings.sensitivity))
                }
            };
            let beat = (index * window) as f64 / frames_per_beat / self.clip.source_rate;
            if attack
                && beat > 1e-9
                && beat < self.clip.length - 1e-9
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

    pub(crate) fn silence(
        &mut self,
        range: (f64, f64),
        settings: AudioSilenceSettings,
        mut check_cancelled: impl FnMut() -> Result<()>,
    ) -> Result<Vec<(f64, f64)>> {
        self.clip.audio_region(range.0, range.1)?;
        settings.validate()?;
        let source_frames_per_beat =
            f64::from(self.sample_rate) * 60.0 / self.tempo * self.clip.source_rate;
        let frames = ((range.1 - range.0) * source_frames_per_beat).ceil();
        ensure!(
            frames <= 134_217_728.0,
            "Audio selection is too long for silence analysis"
        );
        let frames = frames as usize;
        let minimum = (f64::from(settings.minimum_silence_ms)
            * 0.001
            * f64::from(self.sample_rate)
            * self.clip.source_rate)
            .ceil()
            .max(1.0) as usize;
        let padding = f64::from(settings.padding_ms) * 0.001 * self.tempo / 60.0;
        let threshold = 10_f32.powf(settings.threshold_db / 20.0);
        let mut quiet = None;
        let mut gaps = Vec::new();
        for frame in 0..=frames {
            if frame & 4095 == 0 {
                check_cancelled()?;
            }
            let below = if frame == frames {
                false
            } else {
                let beat = range.0 - self.clip.start + frame as f64 / source_frames_per_beat;
                let source = self
                    .clip
                    .audio_source_frame(beat, self.tempo, self.sample_rate);
                let mut below = true;
                for channel in 0..self.channels {
                    let value = (self.sample)(source, channel)?.abs();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_keeps_extreme_finite_samples_and_endpoints_finite() {
        for (a, b) in [(f32::MAX, -f32::MAX), (-f32::MAX, f32::MAX)] {
            assert_eq!(interpolate_sample(a, b, 0.0), a);
            assert_eq!(interpolate_sample(a, b, 1.0), b);
            assert_eq!(interpolate_sample(a, b, 0.5), 0.0);
            for step in 0..=100 {
                let sample = interpolate_sample(a, b, step as f32 / 100.0);
                assert!(sample.is_finite());
                assert!((a.min(b)..=a.max(b)).contains(&sample));
            }
        }
    }
}
