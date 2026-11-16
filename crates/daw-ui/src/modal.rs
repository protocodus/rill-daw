//! Focused decisions that block background input without replacing app navigation.
use crate::{panel, text, tokens};
use egui::{Context, Id, ModalResponse, Ui};

/// A centered, bounded dialog with shared typography and window treatment.
/// Callers decide which dismissals mean Cancel; backdrop clicks never approve
/// an action. Applications must gate raw keyboard/drop handlers while visible,
/// and skip background edits in the frame that requests the modal.
pub fn modal<R>(
    ctx: &Context,
    id: Id,
    title: &str,
    contents: impl FnOnce(&mut Ui) -> R,
) -> ModalResponse<R> {
    // Imported names can exceed the name field's limit. Reserve a bounded
    // heading so the decision body and actions retain their compact layout;
    // bounded_block keeps the complete source text and exposes it on hover.
    modal_with_title_rows(ctx, id, title, Some(2), contents)
}

pub(crate) fn modal_with_title_rows<R>(
    ctx: &Context,
    id: Id,
    title: &str,
    title_rows: Option<usize>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> ModalResponse<R> {
    let painter = ctx.layer_painter(egui::Modal::default_area(id).layer());
    crate::backdrop_blur::paint_modal_blur(ctx, &painter);
    let width = (ctx.content_rect().width() - 4.0 * tokens::POPUP_MARGIN)
        .clamp(tokens::FORM_LABEL_WIDTH, tokens::POPUP_WIDTH_SM);
    egui::Modal::new(id)
        .backdrop_color(crate::theme::palette(ctx).popup_scrim)
        .frame(
            panel(ctx)
                .inner_margin(tokens::WINDOW_MARGIN)
                .stroke(ctx.style().visuals.window_stroke)
                .shadow(ctx.style().visuals.window_shadow),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            // The same centered title row and separator as a popup window, so
            // blocking decisions and utility windows read as one family.
            ui.vertical_centered(|ui| {
                if let Some(rows) = title_rows {
                    text::bounded_block(ui, text::TextRole::Title, title, rows);
                } else {
                    text::title(ui, title);
                }
            });
            ui.add(egui::Separator::default().spacing(tokens::SPACE_MD));
            contents(ui)
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GhostChoice {
    Propagate,
    Disconnect,
    DeleteCopies,
}

pub enum GhostDecision {
    Cancel,
    Apply(GhostChoice),
}

/// A destructive linked-clip edit needs an explicit scope. Escape and a click
/// outside always cancel. No action is preselected.
pub fn ghost_change_dialog(ctx: &Context, action: &str, count: usize) -> Option<GhostDecision> {
    let response = modal(ctx, Id::new("ghost_change"), "Linked copies", |ui| {
        let copies = if count == 1 { "copy" } else { "copies" };
        text::body(
            ui,
            format!("{action} affects {count} other linked {copies}."),
        );
        ui.add_space(tokens::SPACE_SM);
        if crate::button(ui, "Cancel").clicked() {
            return Some(GhostDecision::Cancel);
        }
        for (label, choice, help) in [
            (
                "Propagate change",
                GhostChoice::Propagate,
                "Apply this edit to all linked occurrences.",
            ),
            (
                "Apply and disconnect",
                GhostChoice::Disconnect,
                "Edit the selection and detach it from the other copies.",
            ),
            (
                "Apply and delete all copies",
                GhostChoice::DeleteCopies,
                "Keep the edited selection and remove the other linked copies.",
            ),
        ] {
            if crate::button(ui, label).on_hover_text(help).clicked() {
                return Some(GhostDecision::Apply(choice));
            }
        }
        None
    });
    let cancelled = response.should_close();
    response
        .inner
        .or_else(|| cancelled.then_some(GhostDecision::Cancel))
}
