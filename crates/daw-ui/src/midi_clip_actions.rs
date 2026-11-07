//! Reusable MIDI clip and selected-note musical actions.
use egui::{Rect, Ui};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MidiNoteLengthMode {
    #[default]
    Fixed,
    Grow,
    Shrink,
}

impl MidiNoteLengthMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Grow => "Grow",
            Self::Shrink => "Shrink",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MidiNoteLengthSettings {
    pub mode: MidiNoteLengthMode,
    pub beats: f64,
}

impl Default for MidiNoteLengthSettings {
    fn default() -> Self {
        Self {
            mode: MidiNoteLengthMode::Fixed,
            beats: 0.25,
        }
    }
}

pub struct MidiNoteLengthResponse {
    pub amount: Rect,
    pub apply: bool,
    pub cancel: bool,
}

/// On-demand controls for an explicit, reversible selected-note length preview.
/// Values are audible beats; model conversion and group limits stay in the host.
pub fn midi_note_length_preview(
    ui: &mut Ui,
    settings: &mut MidiNoteLengthSettings,
    count: usize,
    minimum: f64,
    maximum: f64,
) -> MidiNoteLengthResponse {
    let narrow = ui.available_width() < 560.0;
    ui.horizontal_wrapped(|ui| {
        crate::text::label(ui, "Note length preview");
        crate::text::note(
            ui,
            format!(
                "{count} selected {}",
                if count == 1 { "note" } else { "notes" }
            ),
        );
        if !narrow {
            length_modes(ui, settings, minimum);
        }
    });
    if narrow {
        ui.horizontal_wrapped(|ui| length_modes(ui, settings, minimum));
    }
    let mut result = MidiNoteLengthResponse {
        amount: Rect::NOTHING,
        apply: false,
        cancel: false,
    };
    crate::parameter_row_wrapped(ui, |ui| {
        let label = crate::text::label(
            ui,
            if settings.mode == MidiNoteLengthMode::Fixed {
                "Length"
            } else {
                "Amount"
            },
        );
        result.amount = ui
            .add(
                crate::number(&mut settings.beats)
                    .range(
                        if settings.mode == MidiNoteLengthMode::Fixed {
                            minimum
                        } else {
                            0.0
                        }..=maximum,
                    )
                    .speed(minimum)
                    .max_decimals(8)
                    .suffix(" beats"),
            )
            .labelled_by(label.id)
            .rect;
        result.apply = crate::primary_button(ui, "Apply lengths").clicked();
        result.cancel = crate::button(ui, "Cancel lengths").clicked();
    });
    result
}

fn length_modes(ui: &mut Ui, settings: &mut MidiNoteLengthSettings, minimum: f64) {
    for mode in [
        MidiNoteLengthMode::Fixed,
        MidiNoteLengthMode::Grow,
        MidiNoteLengthMode::Shrink,
    ] {
        if crate::selectable_value(ui, &mut settings.mode, mode, mode.label()).changed()
            && mode == MidiNoteLengthMode::Fixed
        {
            settings.beats = settings.beats.max(minimum);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiClipAction {
    HalfTime,
    DoubleTime,
    Transpose(i16),
    Quantize,
    Humanize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiNoteFlip {
    Horizontal,
    Vertical,
}

/// Selected-note mirrors use familiar time/pitch axes and the same commands
/// wherever a host exposes its note-editing menu.
pub fn midi_note_flip_menu_contents(ui: &mut Ui, enabled: bool) -> Option<MidiNoteFlip> {
    let mut action = None;
    for (label, hint, axis) in [
        (
            "Flip horizontally",
            "Mirror selected notes within their time span; keep lengths, pitches and velocities",
            MidiNoteFlip::Horizontal,
        ),
        (
            "Flip vertically",
            "Invert selected pitches within their range; keep timing, lengths and velocities",
            MidiNoteFlip::Vertical,
        ),
    ] {
        if ui
            .add_enabled(enabled, crate::button_widget(label))
            .on_hover_text(hint)
            .clicked()
        {
            action = Some(axis);
            ui.close();
        }
    }
    action
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MidiClipActionAvailability {
    pub has_notes: bool,
    pub half_time: bool,
    pub double_time: bool,
    pub semitone_up: bool,
    pub semitone_down: bool,
    pub octave_up: bool,
    pub octave_down: bool,
}

/// Menu contents shared by clip context menus and the MIDI editor's Clip menu.
/// The caller owns validation, transformation and the undo transaction.
pub fn midi_clip_action_menu_contents(
    ui: &mut Ui,
    available: MidiClipActionAvailability,
) -> Option<MidiClipAction> {
    let mut result = None;
    for (label, hint, enabled, action) in [
        (
            "Half time",
            "Double the clip length; preserve the source notes and start position",
            available.half_time,
            MidiClipAction::HalfTime,
        ),
        (
            "Double time",
            "Halve the clip length; preserve the source notes and start position",
            available.double_time,
            MidiClipAction::DoubleTime,
        ),
    ] {
        if ui
            .add_enabled(enabled, crate::button_widget(label))
            .on_hover_text(hint)
            .clicked()
        {
            result = Some(action);
            ui.close();
        }
    }
    ui.separator();
    for (label, enabled, semitones) in [
        ("Semitone up", available.semitone_up, 1),
        ("Semitone down", available.semitone_down, -1),
        ("Octave up", available.octave_up, 12),
        ("Octave down", available.octave_down, -12),
    ] {
        if ui.add_enabled(enabled, crate::button_widget(label))
            .on_hover_text("Transpose all source notes, including trimmed notes and linked occurrences; timing and velocity stay unchanged")
            .clicked()
        {
            result = Some(MidiClipAction::Transpose(semitones));
            ui.close();
        }
    }
    ui.separator();
    for (label, hint, action) in [
        (
            "Quantize…",
            "Preview timing changes for all notes audible in this clip window",
            MidiClipAction::Quantize,
        ),
        (
            "Humanize…",
            "Preview a repeatable velocity and timing variation for this clip window",
            MidiClipAction::Humanize,
        ),
    ] {
        if ui
            .add_enabled(available.has_notes, crate::button_widget(label))
            .on_hover_text(hint)
            .clicked()
        {
            result = Some(action);
            ui.close();
        }
    }
    result
}
