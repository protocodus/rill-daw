//! Shared Rill design system: tokens, theme, and accessible interactive controls.
//!
//! This crate uses egui and serde; audio, application state, and eframe windowing
//! stay outside it. Components return native egui responses/builders so callers
//! retain keyboard focus, tooltips, disabled state, and layout composition.

pub mod curve;
pub mod icons;
pub mod piano;
pub mod text;
pub mod theme;
pub mod timeline;
pub mod tokens;
pub mod waveform;
pub mod widgets;

pub use curve::*;
pub use icons::*;
pub use piano::*;
pub use timeline::*;
pub use waveform::*;
pub use widgets::*;
