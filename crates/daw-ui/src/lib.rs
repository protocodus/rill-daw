//! Shared Rill design system: tokens, theme, and accessible interactive controls.
//!
//! Audio, application state, and eframe windowing stay outside this crate.
//! Components return native egui responses/builders so callers
//! retain keyboard focus, tooltips, disabled state, and layout composition.

pub mod channel_controls;
pub mod clip;
pub mod color;
pub mod crossfade;
pub mod curve;
pub mod form;
pub mod icons;
pub mod layout;
pub mod modal;
pub mod native_dialog;
pub mod navigation;
pub mod notice;
pub mod piano;
pub mod popup_backdrop;
pub mod surface;
pub mod text;
pub mod theme;
pub mod timeline;
pub mod tokens;
pub mod waveform;
pub mod widgets;

pub use channel_controls::*;
pub use clip::*;
pub use color::*;
pub use crossfade::*;
pub use curve::*;
pub use form::*;
pub use icons::*;
pub use layout::*;
pub use modal::*;
pub use native_dialog::*;
pub use navigation::*;
pub use notice::*;
pub use piano::*;
pub use popup_backdrop::*;
pub use surface::*;
pub use timeline::*;
pub use waveform::*;
pub use widgets::*;
