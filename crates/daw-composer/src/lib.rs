//! Bounded, deterministic composition primitives adapted from midi-composer-mcp.
//!
//! This crate builds musical data only. It has no project, filesystem, provider,
//! plug-in, or audio-device access. See `UPSTREAM.md` for provenance and scope.
mod accompany;
mod catalog;
mod notes;
mod progression;
mod rhythm;

pub use accompany::compose;
pub use catalog::{catalog, get_chord, get_scale};
pub use progression::progression;
pub use rhythm::euclidean;

use serde::{Deserialize, Serialize};

pub const MAX_CHORDS: usize = 32;
pub const MAX_PARTS: usize = 8;
pub const MAX_NOTES: usize = 8192;
pub const MIN_BEATS_PER_CHORD: f64 = 0.25;
pub const MAX_BEATS_PER_CHORD: f64 = 64.0;
pub const MIN_STEP: f64 = 1.0 / 64.0;
pub const MAX_STEP: f64 = 64.0;
pub const MIN_BASS_OCTAVE: i8 = 0;
pub const MAX_BASS_OCTAVE: i8 = 8;
pub const MAX_DURATION: f64 = MAX_CHORDS as f64 * MAX_BEATS_PER_CHORD;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScaleDefinition {
    pub name: String,
    pub intervals: Vec<i16>,
    pub aliases: Vec<String>,
    pub degrees: Vec<String>,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChordDefinition {
    pub name: String,
    pub intervals: Vec<i16>,
    pub aliases: Vec<String>,
    pub degrees: Vec<String>,
    pub suffix: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Catalog {
    pub scales: Vec<ScaleDefinition>,
    pub chords: Vec<ChordDefinition>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NoteSet {
    pub name: String,
    pub root: String,
    pub notes: Vec<String>,
    /// Absent for abstract pitch classes; concrete pitches use C4 = MIDI 60.
    pub midi: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProgressionChord {
    pub degree: u8,
    pub name: String,
    /// Harmonic root remains explicit when the voiced bass is an inversion.
    pub root_pitch_class: u8,
    pub notes: Vec<String>,
    pub midi: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Progression {
    pub root: String,
    pub scale: String,
    pub chords: Vec<ProgressionChord>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArpeggioStyle {
    #[default]
    Up,
    Down,
    UpDown,
    Alberti,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BassStyle {
    #[default]
    Root,
    RootFifth,
    RootOctave,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PartSpec {
    Chords {
        #[serde(default = "chords_name")]
        name: String,
        #[serde(default = "velocity")]
        velocity: u8,
        #[serde(default = "gate")]
        gate: f64,
        #[serde(default)]
        octave_shift: i8,
    },
    Arpeggio {
        #[serde(default = "arpeggio_name")]
        name: String,
        #[serde(default)]
        style: ArpeggioStyle,
        #[serde(default = "half_beat")]
        step: f64,
        #[serde(default = "velocity")]
        velocity: u8,
        #[serde(default = "gate")]
        gate: f64,
        #[serde(default)]
        octave_shift: i8,
    },
    Bass {
        #[serde(default = "bass_name")]
        name: String,
        #[serde(default)]
        style: BassStyle,
        #[serde(default = "beat")]
        step: f64,
        #[serde(default = "velocity")]
        velocity: u8,
        #[serde(default = "gate")]
        gate: f64,
        #[serde(default = "bass_octave")]
        octave: i8,
    },
}

impl PartSpec {
    pub fn name(&self) -> &str {
        match self {
            Self::Chords { name, .. } | Self::Arpeggio { name, .. } | Self::Bass { name, .. } => {
                name
            }
        }
    }
}

fn chords_name() -> String {
    "Chords".into()
}
fn arpeggio_name() -> String {
    "Arpeggio".into()
}
fn bass_name() -> String {
    "Bass".into()
}
fn velocity() -> u8 {
    90
}
fn gate() -> f64 {
    0.9
}
fn half_beat() -> f64 {
    0.5
}
fn beat() -> f64 {
    1.0
}
fn bass_octave() -> i8 {
    2
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TimedNote {
    pub pitch: u8,
    pub start: f64,
    pub length: f64,
    pub velocity: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DraftPart {
    pub name: String,
    pub length: f64,
    pub notes: Vec<TimedNote>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rhythm {
    pub pattern: String,
    pub pulses: u16,
    pub steps: u16,
    pub rotation: i32,
}
