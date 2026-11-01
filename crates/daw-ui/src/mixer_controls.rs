//! Console controls composed from the shared native slider and button widgets.
//! Audio levels and parameter values always belong to the caller.
use egui::{Align2, FontId, Rect, Response, Sense, SliderClamping, Stroke, Ui, Vec2};

/// Includes the calibrated dB captions to the left of the native fader target.
pub const MIXER_FADER_WIDTH: f32 = 52.0 + crate::tokens::SPACE_XL + crate::tokens::SPACE_XS;
pub const MIXER_LEVEL_METER_HEIGHT: f32 = crate::tokens::PARAMETER_ROW_HEIGHT;

/// Linear 0..=1.5 channel gain with an external dB scale. Existing values are
/// never clamped merely by displaying the fader. The returned native response
/// retains its rail identity for exact entry, automation, and scale geometry.
pub fn mixer_fader(ui: &mut Ui, gain: &mut f32, height: f32) -> Response {
    let response = ui
        .allocate_ui_with_layout(
            Vec2::new(MIXER_FADER_WIDTH, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().slider_width = height;
                ui.add_space(52.0);
                ui.add(
                    crate::slider(gain, 0.0..=1.5)
                        .mixer_style()
                        .zero_reference(1.0)
                        .clamping(SliderClamping::Edits)
                        .vertical()
                        .show_value(false)
                        .scale_labels(false),
                )
            },
        )
        .inner;
    if let Some(scale) = crate::slider_scale_geometry(ui.ctx(), response.id) {
        let colors = crate::theme::palette(ui.ctx());
        for (label, value) in [
            ("+3.5 dB", 1.5_f32),
            ("0", 1.0),
            ("−6", 10.0_f32.powf(-6.0 / 20.0)),
            ("−12", 10.0_f32.powf(-12.0 / 20.0)),
            ("−∞", 0.0),
        ] {
            let center = scale.minimum.lerp(scale.maximum, value / 1.5);
            let color = if value == 1.0 {
                colors.text
            } else {
                colors.muted
            };
            ui.painter().line_segment(
                [
                    egui::pos2(scale.rail.left() - 6.0, center.y),
                    egui::pos2(scale.rail.left() - 1.0, center.y),
                ],
                Stroke::new(crate::tokens::BORDER_WIDTH, color),
            );
            ui.painter().text(
                egui::pos2(scale.rail.left() - 10.0, center.y),
                Align2::RIGHT_CENTER,
                label,
                FontId::proportional(crate::tokens::FONT_CANVAS),
                color,
            );
        }
    }
    crate::hover_text(
        response,
        format!(
            "Channel volume · {}\nRight-click for exact value entry and automation",
            crate::channel_gain_label(*gain)
        ),
    )
}

/// Text-free relative stereo-balance ring with an adjacent current-value
/// caption. The whole row exposes the same context menu;
/// clicking it never jumps the value to an absolute pointer position.
pub fn mixer_pan(ui: &mut Ui, pan: &mut f32, width: f32) -> Response {
    let height = crate::Knob::compact_height(48.0).max(crate::tokens::PARAMETER_ROW_HEIGHT);
    let row = ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(Vec2::new(width, height));
            ui.set_row_height(height);
            ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_SM;
            let response = ui.add(
                crate::knob(pan, -1.0..=1.0)
                    .compact()
                    .diameter(48.0)
                    .text("Pan")
                    .clamping(SliderClamping::Edits),
            );
            let caption = format!("Pan {}", crate::channel_pan_label(*pan));
            let label = ui.add(
                egui::Label::new(
                    crate::text::TextRole::Note
                        .rich(ui.ctx(), caption)
                        .color(crate::theme::palette(ui.ctx()).text),
                )
                .truncate()
                // The row owns context clicks and focus. A selectable label
                // would intercept secondary clicks on the current value.
                .selectable(false)
                .sense(Sense::hover()),
            );
            response.labelled_by(label.id)
        },
    );
    // Keep the dial's identity and calibrated arc while making the caption an
    // equally accessible context target. No extra value mapper is introduced.
    let id = row.inner.id;
    let response = row
        .inner
        .union(ui.interact(row.response.rect, id, Sense::click_and_drag()));
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*pan), "Pan"));
    crate::hover_text(
        response,
        format!(
            "Stereo balance · {} · {}%\nDrag up/right to increase; down/left to decrease. Shift for fine adjustment.\nRight-click for exact value entry and automation",
            crate::channel_pan_label(*pan),
            *pan * 100.0
        ),
    )
}

pub struct MixerMuteSoloResponse {
    pub mute: Response,
    pub solo: Response,
}

/// Independent, equally sized channel switches. Native buttons provide focus,
/// keyboard activation and disabled behavior; selected styling exposes state.
pub fn mixer_mute_solo(
    ui: &mut Ui,
    mute: &mut bool,
    solo: &mut bool,
    width: f32,
) -> MixerMuteSoloResponse {
    let gap = crate::tokens::SPACE_XS;
    let button_width = (width - gap) * 0.5;
    ui.allocate_ui_with_layout(
        Vec2::new(width, crate::tokens::CONTROL_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.spacing_mut().button_padding.x = crate::tokens::SPACE_SM;
            MixerMuteSoloResponse {
                mute: mixer_switch(ui, mute, "Mute", button_width, true),
                solo: mixer_switch(ui, solo, "Solo", button_width, false),
            }
        },
    )
    .inner
}

fn mixer_switch(ui: &mut Ui, value: &mut bool, label: &str, width: f32, mute: bool) -> Response {
    let colors = crate::theme::palette(ui.ctx());
    let accent = if mute { colors.orange } else { colors.accent };
    let stroke = if *value {
        accent
    } else {
        colors.line.lerp_to_gamma(colors.muted, 0.5)
    };
    let mut response = ui.add_sized(
        Vec2::new(width, crate::tokens::CONTROL_HEIGHT),
        crate::IconButton::new(egui::RichText::new(label).strong())
            .selected(*value)
            .fill(if *value {
                colors.panel.lerp_to_gamma(accent, 0.3)
            } else {
                colors.control_fill
            })
            .stroke(Stroke::new(
                if *value {
                    crate::tokens::FOCUS_WIDTH
                } else {
                    crate::tokens::BORDER_WIDTH
                },
                stroke,
            )),
    );
    if response.clicked() && ui.is_enabled() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
    });
    response.on_hover_text(if mute {
        "Mute this channel"
    } else {
        "Solo this channel"
    })
}

/// Stereo post-processing peaks on a fixed −60..0 dBFS scale. Readings are
/// sanitized for painting only; no smoothing, decay or audio state is invented.
pub fn mixer_level_meter(ui: &mut Ui, peaks: [f32; 2], width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width.max(1.0), MIXER_LEVEL_METER_HEIGHT),
        Sense::hover(),
    );
    let colors = crate::theme::palette(ui.ctx());
    let caption_font = FontId::proportional(crate::tokens::FONT_SCALE);
    let peaks = peaks.map(|peak| if peak.is_finite() { peak.max(0.0) } else { 0.0 });
    let db = peaks.map(|peak| 20.0 * peak.log10());
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    painter.text(
        rect.left_top(),
        Align2::LEFT_TOP,
        "Level",
        caption_font.clone(),
        colors.muted,
    );
    painter.text(
        rect.right_top(),
        Align2::RIGHT_TOP,
        "dBFS",
        caption_font.clone(),
        colors.muted,
    );
    let rail_left = (rect.left() + crate::tokens::SPACE_MD).min(rect.right());
    let rail_right = rect.right();
    let rail_width = (rail_right - rail_left).max(0.0);
    for (index, peak) in peaks.into_iter().enumerate() {
        let y = rect.top() + crate::tokens::SPACE_LG + index as f32 * crate::tokens::SPACE_MD;
        let rail = Rect::from_min_size(
            egui::pos2(rail_left, y),
            Vec2::new(rail_width, crate::tokens::SPACE_XS),
        );
        painter.text(
            egui::pos2(rect.left(), rail.center().y),
            Align2::LEFT_CENTER,
            if index == 0 { "L" } else { "R" },
            caption_font.clone(),
            colors.muted,
        );
        painter.rect_filled(rail, 1.0, colors.bg);
        let fraction = ((db[index] + 60.0) / 60.0).clamp(0.0, 1.0);
        if fraction > 0.0 {
            painter.rect_filled(
                Rect::from_min_size(rail.min, Vec2::new(rail.width() * fraction, rail.height())),
                1.0,
                if peak >= 1.0 {
                    colors.orange
                } else {
                    colors.accent
                },
            );
        }
    }
    for (caption, fraction, align) in [
        ("−60", 0.0, Align2::LEFT_BOTTOM),
        ("−30", 0.5, Align2::CENTER_BOTTOM),
        ("0", 1.0, Align2::RIGHT_BOTTOM),
    ] {
        painter.text(
            egui::pos2(rail_left + rail_width * fraction, rect.bottom()),
            align,
            caption,
            caption_font.clone(),
            colors.muted,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            ui.is_enabled(),
            format!("Stereo peak: L {:.1} dBFS, R {:.1} dBFS", db[0], db[1]),
        )
    });
    response.on_hover_text(format!(
        "Channel output level · post-fader\nL {:.1} dBFS · R {:.1} dBFS",
        db[0], db[1]
    ))
}
