use daw_ui::{self as controls, text};
use egui::Ui;

pub struct State {
    name: String,
    search: String,
    secret: String,
    gain: f64,
    duration: f64,
    pan: f64,
    rotary: f64,
    enabled: bool,
    monitor: bool,
    mode: u8,
    rate: u32,
    buffer: u32,
}
impl Default for State {
    fn default() -> Self {
        Self {
            name: "Glass Current".into(),
            search: "Search instruments".into(),
            secret: "sample-key-not-a-credential".into(),
            gain: -6.5,
            duration: 1.25,
            pan: 0.2,
            rotary: 42.375,
            enabled: true,
            monitor: false,
            mode: 0,
            rate: 48000,
            buffer: 256,
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    ui.columns(2, |columns| {
        let left = &mut columns[0];
        text::subtitle(left, "Text and precise values");
        controls::panel(left.ctx()).show(left, |ui| {
            controls::form_grid(ui, "text-values", |ui| {
                text::label(ui, "Project");
                controls::text_field(ui, &mut state.name, "Project name", 290.0);
                ui.end_row();
                text::label(ui, "Gain");
                ui.add(
                    controls::number(&mut state.gain)
                        .suffix(" dB")
                        .speed(0.1)
                        .range(-60.0..=24.0),
                );
                ui.end_row();
                text::label(ui, "Duration");
                ui.add(
                    controls::number(&mut state.duration)
                        .suffix(" beats")
                        .speed(0.01)
                        .range(0.001..=128.0),
                );
                ui.end_row();
                text::label(ui, "API key");
                controls::secret_field(ui, &mut state.secret, "API key", 290.0);
                ui.end_row();
            });
            ui.add_space(12.0);
            controls::text_field(ui, &mut state.search, "Search", ui.available_width());
            text::note(ui, "The masked key above is sample data.");
        });
        left.add_space(20.0);
        text::subtitle(left, "Paths and rotary values");
        controls::card(left.ctx()).show(left, |ui| {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(300.0, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    controls::path_row(
                        ui,
                        std::path::Path::new("Music / Rill / Glass Current / Recordings"),
                        |ui| { controls::small_button(ui, "Choose…"); },
                    );
                    text::note(ui, "Long paths truncate while their actions remain reachable.");
                    ui.add_space(controls::tokens::SPACE_MD);
                    text::body(ui, "Drag the dial or click its number to type.");
                    text::note(ui, "The arc and written state identify automation. Shift makes a drag finer.");
                });
                ui.add(controls::knob(&mut state.rotary, 0.0..=100.0)
                    .text("Normalized")
                    .suffix(" %")
                    .diameter(72.0)
                    .automation(Some(controls::AutomationState::Read)));
            });
        });
        let right = &mut columns[1];
        text::subtitle(right, "Choices and settings");
        controls::panel(right.ctx()).show(right, |ui| {
            controls::form_grid(ui, "settings", |ui| {
                text::label(ui, "Sample rate");
                controls::select("rate", format!("{} Hz", state.rate)).show_ui(ui, |ui| {
                    for value in [44100, 48000, 96000] {
                        controls::selectable_value(
                            ui,
                            &mut state.rate,
                            value,
                            format!("{value} Hz"),
                        );
                    }
                });
                ui.end_row();
                text::label(ui, "Buffer size");
                controls::select("buffer", format!("{} frames", state.buffer)).show_ui(ui, |ui| {
                    for value in [64, 128, 256, 512] {
                        controls::selectable_value(
                            ui,
                            &mut state.buffer,
                            value,
                            format!("{value} frames"),
                        );
                    }
                });
                ui.end_row();
            });
            ui.add_space(16.0);
            controls::switch(ui, &mut state.enabled, "Enable monitoring");
            controls::checkbox(ui, &mut state.monitor, "Arm on selection");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                controls::radio_value(ui, &mut state.mode, 0, "High quality");
                controls::radio_value(ui, &mut state.mode, 1, "Fast");
            });
            ui.add_space(16.0);
            text::label(ui, "Pan");
            ui.add(controls::slider(&mut state.pan, -1.0..=1.0).text("L / R"));
            ui.add_space(12.0);
            controls::action_row(
                ui,
                |ui| {
                    controls::primary_button(ui, "Apply");
                    controls::button(ui, "Cancel");
                },
                |ui| text::note(ui, "Apply the draft settings."),
            );
        });
    });
}
