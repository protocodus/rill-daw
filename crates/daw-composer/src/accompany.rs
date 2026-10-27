//! Small accompaniment idioms over an already resolved, immutable progression.
use crate::{
    ArpeggioStyle, BassStyle, DraftPart, MAX_BASS_OCTAVE, MAX_BEATS_PER_CHORD, MAX_CHORDS,
    MAX_DURATION, MAX_NOTES, MAX_PARTS, MAX_STEP, MIN_BASS_OCTAVE, MIN_BEATS_PER_CHORD, MIN_STEP,
    PartSpec, Progression, ProgressionChord, TimedNote, notes::parse_note,
};
use anyhow::{Result, ensure};

struct PreparedPart {
    pitches: Vec<Vec<u8>>,
    slots: usize,
    note_length: f64,
    step: f64,
    velocity: u8,
    kind: PreparedKind,
}

#[derive(Clone, Copy)]
enum PreparedKind {
    Chords,
    Arpeggio(ArpeggioStyle),
    Bass(BassStyle),
}

/// Build all requested parts or fail without returning a partial arrangement.
/// Durations and positions use quarter-note beats, relative to the draft start.
pub fn compose(
    progression: &Progression,
    beats_per_chord: f64,
    parts: &[PartSpec],
) -> Result<Vec<DraftPart>> {
    validate_progression(progression)?;
    ensure!(
        beats_per_chord.is_finite()
            && (MIN_BEATS_PER_CHORD..=MAX_BEATS_PER_CHORD).contains(&beats_per_chord),
        "beats_per_chord must be finite and between 0.25 and 64"
    );
    ensure!(
        !parts.is_empty() && parts.len() <= MAX_PARTS,
        "Compose between 1 and {MAX_PARTS} parts"
    );
    let length = progression.chords.len() as f64 * beats_per_chord;
    ensure!(
        length <= MAX_DURATION,
        "Composition exceeds {MAX_DURATION} beats"
    );
    let mut total_notes = 0_usize;
    let mut prepared = Vec::with_capacity(parts.len());
    // Inspect counts, pitches and durations before allocating the note arrays.
    for part in parts {
        let name = part.name();
        ensure!(
            !name.trim().is_empty()
                && name.chars().count() <= 128
                && !name.chars().any(char::is_control),
            "Part names must contain 1 to 128 printable characters"
        );
        let plan = prepare_part(progression, beats_per_chord, part)?;
        let notes = match plan.kind {
            PreparedKind::Chords => plan.pitches.iter().map(Vec::len).sum(),
            PreparedKind::Arpeggio(_) => progression.chords.len() * plan.slots,
            PreparedKind::Bass(BassStyle::Root) => progression.chords.len(),
            PreparedKind::Bass(BassStyle::RootFifth) => {
                progression.chords.len() * plan.slots.div_ceil(2)
            }
            PreparedKind::Bass(BassStyle::RootOctave) => progression.chords.len() * plan.slots,
        };
        total_notes = total_notes
            .checked_add(notes)
            .ok_or_else(|| anyhow::anyhow!("Composition note count overflow"))?;
        ensure!(
            total_notes <= MAX_NOTES,
            "Composition exceeds {MAX_NOTES} notes across all parts"
        );
        ensure!(
            plan.note_length.is_finite()
                && plan.note_length > 0.0
                && length + plan.note_length > length,
            "Gate creates a note length too small to represent in this composition"
        );
        prepared.push(plan);
    }
    let mut drafts = Vec::with_capacity(parts.len());
    for (part, plan) in parts.iter().zip(prepared) {
        let mut notes = Vec::new();
        for (chord_index, pitches) in plan.pitches.iter().enumerate() {
            let start = chord_index as f64 * beats_per_chord;
            let end = (chord_index + 1) as f64 * beats_per_chord;
            // Exact rational grids such as .3 / .1 need a tolerance when
            // counting steps. Keep their roundoff fringe inside the chord.
            let event = |pitch, event_start| TimedNote {
                pitch,
                start: event_start,
                length: plan.note_length.min(end - event_start),
                velocity: plan.velocity,
            };
            match plan.kind {
                PreparedKind::Chords => {
                    for &pitch in pitches {
                        notes.push(event(pitch, start));
                    }
                }
                PreparedKind::Arpeggio(style) => {
                    for slot in 0..plan.slots {
                        let pitch = pitches[arpeggio_index(style, slot, pitches.len())];
                        notes.push(event(pitch, start + slot as f64 * plan.step));
                    }
                }
                PreparedKind::Bass(BassStyle::Root) => {
                    notes.push(event(pitches[0], start));
                }
                PreparedKind::Bass(style) => {
                    let stride = if style == BassStyle::RootFifth { 2 } else { 1 };
                    for (index, slot) in (0..plan.slots).step_by(stride).enumerate() {
                        notes.push(event(pitches[index % 2], start + slot as f64 * plan.step));
                    }
                }
            }
        }
        drafts.push(DraftPart {
            name: part.name().to_owned(),
            length,
            notes,
        });
    }
    Ok(drafts)
}

fn validate_progression(progression: &Progression) -> Result<()> {
    ensure!(
        !progression.chords.is_empty() && progression.chords.len() <= MAX_CHORDS,
        "Provide between 1 and {MAX_CHORDS} chords"
    );
    for chord in &progression.chords {
        ensure!(
            (1..=7).contains(&chord.degree),
            "Chord degrees must be between 1 and 7"
        );
        ensure!(
            chord.root_pitch_class < 12,
            "Chord root pitch class must be between 0 and 11"
        );
        ensure!(
            !chord.midi.is_empty()
                && chord.midi.len() <= 12
                && chord.midi.len() == chord.notes.len(),
            "Each chord must have 1 to 12 matching pitches and note names"
        );
        ensure!(
            chord.midi.iter().all(|pitch| *pitch <= 127),
            "Chord pitches must be within MIDI 0 to 127"
        );
        ensure!(
            chord
                .midi
                .iter()
                .any(|pitch| *pitch % 12 == chord.root_pitch_class),
            "The harmonic root must be present in its chord"
        );
        for (name, &pitch) in chord.notes.iter().zip(&chord.midi) {
            ensure!(
                parse_note(name)?.midi() == Some(i16::from(pitch)),
                "Chord note names must match their concrete MIDI pitches"
            );
        }
    }
    Ok(())
}

fn prepare_part(progression: &Progression, beats: f64, part: &PartSpec) -> Result<PreparedPart> {
    let (velocity, gate, step, kind, shift) = match part {
        PartSpec::Chords {
            velocity,
            gate,
            octave_shift,
            ..
        } => (*velocity, *gate, beats, PreparedKind::Chords, *octave_shift),
        PartSpec::Arpeggio {
            style,
            step,
            velocity,
            gate,
            octave_shift,
            ..
        } => (
            *velocity,
            *gate,
            *step,
            PreparedKind::Arpeggio(*style),
            *octave_shift,
        ),
        PartSpec::Bass {
            style,
            step,
            velocity,
            gate,
            ..
        } => (*velocity, *gate, *step, PreparedKind::Bass(*style), 0),
    };
    ensure!(
        (1..=127).contains(&velocity),
        "velocity must be between 1 and 127"
    );
    ensure!(
        gate.is_finite() && gate > 0.0 && gate <= 1.0,
        "gate must be finite, greater than 0 and at most 1"
    );
    ensure!(
        step.is_finite() && (MIN_STEP..=MAX_STEP).contains(&step),
        "step must be finite and between 1/64 and 64 beats"
    );
    // A held root is one note per chord, so its effective grid is the chord
    // itself. The common step field remains range-checked but only controls
    // rhythmic bass styles; its one-beat default must not reject shorter chords.
    let step = if matches!(kind, PreparedKind::Bass(BassStyle::Root)) {
        beats
    } else {
        step
    };
    let ratio = beats / step;
    ensure!(
        ratio >= 1.0 && (ratio - ratio.round()).abs() <= 1e-9,
        "Each chord must contain a whole number of steps"
    );
    let slots = ratio.round() as usize;
    let mut pitches = Vec::with_capacity(progression.chords.len());
    for chord in &progression.chords {
        let row = if let PartSpec::Bass { octave, style, .. } = part {
            ensure!(
                (MIN_BASS_OCTAVE..=MAX_BASS_OCTAVE).contains(octave),
                "bass octave must be between 0 and 8"
            );
            let root_note = chord
                .notes
                .iter()
                .map(|name| parse_note(name))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .find(|note| note.pitch_class() == chord.root_pitch_class)
                .expect("validated harmonic root");
            let root = checked_pitch(
                root_note
                    .with_octave(i16::from(*octave))
                    .midi()
                    .expect("explicit bass octave"),
            )?;
            match style {
                BassStyle::Root => vec![root],
                BassStyle::RootFifth => {
                    let fifth_pc = fifth_pitch_class(chord)?;
                    let interval =
                        (i16::from(fifth_pc) - i16::from(chord.root_pitch_class)).rem_euclid(12);
                    let fifth = if interval == 0 {
                        root
                    } else {
                        checked_pitch(i16::from(root) + interval - 12)?
                    };
                    vec![root, fifth]
                }
                BassStyle::RootOctave => vec![root, checked_pitch(i16::from(root) + 12)?],
            }
        } else {
            let mut row = chord
                .midi
                .iter()
                .map(|pitch| checked_pitch(i16::from(*pitch) + i16::from(shift) * 12))
                .collect::<Result<Vec<_>>>()?;
            row.sort_unstable();
            if matches!(kind, PreparedKind::Arpeggio(ArpeggioStyle::Alberti)) {
                ensure!(
                    row.len() >= 3,
                    "Alberti accompaniment requires at least three chord tones"
                );
            }
            row
        };
        pitches.push(row);
    }
    let note_length = match kind {
        PreparedKind::Chords | PreparedKind::Bass(BassStyle::Root) => beats * gate,
        _ => step * gate,
    };
    Ok(PreparedPart {
        pitches,
        slots,
        note_length,
        step,
        velocity,
        kind,
    })
}

fn checked_pitch(pitch: i16) -> Result<u8> {
    ensure!(
        (0..=127).contains(&pitch),
        "Part octave places a note outside MIDI 0 to 127"
    );
    Ok(pitch as u8)
}

fn fifth_pitch_class(chord: &ProgressionChord) -> Result<u8> {
    let tones = chord
        .notes
        .iter()
        .map(|name| parse_note(name))
        .collect::<Result<Vec<_>>>()?;
    let root = tones
        .iter()
        .find(|note| note.pitch_class() == chord.root_pitch_class)
        .expect("validated root");
    if let Some(fifth) = tones
        .iter()
        .find(|note| note.letter == (root.letter + 4) % 7)
    {
        return Ok(fifth.pitch_class());
    }
    // Practical respelling can hide the fifth's staff letter in extreme keys.
    Ok([7, 6, 8]
        .into_iter()
        .map(|interval| (chord.root_pitch_class + interval) % 12)
        .find(|pc| chord.midi.iter().any(|p| p % 12 == *pc))
        .unwrap_or(chord.root_pitch_class))
}

fn arpeggio_index(style: ArpeggioStyle, slot: usize, count: usize) -> usize {
    match style {
        ArpeggioStyle::Up => slot % count,
        ArpeggioStyle::Down => count - 1 - slot % count,
        ArpeggioStyle::UpDown if count > 1 => {
            let index = slot % (2 * count - 2);
            if index < count {
                index
            } else {
                2 * count - 2 - index
            }
        }
        ArpeggioStyle::UpDown => 0,
        ArpeggioStyle::Alberti => [0, 2, 1, 2][slot % 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn progression_fixture() -> Progression {
        Progression {
            root: "C4".into(),
            scale: "major".into(),
            chords: vec![
                ProgressionChord {
                    degree: 1,
                    name: "C".into(),
                    root_pitch_class: 0,
                    notes: vec!["C4".into(), "E4".into(), "G4".into()],
                    midi: vec![60, 64, 67],
                },
                ProgressionChord {
                    degree: 7,
                    name: "Bdim".into(),
                    root_pitch_class: 11,
                    notes: vec!["B3".into(), "D4".into(), "F4".into()],
                    midi: vec![59, 62, 65],
                },
            ],
        }
    }

    #[test]
    fn parts_keep_the_full_timeline_and_distinct_accompaniment_rules() {
        let specs = serde_json::from_value::<Vec<PartSpec>>(json!([
            {"kind":"chords"}, {"kind":"arpeggio", "style":"alberti"},
            {"kind":"bass", "style":"root_fifth"}, {"kind":"bass", "style":"root_octave"}, {"kind":"bass"}
        ])).unwrap();
        let parts = compose(&progression_fixture(), 4.0, &specs).unwrap();
        assert!(parts.iter().all(|p| p.length == 8.0));
        assert_eq!(parts[0].notes.len(), 6);
        assert_eq!(parts[0].notes[0].length, 3.6);
        assert_eq!(
            parts[1]
                .notes
                .iter()
                .take(4)
                .map(|n| n.pitch)
                .collect::<Vec<_>>(),
            [60, 67, 64, 67]
        );
        assert_eq!(
            parts[2]
                .notes
                .iter()
                .map(|n| (n.pitch, n.start))
                .collect::<Vec<_>>(),
            [(36, 0.0), (31, 2.0), (47, 4.0), (41, 6.0)]
        );
        assert_eq!(
            parts[3]
                .notes
                .iter()
                .take(4)
                .map(|n| n.pitch)
                .collect::<Vec<_>>(),
            [36, 48, 36, 48]
        );
        assert_eq!(
            parts[4]
                .notes
                .iter()
                .map(|n| (n.pitch, n.start, n.length))
                .collect::<Vec<_>>(),
            [(36, 0.0, 3.6), (47, 4.0, 3.6)]
        );
    }

    #[test]
    fn inversion_does_not_change_the_bass_root_or_its_actual_diminished_fifth() {
        let mut p = progression_fixture();
        p.chords[1].notes = vec!["D4".into(), "F4".into(), "B4".into()];
        p.chords[1].midi = vec![62, 65, 71];
        let spec = serde_json::from_value::<PartSpec>(json!({"kind":"bass", "style":"root_fifth"}))
            .unwrap();
        let notes = compose(&p, 4.0, &[spec]).unwrap().remove(0).notes;
        assert_eq!(notes[2].pitch, 47);
        assert_eq!(notes[3].pitch, 41);
    }

    #[test]
    fn bass_octave_follows_the_spelled_letter_at_enharmonic_boundaries() {
        let mut p = progression_fixture();
        p.chords[0] = ProgressionChord {
            degree: 1,
            name: "Cb".into(),
            root_pitch_class: 11,
            notes: vec!["Cb4".into(), "Eb4".into(), "Gb4".into()],
            midi: vec![59, 63, 66],
        };
        let part = serde_json::from_value::<PartSpec>(json!({"kind":"bass"})).unwrap();
        let notes = compose(&p, 4.0, &[part]).unwrap().remove(0).notes;
        assert_eq!(notes[0].pitch, 35); // Cb2, rather than B2.
    }

    #[test]
    fn held_root_bass_works_for_short_chords_without_a_matching_step_grid() {
        let p = progression_fixture();
        for (beats, step) in [(0.25, 1.0), (0.3, 1.0), (0.25, 64.0)] {
            let part = serde_json::from_value::<PartSpec>(json!({
                "kind":"bass", "style":"root", "step":step, "gate":1.0
            }))
            .unwrap();
            let drafts = compose(&p, beats, &[part]).unwrap();
            assert_eq!(drafts[0].length, 2.0 * beats);
            assert_eq!(drafts[0].notes.len(), 2);
            assert_eq!(drafts[0].notes[0].pitch, 36);
            assert_eq!(drafts[0].notes[1].pitch, 47);
            assert_eq!(drafts[0].notes[0].start, 0.0);
            assert_eq!(drafts[0].notes[1].start, beats);
            assert_eq!(drafts[0].notes[0].length, beats);
            assert_eq!(drafts[0].notes[1].length, beats);
        }
        let rhythmic = serde_json::from_value::<PartSpec>(json!({
            "kind":"bass", "style":"root_octave"
        }))
        .unwrap();
        assert!(compose(&p, 0.25, &[rhythmic]).is_err());
    }

    #[test]
    fn fractional_step_roundoff_stays_inside_each_chord_and_the_full_span() {
        let p = progression_fixture();
        let parts = serde_json::from_value::<Vec<PartSpec>>(json!([
            {"kind":"arpeggio", "step":0.1, "gate":1.0},
            {"kind":"chords", "gate":1.0},
            {"kind":"bass", "style":"root_octave", "step":0.1, "gate":1.0}
        ]))
        .unwrap();
        let drafts = compose(&p, 0.3, &parts).unwrap();
        assert_eq!(drafts[0].notes.len(), 6);
        assert_eq!(drafts[2].notes.len(), 6);
        for part in drafts {
            assert_eq!(part.length, 0.6);
            for note in part.notes {
                let chord_end = if note.start < 0.3 { 0.3 } else { 0.6 };
                assert!(note.length > 0.0);
                assert!(note.start + note.length <= chord_end);
            }
        }
    }

    #[test]
    fn limits_apply_across_parts_and_never_truncate_or_create_invalid_notes() {
        let p = progression_fixture();
        let dense = serde_json::from_value::<PartSpec>(json!({"kind":"arpeggio", "step":0.015625}))
            .unwrap();
        assert_eq!(
            compose(&p, 64.0, std::slice::from_ref(&dense)).unwrap()[0]
                .notes
                .len(),
            MAX_NOTES
        );
        assert!(compose(&p, 64.0, &[dense.clone(), dense]).is_err());
        for spec in [
            json!({"kind":"chords", "octave_shift":8}),
            json!({"kind":"chords", "velocity":0}),
            json!({"kind":"chords", "name":"\n"}),
            json!({"kind":"arpeggio", "step":0.3}),
            json!({"kind":"chords", "gate":1e-300}),
            json!({"kind":"bass", "octave":9}),
            json!({"kind":"chords", "gate":0.0}),
        ] {
            assert!(compose(&p, 4.0, &[serde_json::from_value(spec).unwrap()]).is_err());
        }
        assert!(compose(&p, f64::NAN, &[]).is_err());
        assert!(compose(&p, 4.0, &[]).is_err());
        let mut malformed = p;
        malformed.chords[0].notes[0] = "C#4".into();
        let part = serde_json::from_value::<PartSpec>(json!({"kind":"chords"})).unwrap();
        assert!(compose(&malformed, 4.0, &[part]).is_err());
    }
}
