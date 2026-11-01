use daw_ui::{self as ui, Icon, text};
use egui::Ui;

#[derive(Default)]
pub struct State {
    tool: usize,
    editor_tool: ui::EditorTool,
    looped: bool,
}

pub fn show(ui: &mut Ui, state: &mut State) {
    ui.columns(2, |columns| {
        let left = &mut columns[0];
        text::subtitle(left, "Commands");
        ui::panel(left.ctx()).show(left, |ui| {
            text::note(
                ui,
                "One primary action; secondary commands share a surface.",
            );
            ui.add_space(ui::tokens::SPACE_MD);
            ui::command_group(ui, 350.0, |ui| {
                ui::quiet_button(ui, "Start");
                ui::primary_button(ui, "Play");
                ui::quiet_button(ui, "Stop");
                ui::record_button(ui, false, true);
            });
            ui.add_space(ui::tokens::SPACE_MD);
            ui.horizontal(|ui| {
                ui::primary_button(ui, "Add notes");
                ui::button(ui, "Duplicate");
                ui::small_button(ui, "Undo");
            });
            ui.add_space(ui::tokens::SPACE_MD);
            ui.horizontal(|ui| {
                ui.add_enabled(false, ui::button_widget("Save"));
                ui::button(ui, "Export WAV…");
            });
        });
        left.add_space(ui::tokens::SPACE_LG);
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
                    let (rect, _) = ui.allocate_exact_size(
                        egui::Vec2::splat(ui::tokens::CONTROL_HEIGHT),
                        egui::Sense::hover(),
                    );
                    icon.paint(
                        ui.painter(),
                        rect.shrink(ui::tokens::SPACE_XS),
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
            text::note(ui, "A glowing border marks the active tool.");
            ui.add_space(ui::tokens::SPACE_MD);
            ui.horizontal(|ui| {
                ui::tool_value(ui, &mut state.tool, 0, "Select");
                ui::tool_value(ui, &mut state.tool, 1, "Pencil");
                ui::tool_value(ui, &mut state.tool, 2, "Cut");
            });
            ui.add_space(ui::tokens::SPACE_LG);
            text::subtitle(ui, "Presentation");
            text::note(ui, "Quiet navigation keeps the current workspace clear.");
            ui.add_space(ui::tokens::SPACE_SM);
            ui.horizontal(|ui| {
                ui::tab_choice(ui, true, "Arranger");
                ui::tab_choice(ui, false, "Mixer");
                ui::timeline_fit_button(ui);
            });
            ui.add_space(ui::tokens::SPACE_MD);
            ui::quiet_toggle(ui, &mut state.looped, "Loop");
        });
        right.add_space(ui::tokens::SPACE_LG);
        ui::inset(right.ctx()).show(right, |ui| {
            text::title(ui, "Inter · Shared typography");
            text::body(ui, "Titles, labels, and notes share one palette.");
            text::label(ui, "音楽 · Melodía · Glass Current");
            text::note(
                ui,
                "Native focus and accessible labels stay with each control.",
            );
        });
    });
    ui.add_space(ui::tokens::SPACE_LG);
    text::subtitle(ui, "Focused editing tools");
    ui::toolbar(ui, |ui| {
        use ui::EditorTool::*;
        ui::editor_tools_with_context(
            ui,
            &mut state.editor_tool,
            &[
                Select, Pencil, Cursor, Cut, Duplicate, Glue, Ghost, Stretch, Slip, Erase,
            ],
            ui::ToolHintContext::Arrangement,
        );
    });
}
