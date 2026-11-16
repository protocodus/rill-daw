//! Deliberate exact-value entry for parameter context menus.
//! The caller owns the parameter; typing and dismissing this editor never edit it.
use crate::{text, tokens};
use egui::{Context, Id, Key, Modifiers, Rect, Ui};
use std::{hash::Hash, ops::RangeInclusive};

/// The value entry popover keeps one width for its field and feedback.
const ENTRY_WIDTH: f32 = tokens::SEARCH_CHOICE_WIDTH;

#[derive(Clone)]
struct ValueDraft {
    text: String,
    initial_value: f64,
    edited: bool,
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

/// Readable parameter display. Editing retains the exact opening value until
/// the user changes this text, so displaying six significant digits is lossless.
pub fn parameter_value_text(value: f64) -> String {
    if !value.is_finite() || value == 0.0 {
        return exact_value_text(value);
    }
    let magnitude = value.abs().log10().floor() as i32;
    let (mut text, exponent) = if !(-6..9).contains(&magnitude) {
        let scientific = format!("{value:.5e}");
        let (mantissa, exponent) = scientific.split_once('e').unwrap();
        (mantissa.to_owned(), format!("e{exponent}"))
    } else {
        let decimals = (5 - magnitude).max(0) as usize;
        (format!("{value:.decimals$}"), String::new())
    };
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text + &exponent
}

/// Native DragValue retains text while focused. A sibling rail/dial may edit
/// its value first; discard only that now-stale text, never a stable draft.
pub(crate) fn synchronize_number(ui: &Ui, id: Id, current: f64) {
    let previous = ui.data(|data| data.get_temp::<f64>(id.with("number_current_value")));
    let external_change = previous.is_some_and(|previous| previous.to_bits() != current.to_bits());
    // Slider and Knob commit native text edits as they are typed. DragValue's
    // additional blur parse is redundant, and can quantize an untouched value
    // when its freshly formatted display has fewer digits than its storage.
    let losing_focus = ui.memory(|memory| memory.had_focus_last_frame(id) && !memory.has_focus(id));
    if external_change || losing_focus {
        ui.data_mut(|data| {
            data.remove::<String>(id);
            if external_change {
                data.remove::<f64>(id);
            }
        });
    }
}

/// Remember either the untouched input or the owner's exact acknowledged
/// display after a write. Incoming changes still use an exact bit comparison.
pub(crate) fn remember_number_value(ui: &Ui, id: Id, expected: f64) {
    ui.data_mut(|data| data.insert_temp(id.with("number_current_value"), expected));
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
    value_entry_named(ui, id_salt, current, range, unit, "Value")
}

/// Exact entry identified by its channel and parameter.
pub fn value_entry_named(
    ui: &mut Ui,
    id_salt: impl Hash,
    current: f64,
    range: RangeInclusive<f64>,
    unit: &str,
    name: &str,
) -> Option<f64> {
    value_entry_contents(
        ui,
        id_salt,
        current,
        range,
        ValueEntryOptions {
            unit,
            name,
            whole_numbers: false,
            concise: true,
        },
    )
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
            whole_numbers,
            concise: false,
        },
    )
}

struct ValueEntryOptions<'a> {
    unit: &'a str,
    name: &'a str,
    whole_numbers: bool,
    concise: bool,
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
        whole_numbers,
        concise,
    } = options;
    ui.set_min_width(ENTRY_WIDTH);
    ui.set_max_width(ENTRY_WIDTH);
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
        let text = if concise {
            parameter_value_text(current)
        } else {
            exact_value_text(current)
        };
        ValueDraft {
            text,
            initial_value: current,
            edited: false,
            last_frame: frame,
            field: Rect::NOTHING,
            focus_pending: true,
        }
    } else {
        previous.expect("an existing draft is present")
    };
    // Follow a moving parameter while viewing it. Once the user starts a
    // draft, automation must not replace their text or move their caret.
    if draft.initial_value.to_bits() != current.to_bits() && !draft.edited {
        draft.text = if concise {
            parameter_value_text(current)
        } else {
            exact_value_text(current)
        };
        draft.initial_value = current;
    }
    let input_id = id.with("input");
    let focused = ui.is_enabled() && ui.memory(|memory| memory.has_focus(input_id));
    let enter = focused && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let escape = focused && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    let minimum = range.start().min(*range.end());
    let maximum = range.start().max(*range.end());
    let range_valid = range.start().is_finite() && range.end().is_finite();
    let label = text::bounded_block(ui, text::TextRole::Label, name, 2);
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
                .desired_width(ui.available_width())
                .char_limit(128)
                .margin(egui::vec2(
                    tokens::BUTTON_PADDING_X,
                    tokens::BUTTON_PADDING_Y,
                ))
                .vertical_align(egui::Align::Center)
                .min_size(egui::vec2(0.0, tokens::CONTROL_HEIGHT)),
        )
        .labelled_by(label.id)
        .on_hover_text(format!("Stored value: {}", exact_value_text(current)));
    draft.field = response.rect;
    draft.edited |= response.changed();
    draft.last_frame = frame;
    // Popups first measure their contents in a disabled sizing pass. Defer
    // focus until the field is genuinely interactive, rather than consuming
    // the initial selection during that invisible pass.
    if draft.focus_pending && ui.is_enabled() {
        response.request_focus();
        crate::widgets::select_field_text(ui, input_id, &draft.text);
        draft.focus_pending = false;
    }
    ui.data_mut(|data| data.insert_temp(Id::new("exact_value_input_rect"), (frame, response.rect)));
    // The concise display can round just beyond a range endpoint. Until an
    // actual edit, validate the exact stored value represented by that text.
    let parsed = if draft.edited {
        draft.text.trim().parse::<f64>().ok()
    } else {
        Some(draft.initial_value)
    }
    .filter(|value| value.is_finite());
    let in_range = parsed.filter(|value| range_valid && (minimum..=maximum).contains(value));
    let valid = in_range.filter(|value| !whole_numbers || value.fract() == 0.0);
    let feedback = if parsed.is_none() {
        "Enter a finite number."
    } else if in_range.is_none() {
        "The value must be inside the range."
    } else if valid.is_none() {
        "Enter a whole number."
    } else {
        ""
    };
    if !feedback.is_empty() {
        text::note(ui, feedback);
    }
    let (apply, cancel) = crate::dialog_actions(
        ui,
        |ui| {
            let apply = ui
                .add_enabled_ui(valid.is_some(), |ui| {
                    crate::dialog_primary(ui, "Apply value")
                })
                .inner;
            let cancel = crate::dialog_button(ui, "Cancel");
            (apply.clicked(), cancel.clicked())
        },
        |_| {},
    )
    .inner
    .0;
    if ui.is_enabled() && (cancel || escape || ((apply || enter) && valid.is_some())) {
        ui.data_mut(|data| data.remove::<(u64, Rect)>(Id::new("exact_value_input_rect")));
        ui.close();
        if !cancel && !escape {
            // An untouched rounded display is a view of the stored value,
            // never a request to round it or create an automation override.
            return valid.filter(|value| draft.edited && *value != current);
        }
    } else {
        ui.data_mut(|data| data.insert_temp(id, draft));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::exact_value_text;

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
