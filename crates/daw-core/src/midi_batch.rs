//! Selected-note duration edits in audible beats, preserving source precision.
use crate::{
    ClipContent, MAX_BATCH_NOTES, MAX_BEAT, MAX_SOURCE_RATE, MIN_SOURCE_RATE, Note, Project,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MIN_MIDI_NOTE_LENGTH: f64 = 1.0 / 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MidiNoteLengthChange {
    Set { length: f64 },
    Add { delta: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MidiNoteLengthResult {
    pub changed: usize,
    /// Actual uniform change in audible beats; None for fixed lengths.
    pub applied_delta: Option<f64>,
}

/// Preview and commit share this source-time transformation. An additive edit
/// stops the whole group at its shortest note, preserving length differences.
/// Imported shorter notes are not lengthened just because the group hit a bound.
/// Expression points beyond a shortened end remain retained and editable.
pub fn midi_note_lengths(
    notes: &[Note],
    selected: &HashSet<u64>,
    source_rate: f64,
    change: MidiNoteLengthChange,
) -> Result<(Vec<Note>, MidiNoteLengthResult)> {
    ensure!(
        source_rate.is_finite() && (MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&source_rate),
        "Invalid clip source rate"
    );
    ensure!(
        !selected.is_empty() && selected.len() <= MAX_BATCH_NOTES,
        "Select notes to change length"
    );
    let mut ids = HashSet::new();
    for note in notes {
        crate::validate_note(note)?;
        ensure!(
            note.id > 0 && note.id < u64::MAX && ids.insert(note.id),
            "Duplicate or invalid note identity"
        );
    }
    ensure!(selected.is_subset(&ids), "Selected notes no longer exist");
    let source_minimum = MIN_MIDI_NOTE_LENGTH * source_rate;
    let mut edited = notes.to_vec();
    let applied_delta = match change {
        MidiNoteLengthChange::Set { length } => {
            ensure!(
                length.is_finite() && (MIN_MIDI_NOTE_LENGTH..=MAX_BEAT).contains(&length),
                "Invalid fixed note length"
            );
            let length = length * source_rate;
            ensure!(
                length.is_finite(),
                "Fixed note length exceeds source bounds"
            );
            for note in edited.iter_mut().filter(|note| selected.contains(&note.id)) {
                note.length = length;
            }
            None
        }
        MidiNoteLengthChange::Add { delta } => {
            ensure!(
                delta.is_finite() && delta.abs() <= MAX_BEAT,
                "Invalid note length change"
            );
            let minimum = notes
                .iter()
                .filter(|note| selected.contains(&note.id))
                .map(|note| note.length.min(source_minimum) - note.length)
                .fold(f64::NEG_INFINITY, f64::max);
            let maximum = notes
                .iter()
                .filter(|note| selected.contains(&note.id))
                .map(|note| MAX_BEAT - (note.start + note.length))
                .fold(f64::INFINITY, f64::min);
            let delta = (delta * source_rate).clamp(minimum, maximum);
            if delta != 0.0 {
                for note in edited.iter_mut().filter(|note| selected.contains(&note.id)) {
                    note.length += delta;
                }
            }
            Some(delta / source_rate)
        }
    };
    for note in &edited {
        crate::validate_note(note)?;
    }
    let changed = notes
        .iter()
        .zip(&edited)
        .filter(|(before, after)| before != after)
        .count();
    Ok((
        edited,
        MidiNoteLengthResult {
            changed,
            applied_delta,
        },
    ))
}

impl Project {
    /// Atomic fixed/additive duration edit. Only selected source lengths change;
    /// linked occurrences share those edits but keep their own trim/stretch.
    pub fn edit_midi_note_lengths(
        &mut self,
        track: u64,
        clip: u64,
        selected: &HashSet<u64>,
        change: MidiNoteLengthChange,
    ) -> Result<MidiNoteLengthResult> {
        let source = self.clip(track, clip).context("MIDI clip not found")?;
        let ClipContent::Midi { notes } = &source.content else {
            anyhow::bail!("Choose a MIDI clip")
        };
        let (notes, result) = midi_note_lengths(notes, selected, source.source_rate, change)?;
        if result.changed > 0 {
            let mut candidate = self.clone();
            *candidate
                .midi_notes_mut(track, clip)
                .expect("validated MIDI clip") = notes;
            candidate.synchronize_linked_clip(track, clip)?;
            candidate.validate()?;
            *self = candidate;
        }
        Ok(result)
    }
}
