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

/// Shape of the segment leaving one breakpoint. The owner evaluates the curve;
/// this normalized bias only drives the shared editing affordance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveSegment {
    pub start: u64,
    pub curve: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct CurveSegmentHandle {
    pub start: u64,
    pub end: u64,
    pub rect: Rect,
    pub curve: f32,
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
    pub selection: Vec<u64>,
    pub delete_selection: bool,
    pub segment_edit: Option<(u64, f32)>,
    pub segment_handles: Vec<CurveSegmentHandle>,
}

#[derive(Clone, Copy, Debug)]
pub struct CurveTimeline {
    pub snap: f64,
    pub height: f32,
    pub tempo: f64,
    pub beats_per_bar: u8,
    pub fit: bool,
    /// Route view shortcuts here when this is the foreground editor.
    pub keyboard_active: bool,
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
    let mut path = Vec::with_capacity(samples + 1);
    for sample in 0..=samples {
        let fraction = sample as f64 / samples as f64;
        let value = evaluate(start + fraction * (end - start)).clamp(0.0, 1.0);
        let position = Pos2::new(
            rect.left() + fraction as f32 * rect.width(),
            rect.bottom() - value * rect.height(),
        );
        path.push(position);
    }
    painter.add(egui::Shape::line(path, Stroke::new(1.8_f32, color)));
}

pub fn curve_editor(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
    selected: &mut Option<u64>,
    view: CurveView,
    evaluate: impl Fn(f64) -> f32,
) -> CurveResponse {
    curve_editor_inner(ui, id, points, &[], None, selected, view, None, evaluate)
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
        &[],
        None,
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

/// Timeline editor with independently adjustable spline segments. Diamond
/// handles select their outgoing breakpoint and bend the segment vertically;
/// native context menus expose the same Log, Linear and Exp choices by mouse.
#[allow(clippy::too_many_arguments)]
pub fn curve_timeline_editor_with_segments(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
    segments: &[CurveSegment],
    override_value: Option<f32>,
    selected: &mut Option<u64>,
    viewport: &mut TimelineViewport,
    timeline: CurveTimeline,
    evaluate: impl Fn(f64) -> f32,
) -> CurveResponse {
    curve_editor_inner(
        ui,
        id,
        points,
        segments,
        override_value,
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

/// Responsive editor actions using the shared toolbar's styling and layout.
pub fn curve_toolbar<R>(
    ui: &mut Ui,
    compact: bool,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.scope(|ui| {
        if compact {
            // Preserve native hit heights while fitting the editor's complete
            // primary action row above its canvas on compact workspaces.
            ui.spacing_mut().button_padding.x = crate::tokens::SPACE_SM;
        }
        crate::toolbar(ui, contents).inner
    })
}

/// The same segment adjustment is available without dragging a small canvas
/// handle. A zero slope is linear; negative and positive values bend toward
/// logarithmic and exponential shapes respectively.
pub fn curve_segment_controls(ui: &mut Ui, curve: &mut f32) -> Response {
    let mut changed = false;
    let row = ui.horizontal(|ui| {
        crate::text::label(ui, "Segment");
        for (label, value) in [("Log", -1.0_f32), ("Linear", 0.0), ("Exp", 1.0)] {
            if crate::selectable_value(ui, curve, value, label)
                .on_hover_text("Shape the segment after the selected point")
                .clicked()
            {
                changed = true;
            }
        }
        ui.spacing_mut().slider_width = 150.0;
        changed |= ui
            .add(
                crate::slider(curve, -1.0..=1.0)
                    // The adjacent Log/Linear/Exp choices already explain
                    // the endpoints; keep this inline control on one row.
                    .scale_labels(false)
                    .text("Slope")
                    .step_by(0.01)
                    .fixed_decimals(2),
            )
            .on_hover_text("−1 Log · 0 Linear · +1 Exp; affects only this pair of points")
            .changed();
    });
    let mut response = row.response;
    if changed {
        response.mark_changed();
    }
    response
}

/// Read-only manual value over the retained curve, using the same override hue
/// as the automation override badge. It creates no pointer target.
pub fn paint_curve_override(painter: &egui::Painter, plot: Rect, value: f32) {
    let colors = theme::palette(painter.ctx());
    let y = plot.bottom() - value.clamp(0.0, 1.0) * plot.height();
    let painter = painter.with_clip_rect(plot);
    painter.hline(
        plot.x_range(),
        y,
        Stroke::new(2.0_f32, colors.automation_override),
    );
}

/// Endpoints apply on both sides of the editable points. The concise caption
/// stays outside the canvas, so even a tightly fitted view explains the holds.
pub fn curve_endpoint_summary(ui: &mut Ui, first: f32, last: f32) -> Response {
    crate::text::note(ui, format!("← {:.0}%  ·  {:.0}% →", first * 100.0, last * 100.0))
        .on_hover_text("The first value is held before the first point, including before playback starts. The last value is held indefinitely after the last point.")
}

#[allow(clippy::too_many_arguments)]
fn curve_editor_inner(
    ui: &mut Ui,
    id: Id,
    points: &[CurvePoint],
    segments: &[CurveSegment],
    override_value: Option<f32>,
    selected: &mut Option<u64>,
    view: CurveView,
    timeline: Option<(&mut TimelineViewport, CurveTimeline)>,
    evaluate: impl Fn(f64) -> f32,
) -> CurveResponse {
    let mut start = view.start.max(0.0);
    let mut end = view.end.max(start + 0.01);
    let musical = timeline.is_some();
    let top_inset = crate::tokens::SPACE_MD
        + if musical {
            crate::tokens::TIMELINE_RULER_HEIGHT
        } else {
            0.0
        };
    let bottom_inset = if musical {
        crate::tokens::SPACE_MD
    } else {
        crate::tokens::SPACE_XL
    };
    // A bounded utility can surrender canvas space to its inspector rows.
    // Keep the ruler, labels and a nonzero plot rather than imposing 120 px.
    let minimum_height = top_inset + bottom_inset + crate::tokens::SPACE_LG;
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(
            ui.available_width().max(180.0),
            view.height.max(minimum_height),
        ),
        Sense::click(),
    );
    let keyboard_active = timeline
        .as_ref()
        .is_some_and(|(_, timeline)| timeline.keyboard_active);
    let selection_id = id.with("selection");
    let mut selection = ui.ctx().data_mut(|data| {
        data.get_temp::<std::collections::HashSet<u64>>(selection_id)
            .unwrap_or_default()
    });
    let valid_ids: std::collections::HashSet<_> = points.iter().map(|point| point.id).collect();
    selection.retain(|id| valid_ids.contains(id));
    if let Some(current) = *selected {
        if !selection.contains(&current) {
            selection.clear();
            selection.insert(current);
        }
    } else {
        selection.clear();
    }
    let mut reveal = None;
    if (keyboard_active || response.has_focus())
        && let Some((direction, extend)) = crate::selection_navigation_input(ui, Some(response.id))
    {
        let items: Vec<_> = points
            .iter()
            .map(|point| crate::SelectionItem {
                id: point.id,
                start: point.x,
                end: point.x,
                row: f64::from(1.0 - point.value),
            })
            .collect();
        if let Some(next) = crate::adjacent_selection_item(&items, *selected, direction) {
            if !extend {
                selection.clear();
            }
            selection.insert(next);
            *selected = Some(next);
            reveal = points
                .iter()
                .find(|point| point.id == next)
                .map(|point| point.x);
        }
    }
    let plot = Rect::from_min_max(
        rect.min
            + Vec2::new(
                crate::tokens::CONTROL_HEIGHT + crate::tokens::SPACE_MD,
                top_inset,
            ),
        rect.max - Vec2::new(crate::tokens::SPACE_MD, bottom_inset),
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
        let pane = TimelinePane::new(id.with("timeline"), plot)
            .horizontal_only()
            .keyboard_focus(response.id);
        navigation = if timeline.keyboard_active {
            pane.keyboard_active()
        } else {
            pane
        }
        .navigate(ui, viewport, bounds);
        if let Some(beat) = reveal {
            let visible = f64::from(plot.width()) / f64::from(viewport.pixels_per_beat);
            if beat < viewport.start_beat {
                viewport.start_beat = beat.max(0.0);
            } else if beat > viewport.start_beat + visible {
                viewport.start_beat = (beat - visible).max(0.0);
            }
        }
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
    // Keep endpoint labels at every height, and add intermediate percentages
    // only when their measured rows retain a clear gap at the active scale.
    let tick_font = FontId::monospace(crate::tokens::FONT_CANVAS);
    let tick_spacing = ui.fonts_mut(|fonts| fonts.row_height(&tick_font)) + crate::tokens::SPACE_XS;
    let tick_intervals = [4, 2, 1]
        .into_iter()
        .find(|intervals| plot.height() / *intervals as f32 >= tick_spacing)
        .unwrap_or(1);
    for index in 0..=tick_intervals {
        let value = index as f32 / tick_intervals as f32;
        let y = plot.bottom() - value * plot.height();
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0_f32, theme::palette(ui.ctx()).line),
        );
        painter.text(
            Pos2::new(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{}%", index * 100 / tick_intervals),
            tick_font.clone(),
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
        if override_value.is_some() {
            theme::palette(ui.ctx()).muted
        } else {
            theme::palette(ui.ctx()).automation
        },
        &evaluate,
    );
    if let Some(value) = override_value {
        paint_curve_override(&painter, plot, value);
    }
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
    let mut segment_edit = None;
    let mut segment_handles = Vec::new();
    // Pair iteration stays linear even for large lanes. Callers supply segment
    // entries in point order; absent/legacy entries simply omit their handles.
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
        let midpoint = a.x + (b.x - a.x) * 0.5;
        if midpoint < start || midpoint > end {
            continue;
        }
        let a_screen = to_screen(a);
        let b_screen = to_screen(b);
        // Dense breakpoints still have precise footer controls; do not put a
        // second hit target on top of either endpoint.
        if (b_screen.x - a_screen.x).abs() < 40.0 {
            continue;
        }
        let position = to_screen(CurvePoint {
            id: a.id,
            x: midpoint,
            value: evaluate(midpoint),
        });
        let handle = Rect::from_center_size(position, Vec2::splat(18.0));
        let handle_response =
            ui.interact(handle, id.with(("segment", a.id)), Sense::click_and_drag());
        handle_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), "Segment slope")
        });
        point_hit |=
            handle_response.hovered() || handle_response.dragged() || handle_response.clicked();
        if editing && (handle_response.clicked() || handle_response.drag_started()) {
            selection.clear();
            selection.insert(a.id);
            *selected = Some(a.id);
            response.request_focus();
        }
        if editing && handle_response.drag_started() {
            let difference = b.value - a.value;
            let travel = if difference.abs() > 0.001 {
                0.375 * difference.signum() * difference.abs().max(0.05) * plot.height()
            } else {
                120.0
            };
            ui.ctx().data_mut(|data| {
                data.insert_temp(id.with("segment_drag"), (segment.curve, travel))
            });
        }
        if editing && handle_response.dragged() {
            let (origin, travel) = ui
                .ctx()
                .data(|data| data.get_temp::<(f32, f32)>(id.with("segment_drag")))
                .unwrap_or((segment.curve, 120.0));
            let delta = handle_response.total_drag_delta().unwrap_or_default();
            segment_edit = Some((a.id, (origin + delta.y / travel).clamp(-1.0, 1.0)));
        }
        if !navigation.navigating {
            handle_response.context_menu(|ui| {
                for (label, curve) in [("Log", -1.0), ("Linear", 0.0), ("Exp", 1.0)] {
                    if crate::button(ui, label).clicked() {
                        selection.clear();
                        selection.insert(a.id);
                        *selected = Some(a.id);
                        segment_edit = Some((a.id, curve));
                        ui.close();
                    }
                }
            });
        }
        let colors = theme::palette(ui.ctx());
        let radius = if *selected == Some(a.id) || handle_response.hovered() {
            5.0
        } else {
            4.0
        };
        painter.add(egui::Shape::convex_polygon(
            vec![
                position + Vec2::new(0.0, -radius),
                position + Vec2::new(radius, 0.0),
                position + Vec2::new(0.0, radius),
                position + Vec2::new(-radius, 0.0),
            ],
            colors.bg,
            Stroke::new(1.5_f32, colors.automation),
        ));
        handle_response
            .on_hover_text("Segment slope · drag to bend · right-click for Log, Linear or Exp");
        segment_handles.push(CurveSegmentHandle {
            start: a.id,
            end: b.id,
            rect: handle,
            curve: segment.curve,
        });
    }
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
            if ui.input(|input| input.modifiers.shift) {
                if point_response.clicked() && !selection.insert(point.id) {
                    selection.remove(&point.id);
                } else {
                    selection.insert(point.id);
                }
                *selected = selection
                    .contains(&point.id)
                    .then_some(point.id)
                    .or_else(|| selection.iter().copied().min());
            } else {
                selection.clear();
                selection.insert(point.id);
                *selected = Some(point.id);
            }
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
                let selected_point = selection.contains(&point.id);
                if crate::selection_membership_action(ui, selected_point) {
                    if selected_point {
                        selection.remove(&point.id);
                        if *selected == Some(point.id) {
                            *selected = selection.iter().copied().min();
                        }
                    } else {
                        selection.insert(point.id);
                        *selected = Some(point.id);
                    }
                }
                if crate::button(ui, "Delete point").clicked() {
                    edit = Some(CurveEdit::Delete { id: point.id });
                    ui.close();
                }
            });
        }
        painter.circle_filled(
            position,
            if selection.contains(&point.id) {
                5.5
            } else {
                4.0
            },
            theme::palette(ui.ctx()).automation,
        );
        if selection.contains(&point.id) {
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
    let mut delete_selection = false;
    if (keyboard_active || response.has_focus())
        && editing
        && (!ui.ctx().wants_keyboard_input() || response.has_focus())
        && ui.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        })
        && let Some(id) = *selected
    {
        edit = Some(CurveEdit::Delete { id });
        delete_selection = true;
    }
    if edit.is_some() || segment_edit.is_some() {
        response.mark_changed();
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(selection_id, selection.clone()));
    let mut selection: Vec<_> = selection.into_iter().collect();
    selection.sort_unstable();
    CurveResponse {
        response,
        plot_rect: plot,
        edit,
        navigation,
        viewport,
        selection,
        delete_selection,
        segment_edit,
        segment_handles,
    }
}
