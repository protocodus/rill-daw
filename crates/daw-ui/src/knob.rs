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
    /// Tip of the value index, exactly on the calibrated arc.
    pub index: Pos2,
    pub range: [f64; 2],
    pub value: f64,
    pub divisions: usize,
    pub ticks: Vec<Pos2>,
}

pub fn knob_geometry(ctx: &egui::Context, id: Id) -> Option<KnobGeometry> {
    ctx.data(|data| data.get_temp(id.with("knob_geometry")))
}

#[derive(Clone, Copy, Default)]
struct DragState {
    value: f64,
    delta: Vec2,
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
        let span = self.range.end() - self.range.start();
        if span == 0.0 || !span.is_finite() || !value.is_finite() {
            0.5
        } else {
            ((value - self.range.start()) / span).clamp(0.0, 1.0) as f32
        }
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
        let label = (!self.text.is_empty()).then(|| text::label(ui, &self.text));
        let (dial, mut response) =
            ui.allocate_exact_size(Vec2::splat(self.diameter), egui::Sense::click_and_drag());
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
        if ui.is_enabled() && valid_range {
            if response.clicked() {
                response.request_focus();
            }
            if response.drag_started_by(egui::PointerButton::Primary) {
                ui.data_mut(|data| {
                    data.insert_temp(
                        drag_id,
                        DragState {
                            value: (self.value)(None),
                            delta: Vec2::ZERO,
                        },
                    )
                });
            }
            if response.dragged_by(egui::PointerButton::Primary) {
                let mut drag = ui
                    .data(|data| data.get_temp::<DragState>(drag_id))
                    .unwrap_or(DragState {
                        value: (self.value)(None),
                        delta: Vec2::ZERO,
                    });
                let delta = response.total_drag_delta().unwrap_or_default();
                let movement = delta - drag.delta;
                let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
                let span = self.range.end() - self.range.start();
                drag.value += f64::from(movement.x - movement.y) * span / 200.0 * fine;
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
                drag.delta = delta;
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
                    let step = self
                        .step
                        .unwrap_or((self.range.end() - self.range.start()).abs() / 200.0);
                    let current = (self.value)(None);
                    let value = self.edited_value(current + f64::from(direction) * step);
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
        let (labels_rect, _) = ui.allocate_exact_size(
            egui::vec2(width, tokens::FONT_SMALL + tokens::SPACE_XS),
            egui::Sense::hover(),
        );
        let colors = theme::palette(ui.ctx());
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

        let mut current = (self.value)(None);
        let current_before_number = current;
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
        if numeric.has_focus() || numeric.gained_focus() || numeric.lost_focus() {
            response = numeric.clone().union(response);
        } else {
            response = response.union(numeric.clone());
        }
        if let Some(label) = label {
            response = response.labelled_by(label.id);
        }
        let current = (self.value)(None);
        let fraction = self.normalized(current);
        let radius = (self.diameter * 0.38).min(self.diameter * 0.5 - 9.0);
        let center = dial.center();
        let divisions = if self.diameter >= 100.0 {
            10
        } else if self.diameter >= 64.0 {
            5
        } else {
            2
        };
        let mut tick_fractions: Vec<f32> = (0..=divisions)
            .map(|index| index as f32 / divisions as f32)
            .collect();
        if divisions == 5 {
            tick_fractions.insert(3, 0.5);
        }
        let samples = arc_samples();
        let index_direction = Vec2::angled(START_ANGLE + ARC_ANGLE * fraction);
        let geometry = KnobGeometry {
            dial,
            number: numeric.rect,
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
            let visuals = ui.style().interact(&dial_response);
            ui.painter()
                .circle(center, radius * 0.75, visuals.bg_fill, visuals.bg_stroke);
            ui.painter().add(egui::Shape::line(
                samples.iter().map(|unit| center + *unit * radius).collect(),
                Stroke::new(2.0_f32, colors.line),
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
            ui.painter().line_segment(
                [center + index_direction * radius * 0.18, geometry.index],
                Stroke::new(2.0_f32, colors.text),
            );
            ui.painter().circle_filled(center, 2.0, active);
        }
        if self.show_automation_label
            && let Some(state) = self.automation
        {
            crate::automation_badge(ui, state);
        }
        if current < self.range.start().min(*self.range.end())
            || current > self.range.start().max(*self.range.end())
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
            data.insert_temp(numeric.id.with("knob_geometry"), geometry);
        });
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
        let width = self.diameter.max(112.0).max(number_width);
        let response = ui.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| self.add_contents(ui, width),
        );
        response.inner.union(response.response)
    }
}
