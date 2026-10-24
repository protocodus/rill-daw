//! Shared Rill design system: tokens, theme, and accessible interactive controls.
//!
//! This crate uses egui and serde; audio, application state, and eframe windowing
//! stay outside it. Components return native egui responses/builders so callers
//! retain keyboard focus, tooltips, disabled state, and layout composition.

pub mod clip;
pub mod crossfade;
pub mod curve;
pub mod form;
pub mod icons;
pub mod layout;
pub mod modal;
pub mod notice;
pub mod piano;
pub mod surface;
pub mod text;
pub mod theme;
pub mod timeline;
pub mod tokens;
pub mod waveform;
pub mod widgets;

pub use clip::*;
pub use crossfade::*;
pub use curve::*;
pub use form::*;
pub use icons::*;
pub use layout::*;
pub use modal::*;
pub use notice::*;
pub use piano::*;
pub use surface::*;
pub use timeline::*;
pub use waveform::*;
pub use widgets::*;
