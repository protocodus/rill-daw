//! Upstream's fixed scale/chord vocabulary and octave-aware realization.
use crate::notes::{Note, degree_steps, parse_note, transpose};
use crate::{Catalog, ChordDefinition, NoteSet, ScaleDefinition};
use anyhow::{Result, bail, ensure};
use std::sync::OnceLock;

fn stored() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("catalog-data.json"))
            .expect("compiled composition catalog")
    })
}

pub fn catalog() -> Catalog {
    stored().clone()
}

fn normalize_scale(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn find_scale(name: &str) -> Result<&'static ScaleDefinition> {
    ensure!(name.len() <= 128, "Scale type name is too long");
    let normalized = normalize_scale(name);
    stored()
        .scales
        .iter()
        .find(|scale| {
            normalize_scale(&scale.name) == normalized
                || scale
                    .aliases
                    .iter()
                    .any(|alias| normalize_scale(alias) == normalized)
        })
        .ok_or_else(|| {
            anyhow::anyhow!("Unknown scale type; choose a name or alias from the catalog")
        })
}

fn chord_keys(chord: &ChordDefinition) -> impl Iterator<Item = &str> {
    std::iter::once(chord.suffix.as_str())
        .chain(std::iter::once(chord.name.as_str()))
        .chain(chord.aliases.iter().map(String::as_str))
}

fn exact_chord(text: &str) -> Option<&'static ChordDefinition> {
    stored()
        .chords
        .iter()
        .find(|chord| chord_keys(chord).any(|key| key == text))
}

fn normalize_chord(text: &str) -> String {
    let compact: Vec<_> = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    compact
        .iter()
        .enumerate()
        .filter_map(|(i, &ch)| {
            if ch == '-'
                && i > 0
                && compact[i - 1].is_ascii_alphabetic()
                && compact.get(i + 1).is_some_and(char::is_ascii_alphanumeric)
            {
                None
            } else {
                Some(ch)
            }
        })
        .collect()
}

fn translated_suffix(text: &str) -> String {
    let text = text
        .strip_prefix('-')
        .map_or_else(|| text.to_owned(), |tail| format!("m{tail}"));
    let chars: Vec<_> = text.chars().collect();
    let mut converted = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        let between_digits = i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        converted.push(if between_digits && ch == '-' {
            'b'
        } else if between_digits && ch == '+' {
            '#'
        } else {
            ch
        });
    }
    if let Some(tail) = converted.strip_prefix('M')
        && (tail.starts_with(|c: char| c.is_ascii_digit()) || tail.starts_with("add"))
    {
        return format!("maj{tail}");
    }
    if let Some(digits) = converted.strip_suffix('M')
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
    {
        return format!("maj{digits}");
    }
    converted
}

fn is_minor(chord: &ChordDefinition) -> bool {
    chord.suffix.starts_with('m') && !chord.suffix.starts_with("maj")
}

pub(crate) fn find_chord(name: &str) -> Result<&'static ChordDefinition> {
    ensure!(name.len() <= 128, "Chord type name is too long");
    let text = name.trim();
    if let Some(chord) = exact_chord(text) {
        return Ok(chord);
    }
    let translated = translated_suffix(text);
    let normalized = normalize_chord(&translated);
    let mut matches = stored()
        .chords
        .iter()
        .filter(|chord| chord_keys(chord).any(|key| normalize_chord(key) == normalized));
    let found = exact_chord(&translated).or_else(|| {
        // A folded alias must identify exactly one chord, especially m/M.
        if normalized == "mm7" {
            return None;
        }
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    });
    if let Some(chord) = found {
        let transformed = text
            .strip_prefix('-')
            .map_or_else(|| text.to_owned(), |tail| format!("m{tail}"));
        let uppercase_major = transformed.starts_with('M')
            && !["Maj", "MAJ", "Min", "MIN"]
                .iter()
                .any(|s| transformed.starts_with(s));
        let lowercase_minor = transformed.starts_with('m')
            && !["maj", "ma7", "major"]
                .iter()
                .any(|s| transformed.starts_with(s));
        if !(text.starts_with('-') && !is_minor(chord)
            || uppercase_major && is_minor(chord)
            || lowercase_minor && !is_minor(chord))
        {
            return Ok(chord);
        }
    }
    bail!("Unknown or ambiguous chord type; choose a name, suffix or alias from the catalog")
}

fn note_set(root: Note, name: &str, notes: Vec<Note>) -> Result<NoteSet> {
    let midi = if root.octave.is_some() {
        Some(
            notes
                .iter()
                .map(|note| {
                    let value = note.midi().expect("concrete root yields concrete notes");
                    ensure!(
                        (0..=127).contains(&value),
                        "Generated note falls outside MIDI 0–127"
                    );
                    Ok(value as u8)
                })
                .collect::<Result<Vec<_>>>()?,
        )
    } else {
        None
    };
    Ok(NoteSet {
        name: name.to_owned(),
        root: root.name(),
        notes: notes.into_iter().map(Note::name).collect(),
        midi,
    })
}

pub fn get_scale(root: &str, scale: &str) -> Result<NoteSet> {
    let scale = find_scale(scale)?;
    let root = parse_note(root)?;
    let mut notes = scale
        .intervals
        .iter()
        .zip(&scale.degrees)
        .map(|(&semitones, label)| transpose(root, semitones, Some(degree_steps(label)?)))
        .collect::<Result<Vec<_>>>()?;
    notes.push(transpose(root, 12, Some(7))?);
    note_set(root, &scale.name, notes)
}

pub fn get_chord(root: &str, chord: &str) -> Result<NoteSet> {
    let chord = find_chord(chord)?;
    let root = parse_note(root)?;
    let notes = chord
        .intervals
        .iter()
        .zip(&chord.degrees)
        .map(|(&semitones, label)| transpose(root, semitones, Some(degree_steps(label)?)))
        .collect::<Result<Vec<_>>>()?;
    note_set(root, &chord.name, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_is_complete_and_all_names_and_aliases_resolve() {
        let catalog = catalog();
        assert_eq!(catalog.scales.len(), 50);
        assert_eq!(catalog.chords.len(), 37);
        for scale in &catalog.scales {
            assert!(!scale.description.is_empty());
            for key in std::iter::once(&scale.name).chain(&scale.aliases) {
                assert_eq!(get_scale("C4", key).unwrap().name, scale.name);
            }
        }
        for chord in &catalog.chords {
            assert!(!chord.description.is_empty());
            for key in std::iter::once(&chord.name)
                .chain(std::iter::once(&chord.suffix))
                .chain(&chord.aliases)
            {
                assert_eq!(get_chord("C4", key).unwrap().name, chord.name, "{key}");
            }
        }
    }

    #[test]
    fn spelling_and_extensions_match_the_source_vocabulary() {
        assert_eq!(
            get_scale("F", "maj").unwrap().notes,
            ["F", "G", "A", "Bb", "C", "D", "E", "F"]
        );
        assert_eq!(
            get_chord("C4", "dim7").unwrap().notes,
            ["C4", "Eb4", "Gb4", "Bbb4"]
        );
        assert_eq!(
            get_chord("C4", "9").unwrap().midi,
            Some(vec![60, 64, 67, 70, 74])
        );
        assert_eq!(
            get_scale("C4", "major").unwrap().midi,
            Some(vec![60, 62, 64, 65, 67, 69, 71, 72])
        );
        assert!(get_scale("G9", "major").is_err());
        assert!(get_chord("G9", "major").is_err());
        assert!(get_scale("C", "unknown").is_err());
        assert!(get_chord("C", "unknown").is_err());
    }

    #[test]
    fn chord_aliases_do_not_guess_across_major_and_minor() {
        assert_eq!(
            get_chord("C4", "M7").unwrap().midi,
            Some(vec![60, 64, 67, 71])
        );
        assert_eq!(
            get_chord("C4", "m7").unwrap().midi,
            Some(vec![60, 63, 67, 70])
        );
        assert_eq!(get_chord("C4", "-9").unwrap().name, "minor 9");
        assert_eq!(get_chord("C4", "7-9").unwrap().name, "dominant 7 flat 9");
        assert_eq!(get_chord("C4", "MAJOR_7").unwrap().name, "major 7");
        assert!(get_chord("C4", "mm7").is_err());
        assert!(get_chord("C4", "M13b5").is_err());
    }
}
