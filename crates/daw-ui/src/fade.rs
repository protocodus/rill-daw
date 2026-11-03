//! Non-destructive clip envelopes and canvas handles. Callers own source
//! coordinates, snapping, audio processing and the document's undo history.
use crate::{TimelineViewport, theme, tokens};
use egui::{
    Id, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, emath::GuiRounding,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipFadeEdge {
    In,
    Out,
}

#[derive(Clone, Copy, Debug)]
pub struct ClipFadeBounds {
    pub start: f64,
    pub end: f64,
    pub fade_in_end: f64,
    pub fade_out_start: f64,
}

pub struct ClipFadeResponse {
    pub handles: [Option<Rect>; 2],
    /// Arrangement beat of the moved endpoint. Snap and map to the source in
    /// the caller, preserving the other fade and both curve shapes.
    pub edit: Option<(ClipFadeEdge, f64)>,
    /// Suppress body move/trim/cut gestures while a handle owns the pointer.
    pub interacting: bool,
}

// Track clicks per handle; a preceding drag or a click on another control must
// never count toward removing a fade or opening the crossfade editor.
fn handle_double_click(ui: &Ui, id: Id, response: &Response) -> bool {
    let click_id = id.with("last_click");
    let mut open_curve = false;
    if response.drag_started() || response.dragged() || response.drag_stopped() {
        ui.ctx().data_mut(|data| data.remove::<f64>(click_id));
    } else if response.clicked() {
        let now = ui.input(|input| input.time);
        open_curve = ui
            .ctx()
            .data(|data| data.get_temp::<f64>(click_id))
            .is_some_and(|last| now - last < 0.4);
        ui.ctx().data_mut(|data| {
            if open_curve {
                data.remove::<f64>(click_id);
            } else {
                data.insert_temp(click_id, now);
            }
        });
    }
    open_curve
}

/// Draw the actual gain envelope, including inherited trimmed/split envelopes.
/// Work is bounded by the visible canvas width rather than the source length.
pub fn paint_clip_fades(
    painter: &egui::Painter,
    rect: Rect,
    view: TimelineViewport,
    start: f64,
    end: f64,
    gain: impl Fn(f64) -> f32,
) {
    let visible = rect
        .intersect(painter.clip_rect())
        .intersect(Rect::from_min_max(
            Pos2::new(view.x_at(rect, start), rect.top()),
            Pos2::new(view.x_at(rect, end), rect.bottom()),
        ));
    if !visible.is_positive() {
        return;
    }
    let painter = painter.with_clip_rect(visible);
    let color = theme::palette(painter.ctx()).orange;
    let samples = (visible.width().ceil() as usize).clamp(2, 2048);
    let mut points = Vec::with_capacity(samples + 1);
    let mut unity = Vec::with_capacity(samples + 1);
    // Inset unity from the canvas edge so an untouched envelope stays visible.
    let top = rect.top() + crate::tokens::SPACE_XS;
    let height = (rect.bottom() - top).max(0.0);
    for index in 0..=samples {
        let x = egui::lerp(visible.x_range(), index as f32 / samples as f32);
        let value = gain(view.beat_at(rect, x)).clamp(0.0, 1.0);
        points.push(Pos2::new(x, rect.bottom() - value * height));
        unity.push(value >= 0.999);
    }
    // A quiet veil above the envelope distinguishes attenuated audio without
    // repainting or baking the source waveform.
    let mut mesh = egui::Mesh::default();
    for point in &points {
        mesh.colored_vertex(Pos2::new(point.x, rect.top()), color.gamma_multiply(0.12));
        mesh.colored_vertex(*point, color.gamma_multiply(0.12));
    }
    for index in 0..samples as u32 {
        let a = index * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(Shape::mesh(mesh));
    // Unity runs are quiet; attenuated runs (fades, lowered envelope) are solid.
    let mut run = vec![points[0]];
    for index in 1..points.len() {
        run.push(points[index]);
        let quiet = unity[index - 1] && unity[index];
        let next_quiet = index + 1 < points.len() && unity[index] && unity[index + 1];
        if index + 1 == points.len() || quiet != next_quiet {
            let stroke = if quiet {
                Stroke::new(1.0_f32, color.gamma_multiply(0.5))
            } else {
                Stroke::new(2.0_f32, color)
            };
            painter.add(Shape::line(std::mem::take(&mut run), stroke));
            run.push(points[index]);
        }
    }
}

/// Visible fade handle edge length (`SPACE_MD`); the hit target stays 16.
const HANDLE_SIZE: f32 = crate::tokens::SPACE_MD;

/// Dedicated fade targets occupy the top of a clip, separate from its trim
/// edges. They remain available at zero duration and on unselected clips.
pub fn clip_fade_handles(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    view: TimelineViewport,
    bounds: ClipFadeBounds,
    editable: bool,
) -> ClipFadeResponse {
    let mut result = ClipFadeResponse {
        handles: [None, None],
        edit: None,
        interacting: false,
    };
    if !rect.is_positive() || bounds.end <= bounds.start {
        return result;
    }
    let left = view.x_at(rect, bounds.start);
    let right = view.x_at(rect, bounds.end);
    let inset = 7.0_f32.min((right - left) / 4.0);
    let positions = [bounds.fade_in_end, bounds.fade_out_start].map(|beat| {
        view.x_at(rect, beat.clamp(bounds.start, bounds.end))
            .clamp(left + inset, right - inset)
    });
    let separate = (positions[0] - positions[1]).abs() < tokens::HIT_TARGET;
    for (index, edge) in [ClipFadeEdge::In, ClipFadeEdge::Out]
        .into_iter()
        .enumerate()
    {
        let beat =
            [bounds.fade_in_end, bounds.fade_out_start][index].clamp(bounds.start, bounds.end);
        // Square handles sit SPACE_SM below the canvas top, clear of the
        // loop lane and of waveform peaks at full scale.
        let half = HANDLE_SIZE * 0.5;
        let position = Pos2::new(
            positions[index],
            rect.top()
                + crate::tokens::SPACE_SM
                + half
                + if separate && index == 1 {
                    HANDLE_SIZE + crate::tokens::SPACE_XS
                } else {
                    0.0
                },
        );
        let target = Rect::from_center_size(position, Vec2::splat(16.0)).intersect(rect);
        if !ui.clip_rect().contains(position) || !rect.contains(position) {
            continue;
        }
        result.handles[index] = Some(target);
        let mut response = ui.interact(
            target,
            id.with(index),
            if editable {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
        );
        if editable && ui.is_enabled() {
            result.interacting |= response.dragged()
                || response.clicked()
                || ui.input(|input| {
                    input
                        .pointer
                        .hover_pos()
                        .is_some_and(|point| target.contains(point))
                }) && ui.rect_contains_pointer(target);
            let origin_id = id.with(("origin", index));
            if response.is_pointer_button_down_on()
                && ui.input(|input| input.pointer.primary_pressed())
            {
                let x = ui
                    .input(|input| input.pointer.press_origin())
                    .unwrap_or(position)
                    .x;
                ui.ctx()
                    .data_mut(|data| data.insert_temp(origin_id, (beat, x)));
            }
            let origin = ui.ctx().data(|data| data.get_temp::<(f64, f32)>(origin_id));
            let owns = response.is_pointer_button_down_on()
                || response.dragged()
                || response.clicked()
                || response.drag_stopped() && ui.input(|input| input.pointer.primary_released());
            let delta = origin
                .and_then(|(_, x)| {
                    ui.input(|input| input.pointer.interact_pos())
                        .map(|point| point.x - x)
                })
                .unwrap_or(0.0);
            if delta.abs() > 0.0
                && owns
                && let Some((origin, _)) = origin
            {
                ui.ctx()
                    .data_mut(|data| data.remove::<f64>(id.with(index).with("last_click")));
                let raw = origin + f64::from(delta) / f64::from(view.pixels_per_beat);
                let next = if raw <= bounds.start {
                    bounds.start
                } else if raw >= bounds.end {
                    bounds.end
                } else {
                    raw
                };
                result.edit = Some((edge, next));
                result.interacting = true;
            } else if handle_double_click(ui, id.with(index), &response) {
                result.edit = Some((edge, if index == 0 { bounds.start } else { bounds.end }));
            }
            if !ui.input(|input| input.pointer.primary_down()) || !owns {
                ui.ctx()
                    .data_mut(|data| data.remove::<(f64, f32)>(origin_id));
            }
            if result.edit.is_some_and(|(changed, _)| changed == edge) {
                response.mark_changed();
            }
        }
        let label = if index == 0 { "Fade in" } else { "Fade out" };
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Slider, editable && ui.is_enabled(), label)
        });
        let hovered = response.hovered() || response.dragged();
        response
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            .on_hover_text(format!("{label} · Drag to adjust · Double-click to remove"));
        let colors = theme::palette(ui.ctx());
        let painter = ui.painter_at(rect);
        if hovered {
            // A dotted guide shows where the fade boundary meets the audio.
            let x = position.x.round_to_pixel_center(painter.pixels_per_point());
            painter.extend(Shape::dotted_line(
                &[Pos2::new(x, position.y + half), Pos2::new(x, rect.bottom())],
                colors.orange.gamma_multiply(0.7),
                crate::tokens::SPACE_XS,
                0.75,
            ));
        }
        let marker = Rect::from_center_size(position, Vec2::splat(HANDLE_SIZE));
        painter.rect_filled(marker, crate::tokens::NOTE_RADIUS, colors.orange);
        painter.rect_stroke(
            marker,
            crate::tokens::NOTE_RADIUS,
            Stroke::new(1.5_f32, colors.bg),
            StrokeKind::Inside,
        );
    }
    result
}

pub struct CrossfadeHandleResponse {
    pub response: Response,
    pub handle: Option<Rect>,
    pub duration: Option<f64>,
    pub interacting: bool,
    pub open_curve: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct CrossfadeBounds {
    pub start: f64,
    pub end: f64,
    pub duration: f64,
}

/// The lower diamond edits the duration of an incoming overlap. A zero-length
/// transition still has a target; double-click opens the caller's curve editor.
pub fn crossfade_duration_handle(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    view: TimelineViewport,
    bounds: CrossfadeBounds,
    editable: bool,
) -> CrossfadeHandleResponse {
    let CrossfadeBounds {
        start,
        end,
        duration,
    } = bounds;
    let effective = duration.clamp(0.0, (end - start).max(0.0));
    let position = Pos2::new(view.x_at(rect, start + effective), rect.bottom() - 9.0);
    let target = Rect::from_center_size(position, Vec2::splat(tokens::HIT_TARGET)).intersect(rect);
    let origin_id = id.with("origin");
    let active_origin = ui.ctx().data(|data| data.get_temp::<(f64, f32)>(origin_id));
    let visible = end > start
        && (active_origin.is_some()
            || (ui.clip_rect().contains(position) && rect.contains(position)));
    let mut response = ui.interact(
        if visible { target } else { Rect::NOTHING },
        id,
        if editable && visible {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let mut next = None;
    let mut interacting = false;
    if editable && visible && ui.is_enabled() {
        interacting = response.dragged()
            || response.clicked()
            || ui.input(|input| {
                input
                    .pointer
                    .hover_pos()
                    .is_some_and(|point| target.contains(point))
            }) && ui.rect_contains_pointer(target);
        if response.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_pressed())
        {
            let x = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(position)
                .x;
            ui.ctx()
                .data_mut(|data| data.insert_temp(origin_id, (effective, x)));
        }
        let origin = ui.ctx().data(|data| data.get_temp::<(f64, f32)>(origin_id));
        let owns = response.is_pointer_button_down_on()
            || response.dragged()
            || response.clicked()
            || response.drag_stopped() && ui.input(|input| input.pointer.primary_released());
        let delta = origin
            .and_then(|(_, x)| {
                ui.input(|input| input.pointer.interact_pos())
                    .map(|point| point.x - x)
            })
            .unwrap_or(0.0);
        if delta.abs() > 0.0
            && owns
            && let Some((origin, _)) = origin
        {
            ui.ctx()
                .data_mut(|data| data.remove::<f64>(id.with("last_click")));
            let raw = origin + f64::from(delta) / f64::from(view.pixels_per_beat);
            next = Some(if raw <= 0.0 {
                0.0
            } else if raw >= end - start {
                end - start
            } else {
                raw
            });
            interacting = true;
            response.mark_changed();
        }
        if !ui.input(|input| input.pointer.primary_down()) || !owns {
            ui.ctx()
                .data_mut(|data| data.remove::<(f64, f32)>(origin_id));
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Slider,
            editable && visible && ui.is_enabled(),
            "Crossfade duration",
        )
    });
    let open_curve = next.is_none() && handle_double_click(ui, id, &response);
    if visible {
        let color = theme::palette(ui.ctx()).automation_override;
        ui.painter_at(rect).add(Shape::convex_polygon(
            vec![
                position - Vec2::new(0.0, 6.0),
                position + Vec2::new(6.0, 0.0),
                position + Vec2::new(0.0, 6.0),
                position - Vec2::new(6.0, 0.0),
            ],
            color,
            Stroke::new(1.0_f32, theme::palette(ui.ctx()).bg),
        ));
    }
    CrossfadeHandleResponse {
        open_curve,
        interacting,
        response: response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            .on_hover_text("Crossfade · Drag to blend the overlap · Double-click to shape the curve · Shift bypasses snap"),
        handle: visible.then_some(target),
        duration: next,
    }
}
