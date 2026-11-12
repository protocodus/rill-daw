//! CPU time per 256-frame block for every registered device at 48 kHz:
//! instruments playing a held eight-note chord, effects on noise. The sampler
//! plays a four-second stereo recording. These are measurements on this
//! machine, not guarantees.
//!
//! `cargo run --release -p daw-devices --example devices_bench`
use daw_devices::devices::{compressor, modulator, sampler, saturator, spacer};
use daw_devices::{DeviceKind, Event, EventKind, ProcessContext, SampleData, StereoIo, registry};
use std::sync::Arc;
use std::time::Instant;

const RATE: f32 = 48_000.0;
const BLOCK: usize = 256;
const BLOCKS: usize = 4000;

type Variant = (String, Vec<(u32, f32)>, usize);

/// The settings whose cost differs: a label, the plain values to set and the
/// number of notes held.
fn variants(id: &str) -> Vec<Variant> {
    let choices = |parameter: u32, labels: &'static [&'static str]| {
        labels
            .iter()
            .enumerate()
            .map(|(choice, label)| (label.to_string(), vec![(parameter, choice as f32)], 8))
            .collect()
    };
    match id {
        "r1-saturator" => choices(saturator::QUALITY, saturator::QUALITIES),
        "r1-compressor" => choices(compressor::CHARACTER, compressor::CHARACTERS),
        "r1-modulator" => choices(modulator::MODE, modulator::MODES),
        "r1-spacer" => {
            // Each model alone, and the device as inserted.
            let section = |name: &str, other: u32, model: u32, labels: &'static [&'static str]| {
                labels
                    .iter()
                    .enumerate()
                    .map(|(choice, label)| {
                        (
                            format!("{name} {label}"),
                            vec![(other, 0.0), (model, choice as f32)],
                            8,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let mut lines = vec![("Default".to_string(), Vec::new(), 8)];
            lines.extend(section(
                "Echo",
                spacer::REVERB_ON,
                spacer::ECHO_MODEL,
                spacer::ECHO_MODELS,
            ));
            lines.extend(section(
                "Reverb",
                spacer::ECHO_ON,
                spacer::REVERB_MODEL,
                spacer::REVERB_MODELS,
            ));
            lines
        }
        "r1-sampler" => {
            // Looped, so every voice plays for the whole measurement.
            let looped = vec![(sampler::LOOP_MODE, 1.0)];
            let filtered = [looped.clone(), vec![(sampler::FILTER_ON, 1.0)]].concat();
            let modulated = [
                filtered.clone(),
                vec![(sampler::LFO_PITCH, 0.5), (sampler::LFO_CUTOFF, 1.0)],
            ]
            .concat();
            vec![
                ("Loop".to_string(), looped, 8),
                ("Filter".to_string(), filtered.clone(), 8),
                ("LFO".to_string(), modulated, 8),
                ("Filter".to_string(), filtered, 32),
            ]
        }
        _ => Vec::new(),
    }
}

/// Four seconds of stereo noise at 44.1 kHz.
fn recording() -> Arc<SampleData> {
    let mut state = 7u32;
    let mut noise = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32 - 0.5
    };
    let left: Vec<f32> = (0..176_400).map(|_| noise()).collect();
    let right: Vec<f32> = (0..176_400).map(|_| noise()).collect();
    Arc::new(SampleData::new(44_100.0, &[&left, &right]))
}

fn main() {
    for descriptor in registry() {
        let variants = variants(descriptor.id);
        if variants.is_empty() {
            measure(descriptor, "", &[], 8);
        }
        for (label, settings, voices) in variants {
            measure(descriptor, &label, &settings, voices);
        }
    }
}

fn measure(
    descriptor: &daw_devices::DeviceDescriptor,
    label: &str,
    settings: &[(u32, f32)],
    voices: usize,
) {
    let mut device = (descriptor.create)();
    device.prepare(RATE, BLOCK);
    for slot in 0..descriptor.sample_slots {
        device.set_sample(slot, Some(recording()));
    }
    for (id, plain) in settings {
        let spec = descriptor
            .parameter(*id)
            .expect("a parameter of the device");
        device.set_parameter(*id, spec.to_normalized(*plain));
    }
    // More than eight voices repeat the chord, slightly detuned.
    let pitches = [48.0, 52.0, 55.0, 59.0, 60.0, 64.0, 67.0, 71.0];
    let chord: Vec<Event> = (0..voices)
        .map(|note| Event {
            offset: 0,
            kind: EventKind::NoteOn {
                note: note as u32,
                pitch: pitches[note % 8] + 0.1 * (note / 8) as f32,
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
            format!(" ({voices} voices)")
        } else {
            String::new()
        }
    );
}
