//! Shared Rill design system: tokens, theme, and accessible interactive controls.
//!
//! Audio, application state, and eframe windowing stay outside this crate.
//! Components return native egui responses/builders so callers
//! retain keyboard focus, tooltips, disabled state, and layout composition.

pub mod brand;
pub mod channel_controls;
pub mod clip;
pub mod color;
pub mod crossfade;
pub mod curve;
pub mod device_cards;
pub mod error_details;
pub mod form;
pub mod generation;
pub mod icons;
pub mod layout;
pub mod modal;
pub mod native_context_menu;
pub mod native_dialog;
pub mod navigation;
pub mod note_length;
pub mod notice;
pub mod parameter_browser;
pub mod piano;
pub mod popup_backdrop;
pub mod recording;
pub mod spectrogram;
pub mod surface;
pub mod take_lane;
pub mod text;
pub mod theme;
pub mod timeline;
pub mod tokens;
pub mod waveform;
pub mod widgets;

pub use brand::*;
pub use channel_controls::*;
pub use clip::*;
pub use color::*;
pub use crossfade::*;
pub use curve::*;
pub use device_cards::*;
pub use error_details::*;
pub use form::*;
pub use generation::*;
pub use icons::*;
pub use layout::*;
pub use modal::*;
pub use native_context_menu::*;
pub use native_dialog::*;
pub use navigation::*;
pub use note_length::*;
pub use notice::*;
pub use parameter_browser::*;
pub use piano::*;
pub use popup_backdrop::*;
pub use recording::*;
pub use spectrogram::*;
pub use surface::*;
pub use take_lane::*;
pub use timeline::*;
pub use waveform::*;
pub use widgets::*;
