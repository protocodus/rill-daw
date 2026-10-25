//! Preparation-only overlap compilation. Rendering traverses disjoint segments,
//! reads at most two sources, and linearly interpolates a bounded-error curve.
use anyhow::Result;
use daw_core::{AudioSchedule, Clip, ClipContent, CrossfadeCurve};
use std::{collections::HashMap, sync::Arc};

pub(super) struct Segment {
    pub start: u64,
    pub end: u64,
    pub incoming: usize,
    pub outgoing: Option<usize>,
    pub fade: Option<Arc<Curve>>,
}

pub(super) fn compile(clips: &[Clip], samples_per_beat: f64) -> Result<Vec<Segment>> {
    let source = AudioSchedule::new(clips)?;
    // Core schedule indices refer to the project vector, which also retains
    // inactive take sources. The render source vector contains audible audio only.
    let mut indices = vec![usize::MAX; clips.len()];
    let mut next = 0;
    for (index, clip) in clips.iter().enumerate() {
        if !clip.is_take_source() && matches!(clip.content, ClipContent::Audio { .. }) {
            indices[index] = next;
            next += 1;
        }
    }
    let mut curves = HashMap::new();
    let to_sample = |beat: f64| (beat * samples_per_beat).round() as u64;
    Ok(source
        .segments
        .iter()
        .filter_map(|segment| {
            let start = to_sample(segment.start);
            let end = to_sample(segment.end);
            if start == end {
                return None;
            }
            let fade = segment.crossfade.map(|index| {
                let curve = source.crossfades[index].curve;
                let key = [
                    curve.control1[0].to_bits(),
                    curve.control1[1].to_bits(),
                    curve.control2[0].to_bits(),
                    curve.control2[1].to_bits(),
                ];
                Arc::clone(
                    curves
                        .entry(key)
                        .or_insert_with(|| Arc::new(Curve::new(curve))),
                )
            });
            Some(Segment {
                start,
                end,
                incoming: indices[segment.incoming],
                outgoing: segment.outgoing.map(|index| indices[index]),
                fade,
            })
        })
        .collect())
}

pub(super) struct Curve {
    points: Vec<[f64; 2]>,
}
impl Curve {
    fn new(curve: CrossfadeCurve) -> Self {
        let mut points = vec![[0.0, 0.0]];
        let mut work = vec![(
            [
                [0.0, 0.0],
                curve.control1.map(f64::from),
                curve.control2.map(f64::from),
                [1.0, 1.0],
            ],
            0_u8,
        )];
        while let Some((p, depth)) = work.pop() {
            let dx = p[3][0] - p[0][0];
            // The entire Bezier is inside its control polygon. Its maximum
            // vertical distance from this line bounds amplitude lookup error.
            let deviation = if dx > 0.0 {
                let at = |point: [f64; 2]| {
                    (point[1] - p[0][1] - (point[0] - p[0][0]) / dx * (p[3][1] - p[0][1])).abs()
                };
                at(p[1]).max(at(p[2]))
            } else {
                0.0
            };
            if deviation <= 1e-7 || depth == 24 {
                points.push(p[3]);
            } else {
                let half = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                let a = half(p[0], p[1]);
                let b = half(p[1], p[2]);
                let c = half(p[2], p[3]);
                let d = half(a, b);
                let e = half(b, c);
                let middle = half(d, e);
                work.push(([middle, e, c, p[3]], depth + 1));
                work.push(([p[0], a, d, middle], depth + 1));
            }
        }
        Self { points }
    }
    pub fn cursor(&self, progress: f64) -> usize {
        self.points
            .partition_point(|point| point[0] <= progress)
            .saturating_sub(1)
            .min(self.points.len() - 2)
    }
    #[inline]
    pub fn value(&self, progress: f64, cursor: &mut usize) -> f32 {
        if progress <= 0.0 {
            return 0.0;
        }
        if progress >= 1.0 {
            return 1.0;
        }
        while *cursor + 2 < self.points.len() && self.points[*cursor + 1][0] < progress {
            *cursor += 1;
        }
        let a = self.points[*cursor];
        let b = self.points[*cursor + 1];
        let t = ((progress - a[0]) / (b[0] - a[0])).clamp(0.0, 1.0);
        (a[1] + (b[1] - a[1]) * t).clamp(0.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiled_curves_match_exact_splines_including_steep_and_reversed_y_handles() {
        for curve in [
            CrossfadeCurve::LINEAR,
            CrossfadeCurve::SMOOTH,
            CrossfadeCurve {
                control1: [0.0, 1.0],
                control2: [0.0, 1.0],
            },
            CrossfadeCurve {
                control1: [1.0, 0.0],
                control2: [1.0, 0.0],
            },
            CrossfadeCurve {
                control1: [0.1, 0.95],
                control2: [0.9, 0.05],
            },
        ] {
            let compiled = Curve::new(curve);
            assert!(compiled.points.len() < 16384);
            let mut cursor = 0;
            for step in 0..=10000 {
                let x = f64::from(step) / 10000.0;
                assert!(
                    (compiled.value(x, &mut cursor) - curve.value(x)).abs() < 3e-7,
                    "{curve:?} at{x}"
                );
            }
            for x in [0.0000001, 0.25, 0.77, 0.9999999] {
                let mut cursor = compiled.cursor(x);
                assert!((compiled.value(x, &mut cursor) - curve.value(x)).abs() < 3e-7);
            }
        }
    }
}
