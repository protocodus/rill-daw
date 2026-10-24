//! Audio-region visualization shared by clip editors and other timeline views.

use crate::{TimelineViewport, theme};
use egui::{Color32, Painter, Pos2, Rect, Stroke};

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
