//! Quiet component materials and broad surface tint. These helpers paint behind
//! existing controls; they never allocate an input target or own interaction state.
use crate::{theme, tokens};
use egui::{Color32, Frame, InnerResponse, Painter, Rect, Shape, Stroke, Ui};

const COLUMNS: usize = 21;
const ROWS: usize = 13;
const MATERIAL_WIDTH: usize = 55;
const MATERIAL_HEIGHT: usize = 34;

/// Visual emphasis for a component. Native widgets continue to own interaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComponentMaterial {
    /// Lift a floating surface with a small, blurred shadow.
    pub elevated: bool,
    /// Add a quiet accent halo for the selected or keyboard-focused component.
    pub active: bool,
}

/// Build a complete component background for a reserved painter slot.
///
/// Reserve the slot before a native widget with transparent fill and no stroke,
/// then replace it with this shape using the response rectangle and native
/// visual state. Keep the native frame setting so its padding is preserved. The flat
/// fill, rounded gradient and inset border share native egui anti-aliasing and
/// pixel snapping. The optional shadow and glow are native blurred rectangles;
/// no software blur, layout, interaction, or repaint state is introduced.
pub fn component_shape(
    painter: &Painter,
    rect: Rect,
    radius: u8,
    fill: Color32,
    stroke: Stroke,
    material: ComponentMaterial,
) -> Shape {
    if !material_is_visible(painter, rect) {
        return Shape::Noop;
    }

    let dark = painter.ctx().theme() == egui::Theme::Dark;
    let mut shapes = Vec::with_capacity(6);
    if material.elevated {
        shapes.push(
            egui::epaint::Shadow {
                offset: [0, 3],
                blur: 8,
                spread: 0,
                color: Color32::from_black_alpha(if dark { 34 } else { 21 }),
            }
            .as_shape(rect, radius)
            .with_round_to_pixels(true)
            .into(),
        );
    }
    if material.active {
        let accent = theme::palette(painter.ctx()).accent;
        shapes.push(
            egui::epaint::Shadow {
                offset: [0, 0],
                blur: 8,
                spread: 0,
                color: Color32::from_rgba_unmultiplied(
                    accent.r(),
                    accent.g(),
                    accent.b(),
                    if dark { 21 } else { 13 },
                ),
            }
            .as_shape(rect, radius)
            .with_round_to_pixels(true)
            .into(),
        );
    }
    if fill != Color32::TRANSPARENT {
        shapes.push(
            egui::epaint::RectShape::filled(rect, radius, fill)
                .with_round_to_pixels(true)
                .into(),
        );
        shapes.push(material_overlay_shape(painter, rect, radius));
    }
    if !stroke.is_empty() {
        shapes.push(
            egui::epaint::RectShape::stroke(rect, radius, stroke, egui::StrokeKind::Inside)
                .with_round_to_pixels(true)
                .into(),
        );
    }
    Shape::Vec(shapes)
}

/// Build a restrained linear and radial tint above a flat fill, below content.
///
/// One 55 × 34 texture is retained per appearance and reused at every size. Its
/// brush uses egui's rounded rectangle tessellation, including anti-aliased
/// corners, instead of a rectangular mesh or an offscreen render target.
pub fn material_overlay_shape(painter: &Painter, rect: Rect, radius: u8) -> Shape {
    if !material_is_visible(painter, rect) {
        return Shape::Noop;
    }
    let texture = material_texture(painter.ctx());
    egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
        .with_texture(
            texture.id(),
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        )
        .with_round_to_pixels(true)
        .into()
}

fn material_is_visible(painter: &Painter, rect: Rect) -> bool {
    rect.is_finite()
        && rect.is_positive()
        && rect.size().is_finite()
        && painter.is_visible()
        && painter.clip_rect().intersects(rect.expand(8.0))
}

fn material_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let dark = ctx.theme() == egui::Theme::Dark;
    let id = egui::Id::new("rill_component_material").with(dark);
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture;
    }

    let accent = theme::palette(ctx).accent;
    let ratio = tokens::GOLDEN_RATIO_INVERSE;
    let complementary = 1.0 - ratio;
    let max_alpha = if dark { 13.0 } else { 8.0 };
    let mut pixels = Vec::with_capacity(MATERIAL_WIDTH * MATERIAL_HEIGHT);
    for row in 0..MATERIAL_HEIGHT {
        let y = row as f32 / (MATERIAL_HEIGHT - 1) as f32;
        for column in 0..MATERIAL_WIDTH {
            let x = column as f32 / (MATERIAL_WIDTH - 1) as f32;
            let linear = (1.0 - smooth(y)) * ratio;
            let radial = radial(x, y, complementary, complementary, ratio, 1.0) * complementary;
            let strength = linear + radial;
            let tint = radial / strength.max(f32::EPSILON);
            let channel =
                |value: u8| (255.0 * (1.0 - tint) + f32::from(value) * tint).round() as u8;
            pixels.push(Color32::from_rgba_unmultiplied(
                channel(accent.r()),
                channel(accent.g()),
                channel(accent.b()),
                (max_alpha * strength).round() as u8,
            ));
        }
    }
    let texture = ctx.load_texture(
        if dark {
            "rill_component_material_dark"
        } else {
            "rill_component_material_light"
        },
        egui::ColorImage::new([MATERIAL_WIDTH, MATERIAL_HEIGHT], pixels),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

/// Paint a soft, bounded halo around a selected clip, channel, or control.
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
        || rect.width() < 55.0
        || rect.height() < 34.0
        || !rect.size().is_finite()
        || !painter.clip_rect().intersects(rect)
        || !painter.is_visible()
    {
        return None;
    }

    let colors = theme::palette(painter.ctx());
    let max_alpha = if painter.ctx().theme() == egui::Theme::Dark {
        13.0
    } else {
        8.0
    };
    // The bound is independent of display resolution and surface dimensions.
    // A short header needs fewer vertical samples than a large utility backdrop.
    let columns = ((rect.width() / 55.0).ceil() as usize).clamp(8, COLUMNS);
    let rows = ((rect.height() / 55.0).ceil() as usize).clamp(3, ROWS);
    let ratio = tokens::GOLDEN_RATIO_INVERSE;
    let complementary = 1.0 - ratio;
    let stride = columns + 1;
    let mut mesh = egui::Mesh::default();
    mesh.vertices.reserve_exact(stride * (rows + 1));
    mesh.indices.reserve_exact(columns * rows * 6);
    for row in 0..=rows {
        let y = row as f32 / rows as f32;
        for column in 0..=columns {
            let x = column as f32 / columns as f32;
            let primary = radial(x, y, complementary, complementary, ratio, 1.0);
            let secondary = complementary * radial(x, y, ratio, ratio, ratio, 1.0);
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
