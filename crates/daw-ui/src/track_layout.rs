//! Shared arranger sizing and hierarchy affordances. The caller owns view state.
use crate::{Icon, theme, tokens};
use egui::{Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

#[derive(Clone, Copy)]
pub enum ResizeAxis {
    Horizontal,
    Vertical,
}

/// Preorder lane geometry supplied by the arranger. Rectangles include complete
/// unclipped rows, so a group can continue above or below the scroll viewport.
#[derive(Clone, Copy)]
pub struct FolderLane {
    pub id: u64,
    pub depth: usize,
    pub folder: bool,
    pub color: egui::Color32,
    pub rect: Rect,
}

/// Inner groups close before outer groups. A small footer per closed group
/// keeps their bottom borders apart without changing the shared beat mapping.
pub fn folder_closures(rows: &[FolderLane]) -> Vec<Vec<usize>> {
    let mut result = vec![Vec::new(); rows.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if row.folder {
            stack.push(index);
        }
        while let Some(&folder) = stack.last() {
            if rows
                .get(index + 1)
                .is_some_and(|next| rows[folder].depth < next.depth)
            {
                break;
            }
            result[index].push(stack.pop().unwrap());
        }
    }
    result
}

/// Keep recursive borders separate while retaining useful header controls and
/// name space in narrow layouts. Very deep nesting compresses the inset.
pub fn folder_lane_padding(rows: &[FolderLane], label_width: f32) -> (f32, f32) {
    let levels = rows
        .iter()
        .filter(|row| row.folder)
        .map(|row| row.depth + 1)
        .max()
        .unwrap_or(0)
        .min(32);
    if levels == 0 {
        return (0.0, 0.0);
    }
    let padding = (levels as f32 * 6.0).min((label_width - 184.0).clamp(24.0, 96.0));
    (padding / levels as f32, padding)
}

pub fn folder_footer_spacing(step: f32) -> f32 {
    if step > 0.0 { step.max(3.0) } else { 0.0 }
}

pub fn folder_content_inset(depth: usize, folder: bool, step: f32) -> f32 {
    let layers = (depth + usize::from(folder)).min(32);
    if layers == 0 {
        0.0
    } else {
        (layers - 1) as f32 * step + 6.0
    }
}

#[derive(Clone, Debug)]
pub struct FolderContainerPaint {
    pub folder: u64,
    pub depth: usize,
    pub rect: Rect,
    pub visible: Rect,
    pub color: egui::Color32,
    /// Top, left, right and bottom strips, individually clipped. Missing caps
    /// remain missing when the real group edge has scrolled out of view.
    pub borders: [Option<Rect>; 4],
}

/// Paint recursive colored containers after their lane contents. The caller
/// reserves the same maximum inset at the right of EVERY lane, so all depths
/// retain one timeline origin and no clip extends through an inner border.
pub fn paint_folder_containers(
    painter: &egui::Painter,
    rows: &[FolderLane],
    viewport: Rect,
    step: f32,
) -> Vec<FolderContainerPaint> {
    if step <= 0.0 || !step.is_finite() {
        return Vec::new();
    }
    let clip = viewport.intersect(painter.clip_rect());
    let footer = folder_footer_spacing(step);
    let closures = folder_closures(rows);
    let mut result = Vec::new();
    for (end, closed) in closures.iter().enumerate() {
        for (order, &start) in closed.iter().enumerate() {
            let row = rows[start];
            let inset = row.depth.min(31) as f32 * step + 2.0;
            let rect = Rect::from_min_max(
                Pos2::new(row.rect.left() + inset, row.rect.top()),
                Pos2::new(
                    row.rect.right() - inset,
                    rows[end].rect.bottom() + footer * (order + 1) as f32 - 2.0,
                ),
            );
            let visible = rect.intersect(clip);
            if !visible.is_positive() {
                continue;
            }
            let width = 1.0_f32.min(rect.width() * 0.5).min(rect.height() * 0.5);
            let strips = [
                Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + width)),
                Rect::from_min_max(
                    Pos2::new(rect.left(), rect.top() + width),
                    Pos2::new(rect.left() + width, rect.bottom() - width),
                ),
                Rect::from_min_max(
                    Pos2::new(rect.right() - width, rect.top() + width),
                    Pos2::new(rect.right(), rect.bottom() - width),
                ),
                Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - width), rect.max),
            ];
            let borders = strips.map(|strip| {
                let visible = strip.intersect(clip);
                visible.is_positive().then_some(visible)
            });
            result.push(FolderContainerPaint {
                folder: row.id,
                depth: row.depth,
                rect,
                visible,
                color: row.color,
                borders,
            });
        }
    }
    result.sort_by_key(|group| group.depth);
    for group in &result {
        for rect in group.borders.iter().flatten() {
            painter.rect_filled(*rect, 0.0, group.color);
        }
    }
    result
}

/// A visible, focus-free drag gutter. One captured starting value prevents
/// moving layout geometry from changing the scale midway through a gesture.
pub fn resize_value_at(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    axis: ResizeAxis,
    hint: &str,
) -> Response {
    let mut response = ui.interact(rect, id, Sense::drag()).on_hover_text(hint);
    let start_id = id.with("start");
    if response.drag_started_by(egui::PointerButton::Primary)
        && !ui.input(|input| input.modifiers.alt || input.modifiers.ctrl)
    {
        ui.ctx().data_mut(|data| data.insert_temp(start_id, *value));
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && ui.is_enabled()
        && let Some(start) = ui.ctx().data(|data| data.get_temp::<f32>(start_id))
    {
        let delta = response.total_drag_delta().unwrap_or_default();
        let delta = match axis {
            ResizeAxis::Horizontal => delta.x,
            ResizeAxis::Vertical => delta.y,
        };
        let next = (start + delta).clamp(*range.start(), *range.end());
        if next != *value {
            *value = next;
            response.mark_changed();
        }
    }
    if response.drag_stopped() {
        ui.ctx().data_mut(|data| data.remove::<f32>(start_id));
    }
    let colors = theme::palette(ui.ctx());
    let active = response.hovered() || response.dragged();
    if active {
        ui.ctx().set_cursor_icon(match axis {
            ResizeAxis::Horizontal => egui::CursorIcon::ResizeHorizontal,
            ResizeAxis::Vertical => egui::CursorIcon::ResizeVertical,
        });
    }
    let color = if active { colors.accent } else { colors.line };
    let center = rect.center();
    let (a, b) = match axis {
        ResizeAxis::Horizontal => (
            Pos2::new(center.x, rect.top()),
            Pos2::new(center.x, rect.bottom()),
        ),
        ResizeAxis::Vertical => (
            Pos2::new(rect.left(), center.y),
            Pos2::new(rect.right(), center.y),
        ),
    };
    ui.painter()
        .line_segment([a, b], Stroke::new(if active { 2.0 } else { 1.0 }, color));
    response
}

/// Related view dimensions stay together in the on-demand View menu.
pub fn track_dimensions(ui: &mut Ui, header: &mut f32, row_height: &mut f32, max_header: f32) {
    ui.separator();
    ui.label("Track layout");
    egui::Grid::new("track_dimensions")
        .num_columns(2)
        .show(ui, |ui| {
            let label = ui.label("Label width");
            ui.add(
                crate::number(header)
                    .range(224.0..=max_header)
                    .clamp_existing_to_range(false)
                    .speed(1.0)
                    .fixed_decimals(0)
                    .suffix(" px"),
            )
            .labelled_by(label.id);
            ui.end_row();
            let label = ui.label("Row height");
            ui.add(
                crate::number(row_height)
                    .range(56.0..=1024.0)
                    .clamp_existing_to_range(false)
                    .speed(1.0)
                    .fixed_decimals(0)
                    .suffix(" px"),
            )
            .labelled_by(label.id);
            ui.end_row();
        });
}

/// A bus retains its user color while clearly differing from a source channel.
pub fn bus_header_fill(ctx: &egui::Context, tint: egui::Color32) -> egui::Color32 {
    theme::palette(ctx).panel.lerp_to_gamma(tint, 0.12)
}

/// The same compact bus identity appears in arranger and mixer headers. The
/// caller reserves its bounds so it cannot cover the channel's stored name.
pub fn paint_bus_badge(painter: &egui::Painter, rect: Rect, tint: egui::Color32) {
    if !rect.is_positive() {
        return;
    }
    let colors = theme::palette(painter.ctx());
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 3.0, colors.raised.lerp_to_gamma(tint, 0.16));
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, tint.gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    crate::text::paint_single_line(
        &painter,
        rect.shrink2(Vec2::new(4.0, 1.0)),
        egui::FontId::proportional(tokens::FONT_CANVAS),
        colors.text,
        "BUS",
    );
}

/// Paint aligned hierarchy guides and a vector folder/disclosure identity.
/// Returns the name bounds for diagnostics and callers' context actions.
pub fn track_hierarchy(
    ui: &Ui,
    rect: Rect,
    depth: usize,
    name: &str,
    icon: Icon,
    collapsed: bool,
    tint: egui::Color32,
) -> Rect {
    let colors = theme::palette(ui.ctx());
    let painter = ui.painter_at(rect);
    let indent = tokens::SPACE_LG;
    // Deep nesting compresses the guides before sacrificing the name. The
    // caller retains the actual depth for tooltips, grouping and diagnostics.
    let depth = depth
        .min(32)
        .min(((rect.width() - 116.0).max(0.0) / indent) as usize);
    for level in 0..depth {
        let x = rect.left() + tokens::SPACE_XS + level as f32 * indent;
        if x < rect.right() {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, colors.line),
            );
        }
    }
    let left = rect.left() + depth as f32 * indent;
    let top = rect.top() + tokens::SPACE_XS;
    let text_left = left + 36.0;
    let disclosure = Rect::from_min_size(Pos2::new(left, top + 3.0), Vec2::splat(14.0));
    if icon == Icon::Folder {
        (if collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        })
        .paint(&painter, disclosure, colors.muted);
    }
    icon.paint(&painter, disclosure.translate(Vec2::new(16.0, 0.0)), tint);
    let name_rect = Rect::from_min_max(
        Pos2::new(text_left, top),
        Pos2::new(rect.right(), (top + 20.0).min(rect.bottom())),
    );
    crate::text::paint_single_line(
        &painter,
        name_rect,
        egui::FontId::proportional(tokens::FONT_TRACK),
        colors.text,
        name,
    );
    name_rect
}
