//! Inline automation controls use the same curve model and theme as the
//! focused editor. Application views supply data and own document history.
use crate::{CurveEdit, CurveInterpolation, CurvePoint, TimelineViewport, theme, tokens};
use egui::{Color32, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

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
}

pub fn automation_canvas_at(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
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
        evaluate,
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
        if editable && point_response.drag_started() {
            ui.ctx()
                .data_mut(|data| data.insert_temp(id.with("origin"), *point));
        }
        if editable && point_response.dragged() {
            let origin = ui
                .ctx()
                .data(|data| data.get_temp::<CurvePoint>(id.with("origin")))
                .unwrap_or(*point);
            let delta = point_response.total_drag_delta().unwrap_or_default();
            edit = Some(CurveEdit::Move {
                id: point.id,
                x: snap(origin.x + delta.x as f64 / viewport.pixels_per_beat as f64),
                value: (origin.value - delta.y / rect.height()).clamp(0.0, 1.0),
            });
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
            Stroke::new(1.0, theme::palette(ui.ctx()).bg),
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
    let colors = theme::palette(ui.ctx());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, tokens::RADIUS, colors.bg);
    painter.rect_filled(
        rect,
        tokens::RADIUS,
        pattern
            .color
            .gamma_multiply(if pattern.selected { 0.25 } else { 0.12 }),
    );
    painter.rect_stroke(
        rect,
        tokens::RADIUS,
        Stroke::new(if pattern.selected { 1.5 } else { 0.8 }, pattern.color),
        StrokeKind::Inside,
    );
    let header = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.right(), (rect.top() + 20.0).min(rect.bottom())),
    );
    painter.rect_filled(header, tokens::RADIUS, pattern.color.gamma_multiply(0.2));
    crate::clip_title(
        &painter,
        header.shrink2(Vec2::new(tokens::SPACE_SM, 0.0)),
        egui::FontId::proportional(tokens::FONT_CANVAS),
        colors.text,
        pattern.name,
        pattern.linked,
    );
    ui.interact(header.intersect(ui.clip_rect()), id, if editable { Sense::click_and_drag() } else { Sense::hover() })
        .on_hover_text("Automation pattern · Drag header to move or its edges to trim · Right-click for Copy, Duplicate or Ghost copy")
}
