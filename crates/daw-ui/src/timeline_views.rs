//! Shared, presentation-only selection fitting and previous-view recall.
use crate::TimelineViewport;
use egui::{Context, Id, Ui};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum TimelineViewAction {
    #[default]
    None,
    FitSelection,
    Previous,
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
