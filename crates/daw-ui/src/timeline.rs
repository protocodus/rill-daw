//! Shared musical-time viewport and gestures for every editing canvas.
use egui::{Id, Key, PointerButton, Pos2, Rect, Response, Ui, Vec2};
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
    Ghost,
    Stretch,
    Slip,
}

impl EditorTool {
    #[allow(non_upper_case_globals)]
    pub const Draw: Self = Self::Pencil;
    #[allow(non_upper_case_globals)]
    pub const Split: Self = Self::Cut;

    pub const ALL: [Self; 13] = [
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
        Self::Ghost,
        Self::Stretch,
        Self::Slip,
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
            Self::Ghost => "Ghost copy",
            Self::Stretch => "Stretch",
            Self::Slip => "Slip",
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
            Self::Ghost => Key::G,
            Self::Stretch => Key::S,
            Self::Slip => Key::L,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Select => {
                "Drag to move; drag an edge to trim. Shift-drag copies; Shift at an edge bypasses snap."
            }
            Self::Pencil => "Draw a note or MIDI clip.",
            Self::Cut => "Split at the pointer.",
            Self::Duplicate => "Create an independent copy.",
            Self::Glue => "Join clips or selected notes.",
            Self::Erase => "Delete at the pointer.",
            Self::MultiDraw => "Draw the selected chord or scale.",
            Self::FadeIn => "Drag to shape the audio fade-in.",
            Self::FadeOut => "Drag to shape the audio fade-out.",
            Self::Cursor => "Place the playback cursor.",
            Self::Ghost => {
                "Create a linked copy. Source edits and the name remain shared; placement, trim and stretch stay independent."
            }
            Self::Stretch => {
                "Drag to change duration. Audio keeps its pitch; MIDI note timing scales. Hold Shift to bypass snap."
            }
            Self::Slip => {
                "Drag the audio inside fixed clip edges. Source bounds are preserved; Shift bypasses grid snap."
            }
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
    editor_tools_with_labels(ui, tool, supported, EditorTool::label);
}

/// Contextual wording keeps the same tool identity and shortcut in each editor.
pub fn editor_tools_with_labels(
    ui: &mut Ui,
    tool: &mut EditorTool,
    supported: &[EditorTool],
    label: impl Fn(EditorTool) -> &'static str,
) {
    for candidate in supported {
        crate::tool_value(ui, tool, *candidate, label(*candidate)).on_hover_text(format!(
            "{} tool · {}\n{}",
            label(*candidate),
            candidate.key().name(),
            candidate.hint(),
        ));
    }
    if !supported.is_empty() {
        ui.separator();
    }
}

pub fn timeline_fit_button(ui: &mut Ui) -> egui::Response {
    crate::view_button(ui, "Fit").on_hover_text("Fit all content on both axes · F")
}

/// Shared by the arranger and focused audio editor. This is an edit increment,
/// not a viewport option. One sample always means the active output sample rate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NudgeUnit {
    #[default]
    Grid,
    Millisecond,
    Sample,
}

impl NudgeUnit {
    pub fn beats(self, grid: f64, tempo: f64, sample_rate: u32) -> f64 {
        match self {
            Self::Grid => {
                if grid > 0.0 {
                    grid
                } else {
                    0.25
                }
            }
            Self::Millisecond => tempo / 60_000.0,
            Self::Sample => tempo / 60.0 / f64::from(sample_rate.max(1)),
        }
    }

    pub fn load(ctx: &egui::Context) -> Self {
        ctx.data(|data| data.get_temp(egui::Id::new("timeline_nudge_unit")))
            .unwrap_or_default()
    }
}

/// On-demand edit menu shared across timelines. Choices and actions have the
/// same semantics and appearance in both workspaces.
pub fn nudge_menu(ui: &mut Ui, enabled: bool) -> i32 {
    let mut unit = NudgeUnit::load(ui.ctx());
    let mut direction = 0;
    crate::action_menu(ui, "Nudge", |ui| {
        crate::text::label(ui, "Increment");
        for (value, label) in [
            (NudgeUnit::Grid, "Grid"),
            (NudgeUnit::Millisecond, "1 ms"),
            (NudgeUnit::Sample, "1 sample"),
        ] {
            crate::selectable_value(ui, &mut unit, value, label);
        }
        ui.separator();
        for (value, label, hint) in [
            (
                -1,
                "Nudge left",
                "Move selected clip left · Ctrl/Cmd+Left arrow",
            ),
            (
                1,
                "Nudge right",
                "Move selected clip right · Ctrl/Cmd+Right arrow",
            ),
        ] {
            if ui
                .add_enabled(enabled, crate::button_widget(label))
                .on_hover_text(hint)
                .clicked()
            {
                direction = value;
                ui.close();
            }
        }
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new("timeline_nudge_unit"), unit));
    direction
}

/// The current project model counts quarter-note beats per bar. This compact
/// choice deliberately exposes only its supported numerators and denominator.
pub fn time_signature_choice(ui: &mut Ui, beats_per_bar: &mut u8) -> Response {
    let before = *beats_per_bar;
    let height = (ui.ctx().content_rect().height() - 100.0).clamp(68.0, 240.0);
    let mut response = crate::select("time_signature", format!("{}/4", *beats_per_bar))
        .width(56.0)
        .height(height)
        .show_ui(ui, |ui| {
            for beats in 1..=32 {
                if crate::selectable_value(ui, beats_per_bar, beats, format!("{beats}/4")).clicked()
                {
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Time signature · quarter-note beats per bar");
    if *beats_per_bar != before {
        response.mark_changed();
    }
    response
}

/// Musical position and elapsed clock time use the canvas's exact mapping.
/// At closer zoom, beat and quarter-beat anchors keep a view between bar lines
/// readable. Bar labels remain primary; all labels require their full bounds.
pub fn paint_timeline_ruler(
    painter: &egui::Painter,
    rect: Rect,
    view: TimelineViewport,
    tempo: f64,
    beats_per_bar: u8,
) {
    if !rect.is_finite()
        || !rect.is_positive()
        || !view.start_beat.is_finite()
        || !view.pixels_per_beat.is_finite()
        || view.pixels_per_beat <= 0.0
    {
        return;
    }
    let end_beat = view.beat_at(rect, rect.right());
    if !end_beat.is_finite() {
        return;
    }
    let bar = usize::from(beats_per_bar.max(1));
    const LABEL_SPACING: f32 = 80.0;
    let step = ((LABEL_SPACING / view.pixels_per_beat).ceil() as usize)
        .max(bar)
        .div_ceil(bar)
        .saturating_mul(bar);
    let start = (view.start_beat.max(0.0) as usize / step) * step;
    let end = end_beat.ceil().max(0.0) as usize;
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let palette = crate::theme::palette(painter.ctx());
    let font = egui::FontId::monospace(crate::tokens::FONT_CANVAS);
    let mut upper = Vec::new();
    let mut lower = Vec::new();
    let label = |text: String, x: f32, top: bool, color, placed: &mut Vec<Rect>| {
        let anchor = egui::pos2(
            x + crate::tokens::SPACE_XS,
            if top {
                rect.top() + 2.0
            } else {
                rect.bottom() - 2.0
            },
        );
        let alignment = if top {
            egui::Align2::LEFT_TOP
        } else {
            egui::Align2::LEFT_BOTTOM
        };
        let galley = painter.layout_no_wrap(text, font.clone(), color);
        let position = alignment.anchor_size(anchor, galley.size()).min;
        let bounds = galley
            .rect
            .union(galley.mesh_bounds)
            .translate(position.to_vec2());
        if painter.clip_rect().contains_rect(bounds)
            && placed
                .iter()
                .all(|previous| !previous.expand(crate::tokens::SPACE_XS).intersects(bounds))
        {
            placed.push(bounds);
            painter.galley(position, galley, color);
        }
    };
    let clock = |beat: f64| {
        let seconds = beat * 60.0 / tempo.max(1.0);
        seconds
            .is_finite()
            .then(|| format!("{}:{:05.2}", seconds as u64 / 60, seconds % 60.0))
    };
    let tick = |x: f32, length: f32| {
        if x >= rect.left() && x <= rect.right() {
            painter.line_segment(
                [
                    egui::pos2(x, rect.bottom() - length),
                    egui::pos2(x, rect.bottom()),
                ],
                egui::Stroke::new(1.0_f32, palette.line),
            );
        }
    };
    // Reserve the primary bar and clock labels first so finer anchors cannot
    // displace them, including at large bar numbers or a clipped viewport edge.
    for beat in (start..=end).step_by(step).take(2048) {
        let x = view.x_at(rect, beat as f64);
        label(
            format!("{}", (beat / bar).saturating_add(1)),
            x,
            true,
            palette.text,
            &mut upper,
        );
        if let Some(text) = clock(beat as f64) {
            label(text, x, false, palette.muted, &mut lower);
        }
        tick(x, 8.0);
    }
    let divisions = [4, 2, 1]
        .into_iter()
        .find(|division| view.pixels_per_beat / *division as f32 >= LABEL_SPACING);
    let Some(divisions) = divisions else {
        return;
    };
    let tick_step = 4 / divisions;
    let first = (view.start_beat.max(0.0) * 4.0).floor() as usize / tick_step * tick_step;
    let last = (end_beat.max(0.0) * 4.0).ceil() as usize;
    for quarter in (first..=last).step_by(tick_step).take(2048) {
        if quarter % (bar * 4) == 0 {
            continue;
        }
        let beat = quarter as f64 / 4.0;
        let x = view.x_at(rect, beat);
        let bar_number = (quarter / (bar * 4)).saturating_add(1);
        let beat_number = quarter / 4 % bar + 1;
        let text = if quarter.is_multiple_of(4) {
            format!("{bar_number}.{beat_number}")
        } else {
            format!("{bar_number}.{beat_number}.{}", quarter % 4 + 1)
        };
        label(text, x, true, palette.muted, &mut upper);
        if let Some(text) = clock(beat) {
            label(text, x, false, palette.muted, &mut lower);
        }
        tick(x, if quarter.is_multiple_of(4) { 5.0 } else { 3.0 });
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
    keyboard_active: bool,
    keyboard_focus: Option<Id>,
}

impl TimelinePane {
    pub fn new(id: impl Hash, rect: Rect) -> Self {
        Self {
            id: Id::new(id),
            rect,
            horizontal_only: false,
            keyboard_active: false,
            keyboard_focus: None,
        }
    }

    /// A normalized parameter curve keeps its complete vertical range visible.
    /// Horizontal gestures stay identical to music editors. Ordinary vertical
    /// scrolling remains available to a containing list or popup; explicit
    /// vertical zoom gestures never change the parameter's value scale.
    pub fn horizontal_only(mut self) -> Self {
        self.horizontal_only = true;
        self
    }

    /// Route view shortcuts to the foreground editor even outside its canvas.
    /// The owner must enable this for only one editor. Text entry, disabled UI,
    /// and open menus continue to keep their keyboard input.
    pub fn keyboard_active(mut self) -> Self {
        self.keyboard_active = true;
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
                    view.pan(Vec2::new(
                        delta.x,
                        if self.horizontal_only { 0.0 } else { delta.y },
                    ));
                }
                ui.input_mut(|i| {
                    i.raw_scroll_delta.x = 0.0;
                    i.smooth_scroll_delta.x = 0.0;
                    if !self.horizontal_only
                        || modifiers.ctrl
                        || modifiers.command
                        || modifiers.shift
                    {
                        i.raw_scroll_delta.y = 0.0;
                        i.smooth_scroll_delta.y = 0.0;
                    }
                });
            }
        }
        let focused = ui.ctx().data_mut(|data| data.get_temp::<Id>(focus_id)) == Some(self.id);
        if (hovered || focused || self.keyboard_active)
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
