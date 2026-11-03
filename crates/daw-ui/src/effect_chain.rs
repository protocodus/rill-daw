//! Insert-chain structure shared by channel and device views. The caller owns
//! plug-in state, ordering and commands; these components only arrange controls.
use crate::{text, theme, tokens};
use egui::{InnerResponse, Response, Ui};

/// The chain's heading with its active count; the processing order is in the
/// count's tooltip rather than a persistent sentence.
pub fn effect_chain_header(ui: &mut Ui, count: usize, active: usize) -> Response {
    crate::action_row(
        ui,
        |ui| {
            text::note(ui, format!("{active} / {count} active")).on_hover_text(
                "Input → Inserts → Volume & pan\nEffects process the signal from top to bottom, before channel volume and pan.",
            )
        },
        |ui| text::label(ui, "Inserts"),
    )
    .inner
    .0
}

/// One insert: an identity row (reorder handle, enable switch with the name,
/// the plug-in model on a second line) and a non-wrapping action row.
/// Bypass reads the same everywhere: muted name, gray rail and a tooltip.
pub fn effect_slot<T: Send + Sync + 'static, R>(
    ui: &mut Ui,
    payload: T,
    index: usize,
    name: &str,
    model: &str,
    enabled: &mut bool,
    actions: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<(Response, R)> {
    let result = crate::card(ui.ctx()).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = tokens::SPACE_XS;
        let colors = theme::palette(ui.ctx());
        let switch = ui
            .horizontal(|ui| {
                crate::reorder_handle(ui, payload);
                let mut title = egui::text::LayoutJob::simple(
                    format!("{:02}  {name}", index + 1),
                    text::TextRole::Label.font(),
                    if *enabled { colors.text } else { colors.muted },
                    (ui.available_width() - tokens::SWITCH_WIDTH - tokens::SPACE_SM).max(0.0),
                );
                title.wrap.max_rows = 1;
                crate::switch(ui, enabled, title).on_hover_text(if *enabled {
                    name.to_owned()
                } else {
                    format!("{name} · Bypassed")
                })
            })
            .inner;
        text::truncated(ui, text::TextRole::Note, model).on_hover_text(model);
        let actions = ui.horizontal(actions).inner;
        (switch, actions)
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
