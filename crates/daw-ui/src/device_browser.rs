//! Discovery surfaces shared by the device browser and other device choosers.
//! Catalogue state, filtering, organization and insertion belong to the caller.
use crate::{Icon, text, text::TextRole, tokens};
use egui::{Response, TextureHandle, Ui};

pub struct DeviceBrowserSearchResponse {
    pub search: Response,
    pub clear: Response,
    pub cleared: bool,
}

/// A dedicated, full-width search row. Clear removes only the query and returns
/// keyboard focus to the input; the caller keeps its existing filters intact.
pub fn device_browser_search(
    ui: &mut Ui,
    query: &mut String,
    hint: &str,
) -> DeviceBrowserSearchResponse {
    let has_query = !query.is_empty();
    let row = crate::action_row(
        ui,
        |ui| {
            ui.set_min_height(tokens::CONTROL_HEIGHT);
            ui.add_enabled(has_query, crate::view_button_widget("Clear"))
                .on_hover_text("Clear the search")
        },
        |ui| {
            crate::text_field(
                ui,
                query,
                hint,
                (ui.available_width() - 2.0 * tokens::BUTTON_PADDING_X).max(0.0),
            )
        },
    );
    let (clear, search) = row.inner;
    let cleared = clear.clicked();
    if cleared {
        query.clear();
        search.request_focus();
    }
    DeviceBrowserSearchResponse {
        search,
        clear,
        cleared,
    }
}

/// The count describes the filtered result set. Optional progress or sorting
/// context is secondary and cannot expand the row beyond its width budget.
pub fn device_browser_results_heading(
    ui: &mut Ui,
    count: usize,
    singular: &str,
    plural: &str,
    status: Option<&str>,
) -> Response {
    ui.scope(|ui| {
        // This read-only line has no interaction targets. Do not inherit the
        // 34-point control minimum and spend a compact catalogue row on labels.
        ui.spacing_mut().interact_size.y = tokens::SPACE_LG;
        ui.horizontal(|ui| {
            ui.set_min_height(tokens::SPACE_LG);
            text::truncated(ui, TextRole::Label, text::count(count, singular, plural));
            if let Some(status) = status.filter(|status| !status.is_empty()) {
                ui.add_space(tokens::SPACE_SM);
                text::truncated(ui, TextRole::Note, status);
            }
        })
        .response
    })
    .inner
}

/// A stable two-line footer identity. Both lines keep the full text in their
/// accessibility/hover labels while truncating visually inside the action row.
pub fn device_browser_selection_summary(ui: &mut Ui, name: &str, destination: &str) -> Response {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        text::truncated(ui, TextRole::Label, name);
        text::truncated(ui, TextRole::Note, destination);
    })
    .response
}

pub struct DeviceBrowserIdentity<'a> {
    /// Empty when the surrounding popup title already identifies the device.
    pub name: &'a str,
    pub developer: &'a str,
    pub kind_format: &'a str,
    pub categories: &'a str,
    pub preview: Option<&'a TextureHandle>,
    pub icon: Icon,
}

/// Read-only identity for a side inspector or compact Details popup. An absent
/// editor screenshot consumes no extra space and never implies a load failure.
pub fn device_browser_identity(ui: &mut Ui, identity: &DeviceBrowserIdentity<'_>) -> Response {
    ui.vertical(|ui| {
        if !identity.name.is_empty() {
            text::bounded_block(ui, TextRole::Title, identity.name, 2);
        }
        text::truncated(
            ui,
            TextRole::Note,
            if identity.developer.is_empty() {
                "Unknown developer"
            } else {
                identity.developer
            },
        );
        text::truncated(ui, TextRole::Note, identity.kind_format);
        if !identity.categories.is_empty() {
            text::bounded_block(ui, TextRole::Body, identity.categories, 2);
        }
        if let Some(preview) = identity.preview {
            ui.add_space(tokens::SPACE_SM);
            let width = ui.available_width();
            crate::device_preview(
                ui,
                Some(preview),
                identity.icon,
                egui::vec2(width, (width * 0.55).min(tokens::SELECT_WIDTH)),
            );
        }
    })
    .response
}
