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
}

/// Knob diameters from the panel grammar.
pub const KNOB_DENSE: f32 = 48.0;
pub const KNOB_HERO: f32 = 64.0;
const CELL_WIDTH: f32 = 84.0;

/// The panel's width, for the window that hosts it.
pub fn panel_width(descriptor: &DeviceDescriptor) -> f32 {
    panels::layout(descriptor).width
}

/// Draws a device's panel: header with presets, the response, then the
/// sections in signal order, side by side where they share a row.
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
            .filter_map(|section| {
                let visible: Vec<_> = section
                    .parameters
                    .iter()
                    .filter_map(|id| descriptor.parameter(*id))
                    .filter(|spec| (layout.visible)(spec.id, &|id| host.value(id)))
                    .collect();
                (!visible.is_empty()).then_some((section.title, visible))
            })
            .collect();
        if sections.is_empty() {
            continue;
        }
        // Top-aligned: a centered wrap staggers knobs and menus of unequal height.
        let wrap = egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(true);
        ui.with_layout(wrap, |ui| {
            ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_LG;
            for (title, visible) in sections {
                ui.vertical(|ui| {
                    daw_ui::text::subtitle(ui, title);
                    ui.with_layout(wrap, |ui| {
                        ui.spacing_mut().item_spacing.x = daw_ui::tokens::SPACE_XS;
                        for spec in visible {
                            let hero = layout.hero == Some(spec.id);
                            control(ui, descriptor, spec, host, hero, &layout);
                        }
                    });
                });
            }
        });
        daw_ui::section_gap(ui);
    }
    if descriptor.kind == DeviceKind::Instrument {
        performance(ui, host, state);
    }
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
const RESPONSE_HEIGHT: f32 = 120.0;

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

/// One control chosen by the parameter's range, in a fixed-width cell.
fn control(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    spec: &ParamSpec,
    host: &mut dyn DeviceHost,
    hero: bool,
    layout: &panels::Layout,
) {
    let normalized = host.value(spec.id);
    let automation = host.automation(spec.id);
    let id = egui::Id::new((descriptor.id, spec.id));
    let control = AutomationControl::DeviceParameter { id: spec.id };
    ui.allocate_ui_with_layout(
        egui::vec2(if hero { CELL_WIDTH * 1.25 } else { CELL_WIDTH }, 0.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            daw_ui::text::label(ui, spec.name);
            let navigation = match spec.range {
                ParamRange::Linear { .. } | ParamRange::Log { .. } => {
                    let mut value = normalized;
                    let response = ui.add(
                        daw_ui::Knob::new(&mut value, 0.0..=1.0)
                            .compact()
                            .pointer()
                            .diameter(if hero { KNOB_HERO } else { KNOB_DENSE })
                            .text(spec.name)
                            .automation(automation.map(|(_, state)| state)),
                    );
                    let text = spec.format(normalized as f32);
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
                    let current = spec.to_plain(normalized as f32);
                    let label = spec.format(normalized as f32);
                    let response = daw_ui::select(id, label)
                        .width(CELL_WIDTH - daw_ui::tokens::SPACE_XS)
                        .show_ui(ui, |ui| {
                            for (plain, option) in &options {
                                if daw_ui::selectable(ui, *plain == current, option).clicked() {
                                    host.set_value(spec.id, f64::from(spec.to_normalized(*plain)));
                                }
                            }
                        })
                        .response;
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
            if let Some(action) = navigation {
                host.navigate(action);
            }
        },
    );
}

/// Preset name with a modified marker, previous/next, the preset menu, Save
/// as… and Reset. Identical for every device.
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
    daw_ui::toolbar(ui, |ui| {
        daw_ui::text::subtitle(ui, descriptor.name);
        let name = state.preset.clone().unwrap_or_else(|| "Custom".into());
        let label = if modified && state.preset.is_some() {
            format!("{name} *")
        } else {
            name
        };
        let step = |offset: isize| {
            let count = presets.len() as isize;
            let from = current.map_or(if offset > 0 { -1 } else { 0 }, |index| index as isize);
            (from + offset).rem_euclid(count) as usize
        };
        if daw_ui::small_button(ui, "◂")
            .on_hover_text("Previous preset")
            .clicked()
            && !presets.is_empty()
        {
            load(host, state, presets[step(-1)]);
        }
        daw_ui::select(egui::Id::new((descriptor.id, "preset")), label)
            .width(daw_ui::tokens::SELECT_WIDTH)
            .show_ui(ui, |ui| {
                for (index, preset) in presets.iter().enumerate() {
                    let text = if preset.category.is_empty() {
                        preset.name.clone()
                    } else {
                        format!("{} · {}", preset.category, preset.name)
                    };
                    let response = daw_ui::selectable(ui, current == Some(index), text);
                    let response = if preset.description.is_empty() {
                        response
                    } else {
                        response.on_hover_text(&preset.description)
                    };
                    if response.clicked() {
                        load(host, state, preset);
                    }
                }
            });
        if daw_ui::small_button(ui, "▸")
            .on_hover_text("Next preset")
            .clicked()
            && !presets.is_empty()
        {
            load(host, state, presets[step(1)]);
        }
        if daw_ui::small_button(ui, "Save as…").clicked() {
            state.save_name = Some(state.preset.clone().unwrap_or_default());
        }
        if daw_ui::small_button(ui, "Reset")
            .on_hover_text("Return every parameter to its default")
            .clicked()
        {
            let values: Vec<_> = descriptor
                .parameters
                .iter()
                .map(|spec| (spec.id, f64::from(spec.default_normalized())))
                .collect();
            host.set_values(&values);
            state.preset = None;
        }
    });
    if let Some(mut name) = state.save_name.take() {
        let mut keep = true;
        daw_ui::toolbar(ui, |ui| {
            daw_ui::text_field(ui, &mut name, "Preset name", daw_ui::tokens::SELECT_WIDTH);
            let valid = !name.trim().is_empty() && name.len() <= 128;
            if ui
                .add_enabled(valid, daw_ui::button_widget("Save"))
                .clicked()
            {
                let preset = Preset::capture(descriptor, name.trim(), |id| host.value(id) as f32);
                host.save_preset(preset);
                state.preset = Some(name.trim().to_string());
                keep = false;
            }
            if daw_ui::small_button(ui, "Cancel").clicked() {
                keep = false;
            }
        });
        if keep {
            state.save_name = Some(name);
        }
    }
}

#[cfg(test)]
mod tests;
