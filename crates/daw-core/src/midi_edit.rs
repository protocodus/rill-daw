//! Atomic, source-time MIDI transformations shared by editor preview and Apply.
use crate::{MAX_BEAT, Note, new_id};
use anyhow::{Result, ensure};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuantizeSettings {
    pub grid: f64,
    pub strength: f32,
    /// Position of each offbeat within its pair: .5 straight, .75 heavy swing.
    pub swing: f32,
    pub ends: bool,
}
impl Default for QuantizeSettings {
    fn default() -> Self {
        Self {
            grid: 0.25,
            strength: 1.0,
            swing: 0.5,
            ends: false,
        }
    }
}

fn validate_notes(notes: &[Note], selected: &HashSet<u64>) -> Result<()> {
    let mut ids = HashSet::new();
    for note in notes {
        ensure!(
            note.id > 0 && note.id < u64::MAX && ids.insert(note.id),
            "Duplicate or invalid note ID"
        );
        ensure!(
            note.pitch < 128
                && note.velocity > 0
                && note.velocity < 128
                && note.start.is_finite()
                && note.start >= 0.0
                && note.length.is_finite()
                && note.length > 0.0
                && note.start + note.length <= MAX_BEAT,
            "Invalid MIDI note"
        );
    }
    ensure!(selected.is_subset(&ids), "Selected notes no longer exist");
    Ok(())
}

fn swing_grid(beat: f64, grid: f64, swing: f32) -> f64 {
    let pair = (beat / (2.0 * grid)).floor();
    let mut closest = 0.0;
    let mut distance = f64::INFINITY;
    for offset in -1..=1 {
        let base = (pair + f64::from(offset)) * 2.0 * grid;
        for candidate in [base, base + 2.0 * grid * f64::from(swing)] {
            if candidate >= 0.0 && (candidate - beat).abs() < distance {
                closest = candidate;
                distance = (candidate - beat).abs();
            }
        }
    }
    closest
}

/// `origin` maps source note time to arrangement time (clip.start-clip.offset).
/// Starts are quantized by default; optional ends quantize independently without
/// ever creating zero-length notes. The input remains unchanged on all errors.
pub fn quantize_notes(
    notes: &[Note],
    selected: &HashSet<u64>,
    origin: f64,
    settings: QuantizeSettings,
) -> Result<Vec<Note>> {
    validate_notes(notes, selected)?;
    ensure!(
        origin.is_finite()
            && origin.abs() <= MAX_BEAT
            && settings.grid.is_finite()
            && (1.0 / 4096.0..=64.0).contains(&settings.grid)
            && settings.strength.is_finite()
            && (0.0..=1.0).contains(&settings.strength)
            && settings.swing.is_finite()
            && (0.5..=0.75).contains(&settings.swing),
        "Invalid quantize settings"
    );
    let move_time = |time: f64| {
        let absolute = time + origin;
        (time
            + (swing_grid(absolute, settings.grid, settings.swing) - absolute)
                * f64::from(settings.strength))
        .max(0.0)
    };
    let mut edited = notes.to_vec();
    for note in &mut edited {
        if selected.contains(&note.id) {
            let end = note.start + note.length;
            note.start = move_time(note.start);
            if settings.ends {
                note.length = (move_time(end) - note.start).max(1.0 / 4096.0);
            }
        }
    }
    validate_notes(&edited, selected)?;
    Ok(edited)
}

/// Move the selection uniformly, limiting the whole group at MIDI/time bounds
/// so chord intervals and rhythmic spacing are preserved.
pub fn move_midi_notes(
    notes: &[Note],
    selected: &HashSet<u64>,
    beats: f64,
    semitones: i16,
) -> Result<Vec<Note>> {
    validate_notes(notes, selected)?;
    ensure!(beats.is_finite(), "Invalid note movement");
    let chosen: Vec<_> = notes
        .iter()
        .filter(|note| selected.contains(&note.id))
        .collect();
    if chosen.is_empty() {
        return Ok(notes.to_vec());
    }
    let start = chosen
        .iter()
        .map(|note| note.start)
        .fold(f64::INFINITY, f64::min);
    let end = chosen
        .iter()
        .map(|note| note.start + note.length)
        .fold(0.0, f64::max);
    let low = chosen.iter().map(|note| note.pitch).min().unwrap() as i16;
    let high = chosen.iter().map(|note| note.pitch).max().unwrap() as i16;
    let beats = beats.clamp(-start, MAX_BEAT - end);
    let semitones = semitones.clamp(-low, 127 - high);
    let mut edited = notes.to_vec();
    for note in &mut edited {
        if selected.contains(&note.id) {
            note.start += beats;
            note.pitch = (i16::from(note.pitch) + semitones) as u8;
        }
    }
    validate_notes(&edited, selected)?;
    Ok(edited)
}

/// End selected notes at the next distinct onset, preserving simultaneous chords.
/// Final notes with no later onset retain their original lengths.
pub fn legato_notes(notes: &[Note], selected: &HashSet<u64>) -> Result<Vec<Note>> {
    validate_notes(notes, selected)?;
    let mut onsets: Vec<_> = notes.iter().map(|note| note.start).collect();
    onsets.sort_by(f64::total_cmp);
    onsets.dedup();
    let mut edited = notes.to_vec();
    for note in &mut edited {
        if selected.contains(&note.id)
            && let Some(next) = onsets.get(onsets.partition_point(|start| *start <= note.start))
        {
            note.length = *next - note.start;
        }
    }
    validate_notes(&edited, selected)?;
    Ok(edited)
}

/// Clipboard notes keep relative spacing and velocities, anchored at zero.
pub fn copy_midi_notes(notes: &[Note], selected: &HashSet<u64>) -> Result<Vec<Note>> {
    validate_notes(notes, selected)?;
    let mut copied: Vec<_> = notes
        .iter()
        .filter(|note| selected.contains(&note.id))
        .cloned()
        .collect();
    let first = copied
        .iter()
        .map(|note| note.start)
        .fold(f64::INFINITY, f64::min);
    for note in &mut copied {
        note.start -= first;
    }
    Ok(copied)
}

pub fn paste_midi_notes(
    notes: &[Note],
    clipboard: &[Note],
    at: f64,
) -> Result<(Vec<Note>, HashSet<u64>)> {
    validate_notes(notes, &HashSet::new())?;
    validate_notes(clipboard, &HashSet::new())?;
    ensure!(
        at.is_finite() && (0.0..=MAX_BEAT).contains(&at),
        "Invalid paste position"
    );
    let mut edited = notes.to_vec();
    let mut inserted = HashSet::new();
    for original in clipboard {
        let mut note = original.clone();
        note.id = new_id();
        note.start += at;
        inserted.insert(note.id);
        edited.push(note);
    }
    validate_notes(&edited, &inserted)?;
    Ok((edited, inserted))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantize_strength_swing_and_source_offsets_preserve_unselected_notes_and_durations() {
        let notes = vec![
            Note::new(60, 1.18, 0.31, 80),
            Note::new(64, 1.18, 0.31, 90),
            Note::new(67, 2.1, 0.4, 100),
        ];
        let selected = HashSet::from([notes[0].id, notes[1].id]);
        let settings = QuantizeSettings {
            strength: 0.5,
            swing: 0.75,
            ..Default::default()
        };
        let edited = quantize_notes(&notes, &selected, -0.1, settings).unwrap();
        // Source1.18 maps to arrangement1.08, halfway toward straight pair1.0.
        assert!((edited[0].start - 1.14).abs() < 1e-10);
        assert_eq!(edited[0].start, edited[1].start);
        assert_eq!(edited[0].length, notes[0].length);
        assert_eq!(edited[2], notes[2]);
        let straight = quantize_notes(&notes, &selected, 0.0, QuantizeSettings::default()).unwrap();
        let swing = quantize_notes(
            &notes,
            &selected,
            0.0,
            QuantizeSettings {
                swing: 0.75,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(straight[0].start, 1.25);
        assert_eq!(swing[0].start, 1.0);
        let tiny = vec![Note::new(60, 0.12, 0.001, 100)];
        let ends = quantize_notes(
            &tiny,
            &HashSet::from([tiny[0].id]),
            0.0,
            QuantizeSettings {
                ends: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(ends[0].length > 0.0);
        assert!(
            quantize_notes(
                &notes,
                &selected,
                0.0,
                QuantizeSettings {
                    strength: f32::NAN,
                    ..settings
                }
            )
            .is_err()
        );
    }
    #[test]
    fn legato_preserves_chords_and_last_onset_lengths() {
        let notes = vec![
            Note::new(60, 0.0, 0.2, 90),
            Note::new(64, 0.0, 0.4, 100),
            Note::new(67, 1.0, 2.0, 110),
            Note::new(72, 2.0, 0.5, 80),
        ];
        let selected = notes.iter().map(|note| note.id).collect();
        let edited = legato_notes(&notes, &selected).unwrap();
        assert_eq!(
            edited.iter().map(|note| note.length).collect::<Vec<_>>(),
            vec![1.0, 1.0, 1.0, 0.5]
        );
        assert_eq!(
            edited
                .iter()
                .map(|note| (note.id, note.pitch, note.velocity))
                .collect::<Vec<_>>(),
            notes
                .iter()
                .map(|note| (note.id, note.pitch, note.velocity))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn group_bounds_clipboard_ids_and_invalid_paste_are_atomic() {
        let notes = vec![Note::new(120, 1.0, 0.5, 90), Note::new(127, 1.5, 0.5, 100)];
        let selected = notes.iter().map(|note| note.id).collect();
        let bounded = move_midi_notes(&notes, &selected, -2.0, 12).unwrap();
        assert_eq!(bounded[0].start, 0.0);
        assert_eq!(bounded[1].start, 0.5);
        assert_eq!(bounded[0].pitch, 120);
        let copied = copy_midi_notes(&notes, &selected).unwrap();
        assert_eq!(copied[0].start, 0.0);
        let (pasted, ids) = paste_midi_notes(&notes, &copied, 4.0).unwrap();
        assert_eq!(pasted[2].start, 4.0);
        assert_eq!(pasted[3].start, 4.5);
        assert!(ids.is_disjoint(&selected));
        assert!(paste_midi_notes(&notes, &copied, MAX_BEAT).is_err());
        assert_eq!(notes.len(), 2);
    }
}
