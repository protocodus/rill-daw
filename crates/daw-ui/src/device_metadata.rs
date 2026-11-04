//! Read-only device capabilities, shared by catalogues and device inspectors.
use crate::{text, tokens};
use egui::Ui;

#[derive(Clone, Copy)]
pub struct DeviceIoSummary {
    pub input_channels: usize,
    pub input_buses: usize,
    pub output_channels: usize,
    pub output_buses: usize,
    pub midi_input: bool,
    pub midi_output: bool,
    pub parameters: usize,
}

/// Compact port/parameter counts. Missing inspection data remains explicitly
/// unknown; a missing value must never look like a device with zero ports.
pub fn device_io_summary(ui: &mut Ui, summary: Option<DeviceIoSummary>) {
    let audio = |channels, buses, midi| {
        let bus_label = if buses == 1 { "bus" } else { "buses" };
        (
            format!("{channels} audio{}", if midi { " · MIDI" } else { "" }),
            format!(
                "{channels} audio channels across {buses} {bus_label}{}",
                if midi {
                    "; MIDI supported"
                } else {
                    "; no MIDI"
                }
            ),
        )
    };
    let rows = if let Some(summary) = summary {
        [
            (
                "Inputs",
                audio(
                    summary.input_channels,
                    summary.input_buses,
                    summary.midi_input,
                ),
            ),
            (
                "Outputs",
                audio(
                    summary.output_channels,
                    summary.output_buses,
                    summary.midi_output,
                ),
            ),
            (
                "Parameters",
                (
                    summary.parameters.to_string(),
                    "Parameters reported by the plug-in controller".into(),
                ),
            ),
        ]
    } else {
        ["Inputs", "Outputs", "Parameters"].map(|label| {
            (
                label,
                (
                    "Not read".into(),
                    "Read device details to inspect this plug-in".into(),
                ),
            )
        })
    };
    for (label, (value, help)) in rows {
        crate::action_row(
            ui,
            |ui| {
                text::label(ui, value).on_hover_text(help);
            },
            |ui| {
                text::note(ui, label);
            },
        );
    }
}

pub struct DeviceParameterMetadata<'a> {
    pub id: u32,
    pub name: &'a str,
    pub unit: &'a str,
    pub steps: i32,
    pub read_only: bool,
    pub can_automate: bool,
}

/// A virtualized, read-only parameter catalogue. Search matches name, unit or
/// numeric ID; it never edits values or requires a loaded project instance.
pub fn device_parameter_catalog(
    ui: &mut Ui,
    query: &mut String,
    parameters: &[DeviceParameterMetadata<'_>],
    height: f32,
) {
    let has_query = !query.is_empty();
    let clear = crate::action_row(
        ui,
        |ui| {
            ui.add_enabled(has_query, crate::button_widget("Clear"))
                .clicked()
        },
        |ui| crate::text_field(ui, query, "Search device parameters", ui.available_width()),
    )
    .inner
    .0;
    if clear {
        query.clear();
    }
    let query = query.trim().to_lowercase();
    let matches: Vec<_> = parameters
        .iter()
        .filter(|parameter| {
            query.split_whitespace().all(|word| {
                format!("{} {} {}", parameter.name, parameter.unit, parameter.id)
                    .to_lowercase()
                    .contains(word)
            })
        })
        .collect();
    if matches.is_empty() {
        text::note(
            ui,
            if parameters.is_empty() {
                "No parameters reported"
            } else {
                "No matching parameters"
            },
        );
        return;
    }
    text::note(
        ui,
        format!("{} of {} parameters", matches.len(), parameters.len()),
    );
    crate::scroll_area(ui, "device_parameter_catalog", [false, true])
        .max_height(height.max(0.0))
        .min_scrolled_height(0.0)
        .auto_shrink([false, true])
        .show_rows(ui, tokens::CONTROL_HEIGHT, matches.len(), |ui, rows| {
            for row in rows {
                let parameter = matches[row];
                ui.push_id(parameter.id, |ui| {
                    crate::action_row(
                        ui,
                        |ui| {
                            if parameter.read_only {
                                text::note(ui, "Read-only");
                            } else if !parameter.can_automate {
                                text::note(ui, "Manual only");
                            }
                            if !parameter.unit.is_empty() {
                                text::note(ui, parameter.unit);
                            }
                        },
                        |ui| {
                            text::truncated(ui, text::TextRole::Body, parameter.name)
                                .on_hover_text(format!(
                                    "{} · ID {} · {}",
                                    parameter.name,
                                    parameter.id,
                                    if parameter.steps > 0 {
                                        format!("{} values", i64::from(parameter.steps) + 1)
                                    } else {
                                        "Continuous".into()
                                    }
                                ));
                        },
                    );
                });
            }
        });
}
