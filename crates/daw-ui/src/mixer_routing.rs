//! Compact mixer routing controls. Destination color identifies the receiving
//! channel; routing changes remain in the application's focused routing view.
use crate::{theme, tokens};
use egui::{Color32, Response, Ui, Vec2};

/// A non-interactive master identity with the same footprint as channel titles.
/// Its neutral raised surface separates the final output from colored channels.
pub fn mixer_master_title(ui: &mut Ui, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, tokens::CONTROL_HEIGHT),
        egui::Sense::hover(),
    );
    let colors = theme::palette(ui.ctx());
    ui.painter()
        .rect_filled(rect, tokens::RADIUS, colors.raised);
    ui.painter().rect_stroke(
        rect,
        tokens::RADIUS,
        egui::Stroke::new(1.0_f32, colors.line),
        egui::StrokeKind::Inside,
    );
    crate::text::paint_row(
        ui.painter(),
        rect.shrink2(Vec2::new(tokens::SPACE_MD, 0.0)),
        crate::text::TextRole::Label,
        "Master",
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), "Master")
    });
    response.on_hover_text("Final stereo output · receives all channels routed to Master")
}

/// Main output retains a complete native label and a dedicated destination
/// marker. Its arrow and color communicate signal direction without relying on
/// the source channel's tint. Long labels expose their full text on hover.
pub fn mixer_output_button(
    ui: &mut Ui,
    width: f32,
    label: &str,
    destination: &str,
    destination_color: Option<Color32>,
) -> Response {
    let colors = theme::palette(ui.ctx());
    let response = ui
        .scope(|ui| {
            // Keep the destination readable within a channel strip; compact
            // horizontal padding leaves the full-height mouse target unchanged.
            ui.spacing_mut().button_padding.x = tokens::SPACE_XS;
            ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
            ui.add_sized(
                Vec2::new(width, tokens::CONTROL_HEIGHT),
                crate::button_widget(egui::RichText::new(label).small())
                    .truncate()
                    .presentation(),
            )
        })
        .inner;
    let rail = egui::Rect::from_min_max(
        response.rect.left_top() + Vec2::new(1.0, tokens::SPACE_XS),
        response.rect.left_bottom() + Vec2::new(tokens::SPACE_XS, -tokens::SPACE_XS),
    );
    ui.painter()
        .rect_filled(rail, 1.0, destination_color.unwrap_or(colors.accent));
    response.on_hover_text(format!(
        "Main output → {destination}\nAfter inserts, volume and pan · Click to edit routing"
    ))
}

/// One parallel path in the compact sends control. Names and colors are borrowed
/// from the project so rendering does not retain or copy a second routing graph.
pub struct MixerSend<'a> {
    pub destination: &'a str,
    pub color: Color32,
    pub gain: f32,
    pub pre_fader: bool,
}

/// Auxiliary routing has its own mouse target, separate from the main output.
/// The single-send case names its bus; a larger set exposes all destinations,
/// pre/post placement, and levels on hover without growing the strip.
pub fn mixer_sends_button(ui: &mut Ui, width: f32, sends: &[MixerSend<'_>]) -> Response {
    let label = match sends {
        [] => "Add send…".to_owned(),
        [send] => format!("Send · {}", send.destination),
        _ => format!("Sends · {}", sends.len()),
    };
    let response = ui.add_sized(
        Vec2::new(width, tokens::CONTROL_HEIGHT),
        crate::IconButton::with_icon(crate::Icon::Routing, egui::RichText::new(label).small())
            .truncate(),
    );
    if let [send] = sends {
        let rail = egui::Rect::from_min_max(
            response.rect.left_top() + Vec2::new(1.0, tokens::SPACE_XS),
            response.rect.left_bottom() + Vec2::new(tokens::SPACE_XS, -tokens::SPACE_XS),
        );
        ui.painter().rect_filled(rail, 1.0, send.color);
    }
    response.on_hover_ui(|ui| {
        if sends.is_empty() {
            ui.label("Add a parallel send to a bus");
        } else {
            crate::text::label(ui, "Auxiliary sends");
            for send in sends {
                let level = if send.gain > 0.0 {
                    format!("{:.1} dB", 20.0 * send.gain.log10())
                } else {
                    "−∞ dB".to_owned()
                };
                ui.label(format!(
                    "{} · {} · {level}",
                    send.destination,
                    if send.pre_fader {
                        "Pre-fader"
                    } else {
                        "Post-fader"
                    }
                ));
            }
            crate::text::note(ui, "Click to edit sends");
        }
    })
}
