//! Prepared automation lookup shared by channel controls, sends and plug-ins.
//!
//! Small lanes use the core model directly. For larger lanes, resolve overlap
//! precedence once, away from rendering, then locate one disjoint interval per
//! sample. Core curve evaluation remains unchanged, without approximation.
use daw_core::{AutomationLane, AutomationParameter};
use std::collections::BTreeSet;

// Short contiguous scans are cheaper than interval lookup on small lanes.
// Keep their preparation free of event/index allocations as well.
const DIRECT_PATTERN_LIMIT: usize = 32;

#[derive(Debug, PartialEq)]
struct Boundary {
    beat: f64,
    clip: Option<usize>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct CompiledAutomation {
    lane: AutomationLane,
    boundaries: Option<Vec<Boundary>>,
}

impl CompiledAutomation {
    pub(crate) fn new(lane: &AutomationLane) -> Self {
        if lane.clips.len() <= DIRECT_PATTERN_LIMIT {
            return Self {
                lane: lane.clone(),
                boundaries: None,
            };
        }
        // A later start wins; equal starts prefer the later project-vector
        // entry. Empty patterns never mask a populated pattern underneath.
        let mut order: Vec<_> = lane
            .clips
            .iter()
            .enumerate()
            .filter(|(_, clip)| !clip.points.is_empty() && clip.end() > clip.start)
            .map(|(index, _)| index)
            .collect();
        order.sort_unstable_by(|&a, &b| {
            lane.clips[a]
                .start
                .total_cmp(&lane.clips[b].start)
                .then(a.cmp(&b))
        });
        let mut events = Vec::with_capacity(order.len() * 2);
        for (rank, &index) in order.iter().enumerate() {
            let clip = &lane.clips[index];
            events.push((clip.start, rank, true));
            events.push((clip.end(), rank, false));
        }
        events.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut active = BTreeSet::new();
        let mut boundaries = Vec::with_capacity(events.len());
        let mut cursor = 0;
        let mut previous = None;
        while cursor < events.len() {
            let beat = events[cursor].0;
            while cursor < events.len() && events[cursor].0 == beat {
                let (_, rank, entering) = events[cursor];
                if entering {
                    active.insert(rank);
                } else {
                    active.remove(&rank);
                }
                cursor += 1;
            }
            let clip = active.last().map(|&rank| order[rank]);
            if clip != previous {
                boundaries.push(Boundary { beat, clip });
                previous = clip;
            }
        }
        Self {
            lane: lane.clone(),
            boundaries: Some(boundaries),
        }
    }

    pub(crate) fn parameter(&self) -> AutomationParameter {
        self.lane.target.parameter
    }

    pub(crate) fn pattern_only(&self) -> bool {
        self.lane.points.is_empty() && !self.lane.clips.is_empty()
    }

    pub(crate) fn override_value(&self) -> Option<f32> {
        self.lane.override_value
    }

    pub(crate) fn set_override_value(&mut self, value: Option<f32>) {
        self.lane.override_value = value;
    }

    #[inline]
    pub(crate) fn effective_value_at(&self, beat: f64) -> Option<f32> {
        let Some(boundaries) = &self.boundaries else {
            return self.lane.effective_value_at(beat);
        };
        if beat.is_nan() {
            return None;
        }
        if let Some(value) = self.lane.override_value {
            return Some(value);
        }
        let index = boundaries.partition_point(|boundary| boundary.beat <= beat);
        if let Some(index) = index.checked_sub(1)
            && let Some(clip) = boundaries[index].clip
        {
            return self.lane.clips[clip].value_at(beat);
        }
        self.lane.arrangement_value_at(beat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::{AutomationClip, AutomationPoint, AutomationTarget, Interpolation};

    fn fixture(interpolation: Interpolation, patterns: u64) -> AutomationLane {
        let points = |value| {
            let mut points = vec![
                AutomationPoint::new(0.0, value),
                AutomationPoint::new(0.75, 0.85),
                AutomationPoint::new(3.0, 0.15),
                AutomationPoint::new(8.0, 0.55),
            ];
            points[0].curve = -0.65;
            points[1].curve = 0.8;
            points[2].interpolation = Some(Interpolation::Spline);
            points
        };
        AutomationLane {
            target: AutomationTarget {
                track: 1,
                parameter: AutomationParameter::TrackGain,
            },
            points: points(0.4),
            interpolation,
            override_value: None,
            clips: (0..patterns)
                .map(|index| AutomationClip {
                    id: index + 1,
                    name: format!("Pattern {index}"),
                    // Deliberately unsorted starts, ties, nested and crossing ends.
                    start: ((index * 17) % 23) as f64 * 0.25,
                    length: 0.125 + (index % 11) as f64 * 0.25,
                    offset: (index % 5) as f64 * 0.125,
                    link_group: None,
                    points: if index % 7 == 6 {
                        Vec::new()
                    } else {
                        points(index as f32 / patterns.max(1) as f32)
                    },
                    interpolation,
                })
                .collect(),
        }
    }

    #[test]
    fn compiled_patterns_match_model_exactly_at_boundaries_overlaps_gaps_and_seeks() {
        for interpolation in [
            Interpolation::Step,
            Interpolation::Linear,
            Interpolation::Smooth,
            Interpolation::Spline,
        ] {
            for patterns in [0, 1, 8, 32, 33, 128] {
                let mut lane = fixture(interpolation, patterns);
                for arrangement in [true, false] {
                    if !arrangement {
                        lane.points.clear();
                    }
                    let compiled = CompiledAutomation::new(&lane);
                    assert_eq!(
                        compiled.boundaries.is_some(),
                        patterns as usize > DIRECT_PATTERN_LIMIT
                    );
                    let mut beats: Vec<_> =
                        (-64..=4096).map(|index| index as f64 / 256.0).collect();
                    for clip in &lane.clips {
                        for edge in [clip.start, clip.end()] {
                            beats.extend([edge.next_down(), edge, edge.next_up()]);
                        }
                    }
                    beats.extend([f64::NAN, f64::NEG_INFINITY, f64::INFINITY]);
                    // Reverse traversal also exercises arbitrary transport seeks.
                    for beat in beats.into_iter().rev() {
                        assert_eq!(
                            compiled.effective_value_at(beat),
                            lane.effective_value_at(beat),
                            "{patterns} patterns, {interpolation:?} at {beat}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn compiled_overrides_and_sample_lookup_do_not_allocate() {
        for patterns in [0, 32, 33, 128] {
            let lane = fixture(Interpolation::Smooth, patterns);
            let mut compiled = CompiledAutomation::new(&lane);
            assert_eq!(
                crate::allocation_test_support::count_allocations(|| {
                    for sample in 0..48_000 {
                        let beat = sample as f64 / 6_000.0;
                        assert_eq!(
                            compiled.effective_value_at(beat),
                            lane.effective_value_at(beat)
                        );
                    }
                }),
                0
            );
            compiled.set_override_value(Some(0.375));
            for beat in [-1.0, 0.0, 0.25, 10.0, f64::INFINITY] {
                assert_eq!(compiled.effective_value_at(beat), Some(0.375));
            }
            assert_eq!(compiled.effective_value_at(f64::NAN), None);
            compiled.set_override_value(None);
            assert_eq!(
                compiled.effective_value_at(0.25),
                lane.effective_value_at(0.25)
            );
        }
    }

    #[test]
    #[ignore = "manual CPU comparison; timings depend on host load"]
    fn benchmark_pattern_lookup() {
        let run = |lookup: &dyn Fn(f64) -> Option<f32>| {
            let start = std::time::Instant::now();
            let mut sum = 0.0;
            for sample in 0..480_000 {
                sum += std::hint::black_box(lookup(std::hint::black_box(sample as f64 / 60_000.0)))
                    .unwrap_or(0.0);
            }
            (sum, start.elapsed())
        };
        for patterns in [0, 1, 8, 32, 33, 128, 512] {
            let lane = fixture(Interpolation::Smooth, patterns);
            let compiled = CompiledAutomation::new(&lane);
            let (reference, before) = run(&|beat| lane.effective_value_at(beat));
            let (actual, after) = run(&|beat| compiled.effective_value_at(beat));
            assert_eq!(actual, reference);
            eprintln!(
                "{patterns}-pattern lookup, 480000 samples: model={before:?}, prepared={after:?}"
            );
        }
    }
}
