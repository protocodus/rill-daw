//! Shared Rill design system: tokens, theme, and accessible interactive controls.
//!
//! This crate depends only on egui; audio, application state, and eframe windowing
//! stay outside it. Components return native egui responses/builders so callers
//! retain keyboard focus, tooltips, disabled state, and layout composition.

pub mod curve;
pub mod theme;
pub mod tokens;
pub mod widgets;

pub use curve::*;
pub use widgets::*;
