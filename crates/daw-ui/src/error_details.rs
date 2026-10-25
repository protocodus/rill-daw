//! On-demand diagnostic details, without replacing the caller's workspace.
use crate::{text, tokens};
use egui::{Context, Id, Rect, Response};

pub struct ErrorDetailsResponse {
    pub response: Response,
    pub details_viewport: Rect,
    pub copy: Response,
    pub close: Response,
    pub copied: bool,
    pub closed: bool,
}

/// Show a focused diagnostic snapshot. The caller owns whether it is open and
/// gates raw keyboard/drop handlers, as for [`crate::modal`]. Contextual titles are
/// bounded to one row with their full text on hover; put operation, device, paths,
/// and causes in the untruncated `details` text.
/// Copy always includes the exact supplied details, including offscreen lines.
/// Close, Escape, and clicking the backdrop dismiss without changing that snapshot.
pub fn error_details(
    ctx: &Context,
    title: &str,
    summary: &str,
    details: &str,
) -> ErrorDetailsResponse {
    let modal = crate::modal::modal_with_title_rows(
        ctx,
        Id::new("rill_error_details"),
        title,
        Some(1),
        |ui| {
            text::bounded_block(ui, text::TextRole::Subtitle, summary, 2);
            ui.add_space(tokens::SPACE_MD);
            // Reserve the footer before sizing the scroll area. Long messages and
            // paths must never move Copy or Close beyond the visible window.
            let details_height = (ctx.content_rect().height()
                - 4.0 * tokens::POPUP_MARGIN
                - ui.min_rect().height()
                - tokens::SPACE_MD
                - tokens::CONTROL_HEIGHT
                - 2.0 * tokens::SPACE_SM)
                .clamp(36.0, 360.0);
            let scroll = crate::inset(ctx)
                .inner_margin(tokens::SPACE_SM)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        // A different diagnostic opens at its first line, rather
                        // than retaining another error's scroll offset.
                        .id_salt(("diagnostic_details", details))
                        .max_height(details_height)
                        .min_scrolled_height(0.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            text::selectable_block(ui, text::TextRole::Body, details)
                        })
                })
                .inner;
            ui.add_space(tokens::SPACE_MD);
            let buttons = crate::action_row(
                ui,
                |ui| {
                    let close = crate::button(ui, "Close");
                    let copy = crate::button(ui, "Copy details");
                    if copy.clicked() {
                        ctx.copy_text(details.to_owned());
                    }
                    (copy, close)
                },
                |_| {},
            )
            .inner
            .0;
            (scroll.inner_rect, buttons.0, buttons.1)
        },
    );
    let closed = modal.should_close() || modal.inner.2.clicked();
    ErrorDetailsResponse {
        response: modal.response,
        details_viewport: modal.inner.0,
        copied: modal.inner.1.clicked(),
        copy: modal.inner.1,
        close: modal.inner.2,
        closed,
    }
}
