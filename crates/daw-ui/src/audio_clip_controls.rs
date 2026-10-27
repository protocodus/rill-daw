//! Compact direct editing affordances for audio regions.
use egui::{Color32, Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

/// Fade handles share a generous hit target while keeping waveform ink small.
pub fn audio_fade_handle(
    ui: &mut Ui,
    id: Id,
    center: Pos2,
    clip: Rect,
    tint: Color32,
    incoming: bool,
) -> Response {
    let target = Rect::from_center_size(center, Vec2::splat(14.0)).intersect(clip);
    let response = ui.interact(target, id, Sense::drag());
    let painter = ui.painter().with_clip_rect(clip);
    let foreground = if response.hovered() || response.dragged() {
        crate::theme::palette(ui.ctx()).text
    } else {
        tint
    };
    painter.rect_filled(
        Rect::from_center_size(center, Vec2::splat(6.0)),
        1.0,
        foreground,
    );
    let direction = if incoming { -1.0 } else { 1.0 };
    painter.line_segment(
        [center, center + Vec2::new(direction * 6.0, 6.0)],
        Stroke::new(1.0_f32, foreground),
    );
    response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text(if incoming {
            "Fade in · drag to change duration"
        } else {
            "Fade out · drag to change duration"
        })
}
