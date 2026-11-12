//! CPU time per 256-frame block for every registered device at 48 kHz:
//! instruments playing a held eight-note chord, effects on noise. The sampler
//! plays a four-second stereo recording; the drums a one-second stereo hit
//! on every pad, all sixteen together every second, and a busy pattern.
//! These are measurements on this machine, not guarantees.
//!
//! `cargo run --release -p daw-devices --example devices_bench`
use daw_devices::devices::{compressor, drums, modulator, sampler, saturator, spacer};
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
        if descriptor.id == drums::DESCRIPTOR.id {
            measure_drums(descriptor);
            continue;
        }
        let variants = variants(descriptor.id);
        if variants.is_empty() {
            measure(descriptor, "", &[], 8);
        }
        for (label, settings, voices) in variants {
            measure(descriptor, &label, &settings, voices);
        }
    }
}

/// One second of decaying stereo noise at 44.1 kHz: a drum hit, played
/// whole, resampled to the engine rate.
fn hit() -> Arc<SampleData> {
    let mut state = 11u32;
    let mut noise = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32 - 0.5
    };
    let decay = |n: usize| (-3.0 * n as f32 / 44_100.0).exp();
    let left: Vec<f32> = (0..44_100).map(|n| noise() * decay(n)).collect();
    let right: Vec<f32> = (0..44_100).map(|n| noise() * decay(n)).collect();
    Arc::new(SampleData::new(44_100.0, &[&left, &right]))
}

fn note(pad: usize, offset: u32) -> Event {
    let note = u32::from(drums::FIRST_NOTE) + pad as u32;
    Event {
        offset,
        kind: EventKind::NoteOn {
            note,
            pitch: note as f32,
            velocity: 0.8,
        },
    }
}

/// R1 Drums: every pad at once, once a second, so sixteen hits sound all
/// the time, plain and with each pad's filter and drive on; and a busy
/// pattern of sixteenth notes at 120 BPM.
fn measure_drums(descriptor: &daw_devices::DeviceDescriptor) {
    let blocks_per_second = (RATE / BLOCK as f32).round() as usize;
    let together = |block: usize| -> Vec<Event> {
        if block.is_multiple_of(blocks_per_second) {
            (0..drums::PADS).map(|pad| note(pad, 0)).collect()
        } else {
            Vec::new()
        }
    };
    let shaped: Vec<(u32, f32)> = (0..drums::PADS)
        .flat_map(|pad| {
            [
                (
                    drums::id(pad, drums::FILTER),
                    if pad % 2 == 0 { -40.0 } else { 40.0 },
                ),
                (drums::id(pad, drums::RESONANCE), 30.0),
                (drums::id(pad, drums::DRIVE), 9.0),
                (drums::id(pad, drums::PITCH_ENV), 12.0),
            ]
        })
        .collect();
    run_drums(descriptor, "16 pads", &[], &together);
    run_drums(descriptor, "16 shaped", &shaped, &together);
    // Sixteenth notes at 120 BPM: 0.125 s a step.
    let step_frames = (RATE * 0.125) as usize;
    let pattern = |block: usize| -> Vec<Event> {
        let (from, to) = (block * BLOCK, (block + 1) * BLOCK);
        let mut events = Vec::new();
        for step in from.div_ceil(step_frames)..to.div_ceil(step_frames) {
            let offset = (step * step_frames - from) as u32;
            let beat = step % 16;
            // A closed hat on every step, open on the last but one.
            events.push(note(if beat == 14 { 10 } else { 6 }, offset));
            if beat % 8 == 0 || beat == 11 {
                events.push(note(0, offset));
            }
            if beat % 8 == 4 {
                events.push(note(2, offset));
                events.push(note(3, offset));
            }
            if beat % 2 == 1 {
                events.push(note(1, offset));
            }
            if beat >= 12 {
                events.push(note(5 + 2 * (beat - 12), offset));
            }
            if beat == 0 && (step / 16) % 2 == 0 {
                events.push(note(13, offset));
            }
            if beat % 4 == 2 {
                events.push(note(15, offset));
            }
        }
        events
    };
    let mut kit: Vec<(u32, f32)> = [6, 8, 10]
        .into_iter()
        .map(|pad| (drums::id(pad, drums::CHOKE), 1.0))
        .collect();
    kit.extend([
        (drums::id(0, drums::DRIVE), 6.0),
        (drums::id(0, drums::PITCH_ENV), 12.0),
        (drums::id(6, drums::FILTER), 30.0),
        (drums::id(2, drums::FILTER), -20.0),
    ]);
    run_drums(descriptor, "Pattern", &kit, &pattern);
}

fn run_drums(
    descriptor: &daw_devices::DeviceDescriptor,
    label: &str,
    settings: &[(u32, f32)],
    schedule: &dyn Fn(usize) -> Vec<Event>,
) {
    let mut device = (descriptor.create)();
    device.prepare(RATE, BLOCK);
    let recording = hit();
    for slot in 0..descriptor.sample_slots {
        device.set_sample(slot, Some(Arc::clone(&recording)));
    }
    for (id, plain) in settings {
        let spec = descriptor.parameter(*id).expect("a drum parameter");
        device.set_parameter(*id, spec.to_normalized(*plain));
    }
    let telemetry = device.telemetry();
    let mut left = vec![0.0; BLOCK];
    let mut right = vec![0.0; BLOCK];
    let mut elapsed = 0.0;
    let mut voices = 0.0;
    for block in 0..BLOCKS {
        let events = schedule(block);
        let start = Instant::now();
        device.process(
            &ProcessContext::default(),
            &events,
            StereoIo {
                left: &mut left,
                right: &mut right,
                sidechain: None,
            },
        );
        elapsed += start.elapsed().as_secs_f64();
        voices += f64::from(telemetry.value(drums::TELEMETRY_VOICES));
    }
    let per_block = elapsed / BLOCKS as f64;
    let budget = BLOCK as f64 / f64::from(RATE);
    println!(
        "{:<12} {:<10} {:>7.1} µs per block, {:>5.2} % of real time ({:.1} hits sounding on average)",
        descriptor.name,
        label,
        per_block * 1e6,
        100.0 * per_block / budget,
        voices / BLOCKS as f64
    );
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
