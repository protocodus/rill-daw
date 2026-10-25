//! Shared piano-key and velocity-marker primitives for music editors.
use crate::theme;
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

/// Ordered visible MIDI pitches, independent of the musical model. Every piano
/// interaction uses this mapping so folding keys never changes hidden notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PianoRows {
    pitches: Vec<u8>,
    rows: [i16; 128],
}

impl PianoRows {
    pub fn new(mut visible: impl FnMut(u8) -> bool) -> Self {
        let mut pitches: Vec<_> = (0..=127).rev().filter(|pitch| visible(*pitch)).collect();
        // Keep pointer mapping total even if an external filter is empty.
        if pitches.is_empty() {
            pitches.extend((0..=127).rev());
        }
        let mut rows = [-1; 128];
        for (row, pitch) in pitches.iter().enumerate() {
            rows[usize::from(*pitch)] = row as i16;
        }
        Self { pitches, rows }
    }

    pub fn pitches(&self) -> &[u8] {
        &self.pitches
    }

    pub fn row(&self, pitch: u8) -> Option<usize> {
        self.rows
            .get(usize::from(pitch))
            .copied()
            .filter(|row| *row >= 0)
            .map(|row| row as usize)
    }

    pub fn pitch_at(&self, row: f32) -> u8 {
        self.pitches[(row.floor().max(0.0) as usize).min(self.pitches.len() - 1)]
    }

    pub fn nearest_row(&self, pitch: u8) -> usize {
        self.pitches
            .iter()
            .enumerate()
            .min_by_key(|(_, candidate)| candidate.abs_diff(pitch))
            .map_or(0, |(row, _)| row)
    }

    /// Clamp a group movement at the keyboard ends without collapsing pitches.
    pub fn clamp_move(&self, pitches: impl Iterator<Item = u8>, rows: i16) -> i16 {
        let mut minimum = -(self.pitches.len() as i16 - 1);
        let mut maximum = self.pitches.len() as i16 - 1;
        for pitch in pitches {
            if let Some(row) = self.row(pitch) {
                minimum = minimum.max(-(row as i16));
                maximum = maximum.min(self.pitches.len() as i16 - 1 - row as i16);
            }
        }
        rows.clamp(minimum, maximum)
    }
}

/// Reserve a central move target even for short notes. Edge zones never overlap.
pub fn piano_note_edge(rect: Rect, x: f32) -> i8 {
    let edge = (rect.width().max(0.0) / 3.0).min(7.0);
    if x < rect.left() + edge {
        -1
    } else if x > rect.right() - edge {
        1
    } else {
        0
    }
}

/// Soften the grid beyond a clip and retain a precise visible end boundary.
/// This paint-only component neither changes layout nor captures editor input.
pub fn paint_clip_tail(painter: &Painter, plot: Rect, end_x: f32) -> Option<Rect> {
    if !plot.is_positive() || !plot.is_finite() || !end_x.is_finite() || end_x >= plot.right() {
        return None;
    }
    let colors = theme::palette(painter.ctx());
    let tail = Rect::from_min_max(Pos2::new(end_x.max(plot.left()), plot.top()), plot.max);
    let painter = painter.with_clip_rect(plot);
    painter.rect_filled(tail, 0.0, colors.inactive_clip_tint());
    if end_x >= plot.left() {
        painter.line_segment(
            [
                Pos2::new(end_x, plot.top()),
                Pos2::new(end_x, plot.bottom()),
            ],
            Stroke::new(1.5_f32, colors.grid_bar),
        );
    }
    Some(tail)
}

pub fn piano_key(
    painter: &Painter,
    rect: Rect,
    label: &str,
    black: bool,
    highlight: u8,
    active: bool,
) {
    let colors = theme::palette(painter.ctx());
    let base = if black {
        colors.piano_black
    } else {
        colors.piano_white
    };
    let tint = if highlight > 1 {
        colors.note_selected
    } else {
        colors.note
    };
    let amount = if active {
        0.55
    } else if highlight > 0 {
        0.22
    } else {
        0.0
    };
    let fill = Color32::from_rgb(
        egui::lerp(f32::from(base.r())..=f32::from(tint.r()), amount) as u8,
        egui::lerp(f32::from(base.g())..=f32::from(tint.g()), amount) as u8,
        egui::lerp(f32::from(base.b())..=f32::from(tint.b()), amount) as u8,
    );
    painter.rect_filled(rect, 2.0, fill);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0_f32, theme::palette(painter.ctx()).bg),
    );
    if highlight > 0 {
        let color = if highlight > 1 {
            colors.note_selected
        } else {
            colors.note
        };
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.right() - 6.0, rect.top() + 1.0),
                Vec2::new(5.0, (rect.height() - 2.0).max(1.0)),
            ),
            1.0,
            color,
        );
    }
    if !label.is_empty() && rect.height() >= 14.0 && painter.clip_rect().contains_rect(rect) {
        painter.text(
            rect.right_center() - Vec2::new(11.0, 0.0),
            Align2::RIGHT_CENTER,
            label,
            FontId::monospace(crate::tokens::FONT_CANVAS),
            theme::contrast_text(fill),
        );
    }
}

pub fn velocity_marker(painter: &Painter, bounds: Rect, normalized: f32, selected: bool) {
    let y = bounds.bottom() - normalized.clamp(0.0, 1.0) * bounds.height();
    let colors = theme::palette(painter.ctx());
    let color = if selected {
        colors.note_selected
    } else {
        colors.note
    };
    painter.line_segment(
        [
            Pos2::new(bounds.center().x, bounds.bottom()),
            Pos2::new(bounds.center().x, y),
        ],
        Stroke::new(2.0_f32, color),
    );
    painter.rect_filled(
        Rect::from_center_size(Pos2::new(bounds.center().x, y), Vec2::new(7.0, 5.0)),
        1.0,
        color,
    );
    if selected {
        painter.rect_stroke(
            Rect::from_center_size(Pos2::new(bounds.center().x, y), Vec2::new(8.0, 6.0)),
            1.0,
            Stroke::new(0.5_f32, theme::palette(painter.ctx()).text),
            StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_pitch_mapping_is_reversible_and_group_moves_keep_boundaries() {
        let rows = PianoRows::new(|pitch| matches!(pitch % 12, 0 | 2 | 4 | 5 | 7 | 9 | 11));
        for (row, &pitch) in rows.pitches().iter().enumerate() {
            assert_eq!(rows.row(pitch), Some(row));
            assert_eq!(rows.pitch_at(row as f32 + 0.5), pitch);
        }
        assert_eq!(rows.row(61), None);
        assert_eq!(rows.pitch_at(rows.row(60).unwrap() as f32 - 1.0), 62);
        assert_eq!(rows.clamp_move([0, 60].into_iter(), 8), 0);
        assert_eq!(rows.clamp_move([60, 127].into_iter(), -8), 0);
        assert_eq!(rows.clamp_move([60, 64].into_iter(), -2), -2);
        assert_eq!(rows.pitch_at(-500.0), 127);
        assert_eq!(rows.pitch_at(500.0), 0);
        assert_eq!(PianoRows::new(|_| false).pitches().len(), 128);
    }

    #[test]
    fn short_notes_keep_a_move_target_between_both_resize_edges() {
        for width in [3.0, 6.0, 12.0, 30.0, 150.0] {
            let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(width, 18.0));
            assert_eq!(piano_note_edge(rect, rect.center().x), 0);
            assert_eq!(piano_note_edge(rect, rect.left() + 0.1), -1);
            assert_eq!(piano_note_edge(rect, rect.right() - 0.1), 1);
        }
    }
}
