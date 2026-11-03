//! Shared mixer identity, insert and routing summaries, and read-only routing lanes.
use egui::{Color32, FontId, Pos2, Rect, Response, Sense, Ui, Vec2};

/// One reserved route lane under the strips: a 3 pt ribbon and one caption.
pub const MIXER_ROUTE_LANE: f32 = crate::tokens::SPACE_LG + crate::tokens::SPACE_XS + 2.0;
/// Height of the dashed empty-chain slot and of one compact send row.
pub const MIXER_SLOT_HEIGHT: f32 = crate::tokens::COMPACT_HEIGHT + crate::tokens::SPACE_SM;
pub const MIXER_SEND_ROW_HEIGHT: f32 = crate::tokens::COMPACT_HEIGHT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixerNamePlateKind {
    Track,
    Bus,
    Master,
}

fn one_line(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

/// One channel identity for tracks, buses and master: a full-height plate with
/// centered text. Tracks and buses take the channel color (buses add the BUS
/// badge inside the plate); master is neutral and not interactive.
pub fn mixer_name_plate(
    ui: &mut Ui,
    kind: MixerNamePlateKind,
    name: &str,
    width: f32,
    color: Color32,
) -> Response {
    let master = kind == MixerNamePlateKind::Master;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, crate::tokens::CONTROL_HEIGHT),
        if master {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let colors = crate::theme::palette(ui.ctx());
    let hairline = 1.0 / ui.ctx().pixels_per_point();
    let plate = egui::emath::GuiRounding::round_to_pixels(rect, ui.painter().pixels_per_point());
    let fill = if master { colors.raised } else { color };
    let foreground = if master {
        colors.text
    } else {
        crate::theme::contrast_text(fill)
    };
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(plate, crate::tokens::RADIUS, fill);
        if master {
            ui.painter().rect_stroke(
                plate,
                crate::tokens::RADIUS,
                egui::Stroke::new(hairline, colors.line),
                egui::StrokeKind::Inside,
            );
        }
        let mut text_area = rect.shrink2(Vec2::new(crate::tokens::SPACE_SM, 0.0));
        if kind == MixerNamePlateKind::Bus {
            let badge = Rect::from_center_size(
                Pos2::new(text_area.right() - 20.0, rect.center().y),
                Vec2::new(40.0, 18.0),
            );
            crate::paint_bus_badge(ui.painter(), badge, color);
            text_area.max.x = badge.left() - crate::tokens::SPACE_XS;
        }
        let galley = one_line(
            ui,
            name,
            crate::text::TextRole::Label.font(),
            foreground,
            text_area.width(),
        );
        ui.painter()
            .galley(text_area.center() - galley.size() * 0.5, galley, foreground);
        // A fixed identity fill needs its own hover/focus cue without changing hue.
        if !master && (response.hovered() || response.has_focus()) {
            ui.painter().rect_stroke(
                plate,
                crate::tokens::RADIUS,
                egui::Stroke::new(
                    if response.has_focus() {
                        crate::tokens::FOCUS_WIDTH
                    } else {
                        hairline
                    },
                    foreground.gamma_multiply(0.65),
                ),
                egui::StrokeKind::Inside,
            );
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            if master {
                egui::WidgetType::Label
            } else {
                egui::WidgetType::Button
            },
            ui.is_enabled(),
            name,
        )
    });
    if master {
        response.on_hover_text("Final stereo output · receives all channels routed to Master")
    } else {
        response
    }
}

/// The dashed empty-chain slot: one click adds the first insert.
pub fn mixer_insert_slot(ui: &mut Ui, width: f32) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, MIXER_SLOT_HEIGHT), Sense::click());
    let colors = crate::theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        if response.hovered() || response.has_focus() {
            ui.painter()
                .rect_filled(rect, crate::tokens::RADIUS, colors.control_hover);
        }
        let outline = rect.shrink(0.5);
        let stroke = egui::Stroke::new(
            if response.has_focus() {
                crate::tokens::FOCUS_WIDTH
            } else {
                crate::tokens::BORDER_WIDTH
            },
            colors.line.lerp_to_gamma(colors.muted, 0.5),
        );
        let corners = [
            outline.left_top(),
            outline.right_top(),
            outline.right_bottom(),
            outline.left_bottom(),
            outline.left_top(),
        ];
        ui.painter().extend(egui::Shape::dashed_line(
            &corners,
            stroke,
            crate::tokens::SPACE_XS,
            3.0,
        ));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "+ Insert",
            FontId::proportional(crate::tokens::FONT_CAPTION),
            colors.muted,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "+ Insert")
    });
    response.on_hover_text("Add an effect to this channel")
}

/// The source row's caption: the kind plus the actual source, e.g. "Audio · In 1".
pub fn mixer_source_caption(ui: &mut Ui, text: &str, width: f32) -> Response {
    ui.allocate_ui_with_layout(
        Vec2::new(width, crate::tokens::CONTROL_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(Vec2::new(width, crate::tokens::CONTROL_HEIGHT));
            ui.add(
                egui::Label::new(crate::text::TextRole::Note.rich(ui.ctx(), text))
                    .truncate()
                    .selectable(false),
            )
        },
    )
    .inner
}

/// Count the main outputs and auxiliary sends entering a bus ("Bus · 2 in"),
/// with named sources on hover. `None` denotes a main output; `Some` is a send
/// whose boolean identifies its pre-fader position.
pub fn mixer_bus_input_summary<'a>(
    ui: &mut Ui,
    width: f32,
    inputs: impl ExactSizeIterator<Item = (&'a str, Option<bool>)>,
) -> Response {
    let count = inputs.len();
    mixer_source_caption(ui, &format!("Bus · {count} in"), width).on_hover_ui(|ui| {
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

/// The Effects header: the whole row opens the chain; "active/total" sits at
/// its trailing edge once the chain has inserts.
pub fn mixer_effects_button(ui: &mut Ui, width: f32, count: usize, active: usize) -> Response {
    let response = ui.add_sized(
        Vec2::new(width, crate::tokens::CONTROL_HEIGHT),
        crate::view_button_widget("Effects"),
    );
    if count > 0 && ui.is_rect_visible(response.rect) {
        let colors = crate::theme::palette(ui.ctx());
        ui.painter().text(
            Pos2::new(
                response.rect.right() - crate::tokens::SPACE_SM,
                response.rect.center().y,
            ),
            egui::Align2::RIGHT_CENTER,
            format!("{active}/{count}"),
            FontId::proportional(crate::tokens::FONT_CAPTION),
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
        Vec2::new(ui.available_width(), MIXER_ROUTE_LANE),
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
        (rect.bottom() - crate::tokens::SPACE_XS).max(from.y.max(to.y) + head_height + half);
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
                rect.top() + crate::tokens::SPACE_XS + crate::tokens::FONT_CAPTION + 3.0,
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
    Effects,
    Level,
    Output,
}

/// Every frame extends this far beyond its controls on all sides.
pub const MIXER_SECTION_PADDING: f32 = crate::tokens::SPACE_XS;
/// Space between neighboring controls of two sections: a uniform gap between frames.
pub const MIXER_SECTION_GAP: f32 = crate::tokens::SPACE_SM + 2.0 * MIXER_SECTION_PADDING;

impl MixerSection {
    pub const ALL: [Self; 4] = [Self::Source, Self::Effects, Self::Level, Self::Output];
    pub fn name(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Effects => "Effects",
            Self::Level => "Volume & pan",
            Self::Output => "Output & sends",
        }
    }
    pub fn fill(self, ctx: &egui::Context) -> Color32 {
        let colors = crate::theme::palette(ctx);
        match self {
            Self::Source | Self::Level => colors.bg.gamma_multiply(0.55),
            Self::Effects | Self::Output => colors.raised.gamma_multiply(0.8),
        }
    }
}

/// Paint one frame per section behind the contents' bounds. A `None` bound
/// (an empty master section) paints nothing.
pub fn mixer_sections<R>(
    ui: &mut Ui,
    contents: impl FnOnce(&mut Ui) -> (R, [Option<Rect>; 4]),
) -> R {
    let background = ui.painter().add(egui::Shape::Noop);
    let (value, sections) = contents(ui);
    let colors = crate::theme::palette(ui.ctx());
    let hairline = 1.0 / ui.ctx().pixels_per_point();
    let mut shapes = Vec::with_capacity(8);
    for (section, bounds) in MixerSection::ALL.into_iter().zip(sections) {
        let Some(bounds) = bounds else { continue };
        let rect = egui::emath::GuiRounding::round_to_pixels(
            bounds.expand(MIXER_SECTION_PADDING),
            ui.pixels_per_point(),
        );
        shapes.push(egui::Shape::rect_filled(
            rect,
            crate::tokens::RADIUS,
            section.fill(ui.ctx()),
        ));
        shapes.push(egui::Shape::rect_stroke(
            rect,
            crate::tokens::RADIUS,
            egui::Stroke::new(hairline, colors.line),
            egui::StrokeKind::Inside,
        ));
    }
    ui.painter().set(background, egui::Shape::Vec(shapes));
    value
}
