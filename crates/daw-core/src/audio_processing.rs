//! Non-destructive source processing shared by linked audio clips.
use crate::{ClipContent, Project};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioProcessingAlgorithm {
    #[default]
    Signalsmith,
    Bungee,
}

impl AudioProcessingAlgorithm {
    pub const ALL: [Self; 2] = [Self::Signalsmith, Self::Bungee];

    pub fn name(self) -> &'static str {
        match self {
            Self::Signalsmith => "Signalsmith",
            Self::Bungee => "Bungee",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioProcessing {
    #[serde(default)]
    pub algorithm: AudioProcessingAlgorithm,
    /// Independent pitch change; duration and the original audio stay intact.
    #[serde(default)]
    pub transpose_semitones: f64,
}

impl AudioProcessing {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.transpose_semitones.is_finite()
                && (-24.0..=24.0).contains(&self.transpose_semitones),
            "Audio transpose must be between -24 and +24 semitones"
        );
        Ok(())
    }
}

impl Project {
    /// Ghosts share processing with their source. Arrangement duration and
    /// stretch ratio stay local to each occurrence, including after this edit.
    pub fn set_audio_processing(
        &mut self,
        track: u64,
        clip: u64,
        processing: AudioProcessing,
    ) -> Result<()> {
        processing.validate()?;
        ensure!(
            !self
                .clip(track, clip)
                .context("Clip not found")?
                .is_take_source(),
            "Choose a take before editing it"
        );
        let content = &mut self
            .clip_mut(track, clip)
            .context("Clip not found")?
            .content;
        let ClipContent::Audio {
            processing: value, ..
        } = content
        else {
            anyhow::bail!("Select an audio clip");
        };
        *value = processing;
        self.synchronize_linked_clip(track, clip)
    }
}
