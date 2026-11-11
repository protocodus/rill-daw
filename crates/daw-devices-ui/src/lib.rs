//! Panels for Rill's built-in devices, composed only from `daw-ui` components.
//!
//! A panel talks to whatever owns the parameter values through [`DeviceHost`],
//! so the same panel runs in the app, in the gallery and in tests. See
//! `docs/R1_DEVICES.md#panel-host`.
use daw_devices::{DeviceDescriptor, DeviceKind, ParamRange, ParamSpec, Preset, factory_presets};
use daw_ui::{AutomationControl, AutomationNavigation, AutomationState};
use egui::Ui;

mod panels;

/// The panel's view of the parameter owner.
pub trait DeviceHost {
    /// Normalized 0–1.
    fn value(&self, id: u32) -> f64;
    /// One gesture on one control. Consecutive changes of the same parameter
    /// form one Undo step.
    fn set_value(&mut self, id: u32, normalized: f64);
    /// Several parameters at once (preset, reset): one Undo step.
    fn set_values(&mut self, values: &[(u32, f64)]);
    /// The automation lane and its state, when the parameter is automated.
    fn automation(&self, id: u32) -> Option<(u64, AutomationState)>;
    /// Create, edit, override or resume automation. `Create` carries
    /// [`AutomationControl::DeviceParameter`].
    fn navigate(&mut self, action: AutomationNavigation);
    /// Presets the user saved for this device.
    fn user_presets(&self) -> Vec<Preset>;
    fn save_preset(&mut self, preset: Preset);
    /// Instruments: sound one note (MIDI pitch, velocity 0–1) from the panel
    /// keyboard until the next call; `None` releases it.
    fn audition(&mut self, note: Option<(u8, f32)>);
    /// Instruments: the panel's pitch wheel (−1–1) and mod wheel (0–1).
    fn wheels(&mut self, pitch: f32, modulation: f32);
}

/// Per-panel UI state the host keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct PanelState {
    /// Name of the last loaded or saved preset.
    pub preset: Option<String>,
    /// Draft name while "Save as…" is open.
    pub save_name: Option<String>,
    /// Pitch wheel, −1–1; springs back to 0 on release.
    pub pitch_wheel: f32,
    /// Mod wheel, 0–1; stays where it is left.
    pub mod_wheel: f32,
    /// Last drawn pitch wheel, mod wheel and keyboard, for host diagnostics.
    pub performance_rects: Option<[egui::Rect; 3]>,
}

/// The hero knob's diameter from the panel grammar.
pub const KNOB_HERO: f32 = 64.0;
/// Space between neighbouring controls of one card, so captions never touch.
const CELL_GAP: f32 = daw_ui::tokens::SPACE_SM;

/// Extra space between the lines of a cell-grid card, so a value reads with
/// the knob above it and not with the caption below.
const LINE_GAP: f32 = daw_ui::tokens::SPACE_SM;

/// The panel's width, for the window that hosts it.
pub fn panel_width(descriptor: &DeviceDescriptor) -> f32 {
    panels::layout(descriptor).width
}

/// Draws a device's panel: header with presets, the response, then the
/// sections as cards in signal order, side by side where they share a row.
/// Instruments end with an audition keyboard and pitch and mod wheels.
pub fn show(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    let layout = panels::layout(descriptor);
    ui.set_width(layout.width.min(ui.available_width()));
    preset_bar(ui, descriptor, host, state);
    daw_ui::section_gap(ui);
    if let Some(view) = &layout.response {
        response(ui, descriptor, view, host);
        daw_ui::section_gap(ui);
    }
    let rows = layout.sections.iter().map(|section| section.row).max();
    for row in 0..=rows.unwrap_or(0) {
        let sections: Vec<_> = layout
            .sections
            .iter()
            .filter(|section| section.row == row)
            .collect();
        let drawn = if layout.cells {
            cell_row(ui, descriptor, &sections, host, &layout);
            true
        } else {
            natural_row(ui, descriptor, &sections, host, &layout)
        };
        if drawn {
            // The gutter between rows equals the one between cards.
            ui.add_space((daw_ui::tokens::SPACE_SM - ui.spacing().item_spacing.y).max(0.0));
        }
    }
    if descriptor.kind == DeviceKind::Instrument {
        daw_ui::section_gap(ui);
        performance(ui, host, state);
    }
}

/// One row of cards whose controls take the widths their captions and
/// values need. `false` when none of its controls shows.
fn natural_row(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    sections: &[&panels::Section],
    host: &mut dyn DeviceHost,
    layout: &panels::Layout,
) -> bool {
    let cards: Vec<_> = sections
        .iter()
        .filter_map(|section| card(ui, descriptor, section, host, layout))
        .collect();
    if cards.is_empty() {
        return false;
    }
    // One center line per line of the row, so knobs, choices and
    // switches in neighbouring cards line up.
    let lines = cards.iter().map(|card| card.lines.len()).max().unwrap_or(0);
    let heights: Vec<f32> = (0..lines)
        .map(|line| {
            cards
                .iter()
                .filter_map(|card| card.lines.get(line))
                .flatten()
                .map(|(spec, _)| control_height(spec, layout))
                .fold(daw_ui::tokens::CONTROL_HEIGHT, f32::max)
        })
        .collect();
    let spans: Vec<u8> = cards.iter().map(|card| card.span).collect();
    daw_ui::grid_row(ui, &spans, |ui, index| {
        let card = &cards[index];
        daw_ui::device_section_title(ui, card.title);
        ui.add_space(daw_ui::tokens::SPACE_XS);
        let available = ui.available_width();
        let lines = wrap(&card.lines, available);
        for (index, line) in lines.iter().enumerate() {
            let height = heights
                .get(index)
                .or(heights.last())
                .copied()
                .unwrap_or(0.0);
            // Lines with as many controls share their columns, as in
            // a table; a line of another length spreads on its own.
            let peers: Vec<_> = lines
                .iter()
                .filter(|peer| peer.len() == line.len())
                .cloned()
                .collect();
            let columns = columns(&peers, available);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for ((spec, natural), width) in line.iter().zip(&columns) {
                    let cell = Cell {
                        width: width.unwrap_or(*natural),
                        natural: *natural,
                        height,
                        lead: 0.0,
                        fader: false,
                    };
                    control(ui, descriptor, spec, host, cell, layout);
                }
            });
        }
    });
    true
}

/// One row of cards on the cell grid: a card `span` cells wide holds that
/// many equal cells per line, so controls stand in columns within a card
/// and on shared center lines across the row.
fn cell_row(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    sections: &[&panels::Section],
    host: &mut dyn DeviceHost,
    layout: &panels::Layout,
) {
    let lines = sections
        .iter()
        .map(|section| section.lines.len())
        .max()
        .unwrap_or(0);
    let heights: Vec<f32> = (0..lines)
        .map(|line| {
            sections
                .iter()
                .filter_map(|section| section.lines.get(line))
                .flatten()
                .filter_map(|slot| match slot {
                    panels::Slot::Control(id, _) => descriptor.parameter(*id),
                    _ => None,
                })
                .map(|spec| control_height(spec, layout))
                .fold(daw_ui::tokens::CONTROL_HEIGHT, f32::max)
        })
        .collect();
    // What a line adds to its control: the caption above, the value below
    // and the spacing around them. A fader spans the lines it stands beside.
    let text = ui.fonts_mut(|fonts| fonts.row_height(&daw_ui::control_caption_font()));
    let between = 2.0 * text + 3.0 * ui.spacing().item_spacing.y + LINE_GAP;
    let tall = heights.iter().sum::<f32>() + between * lines.saturating_sub(1) as f32;
    let spans: Vec<u8> = sections.iter().map(|section| section.span).collect();
    daw_ui::grid_row(ui, &spans, |ui, index| {
        let section = sections[index];
        daw_ui::device_section_title(ui, section.title);
        ui.add_space(daw_ui::tokens::SPACE_XS);
        let width = ui.available_width() / f32::from(section.span);
        for (line, slots) in section.lines.iter().enumerate() {
            if line > 0 {
                ui.add_space(LINE_GAP);
            }
            // In a card of one line beside taller ones, faders span every
            // line and a knob stands on the last, level with its neighbours.
            let alone = section.lines.len() == 1;
            let last = heights.last().copied().unwrap_or(0.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for slot in slots {
                    let (ids, cells, fader) = match slot {
                        panels::Slot::Control(id, cells) => {
                            (std::slice::from_ref(id), cells, false)
                        }
                        panels::Slot::Faders(ids, cells) => (*ids, cells, true),
                        panels::Slot::Blank(cells) => {
                            ui.add_space(width * f32::from(*cells));
                            continue;
                        }
                    };
                    let width = width * f32::from(*cells) / ids.len() as f32;
                    for spec in ids.iter().filter_map(|id| descriptor.parameter(*id)) {
                        let cell = Cell {
                            width,
                            natural: width,
                            height: match (alone, fader) {
                                (true, true) => tall,
                                (true, false) => last,
                                (false, _) => heights[line],
                            },
                            lead: if alone && !fader { tall - last } else { 0.0 },
                            fader,
                        };
                        control(ui, descriptor, spec, host, cell, layout);
                    }
                }
            });
        }
    });
}

/// A section's visible controls, line by line, with their natural widths.
struct Card<'a> {
    title: &'static str,
    lines: Vec<Vec<(&'a ParamSpec, f32)>>,
    /// The card's width in grid columns.
    span: u8,
}

fn card<'a>(
    ui: &Ui,
    descriptor: &'a DeviceDescriptor,
    section: &panels::Section,
    host: &dyn DeviceHost,
    layout: &panels::Layout,
) -> Option<Card<'a>> {
    let lines: Vec<Vec<_>> = section
        .lines
        .iter()
        .map(|line| {
            line.iter()
                .flat_map(|slot| slot.ids())
                .filter_map(|id| descriptor.parameter(*id))
                .filter(|spec| layout.keep_inactive || active(spec, host, layout))
                .map(|spec| (spec, cell_width(ui, spec, layout)))
                .collect::<Vec<_>>()
        })
        .filter(|line| !line.is_empty())
        .collect();
    (!lines.is_empty()).then_some(Card {
        title: section.title,
        lines,
        span: section.span,
    })
}

/// Whether the control does anything in the device's current mode.
fn active(spec: &ParamSpec, host: &dyn DeviceHost, layout: &panels::Layout) -> bool {
    (layout.visible)(spec.id, &|id| host.value(id))
}

/// The columns of a choice whose toggle buttons form a grid.
fn toggle_columns(spec: &ParamSpec, layout: &panels::Layout) -> Option<usize> {
    layout
        .toggle_columns
        .iter()
        .find(|(id, _)| *id == spec.id && toggles(spec))
        .map(|(_, columns)| *columns)
}

/// Whether a parameter's options are few enough to show as toggle buttons.
fn toggles(spec: &ParamSpec) -> bool {
    matches!(spec.range, ParamRange::Choice(labels) if labels.len() < daw_ui::TOGGLE_OPTION_LIMIT)
}

/// Lines too wide for `width` continue on the next, so a device without
/// planned lines still fits its panel.
fn wrap<T: Copy>(lines: &[Vec<(T, f32)>], width: f32) -> Vec<Vec<(T, f32)>> {
    let mut wrapped = Vec::new();
    for line in lines {
        let mut current: Vec<(T, f32)> = Vec::new();
        let mut used = 0.0;
        for &(item, cell) in line {
            if !current.is_empty() && used + cell > width {
                wrapped.push(std::mem::take(&mut current));
                used = 0.0;
            }
            current.push((item, cell));
            used += cell;
        }
        wrapped.push(current);
    }
    wrapped
}

/// Columns as wide as their widest control, as in a table.
fn grid_width<T>(lines: &[Vec<(T, f32)>]) -> f32 {
    let columns = lines.iter().map(Vec::len).max().unwrap_or(0);
    (0..columns)
        .map(|column| {
            lines
                .iter()
                .filter_map(|line| line.get(column))
                .map(|(_, width)| *width)
                .fold(0.0, f32::max)
        })
        .sum()
}

/// Column widths that align the lines of a card and share its leftover
/// width evenly. `None` when the aligned columns do not fit: each control
/// then keeps its natural width.
fn columns<T>(lines: &[Vec<(T, f32)>], available: f32) -> Vec<Option<f32>> {
    let count = lines.iter().map(Vec::len).max().unwrap_or(0);
    let used = grid_width(lines);
    if used > available {
        return vec![None; count];
    }
    let extra = (available - used) / count.max(1) as f32;
    (0..count)
        .map(|column| {
            let natural = lines
                .iter()
                .filter_map(|line| line.get(column))
                .map(|(_, width)| *width)
                .fold(0.0, f32::max);
            Some(natural + extra)
        })
        .collect()
}

/// The name the panel shows for a parameter: its own, unless the layout
/// moved it to a card where another name reads right.
fn caption(spec: &ParamSpec, layout: &panels::Layout) -> &'static str {
    layout
        .captions
        .iter()
        .find(|(id, _)| *id == spec.id)
        .map_or(spec.name, |(_, caption)| caption)
}

/// A parameter's value as the panel prints it: the device's own text, less
/// the plus sign where the range has no negative side to tell it from.
fn value_text(spec: &ParamSpec, normalized: f32) -> String {
    let text = spec.format(normalized);
    match text.strip_prefix('+') {
        Some(unsigned) if spec.plain_min() >= 0.0 => unsigned.to_owned(),
        _ => text,
    }
}

/// The value at one end of a knob's range, short enough to print under the
/// arc: "20", "20k", "+24". The unit stays only where the two ends differ
/// in more than its prefix ("1ms", "2s"); the full value is under the knob.
fn end_label(spec: &ParamSpec, normalized: f32) -> String {
    let split = |normalized: f32| {
        let text = value_text(spec, normalized);
        let (number, unit) = text.split_once(' ').unwrap_or((&text, ""));
        let number = if number.contains('.') {
            number.trim_end_matches('0').trim_end_matches('.')
        } else {
            number
        };
        let number = if number == "+0" { "0" } else { number };
        (number.to_owned(), unit.to_owned())
    };
    let ((number, unit), (_, low), (_, high)) = (split(normalized), split(0.0), split(1.0));
    if low == high {
        number
    } else if high.strip_prefix('k') == Some(low.as_str()) {
        // "20" to "20k": the kilo prefix alone tells the ends apart.
        format!("{number}{}", if unit == high { "k" } else { "" })
    } else {
        format!("{number}{unit}")
    }
}

fn knob_diameter(spec: &ParamSpec, layout: &panels::Layout) -> f32 {
    if layout.hero == Some(spec.id) {
        KNOB_HERO
    } else {
        layout.knob.diameter()
    }
}

fn is_knob(spec: &ParamSpec) -> bool {
    matches!(
        spec.range,
        ParamRange::Linear { .. } | ParamRange::Log { .. } | ParamRange::Integer { .. }
    )
}

/// Height of the control itself, between its caption and its value.
fn control_height(spec: &ParamSpec, layout: &panels::Layout) -> f32 {
    if is_knob(spec) {
        knob_diameter(spec, layout)
    } else if let (Some(columns), ParamRange::Choice(labels)) =
        (toggle_columns(spec, layout), spec.range)
    {
        daw_ui::TOGGLE_ROW_HEIGHT * labels.len().div_ceil(columns) as f32
    } else {
        daw_ui::tokens::CONTROL_HEIGHT
    }
}

/// The narrowest cell that shows a control's caption, and its widest value
/// whole, with a gap to its neighbours.
fn cell_width(ui: &Ui, spec: &ParamSpec, layout: &panels::Layout) -> f32 {
    let text = |value: &str, font: egui::FontId| {
        ui.painter()
            .layout_no_wrap(value.to_owned(), font, egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    let caption = text(caption(spec, layout), daw_ui::control_caption_font());
    let button = egui::TextStyle::Button.resolve(ui.style());
    let widest = |values: &mut dyn Iterator<Item = String>| {
        values
            .map(|value| text(&value, button.clone()))
            .fold(0.0, f32::max)
    };
    // Text, padding on both sides and the chevron, as the choice draws them.
    let spacing = ui.spacing();
    let choice = |text: f32| {
        text + spacing.icon_spacing + spacing.icon_width + 2.0 * daw_ui::tokens::SPACE_SM
    };
    let control = match spec.range {
        // The knob, or its value below it at either end of the range.
        ParamRange::Linear { .. } | ParamRange::Log { .. } | ParamRange::Integer { .. } => {
            [0.0, 1.0]
                .into_iter()
                .map(|end| text(&value_text(spec, end), daw_ui::text::TextRole::Note.font()))
                .fold(knob_diameter(spec, layout), f32::max)
        }
        ParamRange::Choice(labels) if toggles(spec) => {
            let widths = labels
                .iter()
                .map(|label| text(label, button.clone()) + 2.0 * daw_ui::tokens::SPACE_SM);
            match toggle_columns(spec, layout) {
                Some(columns) => widths.fold(0.0, f32::max) * columns as f32,
                None => widths.sum(),
            }
        }
        ParamRange::Choice(labels) => choice(widest(&mut labels.iter().map(|l| l.to_string()))),
        ParamRange::Toggle => daw_ui::tokens::SWITCH_WIDTH,
    };
    caption.max(control) + CELL_GAP
}

const KEYBOARD_FIRST: u8 = 36;
const KEYBOARD_OCTAVES: u8 = 4;
const KEYBOARD_HEIGHT: f32 = 55.0;
const WHEEL_WIDTH: f32 = 21.0;

/// Pitch and mod wheels beside an audition keyboard. The wheels apply to
/// every note the instrument plays while it is previewed, not only to the
/// keyboard's.
fn performance(ui: &mut Ui, host: &mut dyn DeviceHost, state: &mut PanelState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_XS;
        let size = egui::vec2(WHEEL_WIDTH, KEYBOARD_HEIGHT);
        let pitch = daw_ui::wheel(ui, &mut state.pitch_wheel, true, "Pitch", size)
            .on_hover_text("Pitch wheel: drag to bend; springs back on release");
        let modulation = daw_ui::wheel(ui, &mut state.mod_wheel, false, "Mod", size)
            .on_hover_text("Mod wheel: drag to set; double-click to reset");
        if pitch.changed() || modulation.changed() {
            host.wheels(state.pitch_wheel, state.mod_wheel);
        }
        let width = ui.available_width();
        let keyboard =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, KEYBOARD_HEIGHT));
        state.performance_rects = Some([pitch.rect, modulation.rect, keyboard]);
        let keys = daw_ui::keyboard_strip(
            ui,
            KEYBOARD_FIRST,
            KEYBOARD_OCTAVES,
            &[],
            Some(60),
            egui::vec2(width, KEYBOARD_HEIGHT),
        );
        if let Some(note) = keys.pressed {
            host.audition(Some(note));
        } else if keys.released.is_some() {
            host.audition(None);
        }
    });
}

const RESPONSE_POINTS: usize = 128;
const RESPONSE_RANGE_DB: f32 = 24.0;
const RESPONSE_HEIGHT: f32 = 160.0;

/// The device's response with a node on its frequency parameter. Dragging the
/// node is one gesture on that parameter, like turning its knob.
fn response(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    view: &panels::ResponseView,
    host: &mut dyn DeviceHost,
) {
    let frequencies: Vec<f32> = (0..RESPONSE_POINTS)
        .map(|index| daw_ui::log_grid_frequency(index, RESPONSE_POINTS))
        .collect();
    let mut db = vec![0.0; RESPONSE_POINTS];
    (view.curve)(&|id| host.value(id), &frequencies, &mut db);
    let Some(spec) = descriptor.parameter(view.node) else {
        return;
    };
    let frequency = spec.to_plain(host.value(view.node) as f32);
    let output = daw_ui::frequency_plot(
        ui,
        &daw_ui::FrequencyPlot {
            curves: &[daw_ui::FrequencyCurve {
                db: &db,
                emphasis: true,
            }],
            spectrum: None,
            nodes: &[daw_ui::FrequencyNode {
                frequency,
                gain_db: 0.0,
                selected: false,
                color: None,
                gain_editable: false,
            }],
            range_db: RESPONSE_RANGE_DB,
            size: egui::vec2(ui.available_width(), RESPONSE_HEIGHT),
        },
    );
    if let Some(drag) = output.drags.last() {
        host.set_value(view.node, f64::from(spec.to_normalized(drag.frequency)));
    }
}

/// A control's cell: its width in the line, its own natural width, which a
/// choice keeps, the height of the line's control band, the space above
/// its caption, and whether a continuous value is a fader filling that band
/// instead of a knob.
struct Cell {
    width: f32,
    natural: f32,
    height: f32,
    lead: f32,
    fader: bool,
}

/// One control chosen by the parameter's range, centered in its cell, on the
/// line's center line below its caption.
fn control(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    spec: &ParamSpec,
    host: &mut dyn DeviceHost,
    cell: Cell,
    layout: &panels::Layout,
) {
    let Cell {
        width,
        natural,
        height,
        lead,
        fader,
    } = cell;
    let normalized = host.value(spec.id);
    let automation = host.automation(spec.id);
    let id = egui::Id::new((descriptor.id, spec.id));
    let control = AutomationControl::DeviceParameter { id: spec.id };
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            if !active(spec, host, layout) {
                ui.disable();
            }
            // Compact horizontal padding leaves a choice's text the room of its cell.
            ui.spacing_mut().button_padding.x = daw_ui::tokens::SPACE_SM;
            ui.add_space(lead);
            daw_ui::control_caption(ui, caption(spec, layout));
            let own = if fader {
                height
            } else {
                control_height(spec, layout)
            };
            let offset = (height - own) * 0.5;
            if offset > 0.0 {
                ui.add_space(offset);
            }
            let navigation = match spec.range {
                ParamRange::Linear { .. } | ParamRange::Log { .. } | ParamRange::Integer { .. } => {
                    let mut value = normalized;
                    let state = automation.map(|(_, state)| state);
                    let response = if fader {
                        let size = egui::vec2(daw_ui::FADER_CAP_WIDTH, height);
                        daw_ui::fader(ui, &mut value, spec.name, size, state)
                    } else {
                        let mut knob = daw_ui::Knob::new(&mut value, 0.0..=1.0)
                            .compact()
                            .pointer()
                            .diameter(knob_diameter(spec, layout))
                            .text(caption(spec, layout))
                            .endpoint_labels(end_label(spec, 0.0), end_label(spec, 1.0))
                            .automation(state);
                        if spec.plain_min() == -spec.plain_max() {
                            // A range around zero fills from its center.
                            knob = knob.bipolar();
                        }
                        if let ParamRange::Integer { min, max } = spec.range {
                            // One detent per whole value.
                            knob = knob.step_by(1.0 / f64::from((max - min).max(1)));
                        }
                        ui.add(knob)
                    };
                    let text = value_text(spec, normalized as f32);
                    // Values share one baseline across the line.
                    ui.add_space(offset);
                    daw_ui::text::note(ui, &text);
                    let (action, exact) = daw_ui::automatable_parameter_value_context_named(
                        ui,
                        &response,
                        control,
                        automation,
                        f64::from(spec.to_plain(normalized as f32)),
                        f64::from(spec.plain_min())..=f64::from(spec.plain_max()),
                        spec.unit,
                        spec.name,
                    );
                    if let Some(plain) = exact {
                        host.set_value(spec.id, f64::from(spec.to_normalized(plain as f32)));
                    } else if response.changed() {
                        host.set_value(spec.id, value.clamp(0.0, 1.0));
                    }
                    action
                }
                ParamRange::Choice(labels) => {
                    // Option indices with their labels.
                    let offered = (layout.options)(spec.id, &|id| host.value(id)).unwrap_or(labels);
                    let options: Vec<(f32, String)> = offered
                        .iter()
                        .map(|option| {
                            let index = labels.iter().position(|o| o == option);
                            (index.unwrap_or(0) as f32, (*option).to_string())
                        })
                        .collect();
                    let mut current = spec.to_plain(normalized as f32);
                    let label = spec.format(normalized as f32);
                    let response = if toggles(spec) {
                        let options: Vec<(f32, &str)> = options
                            .iter()
                            .map(|(plain, option)| (*plain, option.as_str()))
                            .collect();
                        let response = match toggle_columns(spec, layout) {
                            Some(columns) => {
                                daw_ui::toggle_grid(ui, &mut current, &options, columns)
                            }
                            None => daw_ui::toggle_group(ui, &mut current, &options),
                        };
                        if response.changed() {
                            host.set_value(spec.id, f64::from(spec.to_normalized(current)));
                        }
                        response
                    } else {
                        let size = egui::vec2(natural - CELL_GAP, daw_ui::tokens::CONTROL_HEIGHT);
                        // A child of the choice's own width, which the cell centers.
                        ui.allocate_ui_with_layout(
                            size,
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                daw_ui::select(id, &label)
                                    .width(size.x)
                                    // Long names ("Transistor Ladder") must not push the
                                    // next control out of its cell; the list shows them whole.
                                    .truncate()
                                    .show_ui(ui, |ui| {
                                        for (plain, option) in &options {
                                            if daw_ui::selectable(ui, *plain == current, option)
                                                .clicked()
                                            {
                                                host.set_value(
                                                    spec.id,
                                                    f64::from(spec.to_normalized(*plain)),
                                                );
                                            }
                                        }
                                    })
                                    .response
                            },
                        )
                        .inner
                        .on_hover_text(&label)
                    };
                    daw_ui::automatable_parameter_context(ui, &response, control, automation)
                }
                ParamRange::Toggle => {
                    let mut on = normalized >= 0.5;
                    let response = daw_ui::switch(ui, &mut on, "");
                    if response.changed() {
                        host.set_value(spec.id, f64::from(u8::from(on)));
                    }
                    daw_ui::automatable_parameter_context(ui, &response, control, automation)
                }
            };
            if !is_knob(spec) {
                // An empty value line, so every cell of a line is as tall as
                // a knob's and the next line starts level across cards.
                ui.add_space(offset);
                daw_ui::text::note(ui, " ");
            }
            if let Some(action) = navigation {
                host.navigate(action);
            }
        },
    );
}

/// The device's name beside its `daw_ui::preset_group`. Identical for every
/// device.
fn preset_bar(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    let factory = factory_presets(descriptor);
    let user = host.user_presets();
    let presets: Vec<&Preset> = factory.iter().chain(&user).collect();
    let current = state
        .preset
        .as_ref()
        .and_then(|name| presets.iter().position(|preset| &preset.name == name));
    let modified = current.is_none_or(|index| {
        presets[index].resolve(descriptor).is_ok_and(|resolved| {
            resolved
                .values
                .iter()
                .any(|(id, value)| (host.value(*id) - f64::from(*value)).abs() > 1e-4)
        })
    });
    let load = |host: &mut dyn DeviceHost, state: &mut PanelState, preset: &Preset| {
        if let Ok(resolved) = preset.resolve(descriptor) {
            let values: Vec<_> = resolved
                .values
                .iter()
                .map(|(id, value)| (*id, f64::from(*value)))
                .collect();
            host.set_values(&values);
            state.preset = Some(preset.name.clone());
        }
    };
    let name = state.preset.clone().unwrap_or_else(|| "Custom".into());
    let name = if modified && state.preset.is_some() {
        format!("{name} *")
    } else {
        name
    };
    let labels: Vec<String> = presets
        .iter()
        .map(|preset| {
            if preset.category.is_empty() {
                preset.name.clone()
            } else {
                format!("{} · {}", preset.category, preset.name)
            }
        })
        .collect();
    let choices: Vec<_> = presets
        .iter()
        .zip(&labels)
        .map(|(preset, label)| daw_ui::PresetChoice {
            label,
            description: &preset.description,
        })
        .collect();
    let step = |offset: isize| {
        let count = presets.len() as isize;
        let from = current.map_or(if offset > 0 { -1 } else { 0 }, |index| index as isize);
        (count > 0).then(|| (from + offset).rem_euclid(count) as usize)
    };
    let saving = state.save_name.is_some();
    // As tall as the group, so the name centers on it.
    let height = daw_ui::tokens::CONTROL_HEIGHT + 2.0 * daw_ui::tokens::SPACE_SM;
    let action = ui
        .allocate_ui_with_layout(
            egui::vec2(ui.available_width(), height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_MD;
                daw_ui::text::subtitle(ui, descriptor.name);
                daw_ui::preset_group(
                    ui,
                    egui::Id::new((descriptor.id, "preset")),
                    &name,
                    &choices,
                    current,
                    &mut state.save_name,
                )
            },
        )
        .inner;
    if !saving && let Some(draft) = &mut state.save_name {
        // Saving starts from the loaded preset's name.
        *draft = state.preset.clone().unwrap_or_default();
    }
    match action {
        Some(daw_ui::PresetAction::Previous) => {
            if let Some(index) = step(-1) {
                load(host, state, presets[index]);
            }
        }
        Some(daw_ui::PresetAction::Next) => {
            if let Some(index) = step(1) {
                load(host, state, presets[index]);
            }
        }
        Some(daw_ui::PresetAction::Load(index)) => load(host, state, presets[index]),
        Some(daw_ui::PresetAction::SaveAs(name)) => {
            let preset = Preset::capture(descriptor, &name, |id| host.value(id) as f32);
            host.save_preset(preset);
            state.preset = Some(name);
        }
        Some(daw_ui::PresetAction::Reset) => {
            let values: Vec<_> = descriptor
                .parameters
                .iter()
                .map(|spec| (spec.id, f64::from(spec.default_normalized())))
                .collect();
            host.set_values(&values);
            state.preset = None;
        }
        Some(daw_ui::PresetAction::ToggleFavorite(_)) | None => {}
    }
}

#[cfg(test)]
mod tests;
