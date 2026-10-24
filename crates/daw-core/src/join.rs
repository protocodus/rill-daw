//! Consolidation keeps source windows and musical timing explicit. Audio is
//! rendered by the engine; the model only validates and replaces clip regions.
use crate::{Clip, ClipContent, MAX_BEAT, Note, Project, TrackKind};
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct AudioJoinSelection {
    pub start: f64,
    pub length: f64,
    /// Original storage order is significant for equal-start audio priority.
    pub clips: Vec<Clip>,
}

impl Project {
    fn join_selection(
        &self,
        track: u64,
        ids: &[u64],
        kind: TrackKind,
    ) -> Result<(f64, f64, Vec<&Clip>)> {
        let selected: HashSet<_> = ids.iter().copied().collect();
        ensure!(
            selected.len() >= 2 && selected.len() == ids.len(),
            "Select at least two distinct clips"
        );
        let track = self.track(track).context("Track not found")?;
        ensure!(
            track.kind == kind,
            "Select clips on a matching audio or MIDI track"
        );
        let clips: Vec<_> = track
            .clips
            .iter()
            .filter(|clip| selected.contains(&clip.id))
            .collect();
        ensure!(clips.len() == ids.len(), "Selected clip no longer exists");
        for clip in &clips {
            ensure!(
                matches!(
                    (&clip.content, kind),
                    (ClipContent::Audio { .. }, TrackKind::Audio)
                        | (ClipContent::Midi { .. }, TrackKind::Midi)
                ),
                "Join clips of the same type"
            );
            ensure!(
                clip.start.is_finite()
                    && clip.start >= 0.0
                    && clip.length.is_finite()
                    && clip.length > 0.0
                    && clip.end() <= MAX_BEAT
                    && clip.offset.is_finite()
                    && clip.offset >= 0.0,
                "Invalid clip range"
            );
        }
        let start = clips
            .iter()
            .map(|clip| clip.start)
            .fold(f64::INFINITY, f64::min);
        let end = clips.iter().map(|clip| clip.end()).fold(0.0, f64::max);
        if kind == TrackKind::Audio {
            // One ordinary joined region cannot preserve separate priority
            // against a third, unselected region inside its new span (even gaps).
            ensure!(
                !track.clips.iter().any(|clip| !selected.contains(&clip.id)
                    && clip.start < end
                    && clip.end() > start),
                "Include overlapping clips before joining"
            );
        }
        Ok((start, end, clips))
    }

    pub fn audio_join_range(&self, track: u64, ids: &[u64]) -> Result<(f64, f64)> {
        let (start, end, _) = self.join_selection(track, ids, TrackKind::Audio)?;
        Ok((start, end))
    }

    pub fn audio_join_selection(&self, track: u64, ids: &[u64]) -> Result<AudioJoinSelection> {
        let (start, end, clips) = self.join_selection(track, ids, TrackKind::Audio)?;
        Ok(AudioJoinSelection {
            start,
            length: end - start,
            clips: clips.into_iter().cloned().collect(),
        })
    }

    pub fn join_midi_clips(&mut self, track: u64, ids: &[u64]) -> Result<u64> {
        let (start, end, clips) = self.join_selection(track, ids, TrackKind::Midi)?;
        let mut joined_notes = Vec::new();
        for clip in clips {
            let ClipContent::Midi { notes } = &clip.content else {
                unreachable!()
            };
            for note in notes {
                let left = note.start.max(clip.offset);
                let right = (note.start + note.length).min(clip.offset + clip.length);
                if right > left {
                    joined_notes.push(Note::new(
                        note.pitch,
                        (clip.start + left - clip.offset - start).max(0.0),
                        right - left,
                        note.velocity,
                    ));
                }
            }
        }
        joined_notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        let joined = Clip::midi("Joined MIDI", start, end - start, joined_notes);
        Ok(self.replace_join_selection(track, ids, joined))
    }

    /// Commit only after successfully writing the new source. Callers performing
    /// asynchronous work must first check their source clips/tempo still match.
    pub fn replace_audio_clips_with_joined(
        &mut self,
        track: u64,
        ids: &[u64],
        path: impl Into<String>,
    ) -> Result<u64> {
        let (start, end) = self.audio_join_range(track, ids)?;
        let path = path.into();
        ensure!(!path.trim().is_empty(), "Joined audio source path is empty");
        let joined = Clip::audio("Joined audio", start, end - start, path);
        Ok(self.replace_join_selection(track, ids, joined))
    }

    fn replace_join_selection(&mut self, track: u64, ids: &[u64], joined: Clip) -> u64 {
        let selected: HashSet<_> = ids.iter().copied().collect();
        let track = self
            .track_mut(track)
            .expect("join selection validated track");
        let index = track
            .clips
            .iter()
            .position(|clip| selected.contains(&clip.id))
            .expect("join selection validated clips");
        track.clips.retain(|clip| !selected.contains(&clip.id));
        let id = joined.id;
        track.clips.insert(index, joined);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_join_clips_source_windows_preserves_absolute_timing_and_refreshes_every_id() {
        let mut project = Project::new();
        let track = project.add_midi_track("MIDI");
        let mut a = Clip::midi(
            "Trimmed",
            4.0,
            2.0,
            vec![
                Note::new(60, 0.0, 2.0, 90),
                Note::new(62, 2.5, 2.0, 100),
                Note::new(63, 4.0, 1.0, 110),
            ],
        );
        a.offset = 1.0;
        let b = Clip::midi("Later", 8.0, 1.0, vec![Note::new(67, 0.25, 2.0, 75)]);
        let untouched = Clip::midi("Outside", 12.0, 1.0, vec![]);
        let ids = [a.id, b.id];
        let old_notes: HashSet<_> = [&a, &b]
            .into_iter()
            .flat_map(|clip| match &clip.content {
                ClipContent::Midi { notes } => notes.iter().map(|note| note.id).collect::<Vec<_>>(),
                _ => unreachable!(),
            })
            .collect();
        project.track_mut(track).unwrap().clips = vec![a, untouched.clone(), b];
        let id = project.join_midi_clips(track, &ids).unwrap();
        assert!(!ids.contains(&id));
        let joined = project.clip(track, id).unwrap();
        assert_eq!(
            (joined.start, joined.length, joined.offset),
            (4.0, 5.0, 0.0)
        );
        let ClipContent::Midi { notes } = &joined.content else {
            unreachable!()
        };
        assert_eq!(
            notes
                .iter()
                .map(|n| (n.pitch, n.start, n.length, n.velocity))
                .collect::<Vec<_>>(),
            vec![
                (60, 0.0, 1.0, 90),
                (62, 1.5, 0.5, 100),
                (67, 4.25, 0.75, 75)
            ]
        );
        assert!(notes.iter().all(|note| !old_notes.contains(&note.id)));
        assert_eq!(project.track(track).unwrap().clips[1], untouched);
        project.validate().unwrap();
    }

    #[test]
    fn audio_join_preserves_tie_order_and_rejects_unselected_overlap_without_mutation() {
        let mut project = Project::new();
        let track = project.add_audio_track("Audio");
        let a = Clip::audio("A", 2.0, 4.0, "a.wav");
        let b = Clip::audio("B", 3.0, 1.0, "b.wav");
        let c = Clip::audio("C", 3.0, 2.0, "c.wav");
        let ids = [a.id, b.id, c.id];
        project.track_mut(track).unwrap().clips = vec![a, b, c];
        let before = project.clone();
        assert!(
            project
                .audio_join_range(track, &ids[..2])
                .unwrap_err()
                .to_string()
                .contains("Include overlapping")
        );
        assert!(
            project
                .replace_audio_clips_with_joined(track, &ids[..2], "out.wav")
                .is_err()
        );
        assert_eq!(project, before);
        let selection = project
            .audio_join_selection(track, &[ids[2], ids[0], ids[1]])
            .unwrap();
        assert_eq!(
            selection
                .clips
                .iter()
                .map(|clip| clip.id)
                .collect::<Vec<_>>(),
            ids
        );
        let id = project
            .replace_audio_clips_with_joined(track, &ids, "joined.wav")
            .unwrap();
        let joined = project.clip(track, id).unwrap();
        assert_eq!(
            (joined.start, joined.length, joined.offset),
            (2.0, 4.0, 0.0)
        );
        assert!(matches!(
            joined.content,
            ClipContent::Audio {
                gain: 1.0,
                fade_in: 0.0,
                fade_out: 0.0,
                reverse: false,
                ..
            }
        ));
        project.validate().unwrap();
    }

    #[test]
    fn invalid_selection_never_mutates_the_document() {
        let mut project = Project::new();
        let track = project.add_midi_track("MIDI");
        let clip = Clip::midi("A", 0.0, 1.0, vec![]);
        let id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        let before = project.clone();
        for ids in [vec![], vec![id], vec![id, id], vec![id, u64::MAX]] {
            assert!(project.join_midi_clips(track, &ids).is_err());
            assert_eq!(project, before);
        }
    }
}
