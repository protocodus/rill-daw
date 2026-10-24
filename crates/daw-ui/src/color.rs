//! Color markers with a generous hit area and an on-demand native picker.
use egui::{Color32, Rect, Response, Ui};
use std::hash::Hash;

pub struct ColorStripResponse {
    pub response: Response,
    pub picker: Option<Rect>,
}

/// Draw a narrow identity marker inside a canvas-owned hit rectangle. The
/// caller owns the RGB value and commits changes through its usual undo path.
pub fn color_strip_at(
    ui: &mut Ui,
    rect: Rect,
    id_salt: impl Hash,
    label: &str,
    value: &mut [u8; 3],
) -> ColorStripResponse {
    let id = ui.make_persistent_id(id_salt);
    let mut response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, ui.is_enabled(), label)
    });
    let mut color = Color32::from_rgb(value[0], value[1], value[2]);
    let popup = egui::Popup::menu(&response)
        .id(id.with("picker"))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            crate::text::label(ui, label);
            ui.spacing_mut().slider_width = 275.0;
            egui::color_picker::color_picker_color32(
                ui,
                &mut color,
                egui::color_picker::Alpha::Opaque,
            )
        });
    if ui.is_enabled() && popup.as_ref().is_some_and(|popup| popup.inner) {
        let rgb = [color.r(), color.g(), color.b()];
        // Simply opening a picker must not quantize a saved color or add undo.
        if rgb != *value {
            *value = rgb;
            response.mark_changed();
        }
    }
    let width: f32 = if response.hovered() || response.has_focus() || popup.is_some() {
        6.0
    } else {
        3.0
    };
    let marker = Rect::from_min_size(rect.min, egui::vec2(width.min(rect.width()), rect.height()));
    ui.painter()
        .rect_filled(marker, 0.0, Color32::from_rgb(value[0], value[1], value[2]));
    ColorStripResponse {
        response,
        picker: popup.map(|popup| popup.response.rect),
    }
}
