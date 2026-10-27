//! Read-only routing lanes below the mixer channel controls.
use egui::{Color32, FontId, Pos2, Rect, Response, Sense, Ui, Vec2};

/// A quiet visual group label aligned with the channel name controls beneath it.
pub fn paint_mixer_group_header(painter: &egui::Painter, rect: Rect, label: &str) {
    crate::clip_title(
        painter,
        rect.shrink2(Vec2::new(crate::tokens::SPACE_MD, 0.0)),
        FontId::proportional(crate::tokens::FONT_CAPTION),
        crate::theme::palette(painter.ctx()).muted,
        label,
        false,
    );
}

#[derive(Clone, Copy, Debug)]
pub struct MixerRoutePaint {
    pub lane: Rect,
    pub shaft: Rect,
    pub head: Rect,
    pub from: Pos2,
    pub to: Pos2,
    pub source_clipped: bool,
    pub target_clipped: bool,
}

pub struct MixerRouteResponse {
    pub response: Response,
    pub painted: Option<MixerRoutePaint>,
}

/// One destination per lane keeps output and auxiliary-send arrows apart.
/// X anchors follow the actual channel strips; an offscreen destination ends at
/// the viewport edge and remains named in the caption. This never edits routing.
pub fn mixer_route_lane(
    ui: &mut Ui,
    source_x: f32,
    target_x: f32,
    caption: &str,
    color: Color32,
) -> MixerRouteResponse {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), crate::tokens::CONTROL_HEIGHT),
        Sense::hover(),
    );
    let visible = rect.intersect(ui.clip_rect());
    if !visible.is_positive()
        || visible.width() < 32.0
        || !source_x.is_finite()
        || !target_x.is_finite()
    {
        return MixerRouteResponse {
            response,
            painted: None,
        };
    }
    let colors = crate::theme::palette(ui.ctx());
    let painter = ui.painter().with_clip_rect(visible);
    let source_clipped = source_x < visible.left() || source_x > visible.right();
    let target_clipped = target_x < visible.left() || target_x > visible.right();
    let caption = if target_clipped {
        format!("{caption} · target offscreen")
    } else if source_clipped {
        format!("{caption} · source offscreen")
    } else {
        caption.to_owned()
    };
    crate::clip_title(
        &painter,
        Rect::from_min_max(
            visible.min + Vec2::new(crate::tokens::SPACE_SM, 0.0),
            Pos2::new(visible.right() - crate::tokens::SPACE_SM, rect.top() + 18.0),
        ),
        FontId::proportional(crate::tokens::FONT_CAPTION),
        colors.muted,
        &caption,
        false,
    );
    let left = visible.left() + crate::tokens::SPACE_SM;
    let right = visible.right() - crate::tokens::SPACE_SM;
    let mut from_x = source_x.clamp(left, right);
    let mut to_x = target_x.clamp(left, right);
    let direction = if target_x >= source_x { 1.0 } else { -1.0 };
    if (to_x - from_x).abs() < 16.0 {
        // Keep the direction legible when both endpoints have scrolled beyond
        // the same viewport edge; the caption identifies the clipped channels.
        to_x = (from_x + direction * 16.0).clamp(left, right);
        from_x = to_x - direction * 16.0;
    }
    let y = rect.bottom() - 7.0;
    let from = Pos2::new(from_x, y);
    let to = Pos2::new(to_x, y);
    let line_color = colors.muted.lerp_to_gamma(color, 0.65);
    let shaft = Rect::from_min_max(
        Pos2::new(from_x.min(to_x), y - 0.75),
        Pos2::new(from_x.max(to_x), y + 0.75),
    );
    painter.rect_filled(shaft, 0.75, line_color);
    painter.circle_filled(from, 2.0, line_color);
    let mut mesh = egui::Mesh::default();
    for vertex in [
        to,
        to + Vec2::new(-direction * 7.0, -4.0),
        to + Vec2::new(-direction * 7.0, 4.0),
    ] {
        mesh.colored_vertex(vertex, line_color);
    }
    mesh.add_triangle(0, 1, 2);
    let head = mesh.calc_bounds();
    painter.add(egui::Shape::mesh(mesh));
    MixerRouteResponse {
        response,
        painted: Some(MixerRoutePaint {
            lane: rect,
            shaft,
            head,
            from,
            to,
            source_clipped,
            target_clipped,
        }),
    }
}
