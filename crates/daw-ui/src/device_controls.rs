//! Controls shared by every built-in device panel and the application.
//!
//! Values stay with the caller; each control reports edits through its
//! `Response` (`changed`, `drag_started`, `drag_stopped`) or a small output
//! struct. Every draggable element has a non-drag path: a typed value, a
//! context-menu action or arrow keys.
use crate::{AutomationState, Icon, IconButton, Knob, Taper, text::TextRole, theme, tokens};
use egui::{
    Align, Align2, Color32, Layout, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    Vec2, WidgetInfo, WidgetType, vec2,
};
use std::ops::RangeInclusive;

/// Reads typed text back to a plain value.
pub type ParseFn<'a> = Box<dyn Fn(&str) -> Option<f64> + 'a>;

/// Knob sizes of a device panel: dense rows, standard grids, one hero per panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KnobSize {
    Dense,
    #[default]
    Standard,
    Hero,
}

impl KnobSize {
    pub fn diameter(self) -> f32 {
        match self {
            Self::Dense => 34.0,
            Self::Standard => 48.0,
            Self::Hero => 64.0,
        }
    }
    /// Width of the caption/dial/readout cell. Fixed per size so panel grids align.
    pub fn cell_width(self) -> f32 {
        match self {
            Self::Dense => 55.0,
            Self::Standard => 72.0,
            Self::Hero => 89.0,
        }
    }
    /// Height of the whole cell: caption, dial and readout.
    pub fn cell_height(self) -> f32 {
        CAPTION_HEIGHT + self.diameter() + READOUT_HEIGHT
    }
}

const CAPTION_HEIGHT: f32 = tokens::FONT_SMALL + tokens::SPACE_XS;
const READOUT_HEIGHT: f32 = tokens::FONT_SMALL + tokens::SPACE_SM;

/// Text for a plain value in a unit, with magnitude-dependent precision.
pub fn value_text(value: f64, unit: &str) -> String {
    let decimals = match value.abs() {
        v if v >= 100.0 => 0,
        v if v >= 10.0 => 1,
        _ => 2,
    };
    let mut text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.truncate(text.trim_end_matches('0').trim_end_matches('.').len());
    }
    if text == "-0" {
        text = "0".into();
    }
    if unit.is_empty() {
        text
    } else {
        format!("{text} {unit}")
    }
}

/// What a [`LabeledKnob`] reports besides its `Response`.
pub struct LabeledKnobOutput {
    pub response: Response,
    /// A modulation source chosen from the knob's context menu or dropped on it.
    pub modulate_with: Option<u32>,
    /// **Automate** was chosen: the host creates (or reveals) the parameter's lane.
    pub automate: bool,
}

/// A knob with its caption above and value below, in a fixed cell.
#[must_use = "Add the knob to a Ui"]
pub struct LabeledKnob<'a> {
    caption: &'a str,
    value: &'a mut f64,
    range: RangeInclusive<f64>,
    size: KnobSize,
    unit: &'a str,
    bipolar: bool,
    taper: Taper,
    default: Option<f64>,
    modulation: Option<RangeInclusive<f64>>,
    automation: Option<AutomationState>,
    format: Option<Box<dyn Fn(f64) -> String + 'a>>,
    parse: Option<ParseFn<'a>>,
    mod_sources: &'a [(u32, &'a str)],
}

pub fn labeled_knob<'a>(
    caption: &'a str,
    value: &'a mut f64,
    range: RangeInclusive<f64>,
) -> LabeledKnob<'a> {
    LabeledKnob {
        caption,
        value,
        range,
        size: KnobSize::Standard,
        unit: "",
        bipolar: false,
        taper: Taper::Linear,
        default: None,
        modulation: None,
        automation: None,
        format: None,
        parse: None,
        mod_sources: &[],
    }
}

impl<'a> LabeledKnob<'a> {
    pub fn size(mut self, size: KnobSize) -> Self {
        self.size = size;
        self
    }
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }
    pub fn bipolar(mut self) -> Self {
        self.bipolar = true;
        self
    }
    pub fn taper(mut self, taper: Taper) -> Self {
        self.taper = taper;
        self
    }
    pub fn default_value(mut self, value: f64) -> Self {
        self.default = Some(value);
        self
    }
    pub fn modulation(mut self, range: RangeInclusive<f64>) -> Self {
        self.modulation = Some(range);
        self
    }
    pub fn automation(mut self, state: Option<AutomationState>) -> Self {
        self.automation = state;
        self
    }
    pub fn format(mut self, format: impl Fn(f64) -> String + 'a) -> Self {
        self.format = Some(Box::new(format));
        self
    }
    pub fn parse(mut self, parse: impl Fn(&str) -> Option<f64> + 'a) -> Self {
        self.parse = Some(Box::new(parse));
        self
    }
    /// Makes the knob a modulation destination: sources can be dropped on it
    /// or chosen from its context menu.
    pub fn mod_sources(mut self, sources: &'a [(u32, &'a str)]) -> Self {
        self.mod_sources = sources;
        self
    }

    fn text(&self, value: f64) -> String {
        self.format
            .as_ref()
            .map_or_else(|| value_text(value, self.unit), |format| format(value))
    }

    fn parsed(&self, text: &str) -> Option<f64> {
        let value = match &self.parse {
            Some(parse) => parse(text)?,
            None => text
                .trim()
                .trim_end_matches(self.unit)
                .trim()
                .parse()
                .ok()?,
        };
        let (low, high) = (
            self.range.start().min(*self.range.end()),
            self.range.start().max(*self.range.end()),
        );
        value.is_finite().then(|| value.clamp(low, high))
    }

    pub fn show(self, ui: &mut Ui) -> LabeledKnobOutput {
        let (cell, _) = ui.allocate_exact_size(
            vec2(self.size.cell_width(), self.size.cell_height()),
            Sense::hover(),
        );
        let mut ui = ui.new_child(
            UiBuilder::new()
                .max_rect(cell)
                .layout(Layout::top_down(Align::Center)),
        );
        let ui = &mut ui;
        ui.spacing_mut().item_spacing.y = 0.0;
        let colors = theme::palette(ui.ctx());
        let caption_rect = Rect::from_min_size(cell.min, vec2(cell.width(), CAPTION_HEIGHT));
        paint_centered(ui, caption_rect, TextRole::Label, self.caption, colors.text);
        ui.add_space(CAPTION_HEIGHT);

        let mut value = *self.value;
        let before = value;
        let mut knob = Knob::new(&mut value, self.range.clone())
            .compact()
            .pointer()
            .diameter(self.size.diameter())
            .taper(self.taper)
            .automation(self.automation);
        if self.bipolar {
            knob = knob.bipolar();
        }
        if let Some(depth) = self.modulation.clone() {
            knob = knob.modulation(depth);
        }
        let mut response = ui.add(knob);
        let dial = response.rect;
        crate::paint_automation_indicator(ui, dial, self.automation);
        let editing_id = response.id.with("typing");
        let mut typing = ui
            .data(|data| data.get_temp::<Option<String>>(editing_id))
            .flatten();
        let mut next = (value != before).then_some(value);

        // The context menu is the non-drag path to Reset, typing and modulation.
        let mut modulate_with = None;
        let (mut reset, mut type_value, mut automate) = (false, false, false);
        let automated = self.automation.is_some();
        let sources = self.mod_sources;
        response.context_menu(|ui| {
            if self.default.is_some() && crate::menu_item(ui, "Reset", None).clicked() {
                reset = true;
                ui.close();
            }
            if crate::menu_item(ui, "Type value…", None).clicked() {
                type_value = true;
                ui.close();
            }
            let lane = if automated {
                "Show automation lane"
            } else {
                "Automate"
            };
            if crate::menu_item(ui, lane, None).clicked() {
                automate = true;
                ui.close();
            }
            if !sources.is_empty() {
                ui.separator();
                for (id, name) in sources {
                    if crate::menu_item(ui, format!("Modulate with {name}"), None).clicked() {
                        modulate_with = Some(*id);
                        ui.close();
                    }
                }
            }
        });
        if let Some(payload) = (!sources.is_empty())
            .then(|| response.dnd_release_payload::<ModSource>())
            .flatten()
        {
            modulate_with = Some(payload.0);
        }
        if !sources.is_empty() && response.dnd_hover_payload::<ModSource>().is_some() {
            ui.painter().circle_stroke(
                dial.center(),
                dial.width() * 0.5,
                Stroke::new(tokens::FOCUS_WIDTH, colors.modulation),
            );
        }
        if let Some(default) = self.default
            && (reset || response.double_clicked())
        {
            next = Some(default);
        }

        let readout = Rect::from_min_size(
            egui::pos2(cell.left(), dial.bottom()),
            vec2(cell.width(), READOUT_HEIGHT),
        );
        if type_value {
            typing = Some(self.text(*self.value));
        }
        if let Some(mut text) = typing.take() {
            let mut edit = ui.new_child(
                UiBuilder::new()
                    .max_rect(readout)
                    .layout(Layout::top_down(Align::Center)),
            );
            let field = edit.add(
                egui::TextEdit::singleline(&mut text)
                    .font(TextRole::Note.font())
                    .desired_width(cell.width())
                    .horizontal_align(Align::Center)
                    .margin(vec2(0.0, 0.0)),
            );
            field.request_focus();
            if field.lost_focus() {
                if !edit.input(|input| input.key_pressed(egui::Key::Escape))
                    && let Some(parsed) = self.parsed(&text)
                {
                    next = Some(parsed);
                }
            } else {
                typing = Some(text);
            }
        } else {
            let read = ui.interact(readout, response.id.with("readout"), Sense::click());
            let color = if read.hovered() {
                colors.text
            } else {
                colors.muted
            };
            paint_centered(
                ui,
                readout,
                TextRole::Note,
                &self.text(next.unwrap_or(*self.value)),
                color,
            );
            if read.clicked() {
                typing = Some(self.text(*self.value));
            }
            crate::hover_text(read, "Click to type a value");
        }
        ui.data_mut(|data| data.insert_temp(editing_id, typing));

        if let Some(next) = next
            && next != *self.value
        {
            *self.value = next;
            response.mark_changed();
        }
        response.widget_info(|| WidgetInfo::slider(true, *self.value, self.caption));
        LabeledKnobOutput {
            response,
            modulate_with,
            automate,
        }
    }
}

impl egui::Widget for LabeledKnob<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui).response
    }
}

fn paint_centered(ui: &Ui, rect: Rect, role: TextRole, text: &str, color: Color32) {
    let font = role.font();
    let width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color)
        .size()
        .x;
    let text_rect = if width < rect.width() {
        Rect::from_center_size(rect.center(), vec2(width + 1.0, rect.height()))
    } else {
        rect
    };
    crate::text::paint_single_line(ui.painter(), text_rect, font, color, text);
}

/// A continuous value on one line: a small arc and, to its right, the value
/// with its unit. For tables of like values (the bands of an equalizer, the
/// pads of a drum machine), where a caption above every dial would repeat
/// the row's name; the caller names the row once and passes that name here
/// for hover and assistive technology. The whole cell drags, Shift for fine;
/// arrow keys nudge when focused; double-click resets to the default.
#[must_use = "Add the knob to a Ui"]
pub struct InlineKnob<'a> {
    name: &'a str,
    value: &'a mut f64,
    range: RangeInclusive<f64>,
    text: String,
    width: Option<f32>,
    bipolar: bool,
    taper: Taper,
    default: Option<f64>,
    automation: Option<AutomationState>,
}

/// `text` is the value as it reads, with its unit ("2.50 kHz").
pub fn inline_knob<'a>(
    name: &'a str,
    value: &'a mut f64,
    range: RangeInclusive<f64>,
    text: impl Into<String>,
) -> InlineKnob<'a> {
    InlineKnob {
        name,
        value,
        range,
        text: text.into(),
        width: None,
        bipolar: false,
        taper: Taper::Linear,
        default: None,
        automation: None,
    }
}

/// Space between an [`InlineKnob`]'s arc and its value. The arc keeps a
/// point of margin of its own.
const INLINE_GAP: f32 = 4.0;

/// The width an [`inline_knob`] needs to show `text` whole.
pub fn inline_knob_width(ui: &Ui, text: &str) -> f32 {
    crate::MINI_KNOB_DIAMETER
        + INLINE_GAP
        + ui.painter()
            .layout_no_wrap(text.to_owned(), TextRole::Note.font(), Color32::PLACEHOLDER)
            .size()
            .x
}

impl InlineKnob<'_> {
    /// A fixed cell width, so the values of a column start on one line
    /// whatever they read. Without it the cell fits its text.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    pub fn bipolar(mut self) -> Self {
        self.bipolar = true;
        self
    }
    pub fn taper(mut self, taper: Taper) -> Self {
        self.taper = taper;
        self
    }
    /// Enables Reset by double-click and from the context menu. A caller
    /// with a context menu of its own resets on `double_clicked` instead.
    pub fn default_value(mut self, value: f64) -> Self {
        self.default = Some(value);
        self
    }
    pub fn automation(mut self, state: Option<AutomationState>) -> Self {
        self.automation = state;
        self
    }
}

impl egui::Widget for InlineKnob<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let width = self
            .width
            .unwrap_or_else(|| inline_knob_width(ui, &self.text));
        let mut knob = Knob::new(self.value, self.range)
            .inline(width)
            .taper(self.taper)
            .text(self.name)
            .automation(self.automation);
        if self.bipolar {
            knob = knob.bipolar();
        }
        if let Some(default) = self.default {
            knob = knob.default_value(default);
        }
        let response = ui.add(knob);
        if ui.is_rect_visible(response.rect) {
            let colors = theme::palette(ui.ctx());
            let mut text = response.rect;
            text.min.x += crate::MINI_KNOB_DIAMETER + INLINE_GAP;
            let color = if response.hovered() || response.dragged() || response.has_focus() {
                colors.text
            } else {
                colors.text.gamma_multiply(0.85)
            };
            crate::text::paint_single_line(
                ui.painter(),
                text,
                TextRole::Note.font(),
                color,
                &self.text,
            );
        }
        response
    }
}

/// Marks the selected column of a table or cell grid: a faint tint with a
/// rule on top. Call it before drawing the column's contents.
pub fn column_highlight(ui: &Ui, rect: Rect) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let colors = theme::palette(ui.ctx());
    ui.painter()
        .rect_filled(rect, tokens::RADIUS, colors.accent.gamma_multiply(0.1));
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min, vec2(rect.width(), 2.0)),
        1.0,
        colors.accent,
    );
}

/// Exclusive choice among two to four short options, as toggle buttons.
/// Falls back to a drop-down for more, or when they do not fit the width.
pub fn segmented(ui: &mut Ui, selected: &mut usize, options: &[&str]) -> Response {
    let needed = crate::toggle_group_width(ui, options.iter().copied());
    if options.len() >= crate::TOGGLE_OPTION_LIMIT || needed > ui.available_width() {
        return choice_fallback(ui, selected, options);
    }
    let indexed: Vec<(usize, &str)> = options.iter().copied().enumerate().collect();
    crate::toggle_group(ui, selected, &indexed)
}

fn choice_fallback(ui: &mut Ui, selected: &mut usize, options: &[&str]) -> Response {
    let mut changed = false;
    let mut response = crate::select(
        ui.next_auto_id(),
        options.get(*selected).copied().unwrap_or(""),
    )
    .show_ui(ui, |ui| {
        for (index, option) in options.iter().enumerate() {
            if ui.selectable_label(*selected == index, *option).clicked() && *selected != index {
                *selected = index;
                changed = true;
            }
        }
    })
    .response;
    if changed {
        response.mark_changed();
    }
    response
}

/// Previous/next arrows around a menu of the current choice, for longer lists.
pub fn stepped_selector(ui: &mut Ui, selected: &mut usize, options: &[&str]) -> Response {
    let last = options.len().saturating_sub(1);
    let mut next = *selected;
    let mut response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tokens::SPACE_XS;
            if ui
                .add_enabled(
                    *selected > 0,
                    IconButton::icon_only(Icon::Back, "Previous choice"),
                )
                .clicked()
            {
                next = *selected - 1;
            }
            crate::choice_menu(ui, options.get(*selected).copied().unwrap_or(""), |ui| {
                for (index, option) in options.iter().enumerate() {
                    if crate::selectable(ui, *selected == index, *option).clicked() {
                        next = index;
                        ui.close();
                    }
                }
            });
            if ui
                .add_enabled(
                    *selected < last,
                    IconButton::icon_only(Icon::ChevronRight, "Next choice"),
                )
                .clicked()
            {
                next = *selected + 1;
            }
        })
        .response;
    if next != *selected {
        *selected = next;
        response.mark_changed();
    }
    response
}

const LED_LAMP: f32 = 13.0;

/// The width a [`toggle_led`] with this label takes.
pub fn toggle_led_width(ui: &Ui, label: &str) -> f32 {
    LED_LAMP
        + tokens::SPACE_SM
        + ui.painter()
            .layout_no_wrap(
                label.to_owned(),
                TextRole::Label.font(),
                Color32::PLACEHOLDER,
            )
            .size()
            .x
}

/// A section enable: a small lit lamp with a caption.
pub fn toggle_led(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let colors = theme::palette(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), TextRole::Label.font(), colors.text);
    let lamp = LED_LAMP;
    let size = vec2(
        lamp + tokens::SPACE_SM + galley.size().x,
        tokens::COMPACT_HEIGHT,
    );
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let center = egui::pos2(rect.left() + lamp * 0.5, rect.center().y);
        if *value {
            ui.painter().circle_filled(
                center,
                lamp * 0.5 + 2.0,
                colors.accent.gamma_multiply(0.25),
            );
            ui.painter()
                .circle_filled(center, lamp * 0.5 - 1.0, colors.accent);
        } else {
            ui.painter()
                .circle_filled(center, lamp * 0.5 - 1.0, colors.control_fill);
            ui.painter().circle_stroke(
                center,
                lamp * 0.5 - 1.0,
                Stroke::new(tokens::BORDER_WIDTH, colors.muted),
            );
        }
        if response.has_focus() {
            ui.painter().circle_stroke(
                center,
                lamp * 0.5 + 2.0,
                Stroke::new(tokens::FOCUS_WIDTH, colors.text),
            );
        }
        ui.painter().galley(
            egui::pos2(
                rect.left() + lamp + tokens::SPACE_SM,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            if *value { colors.text } else { colors.muted },
        );
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, *value, label));
    response
}

/// Per-axis outcome of an [`xy_pad`] frame.
pub struct XyPadOutput {
    pub response: Response,
    pub x_changed: bool,
    pub y_changed: bool,
}

/// Two normalized (0 to 1) parameters on one surface. A press jumps to the
/// pointer; further movement is relative, and Shift makes it fine. Arrow keys
/// nudge the focused pad.
pub fn xy_pad(ui: &mut Ui, x: &mut f64, y: &mut f64, size: Vec2, labels: [&str; 2]) -> XyPadOutput {
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let colors = theme::palette(ui.ctx());
    let (before_x, before_y) = (*x, *y);
    let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
    let inner = rect.shrink(tokens::SPACE_SM);
    if response.drag_started() || response.clicked() {
        response.request_focus();
        if let Some(pointer) = response.interact_pointer_pos() {
            *x = f64::from(((pointer.x - inner.left()) / inner.width()).clamp(0.0, 1.0));
            *y = f64::from((1.0 - (pointer.y - inner.top()) / inner.height()).clamp(0.0, 1.0));
        }
    } else if response.dragged() {
        let delta = response.drag_delta();
        *x = (*x + f64::from(delta.x / inner.width() * fine)).clamp(0.0, 1.0);
        *y = (*y - f64::from(delta.y / inner.height() * fine)).clamp(0.0, 1.0);
    }
    if response.has_focus() {
        let step = 0.01 * f64::from(fine);
        ui.input_mut(|input| {
            for (key, dx, dy) in [
                (egui::Key::ArrowLeft, -1.0, 0.0),
                (egui::Key::ArrowRight, 1.0, 0.0),
                (egui::Key::ArrowUp, 0.0, 1.0),
                (egui::Key::ArrowDown, 0.0, -1.0),
            ] {
                if input.consume_key(egui::Modifiers::NONE, key)
                    || input.consume_key(egui::Modifiers::SHIFT, key)
                {
                    *x = (*x + dx * step).clamp(0.0, 1.0);
                    *y = (*y + dy * step).clamp(0.0, 1.0);
                }
            }
        });
    }
    let (x_changed, y_changed) = (*x != before_x, *y != before_y);
    if x_changed || y_changed {
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, tokens::RADIUS, colors.bg);
        for step in 1..4 {
            let at = step as f32 / 4.0;
            painter.line_segment(
                [
                    egui::pos2(inner.left() + inner.width() * at, inner.top()),
                    egui::pos2(inner.left() + inner.width() * at, inner.bottom()),
                ],
                Stroke::new(1.0_f32, colors.grid_minor),
            );
            painter.line_segment(
                [
                    egui::pos2(inner.left(), inner.top() + inner.height() * at),
                    egui::pos2(inner.right(), inner.top() + inner.height() * at),
                ],
                Stroke::new(1.0_f32, colors.grid_minor),
            );
        }
        painter.rect_stroke(
            rect,
            tokens::RADIUS,
            Stroke::new(
                if response.has_focus() {
                    tokens::FOCUS_WIDTH
                } else {
                    tokens::BORDER_WIDTH
                },
                if response.has_focus() {
                    colors.text
                } else {
                    colors.line
                },
            ),
            StrokeKind::Inside,
        );
        let point = egui::pos2(
            inner.left() + inner.width() * *x as f32,
            inner.bottom() - inner.height() * *y as f32,
        );
        painter.line_segment(
            [
                egui::pos2(point.x, inner.top()),
                egui::pos2(point.x, inner.bottom()),
            ],
            Stroke::new(1.0_f32, colors.line),
        );
        painter.line_segment(
            [
                egui::pos2(inner.left(), point.y),
                egui::pos2(inner.right(), point.y),
            ],
            Stroke::new(1.0_f32, colors.line),
        );
        painter.circle_filled(point, 6.0, colors.accent);
        painter.circle_stroke(point, 6.0, Stroke::new(1.5_f32, colors.bg));
        let font = TextRole::Note.font();
        painter.text(
            egui::pos2(rect.right() - 3.0, rect.bottom() - 3.0),
            Align2::RIGHT_BOTTOM,
            labels[0],
            font.clone(),
            colors.muted,
        );
        painter.text(
            egui::pos2(rect.left() + 3.0, rect.top() + 2.0),
            Align2::LEFT_TOP,
            labels[1],
            font,
            colors.muted,
        );
    }
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Other,
            true,
            format!(
                "{} {:.0} %, {} {:.0} %",
                labels[0],
                *x * 100.0,
                labels[1],
                *y * 100.0
            ),
        )
    });
    XyPadOutput {
        response,
        x_changed,
        y_changed,
    }
}

/// Envelope stages, in the order a note passes through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeParam {
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    AttackCurve,
    DecayCurve,
    ReleaseCurve,
}

/// Plain envelope data: stage times in seconds, sustain 0 to 1, curves −1 to 1
/// (0 linear, positive bends slow-then-fast).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeShape {
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub curves: [f32; 3],
}

impl Default for EnvelopeShape {
    fn default() -> Self {
        Self {
            attack: 0.01,
            hold: 0.0,
            decay: 0.2,
            sustain: 0.7,
            release: 0.3,
            curves: [0.0; 3],
        }
    }
}

/// One requested change to an envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeEdit {
    pub param: EnvelopeParam,
    pub value: f32,
}

pub struct EnvelopeOutput {
    pub response: Response,
    pub edits: Vec<EnvelopeEdit>,
}

impl EnvelopeShape {
    pub fn apply(&mut self, edit: EnvelopeEdit) {
        match edit.param {
            EnvelopeParam::Attack => self.attack = edit.value,
            EnvelopeParam::Hold => self.hold = edit.value,
            EnvelopeParam::Decay => self.decay = edit.value,
            EnvelopeParam::Sustain => self.sustain = edit.value,
            EnvelopeParam::Release => self.release = edit.value,
            EnvelopeParam::AttackCurve => self.curves[0] = edit.value,
            EnvelopeParam::DecayCurve => self.curves[1] = edit.value,
            EnvelopeParam::ReleaseCurve => self.curves[2] = edit.value,
        }
    }
}

const ENVELOPE_MAX_TIME: f32 = 10.0;
const TIME_SCALE: f32 = 100.0;

fn time_to_fraction(seconds: f32) -> f32 {
    (1.0 + seconds.clamp(0.0, ENVELOPE_MAX_TIME) * TIME_SCALE).ln()
        / (1.0 + ENVELOPE_MAX_TIME * TIME_SCALE).ln()
}

fn fraction_to_time(fraction: f32) -> f32 {
    (((1.0 + ENVELOPE_MAX_TIME * TIME_SCALE).ln() * fraction.clamp(0.0, 1.0)).exp() - 1.0)
        / TIME_SCALE
}

/// A segment from `from` to `to` bent by `curve`, sampled at `t` in 0 to 1.
fn bend(t: f32, curve: f32) -> f32 {
    if curve.abs() < 1e-3 {
        t
    } else {
        let k = curve * 4.0;
        ((k * t).exp() - 1.0) / (k.exp() - 1.0)
    }
}

/// Fraction of each quarter of the plot a stage occupies:
/// attack, hold, decay, release.
fn envelope_zones(shape: &EnvelopeShape) -> [f32; 4] {
    let hold = time_to_fraction(shape.hold);
    [
        time_to_fraction(shape.attack),
        hold,
        // Hold and decay share the second quarter.
        time_to_fraction(shape.decay).min(1.0 - hold),
        time_to_fraction(shape.release),
    ]
}

/// ADSR (with optional hold) whose break points can be dragged. Time maps
/// logarithmically; the plot stays four equal zones so short stages remain
/// editable. `playheads` are (position across the plot, level) dots from
/// telemetry. Every parameter also has a knob in the panel, and a focused
/// break point moves with the arrow keys.
pub fn envelope_editor(
    ui: &mut Ui,
    shape: &EnvelopeShape,
    size: Vec2,
    playheads: &[(f32, f32)],
) -> EnvelopeOutput {
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    let plot = rect.shrink2(vec2(tokens::SPACE_MD, tokens::SPACE_MD));
    let zones = envelope_zones(shape);
    let zone_width = plot.width() / 4.0;
    let level_y = |level: f32| plot.bottom() - plot.height() * level.clamp(0.0, 1.0);
    let attack_end = plot.left() + zone_width * zones[0];
    let hold_end = plot.left() + zone_width * (1.0 + zones[1]);
    let decay_end = hold_end.max(plot.left() + zone_width) + zone_width * zones[2];
    let sustain_end = plot.left() + zone_width * 3.0;
    let release_end = sustain_end + zone_width * zones[3];
    let sustain_y = level_y(shape.sustain);
    let nodes: [(EnvelopeParam, Pos2); 5] = [
        (EnvelopeParam::Attack, egui::pos2(attack_end, level_y(1.0))),
        (EnvelopeParam::Hold, egui::pos2(hold_end, level_y(1.0))),
        (EnvelopeParam::Decay, egui::pos2(decay_end, sustain_y)),
        (EnvelopeParam::Sustain, egui::pos2(sustain_end, sustain_y)),
        (
            EnvelopeParam::Release,
            egui::pos2(release_end, level_y(0.0)),
        ),
    ];
    let mut edits = Vec::new();
    let mut interacted = response.clone();
    for (param, position) in nodes {
        let target = Rect::from_center_size(position, Vec2::splat(tokens::HIT_TARGET));
        let node = ui.interact(
            target,
            response.id.with(("node", param as u8)),
            Sense::click_and_drag(),
        );
        let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
        let time_at = |pointer: f32, left: f32| {
            fraction_to_time(((pointer - left) / zone_width).clamp(0.0, 1.0))
        };
        if let Some(pointer) = node
            .dragged()
            .then(|| node.interact_pointer_pos())
            .flatten()
        {
            let level = ((plot.bottom() - pointer.y) / plot.height()).clamp(0.0, 1.0);
            let value = match param {
                EnvelopeParam::Attack => Some(time_at(pointer.x, plot.left())),
                EnvelopeParam::Hold => Some(time_at(pointer.x, plot.left() + zone_width)),
                EnvelopeParam::Decay => {
                    edits.push(EnvelopeEdit {
                        param: EnvelopeParam::Sustain,
                        value: level,
                    });
                    Some(time_at(pointer.x, hold_end.max(plot.left() + zone_width)))
                }
                EnvelopeParam::Sustain => {
                    edits.push(EnvelopeEdit {
                        param: EnvelopeParam::Sustain,
                        value: level,
                    });
                    None
                }
                EnvelopeParam::Release => Some(time_at(pointer.x, sustain_end)),
                _ => None,
            };
            if let Some(value) = value {
                edits.push(EnvelopeEdit { param, value });
            }
        }
        if node.has_focus() {
            ui.input_mut(|input| {
                let step = 0.01 * fine;
                let horizontal =
                    i8::from(input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight))
                        - i8::from(input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft));
                let vertical =
                    i8::from(input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp))
                        - i8::from(input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
                let current = match param {
                    EnvelopeParam::Attack => shape.attack,
                    EnvelopeParam::Hold => shape.hold,
                    EnvelopeParam::Decay => shape.decay,
                    EnvelopeParam::Release => shape.release,
                    _ => 0.0,
                };
                if horizontal != 0 && !matches!(param, EnvelopeParam::Sustain) {
                    edits.push(EnvelopeEdit {
                        param,
                        value: fraction_to_time(
                            (time_to_fraction(current) + f32::from(horizontal) * step)
                                .clamp(0.0, 1.0),
                        ),
                    });
                }
                if vertical != 0 && matches!(param, EnvelopeParam::Decay | EnvelopeParam::Sustain) {
                    edits.push(EnvelopeEdit {
                        param: EnvelopeParam::Sustain,
                        value: (shape.sustain + f32::from(vertical) * step).clamp(0.0, 1.0),
                    });
                }
            });
        }
        interacted = interacted.union(node);
    }
    // Curve handles sit at the middle of the attack, decay and release segments.
    let curve_nodes = [
        (
            EnvelopeParam::AttackCurve,
            0usize,
            egui::pos2(plot.left(), level_y(0.0)),
            nodes[0].1,
        ),
        (EnvelopeParam::DecayCurve, 1, nodes[1].1, nodes[2].1),
        (EnvelopeParam::ReleaseCurve, 2, nodes[3].1, nodes[4].1),
    ];
    for (param, index, from, to) in curve_nodes {
        let curve = shape.curves[index];
        let t = 0.5;
        let level = bend(t, curve);
        let position = egui::pos2(
            from.x + (to.x - from.x) * t,
            from.y + (to.y - from.y) * level,
        );
        let target = Rect::from_center_size(position, Vec2::splat(tokens::HIT_TARGET));
        let handle = ui.interact(
            target,
            response.id.with(("curve", index)),
            Sense::click_and_drag(),
        );
        if handle.dragged() && (to.y - from.y).abs() > 1.0 {
            // Vertical drag bends the segment; the sign follows its direction.
            let direction = (to.y - from.y).signum();
            let delta = handle.drag_delta().y * direction / (to.y - from.y).abs() * 2.0;
            edits.push(EnvelopeEdit {
                param,
                value: (curve + delta).clamp(-1.0, 1.0),
            });
        }
        if handle.double_clicked() {
            edits.push(EnvelopeEdit { param, value: 0.0 });
        }
        handle.context_menu(|ui| {
            if crate::menu_item(ui, "Reset curve", None).clicked() {
                edits.push(EnvelopeEdit { param, value: 0.0 });
                ui.close();
            }
        });
        interacted = interacted.union(handle);
    }
    if !edits.is_empty() {
        interacted.mark_changed();
    }
    response = interacted;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, tokens::RADIUS, colors.bg);
        for step in 1..4 {
            let x = plot.left() + zone_width * step as f32;
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                Stroke::new(1.0_f32, colors.grid_minor),
            );
        }
        let mut line = Vec::new();
        for step in 0..=160 {
            let at = step as f32 / 160.0;
            line.push(egui::pos2(
                plot.left() + plot.width() * at,
                level_y(envelope_level(shape, at)),
            ));
        }
        let mut fill = line.clone();
        fill.push(egui::pos2(plot.right(), plot.bottom()));
        fill.push(egui::pos2(plot.left(), plot.bottom()));
        painter.add(egui::Shape::convex_polygon(
            fill,
            colors.accent.gamma_multiply(0.14),
            Stroke::NONE,
        ));
        painter.add(egui::Shape::line(line, Stroke::new(2.0_f32, colors.accent)));
        for (_, position) in nodes {
            painter.circle_filled(position, 4.5, colors.accent);
            painter.circle_stroke(position, 4.5, Stroke::new(1.5_f32, colors.bg));
        }
        for (_, index, from, to) in curve_nodes {
            let level = bend(0.5, shape.curves[index]);
            painter.circle_stroke(
                egui::pos2(
                    from.x + (to.x - from.x) * 0.5,
                    from.y + (to.y - from.y) * level,
                ),
                3.0,
                Stroke::new(1.0_f32, colors.muted),
            );
        }
        for (position, level) in playheads {
            painter.circle_filled(
                egui::pos2(
                    plot.left() + plot.width() * position.clamp(0.0, 1.0),
                    level_y(*level),
                ),
                3.0,
                colors.text,
            );
        }
        painter.rect_stroke(
            rect,
            tokens::RADIUS,
            Stroke::new(tokens::BORDER_WIDTH, colors.line),
            StrokeKind::Inside,
        );
        let font = TextRole::Note.font();
        for (index, name) in ["Attack", "Decay", "Sustain", "Release"]
            .into_iter()
            .enumerate()
        {
            painter.text(
                egui::pos2(
                    plot.left() + zone_width * (index as f32 + 0.5),
                    rect.bottom() - 2.0,
                ),
                Align2::CENTER_BOTTOM,
                name,
                font.clone(),
                colors.muted,
            );
        }
    }
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Other,
            true,
            format!(
                "Envelope: attack {:.3} s, decay {:.3} s, sustain {:.0} %, release {:.3} s",
                shape.attack,
                shape.decay,
                shape.sustain * 100.0,
                shape.release
            ),
        )
    });
    EnvelopeOutput { response, edits }
}

/// Envelope level at `at` (0 to 1 across the plot), exactly as the plot draws
/// it: each stage fills its quarter up to its time fraction, then holds.
pub fn envelope_level(shape: &EnvelopeShape, at: f32) -> f32 {
    let zones = envelope_zones(shape);
    let at = at.clamp(0.0, 1.0);
    let index = ((at * 4.0) as usize).min(3);
    let local = at * 4.0 - index as f32;
    match index {
        0 => bend((local / zones[0].max(1e-4)).min(1.0), shape.curves[0]),
        1 => {
            if local < zones[1] {
                1.0
            } else {
                1.0 - (1.0 - shape.sustain)
                    * bend(
                        ((local - zones[1]) / zones[2].max(1e-4)).min(1.0),
                        shape.curves[1],
                    )
            }
        }
        2 => shape.sustain,
        _ => shape.sustain * (1.0 - bend((local / zones[3].max(1e-4)).min(1.0), shape.curves[2])),
    }
}

/// A sample name dragged from the library, dropped on a pad or sample view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleDrop(pub String);

/// One pad's display state.
#[derive(Clone, Copy, Debug, Default)]
pub struct PadView<'a> {
    pub name: &'a str,
    pub loaded: bool,
    /// Flash level from telemetry, 0 to 1.
    pub activity: f32,
}

#[derive(Debug, Default, PartialEq)]
pub struct PadGridOutput {
    /// Pad pressed this frame with its audition velocity.
    pub pressed: Option<(usize, f32)>,
    pub released: Option<usize>,
    pub selected: Option<usize>,
    pub dropped: Option<(usize, String)>,
}

/// Audition velocity for a press at `y` within a pad: the top is loudest.
pub fn pad_velocity(pad: Rect, y: f32) -> f32 {
    (1.0 - (y - pad.top()) / pad.height().max(1.0)).clamp(0.0, 1.0) * 0.85 + 0.15
}

/// Pads in a 4×4 grid, bottom row first as on a drum controller.
pub fn pad_grid(
    ui: &mut Ui,
    pads: &[PadView<'_>; 16],
    selected: usize,
    size: Vec2,
) -> PadGridOutput {
    let colors = theme::palette(ui.ctx());
    let gap = tokens::SPACE_SM;
    let cell = vec2((size.x - gap * 3.0) / 4.0, (size.y - gap * 3.0) / 4.0);
    let (area, _) = ui.allocate_exact_size(size, Sense::hover());
    let held_id = ui.id().with("pad_held");
    let held = ui
        .data(|data| data.get_temp::<Option<usize>>(held_id))
        .flatten();
    let mut output = PadGridOutput::default();
    let mut now_held = held;
    for (index, pad) in pads.iter().enumerate() {
        let (column, row) = (index % 4, 3 - index / 4);
        let rect = Rect::from_min_size(
            area.min + vec2(column as f32 * (cell.x + gap), row as f32 * (cell.y + gap)),
            cell,
        );
        let response = ui.interact(rect, ui.id().with(("pad", index)), Sense::click_and_drag());
        if response.hovered()
            && ui.input(|input| input.pointer.primary_pressed())
            && let Some(pointer) = response
                .interact_pointer_pos()
                .or_else(|| ui.input(|i| i.pointer.hover_pos()))
        {
            output.pressed = Some((index, pad_velocity(rect, pointer.y)));
            output.selected = Some(index);
            now_held = Some(index);
            response.request_focus();
        }
        if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Space)) {
            output.pressed = Some((index, 0.8));
            output.selected = Some(index);
            now_held = Some(index);
        }
        if let Some(payload) = response.dnd_release_payload::<SampleDrop>() {
            output.dropped = Some((index, payload.0.clone()));
            output.selected = Some(index);
        }
        let dropping = response.dnd_hover_payload::<SampleDrop>().is_some();
        if ui.is_rect_visible(rect) {
            let is_selected = selected == index;
            let base = if response.hovered() {
                colors.control_hover
            } else {
                colors.control_fill
            };
            let painter = ui.painter();
            painter.rect_filled(rect, tokens::RADIUS, base);
            if pad.activity > 0.0 {
                painter.rect_filled(
                    rect,
                    tokens::RADIUS,
                    colors
                        .accent
                        .gamma_multiply(0.55 * pad.activity.clamp(0.0, 1.0)),
                );
            }
            painter.rect_stroke(
                rect,
                tokens::RADIUS,
                Stroke::new(
                    if is_selected || dropping || response.has_focus() {
                        tokens::FOCUS_WIDTH
                    } else {
                        tokens::BORDER_WIDTH
                    },
                    if dropping {
                        colors.action
                    } else if is_selected || response.has_focus() {
                        colors.text
                    } else {
                        colors.line
                    },
                ),
                StrokeKind::Inside,
            );
            painter.text(
                rect.left_top() + vec2(tokens::SPACE_XS, tokens::SPACE_XS),
                Align2::LEFT_TOP,
                format!("{}", index + 1),
                TextRole::Note.font(),
                colors.muted,
            );
            let label = if pad.loaded { pad.name } else { "Empty" };
            crate::text::paint_single_line(
                painter,
                Rect::from_min_max(
                    egui::pos2(rect.left() + tokens::SPACE_XS, rect.center().y),
                    rect.max - vec2(tokens::SPACE_XS, tokens::SPACE_XS),
                ),
                TextRole::Label.font(),
                if pad.loaded {
                    colors.text
                } else {
                    colors.muted
                },
                label,
            );
        }
        response.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Button,
                true,
                format!(
                    "Pad {}: {}",
                    index + 1,
                    if pad.loaded { pad.name } else { "empty" }
                ),
            )
        });
    }
    if ui.input(|input| input.pointer.primary_released())
        || ui.input(|input| input.key_released(egui::Key::Space))
    {
        output.released = now_held.take();
    }
    ui.data_mut(|data| data.insert_temp(held_id, now_held));
    output
}

#[derive(Debug, Default, PartialEq)]
pub struct KeyboardOutput {
    pub pressed: Option<(u8, f32)>,
    pub released: Option<u8>,
}

const BLACK_KEYS: [bool; 12] = [
    false, true, false, true, false, false, true, false, true, false, true, false,
];

/// Audition keyboard of `octaves` octaves from MIDI note `first` (a C).
/// `held` notes and the `root` note are highlighted.
pub fn keyboard_strip(
    ui: &mut Ui,
    first: u8,
    octaves: u8,
    held: &[u8],
    root: Option<u8>,
    size: Vec2,
) -> KeyboardOutput {
    let colors = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let whites: Vec<u8> = (0..octaves * 12)
        .map(|offset| first + offset)
        .filter(|note| !BLACK_KEYS[usize::from(note % 12)])
        .collect();
    let key_width = rect.width() / whites.len().max(1) as f32;
    let black_size = vec2(key_width * 0.6, rect.height() * 0.62);
    let white_rect = |index: usize| {
        Rect::from_min_size(
            rect.min + vec2(index as f32 * key_width, 0.0),
            vec2(key_width, rect.height()),
        )
    };
    let black_rect = |note: u8| {
        let before = whites
            .iter()
            .position(|white| *white == note - 1)
            .unwrap_or(0);
        Rect::from_min_size(
            egui::pos2(white_rect(before).right() - black_size.x * 0.5, rect.top()),
            black_size,
        )
    };
    let note_at = |pointer: Pos2| {
        (first..first + octaves * 12)
            .filter(|note| BLACK_KEYS[usize::from(note % 12)])
            .find(|note| black_rect(*note).contains(pointer))
            .or_else(|| {
                whites
                    .iter()
                    .enumerate()
                    .find(|(index, _)| white_rect(*index).contains(pointer))
                    .map(|(_, note)| *note)
            })
    };
    let sounding_id = response.id.with("sounding");
    let sounding = ui
        .data(|data| data.get_temp::<Option<u8>>(sounding_id))
        .flatten();
    let mut output = KeyboardOutput::default();
    let mut next = sounding;
    let pointer = response.interact_pointer_pos();
    if response.is_pointer_button_down_on() {
        let note = pointer.and_then(note_at);
        if note != sounding {
            output.released = sounding;
            if let (Some(note), Some(pointer)) = (note, pointer) {
                output.pressed = Some((note, pad_velocity(rect, pointer.y)));
            }
            next = note;
        }
    } else if sounding.is_some() {
        output.released = sounding;
        next = None;
    }
    ui.data_mut(|data| data.insert_temp(sounding_id, next));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let is_held = |note: u8| held.contains(&note) || next == Some(note);
        for (index, note) in whites.iter().enumerate() {
            let key = white_rect(index);
            painter.rect_filled(
                key,
                0.0,
                if is_held(*note) {
                    colors.accent
                } else {
                    colors.raised
                },
            );
            painter.rect_stroke(
                key,
                0.0,
                Stroke::new(1.0_f32, colors.line),
                StrokeKind::Inside,
            );
            if root == Some(*note) {
                painter.circle_filled(
                    egui::pos2(key.center().x, key.bottom() - 8.0),
                    3.0,
                    colors.action,
                );
            }
        }
        for note in first..first + octaves * 12 {
            if BLACK_KEYS[usize::from(note % 12)] {
                let key = black_rect(note);
                painter.rect_filled(
                    key,
                    2.0,
                    if is_held(note) {
                        colors.accent
                    } else {
                        colors.bg
                    },
                );
                painter.rect_stroke(
                    key,
                    2.0,
                    Stroke::new(1.0_f32, colors.line),
                    StrokeKind::Inside,
                );
            }
        }
    }
    output
}

/// A modulation source being dragged onto a destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModSource(pub u32);

/// A draggable modulation source (LFO 1, Envelope 2 …). Drop it on any knob
/// built with [`LabeledKnob::mod_sources`]; the knob's context menu offers the
/// same assignment without dragging.
pub fn mod_source_chip(ui: &mut Ui, id: u32, label: &str) -> Response {
    let colors = theme::palette(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), TextRole::Label.font(), colors.text);
    let size = vec2(
        galley.size().x + 2.0 * tokens::SPACE_MD,
        tokens::COMPACT_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::drag());
    let response = response.on_hover_cursor(egui::CursorIcon::Grab);
    if response.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), ModSource(id));
    }
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            tokens::RADIUS,
            colors
                .modulation
                .gamma_multiply(if response.hovered() { 0.32 } else { 0.2 }),
            Stroke::new(tokens::BORDER_WIDTH, colors.modulation),
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(rect.center() - galley.size() * 0.5, galley, colors.text);
    }
    response.widget_info(|| {
        WidgetInfo::labeled(
            WidgetType::Button,
            true,
            format!("Modulation source {label}"),
        )
    });
    response
}

/// One matrix row: source, destination, amount.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModSlot {
    pub source: Option<u32>,
    pub destination: Option<u32>,
    /// Bipolar depth, −1 to 1.
    pub amount: f64,
}

pub struct ModSlotOutput {
    pub response: Response,
    pub removed: bool,
}

/// A row of the modulation matrix. `sources` and `destinations` are
/// `(id, name)` pairs; an empty slot reads "None".
pub fn mod_slot_row(
    ui: &mut Ui,
    slot: &mut ModSlot,
    sources: &[(u32, &str)],
    destinations: &[(u32, &str)],
) -> ModSlotOutput {
    let name = |list: &[(u32, &str)], id: Option<u32>| {
        id.and_then(|id| list.iter().find(|(candidate, _)| *candidate == id))
            .map_or("None".to_owned(), |(_, name)| (*name).to_owned())
    };
    let mut changed = false;
    let mut removed = false;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = tokens::SPACE_SM;
        for (is_source, list) in [(true, sources), (false, destinations)] {
            let current = if is_source {
                slot.source
            } else {
                slot.destination
            };
            crate::choice_menu(ui, name(list, current), |ui| {
                for (id, label) in list {
                    if crate::selectable(ui, current == Some(*id), *label).clicked() {
                        if is_source {
                            slot.source = Some(*id)
                        } else {
                            slot.destination = Some(*id)
                        }
                        changed = true;
                        ui.close();
                    }
                }
            });
            if is_source {
                crate::text::label(ui, "→");
            }
        }
        let mut amount = slot.amount;
        let knob = Knob::new(&mut amount, -1.0..=1.0)
            .compact()
            .pointer()
            .bipolar()
            .default_value(0.0)
            .diameter(34.0);
        let response = ui.add(knob);
        if response.changed() {
            slot.amount = amount;
            changed = true;
        }
        crate::text::note(ui, format!("{:+.0} %", slot.amount * 100.0));
        if ui
            .add(IconButton::icon_only(Icon::Close, "Remove modulation"))
            .clicked()
        {
            removed = true;
        }
    });
    let mut response = row.response;
    if changed {
        response.mark_changed();
    }
    ModSlotOutput { response, removed }
}

/// A vertical performance wheel. The pitch wheel (`spring`, range −1 to 1)
/// returns to center on release; the mod wheel (0 to 1) stays where it is.
/// Double-click resets to the rest value; arrow keys nudge when focused.
/// Drag motion is relative, with Shift for fine control.
pub fn wheel(ui: &mut Ui, value: &mut f32, bipolar: bool, label: &str, size: Vec2) -> Response {
    let colors = theme::palette(ui.ctx());
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let (low, rest) = if bipolar { (-1.0, 0.0) } else { (0.0, 0.0) };
    let before = *value;
    let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
    let span = 1.0 - low;
    if response.dragged() {
        *value += -response.drag_delta().y / rect.height() * span * fine;
    }
    if response.clicked() {
        response.request_focus();
    }
    if response.has_focus() {
        ui.input_mut(|input| {
            for (key, direction) in [(egui::Key::ArrowUp, 1.0), (egui::Key::ArrowDown, -1.0)] {
                if input.consume_key(egui::Modifiers::NONE, key)
                    || input.consume_key(egui::Modifiers::SHIFT, key)
                {
                    *value += direction * 0.02 * span * fine;
                }
            }
        });
    }
    if response.double_clicked() || (bipolar && response.drag_stopped()) {
        *value = rest;
    }
    *value = value.clamp(low, 1.0);
    if *value != before {
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, tokens::RADIUS, colors.bg);
        let fraction = (*value - low) / span;
        let ridge = rect.bottom() - rect.height() * fraction;
        for step in 0..9 {
            let y = rect.top() + rect.height() * (step as f32 + 0.5) / 9.0;
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 3.0, y),
                    egui::pos2(rect.right() - 3.0, y),
                ],
                Stroke::new(1.0_f32, colors.grid_minor),
            );
        }
        if bipolar {
            painter.line_segment(
                [rect.left_center(), rect.right_center()],
                Stroke::new(1.0_f32, colors.grid_bar),
            );
        }
        painter.rect_filled(
            Rect::from_center_size(
                egui::pos2(
                    rect.center().x,
                    ridge.clamp(rect.top() + 4.0, rect.bottom() - 4.0),
                ),
                vec2(rect.width() - 4.0, 9.0),
            ),
            2.0,
            if response.dragged() {
                colors.accent
            } else {
                colors.control_hover
            },
        );
        painter.rect_stroke(
            rect,
            tokens::RADIUS,
            Stroke::new(
                if response.has_focus() {
                    tokens::FOCUS_WIDTH
                } else {
                    tokens::BORDER_WIDTH
                },
                if response.has_focus() {
                    colors.text
                } else {
                    colors.line
                },
            ),
            StrokeKind::Inside,
        );
    }
    let hint = if bipolar {
        "Drag to bend; releases to center"
    } else {
        "Drag to modulate"
    };
    response.widget_info(|| WidgetInfo::slider(true, f64::from(*value), label));
    response.on_hover_text(format!("{label} · {hint}. Double-click to reset."))
}

/// Width of a [`fader`]'s cap; the fader may be given more, and centers.
pub const FADER_CAP_WIDTH: f32 = tokens::HIT_TARGET;

/// A vertical fader over a normalized value (0 at the bottom): a rail whose
/// lower part is highlighted up to a cap. For values read as a shape across
/// neighbours, as the four stages of an envelope; a knob everywhere else.
/// Drag motion is relative, with Shift for fine control; arrow keys nudge
/// when focused. The caller supplies caption, value text and hover.
pub fn fader(
    ui: &mut Ui,
    value: &mut f64,
    label: &str,
    size: Vec2,
    automation: Option<AutomationState>,
) -> Response {
    let colors = theme::palette(ui.ctx());
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let before = *value;
    let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
    let cap = vec2(FADER_CAP_WIDTH.min(rect.width()), 11.0);
    // The cap's center travels between the rail's ends.
    let travel = (rect.height() - cap.y).max(1.0);
    if ui.is_enabled() {
        // The pointer's last applied height, kept here because egui reports
        // no drag delta on the release frame: motion between the last held
        // frame and the release must still count.
        let drag_id = response.id.with("fader_drag");
        let pointer = response.interact_pointer_pos().map(|position| position.y);
        if response.drag_started_by(egui::PointerButton::Primary) {
            let start = pointer.map(|y| y - response.total_drag_delta().unwrap_or_default().y);
            ui.data_mut(|data| data.insert_temp(drag_id, start));
        }
        if response.dragged_by(egui::PointerButton::Primary)
            || response.drag_stopped_by(egui::PointerButton::Primary)
        {
            let last = ui
                .data(|data| data.get_temp::<Option<f32>>(drag_id))
                .flatten();
            if let Some((y, last)) = pointer.zip(last) {
                *value += f64::from((last - y) / travel) * fine;
            }
            ui.data_mut(|data| data.insert_temp(drag_id, pointer));
        }
        if response.clicked() {
            response.request_focus();
        }
        if response.has_focus() {
            ui.input_mut(|input| {
                for (key, direction) in [(egui::Key::ArrowUp, 1.0), (egui::Key::ArrowDown, -1.0)] {
                    if input.consume_key(egui::Modifiers::NONE, key)
                        || input.consume_key(egui::Modifiers::SHIFT, key)
                    {
                        *value += direction * 0.02 * fine;
                    }
                }
            });
        }
    }
    // Undo and redo keep working while the fader has keyboard focus.
    crate::parameter_focus::register_parameter_history_focus(&response);
    *value = value.clamp(0.0, 1.0);
    if *value != before {
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let active = if ui.is_enabled() {
            automation.map_or(colors.accent, |state| state.color(ui.ctx()))
        } else {
            colors.muted
        };
        let x = rect.center().x;
        let (top, bottom) = (rect.top() + cap.y * 0.5, rect.bottom() - cap.y * 0.5);
        let y = bottom - travel * *value as f32;
        let rail = |from: f32, to: f32, color: Color32| {
            painter.line_segment(
                [egui::pos2(x, from), egui::pos2(x, to)],
                Stroke::new(4.5_f32, color),
            );
        };
        rail(top, bottom, colors.line);
        // As soft as the value arc of a `Knob::pointer`.
        rail(y, bottom, active.gamma_multiply(0.7));
        let cap = Rect::from_center_size(egui::pos2(x, y), cap);
        painter.rect(
            cap,
            3.0,
            if response.hovered() || response.dragged() {
                colors.control_hover
            } else {
                colors.control_fill
            },
            Stroke::new(
                if response.has_focus() {
                    tokens::FOCUS_WIDTH
                } else {
                    tokens::BORDER_WIDTH
                },
                if response.has_focus() {
                    colors.text
                } else {
                    colors.line
                },
            ),
            StrokeKind::Inside,
        );
        painter.line_segment(
            [
                egui::pos2(cap.left() + 4.0, y),
                egui::pos2(cap.right() - 4.0, y),
            ],
            Stroke::new(2.0_f32, colors.text),
        );
    }
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), *value, label));
    response
}
