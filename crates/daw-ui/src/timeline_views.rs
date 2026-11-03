//! Shared, presentation-only selection fitting and previous-view recall.
use crate::TimelineViewport;
use egui::{Context, Id, Rect, Ui};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum TimelineViewAction {
    #[default]
    None,
    ZoomOut,
    ZoomIn,
    FitAll,
    FitSelection,
    Previous,
}

impl TimelineViewAction {
    pub fn is_zoom(self) -> bool {
        matches!(self, Self::ZoomOut | Self::ZoomIn)
    }

    /// Match keyboard zoom factors and bounds, anchored to the visible center.
    /// The toolbar is outside the canvas, so its pointer position is not an anchor.
    pub fn apply_zoom(self, view: &mut TimelineViewport, canvas: Rect) {
        let factor = match self {
            Self::ZoomOut => 0.8,
            Self::ZoomIn => 1.25,
            _ => return,
        };
        view.zoom(canvas, canvas.center(), factor, 1.0);
    }
}

/// Keep ordinary zoom and selection framing visible without opening a menu.
/// Four quiet, same-sized controls wrap as one unit and never change width
/// with the selection. Actions only describe presentation changes; the caller
/// owns the viewport.
pub fn timeline_zoom_controls(ui: &mut Ui, selection_available: bool) -> TimelineViewAction {
    let square = crate::tokens::CONTROL_HEIGHT;
    let width = square * 4.0 + 3.0 * crate::tokens::SPACE_XS;
    crate::toolbar_group_sized_with_spacing(ui, width, crate::tokens::SPACE_XS, |ui| {
        // Square targets: an 18-point glyph plus 8 points of padding each side.
        ui.spacing_mut().button_padding.x = crate::tokens::SPACE_SM;
        let mut action = TimelineViewAction::None;
        for (symbol, label, hint, value) in [
            (
                "−",
                "Zoom out",
                "Zoom out around the view center · −",
                TimelineViewAction::ZoomOut,
            ),
            (
                "+",
                "Zoom in",
                "Zoom in around the view center · +",
                TimelineViewAction::ZoomIn,
            ),
        ] {
            let button = ui
                .add_sized(
                    egui::vec2(square, square),
                    crate::button_widget(symbol).presentation().quiet(),
                )
                .on_hover_text(hint);
            button.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
            });
            if button.clicked() {
                action = value;
            }
        }
        if ui
            .add_sized(
                egui::vec2(square, square),
                crate::IconButton::icon_only(crate::Icon::Fit, "Fit")
                    .presentation()
                    .quiet(),
            )
            .on_hover_text("Fit all content on both axes · F")
            .clicked()
        {
            action = TimelineViewAction::FitAll;
        }
        if ui
            .add_enabled_ui(selection_available, |ui| {
                ui.add_sized(
                    egui::vec2(square, square),
                    crate::IconButton::icon_only(crate::Icon::FitSelection, "Fit selection")
                        .presentation()
                        .quiet(),
                )
            })
            .inner
            .on_hover_text(if selection_available {
                "Fit the selected clips, notes or time range. Return with View → Previous view."
            } else {
                "Select clips, notes or a time range to fit the selection"
            })
            .clicked()
        {
            action = TimelineViewAction::FitSelection;
        }
        action
    })
    .inner
}

/// Reserve a toolbar group at the width its contents took on the previous
/// frame, so a wrapping row moves the complete group without hand-measured widths.
pub fn measured_toolbar_group<R>(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    spacing: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let id = ui.id().with(("measured_toolbar_group", id_salt));
    let width = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or_default();
    let group = crate::toolbar_group_sized_with_spacing(ui, width, spacing, |ui| {
        unbounded(ui);
        contents(ui)
    });
    let measured = group.response.rect.width();
    if measured != width {
        ui.ctx().data_mut(|data| data.insert_temp(id, measured));
        ui.ctx().request_discard("toolbar group width changed");
    }
    group
}

/// The reservation only decides wrapping. Contents measure at their natural
/// width, so a label that grows is never truncated to last frame's width.
fn unbounded(ui: &mut Ui) {
    ui.set_max_width(ui.ctx().content_rect().width());
}

/// `measured_toolbar_group` for a shared `command_group` surface.
pub fn measured_command_group<R>(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let id = ui.id().with(("measured_command_group", id_salt));
    let width = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(id))
        .unwrap_or_default();
    let group = crate::command_group(ui, width, |ui| {
        unbounded(ui);
        contents(ui)
    });
    let measured = group.response.rect.width();
    if measured != width {
        ui.ctx().data_mut(|data| data.insert_temp(id, measured));
        ui.ctx().request_discard("command group width changed");
    }
    group
}

/// Transport position: "1.1.1" over a muted clock, at a width reserved for
/// the longest common values so playback never shifts neighbouring controls.
pub fn transport_position(
    ui: &mut Ui,
    beat: f64,
    beats_per_bar: u8,
    seconds: f64,
) -> egui::Response {
    let colors = crate::theme::palette(ui.ctx());
    let position = egui::FontId::proportional(crate::tokens::FONT_SUBTITLE);
    let clock = egui::FontId::proportional(crate::tokens::FONT_CAPTION);
    let measure = |text: &str, font: &egui::FontId| {
        ui.painter()
            .layout_no_wrap(text.into(), font.clone(), colors.text)
            .size()
            .x
            .ceil()
    };
    let position_width = measure("188.8.8", &position);
    let clock_width = measure("18:88.88", &clock);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_SM;
        let slot = |ui: &mut Ui, width: f32, text: String, font: egui::FontId, color| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, crate::tokens::CONTROL_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(width);
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).font(font).color(color))
                            .selectable(false),
                    )
                },
            )
            .inner
        };
        let bar_beat = slot(
            ui,
            position_width,
            crate::musical_position_label(beat, beats_per_bar),
            position,
            colors.text,
        );
        // Always two decimals: a trimmed "0:01.5" would change width mid-play.
        let hundredths = (seconds.max(0.0) * 100.0).round() as u64;
        slot(
            ui,
            clock_width,
            format!(
                "{}:{:02}.{:02}",
                hundredths / 6000,
                hundredths / 100 % 60,
                hundredths % 100
            ),
            clock,
            colors.muted,
        );
        bar_beat
    })
    .inner
    .on_hover_text("Playhead · bar.beat.sixteenth and elapsed time")
}

#[derive(Clone, Default)]
struct ViewHistory {
    previous: Option<TimelineViewport>,
    last_change: Option<f64>,
    pointer_down: bool,
    drag_start: Option<TimelineViewport>,
}

fn key(scope: &str) -> Id {
    Id::new("timeline_previous_view").with(scope)
}

pub fn timeline_selection_view_actions(
    ui: &mut Ui,
    scope: &str,
    selection_available: bool,
) -> TimelineViewAction {
    let has_previous = ui.ctx().data(|data| {
        data.get_temp::<ViewHistory>(key(scope))
            .is_some_and(|history| history.previous.is_some())
    });
    if ui
        .add_enabled(
            selection_available,
            crate::view_button_widget("Fit selection"),
        )
        .on_hover_text("Fit the selected items while preserving their data")
        .clicked()
    {
        ui.close();
        return TimelineViewAction::FitSelection;
    }
    if ui
        .add_enabled(has_previous, crate::view_button_widget("Previous view"))
        .on_hover_text("Return to the view before the last pan, zoom or Fit")
        .clicked()
    {
        ui.close();
        return TimelineViewAction::Previous;
    }
    TimelineViewAction::None
}

/// Coalesce one continuous wheel/drag gesture rather than remembering every frame.
pub fn remember_timeline_view(
    ctx: &Context,
    scope: &str,
    before: TimelineViewport,
    after: TimelineViewport,
    explicit_fit: bool,
) {
    let now = ctx.input(|input| input.time);
    let pointer_down = ctx.input(|input| input.pointer.any_down());
    ctx.data_mut(|data| {
        let mut history = data.get_temp::<ViewHistory>(key(scope)).unwrap_or_default();
        if pointer_down && !history.pointer_down {
            history.drag_start = Some(before);
        }
        if before != after {
            if explicit_fit
                || pointer_down && history.drag_start.is_some()
                || !pointer_down && history.last_change.is_none_or(|last| now - last > 0.3)
            {
                history.previous = Some(history.drag_start.take().unwrap_or(before));
            }
            history.last_change = Some(now);
        }
        if !pointer_down {
            history.drag_start = None;
        }
        history.pointer_down = pointer_down;
        data.insert_temp(key(scope), history);
    });
}

pub fn restore_previous_timeline_view(
    ctx: &Context,
    scope: &str,
    view: &mut TimelineViewport,
) -> bool {
    let now = ctx.input(|input| input.time);
    let pointer_down = ctx.input(|input| input.pointer.any_down());
    ctx.data_mut(|data| {
        let mut history = data.get_temp::<ViewHistory>(key(scope)).unwrap_or_default();
        let Some(previous) = history.previous else {
            return false;
        };
        history.previous = Some(*view);
        history.last_change = Some(now);
        history.pointer_down = pointer_down;
        history.drag_start = None;
        *view = previous;
        data.insert_temp(key(scope), history);
        true
    })
}

pub fn reset_timeline_view_history(ctx: &Context, scope: &str) {
    ctx.data_mut(|data| data.remove::<ViewHistory>(key(scope)));
}
