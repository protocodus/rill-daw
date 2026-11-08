//! Shared endpoint-preserving bend gestures for detailed and inline automation.
use crate::{CurvePoint, CurveSegmentHandle, theme, tokens};
use egui::{Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

#[derive(Default)]
pub(crate) struct SegmentResponse {
    pub edit: Option<(u64, f32)>,
    pub handle: Option<CurveSegmentHandle>,
    pub interacted: bool,
    pub add: Option<Pos2>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn segment_bend_at(
    ui: &Ui,
    id: Id,
    a: CurvePoint,
    b: CurvePoint,
    curve: f32,
    plot: Rect,
    x_at: impl Fn(f64) -> f32,
    evaluate: impl Fn(f64) -> f32,
    editable: bool,
) -> SegmentResponse {
    let mut result = SegmentResponse::default();
    let left = x_at(a.x);
    let right = x_at(b.x);
    if !left.is_finite() || !right.is_finite() || right - left < 2.0 * tokens::HIT_TARGET {
        return result;
    }
    let visible = Rect::from_x_y_ranges(left..=right, plot.y_range()).intersect(plot);
    if !visible.is_positive() || !ui.is_rect_visible(visible) {
        return result;
    }
    let value_at = |x: f32| {
        evaluate(a.x + f64::from((x - left) / (right - left)) * (b.x - a.x)).clamp(0.0, 1.0)
    };
    let y_at = |x| plot.bottom() - value_at(x) * plot.height();
    let interactive = editable && ui.is_enabled();
    let enabled = interactive && !egui::Popup::is_any_open(ui.ctx());
    let mut bend = |response: &Response| {
        if !enabled {
            return;
        }
        result.interacted |= response.drag_started() || response.dragged() || response.clicked();
        if response.drag_started() {
            let difference = b.value - a.value;
            let travel = if difference.abs() > 0.001 {
                0.375 * difference.signum() * difference.abs().max(0.05) * plot.height()
            } else {
                120.0
            };
            ui.ctx().data_mut(|data| {
                data.insert_temp(id.with(("bend_origin", a.id)), (curve, travel));
            });
        }
        if response.dragged() {
            let (origin, travel) = ui
                .ctx()
                .data(|data| data.get_temp::<(f32, f32)>(id.with(("bend_origin", a.id))))
                .unwrap_or((curve, 120.0));
            let delta = response.total_drag_delta().unwrap_or_default().y;
            let value = (origin + delta / travel).clamp(-1.0, 1.0);
            if delta != 0.0 && value != curve {
                result.edit = Some((a.id, value));
            }
        }
    };
    // Follow the actual rendered curve with small native drag targets. A single
    // bounding box would steal clicks in the empty area above/below the curve.
    // Click-and-drag waits for movement before bending. A plain line click is
    // returned to the owner as the same add-point operation as empty canvas.
    let bands = ((visible.width() / 12.0).ceil() as usize).clamp(1, 256);
    for band in 0..bands {
        let x0 = visible.left() + visible.width() * band as f32 / bands as f32;
        let x1 = visible.left() + visible.width() * (band + 1) as f32 / bands as f32;
        let y0 = y_at(x0);
        let y1 = y_at(x1);
        let target = Rect::from_min_max(
            Pos2::new(x0, y0.min(y1) - 6.0),
            Pos2::new(x1, y0.max(y1) + 6.0),
        )
        .intersect(plot);
        if !target.is_positive() {
            continue;
        }
        let response = ui.interact(
            target,
            id.with(("bend_line", a.id, band)),
            if interactive {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
        );
        bend(&response);
        if enabled && response.clicked() {
            result.add = response.interact_pointer_pos();
        }
        if enabled {
            crate::hover_text(
                response.on_hover_cursor(egui::CursorIcon::ResizeVertical),
                "Drag up or down to bend this segment · Click to add a point",
            );
        }
    }
    let midpoint = (left + right) * 0.5;
    if midpoint >= plot.left() && midpoint <= plot.right() {
        let position = Pos2::new(midpoint, y_at(midpoint));
        let handle = Rect::from_center_size(position, Vec2::splat(tokens::HIT_TARGET));
        let response = ui.interact(
            handle,
            id.with(("segment", a.id)),
            if interactive {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Slider, enabled, "Segment slope")
        });
        bend(&response);
        // The accessible handle extends beyond the small painted diamond.
        // A click anywhere in that target follows the line's add-point action;
        // only actual movement bends the segment.
        if enabled && response.clicked() {
            result.add = response.interact_pointer_pos();
        }
        if interactive {
            response.context_menu(|ui| {
                for (label, shape) in [("Log", -1.0), ("Linear", 0.0), ("Exp", 1.0)] {
                    if crate::button(ui, label).clicked() {
                        result.interacted = true;
                        result.edit = Some((a.id, shape));
                        ui.close();
                    }
                }
            });
        }
        let colors = theme::palette(ui.ctx());
        let radius = if response.hovered() || response.dragged() {
            5.0
        } else {
            4.0
        };
        ui.painter_at(plot).add(egui::Shape::convex_polygon(
            vec![
                position + Vec2::new(0.0, -radius),
                position + Vec2::new(radius, 0.0),
                position + Vec2::new(0.0, radius),
                position + Vec2::new(-radius, 0.0),
            ],
            colors.bg,
            Stroke::new(1.5, colors.automation),
        ));
        crate::hover_text(
            response,
            "Segment slope · Drag the line or diamond to bend · Right-click for Log, Linear or Exp",
        );
        result.handle = Some(CurveSegmentHandle {
            start: a.id,
            end: b.id,
            rect: handle,
            curve,
        });
    }
    result
}
