//! Deliberate exact-value entry for parameter context menus.
//! The caller owns the parameter; typing and dismissing this editor never edit it.
use crate::{text, tokens};
use egui::{Context, Id, Key, Modifiers, Rect, Ui};
use std::{hash::Hash, ops::RangeInclusive};

#[derive(Clone)]
struct ValueDraft {
    text: String,
    last_frame: u64,
    field: Rect,
    focus_pending: bool,
}

fn draft_id(ui: &Ui, id_salt: impl Hash) -> Id {
    ui.make_persistent_id(("exact_value_draft", id_salt))
}

pub(crate) fn clear_value_entry(ui: &Ui, id_salt: impl Hash) {
    ui.data_mut(|data| data.remove::<ValueDraft>(draft_id(ui, id_salt)));
}

/// A short, round-trip representation, including very small/large parameters.
pub fn exact_value_text(value: f64) -> String {
    if value != 0.0 && (value.abs() < 0.000_001 || value.abs() >= 1_000_000_000.0) {
        format!("{value:e}")
    } else {
        value.to_string()
    }
}

/// Active exact-entry input, available to application interaction diagnostics.
/// A previously closed editor stops being reported on the next frame.
pub fn value_entry_rect(ctx: &Context) -> Option<Rect> {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|data| data.get_temp::<(u64, Rect)>(Id::new("exact_value_input_rect")))
        .filter(|(painted, _)| frame <= painted.saturating_add(1))
        .map(|(_, rect)| rect)
}

/// Draw inside a control's existing context menu. A changed finite value is
/// returned only after Apply value (or Enter while the field owns focus).
/// Cancel/Escape and outside dismissal leave the caller's parameter untouched.
pub fn value_entry(
    ui: &mut Ui,
    id_salt: impl Hash,
    current: f64,
    range: RangeInclusive<f64>,
    unit: &str,
) -> Option<f64> {
    let id = draft_id(ui, id_salt);
    let frame = ui.ctx().cumulative_frame_nr();
    let previous = ui.data_mut(|data| {
        let draft = data.get_temp::<ValueDraft>(id);
        data.remove::<ValueDraft>(id);
        draft
    });
    let fresh = previous.as_ref().is_none_or(|draft| {
        frame > draft.last_frame.saturating_add(1)
            || ui.input(|input| {
                input.pointer.button_clicked(egui::PointerButton::Secondary)
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| !draft.field.contains(pos))
            })
    });
    let mut draft = if fresh {
        ValueDraft {
            text: exact_value_text(current),
            last_frame: frame,
            field: Rect::NOTHING,
            focus_pending: true,
        }
    } else {
        previous.expect("an existing draft is present")
    };
    let input_id = id.with("input");
    let focused = ui.is_enabled() && ui.memory(|memory| memory.has_focus(input_id));
    let enter = focused && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let escape = focused && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    let minimum = range.start().min(*range.end());
    let maximum = range.start().max(*range.end());
    let range_valid = range.start().is_finite() && range.end().is_finite();
    let label = text::label(ui, "Value");
    text::note(
        ui,
        format!(
            "Range: {} to {}{}{}",
            exact_value_text(minimum),
            exact_value_text(maximum),
            if unit.is_empty() { "" } else { " " },
            unit,
        ),
    );
    let response = ui
        .add(
            egui::TextEdit::singleline(&mut draft.text)
                .id(input_id)
                .desired_width(220.0_f32.min(ui.available_width()))
                .char_limit(128)
                .margin(egui::vec2(
                    tokens::BUTTON_PADDING_X,
                    tokens::BUTTON_PADDING_Y,
                ))
                .vertical_align(egui::Align::Center)
                .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT)),
        )
        .labelled_by(label.id);
    draft.field = response.rect;
    draft.last_frame = frame;
    // Popups first measure their contents in a disabled sizing pass. Defer
    // focus until the field is genuinely interactive, rather than consuming
    // the initial selection during that invisible pass.
    if draft.focus_pending && ui.is_enabled() {
        response.request_focus();
        let mut state = egui::TextEdit::load_state(ui.ctx(), input_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::default(),
                egui::text::CCursor::new(draft.text.chars().count()),
            )));
        state.store(ui.ctx(), input_id);
        draft.focus_pending = false;
    }
    ui.data_mut(|data| data.insert_temp(Id::new("exact_value_input_rect"), (frame, response.rect)));
    let parsed = draft
        .text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite());
    let valid = parsed.filter(|value| range_valid && (minimum..=maximum).contains(value));
    if parsed.is_none() {
        text::note(ui, "Enter a finite number.");
    } else if valid.is_none() {
        text::note(ui, "The value must be inside the range.");
    }
    let (apply, cancel) = ui
        .horizontal(|ui| {
            let apply = ui
                .add_enabled_ui(valid.is_some(), |ui| {
                    crate::primary_button(ui, "Apply value")
                })
                .inner;
            let cancel = crate::button(ui, "Cancel");
            (apply.clicked(), cancel.clicked())
        })
        .inner;
    if ui.is_enabled() && (cancel || escape || ((apply || enter) && valid.is_some())) {
        ui.data_mut(|data| data.remove::<(u64, Rect)>(Id::new("exact_value_input_rect")));
        ui.close();
        if !cancel && !escape {
            return valid.filter(|value| *value != current);
        }
    } else {
        ui.data_mut(|data| data.insert_temp(id, draft));
    }
    None
}
