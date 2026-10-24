//! Persistent musical data and non-destructive editing, independent of the UI
//! and audio device. All arrangement and source positions are quarter-note beats.

mod audio;
mod automation;
mod demo;
mod midi;
mod model;
mod routing;

pub use audio::AudioData;
pub use automation::*;
pub use midi::{MidiImport, export_midi, import_midi};
pub use model::*;
