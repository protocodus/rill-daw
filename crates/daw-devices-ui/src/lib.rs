//! Panels for Rill's built-in devices, composed only from `daw-ui` components.
//!
//! A panel talks to whatever owns the parameter values through [`DeviceHost`],
//! so the same panel runs in the app, in the gallery and in tests. See
//! `docs/R1_DEVICES.md#panel-host`.
use daw_devices::{DeviceDescriptor, ParamRange, ParamSpec, Preset, factory_presets};
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
}

/// Per-panel UI state the host keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct PanelState {
    /// Name of the last loaded or saved preset.
    pub preset: Option<String>,
    /// Draft name while "Save as…" is open.
    pub save_name: Option<String>,
}

/// Knob diameters from the panel grammar.
pub const KNOB_DENSE: f32 = 48.0;
pub const KNOB_HERO: f32 = 64.0;
const CELL_WIDTH: f32 = 84.0;

/// Draws a device's panel: header with presets, then one section per
/// parameter group in signal order.
pub fn show(
    ui: &mut Ui,
    descriptor: &DeviceDescriptor,
    host: &mut dyn DeviceHost,
    state: &mut PanelState,
) {
    ui.set_width(daw_ui::tokens::POPUP_WIDTH_MD.min(ui.available_width()));
    preset_bar(ui, descriptor, host, state);
    daw_ui::section_gap(ui);
    let layout = panels::layout(descriptor);
    for section in &layout.sections {
        let visible: Vec<_> = section
            .parameters
            .iter()
            .filter_map(|id| descriptor.parameter(*id))
            .filter(|spec| (layout.visible)(spec.id, &|id| host.value(id)))
            .collect();
        if visible.is_empty() {
            continue;
        }
        daw_ui::text::subtitle(ui, section.title);
        ui.horizontal_wrapped(|ui| {
            for spec in visible {
                let hero = layout.hero == Some(spec.id);
                control(ui, descriptor, spec, host, hero, &layout);
            }
        });
        daw_ui::section_gap(ui);
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
                ParamRange::Choice(options) => {
                    let current = spec.to_plain(normalized as f32) as usize;
                    let offered =
                        (layout.options)(spec.id, &|id| host.value(id)).unwrap_or(options);
                    let label = options.get(current).copied().unwrap_or("");
                    let response = daw_ui::select(id, label)
                        .width(CELL_WIDTH - daw_ui::tokens::SPACE_XS)
                        .show_ui(ui, |ui| {
                            for option in offered {
                                let index = options.iter().position(|o| o == option).unwrap_or(0);
                                if daw_ui::selectable(ui, index == current, *option).clicked() {
                                    host.set_value(
                                        spec.id,
                                        f64::from(spec.to_normalized(index as f32)),
                                    );
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
