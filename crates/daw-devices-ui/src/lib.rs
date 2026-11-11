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
        let cards: Vec<_> = layout
            .sections
            .iter()
            .filter(|section| section.row == row)
            .filter_map(|section| card(ui, descriptor, section, host, &layout))
            .collect();
        if cards.is_empty() {
            continue;
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
                    .map(|(spec, _)| control_height(spec, &layout))
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
                        };
                        control(ui, descriptor, spec, host, cell, &layout);
                    }
                });
            }
        });
        // The gutter between rows equals the one between cards.
        ui.add_space((daw_ui::tokens::SPACE_SM - ui.spacing().item_spacing.y).max(0.0));
    }
    if descriptor.kind == DeviceKind::Instrument {
        daw_ui::section_gap(ui);
        performance(ui, host, state);
    }
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

fn stacked(spec: &ParamSpec, layout: &panels::Layout) -> bool {
    toggles(spec) && layout.stacked.contains(&spec.id)
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
        ParamRange::Linear { .. } | ParamRange::Log { .. }
    )
}

/// Height of the control itself, between its caption and its value.
fn control_height(spec: &ParamSpec, layout: &panels::Layout) -> f32 {
    if is_knob(spec) {
        knob_diameter(spec, layout)
    } else if let (true, ParamRange::Choice(labels)) = (stacked(spec, layout), spec.range) {
        daw_ui::TOGGLE_ROW_HEIGHT * labels.len() as f32
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
        ParamRange::Linear { .. } | ParamRange::Log { .. } => [0.0, 1.0]
            .into_iter()
            .map(|end| text(&spec.format(end), daw_ui::text::TextRole::Note.font()))
            .fold(knob_diameter(spec, layout), f32::max),
        ParamRange::Choice(labels) if toggles(spec) => {
            let widths = labels
                .iter()
                .map(|label| text(label, button.clone()) + 2.0 * daw_ui::tokens::SPACE_SM);
            if stacked(spec, layout) {
                widths.fold(0.0, f32::max)
            } else {
                widths.sum()
            }
        }
        ParamRange::Choice(labels) => choice(widest(&mut labels.iter().map(|l| l.to_string()))),
        ParamRange::Integer { .. } => choice(widest(
            &mut (spec.plain_min() as i32..=spec.plain_max() as i32)
                .map(|value| spec.format(spec.to_normalized(value as f32))),
        )),
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
/// choice keeps, and the height of the line's control band.
struct Cell {
    width: f32,
    natural: f32,
    height: f32,
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
            daw_ui::control_caption(ui, caption(spec, layout));
            let offset = (height - control_height(spec, layout)) * 0.5;
            if offset > 0.0 {
                ui.add_space(offset);
            }
            let navigation = match spec.range {
                ParamRange::Linear { .. } | ParamRange::Log { .. } => {
                    let mut value = normalized;
                    let response = ui.add(
                        daw_ui::Knob::new(&mut value, 0.0..=1.0)
                            .compact()
                            .pointer()
                            .diameter(knob_diameter(spec, layout))
                            .text(caption(spec, layout))
                            .automation(automation.map(|(_, state)| state)),
                    );
                    let text = spec.format(normalized as f32);
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
                ParamRange::Choice(_) | ParamRange::Integer { .. } => {
                    // Plain values with their labels: option indices for
                    // choices, the numbers themselves for integers.
                    let options: Vec<(f32, String)> = match spec.range {
                        ParamRange::Choice(labels) => {
                            let offered =
                                (layout.options)(spec.id, &|id| host.value(id)).unwrap_or(labels);
                            offered
                                .iter()
                                .map(|option| {
                                    let index = labels.iter().position(|o| o == option);
                                    (index.unwrap_or(0) as f32, (*option).to_string())
                                })
                                .collect()
                        }
                        _ => (spec.plain_min() as i32..=spec.plain_max() as i32)
                            .map(|value| {
                                let plain = value as f32;
                                (plain, spec.format(spec.to_normalized(plain)))
                            })
                            .collect(),
                    };
                    let mut current = spec.to_plain(normalized as f32);
                    let label = spec.format(normalized as f32);
                    let response = if toggles(spec) {
                        let options: Vec<(f32, &str)> = options
                            .iter()
                            .map(|(plain, option)| (*plain, option.as_str()))
                            .collect();
                        let response = if stacked(spec, layout) {
                            daw_ui::toggle_group_vertical(ui, &mut current, &options)
                        } else {
                            daw_ui::toggle_group(ui, &mut current, &options)
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
