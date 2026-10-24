//! Shared musical-time viewport and gestures for every editing canvas.
use egui::{Id, Key, PointerButton, Pos2, Rect, Ui, Vec2};
use std::hash::Hash;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorTool {
    #[default]
    Select,
    Pencil,
    Cut,
    Duplicate,
    Glue,
    Erase,
    MultiDraw,
    FadeIn,
    FadeOut,
    Cursor,
}

impl EditorTool {
    #[allow(non_upper_case_globals)]
    pub const Draw: Self = Self::Pencil;
    #[allow(non_upper_case_globals)]
    pub const Split: Self = Self::Cut;

    pub const ALL: [Self; 10] = [
        Self::Select,
        Self::Pencil,
        Self::Cut,
        Self::Duplicate,
        Self::Glue,
        Self::Erase,
        Self::MultiDraw,
        Self::FadeIn,
        Self::FadeOut,
        Self::Cursor,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Pencil => "Pencil",
            Self::Cut => "Cut",
            Self::Duplicate => "Duplicate",
            Self::Glue => "Glue",
            Self::Erase => "Erase",
            Self::MultiDraw => "Multi-draw",
            Self::FadeIn => "Fade in",
            Self::FadeOut => "Fade out",
            Self::Cursor => "Cursor",
        }
    }

    pub fn key(self) -> Key {
        match self {
            Self::Select => Key::Q,
            Self::Pencil => Key::W,
            Self::Cut => Key::E,
            Self::Duplicate => Key::R,
            Self::Glue => Key::T,
            Self::Erase => Key::Z,
            Self::MultiDraw => Key::U,
            Self::FadeIn => Key::I,
            Self::FadeOut => Key::O,
            Self::Cursor => Key::P,
        }
    }
}

/// No shortcuts are claimed while a text field, menu, or another popup is editing.
pub fn editor_tool_shortcuts(ui: &mut Ui, tool: &mut EditorTool, supported: &[EditorTool]) {
    if !ui.is_enabled() || ui.ctx().wants_keyboard_input() || egui::Popup::is_any_open(ui.ctx()) {
        return;
    }
    ui.input_mut(|input| {
        if input.modifiers.any() {
            return;
        }
        for candidate in supported {
            if input.consume_key(egui::Modifiers::NONE, candidate.key()) {
                *tool = *candidate;
            }
        }
    });
}

pub fn editor_tools(ui: &mut Ui, tool: &mut EditorTool, supported: &[EditorTool]) {
    for candidate in supported {
        crate::selectable_value(ui, tool, *candidate, candidate.label()).on_hover_text(format!(
            "{} · {}",
            candidate.label(),
            candidate.key().name()
        ));
    }
}

pub fn timeline_fit_button(ui: &mut Ui) -> egui::Response {
    crate::button(ui, "Fit").on_hover_text("Fit all content on both axes · F")
}

/// Musical bars and elapsed clock time share the exact same horizontal mapping.
pub fn paint_timeline_ruler(
    painter: &egui::Painter,
    rect: Rect,
    view: TimelineViewport,
    tempo: f64,
    beats_per_bar: u8,
) {
    let bar = usize::from(beats_per_bar.max(1));
    let step = ((80.0 / view.pixels_per_beat).ceil() as usize)
        .max(bar)
        .div_ceil(bar)
        * bar;
    let start = (view.start_beat.max(0.0) as usize / step) * step;
    let end = view.beat_at(rect, rect.right()).ceil().max(0.0) as usize;
    let painter = painter.with_clip_rect(rect);
    let palette = crate::theme::palette(painter.ctx());
    let font = egui::FontId::monospace(crate::tokens::FONT_CANVAS);
    for beat in (start..=end).step_by(step) {
        let x = view.x_at(rect, beat as f64);
        let seconds = beat as f64 * 60.0 / tempo.max(1.0);
        for (text, anchor, alignment, color) in [
            (
                format!("{}", beat / bar + 1),
                egui::pos2(x + 4.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                palette.text,
            ),
            (
                format!("{}:{:05.2}", seconds as u64 / 60, seconds % 60.0),
                egui::pos2(x + 4.0, rect.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                palette.muted,
            ),
        ] {
            let galley = painter.layout_no_wrap(text, font.clone(), color);
            let position = alignment.anchor_size(anchor, galley.size()).min;
            let bounds = galley
                .rect
                .union(galley.mesh_bounds)
                .translate(position.to_vec2());
            // Partial bar/time glyphs can imply a different position. Preserve
            // their time anchors and omit labels that do not fit in full.
            if painter.clip_rect().contains_rect(bounds) {
                painter.galley(position, galley, color);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineViewport {
    pub start_beat: f64,
    pub pixels_per_beat: f32,
    pub row_offset: f32,
    pub row_height: f32,
}

impl Default for TimelineViewport {
    fn default() -> Self {
        Self {
            start_beat: 0.0,
            pixels_per_beat: 48.0,
            row_offset: 0.0,
            row_height: 24.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TimelineBounds {
    pub start_beat: f64,
    pub end_beat: f64,
    pub start_row: f32,
    pub end_row: f32,
}

impl TimelineViewport {
    pub fn beat_at(self, rect: Rect, x: f32) -> f64 {
        self.start_beat + (x - rect.left()) as f64 / self.pixels_per_beat as f64
    }
    pub fn x_at(self, rect: Rect, beat: f64) -> f32 {
        rect.left() + ((beat - self.start_beat) * self.pixels_per_beat as f64) as f32
    }
    pub fn row_at(self, rect: Rect, y: f32) -> f32 {
        self.row_offset + (y - rect.top()) / self.row_height
    }
    pub fn y_at(self, rect: Rect, row: f32) -> f32 {
        rect.top() + (row - self.row_offset) * self.row_height
    }
    pub fn pan(&mut self, delta: Vec2) {
        self.start_beat = (self.start_beat - delta.x as f64 / self.pixels_per_beat as f64).max(0.0);
        self.row_offset = (self.row_offset - delta.y / self.row_height).max(0.0);
    }
    pub fn zoom(&mut self, rect: Rect, anchor: Pos2, horizontal: f32, vertical: f32) {
        let beat = self.beat_at(rect, anchor.x);
        let row = self.row_at(rect, anchor.y);
        self.pixels_per_beat = (self.pixels_per_beat * horizontal).clamp(0.000_01, 4096.0);
        self.row_height = (self.row_height * vertical).clamp(0.01, 1024.0);
        self.start_beat =
            (beat - (anchor.x - rect.left()) as f64 / self.pixels_per_beat as f64).max(0.0);
        self.row_offset = (row - (anchor.y - rect.top()) / self.row_height).max(0.0);
    }
    pub fn fit_bounds(&mut self, rect: Rect, bounds: TimelineBounds) {
        let start = bounds.start_beat.max(0.0);
        let length = (bounds.end_beat - start).max(0.25);
        let rows = (bounds.end_row - bounds.start_row).max(1.0);
        let padding = 12.0_f32.min(rect.width() * 0.1).min(rect.height() * 0.1);
        self.pixels_per_beat = ((rect.width() - padding * 2.0).max(1.0) as f64 / length) as f32;
        self.row_height = (rect.height() - padding * 2.0).max(1.0) / rows;
        self.start_beat = (start - padding as f64 / self.pixels_per_beat as f64).max(0.0);
        self.row_offset = (bounds.start_row - padding / self.row_height).max(0.0);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TimelineNavigation {
    pub changed: bool,
    /// True for the entire pan/zoom gesture, including its initial press.
    pub navigating: bool,
    pub fit: bool,
}

#[derive(Clone, Copy)]
struct Gesture {
    button: PointerButton,
    zoom: bool,
    anchor: Pos2,
}

pub struct TimelinePane {
    id: Id,
    rect: Rect,
    horizontal_only: bool,
    keyboard_focus: Option<Id>,
}

impl TimelinePane {
    pub fn new(id: impl Hash, rect: Rect) -> Self {
        Self {
            id: Id::new(id),
            rect,
            horizontal_only: false,
            keyboard_focus: None,
        }
    }

    /// A normalized parameter curve keeps its complete vertical range visible.
    /// Horizontal gestures stay identical to music editors; vertical gestures
    /// are consumed without changing its value scale.
    pub fn horizontal_only(mut self) -> Self {
        self.horizontal_only = true;
        self
    }

    /// Allow shortcuts while this non-text canvas owns native keyboard focus.
    pub fn keyboard_focus(mut self, id: Id) -> Self {
        self.keyboard_focus = Some(id);
        self
    }

    /// Call before canvas hit testing. Navigation never claims ordinary left clicks.
    /// Middle/Alt-left pan; Ctrl-right drag zooms around the current pointer.
    pub fn navigate(
        &self,
        ui: &mut Ui,
        view: &mut TimelineViewport,
        bounds: TimelineBounds,
    ) -> TimelineNavigation {
        if !ui.is_enabled() {
            ui.ctx().data_mut(|data| data.remove::<Gesture>(self.id));
            return TimelineNavigation::default();
        }
        let before = *view;
        let mut result = TimelineNavigation::default();
        let focus_id = Id::new("rill_timeline_keyboard_focus");
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let hovered = pointer.is_some_and(|p| self.rect.contains(p))
            && ui.is_enabled()
            && ui.rect_contains_pointer(self.rect);
        let mut gesture = ui.ctx().data_mut(|data| data.get_temp::<Gesture>(self.id));
        if !ui.input(|input| input.focused) {
            gesture = None;
        }
        let mut started = false;
        if hovered && ui.input(|i| i.pointer.any_pressed()) {
            started = true;
            ui.ctx()
                .data_mut(|data| data.insert_temp(focus_id, self.id));
            gesture = ui.input(|i| {
                if i.modifiers.ctrl && i.pointer.button_pressed(PointerButton::Secondary) {
                    Some(Gesture {
                        button: PointerButton::Secondary,
                        zoom: true,
                        anchor: pointer.unwrap_or(self.rect.center()),
                    })
                } else if i.pointer.button_pressed(PointerButton::Middle) {
                    Some(Gesture {
                        button: PointerButton::Middle,
                        zoom: i.modifiers.ctrl,
                        anchor: pointer.unwrap_or(self.rect.center()),
                    })
                } else if i.modifiers.alt && i.pointer.primary_pressed() {
                    Some(Gesture {
                        button: PointerButton::Primary,
                        zoom: false,
                        anchor: pointer.unwrap_or(self.rect.center()),
                    })
                } else {
                    None
                }
            });
        }
        if let Some(active) = gesture {
            result.navigating = true;
            let delta = if started {
                Vec2::ZERO
            } else {
                ui.input(|i| i.pointer.delta())
            };
            if active.zoom {
                let factor = (delta.x * 0.01 - delta.y * 0.01).exp();
                let vertical = ui.input(|i| i.modifiers.shift);
                view.zoom(
                    self.rect,
                    active.anchor,
                    if vertical { 1.0 } else { factor },
                    if vertical { factor } else { 1.0 },
                );
            } else {
                view.pan(delta);
            }
            ui.ctx().set_cursor_icon(if active.zoom {
                egui::CursorIcon::ZoomIn
            } else {
                egui::CursorIcon::Grabbing
            });
            if !ui.input(|i| i.pointer.button_down(active.button)) {
                gesture = None;
            }
        }
        ui.ctx().data_mut(|data| {
            if let Some(active) = gesture {
                data.insert_temp(self.id, active);
            } else {
                data.remove::<Gesture>(self.id);
            }
        });
        if hovered && !egui::Popup::is_any_open(ui.ctx()) {
            let (delta, modifiers) = ui.input(|i| (i.raw_scroll_delta, i.modifiers));
            if delta != Vec2::ZERO {
                if modifiers.ctrl || modifiers.command {
                    let factor = ((delta.y + delta.x) * 0.005).exp();
                    view.zoom(
                        self.rect,
                        pointer.unwrap_or(self.rect.center()),
                        if modifiers.shift { 1.0 } else { factor },
                        if modifiers.shift { factor } else { 1.0 },
                    );
                } else if modifiers.shift {
                    view.pan(Vec2::new(delta.x + delta.y, 0.0));
                } else {
                    view.pan(delta);
                }
                ui.input_mut(|i| {
                    i.raw_scroll_delta = Vec2::ZERO;
                    i.smooth_scroll_delta = Vec2::ZERO;
                });
            }
        }
        let focused = ui.ctx().data_mut(|data| data.get_temp::<Id>(focus_id)) == Some(self.id);
        if (hovered || focused)
            && (!ui.ctx().wants_keyboard_input()
                || self
                    .keyboard_focus
                    .is_some_and(|id| ui.memory(|memory| memory.has_focus(id))))
            && !egui::Popup::is_any_open(ui.ctx())
        {
            ui.input_mut(|i| {
                if i.modifiers.ctrl || i.modifiers.command || i.modifiers.alt {
                    return;
                }
                let modifiers = i.modifiers;
                if i.consume_key(egui::Modifiers::NONE, Key::F) {
                    view.fit_bounds(self.rect, bounds);
                    result.fit = true;
                }
                let factor = if i.consume_key(modifiers, Key::Plus)
                    || i.consume_key(modifiers, Key::Equals)
                {
                    1.25
                } else if i.consume_key(modifiers, Key::Minus) {
                    0.8
                } else {
                    1.0
                };
                if factor != 1.0 {
                    view.zoom(
                        self.rect,
                        self.rect.center(),
                        if modifiers.shift { 1.0 } else { factor },
                        if modifiers.shift { factor } else { 1.0 },
                    );
                }
            });
        }
        if self.horizontal_only {
            view.row_offset = before.row_offset;
            view.row_height = before.row_height;
        }
        result.changed = before != *view;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zoom_keeps_the_musical_point_and_row_under_the_pointer() {
        let rect = Rect::from_min_size(egui::pos2(80.0, 60.0), egui::vec2(800.0, 400.0));
        let anchor = egui::pos2(380.0, 260.0);
        let mut view = TimelineViewport {
            start_beat: 16.0,
            row_offset: 20.0,
            ..Default::default()
        };
        let beat = view.beat_at(rect, anchor.x);
        let row = view.row_at(rect, anchor.y);
        view.zoom(rect, anchor, 2.0, 1.5);
        assert!((view.beat_at(rect, anchor.x) - beat).abs() < 1e-6);
        assert!((view.row_at(rect, anchor.y) - row).abs() < 1e-5);
    }
    #[test]
    fn fit_contains_late_content_and_all_rows_on_both_axes() {
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 400.0));
        let bounds = TimelineBounds {
            start_beat: 100.0,
            end_beat: 140.0,
            start_row: 24.0,
            end_row: 88.0,
        };
        let mut view = TimelineViewport::default();
        view.fit_bounds(rect, bounds);
        assert!(rect.contains(egui::pos2(view.x_at(rect, 100.0), view.y_at(rect, 24.0))));
        assert!(rect.contains(egui::pos2(view.x_at(rect, 140.0), view.y_at(rect, 88.0))));
        assert!(view.start_beat > 99.0);
        assert!(view.row_height < 7.0);
    }
}
