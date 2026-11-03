//! Compact mixer routing controls. Destination color identifies the receiving
//! channel; routing changes remain in the application's focused routing view.
use crate::{theme, tokens};
use egui::{Color32, FontId, Rect, Response, Sense, Ui, Vec2};

fn destination_rail(ui: &Ui, rect: Rect, color: Color32) {
    let rail = Rect::from_min_max(
        rect.left_top() + Vec2::new(1.0, tokens::SPACE_XS),
        rect.left_bottom() + Vec2::new(tokens::SPACE_XS, -tokens::SPACE_XS),
    );
    ui.painter().rect_filled(rail, 1.0, color);
}

/// Main output named by its destination only ("Master", "Drum bus"), with the
/// destination's color rail. Long names expose their full text on hover.
pub fn mixer_output_button(
    ui: &mut Ui,
    width: f32,
    destination: &str,
    destination_color: Option<Color32>,
) -> Response {
    let colors = theme::palette(ui.ctx());
    let response = ui.add_sized(
        Vec2::new(width, tokens::CONTROL_HEIGHT),
        crate::button_widget(destination).truncate().presentation(),
    );
    destination_rail(
        ui,
        response.rect,
        destination_color.unwrap_or(colors.accent),
    );
    response
}

/// One parallel path in the compact sends list. Names and colors are borrowed
/// from the project so rendering does not retain or copy a second routing graph.
pub struct MixerSend<'a> {
    pub destination: &'a str,
    pub color: Color32,
    pub gain: f32,
    pub pre_fader: bool,
}

fn send_level(gain: f32) -> String {
    if gain > 0.0 {
        format!("{:.1}", 20.0 * gain.log10()).replace('-', "−")
    } else {
        "−∞".to_owned()
    }
}

/// One compact send: destination rail, name, a PRE badge when pre-fader and
/// the level in dB. The whole row is one mouse target.
pub fn mixer_send_row(ui: &mut Ui, width: f32, send: &MixerSend<'_>) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, crate::MIXER_SEND_ROW_HEIGHT),
        Sense::click(),
    );
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        if response.hovered() || response.has_focus() {
            ui.painter()
                .rect_filled(rect, tokens::RADIUS, colors.control_hover);
        }
        ui.painter().rect_filled(
            Rect::from_min_size(
                rect.left_top() + Vec2::new(1.0, 3.0),
                Vec2::new(tokens::SPACE_XS - 1.0, rect.height() - 6.0),
            ),
            1.0,
            send.color,
        );
        let font = FontId::proportional(tokens::FONT_CAPTION);
        let level = ui
            .painter()
            .layout_no_wrap(send_level(send.gain), font.clone(), colors.muted);
        let level_left = rect.right() - tokens::SPACE_XS - level.size().x;
        ui.painter().galley(
            egui::pos2(level_left, rect.center().y - level.size().y * 0.5),
            level,
            colors.muted,
        );
        let mut name_right = level_left - tokens::SPACE_SM;
        if send.pre_fader {
            let badge = Rect::from_min_size(
                egui::pos2(name_right - 30.0, rect.center().y - 8.0),
                Vec2::new(30.0, 16.0),
            );
            ui.painter()
                .rect_filled(badge, 3.0, colors.raised.lerp_to_gamma(send.color, 0.16));
            ui.painter().rect_stroke(
                badge,
                3.0,
                egui::Stroke::new(tokens::BORDER_WIDTH, send.color.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                "PRE",
                FontId::proportional(tokens::FONT_SCALE),
                colors.text,
            );
            name_right = badge.left() - tokens::SPACE_XS;
        }
        let name_left = rect.left() + tokens::SPACE_SM + 1.0;
        let mut job = egui::text::LayoutJob::simple_singleline(
            send.destination.to_owned(),
            font,
            colors.text,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width((name_right - name_left).max(0.0));
        let name = ui.painter().layout_job(job);
        ui.painter().galley(
            egui::pos2(name_left, rect.center().y - name.size().y * 0.5),
            name,
            colors.text,
        );
    }
    let placement = if send.pre_fader {
        "Pre-fader"
    } else {
        "Post-fader"
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), send.destination)
    });
    response.on_hover_text(format!(
        "Send → {} · {placement} · {} dB\nClick to edit sends",
        send.destination,
        send_level(send.gain)
    ))
}

/// A summary row for sends beyond the visible rows ("+2 more").
pub fn mixer_more_sends(ui: &mut Ui, width: f32, hidden: usize) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, crate::MIXER_SEND_ROW_HEIGHT),
        Sense::click(),
    );
    let colors = theme::palette(ui.ctx());
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, tokens::RADIUS, colors.control_hover);
    }
    let text = format!("+{hidden} more");
    ui.painter().text(
        rect.left_center() + Vec2::new(tokens::SPACE_SM + 1.0, 0.0),
        egui::Align2::LEFT_CENTER,
        &text,
        FontId::proportional(tokens::FONT_CAPTION),
        colors.muted,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &text));
    response.on_hover_text("Click to edit all sends")
}

/// The add-send opener at the end of the sends list.
pub fn mixer_add_send_button(ui: &mut Ui, width: f32) -> Response {
    ui.add_sized(
        Vec2::new(width, tokens::CONTROL_HEIGHT),
        crate::IconButton::with_icon(crate::Icon::Routing, "Add send…")
            .action()
            .truncate(),
    )
    .on_hover_text("Add a parallel send to a bus")
}
