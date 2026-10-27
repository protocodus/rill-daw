//! Directional item selection shared by timeline and grid editors.
use egui::{Event, Id, Key, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionItem {
    pub id: u64,
    pub start: f64,
    pub end: f64,
    /// Visual row, increasing downwards. This may be fractional for curves.
    pub row: f64,
}

/// The same mouse-only membership action appears on clips, notes and points.
pub fn selection_membership_action(ui: &mut Ui, selected: bool) -> bool {
    let clicked = crate::button(
        ui,
        if selected {
            "Remove from selection"
        } else {
            "Add to selection"
        },
    )
    .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Add a context menu to a canvas item without taking its primary-button drag
/// or double-click gesture away from the parent editor.
pub fn selection_item_context_menu<R>(
    ui: &mut Ui,
    id: Id,
    rect: egui::Rect,
    contents: impl FnOnce(&mut Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    let response = ui.interact(rect, id, egui::Sense::hover());
    let open = ui.is_enabled()
        && ui.rect_contains_pointer(rect)
        && ui.input(|input| input.pointer.button_clicked(egui::PointerButton::Secondary));
    egui::Popup::context_menu(&response)
        .open_memory(open.then_some(egui::SetOpenCommand::Bool(true)))
        .show(contents)
}

/// Left/right follows time; up/down follows rows and prefers overlapping time.
/// Ties use the other axis and then the ID, so storage order cannot change the
/// destination. At an edge the current selection remains in place.
pub fn adjacent_selection_item(
    items: &[SelectionItem],
    current: Option<u64>,
    direction: SelectionDirection,
) -> Option<u64> {
    let valid = |item: &&SelectionItem| {
        item.start.is_finite() && item.end.is_finite() && item.row.is_finite()
    };
    let Some(origin) = items
        .iter()
        .filter(valid)
        .find(|item| Some(item.id) == current)
    else {
        return items
            .iter()
            .filter(valid)
            .min_by(|a, b| {
                a.start
                    .total_cmp(&b.start)
                    .then(a.row.total_cmp(&b.row))
                    .then(a.id.cmp(&b.id))
            })
            .map(|item| item.id);
    };
    let interval_gap = |item: &SelectionItem| {
        (item.start - origin.end)
            .max(origin.start - item.end)
            .max(0.0)
    };
    items
        .iter()
        .filter(valid)
        .filter(|item| {
            item.id != origin.id
                && match direction {
                    SelectionDirection::Left => item.start < origin.start,
                    SelectionDirection::Right => item.start > origin.start,
                    SelectionDirection::Up => item.row < origin.row,
                    SelectionDirection::Down => item.row > origin.row,
                }
        })
        .min_by(|a, b| {
            let time = |item: &SelectionItem| (item.start - origin.start).abs();
            let row = |item: &SelectionItem| (item.row - origin.row).abs();
            match direction {
                SelectionDirection::Left | SelectionDirection::Right => {
                    time(a).total_cmp(&time(b)).then(row(a).total_cmp(&row(b)))
                }
                SelectionDirection::Up | SelectionDirection::Down => row(a)
                    .total_cmp(&row(b))
                    .then(interval_gap(a).total_cmp(&interval_gap(b)))
                    .then(time(a).total_cmp(&time(b))),
            }
            .then(a.id.cmp(&b.id))
        })
        .map(|item| item.id)
        .or(current)
}

/// Foreground editors can use this without first clicking their canvas. A
/// focused canvas may own arrows too; focused text fields always retain them.
pub fn selection_navigation_input(
    ui: &Ui,
    keyboard_focus: Option<Id>,
) -> Option<(SelectionDirection, bool)> {
    if let Some(id) = keyboard_focus {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
    }
    if !ui.is_enabled()
        || egui::Popup::is_any_open(ui.ctx())
        || ui.input(|input| input.pointer.any_down())
        || (ui.ctx().wants_keyboard_input()
            && !keyboard_focus.is_some_and(|id| ui.memory(|memory| memory.has_focus(id))))
    {
        return None;
    }
    let selected = ui.input_mut(|input| {
        // Match the event's modifiers, including integrations which deliver a
        // chord and modifier release in the same frame. egui's logical shortcut
        // matching intentionally ignores extra Alt, which selection must retain.
        let (index, direction, extend) =
            input.events.iter().enumerate().find_map(|(index, event)| {
                let Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                else {
                    return None;
                };
                if modifiers.command || modifiers.ctrl || modifiers.alt {
                    return None;
                }
                let direction = match key {
                    Key::ArrowLeft => SelectionDirection::Left,
                    Key::ArrowRight => SelectionDirection::Right,
                    Key::ArrowUp => SelectionDirection::Up,
                    Key::ArrowDown => SelectionDirection::Down,
                    _ => return None,
                };
                Some((index, direction, modifiers.shift))
            })?;
        input.events.remove(index);
        Some((direction, extend))
    });
    if selected.is_some() {
        // egui determines widget focus direction before the editor consumes the
        // event. Keep item navigation inside its pane for the following key.
        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
    selected
}
