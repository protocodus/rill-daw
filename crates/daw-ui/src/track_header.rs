//! A deliberate, bounded track header: identity, mix, connections and metering.
//! The arranger owns actions and song state; these controls own only geometry
//! and presentation. Compact headers keep a route to the complete channel view.
use crate::{Icon, text, theme, tokens};
use egui::{Color32, Painter, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::hash::Hash;

pub const TRACK_HEADER_WIDTH: f32 = 233.0 + 55.0;
pub const TRACK_HEADER_HEIGHT: f32 = 144.0;
pub const TRACK_HEADER_COMPACT_HEIGHT: f32 = 89.0;

/// All rectangles remain inside the caller's row. Optional sections disappear
/// together when their controls cannot fit, instead of shrinking their labels
/// and targets into overlapping fragments.
#[derive(Clone, Copy, Debug)]
pub struct TrackHeaderLayout {
    pub rect: Rect,
    pub color: Rect,
    pub identity: Rect,
    pub icon: Rect,
    pub name: Rect,
    pub reorder: Rect,
    pub disclosure: Option<Rect>,
    pub bus_badge: Option<Rect>,
    pub record_arm: Option<Rect>,
    pub mix: Option<Rect>,
    pub meter: Option<Rect>,
    pub routing: Option<Rect>,
    pub device: Option<Rect>,
    pub details: Option<Rect>,
    pub compact: bool,
}

impl TrackHeaderLayout {
    pub fn new(rect: Rect) -> Self {
        let rect = if rect.is_finite() && rect.is_positive() {
            rect
        } else {
            Rect::from_min_size(egui::Pos2::ZERO, Vec2::ZERO)
        };
        let vertical_padding = if rect.height() >= TRACK_HEADER_COMPACT_HEIGHT {
            tokens::SPACE_SM
        } else if rect.height() >= tokens::CONTROL_HEIGHT * 2.0 {
            tokens::SPACE_XS
        } else {
            3.0
        }
        .min(rect.height() * 0.5);
        let color_width = tokens::SPACE_SM.min(rect.width());
        let color = rect.with_max_x(rect.left() + color_width);
        let left = (rect.left() + color_width + tokens::SPACE_SM).min(rect.right());
        let right = (rect.right() - tokens::SPACE_SM).max(left);
        let identity = Rect::from_min_size(
            egui::pos2(left, rect.top() + vertical_padding),
            egui::vec2(
                right - left,
                tokens::COMPACT_HEIGHT.min((rect.height() - vertical_padding * 2.0).max(0.0)),
            ),
        );
        let reorder = end_slot(identity, tokens::SPACE_MD);
        let icon = start_slot(identity, tokens::SPACE_MD);
        let name_left = (icon.right() + tokens::SPACE_SM).min(reorder.left());
        let name_right = (reorder.left() - tokens::SPACE_SM).max(name_left);
        let name = Rect::from_min_max(
            egui::pos2(name_left, identity.top()),
            egui::pos2(name_right, identity.bottom()),
        );
        let (meter_width, meter_gap) = if rect.width() >= 233.0 {
            (tokens::SPACE_LG, tokens::SPACE_SM)
        } else {
            (tokens::SPACE_MD, tokens::SPACE_XS)
        };
        let body_right = (right - meter_width - meter_gap).max(left);
        let mix_height = if rect.height() >= TRACK_HEADER_COMPACT_HEIGHT {
            tokens::COMPACT_HEIGHT * 2.0
        } else {
            tokens::COMPACT_HEIGHT
        };
        let mix_top = identity.bottom()
            + if rect.height() >= TRACK_HEADER_COMPACT_HEIGHT {
                tokens::SPACE_SM
            } else {
                tokens::SPACE_XS
            };
        let body_fits = body_right - left >= 179.0
            && rect.height() >= 56.0
            && mix_top + mix_height <= rect.bottom() - vertical_padding;
        let mix = body_fits.then(|| {
            Rect::from_min_size(
                egui::pos2(left, mix_top),
                egui::vec2(body_right - left, mix_height),
            )
        });
        let expanded = body_fits && rect.height() >= TRACK_HEADER_HEIGHT;
        let routing = expanded.then(|| {
            Rect::from_min_size(
                egui::pos2(left, mix_top + mix_height + tokens::SPACE_MD),
                egui::vec2(body_right - left, tokens::COMPACT_HEIGHT),
            )
        });
        let device = routing.map(|routing| {
            routing.translate(egui::vec2(0.0, tokens::COMPACT_HEIGHT + tokens::SPACE_XS))
        });
        let meter = mix.map(|mix| {
            Rect::from_min_max(
                egui::pos2(body_right + meter_gap, mix.top()),
                egui::pos2(right, device.map_or(mix.bottom(), |device| device.bottom())),
            )
        });
        let mut layout = Self {
            rect,
            color,
            identity,
            icon,
            name,
            reorder,
            disclosure: None,
            bus_badge: None,
            record_arm: None,
            mix,
            meter,
            routing,
            device,
            details: None,
            compact: !expanded,
        };
        if !expanded && identity.height() >= tokens::SPACE_MD {
            layout.details = layout.reserve_name_end(tokens::COMPACT_HEIGHT);
        }
        layout
    }

    /// Source channels reserve a distinct arm target beside their identity.
    pub fn with_record_arm(mut self) -> Self {
        if self.identity.height() >= tokens::COMPACT_HEIGHT {
            self.record_arm = self.reserve_name_end(tokens::COMPACT_HEIGHT);
        }
        self
    }

    pub fn with_bus_badge(mut self) -> Self {
        self.bus_badge = self.reserve_name_end(tokens::SPACE_XL);
        self
    }

    pub fn with_folder_disclosure(mut self) -> Self {
        if self.name.width() >= tokens::SPACE_LG + tokens::SPACE_MD + tokens::SPACE_XS {
            self.disclosure = Some(self.icon);
            self.icon = self
                .icon
                .translate(egui::vec2(tokens::SPACE_MD + tokens::SPACE_XS, 0.0));
            self.name.min.x = self.icon.right() + tokens::SPACE_SM;
        }
        self
    }

    /// Hierarchy affects identity only. A shared right-hand indentation limit
    /// prevents deeply nested folder titles from appearing to the right of
    /// their children when their different action slots start to constrain text.
    pub fn with_depth(mut self, depth: usize) -> Self {
        let possible_actions = tokens::SPACE_MD
            + tokens::SPACE_SM
            + tokens::COMPACT_HEIGHT
            + tokens::SPACE_SM
            + tokens::SPACE_XL
            + tokens::SPACE_SM;
        // The related disclosure/icon pair uses a five-point gap; the title
        // keeps eight. Its extra slot is narrower than one hierarchy step,
        // so a child title remains visibly indented beyond its folder title.
        let folder_name_start = self.identity.left()
            + tokens::SPACE_MD
            + tokens::SPACE_XS
            + tokens::SPACE_MD
            + tokens::SPACE_SM;
        let common_limit =
            (self.identity.right() - possible_actions - tokens::SPACE_LG).max(folder_name_start);
        let name_limit = (self.name.right() - tokens::SPACE_LG).min(common_limit);
        let shift =
            (depth.min(32) as f32 * tokens::SPACE_LG).min((name_limit - self.name.left()).max(0.0));
        self.icon = self.icon.translate(egui::vec2(shift, 0.0));
        self.name.min.x += shift;
        self.disclosure = self
            .disclosure
            .map(|rect| rect.translate(egui::vec2(shift, 0.0)));
        self
    }

    /// Folder and automation rows retain their identity and control area,
    /// without implying that they own an audio output or instrument.
    pub fn without_channel_sections(mut self) -> Self {
        self.meter = None;
        self.routing = None;
        self.device = None;
        self.compact = true;
        if self.details.is_none() && self.identity.height() >= tokens::SPACE_MD {
            self.details = self.reserve_name_end(tokens::COMPACT_HEIGHT);
        }
        self
    }

    fn reserve_name_end(&mut self, width: f32) -> Option<Rect> {
        if self.name.width() < width + tokens::SPACE_SM + tokens::SPACE_LG {
            return None;
        }
        let slot = end_slot(self.name, width);
        self.name.max.x = slot.left() - tokens::SPACE_SM;
        Some(slot)
    }
}

fn start_slot(rect: Rect, width: f32) -> Rect {
    rect.with_max_x((rect.left() + width).min(rect.right()))
}

fn end_slot(rect: Rect, width: f32) -> Rect {
    rect.with_min_x((rect.right() - width).max(rect.left()))
}

pub struct TrackHeaderIdentity<'a> {
    pub name: &'a str,
    pub icon: Icon,
    pub tint: Color32,
    pub folder: bool,
    pub collapsed: bool,
}

/// Identity stays a single legible line. The host reuses these bounds for name
/// selection, reordering, double-click and context actions.
pub fn paint_track_header_identity(
    painter: &Painter,
    layout: &TrackHeaderLayout,
    identity: TrackHeaderIdentity<'_>,
) -> Rect {
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(layout.identity));
    if let Some(disclosure) = layout.disclosure.filter(|_| identity.folder) {
        (if identity.collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        })
        .paint(
            &painter,
            centered_icon(disclosure, tokens::SPACE_MD),
            theme::palette(painter.ctx()).muted,
        );
    }
    identity.icon.paint(
        &painter,
        centered_icon(layout.icon, tokens::SPACE_MD),
        identity.tint,
    );
    text::paint_row(&painter, layout.name, text::TextRole::Label, identity.name);
    layout.name
}

/// Fine section rules clarify the vertical order without boxing every control.
/// The meter owns a separate column and never shares a slider's value gutter.
pub fn paint_track_header_sections(painter: &Painter, layout: &TrackHeaderLayout) {
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(layout.rect));
    let stroke = Stroke::new(tokens::BORDER_WIDTH, theme::palette(painter.ctx()).line);
    if let Some(mix) = layout.mix {
        painter.hline(
            layout.identity.x_range(),
            (layout.identity.bottom() + mix.top()) * 0.5,
            stroke,
        );
        if let Some(routing) = layout.routing {
            painter.hline(mix.x_range(), (mix.bottom() + routing.top()) * 0.5, stroke);
        }
        if let Some(meter) = layout.meter {
            painter.vline((mix.right() + meter.left()) * 0.5, meter.y_range(), stroke);
        }
    }
}

/// A bounded section row with a stable label column and a readable destination.
/// The full section and value remain available to accessibility and hover help.
/// Native pointer and keyboard activation are retained without advancing layout.
pub fn track_header_link_at(
    ui: &mut Ui,
    rect: Rect,
    id_salt: impl Hash,
    icon: Icon,
    section: &str,
    value: &str,
) -> Response {
    let response = ui.interact(rect, ui.make_persistent_id(id_salt), Sense::click());
    let accessible = format!("{section}: {value}");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &accessible)
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let colors = theme::palette(ui.ctx());
        if response.hovered() || response.has_focus() || response.is_pointer_button_down_on() {
            let visuals = ui.style().interact(&response);
            painter.add(crate::component_shape(
                &painter,
                rect,
                tokens::RADIUS,
                visuals.weak_bg_fill,
                if response.has_focus() {
                    Stroke::new(tokens::FOCUS_WIDTH, colors.accent)
                } else {
                    Stroke::NONE
                },
                crate::ComponentMaterial {
                    elevated: false,
                    active: response.has_focus(),
                },
            ));
        }
        let inner = rect.shrink2(egui::vec2(tokens::SPACE_XS, 0.0));
        let icon_rect = start_slot(inner, tokens::SPACE_MD);
        icon.paint(
            &painter,
            centered_icon(icon_rect, tokens::SPACE_MD),
            colors.muted,
        );
        let chevron = end_slot(inner, tokens::SPACE_MD);
        Icon::ChevronRight.paint(
            &painter,
            centered_icon(chevron, tokens::SPACE_MD),
            colors.muted,
        );
        let label_left = icon_rect.right() + tokens::SPACE_SM;
        let label_width = (tokens::CONTROL_HEIGHT * 2.0)
            .min((inner.width() * (1.0 - tokens::GOLDEN_RATIO_INVERSE)).max(0.0));
        let label_rect = Rect::from_min_max(
            egui::pos2(label_left, inner.top()),
            egui::pos2(
                (label_left + label_width).min(chevron.left()),
                inner.bottom(),
            ),
        );
        text::paint_single_line(
            &painter,
            label_rect,
            egui::FontId::proportional(tokens::FONT_CAPTION),
            colors.muted,
            section,
        );
        let value_left = (label_rect.right() + tokens::SPACE_SM).min(chevron.left());
        let value_right = (chevron.left() - tokens::SPACE_SM).max(value_left);
        text::paint_single_line(
            &painter,
            Rect::from_min_max(
                egui::pos2(value_left, inner.top()),
                egui::pos2(value_right, inner.bottom()),
            ),
            egui::FontId::proportional(tokens::FONT_CAPTION),
            colors.text,
            value,
        );
    }
    response.on_hover_text(accessible)
}

/// Compact rows keep a visible route to the complete channel controls instead
/// of packing routing and device names into the remaining title space.
pub fn track_header_details_at(ui: &mut Ui, rect: Rect, id_salt: impl Hash) -> Response {
    let response = ui.interact(rect, ui.make_persistent_id(id_salt), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            "Channel controls",
        )
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let colors = theme::palette(ui.ctx());
        if response.hovered() || response.has_focus() || response.is_pointer_button_down_on() {
            let visuals = ui.style().interact(&response);
            painter.add(crate::component_shape(
                &painter,
                rect,
                tokens::RADIUS,
                visuals.weak_bg_fill,
                if response.has_focus() {
                    Stroke::new(tokens::FOCUS_WIDTH, colors.accent)
                } else {
                    Stroke::NONE
                },
                crate::ComponentMaterial {
                    elevated: false,
                    active: response.has_focus(),
                },
            ));
        }
        Icon::Settings.paint(
            &painter,
            centered_icon(rect, tokens::SPACE_MD),
            colors.muted,
        );
    }
    response.on_hover_text("Open channel controls, routing and device")
}

fn centered_icon(rect: Rect, size: f32) -> Rect {
    Rect::from_center_size(
        rect.center(),
        Vec2::splat(size.min(rect.width()).min(rect.height()).max(0.0)),
    )
}
