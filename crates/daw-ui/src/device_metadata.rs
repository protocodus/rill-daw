//! Read-only device capabilities, shared by catalogues and device inspectors.
use crate::{text, tokens};
use egui::Ui;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

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

fn cached_matches(
    ctx: &egui::Context,
    id: egui::Id,
    query: &str,
    parameters: &[DeviceParameterMetadata<'_>],
) -> Arc<[usize]> {
    let mut signature = DefaultHasher::new();
    query.hash(&mut signature);
    parameters.len().hash(&mut signature);
    if !query.trim().is_empty() {
        for parameter in parameters {
            parameter.name.hash(&mut signature);
            parameter.unit.hash(&mut signature);
            parameter.id.hash(&mut signature);
        }
    }
    crate::search_cache::cached_indices(ctx, id, signature.finish(), || {
        let query = query.trim().to_lowercase();
        let words: Vec<_> = query.split_whitespace().collect();
        if words.is_empty() {
            return (0..parameters.len()).collect();
        }
        parameters
            .iter()
            .enumerate()
            .filter_map(|(index, parameter)| {
                // Normalize once per row, not once per search word. Keep the
                // complete text's Unicode casing and numeric-ID search intact.
                let searchable = format!("{} {} {}", parameter.name, parameter.unit, parameter.id)
                    .to_lowercase();
                words
                    .iter()
                    .all(|word| searchable.contains(word))
                    .then_some(index)
            })
            .collect()
    })
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
    let matches = cached_matches(
        ui.ctx(),
        ui.make_persistent_id("device_parameter_catalog_search"),
        query,
        parameters,
    );
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
                let parameter = &parameters[matches[row]];
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parameter(id: u32, name: &str) -> DeviceParameterMetadata<'_> {
        DeviceParameterMetadata {
            id,
            name,
            unit: "Hz",
            steps: 0,
            read_only: false,
            can_automate: true,
        }
    }

    #[test]
    fn cached_catalogue_search_preserves_unicode_tokens_and_numeric_ids() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("metadata");
        let parameters = [
            parameter(123, "CUTOFF 音"),
            parameter(456, "CUTOFF ΩΣ"),
            parameter(789, "Resonance"),
        ];
        for (query, expected) in [
            ("", vec![0, 1, 2]),
            (" \t ", vec![0, 1, 2]),
            ("CUTOFF Hz", vec![0, 1]),
            ("音 23", vec![0]),
            ("ωσ", vec![]),
            ("ΩΣ 456", vec![1]),
            ("missing", vec![]),
        ] {
            assert_eq!(&*cached_matches(&ctx, id, query, &parameters), expected);
        }
    }

    #[test]
    fn catalogue_cache_invalidates_search_fields_and_keeps_display_metadata_live() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("metadata");
        let mut parameters = [parameter(1, "Cutoff"), parameter(2, "Cutoff")];
        let first = cached_matches(&ctx, id, "cutoff", &parameters);
        parameters[0].read_only = true;
        parameters[0].can_automate = false;
        parameters[0].steps = 127;
        assert!(Arc::ptr_eq(
            &first,
            &cached_matches(&ctx, id, "cutoff", &parameters),
        ));
        parameters[0].name = "Resonance";
        assert_eq!(&*cached_matches(&ctx, id, "cutoff", &parameters), &[1]);
        parameters.swap(0, 1);
        assert_eq!(&*cached_matches(&ctx, id, "cutoff", &parameters), &[0]);
        assert!(cached_matches(&ctx, id, "kHz", &parameters).is_empty());
        parameters[0].unit = "kHz";
        assert_eq!(&*cached_matches(&ctx, id, "kHz", &parameters), &[0]);
        assert!(cached_matches(&ctx, id, "123", &parameters).is_empty());
        parameters[1].id = 123;
        assert_eq!(&*cached_matches(&ctx, id, "123", &parameters), &[1]);
        assert!(cached_matches(&ctx, id, "123", &parameters[..1]).is_empty());
        assert!(cached_matches(&ctx, id, "", &[]).is_empty());
    }

    #[test]
    fn unchanged_large_catalogue_retains_the_result_allocation() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("large_metadata");
        let parameters: Vec<_> = (0..10_000).map(|id| parameter(id, "Cutoff 音")).collect();
        for query in ["", "CUTOFF hz"] {
            let first = cached_matches(&ctx, id, query, &parameters);
            assert_eq!(first.len(), parameters.len());
            for _ in 0..60 {
                assert!(Arc::ptr_eq(
                    &first,
                    &cached_matches(&ctx, id, query, &parameters),
                ));
            }
        }
    }
}
