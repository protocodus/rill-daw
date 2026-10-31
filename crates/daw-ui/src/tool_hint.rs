//! Compact gesture guidance with mouse-accessible, on-demand tool help.
use crate::{EditorTool, text::TextRole, theme, tokens};
use egui::{Label, Response, Sense, Ui, text::LayoutJob};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolHintContext {
    Arrangement,
    Midi,
    Audio,
}

impl ToolHintContext {
    pub fn label(self, tool: EditorTool) -> &'static str {
        if self == Self::Arrangement && tool == EditorTool::Glue {
            "Join"
        } else {
            tool.label()
        }
    }

    /// The first pointer gesture, short enough for an editor toolbar caption.
    pub fn gesture(self, tool: EditorTool) -> &'static str {
        use EditorTool::*;
        match (self, tool) {
            (Self::Arrangement, Select) => "Select clips · Drag to move or trim",
            (Self::Midi, Select) => "Select notes · Drag to move or resize",
            (_, Select) => "Drag to move · Drag edges to trim",
            (Self::Arrangement, Pencil) => "Click a MIDI track to draw a clip",
            (_, Pencil) => "Draw notes · Drag to set length",
            (Self::Midi, Cursor) => "Click to place the playback cursor",
            (_, Cursor) => "Click to seek · Drag to select time",
            (Self::Arrangement, Cut) => "Click a clip to split",
            (Self::Midi, Cut) => "Click a note to split",
            (_, Cut) => "Click the waveform to split",
            (Self::Midi, Duplicate) => "Click a note to duplicate",
            (_, Duplicate) => "Click a clip to duplicate",
            (Self::Arrangement, Glue) => "Click a clip to choose clips to join",
            (_, Glue) => "Click notes to join with the selection",
            (Self::Midi, Erase) => "Click or drag across notes to erase",
            (_, Erase) => "Click a clip to erase it",
            (_, MultiDraw) => "Draw a chord or scale",
            (_, FadeIn) => "Drag to set the fade-in",
            (_, FadeOut) => "Drag to set the fade-out",
            (_, Ghost) => "Click a clip to create a linked copy",
            (_, Stretch) => "Drag to change duration",
            (_, Slip) => "Drag audio inside clip edges",
        }
    }

    pub fn hint(self, tool: EditorTool) -> &'static str {
        use EditorTool::*;
        match (self, tool) {
            (Self::Arrangement, Select) => {
                "Click or box-select clips; use their menu to add or remove. Drag a group to move, an edge to trim, or a fade handle to shape audio. Shift-click toggles; Shift-drag copies. Arrows select; Ctrl/Cmd+arrows nudge."
            }
            (Self::Arrangement, Pencil) => {
                "Click empty space on a MIDI track to create a MIDI clip."
            }
            (Self::Arrangement, Cursor) => {
                "Click to seek; drag across tracks to select a phrase, including rests, for copy, duplicate or delete."
            }
            (Self::Arrangement, Cut) => "Click a clip to split it at the pointer.",
            (Self::Arrangement, Duplicate) => {
                "Click a clip to duplicate its selected group as independent copies. A selected time block retains its rests."
            }
            (Self::Arrangement, Glue) => "Click a clip to choose the clips to join.",
            (Self::Arrangement, Erase) => {
                "Click a clip to delete only that clip. Use the selection menu to delete selected clips or a time block."
            }
            (Self::Arrangement, Stretch) => {
                "Drag a clip to change its duration. Audio keeps its pitch; MIDI note timing scales. Shift bypasses snap."
            }
            (Self::Arrangement, Ghost) => {
                "Click a clip to create a linked copy after it. Source edits stay shared; placement, trim, and stretch stay independent."
            }
            (Self::Midi, Select) => {
                "Click or box-select notes; Shift-click toggles. Drag bodies to move or edges to resize. Right-click changes selection."
            }
            (Self::Midi, Pencil) => {
                "Click empty space to draw a note; drag to set length. Fully overlapping notes of the same pitch merge on release."
            }
            (Self::Midi, Cut) => "Click a note to split it at the pointer.",
            (Self::Midi, Duplicate) => "Click a note to create an independent copy after it.",
            (Self::Midi, Glue) => "Click notes to join them with the selection.",
            (Self::Midi, Erase) => "Click a note to delete it; drag across notes to erase.",
            (Self::Midi, MultiDraw) => {
                "Draw the chosen chord or scale; drag to set length. Same-pitch overlaps merge on release."
            }
            (Self::Audio, Cursor) => {
                "Click to place the playback cursor; drag to select time for copy, cut, or trimming."
            }
            (Self::Audio, Cut) => "Click the waveform to split the clip at the pointer.",
            (Self::Audio, FadeIn) => {
                "Click or drag the waveform to set where the fade-in ends. Choose its curve below."
            }
            (Self::Audio, FadeOut) => {
                "Click or drag the waveform to set where the fade-out begins. Choose its curve below."
            }
            (Self::Audio, Stretch) => {
                "Drag the waveform to change the clip's duration while keeping its pitch. Shift bypasses snap."
            }
            _ => tool.hint(),
        }
    }
}

/// Full instructions stay one click from the canvas without a permanent band.
/// This popup changes no tool, selection, viewport, or document state.
pub fn editor_tool_help(ui: &mut Ui, tool: EditorTool, context: ToolHintContext) -> Response {
    let response = crate::quiet_button(ui, "Tool help").on_hover_text(format!(
        "{} · {}\n{}",
        context.label(tool),
        tool.key().name(),
        context.hint(tool),
    ));
    egui::Popup::menu(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_max_width(360.0);
            crate::text::label(
                ui,
                format!("{} · {}", context.label(tool), tool.key().name()),
            );
            crate::text::body(ui, context.hint(tool));
        });
    response
}

/// Fill only the unused part of an existing toolbar row with pointer guidance.
/// Call after the controls in `toolbar_trailing`: the caption cannot wrap the
/// row, move its controls, or add another instruction band above the canvas.
/// Full instructions remain available through the separate Tool help control.
pub fn editor_tool_caption(
    ui: &mut Ui,
    tool: EditorTool,
    context: ToolHintContext,
) -> Option<Response> {
    let available = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    let width = available.width();
    if width < tokens::CONTROL_HEIGHT {
        return None;
    }
    Some(
        ui.allocate_ui_with_layout(
            egui::vec2(width, tokens::CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    Label::new(TextRole::Note.rich(ui.ctx(), context.gesture(tool)))
                        .truncate()
                        .selectable(false)
                        .sense(Sense::hover()),
                )
                .on_hover_text(context.hint(tool))
            },
        )
        .inner,
    )
}

/// Keep the active interaction legible without letting instructions consume the
/// canvas. Help is a view preference scoped to this editor, never a song edit.
pub fn editor_tool_hint(ui: &mut Ui, tool: EditorTool, context: ToolHintContext) -> Response {
    help_panel(
        ui,
        ui.make_persistent_id(("editor_tool_help", context)),
        &format!("{} · {}", context.label(tool), tool.key().name()),
        context.gesture(tool),
        context.hint(tool),
    )
}

/// The same guidance surface for an editor with one fixed interaction mode.
/// Its first gesture remains visible; complete instructions open with Tool help.
pub fn interaction_hint(ui: &mut Ui, label: &str, hint: &str) -> Response {
    let summary = hint.split([';', '.']).next().unwrap_or(hint);
    help_panel(
        ui,
        ui.make_persistent_id(("interaction_help", label)),
        label,
        summary,
        hint,
    )
}

fn help_panel(ui: &mut Ui, id: egui::Id, label: &str, summary: &str, hint: &str) -> Response {
    let colors = theme::palette(ui.ctx());
    // A preceding toolbar may have a wider minimum than the viewport. Keep
    // the disclosure reachable inside the visible canvas, not off its edge.
    let visible_width = ui
        .available_width()
        .min((ui.clip_rect().right() - ui.next_widget_position().x).max(0.0));
    let mut expanded = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id))
        .unwrap_or(false);
    let response = egui::Frame::new()
        .fill(colors.panel)
        .inner_margin(egui::Margin::symmetric(
            tokens::SPACE_MD as i8,
            tokens::SPACE_XS as i8,
        ))
        .show(ui, |ui| {
            ui.set_width((visible_width - tokens::SPACE_MD * 2.0).max(0.0));
            ui.spacing_mut().interact_size.y = tokens::COMPACT_HEIGHT;
            crate::action_row(
                ui,
                |ui| {
                    let help = ui
                        .add(
                            crate::button_widget("Tool help")
                                .presentation()
                                .selected(expanded)
                                .small()
                                .min_size(egui::vec2(0.0, tokens::COMPACT_HEIGHT)),
                        )
                        .on_hover_text(if expanded {
                            "Hide full tool instructions"
                        } else {
                            "Show full tool instructions and shortcuts"
                        });
                    if help.clicked() {
                        expanded = !expanded;
                    }
                },
                |ui| {
                    let mut text = LayoutJob::default();
                    text.append(
                        label,
                        0.0,
                        egui::TextFormat {
                            font_id: TextRole::Label.font(),
                            color: colors.text,
                            ..Default::default()
                        },
                    );
                    text.append(
                        &format!("   {summary}"),
                        0.0,
                        egui::TextFormat {
                            font_id: TextRole::Note.font(),
                            color: colors.muted,
                            ..Default::default()
                        },
                    );
                    ui.add(
                        Label::new(text)
                            .truncate()
                            .selectable(false)
                            .sense(Sense::hover()),
                    );
                },
            );
            if expanded {
                ui.add_space(tokens::SPACE_XS);
                ui.add(
                    Label::new(TextRole::Note.rich(ui.ctx(), hint))
                        .wrap()
                        .selectable(false)
                        .sense(Sense::hover()),
                );
            }
        })
        .response;
    ui.ctx().data_mut(|data| data.insert_temp(id, expanded));
    response
}
