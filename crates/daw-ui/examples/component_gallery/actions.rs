use daw_ui::{self as ui, Icon, text};
use egui::Ui;

#[derive(Default)]
pub struct State {
    tool: usize,
    looped: bool,
}

pub fn show(ui: &mut Ui, state: &mut State) {
    ui.columns(2, |columns| {
        let left = &mut columns[0];
        text::subtitle(left, "Commands");
        ui::panel(left.ctx()).show(left, |ui| {
            text::note(ui, "Filled controls execute an action.");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui::button(ui, "Play");
                ui::button(ui, "Stop");
                ui::button(ui, "Record");
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui::primary_button(ui, "Add notes");
                ui::button(ui, "Duplicate");
                ui::small_button(ui, "Undo");
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_enabled(false, ui::button_widget("Save"));
                ui::button(ui, "Export WAV…");
            });
        });
        left.add_space(24.0);
        text::subtitle(left, "Vector icons");
        ui::card(left.ctx()).show(left, |ui| {
            ui.horizontal(|ui| {
                for icon in [
                    Icon::Pencil,
                    Icon::Cut,
                    Icon::Link,
                    Icon::Stretch,
                    Icon::Waveform,
                    Icon::Instrument,
                    Icon::Effect,
                    Icon::Routing,
                    Icon::Automation,
                    Icon::Folder,
                ] {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                    icon.paint(
                        ui.painter(),
                        rect.shrink(5.0),
                        ui::theme::palette(ui.ctx()).text,
                    );
                }
            });
            text::note(
                ui,
                "Resolution-independent geometry; no symbol-font dependency.",
            );
        });
        let right = &mut columns[1];
        text::subtitle(right, "Interaction tools");
        ui::panel(right.ctx()).show(right, |ui| {
            text::note(
                ui,
                "The active tool retains its underline and selected tint.",
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui::tool_value(ui, &mut state.tool, 0, "Select");
                ui::tool_value(ui, &mut state.tool, 1, "Pencil");
                ui::tool_value(ui, &mut state.tool, 2, "Cut");
            });
            ui.add_space(20.0);
            text::subtitle(ui, "Presentation");
            text::note(ui, "Outlined controls change views or framing.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui::view_choice(ui, true, "Arranger");
                ui::view_choice(ui, false, "Mixer");
                ui::view_button(ui, "Fit");
            });
            ui.add_space(12.0);
            ui::view_toggle(ui, &mut state.looped, "Loop");
        });
    });
    ui.add_space(24.0);
    ui::inset(ui.ctx()).show(ui,|ui| {
        text::title(ui,"Inter · Shared typography");
        text::body(ui,"Titles, body text, labels, and notes share the same roles and palette.");
        text::label(ui,"音楽 · Melodía · Glass Current");
        text::note(ui,"Native focus, disabled states, tooltips, and accessible labels stay with each control.");
    });
}
