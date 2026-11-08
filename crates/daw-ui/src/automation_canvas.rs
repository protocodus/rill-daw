//! Inline automation controls use the same curve model and theme as the
//! focused editor. Application views supply data and own document history.
use crate::{
    CurveEdit, CurveInterpolation, CurvePoint, CurveSegment, CurveSegmentHandle, TimelineViewport,
    theme, tokens,
};
use egui::{Color32, Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

pub struct AutomationCanvas {
    pub rect: Rect,
    pub viewport: TimelineViewport,
    pub snap: f64,
    pub color: Color32,
    pub editable: bool,
}

pub struct AutomationCanvasResponse {
    pub response: Response,
    pub edit: Option<CurveEdit>,
    pub points: Vec<(u64, Rect)>,
    pub segments: Vec<CurveSegmentHandle>,
}

pub fn automation_canvas_at(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
    canvas: AutomationCanvas,
    evaluate: impl Fn(f64) -> f32,
) -> AutomationCanvasResponse {
    automation_canvas_with_segments_at(ui, id, points, &[], canvas, evaluate)
}

pub fn automation_canvas_with_segments_at(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
    segments: &[CurveSegment],
    canvas: AutomationCanvas,
    evaluate: impl Fn(f64) -> f32,
) -> AutomationCanvasResponse {
    let AutomationCanvas {
        rect,
        viewport,
        snap,
        color,
        editable,
    } = canvas;
    let editable = editable && ui.is_enabled();
    let mut response = ui.interact(
        rect,
        id,
        if editable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let bypass = ui.input(|input| input.modifiers.shift);
    let painter = ui.painter_at(rect);
    crate::paint_curve(
        &painter,
        rect,
        viewport.start_beat,
        viewport.beat_at(rect, rect.right()),
        color,
        &evaluate,
    );
    let snap = |beat: f64| {
        if snap > 0.0 && !bypass {
            (beat / snap).round() * snap
        } else {
            beat
        }
        .clamp(
            viewport.start_beat.max(0.0),
            viewport
                .beat_at(rect, rect.right())
                .max(viewport.start_beat),
        )
    };
    let mut edit = None;
    let mut hit = false;
    let mut targets = Vec::new();
    let mut segment_handles = Vec::new();
    let mut segment_index = 0;
    for pair in points.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        let Some(segment) = segments.get(segment_index) else {
            break;
        };
        if segment.start != a.id {
            continue;
        }
        segment_index += 1;
        let response = crate::curve_segment::segment_bend_at(
            ui,
            id,
            a,
            b,
            segment.curve,
            rect,
            |beat| viewport.x_at(rect, beat),
            &evaluate,
            editable,
        );
        hit |= response.interacted;
        if let Some((id, curve)) = response.edit {
            edit = Some(CurveEdit::Bend { id, curve });
        }
        if let Some(position) = response.add {
            edit = Some(CurveEdit::Add {
                x: snap(viewport.beat_at(rect, position.x)),
                value: ((rect.bottom() - position.y) / rect.height()).clamp(0.0, 1.0),
            });
        }
        segment_handles.extend(response.handle);
    }
    for point in points {
        let position = Pos2::new(
            viewport.x_at(rect, point.x),
            rect.bottom() - point.value * rect.height(),
        );
        if !rect.contains(position) || !ui.clip_rect().contains(position) {
            continue;
        }
        let target = Rect::from_center_size(position, Vec2::splat(16.0)).intersect(rect);
        let point_response = ui.interact(
            target,
            id.with(point.id),
            if editable {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
        );
        targets.push((point.id, target));
        hit |= point_response.hovered() || point_response.dragged() || point_response.clicked();
        // Inline plots can be only a few dozen points high: waiting for egui's
        // general drag threshold discards audible value changes. Capture the
        // primary press and use its total movement, including sub-threshold
        // drags, while preserving right-click point menus.
        let primary_down = point_response.is_pointer_button_down_on()
            && ui.input(|input| input.pointer.primary_down());
        let primary_release = ui.input(|input| input.pointer.primary_released())
            && (point_response.clicked_by(egui::PointerButton::Primary)
                || point_response.drag_stopped_by(egui::PointerButton::Primary));
        if editable && (primary_down || primary_release) {
            let origin_id = point_response.id.with("origin");
            // A native frame can contain press, movement and release together.
            // Recover the actual press event, not its final pointer position.
            let press = ui.input(|input| {
                input.events.iter().rev().find_map(|event| match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
            });
            if let Some(mut press) = press {
                if let Some(transform) = ui
                    .ctx()
                    .layer_transform_from_global(point_response.layer_id)
                {
                    press = transform * press;
                }
                ui.ctx()
                    .data_mut(|data| data.insert_temp(origin_id, (*point, press)));
            }
            let origin = ui
                .ctx()
                .data(|data| data.get_temp::<(CurvePoint, Pos2)>(origin_id));
            if let Some(((origin, start), current)) =
                origin.zip(point_response.interact_pointer_pos())
            {
                let delta = current - start;
                let (x, value) = if delta == Vec2::ZERO {
                    // Returning to the press restores even an off-grid point.
                    (origin.x, origin.value)
                } else {
                    (
                        snap(origin.x + delta.x as f64 / viewport.pixels_per_beat as f64),
                        (origin.value - delta.y / rect.height()).clamp(0.0, 1.0),
                    )
                };
                if x != point.x || value != point.value {
                    edit = Some(CurveEdit::Move {
                        id: point.id,
                        x,
                        value,
                    });
                }
            }
        }
        if editable {
            point_response.clone().context_menu(|ui| {
                for (mode, label) in [
                    (CurveInterpolation::Linear, "Linear segment"),
                    (CurveInterpolation::Smooth, "Smooth segment"),
                    (CurveInterpolation::Step, "Step segment"),
                ] {
                    if crate::automation_button(ui, label).clicked() {
                        edit = Some(CurveEdit::Interpolation { id: point.id, mode });
                        ui.close();
                    }
                }
                if crate::automation_button(ui, "Delete point").clicked() {
                    edit = Some(CurveEdit::Delete { id: point.id });
                    ui.close();
                }
            });
        }
        let hovered = point_response.hovered() || point_response.dragged();
        point_response.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text("Drag automation point · Right-click for segment shape or Delete · Shift bypasses snap");
        painter.circle_filled(position, if hovered { 5.0 } else { 3.5 }, color);
        painter.circle_stroke(
            position,
            if hovered { 5.0 } else { 3.5 },
            Stroke::new(1.0_f32, theme::palette(ui.ctx()).bg),
        );
    }
    if editable
        && response.clicked()
        && !hit
        && let Some(pointer) = response.interact_pointer_pos()
    {
        edit = Some(CurveEdit::Add {
            x: snap(viewport.beat_at(rect, pointer.x)),
            value: ((rect.bottom() - pointer.y) / rect.height()).clamp(0.0, 1.0),
        });
    }
    if edit.is_some() {
        response.mark_changed();
    }
    AutomationCanvasResponse {
        response,
        edit,
        points: targets,
        segments: segment_handles,
    }
}

pub struct AutomationPatternHeader<'a> {
    pub name: &'a str,
    pub linked: bool,
    pub selected: bool,
    pub color: Color32,
}

/// The header moves/trims a reusable automation clip; its body remains a point
/// editor. A linked identity uses the same title treatment as audio and MIDI.
pub fn automation_pattern_header(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    pattern: AutomationPatternHeader<'_>,
    editable: bool,
) -> Response {
    // The same surface, title band and selection treatment as audio and MIDI clips.
    let painter = ui.painter_at(rect.expand(tokens::SPACE_SM));
    let surface = crate::clip_surface(&painter, rect, pattern.color, pattern.selected);
    let header = surface
        .title
        .with_min_x(rect.left())
        .with_max_x(rect.right());
    crate::clip_title(
        &painter,
        surface.title,
        egui::FontId::proportional(tokens::FONT_CANVAS),
        surface.title_color,
        pattern.name,
        pattern.linked,
    );
    ui.interact(header.intersect(ui.clip_rect()), id, if editable { Sense::click_and_drag() } else { Sense::hover() })
        .on_hover_text("Automation pattern · Drag header to move or its edges to trim · Right-click for Copy, Duplicate or Ghost copy")
}
