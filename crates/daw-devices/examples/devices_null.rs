//! Null test for changes that must not change the sound: renders every
//! factory preset of every registered device from a fixed performance and
//! either writes the renders or compares them with earlier ones.
//!
//! ```text
//! cargo run --release -p daw-devices --example devices_null -- write target/null
//! cargo run --release -p daw-devices --example devices_null -- compare target/null
//! ```
//!
//! `compare` prints, per render, the residual against the reference (0 dB
//! is a residual as loud as the reference, `-inf` a bit-identical render),
//! the change in level, and the largest change of a third-octave band of
//! the long-term spectrum with that band's frequency and its level under
//! the loudest band, which still reads when a change moved the timing and
//! nothing nulls. An optional third argument keeps the renders whose
//! name contains it.
use daw_devices::devices::drums;
use daw_devices::{
    DeviceDescriptor, DeviceKind, Event, EventKind, ProcessContext, SampleData, StereoIo,
    factory_presets, registry,
};
use rustfft::{FftPlanner, num_complex::Complex};
use std::path::Path;
use std::sync::Arc;

const RATE: f32 = 48_000.0;
const BLOCK: usize = 256;
const SECONDS: usize = 8;
const TEMPO: f64 = 120.0;

struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 - 0.5
    }
}

/// A decaying tone of ten harmonics, as a plucked string is.
fn pluck(hertz: f32, seconds: f32, rate: f32, time: f32) -> f32 {
    if !(0.0..seconds).contains(&time) {
        return 0.0;
    }
    let decay = (-4.0 * time / seconds).exp();
    (1..=10)
        .filter(|harmonic| *harmonic as f32 * hertz < 0.45 * rate)
        .map(|harmonic| {
            let harmonic = harmonic as f32;
            (std::f32::consts::TAU * harmonic * hertz * time).sin() / harmonic
        })
        .sum::<f32>()
        * decay
        * 0.3
}

/// What the effects process: plucked notes panned apart over a noise burst
/// on every second beat, then a second of silence for the tails.
fn program(frame: usize) -> (f32, f32, bool) {
    let time = frame as f32 / RATE;
    if time >= (SECONDS - 2) as f32 {
        return (0.0, 0.0, false);
    }
    let pitches = [110.0, 164.81, 220.0, 329.63, 440.0, 659.26, 880.0, 1318.5];
    let step = (time / 0.25) as usize;
    let since = time - step as f32 * 0.25;
    let note = pluck(pitches[step % pitches.len()], 0.5, RATE, since);
    let earlier = pluck(pitches[(step + 7) % pitches.len()], 0.5, RATE, since + 0.25);
    (
        note + 0.4 * earlier,
        0.4 * note + earlier,
        step % 4 == 2 && since < 0.08,
    )
}

/// A two-second stereo recording for the sample players.
fn recording() -> Arc<SampleData> {
    let rate = 44_100.0;
    let mut noise = Noise(7);
    let left: Vec<f32> = (0..88_200)
        .map(|n| pluck(261.63, 2.0, rate, n as f32 / rate) + 0.02 * noise.next())
        .collect();
    let right: Vec<f32> = (0..88_200)
        .map(|n| pluck(262.4, 2.0, rate, n as f32 / rate) + 0.02 * noise.next())
        .collect();
    Arc::new(SampleData::new(rate, &[&left, &right]))
}

/// The notes an instrument plays in one block: a chord every second, held
/// for 0.7 s, with a melody of eighth notes over it; drums a pattern on
/// their sixteen pads.
fn events(descriptor: &DeviceDescriptor, block: usize) -> Vec<Event> {
    let mut events = Vec::new();
    let (from, to) = (block * BLOCK, (block + 1) * BLOCK);
    let mut push = |frame: usize, kind: EventKind| {
        if (from..to).contains(&frame) && frame < (SECONDS - 2) * RATE as usize {
            events.push(Event {
                offset: (frame - from) as u32,
                kind,
            });
        }
    };
    let on = |note: u32, velocity: f32| EventKind::NoteOn {
        note,
        pitch: note as f32,
        velocity,
    };
    let second = RATE as usize;
    if descriptor.id == drums::DESCRIPTOR.id {
        for step in 0..SECONDS * 8 {
            let frame = step * second / 8;
            let pad = [0, 6, 2, 6, 0, 10, 3, 15][step % 8] + (step / 8 % 2) as u32;
            let note = u32::from(drums::FIRST_NOTE) + pad;
            push(frame, on(note, 0.5 + 0.06 * (step % 8) as f32));
            push(frame + second / 16, EventKind::NoteOff { note });
        }
        return events;
    }
    for bar in 0..SECONDS {
        let root = [48, 53, 55, 50][bar % 4];
        for (index, interval) in [0, 4, 7, 11].into_iter().enumerate() {
            let note = root + interval;
            push(bar * second + 40 * index, on(note, 0.8));
            push(bar * second + 7 * second / 10, EventKind::NoteOff { note });
        }
        for eighth in 0..8 {
            let note = root + 24 + [0, 7, 12, 4, 11, 7, 16, 12][eighth];
            let frame = bar * second + eighth * second / 8;
            push(frame, on(note, 0.4 + 0.07 * eighth as f32));
            push(frame + second / 10, EventKind::NoteOff { note });
        }
        push(bar * second + second / 2, EventKind::PitchBend(0.3));
        push(bar * second + 3 * second / 4, EventKind::PitchBend(0.0));
        push(bar * second, EventKind::ModWheel(0.5 * (bar % 2) as f32));
    }
    events
}

/// Left and right of one render, one after the other.
fn render(descriptor: &DeviceDescriptor, settings: &[(u32, f32)]) -> Vec<f32> {
    let mut device = (descriptor.create)();
    device.prepare(RATE, BLOCK);
    if descriptor.sample_slots > 0 {
        let recording = recording();
        for slot in 0..descriptor.sample_slots {
            device.set_sample(slot, Some(Arc::clone(&recording)));
        }
    }
    for (id, normalized) in settings {
        device.set_parameter(*id, *normalized);
    }
    let frames = SECONDS * RATE as usize;
    let mut out = vec![0.0; 2 * frames];
    let (out_left, out_right) = out.split_at_mut(frames);
    let mut noise = Noise(1);
    let mut left = [0.0; BLOCK];
    let mut right = [0.0; BLOCK];
    for block in 0..frames / BLOCK {
        if descriptor.kind == DeviceKind::Effect {
            for (index, (l, r)) in left.iter_mut().zip(&mut right).enumerate() {
                let (a, b, burst) = program(block * BLOCK + index);
                let hit = if burst { 0.5 * noise.next() } else { 0.0 };
                (*l, *r) = (a + hit, b + hit);
            }
        }
        let events = events(descriptor, block);
        let context = ProcessContext {
            tempo: TEMPO,
            beat: (block * BLOCK) as f64 / f64::from(RATE) * TEMPO / 60.0,
            playing: true,
        };
        device.process(
            &context,
            &events,
            StereoIo {
                left: &mut left,
                right: &mut right,
                sidechain: None,
            },
        );
        let at = block * BLOCK;
        out_left[at..at + BLOCK].copy_from_slice(&left);
        out_right[at..at + BLOCK].copy_from_slice(&right);
    }
    out
}

fn db(ratio: f64) -> f64 {
    20.0 * ratio.max(1e-12).log10()
}

fn rms(samples: &[f32]) -> f64 {
    (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len().max(1) as f64)
        .sqrt()
}

/// Power per third-octave band from 25 Hz to 20 kHz, summed over
/// half-overlapping Hann windows of both channels.
fn bands(samples: &[f32]) -> Vec<f64> {
    const SIZE: usize = 16_384;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(SIZE);
    let mut power = vec![0.0_f64; SIZE / 2];
    let frames = samples.len() / 2;
    for channel in samples.chunks(frames) {
        for window in channel.windows(SIZE).step_by(SIZE / 2) {
            let mut buffer: Vec<Complex<f32>> = window
                .iter()
                .enumerate()
                .map(|(n, s)| {
                    let hann = 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / SIZE as f32).cos();
                    Complex::new(s * hann, 0.0)
                })
                .collect();
            fft.process(&mut buffer);
            for (sum, bin) in power.iter_mut().zip(&buffer) {
                *sum += f64::from(bin.norm_sqr());
            }
        }
    }
    (0..30)
        .map(|band| {
            let centre = 25.0 * 2.0_f64.powf(band as f64 / 3.0);
            let bin = |hertz: f64| (hertz * SIZE as f64 / f64::from(RATE)) as usize;
            let (low, high) = (bin(centre / 1.122), bin(centre * 1.122).min(SIZE / 2 - 1));
            power[low..=high.max(low)].iter().sum()
        })
        .collect()
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (Some(mode), Some(directory)) = (arguments.first(), arguments.get(1)) else {
        eprintln!("devices_null write|compare <directory> [filter]");
        std::process::exit(2);
    };
    let directory = Path::new(directory);
    std::fs::create_dir_all(directory).expect("the directory for the renders");
    let mut worst = (f64::NEG_INFINITY, String::new());
    for descriptor in registry() {
        for preset in factory_presets(descriptor) {
            let name = format!("{} {}", descriptor.id, preset.name);
            if arguments
                .get(2)
                .is_some_and(|filter| !name.contains(filter.as_str()))
            {
                continue;
            }
            let resolved = preset.resolve(descriptor).expect("a factory preset");
            let out = render(descriptor, &resolved.values);
            let path = directory.join(format!("{}.f32", name.replace([' ', '/'], "_")));
            if mode == "write" {
                let bytes: Vec<u8> = out.iter().flat_map(|s| s.to_le_bytes()).collect();
                std::fs::write(&path, bytes).expect("a writable directory");
                println!("{name:<44} {:>7.1} dBFS", db(rms(&out)));
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                println!("{name:<44} no reference");
                continue;
            };
            let reference: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| f32::from_le_bytes(*bytes))
                .collect();
            let residual: Vec<f32> = out.iter().zip(&reference).map(|(a, b)| a - b).collect();
            let null = if residual.iter().all(|s| *s == 0.0) {
                f64::NEG_INFINITY
            } else {
                db(rms(&residual) / rms(&reference).max(1e-12))
            };
            let level = db(rms(&out) / rms(&reference).max(1e-12));
            // Bands more than 60 dB under the loudest say nothing audible.
            let (now, before) = (bands(&out), bands(&reference));
            let loudest = before.iter().copied().fold(0.0, f64::max);
            let (band, hertz, under) = now
                .iter()
                .zip(&before)
                .enumerate()
                .filter(|(_, (_, before))| **before > loudest * 1e-6)
                .map(|(band, (now, before))| {
                    (
                        0.5 * db(now / before),
                        25.0 * 2.0_f64.powf(band as f64 / 3.0),
                        0.5 * db(before / loudest),
                    )
                })
                .fold((0.0_f64, 0.0, 0.0), |a, b| {
                    if b.0.abs() > a.0.abs() { b } else { a }
                });
            println!(
                "{name:<40} residual {null:>7.1} dB  level {level:>+6.2} dB  band {band:>+6.2} dB at {hertz:>5.0} Hz ({under:>+4.0} dB)"
            );
            if null > worst.0 {
                worst = (null, name);
            }
        }
    }
    if mode == "compare" {
        println!("largest residual: {:.1} dB ({})", worst.0, worst.1);
    }
}
