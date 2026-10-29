//! Broad, low-contrast surface tint. These helpers paint behind existing
//! controls; they never allocate an input target or own interaction state.
use crate::{theme, tokens};
use egui::{Color32, Frame, InnerResponse, Painter, Rect, Shape, Ui};

const COLUMNS: usize = 24;
const ROWS: usize = 16;

/// Paint a soft, track-colored halo around an active clip or mixer channel.
///
/// Four rounded rings form one texture-free mesh (128 vertices, 576 indices).
/// Geometry and cost stay constant at every zoom and density. The transparent
/// outer edge fades into the caller's canvas; no offscreen render target, clock,
/// repaint request, widget, or hit target is introduced. Keep the caller's clip
/// rectangle so the halo cannot escape its workspace or cover pinned headers.
pub fn paint_selection_glow(painter: &Painter, rect: Rect, radius: u8, color: Color32) {
    if !rect.is_finite()
        || !rect.is_positive()
        || !rect.size().is_finite()
        || !painter.is_visible()
        || !painter
            .clip_rect()
            .intersects(rect.expand(tokens::SELECTION_GLOW_EXTENT))
    {
        return;
    }

    const STEPS: usize = 8;
    const PERIMETER: usize = 4 * STEPS;
    const RINGS: usize = 4;
    // Fixed quarter-circle samples avoid trigonometry per selected item/frame.
    const ARC: [(f32, f32); STEPS] = [
        (1.0, 0.0),
        (0.974_927_9, 0.222_520_93),
        (0.900_968_85, 0.433_883_73),
        (0.781_831_5, 0.623_489_8),
        (0.623_489_8, 0.781_831_5),
        (0.433_883_73, 0.900_968_85),
        (0.222_520_93, 0.974_927_9),
        (0.0, 1.0),
    ];
    let extent = tokens::SELECTION_GLOW_EXTENT;
    let inner = 1.5_f32.min(rect.width().min(rect.height()) * 0.25);
    let alpha = if painter.ctx().theme() == egui::Theme::Dark {
        64
    } else {
        48
    };
    let rings = [
        (extent, 0),
        (extent * 0.45, alpha / 3),
        (0.0, alpha),
        (-inner, 0),
    ];
    let mut mesh = egui::Mesh::default();
    mesh.vertices.reserve_exact(PERIMETER * RINGS);
    mesh.indices.reserve_exact(PERIMETER * (RINGS - 1) * 6);
    for (offset, alpha) in rings {
        let bounds = rect.expand(offset);
        let radius = (f32::from(radius) + offset)
            .max(0.0)
            .min(bounds.width().min(bounds.height()) * 0.5);
        let centers = [
            bounds.left_top() + egui::vec2(radius, radius),
            bounds.right_top() + egui::vec2(-radius, radius),
            bounds.right_bottom() + egui::vec2(-radius, -radius),
            bounds.left_bottom() + egui::vec2(radius, -radius),
        ];
        let color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        for (corner, center) in centers.into_iter().enumerate() {
            for (cos, sin) in ARC {
                let normal = match corner {
                    0 => egui::vec2(-cos, -sin),
                    1 => egui::vec2(sin, -cos),
                    2 => egui::vec2(cos, sin),
                    _ => egui::vec2(-sin, cos),
                };
                mesh.colored_vertex(center + normal * radius, color);
            }
        }
    }
    for ring in 0..RINGS - 1 {
        for step in 0..PERIMETER {
            let next = (step + 1) % PERIMETER;
            let a = (ring * PERIMETER + step) as u32;
            let b = (ring * PERIMETER + next) as u32;
            let c = ((ring + 1) * PERIMETER + step) as u32;
            let d = ((ring + 1) * PERIMETER + next) as u32;
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    painter.add(mesh);
}

/// Paint a quiet, theme-aware radial wash inside an existing surface.
///
/// A single bounded mesh interpolates two broad tinted lobes. Its boundary is
/// transparent, so there are no rings, hard edges, shadows, or blur textures.
/// Paint before controls; use [`soft_surface`] when the final bounds are unknown.
pub fn paint_ambient(painter: &Painter, rect: Rect) {
    if let Some(mesh) = ambient_mesh(painter, rect) {
        painter.with_clip_rect(rect).add(mesh);
    }
}

/// Show a native frame with a subtle wash between its fill and its contents.
/// Layout, focus, and hit testing are identical to [`Frame::show`].
pub fn soft_surface<R>(
    ui: &mut Ui,
    frame: Frame,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let mut prepared = frame.begin(ui);
    let tint = ui.painter().add(Shape::Noop);
    let inner = contents(&mut prepared.content_ui);
    let rect = prepared.frame.fill_rect(prepared.content_ui.min_rect());
    if let Some(mesh) = ambient_mesh(ui.painter(), rect) {
        ui.painter().with_clip_rect(rect).set(tint, mesh);
    }
    let response = prepared.end(ui);
    InnerResponse::new(inner, response)
}

fn ambient_mesh(painter: &Painter, rect: Rect) -> Option<egui::Mesh> {
    // Avoid allocating for clipped rows, tiny controls, or unbounded layouts.
    if !rect.is_finite()
        || rect.width() < 48.0
        || rect.height() < 32.0
        || !rect.size().is_finite()
        || !painter.clip_rect().intersects(rect)
        || !painter.is_visible()
    {
        return None;
    }

    let colors = theme::palette(painter.ctx());
    let max_alpha = if painter.ctx().theme() == egui::Theme::Dark {
        14.0
    } else {
        10.0
    };
    // The bound is independent of display resolution and surface dimensions.
    // A short header needs fewer vertical samples than a large utility backdrop.
    let columns = ((rect.width() / 48.0).ceil() as usize).clamp(8, COLUMNS);
    let rows = ((rect.height() / 48.0).ceil() as usize).clamp(4, ROWS);
    let stride = columns + 1;
    let mut mesh = egui::Mesh::default();
    mesh.vertices.reserve_exact(stride * (rows + 1));
    mesh.indices.reserve_exact(columns * rows * 6);
    for row in 0..=rows {
        let y = row as f32 / rows as f32;
        for column in 0..=columns {
            let x = column as f32 / columns as f32;
            let primary = radial(x, y, 0.24, 0.34, 0.55, 0.72);
            let secondary = 0.45 * radial(x, y, 0.80, 0.64, 0.60, 0.80);
            let strength = (primary + secondary).min(1.0);
            let fade = smooth((x.min(1.0 - x) * 5.0).min(y.min(1.0 - y) * 5.0));
            let alpha = (max_alpha * strength * fade).round() as u8;
            let color = if alpha == 0 {
                Color32::TRANSPARENT
            } else {
                let cool = secondary / (primary + secondary);
                let channel = |sage: u8, slate: u8| {
                    (f32::from(sage) * (1.0 - cool) + f32::from(slate) * cool).round() as u8
                };
                Color32::from_rgba_unmultiplied(
                    channel(colors.accent.r(), colors.muted.r()),
                    channel(colors.accent.g(), colors.muted.g()),
                    channel(colors.accent.b(), colors.muted.b()),
                    alpha,
                )
            };
            mesh.colored_vertex(
                egui::pos2(egui::lerp(rect.x_range(), x), egui::lerp(rect.y_range(), y)),
                color,
            );
        }
    }
    for row in 0..rows {
        for column in 0..columns {
            let top = (row * stride + column) as u32;
            let bottom = top + stride as u32;
            mesh.add_triangle(top, bottom, top + 1);
            mesh.add_triangle(top + 1, bottom, bottom + 1);
        }
    }
    Some(mesh)
}

fn radial(x: f32, y: f32, center_x: f32, center_y: f32, width: f32, height: f32) -> f32 {
    let x = (x - center_x) / width;
    let y = (y - center_y) / height;
    (-2.0 * (x * x + y * y)).exp()
}

fn smooth(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}
