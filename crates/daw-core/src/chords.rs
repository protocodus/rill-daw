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
}

impl Chord {
    pub const ALL: [Self; 14] = [
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
        }
    }

    /// Unique ascending pitch-class intervals. Extended tones are folded into
    /// one octave deliberately: drawing a stack never spills into a second octave.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Self::Major => &[0, 4, 7],
            Self::Minor => &[0, 3, 7],
            Self::Diminished => &[0, 3, 6],
            Self::Augmented => &[0, 4, 8],
            Self::SuspendedSecond => &[0, 2, 7],
            Self::SuspendedFourth => &[0, 5, 7],
            Self::DominantSeventh => &[0, 4, 7, 10],
            Self::MajorSeventh => &[0, 4, 7, 11],
            Self::MinorSeventh => &[0, 3, 7, 10],
            Self::HalfDiminished => &[0, 3, 6, 10],
            Self::DiminishedSeventh => &[0, 3, 6, 9],
            Self::MajorSixth => &[0, 4, 7, 9],
            Self::MinorSixth => &[0, 3, 7, 9],
            Self::AddNine => &[0, 2, 4, 7],
        }
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
