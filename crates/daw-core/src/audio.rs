use anyhow::{Context, Result, ensure};
use std::path::Path;

impl crate::Project {
    /// Reject outputs that would replace audio used by this project, including
    /// inactive retained takes. File identity covers hard links and native path
    /// aliases in addition to canonical paths and symbolic links.
    pub fn validate_audio_destination(&self, destination: &Path) -> Result<()> {
        let canonical_destination = std::fs::canonicalize(destination).ok();
        let destination_file = same_file::Handle::from_path(destination).ok();
        for track in &self.tracks {
            for clip in &track.clips {
                let crate::ClipContent::Audio { path, .. } = &clip.content else {
                    continue;
                };
                let canonical_match = canonical_destination.as_ref().is_some_and(|destination| {
                    std::fs::canonicalize(path).is_ok_and(|source| source == *destination)
                });
                let identity_match = destination_file.as_ref().is_some_and(|destination| {
                    same_file::Handle::from_path(path).is_ok_and(|source| source == *destination)
                });
                ensure!(
                    !canonical_match && !identity_match,
                    "Choose a different output file; this destination is an audio source: {path}"
                );
            }
        }
        Ok(())
    }
}

/// Decoded, interleaved floating point samples. Decode outside the audio callback.
#[derive(Clone, Debug)]
pub struct AudioData {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl AudioData {
    /// Read a source's bounds without decoding or allocating PCM samples.
    pub fn wav_duration_seconds(path: impl AsRef<Path>) -> Result<f64> {
        let path = path.as_ref();
        let reader =
            hound::WavReader::open(path).with_context(|| format!("Open WAV {}", path.display()))?;
        let spec = reader.spec();
        ensure!(
            spec.channels > 0 && spec.sample_rate > 0,
            "WAV has invalid channel count or sample rate"
        );
        Ok(f64::from(reader.duration()) / f64::from(spec.sample_rate))
    }

    /// Decode at most 512 MiB of samples. Streaming long assets is a separate
    /// operation; malformed headers must not reserve unbounded process memory.
    pub fn load_wav(path: impl AsRef<Path>) -> Result<Self> {
        Self::load_wav_with_limit(path, 512 * 1024 * 1024)
    }

    /// Check declared sample count before collecting or allocating PCM samples.
    pub fn load_wav_with_limit(path: impl AsRef<Path>, max_decoded_bytes: usize) -> Result<Self> {
        let path = path.as_ref();
        let mut reader =
            hound::WavReader::open(path).with_context(|| format!("Open WAV {}", path.display()))?;
        let decoded_bytes = u64::from(reader.len()) * std::mem::size_of::<f32>() as u64;
        ensure!(
            decoded_bytes <= max_decoded_bytes as u64,
            "WAV exceeds decoded audio limit ({} MiB)",
            max_decoded_bytes / (1024 * 1024)
        );
        let spec = reader.spec();
        ensure!(
            spec.channels > 0 && spec.sample_rate > 0,
            "WAV has invalid channel count or sample rate"
        );
        let samples = match spec.sample_format {
            hound::SampleFormat::Float => {
                ensure!(
                    spec.bits_per_sample == 32,
                    "Only 32-bit floating point WAV is supported"
                );
                reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?
            }
            hound::SampleFormat::Int => {
                ensure!(
                    (1..=32).contains(&spec.bits_per_sample),
                    "Unsupported PCM bit depth"
                );
                let scale = 2.0_f64.powi(i32::from(spec.bits_per_sample) - 1);
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|value| (f64::from(value) / scale) as f32))
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        let audio = Self {
            sample_rate: spec.sample_rate,
            channels: usize::from(spec.channels),
            samples,
        };
        audio.validate()?;
        Ok(audio)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.sample_rate > 0, "Sample rate must be nonzero");
        ensure!(
            self.channels > 0 && self.channels <= usize::from(u16::MAX),
            "Invalid channel count"
        );
        ensure!(
            self.samples.len().is_multiple_of(self.channels),
            "Incomplete interleaved frame"
        );
        ensure!(
            self.samples.iter().all(|sample| sample.is_finite()),
            "Audio contains non-finite samples"
        );
        Ok(())
    }

    pub fn frames(&self) -> usize {
        self.samples.len().checked_div(self.channels).unwrap_or(0)
    }

    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / f64::from(self.sample_rate)
        }
    }

    /// Preserve floating point headroom; use a limiter or gain reduction when
    /// targeting integer formats downstream.
    pub fn write_wav(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let spec = hound::WavSpec {
            channels: self.channels as u16,
            sample_rate: self.sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(path, spec)?;
        for sample in &self.samples {
            writer.write_sample(*sample)?;
        }
        writer.finalize()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_sample_limit_is_checked_before_allocation_or_decode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("declared.wav");
        let mut header = Vec::new();
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&(36u32 + 1024 * 1024 * 1024).to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&48_000u32.to_le_bytes());
        header.extend_from_slice(&48_000u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&8u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&(1024u32 * 1024 * 1024).to_le_bytes());
        std::fs::write(&path, header).unwrap();
        assert!(
            AudioData::load_wav_with_limit(&path, 1024)
                .unwrap_err()
                .to_string()
                .contains("decoded audio limit")
        );
    }

    #[test]
    fn exact_decoded_limit_accepts_finite_audio_and_lower_limit_rejects_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("valid.wav");
        AudioData {
            sample_rate: 48_000,
            channels: 1,
            samples: vec![0.25; 8],
        }
        .write_wav(&path)
        .unwrap();
        assert_eq!(
            AudioData::load_wav_with_limit(&path, 32).unwrap().samples,
            vec![0.25; 8]
        );
        assert!(AudioData::load_wav_with_limit(&path, 31).is_err());
    }
}
