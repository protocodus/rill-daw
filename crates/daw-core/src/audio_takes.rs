//! Alternate recordings stay in the document as silent source candidates.
//! A comp consists of ordinary editable audio clips referring to those candidates;
//! no rendered or destructively modified source is needed to choose a passage.
use crate::{
    AudioCrossfade, AudioEnvelope, AudioSchedule, Clip, ClipContent, CrossfadeCurve, MAX_BEAT,
    Project, TrackKind, new_id,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const MAX_AUDIO_TAKES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTakeMembership {
    pub group: u64,
    pub role: AudioTakeRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioTakeRole {
    /// Retained, silent candidate, including its original placement and edits.
    Source,
    /// Audible, independently editable region derived from this source clip ID.
    Comp { source: u64 },
}

impl Clip {
    pub fn is_take_source(&self) -> bool {
        matches!(
            self.audio_take,
            Some(AudioTakeMembership {
                role: AudioTakeRole::Source,
                ..
            })
        )
    }

    pub fn take_group(&self) -> Option<u64> {
        self.audio_take.map(|take| take.group)
    }
}

impl Project {
    /// The first supplied clip becomes the initial audible take. Source candidates
    /// retain their bounds and edits; comp edits never propagate back to them.
    pub fn group_audio_takes(&mut self, track: u64, ids: &[u64]) -> Result<u64> {
        self.validate()?;
        let selected: HashSet<_> = ids.iter().copied().collect();
        ensure!(
            (1..=MAX_AUDIO_TAKES).contains(&ids.len()) && selected.len() == ids.len(),
            "Select between 1 and {MAX_AUDIO_TAKES} distinct audio clips"
        );
        let owner = self.track(track).context("Track not found")?;
        ensure!(
            owner.kind == TrackKind::Audio,
            "Takes require an audio track"
        );
        let mut clips = owner.clips.clone();
        for id in ids {
            let clip = clips
                .iter()
                .find(|clip| clip.id == *id)
                .context("Take clip not found")?;
            ensure!(
                matches!(clip.content, ClipContent::Audio { .. }),
                "Select audio clips"
            );
            ensure!(
                clip.audio_take.is_none(),
                "Clip already belongs to a take group"
            );
        }
        let group = new_id();
        for clip in clips.iter_mut().filter(|clip| selected.contains(&clip.id)) {
            clip.audio_take = Some(AudioTakeMembership {
                group,
                role: AudioTakeRole::Source,
            });
            clip.link_group = None;
        }
        let initial = clips
            .iter()
            .find(|clip| clip.id == ids[0])
            .expect("validated source");
        let comp = comp_from_source(initial, group, initial.start, initial.end());
        clips.push(comp);
        self.commit_take_clips(track, clips)?;
        Ok(group)
    }

    /// Retain another imported or existing clip without replacing the current comp.
    pub fn add_audio_take(&mut self, track: u64, group: u64, clip: u64) -> Result<()> {
        self.validate()?;
        let sources = self.audio_take_sources(track, group);
        ensure!(!sources.is_empty(), "Take group not found");
        ensure!(
            sources.len() < MAX_AUDIO_TAKES,
            "Too many takes in this group"
        );
        let mut clips = self.track(track).context("Track not found")?.clips.clone();
        let source = clips
            .iter_mut()
            .find(|item| item.id == clip)
            .context("Take clip not found")?;
        ensure!(
            matches!(source.content, ClipContent::Audio { .. }),
            "Select an audio clip"
        );
        ensure!(
            source.audio_take.is_none(),
            "Clip already belongs to a take group"
        );
        source.link_group = None;
        source.audio_take = Some(AudioTakeMembership {
            group,
            role: AudioTakeRole::Source,
        });
        self.commit_take_clips(track, clips)
    }

    pub fn audio_take_sources(&self, track: u64, group: u64) -> Vec<&Clip> {
        self.track(track)
            .into_iter()
            .flat_map(|track| &track.clips)
            .filter(|clip| clip.take_group() == Some(group) && clip.is_take_source())
            .collect()
    }

    pub fn audio_take_regions(&self, track: u64, group: u64) -> Vec<&Clip> {
        self.track(track)
            .into_iter()
            .flat_map(|track| &track.clips)
            .filter(|clip| clip.take_group() == Some(group) && !clip.is_take_source())
            .collect()
    }

    /// Choose an entire take, or replace an absolute arrangement-beat range.
    /// Boundary crossfades use real source handles and a smooth complementary
    /// curve. With no handle, the edit stays at the requested beat without a gap.
    pub fn select_audio_take(
        &mut self,
        track: u64,
        group: u64,
        source: u64,
        range: Option<(f64, f64)>,
        crossfade_beats: f64,
    ) -> Result<u64> {
        self.validate()?;
        ensure!(
            crossfade_beats.is_finite() && (0.0..=MAX_BEAT).contains(&crossfade_beats),
            "Invalid take crossfade duration"
        );
        let owner = self.track(track).context("Track not found")?;
        let source = owner
            .clips
            .iter()
            .find(|clip| {
                clip.id == source && clip.take_group() == Some(group) && clip.is_take_source()
            })
            .context("Retained take not found")?;
        let (start, end) = range.unwrap_or((source.start, source.end()));
        ensure!(
            start.is_finite()
                && end.is_finite()
                && start >= source.start
                && end <= source.end()
                && end > start,
            "Selected range must lie inside the take"
        );
        let selected = comp_from_source(source, group, start, end);
        let selected_id = selected.id;
        let mut regions = Vec::new();
        if range.is_some() {
            let old: Vec<_> = self
                .audio_take_regions(track, group)
                .into_iter()
                .cloned()
                .collect();
            // Recover logical regions rather than preserving old crossfade tails
            // as new independent pieces each time a passage is replaced.
            let schedule = AudioSchedule::new(&old)?;
            let mut spans: Vec<(usize, f64, f64)> = Vec::new();
            for segment in schedule.segments {
                if let Some(last) = spans.last_mut()
                    && last.0 == segment.incoming
                    && last.2 == segment.start
                {
                    last.2 = segment.end;
                } else {
                    spans.push((segment.incoming, segment.start, segment.end));
                }
            }
            let mut used_ids = HashSet::new();
            for (index, left, right) in spans {
                for (a, b) in [(left, right.min(start)), (left.max(end), right)] {
                    if b <= a {
                        continue;
                    }
                    let mut kept = slice_audio(&old[index], a, b);
                    if !used_ids.insert(kept.id) {
                        kept.id = new_id();
                    }
                    regions.push(kept);
                }
            }
        }
        regions.push(selected);
        regions.sort_by(|a, b| a.start.total_cmp(&b.start));
        ensure!(regions.len() <= 16_384, "Too many regions in this comp");
        let sources: HashMap<_, _> = self
            .audio_take_sources(track, group)
            .into_iter()
            .map(|clip| (clip.id, clip))
            .collect();
        for index in 0..regions.len() {
            let at_new_boundary = regions[index].id == selected_id
                || (range.is_some() && regions[index].start == end);
            if at_new_boundary {
                set_crossfade(
                    &mut regions[index],
                    (crossfade_beats > 0.0).then_some(AudioCrossfade {
                        duration: crossfade_beats,
                        curve: CrossfadeCurve::SMOOTH,
                    }),
                );
            }
            let Some(config) = crossfade(&regions[index]) else {
                continue;
            };
            if index == 0 || regions[index - 1].end() != regions[index].start {
                set_crossfade(&mut regions[index], None);
                continue;
            }
            let previous = &regions[index - 1];
            let original = previous.audio_take.and_then(|take| match take.role {
                AudioTakeRole::Comp { source } => sources.get(&source).copied(),
                AudioTakeRole::Source => None,
            });
            let handle = original.map_or(0.0, |source| source_handle(previous, source));
            let duration = config
                .duration
                .min(handle)
                .min(regions[index].length)
                .min(MAX_BEAT - previous.end());
            if duration <= 0.0 {
                set_crossfade(&mut regions[index], None);
                continue;
            }
            let previous = &mut regions[index - 1];
            if matches!(previous.content, ClipContent::Audio { reverse: true, .. }) {
                previous.offset = (previous.offset - duration * previous.source_rate).max(0.0);
            }
            previous.length += duration;
            // Store the actual handle-clamped duration so the UI agrees with playback.
            set_crossfade(
                &mut regions[index],
                Some(AudioCrossfade { duration, ..config }),
            );
        }
        let mut clips = owner.clips.clone();
        clips.retain(|clip| clip.take_group() != Some(group) || clip.is_take_source());
        clips.extend(regions);
        self.commit_take_clips(track, clips)?;
        Ok(selected_id)
    }

    /// Discard retained alternatives while keeping the current editable comp.
    /// No audio is rendered and no source files are deleted from disk.
    pub fn detach_audio_take_group(&mut self, track: u64, group: u64) -> Result<()> {
        self.validate()?;
        ensure!(
            !self.audio_take_sources(track, group).is_empty(),
            "Take group not found"
        );
        let mut clips = self.track(track).context("Track not found")?.clips.clone();
        clips.retain(|clip| clip.take_group() != Some(group) || !clip.is_take_source());
        for clip in &mut clips {
            if clip.take_group() == Some(group) {
                clip.audio_take = None;
            }
        }
        self.commit_take_clips(track, clips)
    }

    fn commit_take_clips(&mut self, track: u64, clips: Vec<Clip>) -> Result<()> {
        let previous = std::mem::replace(
            &mut self.track_mut(track).context("Track not found")?.clips,
            clips,
        );
        if let Err(error) = self.validate() {
            self.track_mut(track).expect("existing track").clips = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn validate_audio_takes(&self) -> Result<()> {
        let mut owners = HashMap::new();
        for track in &self.tracks {
            let sources: HashMap<_, _> = track
                .clips
                .iter()
                .filter(|clip| clip.is_take_source())
                .map(|clip| (clip.id, clip))
                .collect();
            let mut counts = HashMap::new();
            for clip in &track.clips {
                let Some(take) = clip.audio_take else {
                    continue;
                };
                ensure!(
                    track.kind == TrackKind::Audio
                        && matches!(clip.content, ClipContent::Audio { .. }),
                    "Only audio clips may belong to a take group"
                );
                ensure!(
                    *owners.entry(take.group).or_insert(track.id) == track.id,
                    "A take group cannot span multiple tracks"
                );
                match take.role {
                    AudioTakeRole::Source => {
                        ensure!(
                            clip.link_group.is_none(),
                            "Retained takes cannot share ghost edits"
                        );
                        *counts.entry(take.group).or_insert(0_usize) += 1;
                    }
                    AudioTakeRole::Comp { source } => {
                        ensure!(
                            sources
                                .get(&source)
                                .is_some_and(|clip| clip.take_group() == Some(take.group)),
                            "Comp region refers to a missing take source"
                        );
                    }
                }
            }
            ensure!(
                counts.values().all(|count| *count <= MAX_AUDIO_TAKES),
                "Too many takes in a group"
            );
        }
        Ok(())
    }
}

fn comp_from_source(source: &Clip, group: u64, start: f64, end: f64) -> Clip {
    let mut clip = slice_audio(source, start, end);
    clip.id = new_id();
    clip.link_group = None;
    clip.audio_take = Some(AudioTakeMembership {
        group,
        role: AudioTakeRole::Comp { source: source.id },
    });
    set_crossfade(&mut clip, None);
    clip
}

/// Slice from arrangement coordinates, keeping source and fade origins exact for
/// both reverse and stretched clips. Crossfades belong only to their original edge.
fn slice_audio(source: &Clip, start: f64, end: f64) -> Clip {
    let mut clip = source.clone();
    let delta = start - source.start;
    let length = end - start;
    clip.offset += if matches!(source.content, ClipContent::Audio { reverse: true, .. }) {
        (source.length - delta - length) * source.source_rate
    } else {
        delta * source.source_rate
    };
    if let ClipContent::Audio {
        envelope,
        boundary_fades,
        ..
    } = &mut clip.content
    {
        let original = envelope.unwrap_or(AudioEnvelope {
            offset: 0.0,
            length: source.source_length(),
        });
        *envelope = Some(AudioEnvelope {
            offset: original.offset + delta * source.source_rate,
            ..original
        });
        if let Some(boundary) = boundary_fades {
            boundary.offset += delta * source.source_rate;
        }
    }
    clip.start = start;
    clip.length = length;
    if start != source.start {
        set_crossfade(&mut clip, None);
    }
    clip
}

fn crossfade(clip: &Clip) -> Option<AudioCrossfade> {
    if let ClipContent::Audio { crossfade, .. } = clip.content {
        crossfade
    } else {
        None
    }
}

fn set_crossfade(clip: &mut Clip, config: Option<AudioCrossfade>) {
    if let ClipContent::Audio { crossfade, .. } = &mut clip.content {
        *crossfade = config;
    }
}

fn source_handle(clip: &Clip, source: &Clip) -> f64 {
    let (ClipContent::Audio { path, reverse, .. }, ClipContent::Audio { path: original, .. }) =
        (&clip.content, &source.content)
    else {
        return 0.0;
    };
    if path != original {
        return 0.0;
    }
    let available = if *reverse {
        clip.offset - source.offset
    } else {
        source.source_end() - clip.source_end()
    };
    (available / clip.source_rate).max(0.0)
}
