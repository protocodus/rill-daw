//! Bounded settings layouts with native input, focus, and accessibility.
use crate::{text, tokens};
use egui::{InnerResponse, Ui};
use std::{hash::Hash, path::Path};

/// Aligned label/control rows. Call `ui.end_row()` after each control.
/// Caption rows with [`form_label`] so every form starts its controls at the
/// same x; size selects and text fields to `tokens::FORM_FIELD_WIDTH`.
pub fn form_grid<R>(
    ui: &mut Ui,
    id_salt: impl Hash,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    egui::Grid::new(id_salt)
        .num_columns(2)
        .min_col_width(tokens::FORM_LABEL_WIDTH)
        .spacing(egui::vec2(tokens::SPACE_LG, tokens::SPACE_MD))
        .min_row_height(tokens::CONTROL_HEIGHT)
        .show(ui, contents)
}

/// A form row caption in the fixed label column. Longer captions truncate
/// and show their full text on hover instead of widening the column.
pub fn form_label(ui: &mut Ui, value: impl Into<String>) -> egui::Response {
    let value = value.into();
    ui.allocate_ui_with_layout(
        egui::vec2(tokens::FORM_LABEL_WIDTH, tokens::CONTROL_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_width(tokens::FORM_LABEL_WIDTH);
            // A truncated native label shows its full text on hover by itself.
            ui.add(egui::Label::new(text::TextRole::Label.rich(ui.ctx(), &value)).truncate())
        },
    )
    .inner
}

/// Keep actions reachable beside arbitrarily long paths. The path elides in
/// its remaining width and exposes its complete Unicode text on hover.
pub fn path_row<R>(
    ui: &mut Ui,
    path: &Path,
    actions: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let row = crate::action_row(ui, actions, |ui| {
        text::truncated(ui, text::TextRole::Body, path.display().to_string())
    });
    InnerResponse::new(row.inner.0, row.response)
}
