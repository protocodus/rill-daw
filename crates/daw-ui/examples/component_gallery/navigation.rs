use daw_ui::{self as controls, ViewLink, text};
use egui::Ui;

pub struct State {
    pub capture: bool,
    view: u8,
    keep_menu_open: bool,
    popup_open: bool,
    highlight: bool,
    clip: String,
    start: f64,
    length: f64,
    duration_seconds: f64,
    tool: controls::EditorTool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            capture: false,
            view: 1,
            keep_menu_open: true,
            popup_open: true,
            highlight: true,
            clip: "Verse · Glass arpeggio".into(),
            start: 9.0,
            length: 16.0,
            duration_seconds: 30.0,
            tool: controls::EditorTool::Duplicate,
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    controls::workspace_header(ui, "Clip Editor · Glass arpeggio", |ui| {
        controls::quiet_button(ui, "Properties");
        controls::quiet_button(ui, "Next");
    });
    ui.add_space(controls::tokens::SPACE_MD);
    if let Some(view) = controls::view_links(
        ui,
        state.view,
        &[
            ViewLink {
                value: 0,
                label: "Arrangement",
                enabled: true,
            },
            ViewLink {
                value: 1,
                label: "Clip Editor",
                enabled: true,
            },
            ViewLink {
                value: 2,
                label: "Instrument",
                enabled: true,
            },
            ViewLink {
                value: 3,
                label: "Effects",
                enabled: true,
            },
            ViewLink {
                value: 4,
                label: "Routing",
                enabled: false,
            },
        ],
    ) {
        state.view = view;
    }
    ui.add_space(controls::tokens::SPACE_LG);
    controls::toolbar(ui, |ui| {
        controls::quiet_action_menu(ui, "Process", |ui| {
            controls::button(ui, "Normalize");
            controls::button(ui, "Reverse");
        });
        let choice = controls::choice_menu(ui, "Natural minor", |ui| {
            controls::command_menu_contents(ui, |ui| {
                text::label(ui, "Scale");
                for label in [
                    "Major",
                    "Natural minor",
                    "Dorian",
                    "Lydian",
                    "Major pentatonic",
                ] {
                    controls::selectable(ui, label == "Natural minor", label);
                }
                ui.separator();
                controls::checkbox(ui, &mut state.highlight, "Highlight scale");
            });
        });
        if state.keep_menu_open {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&choice.response));
        }
        controls::quiet_view_menu(ui, "View", |ui| {
            controls::checkbox(ui, &mut state.highlight, "Highlight scale");
        });
        controls::editor_tools_with_context(
            ui,
            &mut state.tool,
            &[
                controls::EditorTool::Select,
                controls::EditorTool::Duplicate,
            ],
            controls::ToolHintContext::Midi,
        );
        if let Some(seconds) =
            controls::duration::duration_presets(ui, state.duration_seconds, 120.0)
        {
            state.duration_seconds = seconds;
        }
        controls::toolbar_trailing(ui, 100.0, |ui| {
            controls::editor_tool_help(ui, state.tool, controls::ToolHintContext::Midi);
            controls::editor_tool_caption(ui, state.tool, controls::ToolHintContext::Midi);
        });
    });
    ui.add_space(controls::tokens::SPACE_LG);
    if !state.capture {
        ui.horizontal(|ui| {
            controls::checkbox(ui, &mut state.keep_menu_open, "Keep example menu open");
            controls::checkbox(ui, &mut state.popup_open, "Show utility window");
        });
    }
    ui.add_space(controls::tokens::SPACE_LG);
    // Keep the empty-workspace example clear of the simultaneously open menu
    // and utility used to demonstrate native popup behavior.
    let empty_rect = egui::Rect::from_min_size(
        egui::pos2(ui.min_rect().left() + 280.0, ui.cursor().top()),
        egui::vec2(350.0, 180.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(empty_rect), |ui| {
        controls::empty_workspace(
            ui,
            "Start a song",
            "Choose a sound or import a recording.",
            &[
                controls::EmptyWorkspaceAction {
                    id: "instrument",
                    label: "Choose instrument…",
                    description: "Browse installed instruments before creating a track.",
                    primary: true,
                },
                controls::EmptyWorkspaceAction {
                    id: "audio",
                    label: "Import audio…",
                    description: "Choose a recording to add to the arrangement.",
                    primary: false,
                },
            ],
        );
    });
    // Stack the compact generation example below the empty state, in the
    // clear column between the open scale menu and properties utility.
    let generation_rect = egui::Rect::from_min_size(
        egui::pos2(
            empty_rect.left(),
            empty_rect.bottom() + controls::tokens::SPACE_SM,
        ),
        egui::vec2(empty_rect.width(), 112.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(generation_rect), |ui| {
        text::label(ui, "Generate notes");
        ui.add_space(controls::tokens::SPACE_XS);
        controls::generation_footer(
            ui,
            "Added 3 notes · Next range to continue",
            true,
            true,
            true,
        );
    });
    ui.add_space(controls::tokens::SPACE_MD);
    text::note(
        ui,
        "Menus keep native popup behavior; utilities retain caller-owned state.",
    );
    let context = ui.ctx().clone();
    controls::popup("Clip properties", &context)
        .open(&mut state.popup_open)
        .anchor(
            egui::Align2::RIGHT_TOP,
            egui::vec2(-controls::tokens::POPUP_MARGIN, 230.0),
        )
        .default_width(360.0)
        .resizable(false)
        .vscroll(false)
        .show(&context, |ui| {
            controls::form_grid(ui, "clip-properties", |ui| {
                text::label(ui, "Name");
                controls::text_field(ui, &mut state.clip, "Clip name", 180.0);
                ui.end_row();
                text::label(ui, "Start");
                ui.add(controls::number(&mut state.start).suffix(" beats"));
                ui.end_row();
                text::label(ui, "Duration");
                ui.add(controls::number(&mut state.length).suffix(" beats"));
                ui.end_row();
                controls::duration::timing_summary(ui, state.start - 1.0, state.length, 112.0, 4);
            });
            ui.add_space(controls::tokens::SPACE_MD);
            controls::action_row(
                ui,
                |ui| {
                    controls::primary_button(ui, "Apply");
                    controls::button(ui, "Cancel");
                },
                |ui| text::note(ui, "One focused utility."),
            );
        });
}
