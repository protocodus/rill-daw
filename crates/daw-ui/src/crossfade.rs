//! Complementary crossfade spline editor. The caller owns the model and Undo;
//! normalized control points are the only data this component changes.
use crate::{theme, tokens};
use egui::{Align2, FontId, Id, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrossfadeCurve {
    pub control1: [f32; 2],
    pub control2: [f32; 2],
}

impl Default for CrossfadeCurve {
    fn default() -> Self {
        Self {
            control1: [1.0 / 3.0, 1.0 / 3.0],
            control2: [2.0 / 3.0, 2.0 / 3.0],
        }
    }
}

impl CrossfadeCurve {
    fn display_points(self) -> [[f32; 2]; 2] {
        let defaults = Self::default();
        let mut points = [self.control1, self.control2];
        for (point, fallback) in points
            .iter_mut()
            .zip([defaults.control1, defaults.control2])
        {
            for (value, default) in point.iter_mut().zip(fallback) {
                *value = if value.is_finite() {
                    value.clamp(0.0, 1.0)
                } else {
                    default
                };
            }
        }
        points[0][0] = points[0][0].min(points[1][0]);
        points
    }

    fn move_handle(&mut self, index: usize, point: [f32; 2]) {
        let points = self.display_points();
        let min = if index == 0 { 0.0 } else { points[0][0] };
        let max = if index == 0 { points[1][0] } else { 1.0 };
        let next = [point[0].clamp(min, max), point[1].clamp(0.0, 1.0)];
        if index == 0 {
            self.control1 = next;
        } else {
            self.control2 = next;
        }
    }
}

pub struct CrossfadeResponse {
    pub response: Response,
    /// Keyboard targets, so applications can route document Undo while a curve
    /// handle has focus without stealing text-field Undo.
    pub focus_ids: [Id; 3],
    pub plot_rect: Rect,
    /// Screen-space interaction bounds for the two handles, after this edit.
    pub handles: [Rect; 2],
    pub drag_started: bool,
    pub drag_stopped: bool,
}

fn screen(plot: Rect, point: [f32; 2]) -> Pos2 {
    Pos2::new(
        plot.left() + point[0] * plot.width(),
        plot.bottom() - point[1] * plot.height(),
    )
}

fn spline(points: [[f32; 2]; 2], t: f32) -> [f32; 2] {
    let remaining = 1.0 - t;
    let a = 3.0 * remaining * remaining * t;
    let b = 3.0 * remaining * t * t;
    let c = t * t * t;
    [
        a * points[0][0] + b * points[1][0] + c,
        a * points[0][1] + b * points[1][1] + c,
    ]
}

fn paint_gain_curves(painter: &egui::Painter, plot: Rect, curve: CrossfadeCurve, width: f32) {
    let colors = theme::palette(painter.ctx());
    let points = curve.display_points();
    let samples = (plot.width().ceil() as usize).clamp(48, 512);
    let mut incoming = Vec::with_capacity(samples + 1);
    let mut outgoing = Vec::with_capacity(samples + 1);
    for index in 0..=samples {
        // Parametric sampling draws the same x→y Bézier relation as the audio
        // model without approximating its inverse or depending on daw-core.
        let point = spline(points, index as f32 / samples as f32);
        incoming.push(screen(plot, point));
        outgoing.push(screen(plot, [point[0], 1.0 - point[1]]));
    }
    painter.add(Shape::line(outgoing, Stroke::new(width, colors.muted)));
    painter.add(Shape::line(incoming, Stroke::new(width, colors.accent)));
}

/// Paint a noninteractive timeline crossfade overlay without changing layout.
/// Both gain curves use exactly the same spline as [`crossfade_editor`].
pub fn paint_crossfade(painter: &egui::Painter, rect: Rect, curve: CrossfadeCurve) {
    if !rect.is_finite() || !rect.is_positive() {
        return;
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let colors = theme::palette(painter.ctx());
    painter.rect_filled(rect, 0.0, colors.accent.gamma_multiply(0.08));
    paint_gain_curves(&painter, rect, curve, 1.5);
}

/// Edit a normalized cubic Bézier from (0,0) to (1,1). Incoming gain is the
/// spline and outgoing gain its complement, so their sum remains one. Handle
/// times stay ordered; each gain control is independently bounded to [0,1].
///
/// Drag either handle, or focus it and use arrows (Shift: fine adjustment).
/// A background click selects the nearest handle. Home restores a linear fade.
/// `Response::changed()` reports actual edits; drag flags let callers group Undo.
pub fn crossfade_editor(ui: &mut Ui, id: Id, curve: &mut CrossfadeCurve) -> CrossfadeResponse {
    let before = *curve;
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(180.0), 220.0),
        Sense::click(),
    );
    let plot = Rect::from_min_max(
        rect.min + Vec2::new(40.0, 34.0),
        rect.max - Vec2::new(16.0, 28.0),
    );
    let selected_id = id.with("selected");
    let mut selected = ui
        .ctx()
        .data(|data| data.get_temp::<usize>(selected_id))
        .unwrap_or(0)
        .min(1);
    let mut drag_started = false;
    let mut drag_stopped = false;
    let mut handle_focus = [false; 2];
    let mut handle_responses: [Option<Response>; 2] = [None, None];
    let mut handle_hit = false;
    for (index, focus) in handle_focus.iter_mut().enumerate() {
        let point = curve.display_points()[index];
        let handle = ui.interact(
            Rect::from_center_size(screen(plot, point), Vec2::splat(22.0)),
            id.with(index),
            Sense::click_and_drag(),
        );
        handle_hit |= handle.hovered() || handle.dragged() || handle.clicked();
        if handle.clicked() || handle.drag_started() {
            selected = index;
            handle.request_focus();
        }
        if handle.drag_started() {
            ui.ctx()
                .data_mut(|data| data.insert_temp(id.with(("origin", index)), point));
            drag_started = true;
        }
        if handle.dragged() {
            let origin = ui
                .ctx()
                .data(|data| data.get_temp::<[f32; 2]>(id.with(("origin", index))))
                .unwrap_or(point);
            let delta = handle.total_drag_delta().unwrap_or_default();
            curve.move_handle(
                index,
                [
                    origin[0] + delta.x / plot.width(),
                    origin[1] - delta.y / plot.height(),
                ],
            );
        }
        drag_stopped |= handle.drag_stopped();
        *focus = handle.has_focus();
        handle_responses[index] = Some(
            handle
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text(
                    "Drag to shape the crossfade. Arrow keys adjust; Shift makes fine adjustments.",
                ),
        );
    }
    if response.clicked()
        && !handle_hit
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let points = curve.display_points();
        selected = usize::from(
            pointer.distance_sq(screen(plot, points[1]))
                < pointer.distance_sq(screen(plot, points[0])),
        );
        response.request_focus();
    }
    if ui.is_enabled() && (response.has_focus() || handle_focus.iter().any(|focused| *focused)) {
        if let Some(index) = handle_focus.iter().position(|focused| *focused) {
            selected = index;
        }
        let delta = ui.input_mut(|input| {
            if input.modifiers.command || input.modifiers.ctrl || input.modifiers.alt {
                return None;
            }
            let modifiers = input.modifiers;
            let step = if modifiers.shift { 0.002 } else { 0.02 };
            let mut delta = Vec2::ZERO;
            if input.consume_key(modifiers, egui::Key::ArrowLeft) {
                delta.x -= step;
            }
            if input.consume_key(modifiers, egui::Key::ArrowRight) {
                delta.x += step;
            }
            if input.consume_key(modifiers, egui::Key::ArrowDown) {
                delta.y -= step;
            }
            if input.consume_key(modifiers, egui::Key::ArrowUp) {
                delta.y += step;
            }
            Some((delta, input.consume_key(modifiers, egui::Key::Home)))
        });
        if let Some((delta, reset)) = delta {
            if reset {
                *curve = CrossfadeCurve::default();
            } else if delta != Vec2::ZERO {
                let point = curve.display_points()[selected];
                curve.move_handle(selected, [point[0] + delta.x, point[1] + delta.y]);
            }
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(selected_id, selected));
    let points = curve.display_points();
    for (index, handle) in handle_responses.into_iter().enumerate() {
        let mut handle = handle.unwrap();
        if points[index] != before.display_points()[index] {
            handle.mark_changed();
        }
        handle.widget_info(|| {
            egui::WidgetInfo::slider(
                ui.is_enabled(),
                f64::from(points[index][1]),
                format!(
                    "Crossfade handle {} · time {:.0}%",
                    index + 1,
                    points[index][0] * 100.0
                ),
            )
        });
    }
    let handles =
        points.map(|point| Rect::from_center_size(screen(plot, point), Vec2::splat(22.0)));
    let painter = ui.painter_at(rect);
    let colors = theme::palette(ui.ctx());
    painter.rect_filled(rect, tokens::RADIUS, colors.bg);
    let font = FontId::proportional(tokens::FONT_CAPTION);
    for (index, label, color) in [
        (0, "Outgoing", colors.muted),
        (1, "Incoming", colors.accent),
    ] {
        let x = plot.left() + index as f32 * 112.0;
        let y = rect.top() + 14.0;
        painter.line_segment(
            [Pos2::new(x, y), Pos2::new(x + 20.0, y)],
            Stroke::new(2.0_f32, color),
        );
        painter.text(
            Pos2::new(x + 26.0, y),
            Align2::LEFT_CENTER,
            label,
            font.clone(),
            color,
        );
    }
    for fraction in [0.0, 0.5, 1.0] {
        let point = screen(plot, [fraction, fraction]);
        painter.line_segment(
            [
                Pos2::new(plot.left(), point.y),
                Pos2::new(plot.right(), point.y),
            ],
            Stroke::new(1.0_f32, colors.line),
        );
        painter.line_segment(
            [
                Pos2::new(point.x, plot.top()),
                Pos2::new(point.x, plot.bottom()),
            ],
            Stroke::new(1.0_f32, colors.line),
        );
    }
    for (value, label) in [(0.0, "0"), (1.0, "1")] {
        painter.text(
            screen(plot, [0.0, value]) - Vec2::new(8.0, 0.0),
            Align2::RIGHT_CENTER,
            label,
            font.clone(),
            colors.muted,
        );
    }
    painter.text(
        Pos2::new(plot.left(), plot.bottom() + 15.0),
        Align2::LEFT_CENTER,
        "Start",
        font.clone(),
        colors.muted,
    );
    painter.text(
        Pos2::new(plot.right(), plot.bottom() + 15.0),
        Align2::RIGHT_CENTER,
        "End",
        font,
        colors.muted,
    );
    paint_gain_curves(&painter, plot, *curve, 2.0);
    for (index, point) in points.iter().enumerate() {
        let anchor = screen(plot, [index as f32, index as f32]);
        let position = screen(plot, *point);
        painter.line_segment([anchor, position], Stroke::new(1.0_f32, colors.muted));
        painter.circle_filled(anchor, 3.0, colors.accent);
        painter.circle_filled(position, 5.0, colors.accent);
        if selected == index {
            painter.circle_stroke(
                position,
                8.0,
                Stroke::new(
                    if handle_focus[index] || response.has_focus() {
                        1.5_f32
                    } else {
                        1.0_f32
                    },
                    colors.text,
                ),
            );
        }
    }
    if *curve != before {
        response.mark_changed();
    }
    CrossfadeResponse {
        focus_ids: [response.id, id.with(0_usize), id.with(1_usize)],
        response,
        plot_rect: plot,
        handles,
        drag_started,
        drag_stopped,
    }
}
