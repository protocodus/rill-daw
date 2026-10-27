//! Atomic, source-time MIDI transformations shared by editor preview and Apply.
use crate::{MAX_BEAT, Note, new_id};
use anyhow::{Result, ensure};
use std::collections::{HashMap, HashSet};

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
                && note.start + note.length <= MAX_BEAT
                && note.start + note.length > note.start,
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

/// Source notes after a containment merge, with every removed ID mapped directly
/// to its surviving identity. Surviving notes retain their original vector order.
#[derive(Clone, Debug, PartialEq)]
pub struct MidiNoteMerge {
    pub notes: Vec<Note>,
    pub remapped: HashMap<u64, u64>,
}

fn equivalent_note_time(a: f64, b: f64) -> bool {
    // Accommodate a local/source stretch round trip, without merging genuinely
    // different timing. The absolute cap stays far below a sample at any tempo.
    (a - b).abs() <= (8.0 * f64::EPSILON * a.abs().max(b.abs()).max(1.0)).min(1e-9)
}

fn contains_note(outer: &Note, inner: &Note) -> bool {
    (outer.start <= inner.start || equivalent_note_time(outer.start, inner.start))
        && (outer.start + outer.length >= inner.start + inner.length
            || equivalent_note_time(outer.start + outer.length, inner.start + inner.length))
}

fn equivalent_note_range(a: &Note, b: &Note) -> bool {
    equivalent_note_time(a.start, b.start)
        && equivalent_note_time(a.start + a.length, b.start + b.length)
}

fn merge_root(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}

/// Mutable bounds for the existing-note fast path. Leaves keep their initial
/// start-time ordering, while parent bounds follow expanded surviving notes and
/// removed notes. A static index of original ranges would miss expanded owners.
struct ExistingNoteIntervals {
    indices: Vec<usize>,
    leaves: usize,
    bounds: Vec<(f64, f64)>,
    #[cfg(test)]
    visited: std::cell::Cell<usize>,
}

impl ExistingNoteIntervals {
    fn new(mut indices: Vec<usize>, notes: &[Note], positions: &mut [usize]) -> Self {
        indices.sort_by(|a, b| notes[*a].start.total_cmp(&notes[*b].start).then(a.cmp(b)));
        let leaves = indices.len().next_power_of_two();
        let mut result = Self {
            indices,
            leaves,
            bounds: vec![(f64::INFINITY, f64::NEG_INFINITY); leaves * 2],
            #[cfg(test)]
            visited: std::cell::Cell::new(0),
        };
        for (position, &index) in result.indices.iter().enumerate() {
            positions[index] = position;
            result.bounds[leaves + position] =
                (notes[index].start, notes[index].start + notes[index].length);
        }
        for node in (1..leaves).rev() {
            result.refresh(node);
        }
        result
    }

    fn refresh(&mut self, node: usize) {
        let a = self.bounds[node * 2];
        let b = self.bounds[node * 2 + 1];
        self.bounds[node] = (a.0.min(b.0), a.1.max(b.1));
    }

    fn update(&mut self, position: usize, note: Option<&Note>) {
        let mut node = self.leaves + position;
        self.bounds[node] = note.map_or((f64::INFINITY, f64::NEG_INFINITY), |note| {
            (note.start, note.start + note.length)
        });
        while node > 1 {
            node /= 2;
            self.refresh(node);
        }
    }

    fn overlapping(&self, start: f64, end: f64, result: &mut Vec<usize>) {
        #[cfg(test)]
        self.visited.set(0);
        result.clear();
        self.visit(1, start - 1e-9, end + 1e-9, result);
    }

    fn visit(&self, node: usize, start: f64, end: f64, result: &mut Vec<usize>) {
        #[cfg(test)]
        self.visited.set(self.visited.get() + 1);
        let (minimum_start, maximum_end) = self.bounds[node];
        if minimum_start > end || maximum_end < start {
            return;
        }
        if node >= self.leaves {
            result.push(self.indices[node - self.leaves]);
        } else {
            self.visit(node * 2, start, end, result);
            self.visit(node * 2 + 1, start, end, result);
        }
    }
}

/// Merge only newly created or geometrically edited notes with fully overlapping
/// notes of the same pitch. Exact matches and containment merge; partial overlaps
/// and other pitches remain separate. Existing identities/velocities are preferred
/// over affected ones, with exact matches first and vector order breaking ties.
///
/// An edited long note absorbs directly contained notes. An edited short note
/// inside an existing note is consumed into that note, without normalizing other
/// pre-existing overlapping notes. No edit is published on invalid input.
pub fn merge_fully_overlapping_midi_notes(
    notes: &[Note],
    affected: &HashSet<u64>,
) -> Result<MidiNoteMerge> {
    validate_notes(notes, affected)?;
    let mut edited = notes.to_vec();
    let mut parents: Vec<_> = (0..notes.len()).collect();
    let mut alive = vec![true; notes.len()];
    let mut pitches: [Vec<usize>; 128] = std::array::from_fn(|_| Vec::new());
    let mut existing: [Vec<usize>; 128] = std::array::from_fn(|_| Vec::new());
    for (index, note) in notes.iter().enumerate() {
        pitches[usize::from(note.pitch)].push(index);
        if !affected.contains(&note.id) {
            existing[usize::from(note.pitch)].push(index);
        }
    }
    // A static interval index remains valid after merging: containment chooses
    // one of the original outer ranges, and removed entries resolve to its owner.
    let mut prefix_ends: [Vec<f64>; 128] = std::array::from_fn(|_| Vec::new());
    for (pitch, indices) in pitches.iter_mut().enumerate() {
        indices.sort_by(|a, b| notes[*a].start.total_cmp(&notes[*b].start).then(a.cmp(b)));
        let mut end = 0.0_f64;
        for &index in indices.iter() {
            end = end.max(notes[index].start + notes[index].length);
            prefix_ends[pitch].push(end);
        }
    }
    let mut existing_positions = vec![usize::MAX; notes.len()];
    let mut existing_intervals: [_; 128] = std::array::from_fn(|pitch| {
        ExistingNoteIntervals::new(
            std::mem::take(&mut existing[pitch]),
            notes,
            &mut existing_positions,
        )
    });
    let mut existing_candidates = Vec::new();
    let absorb = |target: usize,
                  source: usize,
                  edited: &mut [Note],
                  alive: &mut [bool],
                  parents: &mut [usize],
                  existing_intervals: &mut [ExistingNoteIntervals; 128]| {
        let old = &edited[target];
        let other = &edited[source];
        // Preserve existing source bits when the difference is only roundoff.
        let start = if equivalent_note_time(old.start, other.start) {
            old.start
        } else {
            old.start.min(other.start)
        };
        let old_end = old.start + old.length;
        let other_end = other.start + other.length;
        let end = if equivalent_note_time(old_end, other_end) {
            old_end
        } else {
            old_end.max(other_end)
        };
        if start != old.start || end != old_end {
            edited[target].start = start;
            edited[target].length = end - start;
        }
        alive[source] = false;
        parents[source] = target;
        let pitch = usize::from(edited[target].pitch);
        if existing_positions[target] != usize::MAX {
            existing_intervals[pitch].update(existing_positions[target], Some(&edited[target]));
        }
        if existing_positions[source] != usize::MAX {
            existing_intervals[pitch].update(existing_positions[source], None);
        }
    };
    for (original, note) in notes.iter().enumerate() {
        if !affected.contains(&note.id) || !alive[original] {
            continue;
        }
        let mut current = original;
        loop {
            let pitch = usize::from(edited[current].pitch);
            // This fast path also avoids repeatedly scanning thousands of new
            // notes when they all land inside one existing sustained note.
            existing_intervals[pitch].overlapping(
                edited[current].start,
                edited[current].start + edited[current].length,
                &mut existing_candidates,
            );
            let covering = existing_candidates
                .iter()
                .copied()
                .filter(|&index| {
                    alive[index]
                        && index != current
                        && contains_note(&edited[index], &edited[current])
                })
                .min_by_key(|&index| {
                    (
                        !equivalent_note_range(&edited[index], &edited[current]),
                        index,
                    )
                });
            if let Some(target) = covering {
                absorb(
                    target,
                    current,
                    &mut edited,
                    &mut alive,
                    &mut parents,
                    &mut existing_intervals,
                );
                break;
            }
            let preferred = existing_candidates
                .iter()
                .copied()
                .filter(|&index| {
                    alive[index]
                        && index != current
                        && contains_note(&edited[current], &edited[index])
                })
                .min();
            let start = edited[current].start;
            let end = start + edited[current].length;
            let indices = &pitches[pitch];
            let first = prefix_ends[pitch].partition_point(|&value| value < start - 1e-9);
            let last = indices.partition_point(|&index| notes[index].start <= end + 1e-9);
            let mut seen = HashSet::new();
            let mut contained = Vec::new();
            let mut containers = Vec::new();
            for &index in &indices[first.min(last)..last] {
                let root = merge_root(&mut parents, index);
                if root == current || !seen.insert(root) {
                    continue;
                }
                if contains_note(&edited[current], &edited[root]) {
                    contained.push(root);
                } else if contains_note(&edited[root], &edited[current]) {
                    containers.push(root);
                }
            }
            let target = if !affected.contains(&edited[current].id) {
                current
            } else {
                preferred.unwrap_or_else(|| {
                    contained
                        .iter()
                        .copied()
                        .chain(std::iter::once(current))
                        .min()
                        .unwrap()
                })
            };
            // The preferred existing note is contained or equivalent here.
            // Absorb the whole contained batch without a repeated full scan.
            if target != current {
                absorb(
                    target,
                    current,
                    &mut edited,
                    &mut alive,
                    &mut parents,
                    &mut existing_intervals,
                );
            }
            for source in contained {
                if source != target {
                    absorb(
                        target,
                        source,
                        &mut edited,
                        &mut alive,
                        &mut parents,
                        &mut existing_intervals,
                    );
                }
            }
            current = target;
            let Some(container) = containers.into_iter().min() else {
                break;
            };
            let existing_container = !affected.contains(&edited[container].id);
            let survivor = if !affected.contains(&edited[current].id) {
                current
            } else if existing_container {
                container
            } else {
                current.min(container)
            };
            let removed = if survivor == current {
                container
            } else {
                current
            };
            absorb(
                survivor,
                removed,
                &mut edited,
                &mut alive,
                &mut parents,
                &mut existing_intervals,
            );
            current = survivor;
            if existing_container {
                break;
            }
        }
    }
    let mut remapped = HashMap::new();
    for (index, note) in notes.iter().enumerate() {
        if !alive[index] {
            let root = merge_root(&mut parents, index);
            remapped.insert(note.id, edited[root].id);
        }
    }
    let notes: Vec<_> = edited
        .into_iter()
        .enumerate()
        .filter_map(|(index, note)| alive[index].then_some(note))
        .collect();
    validate_notes(&notes, &HashSet::new())?;
    Ok(MidiNoteMerge { notes, remapped })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn containment_merge_keeps_existing_identity_velocity_and_unrelated_overlaps() {
        let first = Note::new(60, 2.0, 0.5, 77);
        let second = Note::new(60, 4.0, 0.5, 84);
        let partial = Note::new(60, 5.5, 2.0, 91);
        let chord = Note::new(64, 1.0, 5.0, 103);
        let inserted = Note::new(60, 1.0, 5.0, 110);
        let notes = vec![
            first.clone(),
            second.clone(),
            partial.clone(),
            chord.clone(),
            inserted.clone(),
        ];
        let result =
            merge_fully_overlapping_midi_notes(&notes, &HashSet::from([inserted.id])).unwrap();
        let mut expected = first.clone();
        expected.start = 1.0;
        expected.length = 5.0;
        assert_eq!(result.notes, vec![expected, partial, chord]);
        assert_eq!(
            result.remapped,
            HashMap::from([(inserted.id, first.id), (second.id, first.id)])
        );
        assert_eq!(notes[0], first);
        assert_eq!(notes[1], second);

        let a = Note::new(60, 1.0, 4.0, 45);
        let b = Note::new(60, 1.0, 4.0, 88);
        let small = Note::new(60, 2.0, 1.0, 100);
        let result = merge_fully_overlapping_midi_notes(
            &[a.clone(), b.clone(), small.clone()],
            &HashSet::from([small.id]),
        )
        .unwrap();
        assert_eq!(result.notes, vec![a.clone(), b]);
        assert_eq!(result.remapped, HashMap::from([(small.id, a.id)]));

        // A covering old note wins even when an earlier old note sits inside
        // the incoming span; that earlier imported pair stays untouched.
        let inner = Note::new(60, 2.0, 1.0, 65);
        let outer = Note::new(60, 0.0, 6.0, 78);
        let incoming = Note::new(60, 1.0, 4.0, 110);
        let result = merge_fully_overlapping_midi_notes(
            &[inner.clone(), outer.clone(), incoming.clone()],
            &HashSet::from([incoming.id]),
        )
        .unwrap();
        assert_eq!(result.notes, vec![inner, outer.clone()]);
        assert_eq!(result.remapped, HashMap::from([(incoming.id, outer.id)]));
    }

    #[test]
    fn merge_prefers_exact_existing_source_bits_and_accepts_stretch_roundoff_only() {
        let inner = Note::new(60, 2.0, 1.0, 60);
        let exact = Note::new(60, 1.0, 4.0, 83);
        let moved = Note::new(60, 1.0, 4.0, 110);
        let result = merge_fully_overlapping_midi_notes(
            &[inner.clone(), exact.clone(), moved.clone()],
            &HashSet::from([moved.id]),
        )
        .unwrap();
        assert_eq!(result.notes, vec![inner.clone(), exact.clone()]);
        assert_eq!(result.remapped[&moved.id], exact.id);
        assert_eq!(result.remapped, HashMap::from([(moved.id, exact.id)]));

        let duplicate = Note::new(60, exact.start, exact.length, 95);
        let result = merge_fully_overlapping_midi_notes(
            &[exact.clone(), duplicate.clone(), moved.clone()],
            &HashSet::from([moved.id]),
        )
        .unwrap();
        assert_eq!(result.notes, vec![exact.clone(), duplicate]);
        assert_eq!(result.remapped, HashMap::from([(moved.id, exact.id)]));

        let existing = Note::new(62, 0.7, 0.7, 76);
        let mut roundtrip = Note::new(
            62,
            existing.start / 1.3 * 1.3,
            existing.length / 1.3 * 1.3,
            115,
        );
        // Force one ULP at both bounds independently of host constant folding.
        roundtrip.start = f64::from_bits(existing.start.to_bits() + 1);
        let result = merge_fully_overlapping_midi_notes(
            &[existing.clone(), roundtrip.clone()],
            &HashSet::from([roundtrip.id]),
        )
        .unwrap();
        assert_eq!(result.notes, vec![existing.clone()]);
        assert_eq!(result.remapped[&roundtrip.id], existing.id);

        let partial = Note::new(62, existing.start + 1e-6, existing.length, 90);
        let notes = vec![existing, partial.clone()];
        let result =
            merge_fully_overlapping_midi_notes(&notes, &HashSet::from([partial.id])).unwrap();
        assert_eq!(result.notes, notes);
        assert!(result.remapped.is_empty());
    }

    #[test]
    fn affected_merge_cascades_flatten_selection_ids_without_joining_partial_neighbors() {
        let existing = Note::new(60, 2.0, 0.25, 71);
        let small = Note::new(60, 1.0, 2.0, 90);
        let large = Note::new(60, 0.0, 4.0, 100);
        let later_existing = Note::new(60, 3.25, 0.25, 82);
        let partial = Note::new(60, 3.5, 2.0, 94);
        let result = merge_fully_overlapping_midi_notes(
            &[
                existing.clone(),
                small.clone(),
                large.clone(),
                later_existing.clone(),
                partial.clone(),
            ],
            &HashSet::from([small.id, large.id]),
        )
        .unwrap();
        assert_eq!(result.notes.len(), 2);
        assert_eq!(
            (
                result.notes[0].id,
                result.notes[0].start,
                result.notes[0].length,
                result.notes[0].velocity
            ),
            (existing.id, 0.0, 4.0, existing.velocity)
        );
        assert_eq!(result.notes[1], partial);
        for id in [small.id, large.id, later_existing.id] {
            assert_eq!(result.remapped[&id], existing.id);
        }

        let left = Note::new(64, 0.0, 4.0, 80);
        let right = Note::new(64, 1.0, 4.0, 85);
        let inside = Note::new(64, 2.0, 1.0, 90);
        let result = merge_fully_overlapping_midi_notes(
            &[left.clone(), right.clone(), inside.clone()],
            &HashSet::from([left.id, right.id, inside.id]),
        )
        .unwrap();
        assert_eq!(result.notes, vec![left.clone(), right]);
        assert_eq!(result.remapped, HashMap::from([(inside.id, left.id)]));
    }

    #[test]
    fn merge_validation_is_atomic_and_large_pasted_batches_do_not_retain_duplicates() {
        let existing = Note::new(60, 0.0, 8.0, 80);
        let mut notes = vec![existing.clone()];
        let affected: HashSet<_> = (0..8192)
            .map(|_| {
                let note = Note::new(60, 1.0, 0.25, 100);
                let id = note.id;
                notes.push(note);
                id
            })
            .collect();
        let result = merge_fully_overlapping_midi_notes(&notes, &affected).unwrap();
        assert_eq!(result.notes, vec![existing]);
        assert_eq!(result.remapped.len(), 8192);
        assert!(result.remapped.values().all(|id| *id == notes[0].id));
        let generated: Vec<_> = (0..8192).map(|_| Note::new(64, 2.0, 0.5, 93)).collect();
        let generated_ids = generated.iter().map(|note| note.id).collect();
        let collapsed = merge_fully_overlapping_midi_notes(&generated, &generated_ids).unwrap();
        assert_eq!(collapsed.notes, vec![generated[0].clone()]);
        assert_eq!(collapsed.remapped.len(), generated.len() - 1);
        assert!(collapsed.remapped.values().all(|id| *id == generated[0].id));
        assert!(merge_fully_overlapping_midi_notes(&notes, &HashSet::from([u64::MAX])).is_err());
        assert_eq!(notes.len(), 8193);

        let original = notes.clone();
        for invalid in [f64::NAN, f64::INFINITY, -0.1, MAX_BEAT + 1.0] {
            let mut bad = notes.clone();
            bad[0].start = invalid;
            assert!(merge_fully_overlapping_midi_notes(&bad, &affected).is_err());
        }
        let mut duplicate_ids = notes.clone();
        duplicate_ids[1].id = duplicate_ids[0].id;
        assert!(merge_fully_overlapping_midi_notes(&duplicate_ids, &HashSet::new()).is_err());
        let mut degenerate = notes.clone();
        degenerate[0].start = MAX_BEAT;
        degenerate[0].length = f64::MIN_POSITIVE;
        assert!(merge_fully_overlapping_midi_notes(&degenerate, &HashSet::new()).is_err());
        assert_eq!(notes, original);
        let untouched = merge_fully_overlapping_midi_notes(&notes, &HashSet::new()).unwrap();
        assert_eq!(untouched.notes, notes);
        assert!(untouched.remapped.is_empty());
    }
    #[test]
    fn existing_interval_candidates_skip_disjoint_notes_and_follow_expanded_owners() {
        // Count visited tree nodes rather than wall time: each disjoint query
        // must stay logarithmic even in a long, single-pitch arrangement.
        let existing: Vec<_> = (0..20_000)
            .map(|index| Note::new(60, f64::from(index) * 2.0, 0.25, 80))
            .collect();
        let mut positions = vec![usize::MAX; existing.len()];
        let mut index =
            ExistingNoteIntervals::new((0..existing.len()).collect(), &existing, &mut positions);
        let mut candidates = Vec::new();
        for note in &existing {
            index.overlapping(note.start + 1.0, note.start + 1.25, &mut candidates);
            assert!(candidates.is_empty());
            assert!(index.visited.get() <= 2 * usize::BITS as usize);
        }
        let mut expanded = existing[0].clone();
        expanded.length = 1000.0;
        index.update(positions[0], Some(&expanded));
        index.overlapping(999.0, 999.25, &mut candidates);
        assert_eq!(candidates, vec![0]);
        index.update(positions[0], None);
        index.overlapping(999.0, 999.25, &mut candidates);
        assert!(candidates.is_empty());

        let mut notes = existing.clone();
        let affected = existing
            .iter()
            .map(|note| {
                let inserted = Note::new(note.pitch, note.start + 1.0, note.length, 100);
                let id = inserted.id;
                notes.push(inserted);
                id
            })
            .collect();
        let result = merge_fully_overlapping_midi_notes(&notes, &affected).unwrap();
        assert_eq!(result.notes, notes);
        assert!(result.remapped.is_empty());

        // An existing owner enlarged by an earlier edit still wins by original
        // vector order against another covering existing note.
        let first = Note::new(60, 1.0, 1.0, 70);
        let second = Note::new(60, 3.0, 3.0, 80);
        let long = Note::new(60, 0.0, 4.0, 110);
        let small = Note::new(60, 3.25, 0.25, 100);
        let result = merge_fully_overlapping_midi_notes(
            &[first.clone(), second.clone(), long.clone(), small.clone()],
            &HashSet::from([long.id, small.id]),
        )
        .unwrap();
        let mut expected = first.clone();
        expected.start = 0.0;
        expected.length = 4.0;
        assert_eq!(result.notes, vec![expected, second]);
        assert_eq!(
            result.remapped,
            HashMap::from([(long.id, first.id), (small.id, first.id)])
        );
    }

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
