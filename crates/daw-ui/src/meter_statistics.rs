//! Compact calibrated channel RMS readout, supplied by the audio engine.
use egui::{Align2, FontId, Pos2, Response, Sense, Ui, Vec2};

fn dbfs(value: f32) -> String {
    if value.is_finite() && value > 0.0 {
        format!("{:.1}", 20.0 * value.log10())
    } else {
        "−∞".into()
    }
}

/// A read-only 21-point row. The engine supplies linear stereo RMS measured
/// with a 300 ms exponential mean-square time constant. It is not a VU meter.
pub fn channel_meter_statistics(ui: &mut Ui, width: f32, rms: [f32; 2]) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, crate::tokens::COMPACT_HEIGHT),
        Sense::hover(),
    );
    let clean = rms.map(|value| {
        if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        }
    });
    let caption = format!("RMS {}", dbfs(clean[0].max(clean[1])));
    if ui.is_rect_visible(rect) {
        let compact = width < 80.0;
        let origin = egui::emath::GuiRounding::round_to_pixels(
            Pos2::new(
                if compact {
                    rect.center().x
                } else {
                    rect.left() + crate::tokens::SPACE_SM
                },
                rect.center().y,
            ),
            ui.pixels_per_point(),
        );
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .text(
                origin,
                if compact {
                    Align2::CENTER_CENTER
                } else {
                    Align2::LEFT_CENTER
                },
                if compact {
                    dbfs(clean[0].max(clean[1]))
                } else {
                    caption.clone()
                },
                FontId::proportional(crate::tokens::FONT_CAPTION),
                crate::theme::palette(ui.ctx()).muted,
            );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &caption)
    });
    crate::hover_text(
        response,
        format!(
            "Post-fader RMS · 300 ms exponential time constant\nLeft {} dBFS · Right {} dBFS\nThe strip shows the louder channel. Peak meters remain independent.",
            dbfs(clean[0]),
            dbfs(clean[1])
        ),
    )
}
