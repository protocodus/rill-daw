//! Document history shortcuts from focused, non-text parameter controls.
use egui::{Context, Id, Response};

fn focus_key() -> Id {
    Id::new("rill_parameter_history_focus")
}

/// Register only the rail, dial or relative-value target, never its numeric
/// editor. Keyboard dispatch runs before drawing, so the preceding frame's
/// focused target remains eligible until the next frame refreshes it.
pub(crate) fn register_parameter_history_focus(response: &Response) {
    if response.enabled() && response.has_focus() {
        let frame = response.ctx.cumulative_frame_nr();
        response
            .ctx
            .data_mut(|data| data.insert_temp(focus_key(), (frame, response.id)));
    }
}

/// A focused parameter edits song history; exact-entry fields keep native text
/// Undo/Redo. Hidden controls and stale focus registrations cannot claim keys.
pub fn parameter_owns_document_history(ctx: &Context) -> bool {
    !egui::Popup::is_any_open(ctx)
        && ctx
            .data(|data| data.get_temp::<(u64, Id)>(focus_key()))
            .is_some_and(|(frame, id)| {
                ctx.cumulative_frame_nr() <= frame.saturating_add(1)
                    && ctx.memory(|memory| memory.has_focus(id))
            })
}
