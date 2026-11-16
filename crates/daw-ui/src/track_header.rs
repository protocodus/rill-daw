//! A deliberate, bounded track header: identity, mix, connections and metering.
//! The arranger owns actions and song state; these controls own only geometry
//! and presentation. Compact headers keep a route to the complete channel view.
use crate::{Icon, text, theme, tokens};
use egui::{Color32, Painter, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::hash::Hash;

pub const TRACK_HEADER_WIDTH: f32 = 233.0 + 55.0;
pub const TRACK_HEADER_HEIGHT: f32 = 144.0;
pub const TRACK_HEADER_COMPACT_HEIGHT: f32 = 89.0;
/// A deliberately collapsed row keeps just its identity and disclosure.
pub const TRACK_HEADER_COLLAPSED_HEIGHT: f32 = tokens::COMPACT_HEIGHT;
/// A folder row shows only its identity line: name, Mute, Solo and actions.
pub const TRACK_HEADER_FOLDER_HEIGHT: f32 = tokens::CONTROL_HEIGHT + tokens::SPACE_SM;

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
    pub reorder: Option<Rect>,
    pub disclosure: Option<Rect>,
    pub bus_badge: Option<Rect>,
    pub record_arm: Option<Rect>,
    pub effects: Option<Rect>,
    pub state_toggles: Option<Rect>,
    pub mix: Option<Rect>,
    pub meter: Option<Rect>,
    pub routing: Option<Rect>,
    pub device: Option<Rect>,
    pub details: Option<Rect>,
    pub compact: bool,
    collapsed: bool,
}

impl TrackHeaderLayout {
    pub fn new(rect: Rect) -> Self {
        Self::layout(rect, false)
    }

    pub fn collapsed(rect: Rect) -> Self {
        Self::layout(rect, true)
    }

    fn layout(rect: Rect, collapsed: bool) -> Self {
        let rect = if rect.is_finite() && rect.is_positive() {
            rect
        } else {
            Rect::from_min_size(egui::Pos2::ZERO, Vec2::ZERO)
        };
        let vertical_padding = if collapsed {
            0.0
        } else if rect.height() >= TRACK_HEADER_COMPACT_HEIGHT {
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
        // Keep the normal grip easy to see without stealing the minimum
        // legible title width from the compact header's action slots.
        let reorder = (!collapsed && identity.width() >= tokens::SPACE_XL * 2.0).then(|| {
            end_slot(
                identity,
                if rect.width() >= 223.0 {
                    tokens::COMPACT_HEIGHT
                } else {
                    tokens::SPACE_MD
                },
            )
        });
        let disclosure = start_slot(
            identity,
            if rect.width() >= 168.0 {
                tokens::COMPACT_HEIGHT
            } else {
                tokens::SPACE_MD
            },
        );
        let icon_left = (disclosure.right() + tokens::SPACE_XS).min(identity.right());
        let icon = start_slot(identity.with_min_x(icon_left), tokens::SPACE_MD);
        let end = reorder.map_or(identity.right(), |grip| grip.left() - tokens::SPACE_SM);
        let name_left = (icon.right() + tokens::SPACE_SM).min(identity.right());
        let name = Rect::from_min_max(
            egui::pos2(name_left, identity.top()),
            egui::pos2(end.max(name_left), identity.bottom()),
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
        let body_fits = !collapsed
            && body_right - left >= 179.0
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
        // Without a body the identity line is the whole row: center it.
        let (identity, disclosure, icon, name, reorder) = if mix.is_none() {
            let shift = egui::vec2(0.0, rect.center().y - identity.center().y);
            (
                identity.translate(shift),
                disclosure.translate(shift),
                icon.translate(shift),
                name.translate(shift),
                reorder.map(|rect| rect.translate(shift)),
            )
        } else {
            (identity, disclosure, icon, name, reorder)
        };
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
            disclosure: disclosure.is_positive().then_some(disclosure),
            bus_badge: None,
            record_arm: None,
            effects: None,
            state_toggles: None,
            mix,
            meter,
            routing,
            device,
            details: None,
            compact: !expanded,
            collapsed,
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

    /// Expanded channels reserve an effects button when their identity can fit it.
    pub fn with_effects(mut self) -> Self {
        if self.identity.height() >= tokens::COMPACT_HEIGHT {
            self.effects = self.reserve_name_end(tokens::COMPACT_HEIGHT);
        }
        self
    }

    /// Mute and Solo on the identity line, for rows without a mix section.
    pub fn with_state_toggles(mut self) -> Self {
        if self.identity.height() >= tokens::COMPACT_HEIGHT {
            self.state_toggles = self.reserve_name_end(crate::TRACK_STATE_TOGGLES_WIDTH);
        }
        self
    }

    pub fn with_bus_badge(mut self) -> Self {
        self.bus_badge = self.reserve_name_end(tokens::SPACE_XL);
        self
    }

    /// Hierarchy affects identity only. A shared right-hand indentation limit
    /// prevents deeply nested folder titles from appearing to the right of
    /// their children when their different action slots start to constrain text.
    pub fn with_depth(mut self, depth: usize) -> Self {
        let possible_actions = self.reorder.map_or(0.0, |rect| rect.width())
            + tokens::SPACE_SM
            + tokens::COMPACT_HEIGHT
            + tokens::SPACE_SM
            + tokens::SPACE_XL
            + tokens::SPACE_SM;
        // All kinds reserve the same disclosure, icon and title prefix.
        let common_limit =
            (self.identity.right() - possible_actions - tokens::SPACE_LG).max(self.name.left());
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
        if self.collapsed || self.name.width() < width + tokens::SPACE_SM + tokens::SPACE_LG {
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
}

/// Add editable name, color and icon rows to a track properties form.
pub fn track_identity_fields<T: Copy + PartialEq>(
    ui: &mut Ui,
    name: &mut String,
    color: &mut [u8; 3],
    icon: &mut Option<T>,
    choices: &[(T, &str, Icon)],
    default_icon: Icon,
    focus_name: bool,
) -> bool {
    let label = crate::form_label(ui, "Name");
    let field = crate::form_text_field(ui, name, "Track name").labelled_by(label.id);
    if focus_name {
        field.request_focus();
        crate::widgets::select_field_text(ui, field.id, name);
    }
    let mut changed = field.changed();
    ui.end_row();

    let label = crate::form_label(ui, "Color");
    changed |= crate::color_edit(ui, color).labelled_by(label.id).changed();
    ui.end_row();

    let label = crate::form_label(ui, "Icon");
    let selected = choices.iter().find(|(value, _, _)| Some(*value) == *icon);
    let (caption, glyph) = selected.map_or(("Track type", default_icon), |(_, label, glyph)| {
        (*label, *glyph)
    });
    crate::control_row(ui, |ui| {
        crate::form_select("track_icon", caption)
            .show_ui(ui, |ui| {
                changed |= crate::selectable_value(ui, icon, None, "Track type").changed();
                for &(value, label, _) in choices {
                    changed |= crate::selectable_value(ui, icon, Some(value), label).changed();
                }
            })
            .response
            .labelled_by(label.id);
        let (preview, _) = ui.allocate_exact_size(Vec2::splat(tokens::SPACE_LG), Sense::hover());
        glyph.paint(
            ui.painter(),
            preview,
            theme::readable_track_color(ui.ctx(), *color),
        );
    });
    ui.end_row();
    changed
}

/// Identity stays a single legible line. The host reuses these bounds for name
/// selection, reordering, double-click and context actions.
pub fn paint_track_header_identity(
    painter: &Painter,
    layout: &TrackHeaderLayout,
    identity: TrackHeaderIdentity<'_>,
) -> Rect {
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(layout.identity));
    identity.icon.paint(
        &painter,
        centered_icon(layout.icon, tokens::SPACE_MD),
        identity.tint,
    );
    text::paint_row(&painter, layout.name, text::TextRole::Label, identity.name);
    layout.name
}

/// Independent button for the row's disclosure. Its pointer and keyboard
/// targets remain separate from track identity editing and reordering.
pub fn track_header_disclosure_at(
    ui: &mut Ui,
    rect: Rect,
    id: impl Hash,
    collapsed: bool,
    label: &str,
) -> Response {
    let glyph = if collapsed {
        Icon::ChevronRight
    } else {
        Icon::ChevronDown
    };
    let tint = theme::palette(ui.ctx()).text;
    track_header_action_at(ui, rect, id, glyph, label, tint)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
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
    let muted = theme::palette(ui.ctx()).muted;
    track_header_action_at(ui, rect, id_salt, Icon::Settings, "Channel controls", muted)
        .on_hover_text("Open channel controls, routing and device")
}

/// An icon-only action in a slot reserved on the identity line.
pub fn track_header_action_at(
    ui: &mut Ui,
    rect: Rect,
    id_salt: impl Hash,
    icon: Icon,
    label: &str,
    tint: Color32,
) -> Response {
    let response = ui.interact(rect, ui.make_persistent_id(id_salt), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
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
        icon.paint(&painter, centered_icon(rect, tokens::SPACE_MD), tint);
    }
    response
}

fn centered_icon(rect: Rect, size: f32) -> Rect {
    Rect::from_center_size(
        rect.center(),
        Vec2::splat(size.min(rect.width()).min(rect.height()).max(0.0)),
    )
}

/// The track-list menu above the headers: "Tracks ▾", left-aligned so its
/// text starts at the track-icon column, and centered on `rect`'s height.
/// `text_left` is that column; the hover surface extends a little before it.
pub fn track_list_menu_at<R>(
    ui: &mut Ui,
    rect: Rect,
    text_left: f32,
    id_salt: impl Hash,
    label: &str,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<egui::InnerResponse<R>>) {
    let colors = theme::palette(ui.ctx());
    let font = text::TextRole::Label.font();
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), colors.text);
    let chevron = tokens::SPACE_MD;
    let target = Rect::from_min_size(
        egui::pos2(
            text_left - tokens::SPACE_XS,
            rect.center().y - tokens::CONTROL_HEIGHT * 0.5,
        ),
        Vec2::new(
            tokens::SPACE_XS * 2.0 + galley.size().x + tokens::SPACE_XS + chevron,
            tokens::CONTROL_HEIGHT,
        ),
    )
    .intersect(rect);
    let response = ui.interact(target, ui.make_persistent_id(id_salt), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let painter = ui.painter_at(target);
    if response.hovered() || response.has_focus() || open {
        painter.add(crate::component_shape(
            &painter,
            target,
            tokens::RADIUS,
            colors.control_hover,
            if response.has_focus() {
                Stroke::new(tokens::FOCUS_WIDTH, colors.accent)
            } else {
                Stroke::NONE
            },
            crate::ComponentMaterial::default(),
        ));
    }
    let text_rect = Rect::from_min_size(
        egui::pos2(text_left, target.center().y - galley.size().y * 0.5),
        galley.size(),
    );
    painter.galley(text_rect.min, galley, colors.text);
    Icon::ChevronDown.paint(
        &painter,
        Rect::from_center_size(
            egui::pos2(
                text_rect.right() + tokens::SPACE_XS + chevron * 0.5,
                target.center().y,
            ),
            Vec2::splat(chevron),
        ),
        colors.muted,
    );
    let inner = egui::Popup::menu(&response).show(|ui| crate::command_menu_contents(ui, contents));
    (response, inner)
}
