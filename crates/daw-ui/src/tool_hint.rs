//! Persistent, read-only guidance for the active editing interaction.
use crate::{EditorTool, text::TextRole, theme, tokens};
use egui::{Label, Response, Sense, Ui, text::LayoutJob};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
                "Click a selected clip to delete the selected clips or time block; click another clip to delete it."
            }
            (Self::Arrangement, Stretch) => {
                "Drag a clip to change its duration. Audio keeps its pitch; MIDI note timing scales. Shift bypasses snap."
            }
            (Self::Arrangement, Ghost) => {
                "Click a clip to create a linked copy after it. Source edits stay shared; placement, trim, and stretch stay independent."
            }
            (Self::Midi, Select) => {
                "Click or box-select; Shift-click toggles. Arrows select; Shift+arrows extend; Ctrl/Cmd+arrows nudge or transpose. Drag notes to move or edges to resize; Shift-drag copies. Fully overlapping notes of the same pitch merge on release."
            }
            (Self::Midi, Pencil) => {
                "Click empty space to draw a note; drag to set length. Fully overlapping notes of the same pitch merge on release."
            }
            (Self::Midi, Cut) => "Click a note to split it at the pointer.",
            (Self::Midi, Duplicate) => "Click a note to create an independent copy after it.",
            (Self::Midi, Glue) => "Click notes to join them with the selection.",
            (Self::Midi, Erase) => "Click a note to delete it; drag across notes to erase.",
            (Self::Midi, MultiDraw) => {
                "Draw the chosen chord or scale; drag to set length. Fully overlapping notes of the same pitch merge on release."
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

/// Show directly below the tool toolbar. This component never claims focus,
/// consumes input, or owns the caller's tool, selection, or document history.
pub fn editor_tool_hint(ui: &mut Ui, tool: EditorTool, context: ToolHintContext) -> Response {
    interaction_hint(
        ui,
        &format!("{} · {}", context.label(tool), tool.key().name()),
        context.hint(tool),
    )
}

/// The same guidance surface for an editor with one fixed interaction mode.
pub fn interaction_hint(ui: &mut Ui, label: &str, hint: &str) -> Response {
    let colors = theme::palette(ui.ctx());
    egui::Frame::new()
        .fill(colors.panel)
        .inner_margin(egui::Margin::symmetric(
            tokens::SPACE_MD as i8,
            tokens::SPACE_XS as i8,
        ))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
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
                &format!("   {hint}"),
                0.0,
                egui::TextFormat {
                    font_id: TextRole::Note.font(),
                    color: colors.muted,
                    ..Default::default()
                },
            );
            ui.add(
                Label::new(text)
                    .wrap()
                    .selectable(false)
                    .sense(Sense::hover()),
            );
        })
        .response
}
