//! Practical rotary controls: a calibrated arc, a relative drag and an exact
//! numeric editor share one value. This module owns no application state.
use crate::{AutomationState, text, theme, tokens};
use egui::{Id, Pos2, Rect, Response, SliderClamping, Stroke, Ui, Vec2, Widget, WidgetInfo};
use std::{ops::RangeInclusive, sync::OnceLock};

const ARC_STEPS: usize = 60;
const START_ANGLE: f32 = std::f32::consts::PI * 0.75;
const ARC_ANGLE: f32 = std::f32::consts::PI * 1.5;

fn arc_samples() -> &'static [Vec2; ARC_STEPS + 1] {
    static SAMPLES: OnceLock<[Vec2; ARC_STEPS + 1]> = OnceLock::new();
    SAMPLES.get_or_init(|| {
        std::array::from_fn(|index| {
            let angle = START_ANGLE + ARC_ANGLE * index as f32 / ARC_STEPS as f32;
            Vec2::angled(angle)
        })
    })
}

fn number_text(
    value: f64,
    minimum_decimals: usize,
    maximum_decimals: Option<usize>,
    integral: bool,
) -> String {
    if integral {
        format!("{value:.0}")
    } else if let Some(maximum) = maximum_decimals {
        let mut text = format!("{value:.maximum$}");
        if maximum > minimum_decimals {
            let keep = text.find('.').map_or(text.len(), |dot| {
                dot + minimum_decimals + usize::from(minimum_decimals > 0)
            });
            while text.len() > keep && text.ends_with('0') {
                text.pop();
            }
            if text.ends_with('.') {
                text.pop();
            }
        }
        text
    } else {
        let mut text = value.to_string();
        if minimum_decimals > 0 && !text.contains(['e', 'E']) {
            let present = text.find('.').map_or(0, |dot| text.len() - dot - 1);
            if present < minimum_decimals {
                if !text.contains('.') {
                    text.push('.');
                }
                text.extend(std::iter::repeat_n('0', minimum_decimals - present));
            }
        }
        text
    }
}

#[derive(Clone, Debug)]
pub struct KnobGeometry {
    pub dial: Rect,
    pub number: Rect,
    pub center: Pos2,
    pub radius: f32,
    pub minimum: Pos2,
    pub maximum: Pos2,
    /// End of the value arc, and tip of the index on detailed dials.
    pub index: Pos2,
    pub range: [f64; 2],
    pub value: f64,
    pub divisions: usize,
    pub ticks: Vec<Pos2>,
    /// The painted unity landmark for perceptual gain controls.
    pub unity: Option<Pos2>,
}

pub fn knob_geometry(ctx: &egui::Context, id: Id) -> Option<KnobGeometry> {
    ctx.data(|data| data.get_temp(id.with("knob_geometry")))
}

#[derive(Clone, Copy, Default)]
struct DragState {
    value: f64,
    position: Option<Pos2>,
}

#[must_use = "Add the knob to a Ui"]
pub struct Knob<'a> {
    value: Box<dyn FnMut(Option<f64>) -> f64 + 'a>,
    range: RangeInclusive<f64>,
    integral: bool,
    text: String,
    suffix: String,
    endpoint_labels: Option<[String; 2]>,
    diameter: f32,
    min_decimals: usize,
    max_decimals: Option<usize>,
    step: Option<f64>,
    clamping: SliderClamping,
    automation: Option<AutomationState>,
    show_automation_label: bool,
    compact: bool,
    perceptual_gain: bool,
    display_round_trip: Option<Box<dyn Fn(f64) -> f64 + 'a>>,
}

pub fn knob<T: egui::emath::Numeric>(value: &mut T, range: RangeInclusive<T>) -> Knob<'_> {
    Knob::new(value, range)
}

impl<'a> Knob<'a> {
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
            text: String::new(),
            suffix: String::new(),
            endpoint_labels: None,
            diameter: 84.0,
            min_decimals: 0,
            max_decimals: None,
            step: None,
            clamping: SliderClamping::Edits,
            automation: None,
            show_automation_label: true,
            compact: false,
            perceptual_gain: false,
            display_round_trip: None,
        }
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }
    pub fn suffix(mut self, suffix: impl ToString) -> Self {
        self.suffix = suffix.to_string();
        self
    }
    /// Predict the display value the owner will return after accepting an edit,
    /// including its storage type and unit conversion. This preserves native
    /// drafts across an exact model round trip without tolerating real changes.
    /// Called only when this control requests a write; idle input stays exact.
    pub fn display_round_trip(mut self, convert: impl Fn(f64) -> f64 + 'a) -> Self {
        self.display_round_trip = Some(Box::new(convert));
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
    pub fn diameter(mut self, diameter: f32) -> Self {
        if diameter.is_finite() {
            self.diameter = diameter.clamp(48.0, 180.0);
        }
        self
    }
    /// A text-free ring. The caller provides its label, unit-aware hover,
    /// readout, and exact-value context action.
    /// Hidden numeric geometry is `Rect::NOTHING`. The default knob is unchanged.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    /// Full compact target height. Rings use a square hit target.
    /// Hosts can reserve this before centering adjacent controls in the row.
    pub fn compact_height(diameter: f32) -> f32 {
        if diameter.is_finite() {
            diameter.clamp(48.0, 180.0)
        } else {
            84.0
        }
    }
    /// An audio gain taper with continuous silence, useful travel below unity,
    /// and the caller's unchanged linear gain range and stored values.
    /// Detailed dials paint a unity landmark when 1.0 lies inside the range.
    pub fn perceptual_gain(mut self) -> Self {
        self.perceptual_gain =
            *self.range.start() >= 0.0 && *self.range.end() > *self.range.start();
        self
    }
    pub fn min_decimals(mut self, decimals: usize) -> Self {
        self.min_decimals = decimals.min(16);
        self
    }
    pub fn max_decimals(mut self, decimals: usize) -> Self {
        self.max_decimals = Some(decimals.min(16));
        self
    }
    pub fn fixed_decimals(mut self, decimals: usize) -> Self {
        self.min_decimals = decimals.min(16);
        self.max_decimals = Some(decimals.min(16));
        self
    }
    pub fn step_by(mut self, step: f64) -> Self {
        if step.is_finite() && step > 0.0 {
            self.step = Some(step);
        }
        self
    }
    pub fn clamping(mut self, clamping: SliderClamping) -> Self {
        self.clamping = clamping;
        self
    }
    pub fn automation(mut self, automation: Option<AutomationState>) -> Self {
        self.automation = automation;
        self
    }
    /// Hide duplicate wording only when the caller displays the same state
    /// next to its automation actions. The arc always keeps the state color.
    pub fn show_automation_label(mut self, show: bool) -> Self {
        self.show_automation_label = show;
        self
    }

    fn normalized(&self, value: f64) -> f32 {
        let minimum = self.scale_value(*self.range.start());
        let span = self.scale_value(*self.range.end()) - minimum;
        if span == 0.0 || !span.is_finite() || !value.is_finite() {
            0.5
        } else {
            ((self.scale_value(value) - minimum) / span).clamp(0.0, 1.0) as f32
        }
    }

    fn scale_value(&self, value: f64) -> f64 {
        if self.perceptual_gain {
            (value.max(0.0) / 0.01).ln_1p()
        } else {
            value
        }
    }

    fn unscale_value(&self, value: f64) -> f64 {
        if self.perceptual_gain {
            value.exp_m1() * 0.01
        } else {
            value
        }
    }

    fn adjusted_value(&self, value: f64, fraction: f64) -> f64 {
        let span = self.scale_value(*self.range.end()) - self.scale_value(*self.range.start());
        self.unscale_value(self.scale_value(value) + fraction * span)
    }

    fn edited_value(&self, mut value: f64) -> f64 {
        if let Some(step) = self.step.or(self.integral.then_some(1.0)) {
            value = self.range.start() + ((value - self.range.start()) / step).round() * step;
        }
        if let Some(decimals) = self.max_decimals.or(self.integral.then_some(0)) {
            value = egui::emath::round_to_decimals(value, decimals);
        }
        if self.clamping != SliderClamping::Never {
            value = value.clamp(
                self.range.start().min(*self.range.end()),
                self.range.start().max(*self.range.end()),
            );
        }
        value
    }

    fn add_contents(&mut self, ui: &mut Ui, width: f32) -> Response {
        ui.set_width(width);
        let ring_only = self.compact || self.diameter < 64.0;
        let label = (!self.compact && !self.text.is_empty()).then(|| text::label(ui, &self.text));
        let height = if self.compact {
            Self::compact_height(self.diameter)
        } else {
            self.diameter
        };
        let (target, mut response) = ui.allocate_exact_size(
            egui::vec2(self.diameter, height),
            egui::Sense::click_and_drag(),
        );
        let dial = Rect::from_min_size(target.min, Vec2::splat(self.diameter));
        let valid_range = self.range.start().is_finite()
            && self.range.end().is_finite()
            && (self.range.end() - self.range.start()).is_finite();
        let start_value = (self.value)(None);
        if valid_range
            && start_value.is_finite()
            && self.clamping == SliderClamping::Always
            && ui.is_enabled()
        {
            let clamped = start_value.clamp(
                self.range.start().min(*self.range.end()),
                self.range.start().max(*self.range.end()),
            );
            if clamped != start_value {
                (self.value)(Some(clamped));
                response.mark_changed();
            }
        }
        let drag_id = response.id.with("knob_drag");
        let pointer_edit = ui.input(|input| {
            ![egui::PointerButton::Secondary, egui::PointerButton::Middle]
                .into_iter()
                .any(|button| {
                    input.pointer.button_down(button) || input.pointer.button_released(button)
                })
        });
        if !pointer_edit {
            // Navigation can take over an existing primary drag. Discard its
            // accumulator so returning to primary cannot publish deferred motion.
            ui.data_mut(|data| data.remove::<DragState>(drag_id));
        }
        if ui.is_enabled() && valid_range && pointer_edit {
            if response.clicked() {
                response.request_focus();
            }
            if response.drag_started_by(egui::PointerButton::Primary) {
                // Include the movement that first established drag ownership,
                // in the response's local coordinates.
                let position = response
                    .interact_pointer_pos()
                    .map(|position| position - response.total_drag_delta().unwrap_or_default());
                ui.data_mut(|data| {
                    data.insert_temp(
                        drag_id,
                        DragState {
                            value: (self.value)(None),
                            position,
                        },
                    )
                });
            }
            let dragging = response.dragged_by(egui::PointerButton::Primary);
            let releasing = response.drag_stopped_by(egui::PointerButton::Primary);
            if (dragging || releasing)
                && let Some(mut drag) =
                    ui.data(|data| data.get_temp::<DragState>(drag_id))
                        .or_else(|| {
                            // A blocked drag may resume while Primary stays held,
                            // without replaying its suppressed displacement. Release
                            // alone must never create new ownership or an accumulator.
                            dragging.then(|| DragState {
                                value: (self.value)(None),
                                position: response.interact_pointer_pos(),
                            })
                        })
            {
                // egui clears `dragged` and total_drag_delta on release, but
                // retains the final interact_pointer_pos for the owned target.
                let position = response.interact_pointer_pos();
                let movement = position
                    .zip(drag.position)
                    .map_or(Vec2::ZERO, |(position, previous)| position - previous);
                let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
                drag.value = self.adjusted_value(
                    drag.value,
                    f64::from(movement.x - movement.y) / 200.0 * fine,
                );
                if self.clamping != SliderClamping::Never {
                    drag.value = drag.value.clamp(
                        self.range.start().min(*self.range.end()),
                        self.range.start().max(*self.range.end()),
                    );
                }
                let value = self.edited_value(drag.value);
                if value.is_finite() && value != (self.value)(None) {
                    (self.value)(Some(value));
                    response.mark_changed();
                }
                drag.position = position;
                ui.data_mut(|data| data.insert_temp(drag_id, drag));
            }
            if response.has_focus() {
                let direction = ui.input_mut(|input| {
                    let up = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
                    let down = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
                    i8::from(up) - i8::from(down)
                });
                if direction != 0 {
                    let current = (self.value)(None);
                    let next = self.step.map_or_else(
                        || {
                            if self.perceptual_gain {
                                self.adjusted_value(current, f64::from(direction) / 200.0)
                            } else {
                                current
                                    + f64::from(direction)
                                        * (self.range.end() - self.range.start()).abs()
                                        / 200.0
                            }
                        },
                        |step| current + f64::from(direction) * step,
                    );
                    let value = self.edited_value(next);
                    if value.is_finite() && value != (self.value)(None) {
                        (self.value)(Some(value));
                        response.mark_changed();
                    }
                }
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|data| data.remove::<DragState>(drag_id));
        }
        let dial_response = response
            .clone()
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let labels = self.endpoint_labels.clone().unwrap_or_else(|| {
            [
                format!("{}{}", self.range.start(), self.suffix),
                format!("{}{}", self.range.end(), self.suffix),
            ]
        });
        let colors = theme::palette(ui.ctx());
        if !ring_only {
            let labels_rect = ui
                .allocate_exact_size(
                    egui::vec2(width, tokens::FONT_SMALL + tokens::SPACE_XS),
                    egui::Sense::hover(),
                )
                .0;
            for (position, align, value) in [
                (
                    labels_rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    &labels[0],
                ),
                (
                    labels_rect.right_center(),
                    egui::Align2::RIGHT_CENTER,
                    &labels[1],
                ),
            ] {
                ui.painter().text(
                    position,
                    align,
                    value,
                    egui::FontId::proportional(tokens::FONT_CANVAS),
                    colors.muted,
                );
            }
        } else if !self.compact {
            // Keep the native Number ID stable when resizing across the small
            // dial threshold, without reserving an empty caption row or gap.
            ui.skip_ahead_auto_ids(1);
        }

        let numeric = if self.compact {
            None
        } else {
            let mut current = (self.value)(None);
            let current_before_number = current;
            let numeric_id = ui.next_auto_id();
            crate::value_entry::synchronize_number(ui, numeric_id, current);
            let minimum_decimals = self.min_decimals;
            let maximum_decimals = self.max_decimals;
            let integral = self.integral;
            let mut number = crate::number(&mut current)
                .speed(
                    self.step
                        .unwrap_or((self.range.end() - self.range.start()).abs() / 200.0),
                )
                .min_decimals(minimum_decimals)
                .max_decimals_opt(maximum_decimals.or(integral.then_some(0)))
                .custom_formatter(move |value, _| {
                    number_text(value, minimum_decimals, maximum_decimals, integral)
                })
                .suffix(&self.suffix);
            if self.clamping != SliderClamping::Never && valid_range {
                number = number
                    .range(self.range.clone())
                    .clamp_existing_to_range(self.clamping == SliderClamping::Always);
            }
            let numeric = ui.add_enabled(valid_range, number);
            if numeric.changed() && ui.is_enabled() && valid_range && current.is_finite() {
                let next = self.edited_value(current);
                if next != current_before_number {
                    (self.value)(Some(next));
                    response.mark_changed();
                }
            }
            let current = (self.value)(None);
            let expected = if response.changed() || numeric.changed() {
                self.display_round_trip
                    .as_ref()
                    .map_or(current, |convert| convert(current))
            } else {
                current
            };
            crate::value_entry::remember_number_value(ui, numeric_id, expected);
            if numeric.has_focus() || numeric.gained_focus() || numeric.lost_focus() {
                response = numeric.clone().union(response);
            } else {
                response = response.union(numeric.clone());
            }
            Some(numeric)
        };
        if let Some(label) = label {
            response = response.labelled_by(label.id);
        }
        let current = (self.value)(None);
        let fraction = self.normalized(current);
        let radius = (self.diameter * 0.38).min(
            self.diameter * 0.5
                - if ring_only {
                    tokens::SPACE_XS + 1.5
                } else {
                    9.0
                },
        );
        let center = dial.center();
        let divisions = if ring_only {
            0
        } else if self.diameter >= 100.0 {
            10
        } else {
            5
        };
        let mut tick_fractions: Vec<f32> = (0..=divisions)
            .filter(|_| !ring_only)
            .map(|index| index as f32 / divisions as f32)
            .collect();
        if divisions == 5 {
            tick_fractions.insert(3, 0.5);
        }
        let samples = arc_samples();
        let index_direction = Vec2::angled(START_ANGLE + ARC_ANGLE * fraction);
        let geometry = KnobGeometry {
            dial,
            number: numeric
                .as_ref()
                .map_or(Rect::NOTHING, |numeric| numeric.rect),
            center,
            radius,
            minimum: center + samples[0] * radius,
            maximum: center + samples[ARC_STEPS] * radius,
            index: center + index_direction * radius,
            range: [*self.range.start(), *self.range.end()],
            value: current,
            divisions,
            ticks: tick_fractions
                .iter()
                .map(|fraction| {
                    center + samples[(*fraction * ARC_STEPS as f32).round() as usize] * radius
                })
                .collect(),
            unity: (!ring_only && self.perceptual_gain && self.range.contains(&1.0)).then(|| {
                center + Vec2::angled(START_ANGLE + ARC_ANGLE * self.normalized(1.0)) * radius
            }),
        };
        if ui.is_rect_visible(dial) {
            let state_color = self
                .automation
                .map_or(colors.accent, |state| state.color(ui.ctx()));
            let active = if ui.is_enabled() {
                state_color
            } else {
                colors.muted
            };
            if !ring_only {
                let visuals = ui.style().interact(&dial_response);
                ui.painter()
                    .circle(center, radius * 0.75, visuals.bg_fill, visuals.bg_stroke);
            }
            let focused_ring = ring_only && ui.is_enabled() && dial_response.has_focus();
            ui.painter().add(egui::Shape::line(
                samples.iter().map(|unit| center + *unit * radius).collect(),
                Stroke::new(
                    if focused_ring { 3.0_f32 } else { 2.0_f32 },
                    if focused_ring {
                        colors.text
                    } else {
                        colors.line
                    },
                ),
            ));
            let end = (fraction * ARC_STEPS as f32).floor() as usize;
            let mut fill: Vec<Pos2> = samples[..=end]
                .iter()
                .map(|unit| center + *unit * radius)
                .collect();
            fill.push(geometry.index);
            ui.painter()
                .add(egui::Shape::line(fill, Stroke::new(2.5_f32, active)));
            for fraction in &tick_fractions {
                let unit = samples[(*fraction * ARC_STEPS as f32).round() as usize];
                let major = *fraction == 0.0 || *fraction == 0.5 || *fraction == 1.0;
                ui.painter().line_segment(
                    [
                        center + unit * (radius + 3.0),
                        center + unit * (radius + if major { 8.0 } else { 6.0 }),
                    ],
                    Stroke::new(1.0_f32, colors.muted),
                );
            }
            if let Some(unity) = geometry.unity {
                ui.painter()
                    .circle_stroke(unity, 3.0, Stroke::new(1.5_f32, colors.text));
                ui.painter().text(
                    center + egui::vec2(0.0, radius * 0.42),
                    egui::Align2::CENTER_CENTER,
                    "1× unity",
                    egui::FontId::proportional(tokens::FONT_SCALE),
                    colors.muted,
                );
            }
            if !ring_only {
                ui.painter().line_segment(
                    [center + index_direction * radius * 0.18, geometry.index],
                    Stroke::new(2.0_f32, colors.text),
                );
                ui.painter().circle_filled(center, 2.0, active);
            }
        }
        if !self.compact
            && self.show_automation_label
            && let Some(state) = self.automation
        {
            crate::automation_badge(ui, state);
        }
        if !self.compact
            && (current < self.range.start().min(*self.range.end())
                || current > self.range.start().max(*self.range.end()))
        {
            ui.label(
                text::TextRole::Note
                    .rich(ui.ctx(), "Outside range")
                    .color(colors.orange),
            );
        }
        response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), current, &self.text));
        ui.ctx().data_mut(|data| {
            data.insert_temp(response.id.with("knob_geometry"), geometry.clone());
            data.insert_temp(dial_response.id.with("knob_geometry"), geometry.clone());
            if let Some(numeric) = numeric {
                data.insert_temp(numeric.id.with("knob_geometry"), geometry);
            }
        });
        if self.compact {
            // Compact hosts supply a caption and their own unit-aware hint.
            // Avoid stacking a normalized readout or promising a hidden field.
            return response;
        }
        crate::hover_text(
            response,
            format!(
                "{}{} · {} to {}\nDrag up/right to increase; down/left to decrease. Shift for fine adjustment. Click the number to type.",
                current, self.suffix, labels[0], labels[1]
            ),
        )
    }
}

impl Widget for Knob<'_> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        let width = if self.compact {
            self.diameter
        } else {
            let current = (self.value)(None);
            let number = format!(
                "{}{}",
                number_text(current, self.min_decimals, self.max_decimals, self.integral),
                self.suffix
            );
            let number_width = ui
                .painter()
                .layout_no_wrap(
                    number,
                    ui.style().drag_value_text_style.resolve(ui.style()),
                    theme::palette(ui.ctx()).text,
                )
                .size()
                .x
                + ui.spacing().button_padding.x * 2.0
                + 2.0;
            let label_width = ui
                .painter()
                .layout_no_wrap(
                    self.text.clone(),
                    text::TextRole::Label.font(),
                    theme::palette(ui.ctx()).text,
                )
                .size()
                .x
                + 2.0 * tokens::SPACE_XS;
            self.diameter
                .max(112.0)
                .max(number_width)
                .max(label_width.min(ui.available_width().max(0.0)))
        };
        let response = ui.allocate_ui_with_layout(
            // Reserve the dial height before a horizontal host centers this
            // child. A zero-height child would be centered first, then grow
            // downward and make a compact parameter row taller than requested.
            egui::vec2(
                width,
                if self.compact {
                    Self::compact_height(self.diameter)
                } else {
                    0.0
                },
            ),
            egui::Layout::top_down(egui::Align::Center),
            |ui| self.add_contents(ui, width),
        );
        response.inner.union(response.response)
    }
}
