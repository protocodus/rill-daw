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
/// The family wraps as one unit and never changes width with the selection.
/// Actions only describe presentation changes; the caller owns the viewport.
pub fn timeline_zoom_controls(ui: &mut Ui, selection_available: bool) -> TimelineViewAction {
    let square = crate::tokens::CONTROL_HEIGHT;
    let text_width = |label: &str| {
        ui.painter()
            .layout_no_wrap(
                label.into(),
                egui::TextStyle::Button.resolve(ui.style()),
                ui.visuals().text_color(),
            )
            .size()
            .x
    };
    let fit_width =
        text_width("Fit") + 18.0 + crate::tokens::SPACE_XS + 2.0 * ui.spacing().button_padding.x;
    let selection_width = text_width("Fit selection") + 2.0 * ui.spacing().button_padding.x;
    let width =
        square * 2.0 + fit_width.ceil() + selection_width.ceil() + 3.0 * crate::tokens::SPACE_XS;
    crate::tool_group(ui, width, |ui| {
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
                    egui::vec2(square, crate::tokens::CONTROL_HEIGHT),
                    crate::button_widget(symbol).presentation(),
                )
                .on_hover_text(hint);
            button.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
            });
            if button.clicked() {
                action = value;
            }
        }
        if crate::timeline_fit_button(ui).clicked() {
            action = TimelineViewAction::FitAll;
        }
        if ui
            .add_enabled(
                selection_available,
                crate::button_widget("Fit selection").presentation(),
            )
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
