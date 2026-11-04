use daw_ui::{DeviceIoSummary, DeviceParameterMetadata};
use egui::Ui;

#[derive(Default)]
pub struct State {
    query: String,
}

pub fn show(ui: &mut Ui, state: &mut State) {
    ui.columns(2, |columns| {
        let left = &mut columns[0];
        daw_ui::text::subtitle(left, "Device capabilities");
        daw_ui::text::note(left, "device_io_summary · inspected audio and MIDI ports");
        left.add_space(daw_ui::tokens::SPACE_MD);
        daw_ui::device_io_summary(
            left,
            Some(DeviceIoSummary {
                input_channels: 4,
                input_buses: 2,
                output_channels: 2,
                output_buses: 1,
                midi_input: true,
                midi_output: false,
                parameters: 6,
            }),
        );
        daw_ui::text::note(left, "2 input buses · 1 output bus");
        left.add_space(daw_ui::tokens::SPACE_XL);
        daw_ui::text::subtitle(left, "Before inspection");
        daw_ui::text::note(left, "Unknown counts stay distinct from zero ports.");
        left.add_space(daw_ui::tokens::SPACE_MD);
        daw_ui::device_io_summary(left, None);
        daw_ui::button(left, "Read details");

        let right = &mut columns[1];
        daw_ui::text::subtitle(right, "Parameter catalogue");
        daw_ui::text::note(
            right,
            "device_parameter_catalog · names, units and capabilities",
        );
        right.add_space(daw_ui::tokens::SPACE_MD);
        let parameters = [
            (10, "Cutoff frequency", "Hz", 0, false, true),
            (11, "Resonance", "%", 0, false, true),
            (12, "Filter type", "", 3, false, true),
            (13, "Output meter", "dB", 0, true, false),
            (14, "Oversampling quality", "", 2, false, false),
            (15, "Release", "ms", 0, false, true),
        ]
        .map(
            |(id, name, unit, steps, read_only, can_automate)| DeviceParameterMetadata {
                id,
                name,
                unit,
                steps,
                read_only,
                can_automate,
            },
        );
        daw_ui::device_parameter_catalog(right, &mut state.query, &parameters, 377.0);
    });
}
