//! Calibrated native sliders with unambiguous handle centers and adaptive scales.
use egui::{
    Align2, Color32, FontId, Id, Pos2, Rect, Response, SliderClamping, SliderOrientation, Stroke,
    Ui, Widget, WidgetInfo, WidgetText,
};
use std::ops::RangeInclusive;

/// Landmarks shown inside a slider thumb, without changing its stored value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderValueMarker {
    Minimum,
    Maximum,
    Center,
    Zero,
}

/// Endpoints take precedence over zero; zero takes precedence over midpoint.
/// A tiny relative tolerance accommodates f32 parameters without inventing a
/// zero detent, or labelling a pinned out-of-range value as an endpoint.
pub fn slider_value_marker(value: f64, range: RangeInclusive<f64>) -> Option<SliderValueMarker> {
    value_marker_with_zero(value, range, 0.0)
}

fn value_marker_with_zero(
    value: f64,
    range: RangeInclusive<f64>,
    zero_reference: f64,
) -> Option<SliderValueMarker> {
    let (start, end) = (*range.start(), *range.end());
    if !value.is_finite() || !start.is_finite() || !end.is_finite() || start == end {
        return None;
    }
    let (minimum, maximum) = (start.min(end), start.max(end));
    if value < minimum || value > maximum {
        return None;
    }
    let matches = |landmark: f64| {
        (value - landmark).abs() <= f64::from(f32::EPSILON) * landmark.abs().min(maximum - minimum)
    };
    if matches(minimum) {
        Some(SliderValueMarker::Minimum)
    } else if matches(maximum) {
        Some(SliderValueMarker::Maximum)
    } else if value == zero_reference {
        Some(SliderValueMarker::Zero)
    } else if matches(minimum * 0.5 + maximum * 0.5) {
        Some(SliderValueMarker::Center)
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub struct SliderTick {
    pub fraction: f32,
    pub center: Pos2,
    pub major: bool,
}

/// Exact native handle travel. Endpoints describe handle centers, not rail edges.
#[derive(Clone, Debug)]
pub struct SliderScaleGeometry {
    pub rail: Rect,
    pub minimum: Pos2,
    pub maximum: Pos2,
    pub handle: Pos2,
    pub handle_half_extent: egui::Vec2,
    pub orientation: SliderOrientation,
    pub range: [f64; 2],
    pub value: f64,
    /// Numeric value represented as zero by the caller's displayed scale.
    pub zero_reference: f64,
    pub divisions: usize,
    pub ticks: Vec<SliderTick>,
}

impl SliderScaleGeometry {
    pub fn new(
        rail: Rect,
        orientation: SliderOrientation,
        shape: egui::style::HandleShape,
        range: RangeInclusive<f64>,
        value: f64,
    ) -> Self {
        let vertical = orientation == SliderOrientation::Vertical;
        let radius = if vertical {
            rail.width()
        } else {
            rail.height()
        } / 2.5;
        let along = match shape {
            egui::style::HandleShape::Circle => radius,
            egui::style::HandleShape::Rect { aspect_ratio } => radius * aspect_ratio,
        };
        let (minimum, maximum, handle_half_extent) = if vertical {
            (
                egui::pos2(rail.center().x, rail.bottom() - along),
                egui::pos2(rail.center().x, rail.top() + along),
                egui::vec2(radius, along),
            )
        } else {
            (
                egui::pos2(rail.left() + along, rail.center().y),
                egui::pos2(rail.right() - along, rail.center().y),
                egui::vec2(along, radius),
            )
        };
        let length = minimum.distance(maximum);
        let divisions = if length >= 200.0 {
            10
        } else if length >= 100.0 {
            5
        } else {
            2
        };
        let at = |fraction: f32| minimum.lerp(maximum, fraction);
        let mut ticks = (0..=divisions)
            .map(|index| {
                let fraction = index as f32 / divisions as f32;
                SliderTick {
                    fraction,
                    center: at(fraction),
                    major: index == 0 || index == divisions || fraction == 0.5,
                }
            })
            .collect::<Vec<_>>();
        if divisions == 5 {
            ticks.insert(
                3,
                SliderTick {
                    fraction: 0.5,
                    center: at(0.5),
                    major: true,
                },
            );
        }
        let (min, max) = (*range.start(), *range.end());
        let fraction = if min == max || !value.is_finite() {
            0.5
        } else {
            ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
        };
        Self {
            rail,
            minimum,
            maximum,
            handle: at(fraction),
            handle_half_extent,
            orientation,
            range: [min, max],
            value,
            zero_reference: 0.0,
            divisions,
            ticks,
        }
    }

    pub fn value_marker(&self) -> Option<SliderValueMarker> {
        value_marker_with_zero(
            self.value,
            self.range[0]..=self.range[1],
            self.zero_reference,
        )
    }
}

/// The calibrated rail belonging to a slider response, including focused value fields.
pub fn slider_scale_geometry(ctx: &egui::Context, id: Id) -> Option<SliderScaleGeometry> {
    ctx.data(|data| data.get_temp(id.with("slider_scale_geometry")))
}

/// Paint a value landmark inside the native thumb. Returns false for ordinary
/// values so callers may retain their usual index. This adds no hit targets.
pub fn paint_slider_value_marker(ui: &Ui, geometry: &SliderScaleGeometry) -> bool {
    let Some(marker) = geometry.value_marker() else {
        return false;
    };
    let radius = (geometry.handle_half_extent.min_elem() - crate::tokens::BORDER_WIDTH * 0.5)
        .clamp(0.0, crate::tokens::SPACE_XS);
    if radius <= 0.0 || !ui.is_rect_visible(geometry.rail) {
        return false;
    }
    let colors = crate::theme::palette(ui.ctx());
    let color = if ui.is_enabled() {
        colors.text
    } else {
        colors.muted
    };
    match marker {
        SliderValueMarker::Minimum | SliderValueMarker::Maximum => {
            let height = radius * crate::tokens::GOLDEN_RATIO_INVERSE;
            let direction = if marker == SliderValueMarker::Minimum {
                1.0
            } else {
                -1.0
            };
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    geometry.handle + egui::vec2(-radius, -direction * height),
                    geometry.handle + egui::vec2(radius, -direction * height),
                    geometry.handle + egui::vec2(0.0, direction * height),
                ],
                color,
                Stroke::NONE,
            ));
        }
        SliderValueMarker::Center => {
            ui.painter().circle_filled(
                geometry.handle,
                radius * crate::tokens::GOLDEN_RATIO_INVERSE,
                color,
            );
        }
        SliderValueMarker::Zero => {
            let width =
                crate::tokens::BORDER_WIDTH.min(radius * crate::tokens::GOLDEN_RATIO_INVERSE);
            ui.painter().circle_stroke(
                geometry.handle,
                radius - width * 0.5,
                Stroke::new(width, color),
            );
        }
    }
    true
}

/// Paint-only scale: marks never add hit targets or take keyboard focus.
pub fn paint_slider_scale(ui: &Ui, geometry: &SliderScaleGeometry) {
    if !ui.is_rect_visible(geometry.rail) {
        return;
    }
    let colors = crate::theme::palette(ui.ctx());
    let vertical = geometry.orientation == SliderOrientation::Vertical;
    let cross = if vertical {
        geometry.rail.width()
    } else {
        geometry.rail.height()
    } * 0.5;
    let marked = geometry.value_marker().is_some();
    for tick in &geometry.ticks {
        // At a landmark the thumb supplies the index. Keep a scale tick from
        // crossing its hollow circle on the smallest compact rails.
        if marked && tick.center.distance(geometry.handle) < 0.5 {
            continue;
        }
        let reach = if tick.major {
            cross - 1.0
        } else {
            (cross - 1.0).min(6.0)
        };
        // Keep every tick a visible stroke, even on the widest fader or
        // shortest compact rail. A proportional gap alone can collapse minor
        // marks into subpixel dots when the rail thickness changes.
        let inner = (cross * 0.38).min((reach - 2.5).max(0.0));
        let color = if tick.center.distance(geometry.handle) < 0.5 {
            colors.text
        } else {
            colors.muted
        };
        let offset = |distance| {
            if vertical {
                egui::vec2(distance, 0.0)
            } else {
                egui::vec2(0.0, distance)
            }
        };
        for sign in [-1.0, 1.0] {
            ui.painter().line_segment(
                [
                    tick.center + offset(sign * inner),
                    tick.center + offset(sign * reach),
                ],
                Stroke::new(1.0_f32, color),
            );
        }
    }
    let half = if vertical {
        geometry.handle_half_extent.x
    } else {
        geometry.handle_half_extent.y
    };
    let offset = if vertical {
        egui::vec2((half - 2.0).max(1.0), 0.0)
    } else {
        egui::vec2(0.0, (half - 2.0).max(1.0))
    };
    if !paint_slider_value_marker(ui, geometry) {
        ui.painter().line_segment(
            [geometry.handle - offset, geometry.handle + offset],
            Stroke::new(
                1.5_f32,
                if ui.is_enabled() {
                    colors.text
                } else {
                    colors.muted
                },
            ),
        );
    }
    // A pinned handle must not imply an out-of-range stored value equals the
    // endpoint. Show a directional overflow mark without changing that value.
    if geometry.value.is_finite() && geometry.range[0] != geometry.range[1] {
        let fraction =
            (geometry.value - geometry.range[0]) / (geometry.range[1] - geometry.range[0]);
        if !(0.0..=1.0).contains(&fraction) {
            let direction = (geometry.maximum - geometry.minimum).normalized()
                * if fraction < 0.0 { -1.0 } else { 1.0 };
            let along = if vertical {
                geometry.handle_half_extent.y
            } else {
                geometry.handle_half_extent.x
            };
            let radius = (along * 0.45).min(3.0);
            let center = geometry.handle + direction * (along - radius);
            let cross = egui::vec2(-direction.y, direction.x) * radius;
            ui.painter().add(egui::Shape::convex_polygon(
                [
                    center + direction * radius,
                    center - direction * radius + cross,
                    center - direction * radius - cross,
                ]
                .map(|point| point.clamp(geometry.rail.min, geometry.rail.max))
                .to_vec(),
                colors.orange,
                Stroke::NONE,
            ));
        }
    }
}

/// Shared slider builder. Rail interaction is egui's native Slider; its exact
/// numeric editor is the shared Number. Scale painting never writes values.
#[must_use = "Add the slider to a Ui"]
pub struct Slider<'a> {
    value: Box<dyn FnMut(Option<f64>) -> f64 + 'a>,
    range: RangeInclusive<f64>,
    integral: bool,
    clamping: SliderClamping,
    show_value: bool,
    scale_labels: bool,
    compact: bool,
    orientation: SliderOrientation,
    text: WidgetText,
    prefix: String,
    suffix: String,
    endpoint_labels: Option<[String; 2]>,
    current_label: Option<String>,
    zero_reference: f64,
    smart_aim: bool,
    step: Option<f64>,
    min_decimals: usize,
    max_decimals: Option<usize>,
    trailing_fill: Option<bool>,
    handle_shape: Option<egui::style::HandleShape>,
}

impl<'a> Slider<'a> {
    pub fn new<T: egui::emath::Numeric>(value: &'a mut T, range: RangeInclusive<T>) -> Self {
        Self {
            value: Box::new(move |next| {
                if let Some(next) = next {
                    *value = T::from_f64(next);
                }
                value.to_f64()
            }),
            range: range.start().to_f64()..=range.end().to_f64(),
            integral: T::INTEGRAL,
            clamping: SliderClamping::Always,
            show_value: true,
            scale_labels: true,
            compact: false,
            orientation: SliderOrientation::Horizontal,
            text: WidgetText::default(),
            prefix: String::new(),
            suffix: String::new(),
            endpoint_labels: None,
            current_label: None,
            zero_reference: 0.0,
            smart_aim: true,
            step: None,
            min_decimals: 0,
            max_decimals: None,
            trailing_fill: None,
            handle_shape: None,
        }
    }
    pub fn text(mut self, text: impl Into<WidgetText>) -> Self {
        self.text = text.into();
        self
    }
    pub fn prefix(mut self, prefix: impl ToString) -> Self {
        self.prefix = prefix.to_string();
        self
    }
    pub fn suffix(mut self, suffix: impl ToString) -> Self {
        self.suffix = suffix.to_string();
        self
    }
    pub fn show_value(mut self, show: bool) -> Self {
        self.show_value = show;
        self
    }
    pub fn scale_labels(mut self, show: bool) -> Self {
        self.scale_labels = show;
        self
    }
    pub fn endpoint_labels(
        mut self,
        minimum: impl Into<String>,
        maximum: impl Into<String>,
    ) -> Self {
        self.endpoint_labels = Some([minimum.into(), maximum.into()]);
        self
    }
    /// Short readout between the endpoints, for controls with an external exact editor.
    pub fn current_label(mut self, label: impl Into<String>) -> Self {
        self.current_label = Some(label.into());
        self
    }
    /// Identify zero on a converted scale, such as linear gain 1.0 at 0 dB.
    /// This changes only the landmark glyph, never the range or value mapping.
    pub fn zero_reference(mut self, value: f64) -> Self {
        self.zero_reference = value;
        self
    }
    /// A 24-point rail-and-caption control for canvas-owned rows.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    pub fn vertical(mut self) -> Self {
        self.orientation = SliderOrientation::Vertical;
        self
    }
    pub fn orientation(mut self, orientation: SliderOrientation) -> Self {
        self.orientation = orientation;
        self
    }
    pub fn clamping(mut self, clamping: SliderClamping) -> Self {
        self.clamping = clamping;
        self
    }
    pub fn smart_aim(mut self, smart: bool) -> Self {
        self.smart_aim = smart;
        self
    }
    pub fn step_by(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }
    pub fn fixed_decimals(mut self, count: usize) -> Self {
        self.min_decimals = count;
        self.max_decimals = Some(count);
        self
    }
    pub fn min_decimals(mut self, count: usize) -> Self {
        self.min_decimals = count;
        self
    }
    pub fn max_decimals(mut self, count: usize) -> Self {
        self.max_decimals = Some(count);
        self
    }
    pub fn trailing_fill(mut self, fill: bool) -> Self {
        self.trailing_fill = Some(fill);
        self
    }
    pub fn handle_shape(mut self, shape: egui::style::HandleShape) -> Self {
        self.handle_shape = Some(shape);
        self
    }

    fn caption(&self, value: f64) -> String {
        let detail = value
            .abs()
            .min((*self.range.end() - *self.range.start()).abs());
        let adaptive = if detail > 0.0 && detail < 0.01 {
            (-detail.log10()).ceil() as usize + 1
        } else {
            2
        };
        let decimals = if self.integral {
            0
        } else {
            self.max_decimals
                .unwrap_or(adaptive)
                .max(self.min_decimals)
                .min(12)
        };
        let mut number = if (value != 0.0 && value.abs() < 1e-9) || value.abs() >= 1e9 {
            crate::exact_value_text(value)
        } else if self.range.start() != self.range.end()
            && format!("{:.decimals$}", self.range.start())
                == format!("{:.decimals$}", self.range.end())
        {
            // Even an explicitly coarse edit precision must not make distinct
            // endpoints look identical.
            crate::exact_value_text(value)
        } else {
            format!("{value:.decimals$}")
        };
        if number.contains('.') && !number.contains('e') {
            let minimum_length = number.find('.').unwrap() + 1 + self.min_decimals;
            while number.ends_with('0') && number.len() > minimum_length {
                number.pop();
            }
            if number.ends_with('.') {
                number.pop();
            }
        }
        format!("{}{}{}", self.prefix, number, self.suffix)
    }

    fn add_rail(&mut self, ui: &mut Ui) -> (Response, SliderScaleGeometry) {
        let orientation = self.orientation;
        let shape = self.handle_shape.unwrap_or(ui.visuals().handle_shape);
        let thickness = if self.compact {
            (ui.available_height().min(crate::tokens::COMPACT_HEIGHT) - 12.0).clamp(8.0, 12.0)
        } else if orientation == SliderOrientation::Vertical {
            crate::tokens::CONTROL_HEIGHT - crate::tokens::SPACE_XS
        } else {
            crate::tokens::COMPACT_HEIGHT
        };
        let text_height = if self.compact {
            crate::tokens::FONT_SCALE
        } else {
            crate::tokens::FONT_CANVAS
        };
        let mut labels = self.endpoint_labels.clone().unwrap_or_else(|| {
            [
                self.caption(*self.range.start()),
                self.caption(*self.range.end()),
            ]
        });
        let displayed = (self.value)(None);
        let start = *self.range.start();
        let end = *self.range.end();
        if displayed < start.min(end) {
            let index = usize::from(start > end);
            labels[index] = format!("< {}", labels[index]);
        } else if displayed > start.max(end) {
            let index = usize::from(start < end);
            labels[index] = format!("> {}", labels[index]);
        }
        let vertical = orientation == SliderOrientation::Vertical;
        let aligned_horizontal = !vertical && !self.compact;
        let caption_height = if self.compact {
            12.0
        } else if aligned_horizontal {
            (crate::tokens::PARAMETER_ROW_HEIGHT - thickness) * 0.5
        } else {
            14.0
        };
        let rail_padding = if aligned_horizontal {
            if self.scale_labels {
                caption_height
            } else {
                (crate::tokens::CONTROL_HEIGHT - thickness) * 0.5
            }
        } else {
            0.0
        };
        let minimum_width = labels
            .iter()
            .map(|label| {
                ui.painter()
                    .layout_no_wrap(
                        label.clone(),
                        FontId::proportional(text_height),
                        Color32::WHITE,
                    )
                    .size()
                    .x
            })
            .fold(thickness, f32::max);
        let response = ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Balance the endpoint footer with space above the rail. Its
            // center then matches the Number, label and neighboring controls,
            // while the complete control retains one comfortable hit target.
            if aligned_horizontal {
                ui.add_space(rail_padding);
            }
            if vertical && self.scale_labels {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(minimum_width, caption_height),
                    egui::Sense::hover(),
                );
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    &labels[1],
                    FontId::proportional(text_height),
                    crate::theme::palette(ui.ctx()).muted,
                );
            }
            let rail_response = ui
                .scope(|ui| {
                    ui.spacing_mut().interact_size.y = thickness;
                    // Native rail thickness is max(body font, interact size). Compact
                    // rails use the canvas font, keeping their entire scale in24px.
                    ui.style_mut().text_styles.insert(
                        egui::TextStyle::Body,
                        FontId::proportional(if self.compact { thickness * 0.8 } else { 15.0 }),
                    );
                    let pointer_edit = ui.is_enabled()
                        && ui.input(|input| {
                            ![egui::PointerButton::Secondary, egui::PointerButton::Middle]
                                .into_iter()
                                .any(|button| {
                                    input.pointer.button_down(button)
                                        || input.pointer.button_released(button)
                                })
                        });
                    let mut native = egui::Slider::from_get_set(self.range.clone(), |next| {
                        (self.value)(next.filter(|_| pointer_edit))
                    })
                    .orientation(orientation)
                    .show_value(false)
                    .clamping(self.clamping)
                    .smart_aim(self.smart_aim)
                    .min_decimals(self.min_decimals)
                    .handle_shape(shape);
                    if self.integral {
                        native = native.integer();
                    }
                    if let Some(step) = self.step {
                        native = native.step_by(step);
                    }
                    if let Some(max) = self.max_decimals {
                        native = native.max_decimals(max);
                    }
                    if let Some(fill) = self.trailing_fill {
                        native = native.trailing_fill(fill);
                    }
                    let native = ui.add(native);
                    // The native slider consists solely of its rail here. Register
                    // that same identity and rectangle as click-and-drag so a
                    // secondary click can open its caller's context menu. No extra
                    // overlapping widget or independent event handler is created.
                    let mut response =
                        ui.interact(native.rect, native.id, egui::Sense::click_and_drag());
                    if native.changed() {
                        response.mark_changed();
                    }
                    response
                })
                .inner;
            let value = (self.value)(None);
            let mut geometry = SliderScaleGeometry::new(
                rail_response.rect,
                orientation,
                shape,
                self.range.clone(),
                value,
            );
            geometry.zero_reference = self.zero_reference;
            paint_slider_scale(ui, &geometry);
            if self.scale_labels {
                let width = if vertical {
                    minimum_width
                } else {
                    rail_response.rect.width()
                };
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, caption_height), egui::Sense::hover());
                let colors = crate::theme::palette(ui.ctx());
                if vertical {
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        &labels[0],
                        FontId::proportional(text_height),
                        colors.muted,
                    );
                } else {
                    ui.painter().text(
                        rect.left_center(),
                        Align2::LEFT_CENTER,
                        &labels[0],
                        FontId::proportional(text_height),
                        colors.muted,
                    );
                    ui.painter().text(
                        rect.right_center(),
                        Align2::RIGHT_CENTER,
                        &labels[1],
                        FontId::proportional(text_height),
                        colors.muted,
                    );
                    if let Some(current) = &self.current_label {
                        let galley = ui.painter().layout_no_wrap(
                            current.clone(),
                            FontId::proportional(text_height),
                            colors.text,
                        );
                        // The caller chooses concise readouts; still avoid endpoint collisions.
                        let left_width = ui
                            .painter()
                            .layout_no_wrap(
                                labels[0].clone(),
                                FontId::proportional(text_height),
                                colors.muted,
                            )
                            .size()
                            .x;
                        let right_width = ui
                            .painter()
                            .layout_no_wrap(
                                labels[1].clone(),
                                FontId::proportional(text_height),
                                colors.muted,
                            )
                            .size()
                            .x;
                        let available = rect.width() - left_width - right_width - 6.0;
                        let center = egui::pos2(
                            (rect.left() + left_width + rect.right() - right_width) * 0.5,
                            rect.center().y,
                        );
                        if galley.size().x <= available {
                            ui.painter()
                                .galley(center - galley.size() * 0.5, galley, colors.text);
                        }
                    }
                }
            } else if aligned_horizontal {
                ui.add_space(rail_padding);
            }
            (rail_response, geometry)
        });
        let (rail, geometry) = response.inner;
        // The caption belongs to the same control: preserve a comfortable
        // pointer target while the calibrated rail itself stays visually slim.
        let mut full = ui.interact(
            rail.rect.union(response.response.rect),
            rail.id,
            egui::Sense::click_and_drag(),
        );
        if rail.changed() {
            full.mark_changed();
        }
        (full, geometry)
    }

    fn add_contents(&mut self, ui: &mut Ui) -> Response {
        let (mut response, mut geometry) = self.add_rail(ui);
        let value_before_number = (self.value)(None);
        if self.show_value {
            let mut value = value_before_number;
            let length = geometry.minimum.distance(geometry.maximum).max(1.0);
            let speed = self.step.unwrap_or_else(|| {
                ((*self.range.end() - *self.range.start()).abs() / f64::from(length))
                    .max(if self.integral { 1.0 } else { 0.0001 })
            });
            let mut number = crate::number(&mut value)
                .speed(speed)
                .min_decimals(self.min_decimals)
                .max_decimals_opt(if self.integral {
                    Some(0)
                } else {
                    self.max_decimals
                })
                .prefix(&self.prefix)
                .suffix(&self.suffix);
            match self.clamping {
                SliderClamping::Never => {}
                SliderClamping::Edits => {
                    number = number
                        .range(self.range.clone())
                        .clamp_existing_to_range(false)
                }
                SliderClamping::Always => number = number.range(self.range.clone()),
            }
            let numeric = ui.add(number);
            if numeric.changed() && ui.is_enabled() {
                // Match native Slider::set_value before committing to the
                // model, including integer rounding and step anchoring.
                if self.clamping != SliderClamping::Never {
                    value = value.clamp(
                        self.range.start().min(*self.range.end()),
                        self.range.start().max(*self.range.end()),
                    );
                }
                if let Some(step) = self.step.or(self.integral.then_some(1.0)) {
                    value =
                        *self.range.start() + ((value - *self.range.start()) / step).round() * step;
                }
                if let Some(decimals) = self.max_decimals.or(self.integral.then_some(0)) {
                    value = egui::emath::round_to_decimals(value, decimals);
                }
                (self.value)(Some(value));
            }
            if numeric.has_focus() || numeric.gained_focus() || numeric.lost_focus() {
                response = numeric.union(response);
            } else {
                response = response.union(numeric);
            }
        }
        if !self.text.is_empty() {
            let label =
                ui.add(egui::Label::new(self.text.clone()).wrap_mode(egui::TextWrapMode::Extend));
            response = response.labelled_by(label.id);
        }
        let actual = (self.value)(None);
        response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), actual, self.text.text()));
        if actual != geometry.value {
            geometry.value = actual;
            let fraction = if geometry.range[0] == geometry.range[1] || !actual.is_finite() {
                0.5
            } else {
                ((actual - geometry.range[0]) / (geometry.range[1] - geometry.range[0]))
                    .clamp(0.0, 1.0) as f32
            };
            geometry.handle = geometry.minimum.lerp(geometry.maximum, fraction);
        }
        ui.ctx()
            .data_mut(|data| data.insert_temp(response.id.with("slider_scale_geometry"), geometry));
        crate::hover_text(
            response,
            format!(
                "{}{}{} · {} to {}\n{}",
                self.prefix,
                actual,
                self.suffix,
                self.caption(*self.range.start()),
                self.caption(*self.range.end()),
                if actual < self.range.start().min(*self.range.end()) {
                    "Below the displayed range · the stored value is preserved"
                } else if actual > self.range.start().max(*self.range.end()) {
                    "Above the displayed range · the stored value is preserved"
                } else {
                    "The handle's center line marks the value"
                }
            ),
        )
    }
}

impl Widget for Slider<'_> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        ui.scope(|ui| {
            if self.compact {
                ui.spacing_mut().interact_size.y = 0.0;
            }
            let response = match self.orientation {
                SliderOrientation::Horizontal if !self.compact => {
                    let height = if self.scale_labels {
                        crate::tokens::PARAMETER_ROW_HEIGHT
                    } else {
                        crate::tokens::CONTROL_HEIGHT
                    };
                    let layout = if ui.layout().prefer_right_to_left() {
                        egui::Layout::right_to_left(egui::Align::Center)
                    } else {
                        egui::Layout::left_to_right(egui::Align::Center)
                    };
                    // Reserve the complete height before the parent centers
                    // this child. Growing a native 34-point row afterward
                    // would leave its rail below neighboring control centers.
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_size_before_wrap().x, height),
                        layout,
                        |ui| {
                            ui.set_min_height(height);
                            ui.set_row_height(height);
                            self.add_contents(ui)
                        },
                    )
                }
                SliderOrientation::Horizontal => ui.horizontal(|ui| self.add_contents(ui)),
                SliderOrientation::Vertical => ui.vertical(|ui| self.add_contents(ui)),
            };
            response.inner | response.response
        })
        .inner
    }
}
