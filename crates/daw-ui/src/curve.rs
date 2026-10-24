//! Data-independent breakpoint editor. The owner applies edits, owns undo, and
//! supplies evaluation so the displayed curve matches its processing model.

use crate::{TimelineBounds, TimelineNavigation, TimelinePane, TimelineViewport, theme};
use egui::{Align2, FontId, Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurvePoint {
    pub id: u64,
    pub x: f64,
    pub value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveEdit {
    Add { x: f64, value: f32 },
    Move { id: u64, x: f64, value: f32 },
    Delete { id: u64 },
}

#[derive(Clone, Copy, Debug)]
pub struct CurveView {
    pub start: f64,
    pub end: f64,
    pub snap: f64,
    pub height: f32,
}

pub struct CurveResponse {
    pub response: Response,
    pub plot_rect: Rect,
    pub edit: Option<CurveEdit>,
    pub navigation: TimelineNavigation,
    pub viewport: Option<TimelineViewport>,
}

#[derive(Clone, Copy, Debug)]
pub struct CurveTimeline {
    pub snap: f64,
    pub height: f32,
    pub tempo: f64,
    pub beats_per_bar: u8,
    pub fit: bool,
}

/// Paint a normalized curve in a caller-owned rectangle without allocating UI
/// space. Useful for compact overview lanes as well as the full editor.
pub fn paint_curve(
    painter: &egui::Painter,
    rect: Rect,
    start: f64,
    end: f64,
    color: egui::Color32,
    evaluate: impl Fn(f64) -> f32,
) {
    let samples = (rect.width().ceil() as usize).clamp(2, 2048);
    let mut previous = None;
    for sample in 0..=samples {
        let fraction = sample as f64 / samples as f64;
        let value = evaluate(start + fraction * (end - start)).clamp(0.0, 1.0);
        let position = Pos2::new(
            rect.left() + fraction as f32 * rect.width(),
            rect.bottom() - value * rect.height(),
        );
        if let Some(previous) = previous {
            painter.line_segment([previous, position], Stroke::new(1.8_f32, color));
        }
        previous = Some(position);
    }
}

pub fn curve_editor(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
    selected: &mut Option<u64>,
    view: CurveView,
    evaluate: impl Fn(f64) -> f32,
) -> CurveResponse {
    curve_editor_inner(ui, id, points, selected, view, None, evaluate)
}

/// Musical-time curve editor using the same pan, zoom, Fit, and ruler as other
/// editors. Values always span the complete normalized 0–100% range.
pub fn curve_timeline_editor(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
    selected: &mut Option<u64>,
    viewport: &mut TimelineViewport,
    timeline: CurveTimeline,
    evaluate: impl Fn(f64) -> f32,
) -> CurveResponse {
    curve_editor_inner(
        ui,
        id,
        points,
        selected,
        CurveView {
            start: 0.0,
            end: 8.0,
            snap: timeline.snap,
            height: timeline.height,
        },
        Some((viewport, timeline)),
        evaluate,
    )
}

fn curve_editor_inner(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
    selected: &mut Option<u64>,
    view: CurveView,
    timeline: Option<(&mut TimelineViewport, CurveTimeline)>,
    evaluate: impl Fn(f64) -> f32,
) -> CurveResponse {
    let mut start = view.start.max(0.0);
    let mut end = view.end.max(start + 0.01);
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(180.0), view.height.max(120.0)),
        Sense::click(),
    );
    let musical = timeline.is_some();
    let plot = Rect::from_min_max(
        rect.min
            + Vec2::new(
                46.0,
                if musical {
                    crate::tokens::TIMELINE_RULER_HEIGHT + 12.0
                } else {
                    12.0
                },
            ),
        rect.max - Vec2::new(14.0, if musical { 12.0 } else { 28.0 }),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::palette(ui.ctx()).bg);
    let mut navigation = TimelineNavigation::default();
    let viewport = timeline.map(|(viewport, timeline)| {
        let first = points
            .iter()
            .map(|point| point.x)
            .reduce(f64::min)
            .unwrap_or(0.0);
        let last = points
            .iter()
            .map(|point| point.x)
            .reduce(f64::max)
            .unwrap_or(8.0);
        // A new lane has only its initial point. Leave a full bar to draw into
        // instead of magnifying a fraction of a beat around that point.
        let last = if points.len() == 1 {
            first + f64::from(timeline.beats_per_bar.max(1))
        } else {
            last
        };
        let bounds = TimelineBounds {
            start_beat: first,
            end_beat: last.max(first + 0.25),
            start_row: 0.0,
            end_row: 1.0,
        };
        if timeline.fit {
            viewport.fit_bounds(plot, bounds);
        }
        navigation = TimelinePane::new(id.with("timeline"), plot)
            .horizontal_only()
            .keyboard_focus(response.id)
            .navigate(ui, viewport, bounds);
        // Vertical display remains the complete parameter range, independent of
        // fit padding, vertical pan, and pitch-zoom shortcuts in other editors.
        viewport.row_offset = 0.0;
        viewport.row_height = plot.height();
        start = viewport.start_beat;
        end = viewport.beat_at(plot, plot.right());
        crate::paint_timeline_ruler(
            &painter,
            Rect::from_min_max(
                Pos2::new(plot.left(), rect.top()),
                Pos2::new(plot.right(), plot.top() - 12.0),
            ),
            *viewport,
            timeline.tempo,
            timeline.beats_per_bar,
        );
        *viewport
    });
    let editing =
        !navigation.navigating && !navigation.changed && !egui::Popup::is_any_open(ui.ctx());
    for index in 0..=4 {
        let value = index as f32 / 4.0;
        let y = plot.bottom() - value * plot.height();
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0_f32, theme::palette(ui.ctx()).line),
        );
        painter.text(
            Pos2::new(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{}%", index * 25),
            FontId::monospace(crate::tokens::FONT_CANVAS),
            theme::palette(ui.ctx()).muted,
        );
    }
    let tick = ((end - start) / 12.0).ceil().max(1.0);
    let mut x = (start / tick).ceil() * tick;
    while x <= end {
        let position = plot.left() + ((x - start) / (end - start)) as f32 * plot.width();
        painter.line_segment(
            [
                Pos2::new(position, plot.top()),
                Pos2::new(position, plot.bottom()),
            ],
            Stroke::new(1.0_f32, theme::palette(ui.ctx()).line),
        );
        if !musical {
            painter.text(
                Pos2::new(position, plot.bottom() + 12.0),
                Align2::CENTER_CENTER,
                format!("{x:.0}"),
                FontId::monospace(crate::tokens::FONT_CANVAS),
                theme::palette(ui.ctx()).muted,
            );
        }
        x += tick;
    }
    paint_curve(
        &painter,
        plot,
        start,
        end,
        theme::palette(ui.ctx()).accent,
        evaluate,
    );
    let to_screen = |point: CurvePoint| {
        Pos2::new(
            plot.left() + ((point.x - start) / (end - start)) as f32 * plot.width(),
            plot.bottom() - point.value.clamp(0.0, 1.0) * plot.height(),
        )
    };
    let snap = |x: f64| {
        let x = if view.snap > 0.0 {
            (x / view.snap).round() * view.snap
        } else {
            x
        };
        x.clamp(start, end)
    };
    let mut edit = None;
    let mut point_hit = false;
    for &point in points {
        if point.x < start || point.x > end {
            continue;
        }
        let position = to_screen(point);
        let point_response = ui.interact(
            Rect::from_center_size(position, Vec2::splat(18.0)),
            id.with(point.id),
            Sense::click_and_drag(),
        );
        point_hit |=
            point_response.hovered() || point_response.dragged() || point_response.clicked();
        if editing && (point_response.clicked() || point_response.drag_started()) {
            *selected = Some(point.id);
            response.request_focus();
        }
        if editing && point_response.drag_started() {
            ui.ctx()
                .data_mut(|data| data.insert_temp(id.with("drag"), point));
        }
        if editing && point_response.dragged() {
            let origin = ui
                .ctx()
                .data(|data| data.get_temp::<CurvePoint>(id.with("drag")))
                .unwrap_or(point);
            let delta = point_response.total_drag_delta().unwrap_or_default();
            edit = Some(CurveEdit::Move {
                id: point.id,
                x: snap(origin.x + delta.x as f64 / plot.width() as f64 * (end - start)),
                value: (origin.value - delta.y / plot.height()).clamp(0.0, 1.0),
            });
        }
        if !navigation.navigating {
            point_response.context_menu(|ui| {
                if crate::button(ui, "Delete point").clicked() {
                    edit = Some(CurveEdit::Delete { id: point.id });
                    ui.close();
                }
            });
        }
        painter.circle_filled(
            position,
            if *selected == Some(point.id) {
                5.5
            } else {
                4.0
            },
            theme::palette(ui.ctx()).accent,
        );
        if *selected == Some(point.id) {
            painter.circle_stroke(
                position,
                8.0,
                Stroke::new(1.0_f32, theme::palette(ui.ctx()).text),
            );
        }
    }
    if response.clicked()
        && editing
        && !point_hit
        && let Some(position) = response.interact_pointer_pos()
        && plot.contains(position)
    {
        response.request_focus();
        edit = Some(CurveEdit::Add {
            x: snap(
                start + (position.x - plot.left()) as f64 / plot.width() as f64 * (end - start),
            ),
            value: ((plot.bottom() - position.y) / plot.height()).clamp(0.0, 1.0),
        });
    }
    if response.has_focus()
        && editing
        && ui.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        })
        && let Some(id) = *selected
    {
        edit = Some(CurveEdit::Delete { id });
    }
    if edit.is_some() {
        response.mark_changed();
    }
    CurveResponse {
        response,
        plot_rect: plot,
        edit,
        navigation,
        viewport,
    }
}
