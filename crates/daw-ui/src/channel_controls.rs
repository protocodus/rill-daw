//! Compact mixer controls for canvas-owned track headers.
use egui::{Rect, Response, Ui};
use std::hash::Hash;

/// Display the channel's actual gain without normalizing stored quiet values.
/// Zero uses the editing floor; choosing that floor intentionally silences it.
pub fn channel_gain_to_db(gain: f32) -> f32 {
    if gain > 0.0 {
        20.0 * gain.log10()
    } else {
        -60.0
    }
}

pub fn channel_gain_from_db(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// The same precise channel-level field in Mixer and expanded track headers.
/// Values outside the editing range remain intact until an intentional edit.
pub fn channel_gain_number(db: &mut f32) -> egui::DragValue<'_> {
    crate::number(db)
        .range(-60.0..=20.0 * 1.5_f32.log10())
        .clamp_existing_to_range(false)
        .speed(0.1)
        .fixed_decimals(1)
        .suffix(" dB")
}

pub struct ChannelControlsResponse {
    pub gain: Response,
    pub pan: Response,
    pub meter: Response,
    pub gain_slider: Rect,
    pub pan_slider: Rect,
}

fn slider_at(ui: &mut Ui, rect: Rect, id: impl Hash, slider: egui::Slider<'_>) -> Response {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child.spacing_mut().slider_width = rect.width();
    child.spacing_mut().interact_size.y = rect.height();
    child.add(slider)
}

/// Draw real channel gain/pan sliders and a stereo peak meter without advancing
/// the parent canvas. Values belong to the mixer channel, never its clips.
/// Compact headers retain both sliders, with values available in their tooltip.
/// Normal headers also include the same editable numeric readouts as the mixer.
/// The caller supplies actual channel peaks; no signal or decay is fabricated.
pub fn channel_controls_at(
    ui: &mut Ui,
    rect: Rect,
    id_salt: impl Hash,
    gain: &mut f32,
    pan: &mut f32,
    peaks: [f32; 2],
    compact: bool,
) -> ChannelControlsResponse {
    let id = ui.make_persistent_id(id_salt);
    let meter_width = 10.0;
    let gap = 4.0;
    let control_right = rect.right() - meter_width - gap;
    let row_height = (rect.height() - gap) * 0.5;
    let value_width = 66.0;
    let slider_left = rect.left();
    let slider_right = control_right - value_width - gap;
    let (gain_rect, pan_rect) = if compact {
        let half = (control_right - rect.left() - gap) * 0.5;
        (
            Rect::from_min_max(
                egui::pos2(rect.left() + 28.0, rect.top()),
                egui::pos2(rect.left() + half, rect.bottom()),
            ),
            Rect::from_min_max(
                egui::pos2(rect.left() + half + gap + 28.0, rect.top()),
                egui::pos2(control_right, rect.bottom()),
            ),
        )
    } else {
        let gain = Rect::from_min_max(
            egui::pos2(slider_left, rect.top()),
            egui::pos2(slider_right, rect.top() + row_height),
        );
        (gain, gain.translate(egui::vec2(0.0, row_height + gap)))
    };
    let gain_value_rect = Rect::from_min_max(
        egui::pos2(slider_right + gap, rect.top()),
        egui::pos2(control_right, gain_rect.bottom()),
    );
    let pan_value_rect = gain_value_rect.translate(egui::vec2(0.0, row_height + gap));
    let mut db = channel_gain_to_db(*gain);
    let mut gain_response = slider_at(
        ui,
        gain_rect,
        id.with("gain_slider"),
        crate::slider(&mut db, -60.0..=20.0 * 1.5_f32.log10())
            .clamping(egui::SliderClamping::Edits)
            .show_value(false),
    );
    if !compact {
        gain_response = gain_response.union(crate::number_at(
            ui,
            gain_value_rect,
            id.with("gain_value"),
            channel_gain_number(&mut db),
        ));
    }
    if ui.is_enabled() && gain_response.changed() {
        *gain = channel_gain_from_db(db);
    }
    gain_response = gain_response.on_hover_text(format!(
        "Channel volume · {db:.1} dB\nDrag the slider or double-click the number to type"
    ));
    let mut pan_value = *pan * 100.0;
    let mut pan_response = slider_at(
        ui,
        pan_rect,
        id.with("pan_slider"),
        crate::slider(&mut pan_value, -100.0..=100.0)
            .clamping(egui::SliderClamping::Edits)
            .show_value(false),
    );
    if !compact {
        pan_response = pan_response.union(crate::number_at(
            ui,
            pan_value_rect,
            id.with("pan_value"),
            crate::number(&mut pan_value)
                .range(-100.0..=100.0)
                .clamp_existing_to_range(false)
                .speed(1.0)
                .fixed_decimals(0)
                .prefix("Pan "),
        ));
    }
    if ui.is_enabled() && pan_response.changed() {
        *pan = pan_value / 100.0;
    }
    pan_response = pan_response.on_hover_text(format!(
        "Channel pan · {pan_value:.0}\n−100 left · 0 center · 100 right"
    ));
    if compact {
        for (label, row) in [("Vol", gain_rect), ("Pan", pan_rect)] {
            crate::text::paint_single_line(
                ui.painter(),
                Rect::from_min_max(
                    egui::pos2(row.left() - 28.0, row.top()),
                    egui::pos2(row.left() - gap, row.bottom()),
                ),
                egui::FontId::proportional(crate::tokens::FONT_CANVAS),
                crate::theme::palette(ui.ctx()).muted,
                label,
            );
        }
    }
    let meter_rect =
        Rect::from_min_max(egui::pos2(rect.right() - meter_width, rect.top()), rect.max);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id.with("meter"))
            .max_rect(meter_rect),
    );
    child.set_clip_rect(ui.clip_rect().intersect(meter_rect));
    let meter = crate::stereo_meter(&mut child, peaks, meter_rect.size())
        .on_hover_text("Channel output level · post-fader");
    ChannelControlsResponse {
        gain: gain_response,
        pan: pan_response,
        meter,
        gain_slider: gain_rect,
        pan_slider: pan_rect,
    }
}
