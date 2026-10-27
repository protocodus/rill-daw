//! Diatonic stacked-thirds progressions and upstream's greedy close voicings.
use crate::catalog::{catalog, find_scale};
use crate::notes::{LETTER_PCS, Note, degree_steps, parse_note, transpose};
use crate::{ChordDefinition, MAX_CHORDS, Progression, ProgressionChord};
use anyhow::{Result, anyhow, ensure};

/// Resolve integer degree positions in a heptatonic scale, not literal Roman
/// numerals or modal interchange. The root must carry an octave (C4 = MIDI 60).
/// Smoothing follows upstream voice_leading: greedy inversion/register search
/// about that octave with common-tone and inversion tie breaks.
pub fn progression(
    root: &str,
    scale: &str,
    degrees: &[u8],
    sevenths: bool,
    smooth: bool,
) -> Result<Progression> {
    ensure!(
        !degrees.is_empty() && degrees.len() <= MAX_CHORDS,
        "Choose between 1 and {MAX_CHORDS} chord degrees"
    );
    let tonic = parse_note(root)?;
    let octave = tonic
        .octave
        .ok_or_else(|| anyhow!("Progression roots require an octave, for example C4 or Bb3"))?;
    let scale = find_scale(scale)?;
    ensure!(
        scale.intervals.len() == 7 && scale.degrees.len() == 7,
        "Progressions currently require a seven-note scale"
    );
    ensure!(
        degrees.iter().all(|d| (1..=7).contains(d)),
        "Diatonic degree positions must be integers from 1 to 7"
    );
    let chord_definitions = catalog().chords;
    let mut chords = Vec::with_capacity(degrees.len());
    let mut previous: Option<Vec<Note>> = None;
    for &degree in degrees {
        let index = usize::from(degree - 1);
        let count = if sevenths { 4 } else { 3 };
        let mut spelled = Vec::with_capacity(count);
        let mut concrete = Vec::with_capacity(count);
        for voice in 0..count {
            let position = index + 2 * voice;
            let carry = (position / 7) as i16;
            let semitones = scale.intervals[position % 7] + 12 * carry;
            let letters = degree_steps(&scale.degrees[position % 7])? + 7 * carry;
            spelled.push(transpose(tonic.without_octave(), semitones, Some(letters))?);
            if !smooth {
                concrete.push(transpose(tonic, semitones, Some(letters))?);
            }
        }
        let name = chord_name(&spelled, &chord_definitions);
        let root_pitch_class = spelled[0].pitch_class();
        let voiced = if smooth {
            match &previous {
                None => stack(&spelled, octave)?,
                Some(previous) => smooth_voicing(&spelled, octave, previous)?,
            }
        } else {
            concrete
        };
        let midi = voiced
            .iter()
            .map(|n| {
                let midi = n.midi().expect("voicing is concrete");
                ensure!(
                    (0..=127).contains(&midi),
                    "Voicing falls outside MIDI 0–127"
                );
                Ok(midi as u8)
            })
            .collect::<Result<Vec<_>>>()?;
        chords.push(ProgressionChord {
            degree,
            name,
            root_pitch_class,
            notes: voiced.iter().map(|n| n.name()).collect(),
            midi,
        });
        previous = Some(voiced);
    }
    Ok(Progression {
        root: tonic.name(),
        scale: scale.name.clone(),
        chords,
    })
}

fn chord_name(notes: &[Note], definitions: &[ChordDefinition]) -> String {
    let root = notes[0].pitch_class();
    let mut intervals: Vec<_> = notes
        .iter()
        .map(|n| (n.pitch_class() + 12 - root) % 12)
        .collect();
    intervals.sort_unstable();
    intervals.dedup();
    for chord in definitions {
        let mut candidate: Vec<_> = chord
            .intervals
            .iter()
            .map(|n| n.rem_euclid(12) as u8)
            .collect();
        candidate.sort_unstable();
        candidate.dedup();
        if intervals == candidate {
            return format!("{}{}", notes[0].name(), chord.suffix);
        }
    }
    // Some exotic seven-note scales produce clusters absent from the fixed
    // chord vocabulary. Preserve their actual pitches rather than mislabel them.
    notes.iter().map(|n| n.name()).collect::<Vec<_>>().join(" ")
}

fn with_midi(template: Note, midi: i16) -> Note {
    let octave = (midi - LETTER_PCS[template.letter] - template.accidental).div_euclid(12) - 1;
    template.with_octave(octave)
}

fn stack(tones: &[Note], octave: i16) -> Result<Vec<Note>> {
    let mut previous: Option<i16> = None;
    let mut voiced = Vec::with_capacity(tones.len());
    for &tone in tones {
        let midi = if let Some(previous) = previous {
            let distance = (i16::from(tone.pitch_class()) - previous).rem_euclid(12);
            previous + if distance == 0 { 12 } else { distance }
        } else {
            tone.with_octave(octave).midi().expect("octave supplied")
        };
        ensure!(
            (0..=127).contains(&midi),
            "Voicing falls outside MIDI 0–127"
        );
        voiced.push(with_midi(tone, midi));
        previous = Some(midi);
    }
    Ok(voiced)
}

fn smooth_voicing(tones: &[Note], octave: i16, previous: &[Note]) -> Result<Vec<Note>> {
    let previous: Vec<_> = previous
        .iter()
        .map(|n| n.midi().expect("concrete voicing"))
        .collect();
    let mut best: Option<(i32, i32, i16, usize)> = None;
    let mut chosen = None;
    for shift in [-1_i16, 0, 1] {
        for rotation in 0..tones.len() {
            let rotated: Vec<_> = tones[rotation..]
                .iter()
                .chain(&tones[..rotation])
                .copied()
                .collect();
            let Ok(candidate) = stack(&rotated, octave + shift) else {
                continue;
            };
            let midi: Vec<_> = candidate
                .iter()
                .map(|n| n.midi().expect("concrete voicing"))
                .collect();
            let cost = previous
                .iter()
                .zip(&midi)
                .map(|(a, b)| i32::from((a - b).abs()))
                .sum();
            let held = previous.iter().filter(|p| midi.contains(p)).count() as i32;
            let key = (cost, -held, shift.abs(), rotation);
            if best.is_none_or(|old| key < old) {
                best = Some(key);
                chosen = Some(candidate);
            }
        }
    }
    chosen.ok_or_else(|| anyhow!("No voicing fits the MIDI range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_minor_and_sevenths_keep_scale_spelling_and_harmonic_roots() {
        let p = progression("C4", "major", &[1, 5, 6, 4], false, false).unwrap();
        assert_eq!(
            p.chords.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["C", "G", "Am", "F"]
        );
        assert_eq!(p.chords[0].midi, [60, 64, 67]);
        assert_eq!(p.chords[1].midi, [67, 71, 74]);
        assert_eq!(p.chords[2].midi, [69, 72, 76]);
        let minor = progression("D4", "natural minor", &[1, 6, 7], true, false).unwrap();
        assert_eq!(minor.chords[0].notes, ["D4", "F4", "A4", "C5"]);
        assert_eq!(minor.chords[1].name, "Bbmaj7");
        assert_eq!(minor.chords[1].root_pitch_class, 10);
    }

    #[test]
    fn smoothing_preserves_spelling_and_chooses_nearby_inversions() {
        let p = progression("C4", "major", &[1, 5, 6, 4], false, true).unwrap();
        assert_eq!(
            p.chords.iter().map(|c| c.midi.clone()).collect::<Vec<_>>(),
            [
                vec![60, 64, 67],
                vec![59, 62, 67],
                vec![60, 64, 69],
                vec![60, 65, 69],
            ]
        );
        assert_eq!(p.chords[1].notes, ["B3", "D4", "G4"]);
        assert_eq!(p.chords[1].root_pitch_class, 7);
        let flat = progression("Gb4", "major", &[1, 4, 5], false, true).unwrap();
        assert!(
            flat.chords
                .iter()
                .flat_map(|c| &c.notes)
                .any(|n| n.starts_with("Cb"))
        );
    }

    #[test]
    fn bounds_and_unsupported_grammar_fail_explicitly() {
        assert!(progression("C", "major", &[1], false, true).is_err());
        assert!(progression("C4", "major", &[], false, true).is_err());
        assert!(progression("C4", "major", &[0], false, true).is_err());
        assert!(progression("C4", "major", &[8], false, true).is_err());
        assert!(progression("C4", "pentatonic", &[1], false, true).is_err());
        assert!(progression("C4", "major", &[1; 33], false, true).is_err());
        assert!(progression("C9", "major", &[7], true, false).is_err());
    }
}
