//! Bounded settings layouts with native input, focus, and accessibility.
use crate::{text, tokens};
use egui::{InnerResponse, Ui};
use std::{hash::Hash, path::Path};

/// Aligned label/control rows. Call `ui.end_row()` after each control.
pub fn form_grid<R>(
    ui: &mut Ui,
    id_salt: impl Hash,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    egui::Grid::new(id_salt)
        .num_columns(2)
        .min_col_width(tokens::SELECT_WIDTH)
        .spacing(egui::vec2(tokens::SPACE_LG, tokens::SPACE_MD))
        .min_row_height(tokens::CONTROL_HEIGHT)
        .show(ui, contents)
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
