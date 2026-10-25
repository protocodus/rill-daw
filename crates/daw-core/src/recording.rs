//! Persistent input channel assignments and atomic publication of recorded audio.
//! Device ownership, arming, monitoring, disk capture and transport stay outside
//! the document model. A project never starts listening merely because it opens.
use crate::{Clip, ClipContent, MAX_AUDIO_TAKES, Project, TrackKind};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

/// Zero-based channels of the input device chosen in the application's setup.
/// Stereo channels may be non-adjacent, but must represent distinct inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioInputChannels {
    Mono { channel: u16 },
    Stereo { left: u16, right: u16 },
}

impl Default for AudioInputChannels {
    fn default() -> Self {
        Self::Mono { channel: 0 }
    }
}

impl AudioInputChannels {
    pub fn channel_count(self) -> u16 {
        match self {
            Self::Mono { .. } => 1,
            Self::Stereo { .. } => 2,
        }
    }

    /// Validate again against the actual device whenever an input stream opens.
    /// Saved projects validate only representable channel indices; a different
    /// machine can have a device with fewer inputs without corrupting the file.
    pub fn validate(self, device_channels: u16) -> Result<()> {
        match self {
            Self::Mono { channel } => ensure!(
                channel < device_channels,
                "Input channel {} is unavailable on a {device_channels}-channel device",
                u32::from(channel) + 1
            ),
            Self::Stereo { left, right } => {
                ensure!(left != right, "Stereo input channels must be distinct");
                ensure!(
                    left < device_channels && right < device_channels,
                    "Stereo input channels {} and {} are unavailable on a {device_channels}-channel device",
                    u32::from(left) + 1,
                    u32::from(right) + 1
                );
            }
        }
        Ok(())
    }
}

/// A finalized, independently recorded file represented in arrangement beats.
/// A completed loop pass covers its full loop; a stopped partial pass remains
/// available as an alternative and never hides a longer complete pass by default.
#[derive(Clone, Debug)]
pub struct RecordedAudioPass {
    pub clip: Clip,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordingPlacement {
    /// Place audio on the armed channel, using its normal exclusive clip priority.
    #[default]
    Take,
    /// Create a separate audio channel so existing audio and the new layer sum.
    Overdub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordedAudioCommit {
    pub track: u64,
    pub clip: u64,
    pub group: Option<u64>,
}

impl Project {
    pub fn set_audio_input(&mut self, track: u64, input: Option<AudioInputChannels>) -> Result<()> {
        let owner = self.track_mut(track).context("Recording track not found")?;
        ensure!(
            owner.kind == TrackKind::Audio,
            "Only audio tracks have inputs"
        );
        if let Some(input) = input {
            input.validate(u16::MAX)?;
        }
        owner.audio_input = input;
        Ok(())
    }

    /// Publish a finalized recording in one transaction. Existing clips and take
    /// groups remain unchanged. Files have already been finalized by the capture
    /// worker; this operation never rewrites or removes them, including on error.
    /// Callers can bracket it with one ordinary project-history checkpoint.
    pub fn insert_recorded_audio(
        &mut self,
        track: u64,
        passes: Vec<RecordedAudioPass>,
        placement: RecordingPlacement,
        loop_recording: bool,
    ) -> Result<RecordedAudioCommit> {
        self.validate()?;
        let owner = self.track(track).context("Recording track not found")?;
        ensure!(owner.kind == TrackKind::Audio, "Record onto an audio track");
        ensure!(
            (1..=MAX_AUDIO_TAKES).contains(&passes.len()),
            "A recording must contain between 1 and {MAX_AUDIO_TAKES} passes"
        );
        ensure!(
            loop_recording || passes.len() == 1,
            "Separate recording passes require loop recording"
        );
        for pass in &passes {
            ensure!(
                matches!(pass.clip.content, ClipContent::Audio { .. }),
                "A recording pass must contain audio"
            );
            ensure!(
                pass.clip.audio_take.is_none() && pass.clip.link_group.is_none(),
                "A recording pass must be an independent clip"
            );
        }
        let selected = passes
            .iter()
            .rposition(|pass| pass.complete)
            .unwrap_or_else(|| {
                // A recording can start midway through a loop and stop before any
                // full pass is completed. Prefer the greatest coverage, latest on ties.
                passes
                    .iter()
                    .enumerate()
                    .max_by(|(a_index, a), (b_index, b)| {
                        a.clip
                            .length
                            .total_cmp(&b.clip.length)
                            .then(a_index.cmp(b_index))
                    })
                    .map(|(index, _)| index)
                    .expect("nonempty recording")
            });
        let selected_source = passes[selected].clip.id;
        let mut next = self.clone();
        let target = if placement == RecordingPlacement::Overdub {
            let name = format!("{} · overdub", owner.name);
            let id = next.add_audio_track(name);
            let mut layer = next.tracks.pop().expect("new overdub channel");
            layer.output_bus = owner.output_bus;
            layer.parent_folder = owner.parent_folder;
            layer.audio_input = owner.audio_input;
            layer.gain = owner.gain;
            layer.pan = owner.pan;
            let position = next
                .tracks
                .iter()
                .position(|track| track.id == owner.id)
                .expect("recording channel");
            next.tracks.insert(position + 1, layer);
            id
        } else {
            track
        };
        let ids: Vec<_> = std::iter::once(selected_source)
            .chain(
                passes
                    .iter()
                    .filter(|pass| pass.clip.id != selected_source)
                    .map(|pass| pass.clip.id),
            )
            .collect();
        next.track_mut(target)
            .expect("recording target")
            .clips
            .extend(passes.into_iter().map(|pass| pass.clip));
        // Validate all pass bounds, IDs, paths and processing before grouping.
        next.validate()?;
        let group = if loop_recording {
            Some(next.group_audio_takes(target, &ids)?)
        } else {
            None
        };
        let clip = group.map_or(selected_source, |group| {
            next.audio_take_regions(target, group)[0].id
        });
        next.validate()?;
        *self = next;
        Ok(RecordedAudioCommit {
            track: target,
            clip,
            group,
        })
    }
}

/// MIDI channel selection. None on Track means no MIDI input; channel numbers
/// here are zero-based, while the UI displays the conventional 1 through 16.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiInputChannel {
    #[default]
    All,
    Channel {
        channel: u8,
    },
}
impl MidiInputChannel {
    pub fn validate(self) -> Result<()> {
        if let Self::Channel { channel } = self {
            ensure!(channel < 16, "MIDI input channel must be 1 through 16");
        }
        Ok(())
    }
    pub fn channel(self) -> Option<u8> {
        match self {
            Self::All => None,
            Self::Channel { channel } => Some(channel),
        }
    }
}
impl Project {
    /// MIDI recordings add one independent clip, retaining all existing events.
    /// Unlike exclusive audio clips, overlapping MIDI clips play together.
    pub fn insert_recorded_midi(&mut self, track: u64, clip: Clip) -> Result<u64> {
        self.validate()?;
        ensure!(
            self.track(track).context("Recording track not found")?.kind == TrackKind::Midi,
            "Record MIDI onto a MIDI track"
        );
        ensure!(
            matches!(&clip.content, ClipContent::Midi { notes } if !notes.is_empty()),
            "A MIDI recording must contain notes"
        );
        ensure!(
            clip.link_group.is_none() && clip.audio_take.is_none(),
            "A MIDI recording must be independent"
        );
        let id = clip.id;
        let mut next = self.clone();
        next.track_mut(track).unwrap().clips.push(clip);
        next.validate()?;
        *self = next;
        Ok(id)
    }
}
