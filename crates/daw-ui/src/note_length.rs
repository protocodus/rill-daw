//! Musical drawing lengths for note editors. The caller owns its remembered
//! gesture length and converts audible beats into the document's source time.
use egui::{Align2, FontId, Painter, Pos2, Rect, Response, Ui};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawNoteLength {
    #[default]
    LastNote,
    Sixteenth,
    Eighth,
    Quarter,
    Half,
    Bar,
}

impl DrawNoteLength {
    pub const ALL: [Self; 6] = [
        Self::LastNote,
        Self::Sixteenth,
        Self::Eighth,
        Self::Quarter,
        Self::Half,
        Self::Bar,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::LastNote => "Last note",
            Self::Sixteenth => "1/16 note",
            Self::Eighth => "1/8 note",
            Self::Quarter => "1/4 note",
            Self::Half => "1/2 note",
            Self::Bar => "1 bar",
        }
    }

    /// Compact current value for the toolbar chooser ("Length 1/16").
    pub fn short_label(self) -> &'static str {
        match self {
            Self::LastNote => "last",
            Self::Sixteenth => "1/16",
            Self::Eighth => "1/8",
            Self::Quarter => "1/4",
            Self::Half => "1/2",
            Self::Bar => "1 bar",
        }
    }

    /// None retains the editor's remembered drawing length. Explicit choices
    /// are audible quarter-note beats; a bar follows the current meter.
    pub fn beats(self, beats_per_bar: u8) -> Option<f64> {
        match self {
            Self::LastNote => None,
            Self::Sixteenth => Some(0.25),
            Self::Eighth => Some(0.5),
            Self::Quarter => Some(1.0),
            Self::Half => Some(2.0),
            Self::Bar => Some(f64::from(beats_per_bar.max(1))),
        }
    }
}

/// Current-value chooser shown beside Snap. Dragging a new note may override
/// this initial duration; the application retains ownership of that gesture.
pub fn drawing_note_length(ui: &mut Ui, length: &mut DrawNoteLength) -> Response {
    let before = *length;
    let (response, _) =
        crate::quiet_picker_button(ui, format!("Length {}", length.short_label()), |ui| {
            ui.set_min_width(crate::tokens::SELECT_WIDTH);
            for choice in DrawNoteLength::ALL {
                if crate::selectable_value(ui, length, choice, choice.label()).clicked() {
                    ui.close();
                }
            }
        });
    let mut response = response.on_hover_text(
        "Initial length for new notes. Last note starts at 1/16 and remembers the last drawn or selected length. Drag a new note to choose its end.",
    );
    if *length != before {
        response.mark_changed();
    }
    response
}

/// Label a visible clip end in the inactive roll area. This is paint-only;
/// drawing beyond the boundary remains the application's normal note gesture.
pub fn paint_clip_end_label(painter: &Painter, plot: Rect, end_x: f32) -> Option<Rect> {
    if !end_x.is_finite() || end_x < plot.left() || end_x + 64.0 > plot.right() {
        return None;
    }
    Some(painter.with_clip_rect(plot).text(
        Pos2::new(
            end_x + crate::tokens::SPACE_SM,
            plot.top() + crate::tokens::SPACE_SM,
        ),
        Align2::LEFT_TOP,
        "Clip end",
        FontId::proportional(crate::tokens::FONT_CANVAS),
        crate::theme::palette(painter.ctx()).muted,
    ))
}
