//! A ruler lane for setting and moving a transport loop without a popup.
use crate::{TimelineViewport, theme};
use egui::{Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineLoop {
    pub start: f64,
    pub end: f64,
    pub enabled: bool,
}

#[derive(Clone, Copy)]
enum LoopDragMode {
    Start,
    End,
    Move,
    Draw,
}
#[derive(Clone, Copy)]
struct LoopDrag {
    original: TimelineLoop,
    anchor: f64,
    mode: LoopDragMode,
}

pub struct TimelineLoopResponse {
    pub changed: bool,
    pub lane: Rect,
    pub handles: [Option<Rect>; 2],
}

/// Click to place the nearest boundary. Drag an edge to resize, the band to
/// move, or empty space to draw a new loop. Editing enables repetition. Shift
/// bypasses the grid; the range always remains ordered and within `limit`.
pub fn timeline_loop_editor(
    ui: &mut Ui,
    id: Id,
    lane: Rect,
    view: TimelineViewport,
    range: &mut TimelineLoop,
    grid: f64,
    limit: f64,
) -> TimelineLoopResponse {
    let before = *range;
    let bypass = ui.input(|input| input.modifiers.shift);
    let editable = ui.is_enabled()
        && !ui
            .input(|input| input.modifiers.alt || input.modifiers.ctrl || input.modifiers.command);
    let minimum = if grid > 0.0 && !bypass { grid } else { 0.001 };
    let snap = |beat: f64| {
        if grid > 0.0 && !bypass {
            (beat / grid).round() * grid
        } else {
            beat
        }
        .clamp(0.0, limit)
    };
    let x = [view.x_at(lane, range.start), view.x_at(lane, range.end)];
    let response = ui.interact(
        lane,
        id,
        if editable {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    if response.drag_started() {
        let point = ui
            .input(|input| input.pointer.press_origin())
            .unwrap_or(lane.center());
        let distance = [(point.x - x[0]).abs(), (point.x - x[1]).abs()];
        let mode = if distance[0].min(distance[1]) <= 8.0 {
            if distance[0] <= distance[1] {
                LoopDragMode::Start
            } else {
                LoopDragMode::End
            }
        } else if point.x > x[0] && point.x < x[1] {
            LoopDragMode::Move
        } else {
            LoopDragMode::Draw
        };
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                id.with("drag"),
                LoopDrag {
                    original: *range,
                    anchor: view.beat_at(lane, point.x),
                    mode,
                },
            )
        });
    }
    if editable
        && response.dragged()
        && let Some(drag) = ui
            .ctx()
            .data(|data| data.get_temp::<LoopDrag>(id.with("drag")))
        && let Some(point) = response.interact_pointer_pos()
    {
        let beat = snap(view.beat_at(lane, point.x));
        match drag.mode {
            LoopDragMode::Start => range.start = beat.min(range.end - minimum).max(0.0),
            LoopDragMode::End => range.end = beat.max(range.start + minimum).min(limit),
            LoopDragMode::Move => {
                let length = drag.original.end - drag.original.start;
                range.start = snap(drag.original.start + view.beat_at(lane, point.x) - drag.anchor)
                    .min(limit - length)
                    .max(0.0);
                range.end = range.start + length;
            }
            LoopDragMode::Draw => {
                let anchor = snap(drag.anchor).min(limit - minimum);
                range.start = anchor.min(beat).min(limit - minimum);
                range.end = anchor.max(beat).max(range.start + minimum).min(limit);
            }
        }
        range.enabled = true;
    } else if editable
        && response.clicked()
        && let Some(point) = response.interact_pointer_pos()
    {
        let beat = snap(view.beat_at(lane, point.x));
        if (beat - range.start).abs() <= (beat - range.end).abs() {
            range.start = beat.min(range.end - minimum).max(0.0);
        } else {
            range.end = beat.max(range.start + minimum).min(limit);
        }
        range.enabled = true;
    }
    if response.drag_stopped() || !ui.input(|input| input.focused) {
        ui.ctx()
            .data_mut(|data| data.remove::<LoopDrag>(id.with("drag")));
    }
    response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Loop · Click to set a boundary · Drag edges to resize, the band to move, or empty space for a new range · Shift bypasses snap");
    let colors = theme::palette(ui.ctx());
    let painter = ui.painter_at(lane);
    painter.rect_filled(lane, 0.0, colors.bg);
    painter.line_segment(
        [lane.left_bottom(), lane.right_bottom()],
        Stroke::new(1.0, colors.line),
    );
    let x = [view.x_at(lane, range.start), view.x_at(lane, range.end)];
    let band = Rect::from_min_max(
        Pos2::new(x[0], lane.top() + 2.0),
        Pos2::new(x[1], lane.bottom() - 2.0),
    );
    let color = if range.enabled {
        colors.guide
    } else {
        colors.muted
    };
    painter.rect_filled(
        band,
        2.0,
        color.gamma_multiply(if range.enabled { 0.35 } else { 0.12 }),
    );
    let handles = x.map(|x| {
        let position = Pos2::new(x, lane.center().y);
        if !lane.contains(position) {
            return None;
        }
        let rect = Rect::from_center_size(position, Vec2::new(12.0, lane.height())).intersect(lane);
        painter.rect_filled(
            Rect::from_center_size(position, Vec2::new(3.0, lane.height() - 4.0)),
            1.0,
            color,
        );
        Some(rect)
    });
    TimelineLoopResponse {
        changed: *range != before,
        lane,
        handles,
    }
}
