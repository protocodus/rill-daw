//! Compact, non-modal feedback with bounded text and reachable actions.
use crate::{Icon, action_row, text::TextRole, theme, tokens};
use egui::{InnerResponse, Sense, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Busy,
    Success,
    Warning,
    Error,
}

/// Actions are supplied right to left, as in `action_row`. The full message is
/// available on hover; long paths never move the actions outside the viewport.
pub fn notice<R>(
    ui: &mut Ui,
    kind: NoticeKind,
    message: &str,
    actions: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.set_min_height(tokens::CONTROL_HEIGHT);
    let row = action_row(ui, actions, |ui| {
        if kind == NoticeKind::Busy {
            ui.spinner();
        } else {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            let colors = theme::palette(ui.ctx());
            let (icon, color) = if kind == NoticeKind::Success {
                (Icon::Check, colors.accent)
            } else {
                (Icon::Warning, colors.orange)
            };
            icon.paint(ui.painter(), rect, color);
        }
        ui.add(egui::Label::new(TextRole::Body.rich(ui.ctx(), message)).truncate())
            .on_hover_text(message);
    });
    InnerResponse::new(row.inner.0, row.response)
}
