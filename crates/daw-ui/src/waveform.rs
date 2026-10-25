//! Audio-region visualization shared by clip editors and other timeline views.

use crate::{TimelineViewport, theme};
use egui::{Color32, Painter, Pos2, Rect, Stroke};

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioClipEdges {
    pub start: Option<Rect>,
    pub end: Option<Rect>,
}

fn visible_range(rect: Rect, view: TimelineViewport, start: f64, end: f64) -> Option<(f32, f32)> {
    if !rect.is_finite()
        || !rect.is_positive()
        || !start.is_finite()
        || !end.is_finite()
        || end <= start
        || !view.start_beat.is_finite()
        || !view.pixels_per_beat.is_finite()
        || view.pixels_per_beat <= 0.0
    {
        return None;
    }
    let (left, right) = (view.x_at(rect, start), view.x_at(rect, end));
    (left.is_finite() && right.is_finite() && right >= rect.left() && left <= rect.right())
        .then_some((left, right))
}

/// A time selection shared by sample editors and comping views. Clipping never
/// turns an offscreen range boundary into a visible, draggable boundary.
pub fn paint_timeline_selection(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    start: f64,
    end: f64,
) -> Option<Rect> {
    let (left, right) = visible_range(rect, view, start, end)?;
    let visible = Rect::from_min_max(
        egui::pos2(left.max(rect.left()), rect.top()),
        egui::pos2(right.min(rect.right()), rect.bottom()),
    )
    .intersect(painter.clip_rect());
    if !visible.is_positive() {
        return None;
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let color = theme::palette(painter.ctx()).accent;
    painter.rect_filled(visible, 0.0, color.gamma_multiply(0.14));
    for x in [left, right] {
        if x >= rect.left() && x <= rect.right() {
            painter.vline(x, rect.y_range(), Stroke::new(1.0_f32, color));
        }
    }
    Some(visible)
}

/// Visible trim handles, with comfortable eight-point targets. The application
/// owns gesture priority and editing; this component only paints and measures.
pub fn paint_audio_clip_edges(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    start: f64,
    end: f64,
    color: Color32,
) -> AudioClipEdges {
    let Some((left, right)) = visible_range(rect, view, start, end) else {
        return AudioClipEdges::default();
    };
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let edge = |x: f32| {
        if x < rect.left() || x > rect.right() {
            return None;
        }
        let hit = Rect::from_min_max(
            egui::pos2((x - 4.0).max(rect.left()), rect.top()),
            egui::pos2((x + 4.0).min(rect.right()), rect.bottom()),
        );
        painter.vline(x, rect.y_range(), Stroke::new(1.5_f32, color));
        painter.rect_filled(
            Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(4.0, 28.0))
                .intersect(rect),
            2.0,
            color,
        );
        Some(hit)
    };
    AudioClipEdges {
        start: edge(left),
        end: edge(right),
    }
}

#[derive(Clone, Copy)]
pub struct WaveformColumn {
    pub beat: f64,
    pub channel: usize,
    pub minimum: f32,
    pub maximum: f32,
}

/// The caller supplies bounded peak columns in musical time. Each channel uses
/// two positive rows; `headroom` is the amplitude represented by one row. Keeping
/// that reference until Fit avoids channel-center jumps during gain edits.
pub fn paint_audio_waveform(
    painter: &Painter,
    rect: Rect,
    view: TimelineViewport,
    columns: &[WaveformColumn],
    channels: usize,
    headroom: f32,
    color: Color32,
) {
    let painter = painter.with_clip_rect(rect);
    let headroom = headroom.max(0.001);
    for channel in 0..channels {
        let center = view.y_at(rect, (channel * 2 + 1) as f32);
        painter.line_segment(
            [
                Pos2::new(rect.left(), center),
                Pos2::new(rect.right(), center),
            ],
            Stroke::new(1.0_f32, theme::palette(painter.ctx()).line),
        );
    }
    for column in columns {
        let x = view.x_at(rect, column.beat);
        let center = (column.channel * 2 + 1) as f32;
        painter.line_segment(
            [
                Pos2::new(x, view.y_at(rect, center - column.maximum / headroom)),
                Pos2::new(x, view.y_at(rect, center - column.minimum / headroom)),
            ],
            Stroke::new(1.0_f32, color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_hides_offscreen_handles_without_losing_visible_selection() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            let rect = Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(400.0, 160.0));
            let painter = ctx
                .layer_painter(egui::LayerId::background())
                .with_clip_rect(rect);
            let view = TimelineViewport {
                start_beat: 4.0,
                pixels_per_beat: 50.0,
                ..Default::default()
            };
            let edges = paint_audio_clip_edges(&painter, rect, view, 2.0, 8.0, Color32::WHITE);
            assert!(edges.start.is_none());
            let end = edges.end.unwrap();
            assert_eq!(end.center().x, 300.0);
            assert_eq!(end.width(), 8.0);
            let selection = paint_timeline_selection(&painter, rect, view, 2.0, 8.0).unwrap();
            assert_eq!(selection.left(), rect.left());
            assert_eq!(selection.right(), 300.0);
            assert!(paint_timeline_selection(&painter, rect, view, 0.0, 1.0).is_none());
            assert!(paint_timeline_selection(&painter, rect, view, f64::NAN, 8.0).is_none());
            let edges = paint_audio_clip_edges(&painter, rect, view, 14.0, 18.0, Color32::WHITE);
            assert!(edges.start.is_none() && edges.end.is_none());
        });
    }
}
