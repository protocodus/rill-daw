//! Precise clip fields and a small name editor shared by timeline views.
use egui::{Id, Response, Ui};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DurationEditMode {
    #[default]
    Trim,
    Stretch,
}

#[derive(Clone, Copy, Debug)]
pub struct ClipTimingValues {
    pub start: f64,
    pub duration: f64,
    pub source_offset: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ClipTimingEdit {
    pub start: Option<f64>,
    pub duration: Option<(f64, DurationEditMode)>,
    pub source_offset: Option<f64>,
    /// Opening a numeric entry or changing its interpretation ends the prior
    /// typing burst, even when the model value itself has not changed yet.
    pub new_gesture: bool,
}

impl ClipTimingEdit {
    pub fn changed(self) -> bool {
        self.start.is_some() || self.duration.is_some() || self.source_offset.is_some()
    }
}

/// Append timing rows to a shared form grid. Values are staged; owners validate
/// and commit them after releasing model borrows. Merely inspecting a value
/// outside a current editing range never clamps or changes it.
pub fn clip_timing_fields(
    ui: &mut Ui,
    mut values: ClipTimingValues,
    mode: &mut DurationEditMode,
    maximum_beat: f64,
    maximum_offset: f64,
) -> ClipTimingEdit {
    let mut edit = ClipTimingEdit::default();
    let label = crate::text::label(ui, "Start");
    let mut start = values.start + 1.0;
    let response = ui
        .add(
            crate::number(&mut start)
                .range(1.0..=(maximum_beat - values.duration + 1.0).max(1.0))
                .clamp_existing_to_range(false)
                .speed(0.01)
                .min_decimals(2)
                .max_decimals(9)
                .prefix("Beat "),
        )
        .labelled_by(label.id)
        .on_hover_text("Move the clip to an exact beat without changing its source window");
    edit.new_gesture |=
        response.gained_focus() || response.double_clicked() || response.drag_started();
    if response.changed() {
        edit.start = Some(start - 1.0);
    }
    ui.end_row();
    let label = crate::text::label(ui, "Duration");
    ui.horizontal(|ui| {
        let response = ui
            .add(
                crate::number(&mut values.duration)
                    .range(0.000_000_001..=(maximum_beat - values.start).max(0.000_000_001))
                    .clamp_existing_to_range(false)
                    .speed(0.01)
                    .min_decimals(2)
                    .max_decimals(9)
                    .suffix(" beats"),
            )
            .labelled_by(label.id)
            .on_hover_text("Trim changes the clip boundary; Stretch changes playback timing");
        edit.new_gesture |=
            response.gained_focus() || response.double_clicked() || response.drag_started();
        if response.changed() {
            edit.duration = Some((values.duration, *mode));
        }
        let previous_mode = *mode;
        crate::select(
            "clip_duration_mode",
            if *mode == DurationEditMode::Trim {
                "Trim"
            } else {
                "Stretch"
            },
        )
        .width(96.0)
        .show_ui(ui, |ui| {
            crate::selectable_value(ui, mode, DurationEditMode::Trim, "Trim");
            crate::selectable_value(ui, mode, DurationEditMode::Stretch, "Stretch");
        })
        .response
        .on_hover_text("Choose how the next duration edit affects this clip");
        edit.new_gesture |= *mode != previous_mode;
    });
    ui.end_row();
    let label = crate::text::label(ui, "Source offset");
    let response = ui
        .add(
            crate::number(&mut values.source_offset)
                .range(0.0..=maximum_offset.max(0.0))
                .clamp_existing_to_range(false)
                .speed(0.01)
                .min_decimals(2)
                .max_decimals(9)
                .suffix(" beats"),
        )
        .labelled_by(label.id)
        .on_hover_text("Slip the source window while keeping the clip's position and duration");
    edit.new_gesture |=
        response.gained_focus() || response.double_clicked() || response.drag_started();
    if response.changed() {
        edit.source_offset = Some(values.source_offset);
    }
    ui.end_row();
    edit
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineNameAction {
    Editing,
    Commit,
    Cancel,
}

pub struct InlineNameResponse {
    pub response: Response,
    pub action: InlineNameAction,
}

/// Single-line naming in the existing item context. The caller owns its draft,
/// anchor and transaction; Enter/focus loss commits and Escape discards it.
pub fn inline_name_editor(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    hint: &str,
    width: f32,
    initial_focus: bool,
) -> InlineNameResponse {
    let cancel = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let row = ui
        .horizontal(|ui| {
            let output = egui::TextEdit::singleline(value)
                .id(id)
                .hint_text(hint)
                .desired_width(width.max(32.0))
                .margin(egui::vec2(
                    crate::tokens::BUTTON_PADDING_X,
                    crate::tokens::BUTTON_PADDING_Y,
                ))
                .min_size(egui::vec2(0.0, crate::tokens::CONTROL_HEIGHT))
                .show(ui);
            let apply = crate::small_button(ui, "Apply name").clicked();
            let cancel = crate::small_button(ui, "Cancel");
            // Text input can lose focus on pointer-down, before a button's
            // release click. Cancel must win over that ordinary blur commit.
            let cancel = cancel.clicked() || cancel.is_pointer_button_down_on();
            (output, apply, cancel)
        })
        .inner;
    let (mut output, apply, cancel_clicked) = row;
    if initial_focus {
        output.response.request_focus();
        output.state.clear_undoer();
        output
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(value.chars().count()),
            )));
        output.state.store(ui.ctx(), id);
    }
    let enter = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let action = if cancel || cancel_clicked {
        InlineNameAction::Cancel
    } else if apply || enter || (!initial_focus && output.response.lost_focus()) {
        InlineNameAction::Commit
    } else {
        InlineNameAction::Editing
    };
    InlineNameResponse {
        response: output.response,
        action,
    }
}
