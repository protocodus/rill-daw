//! Meter-aware downbeats and the arranger's shared track grid.
use crate::{MeterMap, TimelineViewport, theme};
use egui::{Color32, Painter, Rect, Stroke};

pub(crate) const RULER_LABEL_SPACING: f32 = 80.0;

pub(crate) struct VisibleBar {
    pub number: u32,
    pub beat: f64,
    pub emphasized: bool,
    pub labeled: bool,
}

/// Keep bar groupings anchored to bar one as the view zooms through 1, 2, 4, … bars.
fn bar_stride(points_per_bar: f32, minimum: f32) -> u32 {
    let bars = (minimum / points_per_bar)
        .ceil()
        .clamp(1.0, (1_u32 << 31) as f32);
    (bars as u32).next_power_of_two()
}

/// Start with the bar containing the left edge: its label may still fit when
/// panned a few pixels past the downbeat. Thin each meter segment at its own scale.
pub(crate) fn visible_bars(
    rect: Rect,
    view: TimelineViewport,
    meter: &MeterMap,
) -> impl Iterator<Item = VisibleBar> {
    let first = meter.position(view.start_beat).0;
    let end = view.beat_at(rect, rect.right());
    let last = meter.position(end).0;
    let segments = meter.segments();
    segments
        .iter()
        .enumerate()
        .flat_map(move |(index, segment)| {
            let bar_width = f32::from(segment.beats_per_bar) * view.pixels_per_beat;
            let stride = bar_stride(bar_width, crate::snap::GRID_MIN_SPACING as f32);
            let label_stride = bar_stride(bar_width, RULER_LABEL_SPACING);
            let first_group = (first.max(segment.bar) - 1).div_ceil(stride);
            let last_bar = segments
                .get(index + 1)
                .map_or(last, |next| last.min(next.bar - 1));
            let last_group = (last_bar - 1) / stride;
            (first_group..=last_group).filter_map(move |group| {
                let bar = group * stride + 1;
                let beat =
                    segment.start + f64::from(bar - segment.bar) * f64::from(segment.beats_per_bar);
                (beat <= end).then_some(VisibleBar {
                    number: bar,
                    beat,
                    emphasized: group.is_multiple_of(4),
                    labeled: (bar - 1).is_multiple_of(label_stride),
                })
            })
        })
        .take(4096)
}

/// Visible positions computed once for all track rows and the space below them.
#[derive(Default)]
pub struct ArrangerGrid {
    bars: Vec<(f32, bool)>,
    steps: Vec<(f32, bool)>,
}

impl ArrangerGrid {
    pub fn new(rect: Rect, view: TimelineViewport, meter: &MeterMap) -> Self {
        if !rect.is_finite()
            || !rect.is_positive()
            || !view.start_beat.is_finite()
            || !view.pixels_per_beat.is_finite()
            || view.pixels_per_beat <= 0.0
        {
            return Self::default();
        }
        let bars: Vec<_> = visible_bars(rect, view, meter)
            .filter_map(|bar| {
                let x = view.x_at(rect, bar.beat);
                rect.x_range().contains(x).then_some((x, bar.emphasized))
            })
            .collect();
        // Grid snap uses uniform beats from song zero. Keep its positions even
        // when a meter change moves a downbeat between two coarse snap lines.
        let mut step = crate::grid_step(view.pixels_per_beat, meter.initial());
        // Extremely wide Fit views can put several snap positions inside a
        // pixel. Draw a power-of-two subset while preserving the edit increment.
        while step * f64::from(view.pixels_per_beat) < crate::snap::GRID_MIN_SPACING {
            step *= 2.0;
        }
        let first = (view.start_beat / step).ceil() as u64;
        let last = (view.beat_at(rect, rect.right()) / step).floor() as u64;
        let steps = (first..=last)
            .take(4096)
            .filter_map(|index| {
                let beat = index as f64 * step;
                let x = view.x_at(rect, beat);
                (rect.x_range().contains(x)
                    && bars
                        .binary_search_by(|(bar_x, _)| bar_x.total_cmp(&x))
                        .is_err())
                .then_some((x, beat.fract() == 0.0))
            })
            .collect();
        Self { bars, steps }
    }

    pub fn paint_track(&self, painter: &Painter, row: Rect) {
        let colors = theme::palette(painter.ctx());
        paint_guides(
            painter,
            row,
            &self.steps,
            colors.grid_minor.gamma_multiply(0.45),
            colors.grid_minor,
        );
        paint_guides(
            painter,
            row,
            &self.bars,
            colors.grid_bar,
            colors.grid_emphasis(),
        );
    }

    /// Musical landmarks continue below the last track with no snap subdivisions.
    pub fn paint_empty(&self, painter: &Painter, row: Rect) {
        let colors = theme::palette(painter.ctx());
        paint_guides(painter, row, &self.bars, colors.grid_minor, colors.grid_bar);
    }
}

fn paint_guides(
    painter: &Painter,
    row: Rect,
    lines: &[(f32, bool)],
    normal: Color32,
    strong: Color32,
) {
    // As in the ruler, the one-pixel stroke is centered on its musical x.
    // egui aligns both strokes to the same pixel center during tessellation.
    let width = 1.0 / painter.pixels_per_point();
    for &(x, emphasized) in lines {
        let color = if emphasized { strong } else { normal };
        painter.vline(x, row.y_range(), Stroke::new(width, color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_timeline::MeterChange;
    use egui::{Pos2, vec2};

    #[test]
    fn long_song_overviews_bound_the_grid_and_keep_large_bar_positions_exact() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 64.0));
        let meter = MeterMap::new(
            4,
            &[MeterChange {
                bar: 3,
                beats_per_bar: 3,
            }],
        );
        // This bar number exceeds the single-precision integer range.
        let large_bar = (1 << 26) + 1;
        let start = meter.bar_start(large_bar);
        for scale in [0.000_01, 8.0, 4096.0] {
            let view = TimelineViewport {
                start_beat: start,
                pixels_per_beat: scale,
                ..Default::default()
            };
            let bars: Vec<_> = visible_bars(rect, view, &meter).collect();
            assert_eq!((bars[0].number, bars[0].beat), (large_bar, start));
            assert!(bars[0].emphasized);
            let grid = ArrangerGrid::new(rect, view, &meter);
            // The full viewport is populated without a dense cluster at its left.
            for positions in [&grid.bars, &grid.steps] {
                assert!(positions.len() <= 121);
                assert!(positions.iter().all(|(x, _)| rect.x_range().contains(*x)));
                assert!(
                    positions
                        .windows(2)
                        .all(|pair| pair[1].0 - pair[0].0 >= 9.99)
                );
            }
            if scale < 1.0 {
                assert!(grid.bars.last().unwrap().0 > 1180.0);
            }
        }
    }
}
