//! CPU time per 256-frame block for every registered device at 48 kHz:
//! instruments playing a held eight-note chord, effects on noise. These are
//! measurements on this machine, not guarantees.
//!
//! `cargo run --release -p daw-devices --example devices_bench`
use daw_devices::devices::{compressor, saturator};
use daw_devices::{DeviceKind, Event, EventKind, ProcessContext, StereoIo, registry};
use std::time::Instant;

const RATE: f32 = 48_000.0;
const BLOCK: usize = 256;
const BLOCKS: usize = 4000;

/// The settings whose cost differs: `(label, parameter, choice)`.
fn variants(id: &str) -> Vec<(&'static str, u32, usize)> {
    let choices = |parameter: u32, labels: &'static [&'static str]| {
        labels
            .iter()
            .enumerate()
            .map(|(choice, label)| (*label, parameter, choice))
            .collect()
    };
    match id {
        "r1-saturator" => choices(saturator::QUALITY, saturator::QUALITIES),
        "r1-compressor" => choices(compressor::CHARACTER, compressor::CHARACTERS),
        _ => Vec::new(),
    }
}

fn main() {
    for descriptor in registry() {
        let variants = variants(descriptor.id);
        if variants.is_empty() {
            measure(descriptor, "", None);
        }
        for (label, parameter, choice) in variants {
            measure(descriptor, label, Some((parameter, choice as f32)));
        }
    }
}

fn measure(descriptor: &daw_devices::DeviceDescriptor, label: &str, setting: Option<(u32, f32)>) {
    let mut device = (descriptor.create)();
    device.prepare(RATE, BLOCK);
    if let Some((id, plain)) = setting {
        let spec = descriptor.parameter(id).expect("a parameter of the device");
        device.set_parameter(id, spec.to_normalized(plain));
    }
    let chord: Vec<Event> = [48.0, 52.0, 55.0, 59.0, 60.0, 64.0, 67.0, 71.0]
        .into_iter()
        .enumerate()
        .map(|(note, pitch)| Event {
            offset: 0,
            kind: EventKind::NoteOn {
                note: note as u32,
                pitch,
                velocity: 0.8,
            },
        })
        .collect();
    let mut left = vec![0.0; BLOCK];
    let mut right = vec![0.0; BLOCK];
    let mut state = 1u32;
    let mut elapsed = 0.0;
    for block in 0..BLOCKS {
        if descriptor.kind == DeviceKind::Effect {
            for (l, r) in left.iter_mut().zip(&mut right) {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *l = state as f32 / u32::MAX as f32 - 0.5;
                *r = *l;
            }
        }
        let events = if block == 0 && descriptor.kind == DeviceKind::Instrument {
            &chord[..]
        } else {
            &[]
        };
        let start = Instant::now();
        device.process(
            &ProcessContext::default(),
            events,
            StereoIo {
                left: &mut left,
                right: &mut right,
                sidechain: None,
            },
        );
        elapsed += start.elapsed().as_secs_f64();
    }
    let per_block = elapsed / BLOCKS as f64;
    let budget = BLOCK as f64 / f64::from(RATE);
    println!(
        "{:<12} {:<8} {:>7.1} µs per block, {:>5.2} % of real time{}",
        descriptor.name,
        label,
        per_block * 1e6,
        100.0 * per_block / budget,
        if descriptor.kind == DeviceKind::Instrument {
            " (8 voices)"
        } else {
            ""
        }
    );
}
