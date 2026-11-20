//! Task-oriented device layouts. The parameter definitions and interactions
//! stay with the device; these workspaces only decide what is shown together.
use super::*;

mod containers;
mod effects;
mod instruments;
mod tone;

pub(crate) struct Workspace {
    pub layout: Layout,
    /// Compact controls that stay below every task, before the keyboard.
    pub output: Vec<u32>,
    /// Compact global context, above the device's picture.
    pub toolbar: Vec<u32>,
    /// Reserved workspace height, excluding navigation, output and keyboard.
    pub body_height: f32,
}

pub(crate) fn tabs(descriptor: &DeviceDescriptor) -> &'static [&'static str] {
    match descriptor.id {
        "r1-mini" => &["Oscillators", "Filter & amp", "Modulation", "Voices"],
        "r1-sampler" => &["Sample", "Pitch & voices", "Filter & amp", "Modulation"],
        "r1-spacer" => &["Echo", "Reverb"],
        _ => &[],
    }
}

pub(crate) fn build(
    descriptor: &DeviceDescriptor,
    pad: usize,
    page: usize,
    band: usize,
) -> Workspace {
    let layout = super::layout_for(descriptor, pad);
    match descriptor.id {
        "r1-mini" | "r1-sampler" | "r1-drums" => instruments::build(layout, descriptor, pad, page),
        "r1-filter" | "r1-equalizer" => tone::build(layout, descriptor, band),
        "r1-split" | "r1-chain" => containers::build(layout, descriptor),
        "r1-saturator" | "r1-compressor" | "r1-modulator" | "r1-spacer" => {
            effects::build(layout, descriptor, page)
        }
        _ => Workspace {
            layout,
            output: vec![],
            toolbar: vec![],
            body_height: 0.0,
        },
    }
}
