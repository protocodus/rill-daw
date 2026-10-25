//! Exclusive audio takes and optional entry crossfades, compiled in musical time.
use crate::{Clip, ClipContent, MAX_BEAT, Project};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossfadeCurve {
    pub control1: [f32; 2],
    pub control2: [f32; 2],
}
impl Default for CrossfadeCurve {
    fn default() -> Self {
        Self::LINEAR
    }
}
impl CrossfadeCurve {
    pub const LINEAR: Self = Self {
        control1: [1.0 / 3.0, 1.0 / 3.0],
        control2: [2.0 / 3.0, 2.0 / 3.0],
    };
    pub const SMOOTH: Self = Self {
        control1: [1.0 / 3.0, 0.0],
        control2: [2.0 / 3.0, 1.0],
    };
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.control1
                .into_iter()
                .chain(self.control2)
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
                && self.control1[0] <= self.control2[0],
            "Invalid crossfade curve handles"
        );
        Ok(())
    }
    /// Incoming amplitude at normalized time. Endpoints are exact. Control x
    /// coordinates describe time, so evaluate y after inverting the cubic x.
    pub fn value(self, progress: f64) -> f32 {
        let x = if progress.is_finite() {
            progress.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if x == 0.0 || x == 1.0 {
            return x as f32;
        }
        if self.control1[0] == self.control1[1] && self.control2[0] == self.control2[1] {
            return x as f32;
        }
        let cubic = |t: f64, a: f32, b: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * f64::from(a) + 3.0 * u * t * t * f64::from(b) + t * t * t
        };
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..32 {
            let t = (low + high) * 0.5;
            if cubic(t, self.control1[0], self.control2[0]) < x {
                low = t;
            } else {
                high = t;
            }
        }
        cubic((low + high) * 0.5, self.control1[1], self.control2[1]).clamp(0.0, 1.0) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioCrossfade {
    /// Requested beats; scheduling clamps this without changing the stored value.
    pub duration: f64,
    pub curve: CrossfadeCurve,
}
impl AudioCrossfade {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.duration.is_finite() && self.duration > 0.0 && self.duration <= MAX_BEAT,
            "Invalid crossfade duration"
        );
        self.curve.validate()
    }
    pub fn gains(self, progress: f64) -> [f32; 2] {
        let incoming = self.curve.value(progress);
        [1.0 - incoming, incoming]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrossfadeWindow {
    /// Indices into the original track.clips vector, never sorted/reassigned IDs.
    pub incoming: usize,
    pub outgoing: usize,
    pub start: f64,
    pub end: f64,
    pub requested_duration: f64,
    pub curve: CrossfadeCurve,
}
impl CrossfadeWindow {
    pub fn gains(self, beat: f64) -> [f32; 2] {
        let incoming = self
            .curve
            .value((beat - self.start) / (self.end - self.start));
        [1.0 - incoming, incoming]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioSegment {
    pub start: f64,
    pub end: f64,
    pub incoming: usize,
    pub outgoing: Option<usize>,
    pub crossfade: Option<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioBlend {
    pub incoming: usize,
    pub outgoing: Option<usize>,
    pub incoming_gain: f32,
    pub outgoing_gain: f32,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioSchedule {
    pub segments: Vec<AudioSegment>,
    pub crossfades: Vec<CrossfadeWindow>,
    pub overlaps: Vec<AudioOverlap>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioOverlap {
    pub incoming: usize,
    pub outgoing: usize,
    pub start: f64,
    pub end: f64,
}
impl AudioSchedule {
    /// Latest-starting active audio clip wins; equal starts use the later vector
    /// entry. On its end, the highest-priority remaining take resumes immediately.
    /// Crossfades apply only at actual clip starts, never to resumed older takes.
    pub fn new(clips: &[Clip]) -> Result<Self> {
        let mut events = Vec::with_capacity(clips.len() * 2);
        for (index, clip) in clips.iter().enumerate() {
            if clip.is_take_source() {
                continue;
            }
            if let ClipContent::Audio { crossfade, .. } = clip.content {
                ensure!(
                    clip.start.is_finite()
                        && clip.start >= 0.0
                        && clip.length.is_finite()
                        && clip.length > 0.0
                        && clip.end() <= MAX_BEAT,
                    "Invalid audio clip bounds"
                );
                if let Some(crossfade) = crossfade {
                    crossfade.validate()?;
                }
                events.push((clip.start, true, index));
                events.push((clip.end(), false, index));
            }
        }
        events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let key = |index: usize| ((clips[index].start + 0.0).to_bits(), index);
        let mut active = BTreeSet::new();
        let mut raw: Vec<AudioSegment> = Vec::new();
        let mut cursor = 0;
        while cursor < events.len() {
            let beat = events[cursor].0;
            while cursor < events.len() && events[cursor].0 == beat {
                let (_, begins, index) = events[cursor];
                if begins {
                    active.insert(key(index));
                } else {
                    active.remove(&key(index));
                }
                cursor += 1;
            }
            if let (Some((_, winner)), Some(next)) = (active.last(), events.get(cursor)) {
                if let Some(previous) = raw.last_mut()
                    && previous.incoming == *winner
                    && previous.end == beat
                {
                    previous.end = next.0;
                } else {
                    raw.push(AudioSegment {
                        start: beat,
                        end: next.0,
                        incoming: *winner,
                        outgoing: None,
                        crossfade: None,
                    });
                }
            }
        }
        let mut schedule = Self::default();
        for (index, mut segment) in raw.iter().copied().enumerate() {
            let incoming = &clips[segment.incoming];
            if segment.start == incoming.start
                && let Some(previous) = index.checked_sub(1).and_then(|i| raw.get(i))
                && previous.end == segment.start
                && clips[previous.incoming].end() > segment.start
            {
                let available_end = segment.end.min(clips[previous.incoming].end());
                schedule.overlaps.push(AudioOverlap {
                    incoming: segment.incoming,
                    outgoing: previous.incoming,
                    start: segment.start,
                    end: available_end,
                });
                if let ClipContent::Audio {
                    crossfade: Some(config),
                    ..
                } = incoming.content
                {
                    let end = (segment.start + config.duration).min(available_end);
                    if end > segment.start {
                        let window = CrossfadeWindow {
                            incoming: segment.incoming,
                            outgoing: previous.incoming,
                            start: segment.start,
                            end,
                            requested_duration: config.duration,
                            curve: config.curve,
                        };
                        let window_index = schedule.crossfades.len();
                        schedule.crossfades.push(window);
                        schedule.segments.push(AudioSegment {
                            end,
                            outgoing: Some(previous.incoming),
                            crossfade: Some(window_index),
                            ..segment
                        });
                        segment.start = end;
                    }
                }
            }
            if segment.end > segment.start {
                schedule.segments.push(segment);
            }
        }
        Ok(schedule)
    }
    pub fn segment_at(&self, beat: f64) -> Option<&AudioSegment> {
        if !beat.is_finite() {
            return None;
        }
        let index = self.segments.partition_point(|segment| segment.end <= beat);
        self.segments
            .get(index)
            .filter(|segment| segment.start <= beat)
    }
    pub fn sample(&self, beat: f64) -> Option<AudioBlend> {
        let segment = self.segment_at(beat)?;
        let [outgoing_gain, incoming_gain] = segment
            .crossfade
            .map_or([0.0, 1.0], |index| self.crossfades[index].gains(beat));
        Some(AudioBlend {
            incoming: segment.incoming,
            outgoing: segment.outgoing,
            incoming_gain,
            outgoing_gain,
        })
    }
}
impl Project {
    pub fn set_audio_crossfade(
        &mut self,
        track: u64,
        clip: u64,
        crossfade: Option<AudioCrossfade>,
    ) -> Result<()> {
        if let Some(value) = crossfade {
            value.validate()?;
        }
        ensure!(
            !self
                .clip(track, clip)
                .context("Clip not found")?
                .is_take_source(),
            "Choose a take before editing it"
        );
        let ClipContent::Audio {
            crossfade: destination,
            ..
        } = &mut self
            .clip_mut(track, clip)
            .context("Clip not found")?
            .content
        else {
            anyhow::bail!("Crossfades require an audio clip");
        };
        *destination = crossfade;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clip(start: f64, end: f64) -> Clip {
        Clip::audio("Take", start, end - start, "fixture.wav")
    }
    fn crossfade(clip: &mut Clip, duration: f64, curve: CrossfadeCurve) {
        if let ClipContent::Audio { crossfade, .. } = &mut clip.content {
            *crossfade = Some(AudioCrossfade { duration, curve });
        }
    }
    #[test]
    fn latest_start_wins_equal_starts_use_vector_order_and_older_takes_resume() {
        let clips = vec![
            clip(0.0, 8.0),
            clip(2.0, 6.0),
            clip(3.0, 4.0),
            clip(2.0, 5.0),
        ];
        let schedule = AudioSchedule::new(&clips).unwrap();
        for (beat, winner) in [
            (0.0, 0),
            (2.0, 3),
            (3.0, 2),
            (4.0, 3),
            (5.0, 1),
            (6.0, 0),
            (7.999, 0),
        ] {
            let sample = schedule.sample(beat).unwrap();
            assert_eq!(sample.incoming, winner);
            assert_eq!(sample.outgoing, None);
            assert_eq!(sample.incoming_gain, 1.0);
        }
        assert!(schedule.sample(-1.0).is_none());
        assert!(schedule.sample(8.0).is_none());
        let mut reordered = clips.clone();
        reordered.swap(1, 3);
        assert_eq!(
            AudioSchedule::new(&reordered)
                .unwrap()
                .sample(2.5)
                .unwrap()
                .incoming,
            3
        );
        assert_eq!(
            reordered[3].id, clips[1].id,
            "Priority follows vector order, not generated IDs"
        );
        let gap = AudioSchedule::new(&[clip(0.0, 1.0), clip(2.0, 3.0)]).unwrap();
        assert!(gap.sample(1.5).is_none());
    }
    #[test]
    fn crossfades_end_before_takeover_and_never_add_a_third_source_or_fade_resumed_takes() {
        let mut clips = vec![clip(0.0, 8.0), clip(2.0, 6.0), clip(3.0, 4.0)];
        crossfade(&mut clips[1], 4.0, CrossfadeCurve::LINEAR);
        crossfade(&mut clips[2], 0.75, CrossfadeCurve::SMOOTH);
        let schedule = AudioSchedule::new(&clips).unwrap();
        assert_eq!(schedule.crossfades.len(), 2);
        assert_eq!(schedule.crossfades[0].end, 3.0);
        assert_eq!(schedule.crossfades[0].requested_duration, 4.0);
        assert_eq!(
            schedule.sample(2.5).unwrap(),
            AudioBlend {
                incoming: 1,
                outgoing: Some(0),
                incoming_gain: 0.5,
                outgoing_gain: 0.5
            }
        );
        let second = schedule.sample(3.375).unwrap();
        assert_eq!((second.incoming, second.outgoing), (2, Some(1)));
        assert!((second.incoming_gain - 0.5).abs() < 1e-6);
        assert_eq!(schedule.sample(4.0).unwrap().outgoing, None);
        assert_eq!(schedule.sample(4.0).unwrap().incoming, 1);
        for step in 0..8000 {
            let mix = schedule.sample(f64::from(step) / 1000.0).unwrap();
            assert!((mix.incoming_gain + mix.outgoing_gain - 1.0).abs() < 1e-6);
        }
    }
    #[test]
    fn entry_candidates_exist_without_configuration_and_clip_ends_bound_the_fade() {
        let mut clips = vec![clip(0.0, 2.5), clip(2.0, 4.0), clip(2.0, 3.0)];
        let without = AudioSchedule::new(&clips).unwrap();
        assert_eq!(
            without.overlaps,
            vec![AudioOverlap {
                incoming: 2,
                outgoing: 0,
                start: 2.0,
                end: 2.5
            }]
        );
        assert!(without.crossfades.is_empty());
        crossfade(&mut clips[2], 1.0, CrossfadeCurve::LINEAR);
        let with = AudioSchedule::new(&clips).unwrap();
        assert_eq!(with.crossfades[0].end, 2.5);
        assert_eq!(with.sample(2.25).unwrap().outgoing, Some(0));
        assert_eq!(with.sample(3.0).unwrap().incoming, 1);
    }
    #[test]
    fn curve_validation_endpoints_and_serialized_model_are_backwards_compatible() {
        for curve in [
            CrossfadeCurve::LINEAR,
            CrossfadeCurve::SMOOTH,
            CrossfadeCurve {
                control1: [0.1, 0.9],
                control2: [0.9, 0.1],
            },
        ] {
            curve.validate().unwrap();
            assert_eq!(curve.value(0.0), 0.0);
            assert_eq!(curve.value(1.0), 1.0);
            for step in 0..=100 {
                assert!((0.0..=1.0).contains(&curve.value(f64::from(step) / 100.0)));
            }
        }
        let mut project = Project::new();
        let track = project.add_audio_track("Audio");
        let clip = clip(1.0, 3.0);
        let id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        let before = project.clone();
        for config in [
            AudioCrossfade {
                duration: f64::NAN,
                curve: CrossfadeCurve::LINEAR,
            },
            AudioCrossfade {
                duration: 1.0,
                curve: CrossfadeCurve {
                    control1: [0.9, 0.0],
                    control2: [0.1, 1.0],
                },
            },
        ] {
            assert!(
                project
                    .set_audio_crossfade(track, id, Some(config))
                    .is_err()
            );
            assert_eq!(project, before);
        }
        let mut history = crate::History::new();
        history.commit_named(&project, "Edit crossfade");
        project
            .set_audio_crossfade(
                track,
                id,
                Some(AudioCrossfade {
                    duration: 8.0,
                    curve: CrossfadeCurve::SMOOTH,
                }),
            )
            .unwrap();
        let restored: Project =
            serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
        assert_eq!(restored, project);
        restored.validate().unwrap();
        assert!(history.undo(&mut project));
        assert_eq!(project, before);
        assert!(history.redo(&mut project));
        assert_eq!(project, restored);
        let mut legacy = serde_json::to_value(&before).unwrap();
        legacy["tracks"][0]["clips"][0]["content"]["Audio"]
            .as_object_mut()
            .unwrap()
            .remove("crossfade");
        assert_eq!(serde_json::from_value::<Project>(legacy).unwrap(), before);
    }
    #[test]
    fn split_creates_a_new_priority_boundary_and_retains_entry_config_only_on_left() {
        let mut project = Project::new();
        let track = project.add_audio_track("Audio");
        let first = clip(0.0, 8.0);
        let first_id = first.id;
        let mut later = clip(1.0, 6.0);
        let later_id = later.id;
        crossfade(&mut later, 3.0, CrossfadeCurve::SMOOTH);
        project.track_mut(track).unwrap().clips = vec![first, later];
        let right = project.split_clip(track, later_id, 2.0).unwrap();
        let clips = &project.track(track).unwrap().clips;
        assert!(matches!(
            project.clip(track, right).unwrap().content,
            ClipContent::Audio {
                crossfade: None,
                ..
            }
        ));
        let schedule = AudioSchedule::new(clips).unwrap();
        assert_eq!(schedule.crossfades[0].end, 2.0);
        assert_eq!(schedule.crossfades[0].requested_duration, 3.0);
        let split_old = project.split_clip(track, first_id, 3.0).unwrap();
        let clips = &project.track(track).unwrap().clips;
        let schedule = AudioSchedule::new(clips).unwrap();
        assert_eq!(clips[schedule.sample(3.5).unwrap().incoming].id, split_old);
        project.trim_clip(track, later_id, 1.25, 0.5).unwrap();
        assert!(matches!(
            project.clip(track, later_id).unwrap().content,
            ClipContent::Audio {
                crossfade: Some(AudioCrossfade { duration: 3.0, .. }),
                ..
            }
        ));
    }
}
