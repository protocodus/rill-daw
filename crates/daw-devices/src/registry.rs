use crate::{Device, DeviceDescriptor, devices};

/// Every built-in device, in Library order. Adding a device is one line here.
static REGISTRY: &[&DeviceDescriptor] = &[
    &devices::mini::DESCRIPTOR,
    &devices::filter::DESCRIPTOR,
    &devices::equalizer::DESCRIPTOR,
    &devices::saturator::DESCRIPTOR,
    &devices::compressor::DESCRIPTOR,
    &devices::modulator::DESCRIPTOR,
    &devices::spacer::DESCRIPTOR,
];

pub fn registry() -> &'static [&'static DeviceDescriptor] {
    REGISTRY
}

pub fn descriptor(id: &str) -> Option<&'static DeviceDescriptor> {
    REGISTRY
        .iter()
        .copied()
        .find(|descriptor| descriptor.id == id)
}

/// A new, unprepared instance at its default parameters.
pub fn create(id: &str) -> Option<Box<dyn Device>> {
    descriptor(id).map(|descriptor| (descriptor.create)())
}
