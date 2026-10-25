//! Persistent musical data and non-destructive editing, independent of the UI
//! and audio device. All arrangement and source positions are quarter-note beats.

mod archive;
mod audio;
mod audio_edit;
mod audio_overlap;
mod audio_processing;
mod automation;
mod chords;
mod clip_links;
mod demo;
mod history;
mod join;
mod midi;
mod midi_edit;
mod model;
mod routing;
mod track_colors;

pub use audio::AudioData;
pub use audio_edit::*;
pub use audio_overlap::*;
pub use audio_processing::*;
pub use automation::*;
pub use chords::*;
pub use history::*;
pub use join::AudioJoinSelection;
pub use midi::{MidiImport, export_midi, import_midi};
pub use midi_edit::*;
pub use model::*;
