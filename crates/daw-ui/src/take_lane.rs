//! Compact waveform choices for take selection. Musical state stays in the caller.
use crate::{text::TextRole, theme};
use egui::{Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui};

pub struct TakeLane<'a> {
    pub name: &'a str,
    pub detail: &'a str,
    pub selected: bool,
    pub color: Color32,
    pub peaks: &'a [(f32, f32)],
    /// Visible source span as a fraction of the shared time range.
    pub span: (f32, f32),
    /// Selected time range, also normalized against that shared range.
    pub selection: Option<(f32, f32)>,
}

pub fn audio_take_lane(ui: &mut Ui, lane: &TakeLane<'_>) -> Response {
    audio_take_lane_with_background(ui, lane, |_, _| {})
}

/// Draw an optional source visualization behind the standard take waveform.
/// The callback receives the full plot; the caller owns source-span mapping.
pub fn audio_take_lane_with_background(
    ui: &mut Ui,
    lane: &TakeLane<'_>,
    background: impl FnOnce(&egui::Painter, Rect),
) -> Response {
    audio_take_lane_with_waveform(ui, lane, |painter, plot| {
        background(painter, plot);
        let (a, b) = lane.span;
        for (i, (low, high)) in lane.peaks.iter().enumerate() {
            let fraction = a + (b - a) * (i as f32 + 0.5) / lane.peaks.len().max(1) as f32;
            if !(0.0..=1.0).contains(&fraction) {
                continue;
            }
            let x = plot.left() + fraction * plot.width();
            painter.line_segment(
                [
                    egui::pos2(
                        x,
                        plot.center().y - high.clamp(-1.0, 1.0) * plot.height() * 0.5,
                    ),
                    egui::pos2(
                        x,
                        plot.center().y - low.clamp(-1.0, 1.0) * plot.height() * 0.5,
                    ),
                ],
                Stroke::new(1.5_f32, lane.color),
            );
        }
    })
}

/// Reuse the take choice layout with a caller-supplied waveform renderer.
/// The painter is clipped to the shared plot, so progressive source previews
/// cannot paint over the title, details, selection controls or neighbouring rows.
pub fn audio_take_lane_with_waveform(
    ui: &mut Ui,
    lane: &TakeLane<'_>,
    waveform: impl FnOnce(&egui::Painter, Rect),
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 86.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            lane.selected,
            lane.name,
        )
    });
    if ui.is_rect_visible(rect) {
        let colors = theme::palette(ui.ctx());
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let fill = lane.color.gamma_multiply(if lane.selected {
            0.16
        } else if response.hovered() {
            0.10
        } else {
            0.04
        });
        painter.rect(
            rect.shrink(1.0),
            6.0,
            fill,
            Stroke::new(
                if lane.selected || response.has_focus() {
                    1.5_f32
                } else {
                    1.0_f32
                },
                if lane.selected || response.has_focus() {
                    lane.color
                } else {
                    colors.line
                },
            ),
            StrokeKind::Inside,
        );
        let text_rect = Rect::from_min_max(
            rect.min + egui::vec2(12.0, 8.0),
            egui::pos2(rect.right() - 12.0, rect.top() + 28.0),
        );
        crate::text::paint_single_line(
            &painter,
            text_rect,
            TextRole::Label.font(),
            colors.text,
            lane.name,
        );
        let detail = text_rect.translate(egui::vec2(0.0, 22.0));
        crate::text::paint_single_line(
            &painter,
            detail,
            TextRole::Note.font(),
            colors.muted,
            lane.detail,
        );
        let plot = Rect::from_min_max(
            rect.min + egui::vec2(12.0, 51.0),
            rect.max - egui::vec2(12.0, 8.0),
        );
        if let Some((a, b)) = lane.selection {
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(plot.left() + a.clamp(0.0, 1.0) * plot.width(), plot.top()),
                    egui::pos2(
                        plot.left() + b.clamp(0.0, 1.0) * plot.width(),
                        plot.bottom(),
                    ),
                ),
                0.0,
                colors.accent.gamma_multiply(0.18),
            );
        }
        waveform(
            &painter.with_clip_rect(plot.intersect(painter.clip_rect())),
            plot,
        );
    }
    response.on_hover_text(format!("{}\n{}", lane.name, lane.detail))
}
