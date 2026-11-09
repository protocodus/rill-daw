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
        // Rows are related controls of one form: the 8-point step keeps a
        // dialog compact while every 34-point target stays separate.
        .spacing(egui::vec2(tokens::SPACE_LG, tokens::SPACE_SM))
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

/// A form select at the shared field width, so stacked fields share both
/// edges. Long choices truncate instead of widening the dialog.
pub fn form_select(
    id_salt: impl Hash,
    selected_text: impl Into<egui::WidgetText>,
) -> egui::ComboBox {
    crate::select(id_salt, selected_text)
        .width(tokens::FORM_FIELD_WIDTH)
        .truncate()
}

/// Text inputs add their padding outside the requested width. Request this
/// width so a text field shares both edges with the selects above and below.
pub const FORM_TEXT_WIDTH: f32 = tokens::FORM_FIELD_WIDTH - 2.0 * tokens::BUTTON_PADDING_X;

/// A single-line text input at the shared form field width.
pub fn form_text_field(ui: &mut Ui, value: &mut String, hint: &str) -> egui::Response {
    crate::text_field(ui, value, hint, FORM_TEXT_WIDTH)
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
