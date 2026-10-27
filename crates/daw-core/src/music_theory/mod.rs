//! Canonical harmony vocabulary and octave-aware spelling for Rill's piano roll.
//!
//! The editor's scale/chord choices, grid highlighting, multi-draw and generation
//! use these same definitions as composition tools. A chord's compound intervals
//! describe voiced extensions; its derived pitch classes describe membership in
//! one octave. The catalogue has one physical data source in this module.
mod catalog;
pub mod notes;

pub use catalog::{catalog, find_chord, find_scale, get_chord, get_scale};
pub(crate) use catalog::{
    chord_definition, chord_pitch_classes, scale_definition, scale_pitch_classes,
};
pub use notes::Note as SpelledNote;

use serde::{Deserialize, Serialize};

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
