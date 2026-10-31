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
    value_entry_contents(
        ui,
        id_salt,
        current,
        range,
        ValueEntryOptions {
            unit,
            name: "Value",
            keypad: false,
            whole_numbers: false,
        },
    )
}

/// Exact numeric entry with a visible mouse keypad. Like [`value_entry`], this
/// keeps a private draft and returns a changed value only after Apply. The
/// caller's field name supplies context without guessing or changing its units.
pub fn mouse_value_entry(
    ui: &mut Ui,
    id_salt: impl Hash,
    current: f64,
    range: RangeInclusive<f64>,
    name: &str,
) -> Option<f64> {
    numeric_value_entry(ui, id_salt, current, range, name, false)
}

pub(crate) fn numeric_value_entry(
    ui: &mut Ui,
    id_salt: impl Hash,
    current: f64,
    range: RangeInclusive<f64>,
    name: &str,
    whole_numbers: bool,
) -> Option<f64> {
    value_entry_contents(
        ui,
        id_salt,
        current,
        range,
        ValueEntryOptions {
            unit: "",
            name,
            keypad: true,
            whole_numbers,
        },
    )
}

struct ValueEntryOptions<'a> {
    unit: &'a str,
    name: &'a str,
    keypad: bool,
    whole_numbers: bool,
}

fn value_entry_contents(
    ui: &mut Ui,
    id_salt: impl Hash,
    current: f64,
    range: RangeInclusive<f64>,
    options: ValueEntryOptions<'_>,
) -> Option<f64> {
    let ValueEntryOptions {
        unit,
        name,
        keypad,
        whole_numbers,
    } = options;
    if keypad {
        ui.set_min_width(220.0);
        ui.set_max_width(220.0);
    }
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
    let label = text::label(ui, name);
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
    if keypad {
        numeric_keypad(ui, input_id, &mut draft.text);
    }
    let parsed = draft
        .text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite());
    let in_range = parsed.filter(|value| range_valid && (minimum..=maximum).contains(value));
    let valid = in_range.filter(|value| !whole_numbers || value.fract() == 0.0);
    // Reserve feedback height for the keypad so Clear/invalid drafts never
    // move Apply or Cancel out from under the pointer.
    let feedback = if parsed.is_none() {
        "Enter a finite number."
    } else if in_range.is_none() {
        "The value must be inside the range."
    } else if valid.is_none() {
        "Enter a whole number."
    } else {
        ""
    };
    if keypad {
        let feedback_height = ui.text_style_height(&egui::TextStyle::Body) * 2.0;
        ui.allocate_ui_with_layout(
            egui::vec2(220.0, feedback_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(feedback_height);
                text::note(ui, if feedback.is_empty() { " " } else { feedback });
            },
        );
    } else if !feedback.is_empty() {
        text::note(ui, feedback);
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

#[derive(Clone, Copy)]
enum NumericKey {
    Insert(char),
    Sign,
    Backspace,
    Clear,
}

/// Pointer keys edit the same selection/caret as native text entry. They never
/// emit keyboard events or apply an intermediate numeric value to the owner.
fn numeric_keypad(ui: &mut Ui, input: Id, draft: &mut String) {
    let mut pressed = None;
    ui.add_space(tokens::SPACE_XS);
    for row in ["789", "456", "123", "±0."] {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
            for character in row.chars() {
                if ui
                    .add_sized(
                        egui::vec2(70.0, tokens::CONTROL_HEIGHT),
                        crate::button_widget(character.to_string()),
                    )
                    .clicked()
                {
                    pressed = Some(if character == '±' {
                        NumericKey::Sign
                    } else {
                        NumericKey::Insert(character)
                    });
                }
            }
        });
    }
    ui.horizontal(|ui| {
        if crate::button(ui, "Backspace").clicked() {
            pressed = Some(NumericKey::Backspace);
        }
        if crate::button(ui, "Clear").clicked() {
            pressed = Some(NumericKey::Clear);
        }
    });
    let Some(pressed) = pressed else {
        return;
    };
    let mut edit = egui::TextEdit::load_state(ui.ctx(), input).unwrap_or_default();
    let count = draft.chars().count();
    let range = edit
        .cursor
        .char_range()
        .map(|range| range.as_sorted_char_range())
        .unwrap_or(count..count);
    let cursor = edit_numeric_draft(draft, range, pressed);
    edit.cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(cursor),
        )));
    edit.store(ui.ctx(), input);
    ui.ctx().memory_mut(|memory| memory.request_focus(input));
}

fn edit_numeric_draft(draft: &mut String, range: std::ops::Range<usize>, key: NumericKey) -> usize {
    let count = draft.chars().count();
    let mut start = range.start.min(count);
    let end = range.end.min(count);
    let inserted = match key {
        NumericKey::Sign => {
            if draft.starts_with('-') {
                draft.remove(0);
                return end.saturating_sub(1);
            }
            if count < 128 {
                draft.insert(0, '-');
                return end + 1;
            }
            return end;
        }
        NumericKey::Clear => {
            draft.clear();
            return 0;
        }
        NumericKey::Backspace => {
            if start == end {
                start = start.saturating_sub(1);
            }
            String::new()
        }
        NumericKey::Insert(character) => character.to_string(),
    };
    // Native text input may contain Unicode even though keypad keys are ASCII.
    // Keep editing safe and use the same 128-character limit as the text field.
    if count - (end - start) + inserted.chars().count() > 128 {
        return end;
    }
    let start_byte = draft
        .char_indices()
        .nth(start)
        .map_or(draft.len(), |(index, _)| index);
    let end_byte = draft
        .char_indices()
        .nth(end)
        .map_or(draft.len(), |(index, _)| index);
    draft.replace_range(start_byte..end_byte, &inserted);
    start + inserted.chars().count()
}

#[cfg(test)]
mod tests {
    use super::{NumericKey, edit_numeric_draft, exact_value_text};

    #[test]
    fn numeric_pointer_keys_respect_selection_caret_sign_and_unicode() {
        let mut draft = "120.12345678901234".to_owned();
        let length = draft.chars().count();
        assert_eq!(
            edit_numeric_draft(&mut draft, 0..length, NumericKey::Insert('7')),
            1
        );
        assert_eq!(draft, "7");
        assert_eq!(
            edit_numeric_draft(&mut draft, 1..1, NumericKey::Insert('.')),
            2
        );
        assert_eq!(
            edit_numeric_draft(&mut draft, 2..2, NumericKey::Insert('5')),
            3
        );
        assert_eq!(edit_numeric_draft(&mut draft, 3..3, NumericKey::Sign), 4);
        assert_eq!(draft, "-7.5");
        assert_eq!(edit_numeric_draft(&mut draft, 4..4, NumericKey::Sign), 3);
        assert_eq!(
            edit_numeric_draft(&mut draft, 3..3, NumericKey::Backspace),
            2
        );
        assert_eq!(draft, "7.");
        draft = "1π2".into();
        assert_eq!(
            edit_numeric_draft(&mut draft, 1..2, NumericKey::Backspace),
            1
        );
        assert_eq!(draft, "12");
        assert_eq!(edit_numeric_draft(&mut draft, 2..2, NumericKey::Clear), 0);
        assert!(draft.is_empty());
    }

    #[test]
    fn opening_precise_and_scientific_values_round_trips_without_rounding() {
        for value in [
            120.123_456_789_012_34,
            1.0e-100,
            -1.0e100,
            f64::MIN_POSITIVE,
            f64::MAX,
            -0.0,
        ] {
            assert_eq!(
                exact_value_text(value).parse::<f64>().unwrap().to_bits(),
                value.to_bits()
            );
        }
    }
}
