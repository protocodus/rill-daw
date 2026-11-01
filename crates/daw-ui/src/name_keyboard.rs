//! Optional pointer text entry. The caller owns the draft and its transaction.
use crate::tokens;
use egui::{Context, Id, Ui};

pub const NAME_MAX_CHARS: usize = 128;

/// Ten full key targets, their gaps and the keyboard frame, rounded outward
/// at the current density so an anchored keyboard never loses its final key.
pub fn name_keyboard_width(ctx: &Context) -> f32 {
    let key = (tokens::CONTROL_HEIGHT * ctx.pixels_per_point()).ceil() / ctx.pixels_per_point();
    10.0 * key
        + 9.0 * tokens::SPACE_SM
        + crate::panel(ctx)
            .inner_margin(tokens::SPACE_MD)
            .total_margin()
            .sum()
            .x
}

/// Height added below the name field, including its gap and keyboard frame.
/// Keep the anchored editor inside the viewport before its first visible frame.
pub fn name_keyboard_height(ctx: &Context) -> f32 {
    let frame = crate::panel(ctx).inner_margin(tokens::SPACE_MD);
    let font = egui::TextStyle::Button.resolve(&ctx.style());
    let row_height =
        ctx.fonts_mut(|fonts| fonts.row_height(&font)) + 2.0 * tokens::BUTTON_PADDING_Y;
    let row_height = (row_height.max(tokens::CONTROL_HEIGHT) * ctx.pixels_per_point()).ceil()
        / ctx.pixels_per_point();
    tokens::SPACE_MD
        + ctx.style().spacing.item_spacing.y
        + 6.0 * row_height
        + 5.0 * tokens::SPACE_SM
        + tokens::SPACE_XS
        + frame.total_margin().sum().y
}

#[derive(Clone, Default)]
struct KeyboardState {
    open: bool,
    uppercase: bool,
}

pub fn name_keyboard_open(ctx: &Context, field: Id) -> bool {
    ctx.data(|data| data.get_temp::<KeyboardState>(field.with("keyboard")))
        .is_some_and(|state| state.open)
}

pub fn set_name_keyboard_open(ctx: &Context, field: Id, open: bool) {
    ctx.data_mut(|data| {
        let mut state = data
            .get_temp::<KeyboardState>(field.with("keyboard"))
            .unwrap_or_default();
        state.open = open;
        data.insert_temp(field.with("keyboard"), state);
    });
}

/// Draw only on request, below the ordinary name field. Native keyboard entry
/// and pointer keys share the field's Unicode character cursor/selection.
pub fn name_keyboard(ui: &mut Ui, field: Id, value: &mut String) -> bool {
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<KeyboardState>(field.with("keyboard")))
        .unwrap_or_default();
    if !state.open {
        return false;
    }
    let mut insert = None;
    let mut backspace = false;
    let mut clear = false;
    let mut select_all = false;
    ui.add_space(tokens::SPACE_MD);
    crate::panel(ui.ctx())
        .inner_margin(tokens::SPACE_MD)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(tokens::SPACE_SM, tokens::SPACE_SM);
            // Single-character keys keep full targets without a word button's
            // horizontal padding widening the uppercase QWERTY row.
            ui.spacing_mut().button_padding.x = tokens::SPACE_SM;
            for row in [
                "1234567890",
                "qwertyuiop",
                "asdfghjkl",
                "zxcvbnm",
                "-_.!'()?&+",
            ] {
                ui.horizontal(|ui| {
                    for character in row.chars() {
                        let text = if state.uppercase {
                            character.to_ascii_uppercase()
                        } else {
                            character
                        }
                        .to_string();
                        if ui
                            .add(
                                crate::button_widget(&text)
                                    .min_size(egui::Vec2::splat(tokens::CONTROL_HEIGHT)),
                            )
                            .clicked()
                        {
                            insert = Some(text);
                        }
                    }
                    if row == "zxcvbnm" {
                        crate::view_toggle(ui, &mut state.uppercase, "Caps")
                            .on_hover_text("Keep letters uppercase until Caps is turned off");
                    }
                });
            }
            ui.add_space(tokens::SPACE_XS);
            ui.horizontal(|ui| {
                if crate::button(ui, "Space").clicked() {
                    insert = Some(" ".into());
                }
                backspace = crate::button(ui, "Backspace").clicked();
                select_all = crate::button(ui, "Select all").clicked();
                clear = crate::button(ui, "Clear").clicked();
            });
        });
    ui.ctx()
        .data_mut(|data| data.insert_temp(field.with("keyboard"), state));
    if insert.is_none() && !backspace && !clear && !select_all {
        return false;
    }
    let mut edit = egui::text_edit::TextEditState::load(ui.ctx(), field).unwrap_or_default();
    let count = value.chars().count();
    let range = edit
        .cursor
        .char_range()
        .map(|range| range.as_sorted_char_range())
        .unwrap_or(count..count);
    let mut start = range.start.min(count);
    let mut end = range.end.min(count);
    if select_all {
        edit.cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(count),
            )));
        edit.store(ui.ctx(), field);
        ui.ctx().memory_mut(|memory| memory.request_focus(field));
        return false;
    }
    if clear {
        start = 0;
        end = count;
    }
    if backspace && start == end {
        start = start.saturating_sub(1);
    }
    let inserted = insert.unwrap_or_default();
    if count - (end - start) + inserted.chars().count() > NAME_MAX_CHARS {
        return false;
    }
    let start_byte = value
        .char_indices()
        .nth(start)
        .map_or(value.len(), |(index, _)| index);
    let end_byte = value
        .char_indices()
        .nth(end)
        .map_or(value.len(), |(index, _)| index);
    let changed = start != end || !inserted.is_empty();
    value.replace_range(start_byte..end_byte, &inserted);
    edit.cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(start + inserted.chars().count()),
        )));
    edit.store(ui.ctx(), field);
    ui.ctx().memory_mut(|memory| memory.request_focus(field));
    changed
}
