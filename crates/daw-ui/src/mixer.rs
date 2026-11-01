//! Shared mixer readouts, calibrated scales, and read-only routing lanes.
use egui::{Color32, FontId, Pos2, Rect, Response, Sense, Ui, Vec2};

/// A bounded insert preview with the full ordered chain available on hover.
/// The iterator borrows names; details are formatted only when the tooltip opens.
pub fn mixer_insert_summary<'a>(
    ui: &mut Ui,
    width: f32,
    inserts: impl ExactSizeIterator<Item = (&'a str, bool)> + Clone,
) -> Response {
    let count = inserts.len();
    let first = inserts.clone().next();
    ui.allocate_ui_with_layout(
        Vec2::new(width, crate::tokens::COMPACT_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            // This is a read-only caption row. Native horizontal layouts use
            // interact_size as their minimum, even when they contain no buttons.
            ui.spacing_mut().interact_size.y = crate::tokens::COMPACT_HEIGHT;
            ui.set_min_height(crate::tokens::COMPACT_HEIGHT);
            crate::action_row(
                ui,
                |ui| {
                    if count > 1 {
                        crate::text::note(ui, format!("+{}", count - 1));
                    }
                },
                |ui| {
                    let (name, enabled) = first.unwrap_or(("No inserts", true));
                    let mut text = crate::text::TextRole::Note.rich(ui.ctx(), name);
                    if !enabled {
                        text = text.strikethrough();
                    }
                    ui.add(egui::Label::new(text).truncate());
                },
            );
        },
    )
    .response
    .on_hover_ui(|ui| {
        crate::text::label(ui, "Inserts · top to bottom");
        if count == 0 {
            crate::text::note(ui, "No inserts");
        }
        for (index, (name, enabled)) in inserts.enumerate() {
            crate::text::note(
                ui,
                format!(
                    "{:02} · {name} · {}",
                    index + 1,
                    if enabled { "Active" } else { "Bypassed" }
                ),
            );
        }
    })
}

/// Count the main outputs and auxiliary sends entering a bus, with named
/// sources available on hover. `None` denotes a main output; `Some` is a send
/// whose boolean identifies its pre-fader position.
pub fn mixer_bus_input_summary<'a>(
    ui: &mut Ui,
    inputs: impl ExactSizeIterator<Item = (&'a str, Option<bool>)>,
) -> Response {
    let count = inputs.len();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_XS;
        let kind = crate::text::note(ui, "Bus");
        let count = ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{count} in"))
                    .size(crate::tokens::FONT_CANVAS)
                    .color(crate::theme::palette(ui.ctx()).muted),
            )
            .truncate(),
        );
        kind.union(count)
    })
    .inner
    .on_hover_ui(|ui| {
        crate::text::label(ui, "Incoming signal");
        if count == 0 {
            crate::text::note(ui, "No incoming channels or sends");
        }
        for (name, pre_fader) in inputs {
            let kind = match pre_fader {
                None => "Output",
                Some(true) => "Pre send",
                Some(false) => "Post send",
            };
            crate::text::note(ui, format!("{kind} · {name}"));
        }
    })
}

/// A native Effects action with a quiet insert-count badge. The full button,
/// including its count, opens the same chain; it retains the ordinary action's
/// label and keyboard behavior. No badge is drawn when the width is too small.
pub fn mixer_effects_button(ui: &mut Ui, width: f32, count: usize, active: usize) -> Response {
    let response = ui.add_sized(
        Vec2::new(width, crate::tokens::CONTROL_HEIGHT),
        crate::view_button_widget("Effects"),
    );
    if response.rect.width() >= 144.0 && ui.is_rect_visible(response.rect) {
        let colors = crate::theme::palette(ui.ctx());
        let badge = Rect::from_center_size(
            Pos2::new(response.rect.right() - 16.0, response.rect.center().y),
            Vec2::new(22.0, 20.0),
        );
        ui.painter()
            .rect_filled(badge, crate::tokens::RADIUS, colors.bg);
        ui.painter().text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            if count > 99 {
                "99+".to_owned()
            } else {
                count.to_string()
            },
            FontId::proportional(crate::tokens::FONT_CANVAS),
            if active > 0 {
                colors.accent
            } else {
                colors.muted
            },
        );
    }
    response.on_hover_text(format!(
        "{active} active / {count} inserts · before volume and pan"
    ))
}

/// Calibrate the native vertical 0..=1.5 gain fader without changing its gain
/// law or hit target. The tick coordinates account for egui's handle travel.
/// Reserve 44 points to its left. Labels supplement the shared fractional ticks.
pub fn paint_mixer_fader_scale(ui: &Ui, fader: Rect) {
    if !ui.is_rect_visible(fader) || fader.height() < 80.0 {
        return;
    }
    let colors = crate::theme::palette(ui.ctx());
    let radius = fader.width() / 2.5;
    let end_padding = match ui.visuals().handle_shape {
        egui::style::HandleShape::Circle => radius,
        egui::style::HandleShape::Rect { aspect_ratio } => radius * aspect_ratio,
    };
    let top = fader.top() + end_padding;
    let bottom = fader.bottom() - end_padding;
    for (label, gain) in [
        ("+3.5 dB", 1.5_f32),
        ("0", 1.0_f32),
        ("−6", 10.0_f32.powf(-6.0 / 20.0)),
        ("−12", 10.0_f32.powf(-12.0 / 20.0)),
        ("−∞", 0.0),
    ] {
        let y = bottom - (bottom - top) * (gain / 1.5);
        let color = if gain == 1.0 {
            colors.text
        } else {
            colors.muted
        };
        ui.painter().line_segment(
            [
                Pos2::new(fader.left() - 6.0, y),
                Pos2::new(fader.left() - 1.0, y),
            ],
            egui::Stroke::new(1.0_f32, color),
        );
        ui.painter().text(
            Pos2::new(fader.left() - 10.0, y),
            egui::Align2::RIGHT_CENTER,
            label,
            FontId::proportional(crate::tokens::FONT_CANVAS),
            color,
        );
    }
}

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

/// The actual attachment point on a channel strip, in screen coordinates.
#[derive(Clone, Copy, Debug)]
pub struct MixerRouteEndpoint {
    pub position: Pos2,
    pub color: Color32,
}

#[derive(Clone, Copy, Debug)]
pub struct MixerRoutePaint {
    pub lane: Rect,
    pub shaft: Rect,
    pub head: Rect,
    pub from: Pos2,
    pub to: Pos2,
    pub path: [Pos2; 4],
    pub source_color: Color32,
    pub target_color: Color32,
    pub stroke_width: f32,
    pub source_clipped: bool,
    pub target_clipped: bool,
}

pub struct MixerRouteResponse {
    pub response: Response,
    pub painted: Option<MixerRoutePaint>,
}

/// One destination per lane. The continuous U-shaped connection starts at the
/// source strip, descends into its lane and rises to an arrowhead touching the
/// destination strip. Vertex colors interpolate along the complete route.
///
/// `connection_clip` is the channel pane's clip rectangle, before entering the
/// lane scroller: stems must remain attached above that scroller. Offscreen
/// endpoints retain their true positions and are clipped, never moved to another
/// channel. Each visible connection uses eight ribbon vertices and one triangle;
/// there are no textures, per-pixel sampling, or extra repaint requests.
pub fn mixer_route_lane(
    ui: &mut Ui,
    source: MixerRouteEndpoint,
    target: MixerRouteEndpoint,
    caption: &str,
    connection_clip: Rect,
) -> MixerRouteResponse {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), crate::tokens::MIXER_ROUTE_LANE_HEIGHT),
        Sense::hover(),
    );
    let visible = rect.intersect(ui.clip_rect());
    if !visible.is_positive()
        || visible.width() < 32.0
        || !source.position.is_finite()
        || !target.position.is_finite()
        || (source.position.x - target.position.x).abs() < crate::tokens::MIXER_ROUTE_WIDTH
    {
        return MixerRouteResponse {
            response,
            painted: None,
        };
    }
    let colors = crate::theme::palette(ui.ctx());
    let source_clipped = !connection_clip.contains(source.position);
    let target_clipped = !connection_clip.contains(target.position);
    let caption = if target_clipped {
        format!("{caption} · target offscreen")
    } else if source_clipped {
        format!("{caption} · source offscreen")
    } else {
        caption.to_owned()
    };
    let from = source.position;
    let to = target.position;
    let half = crate::tokens::MIXER_ROUTE_WIDTH * 0.5;
    let head_height = crate::tokens::MIXER_ROUTE_HEAD;
    let turn_y =
        (rect.bottom() - crate::tokens::SPACE_SM).max(from.y.max(to.y) + head_height + half);
    let path = [from, Pos2::new(from.x, turn_y), Pos2::new(to.x, turn_y), to];
    let clip = Rect::from_min_max(
        Pos2::new(visible.left(), from.y.min(to.y)),
        Pos2::new(visible.right(), visible.bottom()),
    )
    .intersect(connection_clip);
    // Growing only to the caller's channel pane is deliberate: retaining the
    // nested lane scroller's top clip would detach every vertical stem.
    let mut painter = ui.painter().clone();
    painter.set_clip_rect(clip);
    let direction = (to.x - from.x).signum();
    let total_length = (turn_y - from.y) + (to.x - from.x).abs() + (turn_y - to.y);
    let progress = [
        0.0,
        turn_y - from.y,
        turn_y - from.y + (to.x - from.x).abs(),
        total_length - head_height,
    ];
    let stations = [path[0], path[1], path[2], to + Vec2::new(0.0, head_height)];
    // Mitered corners share vertices, so neither direction has hairline gaps.
    let offsets = [
        Vec2::new(-half, 0.0),
        Vec2::new(-half, direction * half),
        Vec2::new(half, direction * half),
        Vec2::new(half, 0.0),
    ];
    let mut mesh = egui::Mesh::default();
    for ((position, offset), distance) in stations.into_iter().zip(offsets).zip(progress) {
        let color = source
            .color
            .lerp_to_gamma(target.color, distance / total_length);
        mesh.colored_vertex(position + offset, color);
        mesh.colored_vertex(position - offset, color);
    }
    for index in 0..3_u32 {
        let start = index * 2;
        mesh.add_triangle(start, start + 1, start + 2);
        mesh.add_triangle(start + 1, start + 3, start + 2);
    }
    let shaft = mesh.calc_bounds();
    painter.add(egui::Shape::mesh(mesh));
    let mut head_mesh = egui::Mesh::default();
    head_mesh.colored_vertex(to, target.color);
    let head_color = source
        .color
        .lerp_to_gamma(target.color, (total_length - head_height) / total_length);
    head_mesh.colored_vertex(to + Vec2::new(-head_height * 0.5, head_height), head_color);
    head_mesh.colored_vertex(to + Vec2::new(head_height * 0.5, head_height), head_color);
    head_mesh.add_triangle(0, 1, 2);
    let head = head_mesh.calc_bounds();
    painter.add(egui::Shape::mesh(head_mesh));

    // Keep captions between channel stems. Long names remain in the hover text;
    // they cannot cover a neighboring route's vertical connection.
    let caption_width = crate::tokens::SELECT_WIDTH;
    let caption_left = if direction > 0.0 {
        from.x + crate::tokens::SPACE_SM
    } else {
        from.x - crate::tokens::SPACE_SM - caption_width
    };
    let caption_left = caption_left.clamp(
        visible.left() + crate::tokens::SPACE_SM,
        (visible.right() - caption_width - crate::tokens::SPACE_SM)
            .max(visible.left() + crate::tokens::SPACE_SM),
    );
    crate::clip_title(
        &ui.painter().with_clip_rect(visible),
        Rect::from_min_max(
            Pos2::new(caption_left, rect.top() + crate::tokens::SPACE_XS),
            Pos2::new(
                (caption_left + caption_width).min(visible.right() - crate::tokens::SPACE_SM),
                rect.top() + 22.0,
            ),
        ),
        FontId::proportional(crate::tokens::FONT_CAPTION),
        colors.muted,
        &caption,
        false,
    );
    MixerRouteResponse {
        response,
        painted: Some(MixerRoutePaint {
            lane: rect,
            shaft,
            head,
            from,
            to,
            path,
            source_color: source.color,
            target_color: target.color,
            stroke_width: crate::tokens::MIXER_ROUTE_WIDTH,
            source_clipped,
            target_clipped,
        }),
    }
}

/// Signal-flow sections share the strip's existing control geometry. Paint the
/// frames before their controls, so borders never obscure fields or hit targets.
#[derive(Clone, Copy)]
pub enum MixerSection {
    Source,
    Inserts,
    Level,
    Output,
}

impl MixerSection {
    pub const ALL: [Self; 4] = [Self::Source, Self::Inserts, Self::Level, Self::Output];
    pub fn name(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Inserts => "Inserts",
            Self::Level => "Volume & pan",
            Self::Output => "Output & sends",
        }
    }
    pub fn fill(self, ctx: &egui::Context) -> Color32 {
        let colors = crate::theme::palette(ctx);
        match self {
            Self::Source | Self::Level => colors.bg.gamma_multiply(0.55),
            Self::Inserts | Self::Output => colors.raised.gamma_multiply(0.8),
        }
    }
}

pub fn mixer_sections<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> (R, [Rect; 4])) -> R {
    let background = ui.painter().add(egui::Shape::Noop);
    let (value, sections) = contents(ui);
    let colors = crate::theme::palette(ui.ctx());
    let mut shapes = Vec::with_capacity(11);
    for (section, bounds) in MixerSection::ALL.into_iter().zip(sections) {
        let rect = bounds.expand2(egui::vec2(crate::tokens::SPACE_XS, 2.0));
        shapes.push(egui::Shape::rect_filled(
            rect,
            crate::tokens::RADIUS,
            section.fill(ui.ctx()),
        ));
        shapes.push(egui::Shape::rect_stroke(
            rect,
            crate::tokens::RADIUS,
            egui::Stroke::new(crate::tokens::BORDER_WIDTH, colors.line),
            egui::StrokeKind::Inside,
        ));
    }
    for pair in sections.windows(2) {
        let center = egui::pos2(pair[0].center().x, (pair[0].bottom() + pair[1].top()) * 0.5);
        let radius = crate::tokens::SPACE_XS * 0.5;
        shapes.push(egui::Shape::convex_polygon(
            vec![
                center + egui::vec2(-radius, -radius * 0.5),
                center + egui::vec2(radius, -radius * 0.5),
                center + egui::vec2(0.0, radius),
            ],
            colors.muted,
            egui::Stroke::NONE,
        ));
    }
    ui.painter().set(background, egui::Shape::Vec(shapes));
    value
}
