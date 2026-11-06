//! Quiet mix controls for arranger headers. Native sliders own editing; the
//! header owns geometry, channel state, automation and its separate level meter.
use crate::{SliderScaleGeometry, theme, tokens};
use egui::{Color32, FontId, Id, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::{hash::Hash, ops::RangeInclusive};

pub struct TrackMixResponse {
    pub gain: Response,
    pub pan: Response,
    pub mute: Response,
    pub solo: Response,
    pub gain_slider: Rect,
    pub pan_slider: Rect,
    pub gain_readout: Rect,
    pub pan_readout: Rect,
    pub gain_scale: Option<SliderScaleGeometry>,
    pub pan_scale: Option<SliderScaleGeometry>,
}

/// A concise visual readout; exact entry and automation retain the stored value.
pub fn track_pan_caption(pan: f32) -> String {
    let percent = (pan * 100.0).round();
    if percent == 0.0 {
        "C".into()
    } else {
        format!(
            "{}{:.0}",
            if percent < 0.0 { "L" } else { "R" },
            percent.abs()
        )
    }
}

/// Draw a 42-point mix section, or readable value fields in a short track. The
/// captions open deliberate exact entry; dragging and arrows retain native
/// editing. Mute and Solo are commands returned to the host, never local state.
/// The caller reserves at least 179 points for the complete section.
pub fn track_mix_controls_at(
    ui: &mut Ui,
    rect: Rect,
    id_salt: impl Hash,
    gain: &mut f32,
    pan: &mut f32,
    mute: bool,
    solo: bool,
) -> TrackMixResponse {
    track_mix_controls_at_named(ui, rect, id_salt, gain, pan, mute, solo, "Channel")
}

/// The same header controls with contextual names in both compact and expanded
/// exact editors. Presentation never changes parameter identity or precision.
#[allow(clippy::too_many_arguments)] // Backward-compatible extension of track_mix_controls_at.
pub fn track_mix_controls_at_named(
    ui: &mut Ui,
    rect: Rect,
    id_salt: impl Hash,
    gain: &mut f32,
    pan: &mut f32,
    mute: bool,
    solo: bool,
    channel_name: &str,
) -> TrackMixResponse {
    let id = ui.make_persistent_id(id_salt);
    let compact = rect.height() < tokens::COMPACT_HEIGHT * 2.0;
    if compact {
        return compact_mix_at(ui, rect, id, gain, pan, [mute, solo], channel_name);
    }
    let geometry = mix_geometry(rect);
    let gain_rect = geometry.gain;
    let pan_rect = geometry.pan;
    let caption_height = tokens::COMPACT_HEIGHT;
    let caption_rect = |rect: Rect| rect.with_max_y(rect.top() + caption_height);
    let rail_rect = |rect: Rect| rect.with_min_y(rect.top() + caption_height);
    let mut db = crate::channel_gain_to_db(*gain);
    let gain_caption = if *gain <= 0.0 {
        "−∞ dB".into()
    } else {
        format!("{db:.1} dB")
    };
    let gain_range = -60.0..=f64::from(20.0 * 1.5_f32.log10());
    let (gain_rail, mut gain_scale) = rail_at(
        ui,
        rail_rect(gain_rect),
        id.with("gain_slider"),
        &mut db,
        gain_range.clone(),
        "Channel volume",
    );
    let gain_readout = readout_at(
        ui,
        caption_rect(gain_rect),
        id.with("gain_value"),
        ("Volume", &gain_caption),
        &mut db,
        gain_range,
        "dB",
        &format!("{channel_name} · Volume"),
    );
    let mut gain_response = gain_rail.union(gain_readout.clone());
    if ui.is_enabled() && gain_response.changed() {
        *gain = crate::channel_gain_from_db(db);
    }
    update_geometry(&mut gain_scale, f64::from(db));
    gain_response = crate::hover_text(
        gain_response,
        format!(
            "Channel volume · {db} dB\nDrag the rail or click the value. Right-click for exact value entry and automation"
        ),
    );

    let mut percent = *pan * 100.0;
    let pan_caption = track_pan_caption(*pan);
    let (pan_rail, mut pan_scale) = rail_at(
        ui,
        rail_rect(pan_rect),
        id.with("pan_slider"),
        &mut percent,
        -100.0..=100.0,
        "Channel pan",
    );
    let pan_readout = readout_at(
        ui,
        caption_rect(pan_rect),
        id.with("pan_value"),
        ("Pan", &pan_caption),
        &mut percent,
        -100.0..=100.0,
        "%",
        &format!("{channel_name} · Pan"),
    );
    let mut pan_response = pan_rail.union(pan_readout.clone());
    if ui.is_enabled() && pan_response.changed() {
        *pan = percent / 100.0;
    }
    update_geometry(&mut pan_scale, f64::from(percent));
    pan_response = crate::hover_text(
        pan_response,
        format!(
            "Channel pan · {percent}%\n−100 left · 0 center · 100 right\nDrag the rail or click the value. Right-click for automation"
        ),
    );

    let mute = state_button_at(ui, geometry.mute, id.with("mute"), mute, "M", "Mute track");
    let solo = state_button_at(ui, geometry.solo, id.with("solo"), solo, "S", "Solo track");
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            gain_response.id.with("slider_scale_geometry"),
            gain_scale.clone(),
        );
        data.insert_temp(
            pan_response.id.with("slider_scale_geometry"),
            pan_scale.clone(),
        );
    });
    TrackMixResponse {
        gain: gain_response,
        pan: pan_response,
        mute,
        solo,
        gain_slider: gain_scale.rail,
        pan_slider: pan_scale.rail,
        gain_readout: gain_readout.rect,
        pan_readout: pan_readout.rect,
        gain_scale: Some(gain_scale),
        pan_scale: Some(pan_scale),
    }
}

struct MixGeometry {
    gain: Rect,
    pan: Rect,
    mute: Rect,
    solo: Rect,
}

/// Equal gain and pan columns, then square Mute and Solo centered on the rail
/// line (or on the row in a compact header) so all four read as one row.
fn mix_geometry(rect: Rect) -> MixGeometry {
    let gap = tokens::SPACE_MD;
    let button = tokens::COMPACT_HEIGHT.min(rect.height());
    let toggles_width = button * 2.0 + tokens::SPACE_XS;
    let parameter_width = (rect.width() - toggles_width - gap).max(0.0);
    let column = ((parameter_width - gap) * 0.5).max(0.0);
    let gain_rect = Rect::from_min_size(rect.min, egui::vec2(column, rect.height()));
    let pan_rect = gain_rect.translate(egui::vec2(column + gap, 0.0));
    let center_y = if rect.height() >= tokens::COMPACT_HEIGHT * 2.0 {
        rect.top() + tokens::COMPACT_HEIGHT + (rect.height() - tokens::COMPACT_HEIGHT) * 0.5
    } else {
        rect.center().y
    };
    let mute_rect = Rect::from_center_size(
        egui::pos2(rect.right() - toggles_width + button * 0.5, center_y),
        Vec2::splat(button),
    );
    let solo_rect = mute_rect.translate(egui::vec2(button + tokens::SPACE_XS, 0.0));
    MixGeometry {
        gain: gain_rect,
        pan: pan_rect,
        mute: mute_rect,
        solo: solo_rect,
    }
}

/// Mute and Solo for a row without gain or pan, such as a folder. The pair
/// sits at the right of `rect`, vertically centered, the same as on channels.
pub fn track_state_toggles_at(
    ui: &mut Ui,
    rect: Rect,
    id_salt: impl Hash,
    mute: bool,
    solo: bool,
) -> [Response; 2] {
    let id = ui.make_persistent_id(id_salt);
    let geometry = mix_geometry(rect);
    [
        state_button_at(ui, geometry.mute, id.with("mute"), mute, "M", "Mute track"),
        state_button_at(ui, geometry.solo, id.with("solo"), solo, "S", "Solo track"),
    ]
}

/// Width of the Mute and Solo pair, for callers that reserve its slot.
pub const TRACK_STATE_TOGGLES_WIDTH: f32 = tokens::COMPACT_HEIGHT * 2.0 + tokens::SPACE_XS;

#[derive(Clone, Copy)]
enum MixParameter {
    Gain,
    Pan,
}

impl MixParameter {
    fn range(self) -> RangeInclusive<f64> {
        match self {
            Self::Gain => -60.0..=f64::from(20.0 * 1.5_f32.log10()),
            Self::Pan => -100.0..=100.0,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Gain => "Channel volume",
            Self::Pan => "Channel pan",
        }
    }
    fn unit(self) -> &'static str {
        match self {
            Self::Gain => "dB",
            Self::Pan => "%",
        }
    }
    fn step(self) -> f64 {
        match self {
            Self::Gain => 0.1,
            Self::Pan => 1.0,
        }
    }
    fn caption(self, value: f32) -> String {
        match self {
            Self::Gain if value == -60.0 => "−∞ dB".into(),
            Self::Gain => format!("{value:.1} dB"),
            Self::Pan => track_pan_caption(value / 100.0),
        }
    }
}

fn compact_mix_at(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    gain: &mut f32,
    pan: &mut f32,
    states: [bool; 2],
    channel_name: &str,
) -> TrackMixResponse {
    let geometry = mix_geometry(rect);
    let mut db = crate::channel_gain_to_db(*gain);
    let caption = if *gain <= 0.0 {
        "−∞ dB".into()
    } else {
        format!("{db:.1} dB")
    };
    let gain_response = compact_value_at(
        ui,
        geometry.gain,
        id.with("gain_value"),
        &mut db,
        MixParameter::Gain,
        &caption,
        &format!("{channel_name} · Volume"),
    );
    if gain_response.changed() && ui.is_enabled() {
        *gain = crate::channel_gain_from_db(db);
    }
    let mut percent = *pan * 100.0;
    let pan_response = compact_value_at(
        ui,
        geometry.pan,
        id.with("pan_value"),
        &mut percent,
        MixParameter::Pan,
        &track_pan_caption(*pan),
        &format!("{channel_name} · Pan"),
    );
    if pan_response.changed() && ui.is_enabled() {
        *pan = percent / 100.0;
    }
    let mute = state_button_at(
        ui,
        geometry.mute,
        id.with("mute"),
        states[0],
        "M",
        "Mute track",
    );
    let solo = state_button_at(
        ui,
        geometry.solo,
        id.with("solo"),
        states[1],
        "S",
        "Solo track",
    );
    TrackMixResponse {
        gain_slider: gain_response.rect,
        pan_slider: pan_response.rect,
        gain_readout: gain_response.rect,
        pan_readout: pan_response.rect,
        gain: gain_response,
        pan: pan_response,
        mute,
        solo,
        gain_scale: None,
        pan_scale: None,
    }
}

/// A full-size value field uses native slider focus, keys and accessibility,
/// with relative dragging and an explicit exact-entry popup. A plain click must
/// never jump to an invisible position on a rail or clamp a stored quiet gain.
fn compact_value_at(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    value: &mut f32,
    parameter: MixParameter,
    caption: &str,
    name: &str,
) -> Response {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child.spacing_mut().slider_width = rect.width();
    child.spacing_mut().interact_size.y = rect.height();
    child.spacing_mut().item_spacing = Vec2::ZERO;
    child.style_mut().text_styles.insert(
        egui::TextStyle::Body,
        FontId::proportional(tokens::FONT_CAPTION),
    );
    let background = child.painter().add(egui::Shape::Noop);
    let previous_style = child.style().clone();
    let visuals = &mut child.visuals_mut().widgets;
    for visual in [
        &mut visuals.noninteractive,
        &mut visuals.inactive,
        &mut visuals.hovered,
        &mut visuals.active,
        &mut visuals.open,
    ] {
        visual.bg_fill = Color32::TRANSPARENT;
        visual.weak_bg_fill = Color32::TRANSPARENT;
        visual.bg_stroke = Stroke::NONE;
        visual.fg_stroke = Stroke::NONE;
    }
    let drag_id = id.with("relative_drag");
    let (pressed, released, down, secondary, dragging, origin, position, fine) =
        child.input(|input| {
            (
                input.pointer.button_pressed(egui::PointerButton::Primary),
                input.pointer.button_released(egui::PointerButton::Primary),
                input.pointer.button_down(egui::PointerButton::Primary),
                [egui::PointerButton::Secondary, egui::PointerButton::Middle]
                    .into_iter()
                    .any(|button| {
                        input.pointer.button_down(button) || input.pointer.button_released(button)
                    }),
                input.pointer.is_decidedly_dragging(),
                input.pointer.press_origin(),
                input.pointer.interact_pos(),
                input.modifiers.shift,
            )
        });
    if pressed && let Some(origin) = origin.filter(|origin| rect.contains(*origin)) {
        child.data_mut(|data| data.insert_temp(drag_id, (*value, origin)));
    }
    let range = parameter.range();
    let relative_value = child
        .data(|data| data.get_temp::<(f32, egui::Pos2)>(drag_id))
        .zip(position)
        .filter(|_| down && dragging && !secondary)
        .map(|((start, origin), at)| {
            let movement = at - origin;
            let speed = parameter.step() * if fine { 0.1 } else { 1.0 };
            (f64::from(start) + f64::from(movement.x - movement.y) * speed)
                .clamp(*range.start(), *range.end())
        });
    let keyboard_edit = !down && !pressed && !released && !secondary;
    let editable = child.is_enabled() && !egui::Popup::is_any_open(child.ctx());
    let event_index = child.output(|output| output.events.len());
    let native = child.add(
        egui::Slider::from_get_set(range.clone(), |next| {
            if editable
                && let Some(next) = next
                && let Some(next) = relative_value.or(keyboard_edit.then_some(next))
            {
                *value = next as f32;
            }
            f64::from(*value)
        })
        .show_value(false)
        .clamping(egui::SliderClamping::Edits)
        .step_by(parameter.step())
        .smart_aim(false)
        .trailing_fill(false),
    );
    child.set_style(previous_style);
    let mut response = child.interact(native.rect, native.id, Sense::click_and_drag());
    if native.changed() {
        response.mark_changed();
    }
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    crate::parameter_focus::register_parameter_history_focus(&response);
    if released {
        child.data_mut(|data| data.remove::<(f32, egui::Pos2)>(drag_id));
    }
    child.output_mut(|output| {
        if output.events.get(event_index).is_some_and(|event| {
            let info = event.widget_info();
            info.typ == egui::WidgetType::Slider
                && info.label.as_deref().unwrap_or_default().is_empty()
        }) {
            output.events.remove(event_index);
        }
    });
    response.widget_info(|| {
        egui::WidgetInfo::slider(child.is_enabled(), f64::from(*value), parameter.label())
    });
    let colors = theme::palette(ui.ctx());
    child.painter().set(
        background,
        crate::component_shape(
            child.painter(),
            rect,
            tokens::RADIUS,
            if response.hovered() || response.dragged() {
                colors.control_hover
            } else {
                colors.bg
            },
            Stroke::new(
                tokens::BORDER_WIDTH,
                if response.has_focus() {
                    colors.accent
                } else {
                    colors.line
                },
            ),
            crate::ComponentMaterial::default(),
        ),
    );
    let displayed = if response.changed() {
        parameter.caption(*value)
    } else {
        caption.into()
    };
    let text = child.painter().layout_no_wrap(
        displayed,
        FontId::proportional(tokens::FONT_CAPTION),
        colors.text,
    );
    let position = egui::pos2(
        (rect.center().x - text.size().x * 0.5).max(rect.left() + 3.0),
        rect.center().y - text.size().y * 0.5,
    );
    child.painter().galley(position, text, colors.text);
    let popup = egui::Popup::menu(&response)
        .id(id.with("exact_value_popup"))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            crate::value_entry_named(ui, id, f64::from(*value), range, parameter.unit(), name)
        });
    if let Some(next) = popup.and_then(|popup| popup.inner)
        && ui.is_enabled()
        && *value != next as f32
    {
        *value = next as f32;
        response.mark_changed();
    }
    crate::hover_text(
        response,
        format!(
            "{} · {} {}\nDrag to adjust; click for exact entry. Right-click for automation",
            parameter.label(),
            value,
            parameter.unit()
        ),
    )
}

fn update_geometry(geometry: &mut SliderScaleGeometry, value: f64) {
    geometry.value = value;
    let fraction = ((value - geometry.range[0]) / (geometry.range[1] - geometry.range[0]))
        .clamp(0.0, 1.0) as f32;
    geometry.handle = geometry.minimum.lerp(geometry.maximum, fraction);
}

fn rail_at(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    value: &mut f32,
    range: RangeInclusive<f64>,
    label: &str,
) -> (Response, SliderScaleGeometry) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child.spacing_mut().slider_width = rect.width();
    child.spacing_mut().interact_size.y = rect.height();
    child.spacing_mut().item_spacing = Vec2::ZERO;
    child.style_mut().text_styles.insert(
        egui::TextStyle::Body,
        FontId::proportional((rect.height() * tokens::GOLDEN_RATIO_INVERSE).max(1.0)),
    );
    // Native Slider owns focus, accessibility actions and value mapping. Its
    // decorative frame is replaced with the shared slider rail and thumb.
    let background = child.painter().add(egui::Shape::Noop);
    let previous_style = child.style().clone();
    let visuals = &mut child.visuals_mut().widgets;
    for visual in [
        &mut visuals.noninteractive,
        &mut visuals.inactive,
        &mut visuals.hovered,
        &mut visuals.active,
        &mut visuals.open,
    ] {
        visual.bg_fill = Color32::TRANSPARENT;
        visual.weak_bg_fill = Color32::TRANSPARENT;
        visual.bg_stroke = Stroke::NONE;
        visual.fg_stroke = Stroke::NONE;
    }
    let pointer_edit = child.is_enabled()
        && child.input(|input| {
            ![egui::PointerButton::Secondary, egui::PointerButton::Middle]
                .into_iter()
                .any(|button| {
                    input.pointer.button_down(button) || input.pointer.button_released(button)
                })
        });
    let shape = egui::style::HandleShape::Rect {
        aspect_ratio: tokens::GOLDEN_RATIO_INVERSE * 1.25,
    };
    let event_index = child.output(|output| output.events.len());
    let native = child.add(
        egui::Slider::from_get_set(range.clone(), |next| {
            if let Some(next) = next.filter(|_| pointer_edit) {
                *value = next as f32;
            }
            f64::from(*value)
        })
        .show_value(false)
        .clamping(egui::SliderClamping::Edits)
        .smart_aim(false)
        .handle_shape(shape)
        .trailing_fill(false),
    );
    child.set_style(previous_style);
    let mut response = child.interact(native.rect, native.id, Sense::click_and_drag());
    if native.changed() {
        response.mark_changed();
    }
    crate::parameter_focus::register_parameter_history_focus(&response);
    child.output_mut(|output| {
        if output.events.get(event_index).is_some_and(|event| {
            let info = event.widget_info();
            info.typ == egui::WidgetType::Slider
                && info.label.as_deref().unwrap_or_default().is_empty()
        }) {
            output.events.remove(event_index);
        }
    });
    response.widget_info(|| egui::WidgetInfo::slider(child.is_enabled(), f64::from(*value), label));
    let mut geometry = SliderScaleGeometry::new(
        native.rect,
        egui::SliderOrientation::Horizontal,
        shape,
        range,
        f64::from(*value),
    );
    geometry.ticks.clear();
    geometry.divisions = 0;
    // Header rails share the same trough, rectangular thumb and center point
    // as Mixer, channel controls and ordinary parameter sliders.
    child.painter().set(background, egui::Shape::Noop);
    crate::paint_slider_rail(&child, &response, &geometry);
    (response, geometry)
}

#[allow(clippy::too_many_arguments)] // Keep canvas bounds separate from parameter and contextual display metadata.
fn readout_at(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    label_and_value: (&str, &str),
    value: &mut f32,
    range: RangeInclusive<f64>,
    unit: &str,
    name: &str,
) -> Response {
    let (label, caption) = label_and_value;
    let compact = rect.height() < tokens::COMPACT_HEIGHT;
    let mut response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("{label} · {caption}"),
        )
    });
    let colors = theme::palette(ui.ctx());
    let painter = ui.painter_at(rect);
    if response.hovered() || response.has_focus() {
        painter.add(crate::component_shape(
            &painter,
            rect,
            tokens::RADIUS,
            colors.control_hover,
            if response.has_focus() {
                Stroke::new(tokens::BORDER_WIDTH, colors.accent)
            } else {
                Stroke::NONE
            },
            crate::ComponentMaterial::default(),
        ));
    }
    // The value carries its unit; the parameter name stays in the accessible
    // label and tooltip so adjacent columns never read as one phrase.
    let font = FontId::proportional(if compact {
        tokens::FONT_SCALE
    } else {
        tokens::FONT_CAPTION
    });
    crate::text::paint_single_line(&painter, rect, font, colors.text, caption);
    let popup = egui::Popup::menu(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| crate::value_entry_named(ui, id, f64::from(*value), range, unit, name));
    if let Some(next) = popup.and_then(|popup| popup.inner)
        && ui.is_enabled()
        && *value != next as f32
    {
        *value = next as f32;
        response.mark_changed();
    }
    response.on_hover_text(format!(
        "{label} · {} {unit}\nClick to enter exact value",
        value
    ))
}

fn state_button_at(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    selected: bool,
    glyph: &str,
    label: &str,
) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), selected, label)
    });
    let colors = theme::palette(ui.ctx());
    let painter = ui.painter_at(rect);
    let active = if glyph == "S" {
        colors.solo
    } else {
        colors.mute
    };
    let fill = if selected {
        active
    } else if response.hovered() {
        colors.control_hover
    } else {
        colors.bg
    };
    painter.add(crate::component_shape(
        &painter,
        rect,
        tokens::RADIUS,
        fill,
        if selected {
            Stroke::NONE
        } else {
            Stroke::new(
                tokens::BORDER_WIDTH,
                if response.has_focus() {
                    colors.accent
                } else {
                    colors.line
                },
            )
        },
        crate::ComponentMaterial {
            elevated: false,
            active: !selected && response.has_focus() && response.enabled(),
        },
    ));
    if selected && response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            tokens::RADIUS,
            Stroke::new(tokens::FOCUS_WIDTH, colors.accent),
            egui::StrokeKind::Outside,
        );
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        crate::text::TextRole::Label.font(),
        if selected {
            theme::contrast_text(active)
        } else {
            active
        },
    );
    response.on_hover_text(label)
}
