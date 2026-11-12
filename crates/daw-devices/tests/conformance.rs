//! Registry-wide conformance: every registered device is checked here without
//! writing these tests again. See `docs/R1_DEVICES.md#verification`.
use daw_devices::{
    Device, DeviceDescriptor, DeviceKind, Event, EventKind, ParamRange, Preset, ProcessContext,
    SampleData, StereoIo, factory_presets, registry,
};
use std::sync::{Arc, OnceLock};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

// Forwards every call to System unchanged; only counts on tracked threads.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(pointer, layout, size) }
    }
}

fn count() {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count_allocations(action: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    action();
    TRACKING.with(|tracking| tracking.set(false));
    ALLOCATIONS.with(Cell::get)
}

const BLOCK: usize = 256;
const RATE: f32 = 48_000.0;

/// Seeded generator so every run tests the same inputs.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A stereo recording at another rate than the tests run at, so a device
/// with sample slots plays something that moves its whole voice path.
fn recording() -> Arc<SampleData> {
    static RECORDING: OnceLock<Arc<SampleData>> = OnceLock::new();
    Arc::clone(RECORDING.get_or_init(|| {
        let mut noise = Lcg(77);
        let (left, right): (Vec<f32>, Vec<f32>) = (0..22_050)
            .map(|n| {
                let t = n as f32 / 44_100.0;
                let decay = (-3.0 * t).exp();
                let tone = (std::f32::consts::TAU * 220.0 * t).sin();
                (
                    decay * (0.6 * tone + 0.2 * (noise.next() - 0.5)),
                    decay * (0.6 * tone - 0.2 * (noise.next() - 0.5)),
                )
            })
            .unzip();
        Arc::new(SampleData::new(44_100.0, &[&left, &right]))
    }))
}

fn prepared(descriptor: &DeviceDescriptor) -> Box<dyn Device> {
    let mut device = (descriptor.create)();
    device.prepare(RATE, BLOCK);
    for slot in 0..descriptor.sample_slots {
        device.set_sample(slot, Some(recording()));
    }
    device
}

/// Instruments get a chord with expression; effects get audio.
fn block_events(descriptor: &DeviceDescriptor, block: usize) -> Vec<Event> {
    if descriptor.kind != DeviceKind::Instrument {
        return Vec::new();
    }
    match block % 8 {
        0 => [48.0, 55.0, 63.5]
            .into_iter()
            .enumerate()
            .map(|(note, pitch)| Event {
                offset: 17 * note as u32,
                kind: EventKind::NoteOn {
                    note: note as u32,
                    pitch,
                    velocity: 0.8,
                },
            })
            .chain([
                Event {
                    offset: 100,
                    kind: EventKind::PitchBend(0.5),
                },
                Event {
                    offset: 120,
                    kind: EventKind::ModWheel(0.7),
                },
            ])
            .collect(),
        5 => (0..3)
            .map(|note| Event {
                offset: 31,
                kind: EventKind::NoteOff { note },
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether a device gets an external sidechain: the registry-wide tests run
/// a device that declares the input once without and once with it, so both
/// branches are swept.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sidechain {
    None,
    Noise,
}

fn sidechain_runs(descriptor: &DeviceDescriptor) -> &'static [Sidechain] {
    if descriptor.sidechain_input {
        &[Sidechain::None, Sidechain::Noise]
    } else {
        &[Sidechain::None]
    }
}

/// Runs `blocks` blocks, calling `before` ahead of each, and returns the
/// interleaved output.
fn render(
    device: &mut dyn Device,
    descriptor: &DeviceDescriptor,
    blocks: usize,
    input: &mut dyn FnMut() -> f32,
    before: &mut dyn FnMut(&mut dyn Device, usize),
) -> Vec<f32> {
    render_with(device, descriptor, blocks, input, before, Sidechain::None)
}

fn render_with(
    device: &mut dyn Device,
    descriptor: &DeviceDescriptor,
    blocks: usize,
    input: &mut dyn FnMut() -> f32,
    before: &mut dyn FnMut(&mut dyn Device, usize),
    sidechain: Sidechain,
) -> Vec<f32> {
    let mut output = Vec::with_capacity(blocks * BLOCK * 2);
    let mut left = vec![0.0; BLOCK];
    let mut right = vec![0.0; BLOCK];
    let mut key = Lcg(41);
    let mut key_left = vec![0.0; BLOCK];
    let mut key_right = vec![0.0; BLOCK];
    for block in 0..blocks {
        before(device, block);
        // Bursts, so the detector rises and falls.
        let level = if block % 6 < 3 { 2.0 } else { 0.01 };
        for (l, r) in key_left.iter_mut().zip(&mut key_right) {
            *l = level * (key.next() - 0.5);
            *r = level * (key.next() - 0.5);
        }
        for (l, r) in left.iter_mut().zip(&mut right) {
            *l = input();
            *r = input();
        }
        let context = ProcessContext {
            tempo: 120.0,
            beat: (block * BLOCK) as f64 / f64::from(RATE) * 2.0,
            playing: true,
        };
        let events = block_events(descriptor, block);
        device.process(
            &context,
            &events,
            StereoIo {
                left: &mut left,
                right: &mut right,
                sidechain: (sidechain == Sidechain::Noise)
                    .then_some((&key_left[..], &key_right[..])),
            },
        );
        for (l, r) in left.iter().zip(&right) {
            output.push(*l);
            output.push(*r);
        }
    }
    output
}

fn noise(seed: u64) -> impl FnMut() -> f32 {
    let mut random = Lcg(seed);
    move || random.next() - 0.5
}

#[test]
fn descriptors_are_well_formed() {
    let mut ids = std::collections::HashSet::new();
    for descriptor in registry() {
        assert!(
            ids.insert(descriptor.id),
            "duplicate device {}",
            descriptor.id
        );
        assert!(
            (1..=32).contains(&descriptor.id.len())
                && descriptor
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
            "{} must be 1–32 characters of a-z, 0-9 and -",
            descriptor.id
        );
        let mut parameter_ids = std::collections::HashSet::new();
        let mut keys = std::collections::HashSet::new();
        for spec in descriptor.parameters {
            let label = format!("{} {}", descriptor.id, spec.key);
            assert!(parameter_ids.insert(spec.id), "{label}: duplicate ID");
            assert!(keys.insert(spec.key), "{label}: duplicate key");
            assert!(!spec.name.is_empty() && !spec.group.is_empty(), "{label}");
            assert!(spec.default.is_finite(), "{label}: default");
            assert!(
                (spec.plain_min()..=spec.plain_max()).contains(&spec.default),
                "{label}: default out of range"
            );
            match spec.range {
                ParamRange::Linear { min, max } => assert!(min < max, "{label}"),
                ParamRange::Log { min, max } => assert!(0.0 < min && min < max, "{label}"),
                ParamRange::Integer { min, max } => {
                    assert!(min < max && spec.default.fract() == 0.0, "{label}")
                }
                ParamRange::Choice(options) => assert!(!options.is_empty(), "{label}"),
                ParamRange::Toggle => {}
            }
            let normalized = spec.default_normalized();
            assert_eq!(
                spec.parse(&spec.format(normalized))
                    .map(|value| spec.format(value)),
                Some(spec.format(normalized)),
                "{label}: formatted default does not parse back"
            );
        }
        assert_eq!(
            (descriptor.create)().descriptor().id,
            descriptor.id,
            "create() builds another device"
        );
    }
}

/// Saved projects and automation refer to IDs, presets to keys. Lines may be
/// appended for new parameters; existing lines never change.
#[test]
fn parameter_ids_and_keys_never_change() {
    let mut current = String::new();
    for descriptor in registry() {
        for spec in descriptor.parameters {
            let range = match spec.range {
                ParamRange::Linear { min, max } => format!("linear {min} {max}"),
                ParamRange::Log { min, max } => format!("log {min} {max}"),
                ParamRange::Integer { min, max } => format!("integer {min} {max}"),
                ParamRange::Choice(options) => format!("choice {}", options.join("|")),
                ParamRange::Toggle => "toggle".into(),
            };
            current.push_str(&format!(
                "{} {} {} \"{}\" {}\n",
                descriptor.id, spec.id, spec.key, spec.name, range
            ));
        }
    }
    let snapshot = include_str!("parameters.snapshot");
    let current_lines: Vec<_> = current.lines().collect();
    for line in snapshot.lines() {
        assert!(
            current_lines.contains(&line),
            "Parameter changed or removed: {line}\nIDs and keys are persisted; add a new parameter instead."
        );
    }
    for line in &current_lines {
        assert!(
            snapshot.lines().any(|saved| saved == *line),
            "New parameter not in tests/parameters.snapshot; append this line: {line}"
        );
    }
}

#[test]
fn factory_presets_load_without_skipped_keys() {
    for descriptor in registry() {
        assert!(
            !descriptor.factory_presets.is_empty(),
            "{} needs at least a default preset",
            descriptor.id
        );
        for text in descriptor.factory_presets {
            let preset = Preset::parse(text).unwrap_or_else(|error| panic!("{error}"));
            let resolved = preset.resolve(descriptor).unwrap();
            assert!(
                resolved.skipped.is_empty(),
                "{} preset \"{}\" skips {:?}",
                descriptor.id,
                preset.name,
                resolved.skipped
            );
        }
        assert_eq!(
            factory_presets(descriptor).len(),
            descriptor.factory_presets.len()
        );
    }
}

#[test]
fn presets_round_trip_through_capture() {
    for descriptor in registry() {
        let mut random = Lcg(7);
        let values: Vec<_> = descriptor
            .parameters
            .iter()
            .map(|spec| (spec.id, spec.to_normalized(spec.to_plain(random.next()))))
            .collect();
        let value = |id| values.iter().find(|(key, _)| *key == id).unwrap().1;
        let text = Preset::capture(descriptor, "Round trip", value).to_json();
        let resolved = Preset::parse(&text).unwrap().resolve(descriptor).unwrap();
        assert!(resolved.skipped.is_empty());
        for (id, normalized) in resolved.values {
            assert!(
                (normalized - value(id)).abs() < 1e-4,
                "{} parameter {id}",
                descriptor.id
            );
        }
    }
}

#[test]
fn processing_is_deterministic() {
    for descriptor in registry() {
        for &sidechain in sidechain_runs(descriptor) {
            let run = || {
                let mut device = prepared(descriptor);
                let mut random = Lcg(11);
                render_with(
                    &mut *device,
                    descriptor,
                    40,
                    &mut noise(3),
                    &mut |device, block| {
                        if block % 4 == 0 {
                            for spec in descriptor.parameters {
                                device.set_parameter(spec.id, random.next());
                            }
                        }
                    },
                    sidechain,
                )
            };
            let first = run();
            let second = run();
            assert!(
                first
                    .iter()
                    .zip(&second)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{} output differs between runs ({sidechain:?})",
                descriptor.id
            );
        }
    }
}

#[test]
fn reset_restores_the_initial_state() {
    for descriptor in registry() {
        let mut device = prepared(descriptor);
        let run =
            |device: &mut dyn Device| render(device, descriptor, 8, &mut noise(5), &mut |_, _| {});
        let first = run(&mut *device);
        device.reset();
        let second = run(&mut *device);
        assert_eq!(first, second, "{} keeps state across reset", descriptor.id);
    }
}

#[test]
fn random_parameters_and_hostile_input_stay_finite() {
    for descriptor in registry() {
        for seed in 0..12 * sidechain_runs(descriptor).len() as u64 {
            let sidechain = sidechain_runs(descriptor)[seed as usize / 12];
            let mut device = prepared(descriptor);
            let mut random = Lcg(seed);
            let mut source = Lcg(seed + 100);
            let mut sample = 0usize;
            let mut input = move || {
                sample += 1;
                match (sample / 3000) % 4 {
                    0 => source.next() - 0.5,
                    1 => 0.0,
                    // Full-scale square and a large overload.
                    2 => {
                        if (sample / 40).is_multiple_of(2) {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    _ => 8.0 * (source.next() - 0.5),
                }
            };
            let output = render_with(
                &mut *device,
                descriptor,
                120,
                &mut input,
                &mut |device, block| {
                    if block % 3 == 0 {
                        // Extremes are where filters blow up.
                        for spec in descriptor.parameters {
                            let value = match (random.next() * 3.0) as u32 {
                                0 => 0.0,
                                1 => 1.0,
                                _ => random.next(),
                            };
                            device.set_parameter(spec.id, value);
                        }
                    }
                },
                sidechain,
            );
            let peak = output.iter().fold(0.0_f32, |peak, x| peak.max(x.abs()));
            assert!(
                output.iter().all(|x| x.is_finite()) && peak < 1000.0,
                "{} seed {seed} ({sidechain:?}): peak {peak}",
                descriptor.id
            );
        }
    }
}

#[test]
fn silence_in_gives_exact_silence_out() {
    for descriptor in registry() {
        if descriptor.kind == DeviceKind::Instrument {
            continue;
        }
        let mut device = prepared(descriptor);
        let output = render(&mut *device, descriptor, 20, &mut || 0.0, &mut |_, _| {});
        assert!(
            output.iter().all(|x| *x == 0.0),
            "{} produces output from silence",
            descriptor.id
        );
    }
}

#[test]
fn decays_do_not_produce_denormals() {
    for descriptor in registry() {
        let mut device = prepared(descriptor);
        let mut sample = 0usize;
        let mut input = || {
            sample += 1;
            if sample < 2000 { 0.5 } else { 0.0 }
        };
        let output = render(&mut *device, descriptor, 600, &mut input, &mut |_, _| {});
        assert!(
            output.iter().all(|x| *x == 0.0 || x.is_normal()),
            "{} output contains denormal numbers",
            descriptor.id
        );
    }
}

#[test]
fn process_does_not_allocate() {
    for descriptor in registry() {
        let mut device = prepared(descriptor);
        let mut random = Lcg(19);
        let mut left = vec![0.0; BLOCK];
        let mut right = vec![0.0; BLOCK];
        let events: Vec<_> = (0..8)
            .map(|block| block_events(descriptor, block))
            .collect();
        let mut input = noise(23);
        let key = vec![0.3; BLOCK];
        let allocations = count_allocations(|| {
            for block in 0..64 {
                for spec in descriptor.parameters {
                    device.set_parameter(spec.id, random.next());
                }
                for (l, r) in left.iter_mut().zip(&mut right) {
                    *l = input();
                    *r = input();
                }
                device.process(
                    &ProcessContext::default(),
                    &events[block % 8],
                    StereoIo {
                        left: &mut left,
                        right: &mut right,
                        sidechain: (descriptor.sidechain_input && block % 2 == 1)
                            .then_some((&key[..], &key[..])),
                    },
                );
            }
        });
        assert_eq!(allocations, 0, "{} allocates in process", descriptor.id);
    }
}

/// A full-range jump of any continuous parameter must ramp, not click.
#[test]
fn parameter_jumps_are_click_free() {
    for descriptor in registry() {
        let continuous = descriptor.parameters.iter().filter(|spec| {
            matches!(
                spec.range,
                ParamRange::Linear { .. } | ParamRange::Log { .. }
            )
        });
        for spec in continuous {
            let sine = |n: usize| 0.25 * (n as f32 * 0.05).sin();
            let max_step = |from: f32, to: f32| {
                let mut device = prepared(descriptor);
                device.set_parameter(spec.id, from);
                let mut n = 0;
                let output = render(
                    &mut *device,
                    descriptor,
                    24,
                    &mut || {
                        n += 1;
                        sine(n / 2)
                    },
                    &mut |device, block| {
                        if block == 12 {
                            device.set_parameter(spec.id, to);
                        }
                    },
                );
                let left: Vec<_> = output.iter().step_by(2).copied().collect();
                // Skip the start-up transient; measure across the jump.
                left[6 * BLOCK..]
                    .windows(2)
                    .map(|pair| (pair[1] - pair[0]).abs())
                    .fold(0.0_f32, f32::max)
            };
            let steady = max_step(0.0, 0.0).max(max_step(1.0, 1.0));
            for (from, to) in [(0.0, 1.0), (1.0, 0.0)] {
                let jump = max_step(from, to);
                assert!(
                    jump <= steady * 1.5 + 0.02,
                    "{} {}: jump {from}→{to} steps {jump}, steady {steady}",
                    descriptor.id,
                    spec.key
                );
            }
        }
    }
}

/// The engine compares `descriptor.latency` across an edit to decide whether
/// delay compensation must be rebuilt, so it has to be what the device
/// reports for the same values.
#[test]
fn descriptor_latency_is_what_the_device_reports() {
    for descriptor in registry() {
        let mut random = Lcg(29);
        for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for round in 0..40 {
                let values: Vec<_> = descriptor
                    .parameters
                    .iter()
                    .map(|spec| {
                        let value = if round == 0 {
                            spec.default_normalized()
                        } else {
                            random.next()
                        };
                        (spec.id, value)
                    })
                    .collect();
                let mut device = (descriptor.create)();
                device.prepare(rate, BLOCK);
                for (id, value) in &values {
                    device.set_parameter(*id, *value);
                }
                let value = |id| values.iter().find(|(key, _)| *key == id).unwrap().1;
                assert_eq!(
                    (descriptor.latency)(&value, rate),
                    device.latency_samples(),
                    "{} at {rate} Hz, round {round}",
                    descriptor.id
                );
            }
        }
    }
}

#[test]
fn a_roll_stays_within_its_amount_and_keeps_the_output_level() {
    for descriptor in registry() {
        let defaults = |id: u32| {
            descriptor
                .parameter(id)
                .map_or(0.0, |spec| spec.default_normalized())
        };
        for seed in 0..32 {
            for amount in [0.01_f32, 0.1] {
                for (id, value) in daw_devices::randomized(descriptor, defaults, amount, seed) {
                    let spec = descriptor.parameter(id).unwrap();
                    assert_ne!(spec.key, "output", "{}", descriptor.id);
                    assert!((0.0..=1.0).contains(&value));
                    if spec.steps().is_none() {
                        assert!(
                            (value - defaults(id)).abs() <= amount + 1e-6,
                            "{} {} moved {} at {amount}",
                            descriptor.id,
                            spec.key,
                            value - defaults(id)
                        );
                    }
                }
            }
        }
        // A full roll moves most of the device, choices included.
        let rolled = daw_devices::randomized(descriptor, defaults, 1.0, 7);
        assert!(
            rolled.len() * 2 > descriptor.parameters.len(),
            "{}",
            descriptor.id
        );
        for (id, value) in &rolled {
            let spec = descriptor.parameter(*id).unwrap();
            if spec.steps().is_some()
                && !matches!(spec.range, daw_devices::ParamRange::Integer { .. })
            {
                assert_ne!(spec.to_plain(*value), spec.to_plain(defaults(*id)));
            }
        }
    }
}
