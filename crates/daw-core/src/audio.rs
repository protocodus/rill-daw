use anyhow::{Context, Result, ensure};
use std::path::Path;

/// Decoded, interleaved floating point samples. Decode outside the audio callback.
#[derive(Clone, Debug)]
pub struct AudioData {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl AudioData {
    pub fn load_wav(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut reader =
            hound::WavReader::open(path).with_context(|| format!("Open WAV {}", path.display()))?;
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
