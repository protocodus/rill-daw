//! Compact mixer routing controls. Destination color identifies the receiving
//! channel; routing changes remain in the application's focused routing view.
use crate::{theme, tokens};
use egui::{Color32, Response, Ui};

/// Main output named by its destination only ("Master", "Drum bus"), as a
/// quiet row with the destination's color rail. Long names show on hover.
pub fn mixer_output_button(
    ui: &mut Ui,
    width: f32,
    destination: &str,
    destination_color: Option<Color32>,
) -> Response {
    let colors = theme::palette(ui.ctx());
    crate::mixer_row(
        ui,
        width,
        tokens::CONTROL_HEIGHT,
        destination,
        crate::MixerRowText::Body,
        Some(destination_color.unwrap_or(colors.accent)),
        None,
    )
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

/// One compact send: destination rail, name, "PRE" when pre-fader and the
/// level in dB. The whole row is one mouse target.
pub fn mixer_send_row(ui: &mut Ui, width: f32, send: &MixerSend<'_>) -> Response {
    let colors = theme::palette(ui.ctx());
    let level = send_level(send.gain);
    let trailing = if send.pre_fader {
        format!("PRE {level}")
    } else {
        level.clone()
    };
    let placement = if send.pre_fader {
        "Pre-fader"
    } else {
        "Post-fader"
    };
    crate::mixer_row(
        ui,
        width,
        crate::MIXER_SEND_ROW_HEIGHT,
        send.destination,
        crate::MixerRowText::Caption,
        Some(send.color),
        Some((&trailing, colors.muted)),
    )
    .on_hover_text(format!(
        "Send → {} · {placement} · {level} dB\nClick to edit sends",
        send.destination,
    ))
}

/// A summary row for sends beyond the visible rows ("+2 more").
pub fn mixer_more_sends(ui: &mut Ui, width: f32, hidden: usize) -> Response {
    crate::mixer_row(
        ui,
        width,
        crate::MIXER_SEND_ROW_HEIGHT,
        &format!("+{hidden} more"),
        crate::MixerRowText::Placeholder,
        None,
        None,
    )
    .on_hover_text("Click to edit all sends")
}

/// The add-send opener at the end of the sends list.
pub fn mixer_add_send_button(ui: &mut Ui, width: f32) -> Response {
    crate::mixer_row(
        ui,
        width,
        crate::MIXER_SEND_ROW_HEIGHT,
        "Add send…",
        crate::MixerRowText::Action,
        None,
        None,
    )
    .on_hover_text("Add a parallel send to a bus")
}
