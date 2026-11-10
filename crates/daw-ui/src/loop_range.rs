//! A ruler lane for setting and moving a transport loop without a popup.
use crate::{TimelineViewport, theme};
use egui::{Id, Pos2, Rect, Sense, Ui, Vec2};

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

/// Musical context for the strip's label. Without it the strip stays unlabelled.
#[derive(Clone, Copy, Debug)]
pub struct TimelineLoopLabel<'a> {
    pub tempo: f64,
    pub meter: &'a crate::MeterMap,
    pub playhead: f64,
}

pub struct TimelineLoopResponse {
    pub changed: bool,
    /// A drag began this frame; the caller can start a separate undo step.
    pub started: bool,
    pub lane: Rect,
    /// The visible part of the loop band, if any.
    pub band: Option<Rect>,
    pub handles: [Option<Rect>; 2],
}

/// "1.1" for a downbeat, "9.2.3" otherwise: the ruler's form without a
/// redundant first sixteenth.
fn short_position(beat: f64, meter: &crate::MeterMap) -> String {
    let label = crate::musical_position_label(beat, meter);
    match label.strip_suffix(".1") {
        Some(short) => short.to_owned(),
        None => label,
    }
}

/// Short strip label: "1.1 – 9.1 · 0:17.1" (start, end, duration).
pub fn loop_strip_label(start: f64, end: f64, tempo: f64, meter: &crate::MeterMap) -> String {
    format!(
        "{} – {} · {}",
        short_position(start, meter),
        short_position(end, meter),
        crate::format_clock_time((end - start) * 60.0 / tempo.max(1.0), 1),
    )
}

/// Click to place the nearest boundary. Drag an edge to resize, the band to
/// move, or empty space to draw a new loop. Editing enables repetition. Shift
/// bypasses the grid; the range always remains ordered and within `limit`.
/// With `label`, the band names its range and duration, kept clear of the playhead.
#[allow(clippy::too_many_arguments)]
pub fn timeline_loop_editor(
    ui: &mut Ui,
    id: Id,
    lane: Rect,
    view: TimelineViewport,
    range: &mut TimelineLoop,
    grid: f64,
    limit: f64,
    label: Option<TimelineLoopLabel<'_>>,
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
    let started = response.drag_started() && editable;
    let hint = "Click to set a boundary · Drag edges to resize, the band to move, or empty space for a new range · Shift bypasses snap";
    let hover = match label {
        Some(label) => {
            let seconds =
                |beat: f64| crate::duration::format_seconds(beat * 60.0 / label.tempo.max(1.0));
            format!(
                "Loop from {} ({}) to {} ({}) · {}\n{hint}",
                crate::duration::musical_position(range.start, label.meter),
                seconds(range.start),
                crate::duration::musical_position(range.end, label.meter),
                seconds(range.end),
                seconds(range.end - range.start),
            )
        }
        None => format!("Loop · {hint}"),
    };
    response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text(hover);
    let colors = theme::palette(ui.ctx());
    let painter = ui.painter_at(lane);
    painter.rect_filled(lane, 0.0, colors.bg);
    let x = [view.x_at(lane, range.start), view.x_at(lane, range.end)];
    let inset = crate::tokens::BORDER_WIDTH * 2.0;
    let band = Rect::from_min_max(
        Pos2::new(x[0], lane.top() + inset),
        Pos2::new(x[1], lane.bottom() - inset),
    );
    let color = if range.enabled {
        colors.guide
    } else {
        colors.muted
    };
    // Opaque, so the label can take a contrasting ink.
    let fill = colors
        .bg
        .lerp_to_gamma(color, if range.enabled { 0.35 } else { 0.12 });
    painter.rect_filled(band, 2.0, fill);
    let visible = band.intersect(lane);
    if let Some(label) = label
        && range.enabled
        && visible.is_positive()
    {
        let galley = painter.layout_no_wrap(
            loop_strip_label(range.start, range.end, label.tempo, label.meter),
            egui::FontId::proportional(crate::tokens::FONT_CAPTION),
            crate::theme::contrast_text(fill),
        );
        let natural = visible.left() + crate::tokens::SPACE_SM;
        let playhead = view.x_at(lane, label.playhead);
        // Step past the playhead only where it would cross the text.
        let left = if (natural - crate::tokens::SPACE_XS
            ..=natural + galley.size().x + crate::tokens::SPACE_XS)
            .contains(&playhead)
        {
            playhead + crate::tokens::SPACE_SM
        } else {
            natural
        };
        if left + galley.size().x + crate::tokens::SPACE_XS <= visible.right()
            && galley.size().y <= lane.height()
        {
            painter.galley(
                Pos2::new(left, visible.center().y - galley.size().y * 0.5),
                galley,
                crate::theme::contrast_text(fill),
            );
        }
    }
    let handles = x.map(|x| {
        let position = Pos2::new(x, lane.center().y);
        if !lane.contains(position) {
            return None;
        }
        let rect = Rect::from_center_size(position, Vec2::new(12.0, lane.height())).intersect(lane);
        painter.rect_filled(
            Rect::from_center_size(position, Vec2::new(3.0, lane.height() - 6.0)),
            1.0,
            color,
        );
        Some(rect)
    });
    TimelineLoopResponse {
        changed: *range != before,
        started,
        lane,
        band: visible.is_positive().then_some(visible),
        handles,
    }
}
