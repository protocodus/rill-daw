//! Selection feedback shared by time and item canvases.
use egui::{Painter, Pos2, Rect, Stroke, StrokeKind};

/// egui counts rapid clicks globally, so an item must also verify identity and
/// location before treating a selection click as a double-click action.
#[derive(Clone, Default)]
pub struct ItemClickTracker {
    previous: Option<(u64, Pos2, f64)>,
}

impl ItemClickTracker {
    pub fn register(&mut self, id: u64, position: Pos2, now: f64, plain: bool) -> bool {
        let double = plain
            && self.previous.is_some_and(|(last, point, time)| {
                last == id && point.distance(position) < 8.0 && now - time < 0.4
            });
        self.previous = if double || !plain {
            None
        } else {
            Some((id, position, now))
        };
        double
    }

    pub fn clear(&mut self) {
        self.previous = None;
    }
}

/// A clipped, translucent selection box; hit testing belongs to its canvas.
pub fn paint_selection_region(painter: &Painter, region: Rect) {
    let colors = crate::theme::palette(painter.ctx());
    painter.rect_filled(region, 0.0, colors.accent.gamma_multiply(0.10));
    painter.rect_stroke(
        region,
        0.0,
        Stroke::new(1.0_f32, colors.accent),
        StrokeKind::Inside,
    );
}
