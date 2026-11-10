//! Rill's built-in R1 instruments and effects.
//!
//! This crate knows nothing about the project model, the engine, the UI or the
//! file system. The DAW reaches a device only through [`Device`], its
//! [`DeviceDescriptor`] and the [`registry`]. See `docs/R1_DEVICES.md`.
#![forbid(unsafe_code)]

mod device;
pub mod devices;
mod param;
mod preset;
mod registry;
mod telemetry;

pub use daw_dsp as dsp;
pub use device::*;
pub use param::*;
pub use preset::*;
pub use registry::*;
pub use telemetry::*;
