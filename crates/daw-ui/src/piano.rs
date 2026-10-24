//! Shared piano-key and velocity-marker primitives for music editors.
use crate::theme;
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

/// Soften the grid beyond a clip and retain a precise visible end boundary.
/// This paint-only component neither changes layout nor captures editor input.
pub fn paint_clip_tail(painter: &Painter, plot: Rect, end_x: f32) -> Option<Rect> {
    if !plot.is_positive() || !plot.is_finite() || !end_x.is_finite() || end_x >= plot.right() {
        return None;
    }
    let colors = theme::palette(painter.ctx());
    let tail = Rect::from_min_max(Pos2::new(end_x.max(plot.left()), plot.top()), plot.max);
    let painter = painter.with_clip_rect(plot);
    painter.rect_filled(tail, 0.0, colors.inactive_clip_tint());
    if end_x >= plot.left() {
        painter.line_segment(
            [
                Pos2::new(end_x, plot.top()),
                Pos2::new(end_x, plot.bottom()),
            ],
            Stroke::new(1.5_f32, colors.grid_bar),
        );
    }
    Some(tail)
}

pub fn piano_key(
    painter: &Painter,
    rect: Rect,
    label: &str,
    black: bool,
    highlight: u8,
    active: bool,
) {
    let colors = theme::palette(painter.ctx());
    let base = if black {
        colors.piano_black
    } else {
        colors.piano_white
    };
    let tint = if highlight > 1 {
        colors.note_selected
    } else {
        colors.note
    };
    let amount = if active {
        0.55
    } else if highlight > 0 {
        0.22
    } else {
        0.0
    };
    let fill = Color32::from_rgb(
        egui::lerp(f32::from(base.r())..=f32::from(tint.r()), amount) as u8,
        egui::lerp(f32::from(base.g())..=f32::from(tint.g()), amount) as u8,
        egui::lerp(f32::from(base.b())..=f32::from(tint.b()), amount) as u8,
    );
    painter.rect_filled(rect, 2.0, fill);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0_f32, theme::palette(painter.ctx()).bg),
    );
    if highlight > 0 {
        let color = if highlight > 1 {
            colors.note_selected
        } else {
            colors.note
        };
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.right() - 6.0, rect.top() + 1.0),
                Vec2::new(5.0, (rect.height() - 2.0).max(1.0)),
            ),
            1.0,
            color,
        );
    }
    if !label.is_empty() && rect.height() >= 14.0 && painter.clip_rect().contains_rect(rect) {
        painter.text(
            rect.right_center() - Vec2::new(11.0, 0.0),
            Align2::RIGHT_CENTER,
            label,
            FontId::monospace(crate::tokens::FONT_CANVAS),
            theme::contrast_text(fill),
        );
    }
}

pub fn velocity_marker(painter: &Painter, bounds: Rect, normalized: f32, selected: bool) {
    let y = bounds.bottom() - normalized.clamp(0.0, 1.0) * bounds.height();
    let colors = theme::palette(painter.ctx());
    let color = if selected {
        colors.note_selected
    } else {
        colors.note
    };
    painter.line_segment(
        [
            Pos2::new(bounds.center().x, bounds.bottom()),
            Pos2::new(bounds.center().x, y),
        ],
        Stroke::new(2.0_f32, color),
    );
    painter.rect_filled(
        Rect::from_center_size(Pos2::new(bounds.center().x, y), Vec2::new(7.0, 5.0)),
        1.0,
        color,
    );
    if selected {
        painter.rect_stroke(
            Rect::from_center_size(Pos2::new(bounds.center().x, y), Vec2::new(8.0, 6.0)),
            1.0,
            Stroke::new(0.5_f32, theme::palette(painter.ctx()).text),
            StrokeKind::Inside,
        );
    }
}
