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
    // Imported names can exceed the inline keyboard's limit. Reserve a bounded
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
    let width = (ctx.content_rect().width() - 4.0 * tokens::POPUP_MARGIN).clamp(180.0, 460.0);
    egui::Modal::new(id)
        .backdrop_color(crate::theme::palette(ctx).popup_scrim)
        .frame(
            panel(ctx)
                .inner_margin(tokens::POPUP_MARGIN)
                .stroke(ctx.style().visuals.window_stroke)
                .shadow(ctx.style().visuals.window_shadow),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            if let Some(rows) = title_rows {
                text::bounded_block(ui, text::TextRole::Title, title, rows);
            } else {
                text::title(ui, title);
            }
            ui.add_space(tokens::SPACE_MD);
            contents(ui)
        })
}
