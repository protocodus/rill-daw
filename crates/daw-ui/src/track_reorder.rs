//! Track/channel ordering gestures shared by the arranger and mixer.
//! The caller validates ownership/hierarchy and applies one edit after release.
use crate::{Icon, theme, tokens};
use egui::{CursorIcon, DragAndDrop, Id, PointerButton, Pos2, Rect, Response, Sense, Ui, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackDragPayload {
    pub track: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackReorderAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackDropPlacement {
    Before,
    After,
    IntoFolder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackDropIntent {
    pub source: u64,
    pub target: u64,
    pub placement: TrackDropPlacement,
}

#[derive(Clone, Copy, Debug)]
pub struct TrackDropPreview {
    pub intent: TrackDropIntent,
    /// The full insertion line or folder outline, before viewport clipping.
    pub indicator: Rect,
    pub target_rect: Rect,
    pub axis: TrackReorderAxis,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TrackDropResponse {
    pub preview: Option<TrackDropPreview>,
    pub dropped: Option<TrackDropIntent>,
}

fn gesture_enabled(ui: &Ui) -> bool {
    ui.is_enabled()
        && !egui::Popup::is_any_open(ui.ctx())
        && ui.input(|input| {
            input.focused
                && !input.modifiers.alt
                && !input.modifiers.ctrl
                && !input.modifiers.command
        })
}

/// Add dragging to an existing title/name response without a second hit target.
/// Use the returned response for the title's ordinary click/context actions.
pub fn track_reorder_source(ui: &Ui, response: &Response, track: u64) -> Response {
    let response = response.interact(Sense::click_and_drag());
    if gesture_enabled(ui) && response.drag_started_by(PointerButton::Primary) {
        DragAndDrop::set_payload(ui.ctx(), TrackDragPayload { track });
    }
    if response.dragged_by(PointerButton::Primary)
        && DragAndDrop::payload::<TrackDragPayload>(ui.ctx())
            .is_some_and(|payload| payload.track == track)
    {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    response.on_hover_text("Drag to reorder")
}

/// A dedicated, visible drag grip for compact canvas headers. It does not
/// advance the caller's layout or share the color/disclosure button's hit area.
pub fn track_reorder_grip_at(ui: &Ui, rect: Rect, id: Id, track: u64) -> Response {
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let color = if response.hovered() || response.dragged() {
            theme::palette(ui.ctx()).text
        } else {
            theme::palette(ui.ctx()).muted
        };
        Icon::Grip.paint(ui.painter(), rect.shrink(2.0), color);
    }
    let response = track_reorder_source(ui, &response, track);
    response
        .on_hover_cursor(CursorIcon::Grab)
        .on_hover_text("Reorder track · drag to an insertion line")
}

/// Inspect a broad row/strip as a drop destination without intercepting any
/// controls inside it. Vertical folder rows expose an explicit middle-third
/// "Into folder" target; all other rows split into before/after halves.
pub fn track_reorder_target(
    ui: &Ui,
    rect: Rect,
    target: u64,
    axis: TrackReorderAxis,
    folder: bool,
    valid: impl FnOnce(TrackDropIntent) -> bool,
) -> TrackDropResponse {
    track_reorder_target_with_after_edge(ui, rect, target, axis, folder, None, valid)
}

/// A folder header still owns its three hover zones, while dropping After an
/// expanded folder inserts after all descendants. Supply that subtree's bottom
/// edge (or right edge for a horizontal owner) without expanding the hit area.
pub fn track_reorder_target_with_after_edge(
    ui: &Ui,
    rect: Rect,
    target: u64,
    axis: TrackReorderAxis,
    folder: bool,
    after_edge: Option<f32>,
    valid: impl FnOnce(TrackDropIntent) -> bool,
) -> TrackDropResponse {
    if !gesture_enabled(ui) {
        return TrackDropResponse::default();
    }
    let Some(payload) = DragAndDrop::payload::<TrackDragPayload>(ui.ctx()) else {
        return TrackDropResponse::default();
    };
    let visible = rect.intersect(ui.clip_rect());
    let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) else {
        return TrackDropResponse::default();
    };
    if !visible.is_positive() || !visible.contains(pointer) || !ui.rect_contains_pointer(visible) {
        return TrackDropResponse::default();
    }
    let fraction = match axis {
        TrackReorderAxis::Horizontal => (pointer.x - rect.left()) / rect.width(),
        TrackReorderAxis::Vertical => (pointer.y - rect.top()) / rect.height(),
    };
    let placement = if folder && axis == TrackReorderAxis::Vertical {
        if fraction < 1.0 / 3.0 {
            TrackDropPlacement::Before
        } else if fraction > 2.0 / 3.0 {
            TrackDropPlacement::After
        } else {
            TrackDropPlacement::IntoFolder
        }
    } else if fraction < 0.5 {
        TrackDropPlacement::Before
    } else {
        TrackDropPlacement::After
    };
    let intent = TrackDropIntent {
        source: payload.track,
        target,
        placement,
    };
    if intent.source == intent.target || !valid(intent) {
        ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
        return TrackDropResponse::default();
    }
    let mut paint_rect = rect;
    if placement == TrackDropPlacement::After
        && let Some(edge) = after_edge.filter(|edge| edge.is_finite())
    {
        match axis {
            TrackReorderAxis::Horizontal => paint_rect.max.x = edge.max(rect.left()),
            TrackReorderAxis::Vertical => paint_rect.max.y = edge.max(rect.top()),
        }
    }
    let indicator = paint_preview(ui, paint_rect, axis, placement);
    let preview = TrackDropPreview {
        intent,
        indicator,
        target_rect: rect,
        axis,
    };
    track_reorder_preview_label(
        ui,
        &preview,
        match placement {
            TrackDropPlacement::Before => "Before",
            TrackDropPlacement::After => "After",
            TrackDropPlacement::IntoFolder => "Into folder",
        },
    );
    let dropped = ui.input(|input| input.pointer.button_released(PointerButton::Primary));
    if dropped {
        DragAndDrop::take_payload::<TrackDragPayload>(ui.ctx());
    }
    TrackDropResponse {
        preview: Some(preview),
        dropped: dropped.then_some(intent),
    }
}

fn paint_preview(
    ui: &Ui,
    rect: Rect,
    axis: TrackReorderAxis,
    placement: TrackDropPlacement,
) -> Rect {
    let colors = theme::palette(ui.ctx());
    let indicator = match placement {
        TrackDropPlacement::IntoFolder => rect.shrink(2.0),
        TrackDropPlacement::Before | TrackDropPlacement::After => {
            let before = placement == TrackDropPlacement::Before;
            match axis {
                TrackReorderAxis::Horizontal => Rect::from_center_size(
                    Pos2::new(
                        if before { rect.left() } else { rect.right() },
                        rect.center().y,
                    ),
                    Vec2::new(3.0, rect.height()),
                ),
                TrackReorderAxis::Vertical => Rect::from_center_size(
                    Pos2::new(
                        rect.center().x,
                        if before { rect.top() } else { rect.bottom() },
                    ),
                    Vec2::new(rect.width(), 3.0),
                ),
            }
        }
    };
    if placement == TrackDropPlacement::IntoFolder {
        ui.painter().rect_filled(
            indicator,
            tokens::RADIUS,
            colors.accent.gamma_multiply(0.12),
        );
        ui.painter().rect_stroke(
            indicator,
            tokens::RADIUS,
            egui::Stroke::new(2.0, colors.accent),
            egui::StrokeKind::Inside,
        );
    } else {
        // An expanded folder may end beyond the viewport. Keep a visible edge
        // cue there, while the returned geometry retains the actual boundary.
        let clip = ui.clip_rect();
        let visible_indicator = match axis {
            TrackReorderAxis::Vertical => {
                let y = indicator.center().y.clamp(
                    clip.top() + 1.5,
                    (clip.bottom() - 1.5).max(clip.top() + 1.5),
                );
                indicator.translate(Vec2::new(0.0, y - indicator.center().y))
            }
            TrackReorderAxis::Horizontal => {
                let x = indicator.center().x.clamp(
                    clip.left() + 1.5,
                    (clip.right() - 1.5).max(clip.left() + 1.5),
                );
                indicator.translate(Vec2::new(x - indicator.center().x, 0.0))
            }
        };
        ui.painter()
            .rect_filled(visible_indicator, 1.0, colors.accent);
    }
    indicator
}

/// Replace the short default cue with destination details such as its name and
/// parent folder. The shared surface paints over the default at the same anchor.
pub fn track_reorder_preview_label(ui: &Ui, preview: &TrackDropPreview, label: &str) {
    let colors = theme::palette(ui.ctx());
    let rect = preview.target_rect;
    let axis = preview.axis;
    let placement = preview.intent.placement;
    // Paint-only feedback stays within the destination instead of creating a
    // popup/hit target that could steal the pointer before release.
    let bounds = ui.clip_rect().shrink(4.0);
    let width = bounds.width().clamp(16.0, 360.0);
    let mut job = egui::text::LayoutJob::simple(
        label.into(),
        egui::FontId::proportional(tokens::FONT_CANVAS),
        colors.text,
        width - 12.0,
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let size = galley.size() + Vec2::new(12.0, 6.0);
    let after = placement == TrackDropPlacement::After;
    let mut min = match axis {
        TrackReorderAxis::Horizontal if after => {
            Pos2::new(rect.right() - size.x - 5.0, rect.top() + 5.0)
        }
        TrackReorderAxis::Vertical if after => {
            Pos2::new(rect.left() + 5.0, rect.bottom() - size.y - 5.0)
        }
        _ => rect.min + Vec2::splat(5.0),
    };
    min.x = min
        .x
        .clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left()));
    min.y = min
        .y
        .clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
    let label_rect = Rect::from_min_size(min, size);
    let painter = ui.painter().with_clip_rect(bounds);
    painter.rect_filled(label_rect, tokens::RADIUS, colors.raised);
    painter.galley(label_rect.min + Vec2::new(6.0, 3.0), galley, colors.text);
}

/// Native content-scroll delta while dragging near an edge of a visible pane.
/// Pass it to `Ui::scroll_with_delta`; a custom viewport should subtract it
/// from its pixel offset. No movement occurs without an active primary drag.
pub fn track_reorder_scroll_delta(ui: &Ui, viewport: Rect, axis: TrackReorderAxis) -> Vec2 {
    if !gesture_enabled(ui) || !DragAndDrop::has_payload_of_type::<TrackDragPayload>(ui.ctx()) {
        return Vec2::ZERO;
    }
    let viewport = viewport.intersect(ui.clip_rect());
    let (pointer, down, dt) = ui.input(|input| {
        (
            input.pointer.interact_pos(),
            input.pointer.button_down(PointerButton::Primary),
            input.stable_dt.clamp(1.0 / 240.0, 0.05),
        )
    });
    let Some(pointer) = pointer.filter(|point| down && viewport.contains(*point)) else {
        return Vec2::ZERO;
    };
    let (position, start, end) = match axis {
        TrackReorderAxis::Horizontal => (pointer.x, viewport.left(), viewport.right()),
        TrackReorderAxis::Vertical => (pointer.y, viewport.top(), viewport.bottom()),
    };
    let margin = ((end - start) * 0.25).clamp(1.0, 32.0);
    let strength = if position < start + margin {
        (start + margin - position) / margin
    } else if position > end - margin {
        -(position - (end - margin)) / margin
    } else {
        0.0
    };
    if strength != 0.0 {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    let amount = strength * 600.0 * dt;
    match axis {
        TrackReorderAxis::Horizontal => Vec2::new(amount, 0.0),
        TrackReorderAxis::Vertical => Vec2::new(0.0, amount),
    }
}
