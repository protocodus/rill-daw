//! Shared mixer identity, insert and routing summaries, and read-only routing lanes.
use egui::{Align2, Color32, FontId, Pos2, Rect, Response, Sense, Ui, Vec2};

/// Channel card width: 8 pt margins leave 128 pt for the strip's controls.
pub const MIXER_CHANNEL_WIDTH: f32 = crate::tokens::MIXER_STRIP_WIDTH;
pub const MIXER_CHANNEL_MARGIN: f32 = crate::tokens::SPACE_SM;
pub const MIXER_CONTENT_WIDTH: f32 = MIXER_CHANNEL_WIDTH - 2.0 * MIXER_CHANNEL_MARGIN;
/// Space between rows of one section, and between sections. Sections are
/// separated by space alone: the strip has no frames or rules inside.
pub const MIXER_ROW_GAP: f32 = crate::tokens::SPACE_XS;
/// Padding inside a section band, and the band's distance to the strip edge.
pub const MIXER_BAND_PADDING: f32 = MIXER_CHANNEL_MARGIN * 0.5;
/// Band padding, the gap between bands, then the next band's padding: every
/// band sits the same distance from its neighbours and from the strip edges.
pub const MIXER_SECTION_GAP: f32 = 3.0 * MIXER_BAND_PADDING;
/// One reserved route lane under the strips: a caption over a 3 pt ribbon.
pub const MIXER_ROUTE_LANE: f32 = crate::tokens::CONTROL_HEIGHT;
/// Height of the empty-chain slot, one compact send row and "Add send…".
pub const MIXER_SLOT_HEIGHT: f32 = crate::tokens::COMPACT_HEIGHT;
pub const MIXER_SEND_ROW_HEIGHT: f32 = crate::tokens::COMPACT_HEIGHT;
/// Width of the destination color rail at the leading edge of routing rows.
pub const MIXER_RAIL_WIDTH: f32 = 3.0;

/// A role row above bus identities. Functional console rows share a baseline
/// below this identity band, so a return never has a shorter fader than a track.
pub const MIXER_ROLE_HEIGHT: f32 = crate::tokens::CONTROL_HEIGHT;

/// Apply the console's existing compact control scale in short workspaces.
/// The style is local to the strip; it never changes document or parameter state.
pub fn mixer_density_style(ui: &mut Ui, compact: bool) {
    if compact {
        ui.spacing_mut().interact_size.y = crate::tokens::COMPACT_HEIGHT;
        ui.spacing_mut().button_padding.y = 2.0;
        ui.style_mut().drag_value_text_style = egui::TextStyle::Small;
        ui.style_mut().text_styles.insert(
            egui::TextStyle::Button,
            FontId::proportional(crate::tokens::FONT_CAPTION),
        );
    }
}

/// Console rows use either the standard or compact shared hit-target height.
pub fn mixer_control_height(ui: &Ui) -> f32 {
    ui.spacing()
        .interact_size
        .y
        .clamp(crate::tokens::COMPACT_HEIGHT, crate::tokens::CONTROL_HEIGHT)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixerChannelRole {
    Bus,
    SendReturn,
    InstrumentOutput,
}

impl MixerChannelRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bus => "Bus",
            Self::SendReturn => "Send return",
            Self::InstrumentOutput => "Output",
        }
    }
}

/// Large, read-only channel type above downstream buses. Send destinations use
/// an incoming-arrow icon; ordinary grouping buses use the routing graph icon.
pub fn mixer_role_heading(
    ui: &mut Ui,
    role: MixerChannelRole,
    width: f32,
    color: Color32,
) -> Response {
    let height = mixer_control_height(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let icon_size = height.min(crate::tokens::CONTROL_HEIGHT);
        let icon = Rect::from_center_size(
            Pos2::new(
                rect.left() + icon_size * 0.5 + crate::tokens::SPACE_XS,
                rect.center().y,
            ),
            Vec2::splat(icon_size),
        );
        match role {
            MixerChannelRole::Bus => crate::Icon::Routing,
            MixerChannelRole::SendReturn => crate::Icon::Input,
            MixerChannelRole::InstrumentOutput => crate::Icon::Instrument,
        }
        .paint(
            ui.painter(),
            icon,
            color.lerp_to_gamma(crate::theme::palette(ui.ctx()).text, 0.6),
        );
        ui.painter().text(
            Pos2::new(icon.right() + crate::tokens::SPACE_XS, rect.center().y),
            egui::Align2::LEFT_CENTER,
            role.label(),
            FontId::proportional(crate::tokens::FONT_CAPTION),
            crate::theme::palette(ui.ctx()).text,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), role.label())
    });
    crate::hover_text(
        response,
        match role {
            MixerChannelRole::Bus => "Bus · groups channels through their main outputs",
            MixerChannelRole::SendReturn => {
                "Send return · receives one or more sends; it can also receive main outputs"
            }
            MixerChannelRole::InstrumentOutput => {
                "Instrument output · independent audio from the grouped instrument"
            }
        },
    )
}

/// Text style of a quiet strip row.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MixerRowText {
    /// Body text for primary rows (Effects, the output destination).
    Body,
    /// Caption text for compact rows (sends).
    Caption,
    /// A document-changing command ("Add send…", "Add insert…").
    Action,
    /// Muted caption for placeholders.
    Placeholder,
}

/// A quiet strip row: no surface until hover or keyboard focus, text inset
/// SPACE_SM from the leading edge, an optional destination rail in front of it
/// and an optional caption at the trailing edge. Effects, insert slot, output,
/// send and add-send rows all share it, so their text starts on one line.
pub fn mixer_row(
    ui: &mut Ui,
    width: f32,
    height: f32,
    label: &str,
    style: MixerRowText,
    rail: Option<Color32>,
    trailing: Option<(&str, Color32)>,
) -> Response {
    mixer_row_with_icon(ui, width, height, label, style, rail, trailing, None)
}

/// [`mixer_row`] with a small leading icon, such as the input and output arrows.
#[allow(clippy::too_many_arguments)]
pub fn mixer_row_with_icon(
    ui: &mut Ui,
    width: f32,
    height: f32,
    label: &str,
    style: MixerRowText,
    rail: Option<Color32>,
    trailing: Option<(&str, Color32)>,
    icon: Option<crate::Icon>,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let colors = crate::theme::palette(ui.ctx());
        let ppp = ui.painter().pixels_per_point();
        let surface = egui::emath::GuiRounding::round_to_pixels(rect, ppp);
        if response.hovered() || response.has_focus() {
            ui.painter()
                .rect_filled(surface, crate::tokens::RADIUS, colors.control_hover);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                surface,
                crate::tokens::RADIUS,
                egui::Stroke::new(crate::tokens::FOCUS_WIDTH, colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(color) = rail {
            let inset = if height > crate::tokens::COMPACT_HEIGHT {
                crate::tokens::SPACE_SM
            } else {
                crate::tokens::SPACE_XS
            };
            let band = egui::emath::GuiRounding::round_to_pixels(
                Rect::from_min_max(
                    Pos2::new(rect.left(), rect.top() + inset),
                    Pos2::new(rect.left() + MIXER_RAIL_WIDTH, rect.bottom() - inset),
                ),
                ppp,
            );
            ui.painter().rect_filled(band, 1.0, color);
        }
        let (font, color) = match style {
            MixerRowText::Body => (FontId::proportional(crate::tokens::FONT_BODY), colors.text),
            MixerRowText::Caption => (
                FontId::proportional(crate::tokens::FONT_CAPTION),
                colors.text,
            ),
            MixerRowText::Action => (
                FontId::proportional(crate::tokens::FONT_BODY),
                colors.action,
            ),
            MixerRowText::Placeholder => (
                FontId::proportional(crate::tokens::FONT_CAPTION),
                colors.muted,
            ),
        };
        let mut right = rect.right() - crate::tokens::SPACE_XS;
        if let Some((text, trailing_color)) = trailing {
            let galley = ui.painter().layout_no_wrap(
                text.to_owned(),
                FontId::proportional(crate::tokens::FONT_CAPTION),
                trailing_color,
            );
            right -= galley.size().x;
            ui.painter().galley(
                Pos2::new(right, rect.center().y - galley.size().y * 0.5),
                galley,
                trailing_color,
            );
            right -= crate::tokens::SPACE_XS;
        }
        let mut left = rect.left() + crate::tokens::SPACE_SM;
        if let Some(icon) = icon {
            left = paint_leading_icon(ui, icon, left, rect.center().y);
        }
        let galley = one_line(ui, label, font, color, right - left);
        ui.painter().galley(
            Pos2::new(left, rect.center().y - galley.size().y * 0.5),
            galley,
            color,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixerNamePlateKind {
    Track,
    Bus,
    Master,
}

/// Size of the input and output arrows that lead mixer rows.
const MIXER_ROW_ICON: f32 = crate::tokens::SPACE_MD;

/// Paints a muted row icon at `left`, centered on `center_y`; returns where the text starts.
fn paint_leading_icon(ui: &Ui, icon: crate::Icon, left: f32, center_y: f32) -> f32 {
    let rect = Rect::from_min_size(
        Pos2::new(left, center_y - MIXER_ROW_ICON * 0.5),
        Vec2::splat(MIXER_ROW_ICON),
    );
    icon.paint(ui.painter(), rect, crate::theme::palette(ui.ctx()).muted);
    rect.right() + crate::tokens::SPACE_XS
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
/// centered text. Tracks and buses take the channel color; their separate role
/// heading leaves the full name width available. Master is neutral.
pub fn mixer_name_plate(
    ui: &mut Ui,
    kind: MixerNamePlateKind,
    name: &str,
    width: f32,
    color: Color32,
) -> Response {
    mixer_name_plate_inner(ui, kind, name, width, color, false)
}

/// A channel identity with a small dedicated drag grip. The name retains its
/// own selection, open and context-menu hit area; the two never overlap.
pub fn mixer_name_plate_with_grip(
    ui: &mut Ui,
    kind: MixerNamePlateKind,
    name: &str,
    width: f32,
    color: Color32,
    track: u64,
    enabled: bool,
) -> (Response, Response) {
    let plate = mixer_name_plate_inner(ui, kind, name, width, color, true);
    let grip_rect = Rect::from_min_max(
        plate.rect.min,
        Pos2::new(
            plate.rect.left() + crate::tokens::COMPACT_HEIGHT,
            plate.rect.bottom(),
        ),
    );
    let foreground = crate::theme::contrast_text(color);
    let grip = crate::track_reorder_grip_tinted_at(
        ui,
        grip_rect.shrink2(Vec2::new(
            2.0,
            if grip_rect.height() <= crate::tokens::COMPACT_HEIGHT {
                2.0
            } else {
                6.0
            },
        )),
        plate.id.with("reorder_grip"),
        track,
        if enabled {
            foreground
        } else {
            foreground.gamma_multiply(0.5)
        },
        enabled,
    );
    let name_rect = Rect::from_min_max(
        Pos2::new(grip_rect.right(), plate.rect.top()),
        plate.rect.max,
    );
    let name_response = ui.interact(name_rect, plate.id.with("name"), Sense::click());
    name_response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name));
    if name_response.has_focus() {
        ui.painter().rect_stroke(
            plate.rect,
            crate::tokens::RADIUS,
            egui::Stroke::new(crate::tokens::FOCUS_WIDTH, foreground.gamma_multiply(0.65)),
            egui::StrokeKind::Inside,
        );
    }
    (name_response, grip)
}

fn mixer_name_plate_inner(
    ui: &mut Ui,
    kind: MixerNamePlateKind,
    name: &str,
    width: f32,
    color: Color32,
    grip: bool,
) -> Response {
    let master = kind == MixerNamePlateKind::Master;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, mixer_control_height(ui)),
        if grip { Sense::hover() } else { Sense::click() },
    );
    let colors = crate::theme::palette(ui.ctx());
    let plate = egui::emath::GuiRounding::round_to_pixels(rect, ui.painter().pixels_per_point());
    let fill = if master { colors.raised } else { color };
    let foreground = if master {
        colors.text
    } else {
        crate::theme::contrast_text(fill)
    };
    if ui.is_rect_visible(rect) {
        // Hover lightens the identity fill toward its text without changing hue.
        let hovered = !master && response.hovered();
        ui.painter().rect_filled(
            plate,
            crate::tokens::RADIUS,
            if hovered {
                fill.lerp_to_gamma(foreground, 0.12)
            } else {
                fill
            },
        );
        let mut text_area = rect.shrink2(Vec2::new(crate::tokens::SPACE_SM, 0.0));
        if grip {
            text_area.min.x = rect.left() + crate::tokens::COMPACT_HEIGHT;
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
        if !master && response.has_focus() {
            ui.painter().rect_stroke(
                plate,
                crate::tokens::RADIUS,
                egui::Stroke::new(crate::tokens::FOCUS_WIDTH, foreground.gamma_multiply(0.65)),
                egui::StrokeKind::Inside,
            );
        }
    }
    if !grip {
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name)
        });
    }
    if master {
        response.on_hover_text("Final stereo output · receives all channels routed to Master")
    } else {
        response
    }
}

pub struct MixerAddEffectResponse {
    pub insert: Response,
    pub eq: Response,
    pub comp: Response,
}

/// The row that ends every strip's Effects section: "Insert…" opens the
/// effect library, "EQ" and "Comp" add the built-in equalizer and compressor
/// and open it. The cells share the row's width, so each target spans its
/// part of it, with a muted dot between them.
pub fn mixer_add_effect_row(ui: &mut Ui, width: f32) -> MixerAddEffectResponse {
    let cells = [
        (
            "Insert…",
            "Add insert…",
            "Choose an effect for this channel",
        ),
        (
            "EQ",
            "Add EQ…",
            "Add R1 Equalizer to this channel and open it",
        ),
        (
            "Comp",
            "Add Comp…",
            "Add R1 Compressor to this channel and open it",
        ),
    ];
    let font = FontId::proportional(crate::tokens::FONT_CAPTION);
    let colors = crate::theme::palette(ui.ctx());
    let galleys = cells.map(|(label, _, _)| {
        ui.painter()
            .layout_no_wrap(label.to_owned(), font.clone(), colors.action)
    });
    let text: f32 = galleys.iter().map(|galley| galley.size().x).sum();
    let slack = ((width - text) / cells.len() as f32).max(0.0);
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, MIXER_SLOT_HEIGHT), Sense::hover());
    let mut left = row.left();
    let [insert, eq, comp] = std::array::from_fn(|index| {
        let (_, name, hint) = cells[index];
        let galley = galleys[index].clone();
        let rect = Rect::from_min_size(
            Pos2::new(left, row.top()),
            Vec2::new(
                (galley.size().x + slack).min(row.right() - left),
                row.height(),
            ),
        );
        left = rect.right();
        if index > 0 && ui.is_rect_visible(rect) {
            ui.painter()
                .circle_filled(Pos2::new(rect.left(), rect.center().y), 1.5, colors.muted);
        }
        let response = ui.interact(
            rect,
            ui.id().with(("mixer_add_effect", name)),
            Sense::click(),
        );
        if ui.is_rect_visible(rect) {
            let surface =
                egui::emath::GuiRounding::round_to_pixels(rect, ui.painter().pixels_per_point());
            if response.hovered() || response.has_focus() {
                ui.painter()
                    .rect_filled(surface, crate::tokens::RADIUS, colors.control_hover);
            }
            if response.has_focus() {
                ui.painter().rect_stroke(
                    surface,
                    crate::tokens::RADIUS,
                    egui::Stroke::new(crate::tokens::FOCUS_WIDTH, colors.accent),
                    egui::StrokeKind::Inside,
                );
            }
            ui.painter()
                .galley(rect.center() - galley.size() * 0.5, galley, colors.action);
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name)
        });
        crate::hover_text(response, hint)
    });
    MixerAddEffectResponse { insert, eq, comp }
}

/// The source row's caption after an input arrow: the kind plus the actual source, e.g. "Audio · In 1".
pub fn mixer_source_caption(ui: &mut Ui, text: &str, width: f32) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, mixer_control_height(ui)), Sense::hover());
    if ui.is_rect_visible(rect) {
        let color = crate::theme::palette(ui.ctx()).muted;
        let left = paint_leading_icon(
            ui,
            crate::Icon::Input,
            rect.left() + crate::tokens::SPACE_SM,
            rect.center().y,
        );
        let galley = one_line(
            ui,
            text,
            FontId::proportional(crate::tokens::FONT_CAPTION),
            color,
            rect.right() - left,
        );
        ui.painter().galley(
            Pos2::new(left, rect.center().y - galley.size().y * 0.5),
            galley,
            color,
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    response
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

/// The Effects header: a quiet row that opens the chain; "active/total" sits
/// at its trailing edge once the chain has inserts.
pub fn mixer_effects_button(ui: &mut Ui, width: f32, count: usize, active: usize) -> Response {
    let colors = crate::theme::palette(ui.ctx());
    let summary = format!("{active}/{count}");
    crate::hover_text(
        mixer_row(
            ui,
            width,
            mixer_control_height(ui),
            "Effects",
            MixerRowText::Body,
            None,
            (count > 0).then_some((
                summary.as_str(),
                if active > 0 {
                    colors.accent
                } else {
                    colors.muted
                },
            )),
        ),
        format!("{active} active / {count} inserts · before volume and pan"),
    )
}

/// One outlined analysis action, `height` square, for the measuring row.
pub fn mixer_analysis_action(ui: &mut Ui, height: f32) -> Response {
    let colors = crate::theme::palette(ui.ctx());
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = Vec2::ZERO;
            ui.add_sized(
                Vec2::splat(height),
                crate::IconButton::icon_only(crate::Icon::Waveform, "Analyze")
                    .min_size(Vec2::splat(height))
                    .small()
                    .quiet()
                    .stroke(egui::Stroke::new(
                        1.0 / ui.ctx().pixels_per_point(),
                        colors.line,
                    )),
            )
        })
        .inner;
    crate::hover_text(
        response,
        "Live spectrum, LUFS loudness, stereo correlation and phase",
    )
}

/// The outlined Effects action heading a strip's processing section.
pub fn mixer_processing_row(ui: &mut Ui, width: f32, count: usize, active: usize) -> Response {
    let effects = mixer_effects_button(ui, width, count, active);
    let colors = crate::theme::palette(ui.ctx());
    ui.painter().rect_stroke(
        effects.rect,
        crate::tokens::RADIUS,
        egui::Stroke::new(1.0 / ui.ctx().pixels_per_point(), colors.line),
        egui::StrokeKind::Inside,
    );
    effects
}

pub struct MixerMeasureResponse {
    pub statistics: Response,
    pub analyze: Response,
}

/// The RMS readout under the meters with the Analyze action at its end, so
/// deeper measurement sits next to the levels it explains.
pub fn mixer_measure_row(ui: &mut Ui, width: f32, rms: [f32; 2]) -> MixerMeasureResponse {
    let height = crate::tokens::COMPACT_HEIGHT;
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = crate::tokens::SPACE_XS;
            let statistics = crate::channel_meter_statistics(
                ui,
                (width - height - crate::tokens::SPACE_XS).max(0.0),
                rms,
            );
            let analyze = mixer_analysis_action(ui, height);
            MixerMeasureResponse {
                statistics,
                analyze,
            }
        },
    )
    .inner
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

#[derive(Clone, Debug)]
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
    /// Where the lane hops over other routes' stems (screen x).
    pub hops: Vec<f32>,
    pub source_clipped: bool,
    pub target_clipped: bool,
}

pub struct MixerRouteResponse {
    pub response: Response,
    pub painted: Option<MixerRoutePaint>,
}

/// Segments of a route's hop over another route's stem.
const HOP_STEPS: usize = 8;

/// One destination per lane. The continuous U-shaped connection starts at the
/// source strip, descends into its lane and rises to an arrowhead touching the
/// destination strip. Vertex colors interpolate along the complete route.
///
/// `connection_clip` is the channel pane's clip rectangle, before entering the
/// lane scroller: stems must remain attached above that scroller. Offscreen
/// endpoints retain their true positions and are clipped, never moved to another
/// channel. Each visible connection is one ribbon mesh (eight vertices, plus
/// eighteen per hop) and one triangle; there are no textures, per-pixel sampling, or extra repaint requests.
pub fn mixer_route_lane(
    ui: &mut Ui,
    source: MixerRouteEndpoint,
    target: MixerRouteEndpoint,
    caption: &str,
    connection_clip: Rect,
) -> MixerRouteResponse {
    mixer_route_lane_with_height(
        ui,
        source,
        target,
        caption,
        connection_clip,
        MIXER_ROUTE_LANE,
    )
}

/// A shorter routing overview uses the same connections and nested scrolling
/// in laptop workspaces, keeping the channel's real controls above the fold.
pub fn mixer_route_lane_with_height(
    ui: &mut Ui,
    source: MixerRouteEndpoint,
    target: MixerRouteEndpoint,
    caption: &str,
    connection_clip: Rect,
    height: f32,
) -> MixerRouteResponse {
    mixer_route_lane_with_crossings(ui, source, target, caption, connection_clip, height, &[])
}

/// [`mixer_route_lane_with_height`] whose horizontal run hops over the
/// vertical stems of other routes at `crossings` (screen x): the lines cross
/// there without connecting.
pub fn mixer_route_lane_with_crossings(
    ui: &mut Ui,
    source: MixerRouteEndpoint,
    target: MixerRouteEndpoint,
    caption: &str,
    connection_clip: Rect,
    height: f32,
    crossings: &[f32],
) -> MixerRouteResponse {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(
            ui.available_width(),
            height.max(crate::tokens::COMPACT_HEIGHT),
        ),
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
    let compact_lane = rect.height() < crate::tokens::CONTROL_HEIGHT;
    let turn_inset = if compact_lane {
        half
    } else {
        crate::tokens::SPACE_XS
    };
    let turn_y = (rect.bottom() - turn_inset).max(from.y.max(to.y) + head_height + half);
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
    // The centerline: down the source stem, along the lane (hopping over
    // other routes' stems), and up to the arrowhead's base.
    let mut line = vec![path[0], path[1]];
    let hop = crate::tokens::SPACE_SM;
    let mut hops: Vec<f32> = crossings
        .iter()
        .copied()
        .filter(|x| {
            x.is_finite()
                && *x > from.x.min(to.x) + hop + half
                && *x < from.x.max(to.x) - hop - half
        })
        .collect();
    hops.sort_by(|a, b| (direction * a).total_cmp(&(direction * b)));
    for x in hops.iter().copied() {
        line.extend((0..=HOP_STEPS).map(|step| {
            let angle = std::f32::consts::PI * step as f32 / HOP_STEPS as f32;
            Pos2::new(
                x - direction * hop * angle.cos(),
                turn_y - hop * angle.sin(),
            )
        }));
    }
    line.extend([path[2], to + Vec2::new(0.0, head_height)]);
    let mut distances = vec![0.0];
    for pair in line.windows(2) {
        distances.push(distances.last().unwrap() + pair[0].distance(pair[1]));
    }
    let total_length = distances.last().unwrap() + head_height;
    let normal = |a: Pos2, b: Pos2| {
        let along = (b - a).normalized();
        Vec2::new(-along.y, along.x)
    };
    let mut mesh = egui::Mesh::default();
    for (index, (position, distance)) in line.iter().zip(&distances).enumerate() {
        // Mitered joins share vertices, so no direction leaves hairline gaps.
        let before = (index > 0).then(|| normal(line[index - 1], *position));
        let after = line.get(index + 1).map(|next| normal(*position, *next));
        let offset = match (before, after) {
            (Some(a), Some(b)) => (a + b) / (1.0 + a.dot(b)).max(0.1),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => Vec2::ZERO,
        } * half;
        let color = source
            .color
            .lerp_to_gamma(target.color, distance / total_length);
        mesh.colored_vertex(*position + offset, color);
        mesh.colored_vertex(*position - offset, color);
    }
    for index in 0..line.len() as u32 - 1 {
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
    let caption_top = rect.top()
        + if compact_lane {
            0.0
        } else {
            crate::tokens::SPACE_XS
        };
    crate::clip_title(
        &ui.painter().with_clip_rect(visible),
        Rect::from_min_max(
            Pos2::new(caption_left, caption_top),
            Pos2::new(
                (caption_left + caption_width).min(visible.right() - crate::tokens::SPACE_SM),
                caption_top + crate::tokens::FONT_CAPTION + 3.0,
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
            hops,
            source_clipped,
            target_clipped,
        }),
    }
}

/// Signal-flow sections of a strip, top to bottom. They are told apart by
/// MIXER_SECTION_GAP alone; the names serve tooltips and diagnostics.
#[derive(Clone, Copy)]
pub enum MixerSection {
    Source,
    Effects,
    Level,
    Output,
}

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
}

/// Width of the legend column naming the strip sections, left of the strips.
pub const MIXER_LEGEND_WIDTH: f32 = crate::tokens::SPACE_XL + crate::tokens::SPACE_LG;

/// Names each strip section once, in a column left of the strips: every
/// strip shares the same rows, so a label per strip would only repeat. `rows`
/// pairs a name with the section's content rect in one strip.
pub fn paint_mixer_section_legend(ui: &Ui, column: Rect, rows: &[(&str, Rect)]) {
    let colors = crate::theme::palette(ui.ctx());
    let painter = ui.painter_at(column);
    for (name, section) in rows {
        painter.text(
            egui::pos2(column.right(), section.top()),
            Align2::RIGHT_TOP,
            *name,
            FontId::proportional(crate::tokens::FONT_CAPTION),
            colors.muted,
        );
    }
}

/// Quiet background bands behind a strip's sections (input, effects, pan,
/// level, output, sends). Space separates them first; the band only groups
/// what belongs together, without frames or rules. Reserve before the
/// controls so the bands paint beneath them.
pub struct MixerSectionBands(egui::layers::ShapeIdx);

impl MixerSectionBands {
    pub fn reserve(ui: &Ui) -> Self {
        Self(ui.painter().add(egui::Shape::Noop))
    }

    /// `gap` is the strip's section gap; a third of it pads each band, so
    /// the visible space between bands matches their padding.
    pub fn paint(self, ui: &Ui, sections: &[Rect], gap: f32) {
        let colors = crate::theme::palette(ui.ctx());
        let fill = colors.section;
        let shapes = sections
            .iter()
            .filter(|rect| rect.is_positive())
            .map(|rect| {
                let band = egui::emath::GuiRounding::round_to_pixels(
                    rect.expand(gap / 3.0),
                    ui.pixels_per_point(),
                );
                egui::Shape::rect_filled(band, crate::tokens::RADIUS, fill)
            })
            .collect();
        ui.painter().set(self.0, egui::Shape::Vec(shapes));
    }
}
