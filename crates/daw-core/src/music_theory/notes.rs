//! Octave-aware pitch spelling, adapted from upstream notes.py.
use anyhow::{Result, bail, ensure};

pub const LETTER_PCS: [i16; 7] = [0, 2, 4, 5, 7, 9, 11];
const LETTERS: [char; 7] = ['C', 'D', 'E', 'F', 'G', 'A', 'B'];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    pub letter: usize,
    pub accidental: i16,
    pub octave: Option<i16>,
}

impl Note {
    pub fn name(self) -> String {
        let mut name = LETTERS[self.letter].to_string();
        name.push_str(
            &if self.accidental < 0 { "b" } else { "#" }
                .repeat(usize::from(self.accidental.unsigned_abs())),
        );
        if let Some(octave) = self.octave {
            name.push_str(&octave.to_string());
        }
        name
    }

    pub fn pitch_class(self) -> u8 {
        (LETTER_PCS[self.letter] + self.accidental).rem_euclid(12) as u8
    }

    pub fn midi(self) -> Option<i16> {
        self.octave
            .map(|octave| LETTER_PCS[self.letter] + self.accidental + (octave + 1) * 12)
    }

    pub fn without_octave(self) -> Self {
        Self {
            octave: None,
            ..self
        }
    }

    pub fn with_octave(self, octave: i16) -> Self {
        Self {
            octave: Some(octave),
            ..self
        }
    }
}

pub fn parse_note(text: &str) -> Result<Note> {
    ensure!(text.len() <= 32, "Note name is too long");
    let text = text.trim();
    let mut chars = text.chars();
    let first = chars
        .next()
        .ok_or_else(|| anyhow::anyhow!("Note name is empty"))?;
    let letter = LETTERS
        .iter()
        .position(|&n| n == first.to_ascii_uppercase())
        .ok_or_else(|| anyhow::anyhow!("Note must begin with a letter A–G"))?;
    let mut accidental = 0_i16;
    let mut has_sharp = false;
    let mut has_flat = false;
    let mut consumed = first.len_utf8();
    for ch in chars {
        match ch {
            '#' | '♯' => {
                accidental += 1;
                has_sharp = true;
            }
            'b' | '♭' => {
                accidental -= 1;
                has_flat = true;
            }
            _ => break,
        }
        consumed += ch.len_utf8();
    }
    ensure!(
        !(has_sharp && has_flat),
        "Mixed sharps and flats are not a note spelling"
    );
    ensure!(
        (-2..=2).contains(&accidental),
        "Notes allow at most a double sharp or flat"
    );
    let remainder = &text[consumed..];
    let octave = if remainder.is_empty() {
        None
    } else {
        let digits = remainder.strip_prefix('-').unwrap_or(remainder);
        ensure!(
            !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()),
            "Invalid note octave"
        );
        let octave = remainder
            .parse::<i16>()
            .map_err(|_| anyhow::anyhow!("Invalid note octave"))?;
        // Bound before MIDI arithmetic; theoretical out-of-range concrete pitches
        // do not silently become pitch classes or wrap into the playable range.
        ensure!((-2..=10).contains(&octave), "Note falls outside MIDI 0–127");
        Some(octave)
    };
    let note = Note {
        letter,
        accidental,
        octave,
    };
    if let Some(midi) = note.midi() {
        ensure!((0..=127).contains(&midi), "Note falls outside MIDI 0–127");
    }
    Ok(note)
}

pub fn prefers_flats(root: Note) -> bool {
    root.accidental < 0 || (root.accidental == 0 && root.pitch_class() == 5)
}

pub fn note_from_midi(midi: u8, prefer_flats: bool) -> Note {
    const SHARP: [(usize, i16); 12] = [
        (0, 0),
        (0, 1),
        (1, 0),
        (1, 1),
        (2, 0),
        (3, 0),
        (3, 1),
        (4, 0),
        (4, 1),
        (5, 0),
        (5, 1),
        (6, 0),
    ];
    const FLAT: [(usize, i16); 12] = [
        (0, 0),
        (1, -1),
        (1, 0),
        (2, -1),
        (2, 0),
        (3, 0),
        (4, -1),
        (4, 0),
        (5, -1),
        (5, 0),
        (6, -1),
        (6, 0),
    ];
    let (letter, accidental) = if prefer_flats { FLAT } else { SHARP }[usize::from(midi % 12)];
    let octave = (i16::from(midi) - LETTER_PCS[letter] - accidental).div_euclid(12) - 1;
    Note {
        letter,
        accidental,
        octave: Some(octave),
    }
}

pub fn transpose(root: Note, semitones: i16, letter_steps: Option<i16>) -> Result<Note> {
    if let Some(letter_steps) = letter_steps {
        let index = i32::try_from(root.letter).expect("seven letters") + i32::from(letter_steps);
        let carry = index.div_euclid(7);
        let letter = index.rem_euclid(7) as usize;
        let target_offset =
            i32::from(LETTER_PCS[root.letter]) + i32::from(root.accidental) + i32::from(semitones);
        let accidental = target_offset - (i32::from(LETTER_PCS[letter]) + 12 * carry);
        if (-2..=2).contains(&accidental) {
            let octave = root.octave.map(|o| i32::from(o) + carry);
            if let Some(octave) = octave {
                let midi = i32::from(LETTER_PCS[letter]) + accidental + (octave + 1) * 12;
                ensure!(
                    (0..=127).contains(&midi),
                    "Transposed note falls outside MIDI 0–127"
                );
            }
            return Ok(Note {
                letter,
                accidental: accidental as i16,
                octave: octave.map(|o| o as i16),
            });
        }
    }
    let flats = prefers_flats(root);
    if let Some(midi) = root.midi() {
        let shifted = i32::from(midi) + i32::from(semitones);
        ensure!(
            (0..=127).contains(&shifted),
            "Transposed note falls outside MIDI 0–127"
        );
        return Ok(note_from_midi(shifted as u8, flats));
    }
    let pc = (i32::from(root.pitch_class()) + i32::from(semitones)).rem_euclid(12);
    Ok(note_from_midi(pc as u8, flats).without_octave())
}

pub fn degree_steps(label: &str) -> Result<i16> {
    let digits = label.trim_start_matches(['#', 'b']);
    let degree = digits
        .parse::<i16>()
        .map_err(|_| anyhow::anyhow!("Invalid scale degree label"))?;
    if !(1..=32).contains(&degree) {
        bail!("Unsupported scale degree label");
    }
    Ok(degree - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_preserves_letter_octaves_and_normalizes_unicode() {
        assert_eq!(parse_note("Cb4").unwrap().midi(), Some(59));
        assert_eq!(parse_note("B#3").unwrap().midi(), Some(60));
        assert_eq!(parse_note("f♯4").unwrap().name(), "F#4");
        assert_eq!(parse_note("E♭♭").unwrap().name(), "Ebb");
        assert_eq!(parse_note("C-1").unwrap().midi(), Some(0));
        assert_eq!(parse_note("G9").unwrap().midi(), Some(127));
        for text in [
            "",
            "H4",
            "C#b4",
            "C###4",
            "C-2",
            "G#9",
            "C+4",
            "C4junk",
            "C 4",
            "C99999999999",
        ] {
            assert!(parse_note(text).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn spelling_uses_diatonic_letters_and_safe_practical_fallback() {
        assert_eq!(
            transpose(parse_note("F4").unwrap(), 5, Some(3))
                .unwrap()
                .name(),
            "Bb4"
        );
        assert_eq!(
            transpose(parse_note("C4").unwrap(), 9, Some(6))
                .unwrap()
                .name(),
            "Bbb4"
        );
        assert_eq!(
            transpose(parse_note("G##4").unwrap(), 11, Some(6))
                .unwrap()
                .name(),
            "G#5"
        );
        assert!(transpose(parse_note("G9").unwrap(), 1, None).is_err());
        for midi in 0..=127 {
            for flats in [false, true] {
                assert_eq!(note_from_midi(midi, flats).midi(), Some(i16::from(midi)));
            }
        }
    }
}
