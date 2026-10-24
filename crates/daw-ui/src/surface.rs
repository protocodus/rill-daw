//! Broad, low-contrast surface tint. These helpers paint behind existing
//! controls; they never allocate an input target or own interaction state.
use crate::theme;
use egui::{Color32, Frame, InnerResponse, Painter, Rect, Shape, Ui};

const COLUMNS: usize = 24;
const ROWS: usize = 16;

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
