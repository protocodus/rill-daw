//! Pitch-class chord vocabulary shared by MIDI drawing and highlighting.
use crate::Scale;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Chord {
    #[default]
    Major,
    Minor,
    Diminished,
    Augmented,
    SuspendedSecond,
    SuspendedFourth,
    DominantSeventh,
    MajorSeventh,
    MinorSeventh,
    HalfDiminished,
    DiminishedSeventh,
    MajorSixth,
    MinorSixth,
    AddNine,
    PowerChord,
    MinorMajorSeventh,
    AugmentedSeventh,
    AugmentedMajorSeventh,
    DominantSeventhFlatFifth,
    DominantSeventhSuspendedFourth,
    DominantSeventhSuspendedSecond,
    MinorAddNine,
    AddFour,
    SixNine,
    MinorSixNine,
    DominantNinth,
    MajorNinth,
    MinorNinth,
    DominantSeventhFlatNinth,
    DominantSeventhSharpNinth,
    DominantSeventhSharpEleventh,
    MajorSeventhSharpEleventh,
    DominantEleventh,
    MinorEleventh,
    DominantThirteenth,
    MajorThirteenth,
    MinorThirteenth,
}

impl Chord {
    pub const ALL: [Self; 37] = [
        Self::Major,
        Self::Minor,
        Self::Diminished,
        Self::Augmented,
        Self::SuspendedSecond,
        Self::SuspendedFourth,
        Self::DominantSeventh,
        Self::MajorSeventh,
        Self::MinorSeventh,
        Self::HalfDiminished,
        Self::DiminishedSeventh,
        Self::MajorSixth,
        Self::MinorSixth,
        Self::AddNine,
        Self::PowerChord,
        Self::MinorMajorSeventh,
        Self::AugmentedSeventh,
        Self::AugmentedMajorSeventh,
        Self::DominantSeventhFlatFifth,
        Self::DominantSeventhSuspendedFourth,
        Self::DominantSeventhSuspendedSecond,
        Self::MinorAddNine,
        Self::AddFour,
        Self::SixNine,
        Self::MinorSixNine,
        Self::DominantNinth,
        Self::MajorNinth,
        Self::MinorNinth,
        Self::DominantSeventhFlatNinth,
        Self::DominantSeventhSharpNinth,
        Self::DominantSeventhSharpEleventh,
        Self::MajorSeventhSharpEleventh,
        Self::DominantEleventh,
        Self::MinorEleventh,
        Self::DominantThirteenth,
        Self::MajorThirteenth,
        Self::MinorThirteenth,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Major => "Major triad",
            Self::Minor => "Minor triad",
            Self::Diminished => "Diminished",
            Self::Augmented => "Augmented",
            Self::SuspendedSecond => "Sus2",
            Self::SuspendedFourth => "Sus4",
            Self::DominantSeventh => "Dominant 7",
            Self::MajorSeventh => "Major 7",
            Self::MinorSeventh => "Minor 7",
            Self::HalfDiminished => "Half diminished",
            Self::DiminishedSeventh => "Diminished 7",
            Self::MajorSixth => "Major 6",
            Self::MinorSixth => "Minor 6",
            Self::AddNine => "Add9",
            Self::PowerChord => "Power chord",
            Self::MinorMajorSeventh => "Minor major 7",
            Self::AugmentedSeventh => "Augmented 7",
            Self::AugmentedMajorSeventh => "Augmented major 7",
            Self::DominantSeventhFlatFifth => "Dominant 7 flat 5",
            Self::DominantSeventhSuspendedFourth => "Dominant 7 sus4",
            Self::DominantSeventhSuspendedSecond => "Dominant 7 sus2",
            Self::MinorAddNine => "Minor add9",
            Self::AddFour => "Add4",
            Self::SixNine => "6/9",
            Self::MinorSixNine => "Minor 6/9",
            Self::DominantNinth => "Dominant 9",
            Self::MajorNinth => "Major 9",
            Self::MinorNinth => "Minor 9",
            Self::DominantSeventhFlatNinth => "Dominant 7 flat 9",
            Self::DominantSeventhSharpNinth => "Dominant 7 sharp 9",
            Self::DominantSeventhSharpEleventh => "Dominant 7 sharp 11",
            Self::MajorSeventhSharpEleventh => "Major 7 sharp 11",
            Self::DominantEleventh => "Dominant 11",
            Self::MinorEleventh => "Minor 11",
            Self::DominantThirteenth => "Dominant 13",
            Self::MajorThirteenth => "Major 13",
            Self::MinorThirteenth => "Minor 13",
        }
    }

    /// The canonical, compound-interval definition used by theory tools.
    pub fn definition(self) -> &'static crate::music_theory::ChordDefinition {
        crate::music_theory::chord_definition(self.definition_index())
    }

    fn definition_index(self) -> usize {
        match self {
            Self::Major => 0,
            Self::Minor => 1,
            Self::Diminished => 2,
            Self::Augmented => 3,
            Self::SuspendedSecond => 5,
            Self::SuspendedFourth => 6,
            Self::DominantSeventh => 9,
            Self::MajorSeventh => 10,
            Self::MinorSeventh => 11,
            Self::HalfDiminished => 14,
            Self::DiminishedSeventh => 13,
            Self::MajorSixth => 7,
            Self::MinorSixth => 8,
            Self::AddNine => 20,
            Self::PowerChord => 4,
            Self::MinorMajorSeventh => 12,
            Self::AugmentedSeventh => 15,
            Self::AugmentedMajorSeventh => 16,
            Self::DominantSeventhFlatFifth => 17,
            Self::DominantSeventhSuspendedFourth => 18,
            Self::DominantSeventhSuspendedSecond => 19,
            Self::MinorAddNine => 21,
            Self::AddFour => 22,
            Self::SixNine => 23,
            Self::MinorSixNine => 24,
            Self::DominantNinth => 25,
            Self::MajorNinth => 26,
            Self::MinorNinth => 27,
            Self::DominantSeventhFlatNinth => 28,
            Self::DominantSeventhSharpNinth => 29,
            Self::DominantSeventhSharpEleventh => 30,
            Self::MajorSeventhSharpEleventh => 31,
            Self::DominantEleventh => 32,
            Self::MinorEleventh => 33,
            Self::DominantThirteenth => 34,
            Self::MajorThirteenth => 35,
            Self::MinorThirteenth => 36,
        }
    }

    /// Resolve canonical names, aliases, or legacy UI identifiers in the shared catalog.
    pub fn from_name(name: &str) -> anyhow::Result<Self> {
        let definition = crate::music_theory::find_chord(name)?;
        Self::ALL
            .into_iter()
            .find(|value| std::ptr::eq(value.definition(), definition))
            .ok_or_else(|| anyhow::anyhow!("Chord definition has no UI binding"))
    }

    /// Unique pitch classes for drawing and highlighting within one octave.
    pub fn intervals(self) -> &'static [u8] {
        crate::music_theory::chord_pitch_classes(self.definition_index())
    }

    pub fn contains(self, root: u8, pitch: u8) -> bool {
        self.intervals()
            .contains(&((pitch % 12 + 12 - root % 12) % 12))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoteStack {
    #[default]
    Single,
    Chord,
    Scale,
}

impl NoteStack {
    pub fn name(self) -> &'static str {
        match self {
            Self::Single => "Single note",
            Self::Chord => "Chord stack",
            Self::Scale => "Scale stack",
        }
    }

    pub fn pitches(self, pitch: u8, root: u8, chord: Chord, scale: Scale) -> Vec<u8> {
        if self == Self::Single {
            return vec![pitch.min(127)];
        }
        let octave = u16::from(pitch.min(127) / 12) * 12;
        (octave..(octave + 12).min(128))
            .map(|p| p as u8)
            .filter(|p| match self {
                Self::Chord => chord.contains(root, *p),
                Self::Scale => scale.contains(root, *p),
                Self::Single => false,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scale_and_chord_has_one_shared_definition_and_alias_binding() {
        use std::collections::HashSet;

        assert_eq!(Chord::ALL.len(), 37);
        assert_eq!(Scale::ALL.len(), 50);
        let chords: HashSet<_> = Chord::ALL
            .iter()
            .map(|value| value.definition().name.as_str())
            .collect();
        let scales: HashSet<_> = Scale::ALL
            .iter()
            .map(|value| value.definition().name.as_str())
            .collect();
        assert_eq!(chords.len(), 37);
        assert_eq!(scales.len(), 50);
        for index in 0..37 {
            assert!(chords.contains(crate::music_theory::chord_definition(index).name.as_str()));
        }
        for index in 0..50 {
            assert!(scales.contains(crate::music_theory::scale_definition(index).name.as_str()));
        }
        for value in Chord::ALL {
            assert_eq!(Chord::from_name(&format!("{value:?}")).unwrap(), value);
            assert_eq!(Chord::from_name(value.name()).unwrap(), value);
            assert_eq!(Chord::from_name(&value.definition().name).unwrap(), value);
        }
        for value in Scale::ALL {
            assert_eq!(Scale::from_name(&format!("{value:?}")).unwrap(), value);
            assert_eq!(Scale::from_name(value.name()).unwrap(), value);
            assert_eq!(Scale::from_name(&value.definition().name).unwrap(), value);
            assert_eq!(
                serde_json::from_value::<Scale>(serde_json::to_value(value).unwrap()).unwrap(),
                value
            );
        }
    }

    #[test]
    fn legacy_identifiers_and_case_sensitive_chord_suffixes_still_resolve() {
        for (json, expected) in [
            ("\"Chromatic\"", Scale::Chromatic),
            ("\"Major\"", Scale::Major),
            ("\"Minor\"", Scale::Minor),
            ("\"Dorian\"", Scale::Dorian),
            ("\"Pentatonic\"", Scale::Pentatonic),
        ] {
            assert_eq!(serde_json::from_str::<Scale>(json).unwrap(), expected);
            assert_eq!(serde_json::to_string(&expected).unwrap(), json);
        }
        assert_eq!(Chord::from_name("M").unwrap(), Chord::Major);
        assert_eq!(Chord::from_name("m").unwrap(), Chord::Minor);
        assert_eq!(Chord::from_name("M7").unwrap(), Chord::MajorSeventh);
        assert_eq!(Chord::from_name("m7").unwrap(), Chord::MinorSeventh);
        assert_eq!(Scale::from_name("aeolian").unwrap(), Scale::Minor);
        assert_eq!(Scale::from_name("pentatonic").unwrap(), Scale::Pentatonic);
        assert_eq!(Chord::ALL[13], Chord::AddNine);
        assert_eq!(
            Scale::ALL[..5],
            [
                Scale::Chromatic,
                Scale::Major,
                Scale::Minor,
                Scale::Dorian,
                Scale::Pentatonic
            ]
        );
        assert!(Chord::from_name("invented harmony").is_err());
        assert!(Scale::from_name("invented mode").is_err());
    }

    #[test]
    fn compound_intervals_and_folded_drawing_share_one_definition() {
        assert_eq!(Chord::AddNine.definition().intervals, [0, 4, 7, 14]);
        assert_eq!(Chord::AddNine.intervals(), [0, 2, 4, 7]);
        for value in Chord::ALL {
            let mut expected: Vec<_> = value
                .definition()
                .intervals
                .iter()
                .map(|interval| interval.rem_euclid(12) as u8)
                .collect();
            expected.sort_unstable();
            expected.dedup();
            assert_eq!(value.intervals(), expected);
        }
        for value in Scale::ALL {
            let mut expected: Vec<_> = value
                .definition()
                .intervals
                .iter()
                .map(|interval| interval.rem_euclid(12) as u8)
                .collect();
            expected.sort_unstable();
            expected.dedup();
            assert_eq!(value.intervals(), expected);
            assert_eq!(value.intervals().first(), Some(&0));
            assert!(
                value
                    .intervals()
                    .windows(2)
                    .all(|pair| pair[0] < pair[1] && pair[1] < 12)
            );
        }
    }

    #[test]
    fn all_chords_have_unique_valid_intervals_and_transpose_without_overflow() {
        for chord in Chord::ALL {
            assert_eq!(chord.intervals()[0], 0);
            assert!(
                chord
                    .intervals()
                    .windows(2)
                    .all(|p| p[0] < p[1] && p[1] < 12)
            );
            for root in 0..12 {
                assert!(chord.contains(root, root));
            }
        }
    }
    #[test]
    fn stacks_stay_in_the_clicked_octave_and_respect_midi_bounds() {
        assert_eq!(
            NoteStack::Chord.pitches(62, 0, Chord::Major, Scale::Major),
            vec![60, 64, 67]
        );
        assert_eq!(
            NoteStack::Scale.pitches(62, 0, Chord::Major, Scale::Major),
            vec![60, 62, 64, 65, 67, 69, 71]
        );
        assert_eq!(
            NoteStack::Chord.pitches(127, 11, Chord::Major, Scale::Major),
            vec![123, 126]
        );
        assert_eq!(
            NoteStack::Single.pitches(255, 0, Chord::Major, Scale::Major),
            vec![127]
        );
    }
}
