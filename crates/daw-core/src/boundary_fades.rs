//! Occurrence-local anti-click fades that leave musical source envelopes intact.
use crate::{AudioEnvelope, ClipContent, MAX_BEAT, Project};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A small linear envelope in source beats. Its anchor survives later trims and
/// splits, while source edits shared by ghosts leave this occurrence unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioBoundaryFades {
    pub offset: f64,
    pub length: f64,
    pub fade_in: f64,
    pub fade_out: f64,
}

impl AudioBoundaryFades {
    pub fn gain(self, source_position: f64) -> f32 {
        let position = source_position + self.offset;
        let attack = if self.fade_in > 0.0 {
            (position / self.fade_in).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let release = if self.fade_out > 0.0 {
            ((self.length - position) / self.fade_out).clamp(0.0, 1.0)
        } else {
            1.0
        };
        (attack * release) as f32
    }

    pub(crate) fn validate(self) -> Result<()> {
        ensure!(
            self.offset.is_finite()
                && self.offset.abs() <= MAX_BEAT
                && self.length.is_finite()
                && self.length > 0.0
                && self.length <= MAX_BEAT
                && self.fade_in.is_finite()
                && self.fade_in >= 0.0
                && self.fade_in <= self.length
                && self.fade_out.is_finite()
                && self.fade_out >= 0.0
                && self.fade_out <= self.length,
            "Invalid audio boundary fades"
        );
        Ok(())
    }
}

impl Project {
    /// Remove only local anti-click envelopes, preserving shared musical fades.
    pub fn clear_audio_boundary_fades(&mut self, clips: &[(u64, u64)]) -> Result<usize> {
        let mut planned = Vec::new();
        let mut seen = HashSet::new();
        for &(track, id) in clips {
            let clip = self.clip(track, id).context("Clip not found")?;
            ensure!(!clip.is_take_source(), "Choose a take before editing it");
            let ClipContent::Audio { boundary_fades, .. } = &clip.content else {
                anyhow::bail!("Select audio clips");
            };
            if boundary_fades.is_some() && seen.insert(id) {
                planned.push((track, id));
            }
        }
        let changed = planned.len();
        for (track, id) in planned {
            let ClipContent::Audio { boundary_fades, .. } =
                &mut self.clip_mut(track, id).unwrap().content
            else {
                unreachable!()
            };
            *boundary_fades = None;
        }
        Ok(changed)
    }

    /// Add minimum anti-click fades to these audible occurrences in one batch.
    /// This edits parameters only: original sources, shared musical fades and
    /// retained takes are untouched. Validation precedes all mutation.
    pub fn smooth_audio_boundaries(
        &mut self,
        clips: &[(u64, u64)],
        fade_in_ms: f64,
        fade_out_ms: f64,
    ) -> Result<usize> {
        ensure!(
            fade_in_ms.is_finite()
                && fade_out_ms.is_finite()
                && (0.0..=100.0).contains(&fade_in_ms)
                && (0.0..=100.0).contains(&fade_out_ms),
            "Boundary fades must be between 0 and 100 ms"
        );
        ensure!(
            self.tempo.is_finite() && (20.0..=400.0).contains(&self.tempo),
            "Invalid tempo"
        );
        if fade_in_ms == 0.0 && fade_out_ms == 0.0 {
            return Ok(0);
        }
        let mut seen = HashSet::new();
        let mut planned = Vec::with_capacity(clips.len());
        for &(track, id) in clips {
            let clip = self.clip(track, id).context("Clip not found")?;
            if !seen.insert(id) {
                continue;
            }
            ensure!(!clip.is_take_source(), "Choose a take before editing it");
            let ClipContent::Audio {
                fade_in,
                fade_out,
                envelope,
                boundary_fades,
                ..
            } = &clip.content
            else {
                anyhow::bail!("Select audio clips");
            };
            let length = clip.source_length();
            let region = envelope.unwrap_or(AudioEnvelope {
                offset: 0.0,
                length,
            });
            let requested_in =
                (fade_in_ms * self.tempo / 60_000.0 * clip.source_rate).min(length * 0.5);
            let requested_out =
                (fade_out_ms * self.tempo / 60_000.0 * clip.source_rate).min(length * 0.5);
            // Musical fades already reaching zero at this edge need no extra
            // multiplier. Keep their shape and duration audibly identical.
            let attack = if *fade_in >= requested_in && *fade_in > 0.0 && region.offset <= 0.0 {
                0.0
            } else {
                requested_in
            };
            let release = if *fade_out >= requested_out
                && *fade_out > 0.0
                && region.length - region.offset - length <= 1e-10
            {
                0.0
            } else {
                requested_out
            };
            // Trimming the opposite edge must not shorten an already longer
            // local fade at an edge that is still in the same place.
            let previous_in = boundary_fades
                .filter(|old| old.offset == 0.0)
                .map_or(0.0, |old| old.fade_in.min(length));
            let previous_out = boundary_fades
                .filter(|old| (old.length - old.offset - length).abs() <= 1e-10)
                .map_or(0.0, |old| old.fade_out.min(length));
            let value = AudioBoundaryFades {
                offset: 0.0,
                length,
                fade_in: attack.max(previous_in),
                fade_out: release.max(previous_out),
            };
            let value = (value.fade_in > 0.0 || value.fade_out > 0.0).then_some(value);
            if value != *boundary_fades {
                planned.push((track, id, value));
            }
        }
        let changed = planned.len();
        for (track, id, value) in planned {
            let ClipContent::Audio { boundary_fades, .. } = &mut self
                .clip_mut(track, id)
                .expect("prevalidated audio clip")
                .content
            else {
                unreachable!()
            };
            *boundary_fades = value;
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Clip, FadeCurve};

    fn project() -> (Project, u64, u64) {
        let mut project = Project::new();
        let track = project.add_audio_track("Audio");
        let clip = Clip::audio("Original", 0.0, 4.0, "source-does-not-need-decoding.wav");
        let id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        (project, track, id)
    }

    #[test]
    fn existing_musical_fades_at_current_edges_are_exactly_unchanged() {
        let (mut project, track, id) = project();
        project
            .set_audio_fades(
                track,
                id,
                1.0,
                0.5,
                FadeCurve::SCurve,
                FadeCurve::EqualPower,
            )
            .unwrap();
        let original = project.clone();
        assert_eq!(
            project
                .smooth_audio_boundaries(&[(track, id)], 3.0, 5.0)
                .unwrap(),
            0
        );
        assert_eq!(project, original);
    }

    #[test]
    fn inherited_musical_envelopes_and_middle_samples_survive_boundary_smoothing() {
        let (mut project, track, id) = project();
        project
            .set_audio_fades(
                track,
                id,
                2.0,
                2.0,
                FadeCurve::SCurve,
                FadeCurve::EqualPower,
            )
            .unwrap();
        let original = project
            .clip(track, id)
            .unwrap()
            .audio_region(1.0, 3.0)
            .unwrap();
        *project.clip_mut(track, id).unwrap() = original.clone();
        assert_eq!(
            project
                .smooth_audio_boundaries(&[(track, id), (track, id)], 3.0, 5.0)
                .unwrap(),
            1
        );
        let smoothed = project.clip(track, id).unwrap();
        for beat in [0.02, 0.5, 1.0, 1.98] {
            assert_eq!(
                original.audio_envelope_gain(beat),
                smoothed.audio_envelope_gain(beat)
            );
        }
        assert_eq!(smoothed.audio_envelope_gain(0.0), 0.0);
        assert_eq!(smoothed.audio_envelope_gain(smoothed.length), 0.0);
        let mut restored = smoothed.clone();
        if let ClipContent::Audio { boundary_fades, .. } = &mut restored.content {
            *boundary_fades = None;
        }
        assert_eq!(
            restored, original,
            "musical fade values, curves, source and anchors are intact"
        );
        assert_eq!(
            project
                .smooth_audio_boundaries(&[(track, id)], 3.0, 5.0)
                .unwrap(),
            0
        );
        project.validate().unwrap();
    }

    #[test]
    fn ghost_source_edits_preserve_local_boundaries_without_changing_other_occurrences() {
        let (mut project, track, id) = project();
        let ghost = project.clone_clip(track, id).unwrap();
        let unselected = project.clip(track, ghost).unwrap().clone();
        project
            .smooth_audio_boundaries(&[(track, id)], 3.0, 5.0)
            .unwrap();
        assert_eq!(project.clip(track, ghost).unwrap(), &unselected);
        let boundary = match project.clip(track, id).unwrap().content {
            ClipContent::Audio { boundary_fades, .. } => boundary_fades,
            _ => unreachable!(),
        };
        project.set_audio_gain(track, ghost, 0.75).unwrap();
        assert!(matches!(project.clip(track,id).unwrap().content,
            ClipContent::Audio { boundary_fades, gain:0.75, .. } if boundary_fades == boundary));
        assert!(matches!(
            project.clip(track, ghost).unwrap().content,
            ClipContent::Audio {
                boundary_fades: None,
                ..
            }
        ));
        project.validate().unwrap();
    }

    #[test]
    fn splitting_trimming_and_stretching_keep_the_boundary_anchor() {
        let (mut project, track, id) = project();
        project
            .smooth_audio_boundaries(&[(track, id)], 100.0, 100.0)
            .unwrap();
        let original = project.clip(track, id).unwrap().clone();
        let right = project.split_clip(track, id, 1.0).unwrap();
        for beat in [0.0, 0.05, 0.5, 0.99, 1.0, 1.05, 3.9, 3.99] {
            let (clip, local) = if beat < 1.0 {
                (project.clip(track, id).unwrap(), beat)
            } else {
                (project.clip(track, right).unwrap(), beat - 1.0)
            };
            assert_eq!(
                clip.audio_envelope_gain(local),
                original.audio_envelope_gain(beat)
            );
        }
        project.trim_clip(track, right, 1.5, 2.5).unwrap();
        let trimmed = project.clip(track, right).unwrap();
        assert_eq!(
            trimmed.audio_envelope_gain(2.45),
            original.audio_envelope_gain(3.95)
        );
        project.stretch_clip(track, right, 5.0).unwrap();
        let stretched = project.clip(track, right).unwrap();
        assert_eq!(
            stretched.audio_envelope_gain(4.9),
            original.audio_envelope_gain(3.95)
        );
        project.validate().unwrap();
    }

    #[test]
    fn tiny_clips_are_bounded_invalid_batches_are_atomic_and_legacy_json_roundtrips() {
        let (mut project, track, id) = project();
        project.clip_mut(track, id).unwrap().length = 0.001;
        let legacy = serde_json::to_string(&project).unwrap();
        assert!(!legacy.contains("boundary_fades"));
        assert_eq!(serde_json::from_str::<Project>(&legacy).unwrap(), project);
        let original = project.clone();
        assert!(
            project
                .smooth_audio_boundaries(&[(track, id), (track, u64::MAX)], 3.0, 5.0)
                .is_err()
        );
        assert_eq!(project, original);
        assert!(
            project
                .smooth_audio_boundaries(&[(track, id)], f64::NAN, 5.0)
                .is_err()
        );
        assert_eq!(project, original);
        project
            .smooth_audio_boundaries(&[(track, id)], 100.0, 100.0)
            .unwrap();
        assert!(matches!(project.clip(track,id).unwrap().content,
            ClipContent::Audio { boundary_fades:Some(value), .. }
                if value.fade_in == 0.0005 && value.fade_out == 0.0005));
        project.validate().unwrap();
        let encoded = serde_json::to_string(&project).unwrap();
        assert_eq!(serde_json::from_str::<Project>(&encoded).unwrap(), project);
    }

    #[test]
    fn take_selection_preserves_boundary_origins_and_retained_sources() {
        let (mut project, track, id) = project();
        project
            .smooth_audio_boundaries(&[(track, id)], 100.0, 100.0)
            .unwrap();
        let original = project.clip(track, id).unwrap().clone();
        let group = project.group_audio_takes(track, &[id]).unwrap();
        let retained = project.clip(track, id).unwrap().clone();
        let region = project
            .select_audio_take(track, group, id, Some((1.0, 3.0)), 0.0)
            .unwrap();
        for beat in [0.0, 0.1, 1.0, 1.99] {
            assert_eq!(
                project
                    .clip(track, region)
                    .unwrap()
                    .audio_envelope_gain(beat),
                original.audio_envelope_gain(beat + 1.0)
            );
        }
        project
            .smooth_audio_boundaries(&[(track, region)], 3.0, 5.0)
            .unwrap();
        assert_eq!(project.clip(track, id).unwrap(), &retained);
        let edited = project.clone();
        assert!(
            project
                .smooth_audio_boundaries(&[(track, region), (track, id)], 3.0, 5.0)
                .is_err()
        );
        assert_eq!(project, edited);
        project.validate().unwrap();
    }

    #[test]
    fn boundary_anchor_overflow_rejects_transient_batch_before_any_split() {
        let (mut project, track, id) = project();
        if let ClipContent::Audio { boundary_fades, .. } =
            &mut project.clip_mut(track, id).unwrap().content
        {
            *boundary_fades = Some(AudioBoundaryFades {
                offset: MAX_BEAT - 0.5,
                length: 4.0,
                fade_in: 0.1,
                fade_out: 0.1,
            });
        }
        project.validate().unwrap();
        let before = project.clone();
        assert!(
            project
                .split_audio_transients(track, id, &[0.25, 0.75])
                .is_err()
        );
        assert_eq!(project, before);
    }

    #[test]
    fn smoothing_after_a_trim_preserves_the_longer_fade_at_the_untouched_edge() {
        for trim_left in [false, true] {
            let (mut project, track, id) = project();
            project
                .smooth_audio_boundaries(&[(track, id)], 100.0, 100.0)
                .unwrap();
            project
                .trim_clip(track, id, if trim_left { 1.0 } else { 0.0 }, 3.0)
                .unwrap();
            let before = project.clip(track, id).unwrap().clone();
            project
                .smooth_audio_boundaries(&[(track, id)], 3.0, 5.0)
                .unwrap();
            let after = project.clip(track, id).unwrap();
            for step in 0..100 {
                let local = if trim_left {
                    2.8 + step as f64 * 0.002
                } else {
                    step as f64 * 0.002
                };
                assert!(
                    (after.audio_envelope_gain(local) - before.audio_envelope_gain(local)).abs()
                        < 1e-6
                );
            }
            project.validate().unwrap();
        }
    }
}
