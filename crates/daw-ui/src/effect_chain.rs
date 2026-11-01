//! Insert-chain structure shared by channel and device views. The caller owns
//! plug-in state, ordering and commands; these components only arrange controls.
use crate::{text, theme, tokens};
use egui::{InnerResponse, Response, Ui};

/// A concise, read-only explanation of the channel's processing order.
pub fn effect_chain_header(ui: &mut Ui, count: usize, active: usize) -> Response {
    crate::action_row(
        ui,
        |ui| text::note(ui, format!("{active} / {count} active")),
        |ui| text::label(ui, "Signal chain"),
    );
    text::note(ui, "Input → Inserts → Volume & pan").on_hover_text(
        "Effects process the signal from top to bottom, before channel volume and pan.",
    )
}

/// One insert has a bounded identity row and a separate, wrapping action row.
/// The enable switch keeps its complete accessible name even when truncated.
/// Bypassed devices retain their actions and a visible status, without dimming
/// the entire card or making its text difficult to read.
pub fn effect_slot<T: Send + Sync + 'static, R>(
    ui: &mut Ui,
    payload: T,
    index: usize,
    name: &str,
    enabled: &mut bool,
    actions: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<(Response, R)> {
    let result = crate::card(ui.ctx()).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = tokens::SPACE_MD;
        let status = if *enabled { "Active" } else { "Bypassed" };
        let header = crate::action_row(
            ui,
            |ui| text::note(ui, status),
            |ui| {
                crate::reorder_handle(ui, payload);
                let colors = theme::palette(ui.ctx());
                let mut title = egui::text::LayoutJob::simple(
                    format!("{:02}  {name}", index + 1),
                    text::TextRole::Label.font(),
                    colors.text,
                    (ui.available_width() - tokens::SWITCH_WIDTH - tokens::SPACE_SM).max(0.0),
                );
                title.wrap.max_rows = 1;
                crate::switch(ui, enabled, title).on_hover_text(name)
            },
        );
        let actions = ui.horizontal_wrapped(actions).inner;
        (header.inner.1, actions)
    });
    let colors = theme::palette(ui.ctx());
    let rect = result.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, tokens::SPACE_SM),
            egui::vec2(2.0, (rect.height() - 2.0 * tokens::SPACE_SM).max(0.0)),
        ),
        1.0,
        if *enabled { colors.accent } else { colors.line },
    );
    result
}

/// Empty insert chains keep their purpose and insertion action discoverable.
pub fn empty_effect_chain(ui: &mut Ui) -> Response {
    crate::inset(ui.ctx())
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            text::label(ui, "No effects");
            text::note(ui, "Add an insert to shape this channel.")
        })
        .response
}
