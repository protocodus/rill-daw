//! Native scroll panes with an explicit middle-button navigation gesture.
use egui::{Id, PointerButton, ScrollArea, Ui};
use std::hash::Hash;

#[derive(Clone, Copy, Default)]
struct MiddlePan {
    active: bool,
    navigating: bool,
}

fn pan_id(ui: &Ui, id_salt: impl Hash) -> Id {
    ui.make_persistent_id(Id::new(id_salt)).with("middle_pan")
}

/// Build a native pane that reserves middle-drag for scrolling before children
/// can claim it. The pane must occupy the UI's available clipped rectangle.
///
/// Call `scroll_area_is_panning` before showing the contents and suppress value
/// publication during that gesture, including its release frame. Native child
/// widgets can still report an old drag stopping when navigation takes over.
/// This helper does not block arbitrary child actions or reserve Alt-click.
pub fn scroll_area(ui: &mut Ui, id_salt: impl Hash + Copy, axes: [bool; 2]) -> ScrollArea {
    let scroll_id = ui.make_persistent_id(Id::new(id_salt));
    let pan_id = pan_id(ui, id_salt);
    let previous = ui
        .data(|data| data.get_temp::<MiddlePan>(pan_id))
        .unwrap_or_default();
    let (pressed, down, released, position, focused) = ui.input(|input| {
        (
            input.pointer.button_pressed(PointerButton::Middle),
            input.pointer.button_down(PointerButton::Middle),
            input.pointer.button_released(PointerButton::Middle),
            input.pointer.interact_pos(),
            input.focused,
        )
    });
    let enabled = ui.is_enabled() && focused && !egui::Popup::is_any_open(ui.ctx());
    let bounds = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    let starts_here = pressed
        && position.is_some_and(|point| bounds.contains(point))
        && ui.rect_contains_pointer(bounds);
    let active = enabled && (previous.active || starts_here);
    let navigating = active && (down || released);
    ui.data_mut(|data| {
        data.insert_temp(
            pan_id,
            MiddlePan {
                active: active && down,
                navigating,
            },
        )
    });
    if navigating && down && !pressed {
        ui.ctx().set_dragged_id(scroll_id.with("area"));
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    ScrollArea::new(axes)
        .id_salt(id_salt)
        // A press may include the pointer's arrival from elsewhere. Do not let
        // native drag scrolling turn that arrival delta into a viewport jump.
        // Middle-button navigation ends at release, matching musical panes;
        // native primary/touch dragging retains its ordinary kinetic scroll.
        .scroll_source(egui::scroll_area::ScrollSource {
            drag: !(starts_here || navigating && released),
            ..Default::default()
        })
}

/// True while this pane owns a middle gesture, including its release frame.
/// Read this on the same parent UI, after `scroll_area` and before its `show`.
pub fn scroll_area_is_panning(ui: &Ui, id_salt: impl Hash) -> bool {
    ui.data(|data| data.get_temp::<MiddlePan>(pan_id(ui, id_salt)))
        .is_some_and(|state| state.navigating)
}
