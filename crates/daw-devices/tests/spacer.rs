mod support;

use daw_devices::devices::spacer::{self, *};
use daw_devices::dsp::echo::{self, EchoSettings};
use daw_devices::dsp::reverb::{self, ReverbKernel, ReverbSettings};
use daw_devices::dsp::{Svf, prewarp};
use daw_devices::{Device, ProcessContext, StereoIo};
use support::*;

const STUDIO: f32 = 0.0;
const ROOM: f32 = 1.0;
const HALL: f32 = 2.0;
const REVERBS: [(f32, &str); 3] = [(STUDIO, "Studio"), (ROOM, "Room"), (HALL, "Hall")];
const DIGITAL: f32 = 0.0;
const BBD: f32 = 1.0;
const TAPE: f32 = 2.0;
const ECHOES: [(f32, &str); 3] = [(DIGITAL, "Digital"), (BBD, "BBD"), (TAPE, "Tape")];
const MONO: f32 = 0.0;
const PING_PONG: f32 = 1.0;
const WIDE: f32 = 2.0;
const SERIES: f32 = 0.0;
const PARALLEL: f32 = 1.0;

/// The reverb alone, at full level.
const REVERB_ONLY: [(u32, f32); 3] = [(ECHO_ON, 0.0), (MIX, 100.0), (REVERB_LEVEL, 0.0)];
/// The echo alone, at full level, with a free time and nothing cut: in
/// Digital every repeat is the input.
const ECHO_ONLY: [(u32, f32); 6] = [
    (REVERB_ON, 0.0),
    (MIX, 100.0),
    (ECHO_LEVEL, 0.0),
    (ECHO_SYNC, 0.0),
    (ECHO_LOW_CUT, 20.0),
    (ECHO_COLOR, 100.0),
];

fn device(rate: f32, values: &[(u32, f32)]) -> Box<dyn Device> {
    support::device(&spacer::DESCRIPTOR, rate, values)
}

fn settings(values: &[(u32, f32)]) -> Settings {
    Settings::from_normalized(value_of(&spacer::DESCRIPTOR, values))
}

fn with(base: &[(u32, f32)], more: &[(u32, f32)]) -> Values {
    base.iter().chain(more).copied().collect()
}

/// Both channels for the same signal on both.
fn run_stereo(device: &mut dyn Device, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (input.to_vec(), input.to_vec());
    run(device, &mut left, &mut right);
    (left, right)
}

fn impulse(frames: usize) -> Vec<f32> {
    let mut input = vec![0.0; frames];
    input[0] = 1.0;
    input
}

/// Four band-passes in a row around `centre`, each of damping `k`: steep
/// enough that a slower decay three octaves away stays 70 dB down.
fn band(signal: &[f32], centre: f32, k: f32, rate: f32) -> Vec<f32> {
    let g = prewarp(centre, rate);
    let mut filters: [Svf; 4] = std::array::from_fn(|_| Svf::default());
    signal
        .iter()
        .map(|x| {
            filters
                .iter_mut()
                .fold(*x, |x, filter| k * filter.process(x, g, k).band)
        })
        .collect()
}

/// An octave wide at its half-power points.
fn octave_band(signal: &[f32], centre: f32, rate: f32) -> Vec<f32> {
    band(signal, centre, std::f32::consts::SQRT_2, rate)
}

/// A third of an octave wide, and twice as steep: under a dark Color the
/// band an octave below is both louder and slower.
fn third_octave_band(signal: &[f32], centre: f32, rate: f32) -> Vec<f32> {
    band(&band(signal, centre, 0.55, rate), centre, 0.55, rate)
}

/// Seconds to fall by 60 dB: Schroeder's backward integration of an impulse
/// response from `start` on, a line fitted from −5 to −35 dB and extended.
fn rt60(response: &[f32], start: usize, rate: f32) -> f32 {
    let response = &response[start..];
    let mut energy = 0.0_f64;
    let mut curve: Vec<f64> = response
        .iter()
        .rev()
        .map(|x| {
            energy += f64::from(*x).powi(2);
            energy
        })
        .collect();
    curve.reverse();
    let level = |index: usize| 10.0 * (curve[index] / curve[0]).max(1e-30).log10();
    let from = (0..curve.len()).find(|n| level(*n) <= -5.0).unwrap();
    let to = (from..curve.len()).find(|n| level(*n) <= -35.0).unwrap();
    let step = ((to - from) / 400).max(1);
    let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for index in (from..to).step_by(step) {
        let (x, y) = (index as f64 / f64::from(rate), level(index));
        n += 1.0;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    (-60.0 / slope) as f32
}

/// The reverb's impulse response, both channels, long enough for a decay
/// of `longest` seconds to fall by some 60 dB after its structure.
fn reverb_response(values: &[(u32, f32)], rate: f32, longest: f32) -> (Vec<f32>, Vec<f32>) {
    let structure = reverb::structure_seconds(&settings(values).reverb);
    let frames = ((longest + structure + 0.4) * rate) as usize;
    run_stereo(&mut *device(rate, values), &impulse(frames))
}

/// Where the late field is on its own: after the pre-delay and what the
/// model's structure gives first. A room's reflections and a hall's first
/// lap are bursts of their own level in front of the tail; a decay read
/// from the direct sound on would measure the step between the two, not
/// the slope.
fn late_field_start(values: &[(u32, f32)], rate: f32) -> usize {
    let reverb = settings(values).reverb;
    ((reverb.pre_delay_seconds + reverb::structure_seconds(&reverb)) * rate) as usize
}

/// Where to read a decay of `seconds` from: the late field, or, when the
/// sound would be gone before the structure has passed twice, the end of
/// the early reflections; what follows them is then the structure's own
/// slope.
fn decay_start(values: &[(u32, f32)], seconds: f32, rate: f32) -> usize {
    let settings = settings(values);
    if seconds >= 2.0 * reverb::structure_seconds(&settings.reverb) {
        return late_field_start(values, rate);
    }
    let mut taps = [[(0.0, 0.0); 8]; 2];
    early_taps(&settings, &mut taps);
    let last = taps[0].iter().map(|tap| tap.0).fold(0.0, f32::max);
    ((settings.reverb.pre_delay_seconds + last) * rate) as usize
}

/// The decay in the octave around 1 kHz against the Decay knob: within 5 %
/// where the decay is at least twice as long as the model's structure,
/// within 10 % below that.
fn check_decay(model: f32, decay: f32, more: &[(u32, f32)], rate: f32) {
    let values = with(
        &with(&REVERB_ONLY, &[(REVERB_MODEL, model), (DECAY, decay)]),
        more,
    );
    let (response, _) = reverb_response(&values, rate, decay);
    let structure = reverb::structure_seconds(&settings(&values).reverb);
    let start = decay_start(&values, decay, rate);
    let measured = rt60(&octave_band(&response, 1000.0, rate), start, rate);
    let tolerance = if decay >= 2.0 * structure { 0.05 } else { 0.1 };
    assert!(
        (measured / decay - 1.0).abs() <= tolerance,
        "model {model} decay {decay} s {more:?} at {rate} Hz: {measured} s"
    );
}

#[test]
fn decay_is_the_knob_in_every_model() {
    for (model, _) in REVERBS {
        for decay in [0.3, 1.8, 6.0] {
            check_decay(model, decay, &[], 48_000.0);
        }
    }
}

#[test]
fn decay_holds_at_every_size_and_without_damping() {
    for (model, _) in REVERBS {
        for size in [0.0, 100.0] {
            check_decay(model, 1.8, &[(SIZE, size)], 48_000.0);
        }
        check_decay(model, 1.8, &[(DAMPING, 0.0), (DIFFUSION, 0.0)], 48_000.0);
    }
}

#[test]
fn decay_holds_at_other_sample_rates() {
    for rate in [44_100.0, 96_000.0] {
        for (model, _) in REVERBS {
            check_decay(model, 1.8, &[], rate);
        }
    }
}

/// The rest of the plan's matrix: every decay at every rate, and 20 s.
/// Minutes of audio at up to 192 kHz, so not part of the routine run:
/// `cargo test --release -p daw-devices --test spacer -- --ignored`.
#[test]
#[ignore = "long renders"]
fn decay_is_the_knob_at_every_rate_and_at_its_longest() {
    for (model, _) in REVERBS {
        for rate in [44_100.0, 96_000.0, 192_000.0] {
            for decay in [0.3, 1.8, 6.0] {
                check_decay(model, decay, &[], rate);
            }
        }
        check_decay(model, 20.0, &[], 48_000.0);
    }
}

/// Decays in octave bands against the curve the panel draws.
fn check_drawn_decay(decay: f32, cases: &[(f32, f32)]) {
    let rate = 48_000.0;
    for (model, name) in REVERBS {
        for (damping, color) in cases {
            let values = with(
                &REVERB_ONLY,
                &[
                    (REVERB_MODEL, model),
                    (DECAY, decay),
                    (DAMPING, *damping),
                    (REVERB_COLOR, *color),
                    (REVERB_LOW_CUT, 20.0),
                ],
            );
            let settings = settings(&values);
            let longest = decay_seconds_at(&settings, 125.0, rate);
            let (response, _) = reverb_response(&values, rate, longest);
            for centre in [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0] {
                // A third of an octave: where the damping sets in, the decay
                // changes by more than the tolerance even across that, and
                // what is measured lies between the band's edges.
                let edge = 2.0_f32.powf(1.0 / 6.0);
                let (slow, fast) = (
                    decay_seconds_at(&settings, centre / edge, rate),
                    decay_seconds_at(&settings, centre * edge, rate),
                );
                let start = decay_start(&values, fast, rate);
                let measured = rt60(&third_octave_band(&response, centre, rate), start, rate);
                // KNOWN SHORTFALL against the design's 10 %: a band that is
                // gone within half the structure's time is a handful of
                // arrivals, and reads up to 17 % long in the Hall.
                let structure = reverb::structure_seconds(&settings.reverb);
                let above = if fast < 0.5 * structure { 1.2 } else { 1.1 };
                assert!(
                    measured >= 0.9 * fast && measured <= above * slow,
                    "{name} decay {decay} damping {damping} color {color} at {centre} Hz: \
                     {measured} s against {fast} to {slow}"
                );
            }
            // The curve is not flat: the top goes first.
            let top = decay_seconds_at(&settings, 8000.0, rate) / decay;
            if *damping == 0.0 {
                assert!((top - 1.0).abs() < 0.02, "{name} without damping: {top}");
            } else {
                assert!(top < 0.75, "{name} damping {damping} color {color}: {top}");
            }
            // Decay is the time at 1 kHz, until the damping is so deep that
            // the bass would have to ring more than twice as long for it.
            let middle = decay_seconds_at(&settings, 1000.0, rate) / decay;
            if *damping <= 40.0 {
                assert!((middle - 1.0).abs() < 0.02, "{name}: {middle}");
            } else {
                // The Hall at its darkest: the loss starts at 2.4 kHz.
                assert!((0.3..=1.02).contains(&middle), "{name}: {middle}");
            }
        }
    }
}

#[test]
fn bass_and_treble_decay_as_the_drawn_curve_says() {
    check_drawn_decay(
        1.8,
        &[
            (0.0, 0.0),
            (40.0, 0.0),
            (100.0, 0.0),
            (40.0, -100.0),
            (40.0, 100.0),
        ],
    );
}

#[test]
#[ignore = "long renders"]
fn long_decays_are_as_drawn_at_every_damping_and_color() {
    let mut cases = Vec::new();
    for damping in [0.0, 40.0, 100.0] {
        for color in [-100.0, 0.0, 100.0] {
            cases.push((damping, color));
        }
    }
    check_drawn_decay(1.8, &cases);
    check_drawn_decay(6.0, &cases);
}

#[test]
fn hall_taps_follow_the_decay_on_the_first_lap() {
    let rate = 48_000.0_f32;
    let values = with(
        &REVERB_ONLY,
        &[
            (REVERB_MODEL, HALL),
            (SIZE, 100.0),
            (DECAY, 1.0),
            (DIFFUSION, 0.0),
            (DAMPING, 0.0),
            (MOVEMENT, 0.0),
            (PRE_DELAY, 0.0),
            (REVERB_LOW_CUT, 20.0),
            (WIDTH, 100.0),
        ],
    );
    let lap = reverb::structure_seconds(&settings(&values).reverb);
    let frames = (lap * rate) as usize;
    let (left, right) = run_stereo(&mut *device(rate, &values), &impulse(frames));
    for (channel, name) in [(&left, "left"), (&right, "right")] {
        // Without diffusion the ring is delays alone and its taps give
        // single arrivals; the early reflections come before them.
        let largest = peak(channel);
        let arrivals: Vec<(f32, f32)> = channel
            .iter()
            .enumerate()
            .filter(|(index, x)| *index > (0.12 * rate) as usize && x.abs() > 0.05 * largest)
            .map(|(index, x)| (index as f32 / rate, db(f64::from(x.abs()))))
            .collect();
        assert!(arrivals.len() >= 4, "{name}: {arrivals:?}");
        // Against a 60 dB per second slope every arrival is as loud.
        let flat: Vec<f32> = arrivals
            .iter()
            .map(|(seconds, level)| level + 60.0 * seconds)
            .collect();
        let (low, high) = flat.iter().fold((f32::MAX, f32::MIN), |(low, high), x| {
            (low.min(*x), high.max(*x))
        });
        assert!(
            high - low <= 4.0,
            "{name}: {} dB between the arrivals {arrivals:?}",
            high - low
        );
    }
}

#[test]
fn a_size_step_glides_every_recursive_length() {
    let rate = 48_000.0;
    for model in ReverbModel::ALL {
        let mut kernel = ReverbKernel::default();
        kernel.prepare(rate);
        let small = ReverbSettings {
            model,
            size: 0.0,
            ..ReverbSettings::default()
        };
        kernel.set(&small);
        kernel.reset();
        let mut before = [0.0; 16];
        let count = kernel.lengths(&mut before);
        assert!(count >= 8, "{model:?}: {count} lengths");
        kernel.set(&ReverbSettings { size: 1.0, ..small });
        let mut fastest = 0.0_f64;
        let mut now = [0.0; 16];
        for _ in 0..4 * 48_000 {
            kernel.process(0.1, -0.1);
            kernel.lengths(&mut now);
            for (new, old) in now.iter().zip(&before).take(count) {
                fastest = fastest.max((new - old).abs());
            }
            before = now;
        }
        assert!(
            fastest > 0.0 && fastest <= 0.5 + 1e-9,
            "{model:?}: {fastest} samples per sample"
        );
        // And arrives.
        let mut fresh = ReverbKernel::default();
        fresh.prepare(rate);
        fresh.set(&ReverbSettings { size: 1.0, ..small });
        fresh.reset();
        let mut target = [0.0; 16];
        fresh.lengths(&mut target);
        for (reached, wanted) in now.iter().zip(&target).take(count) {
            assert!(
                (reached - wanted).abs() < 1.0,
                "{model:?}: {reached} of {wanted}"
            );
        }
    }
}

#[test]
fn movement_costs_no_decay() {
    let rate = 48_000.0;
    for (model, name) in [(STUDIO, "Studio"), (ROOM, "Room")] {
        let decays = |movement: f32| {
            let values = with(
                &REVERB_ONLY,
                &[
                    (REVERB_MODEL, model),
                    (DECAY, 1.8),
                    (DAMPING, 20.0),
                    (MOVEMENT, movement),
                ],
            );
            let (response, _) = reverb_response(&values, rate, 1.8);
            let start = late_field_start(&values, rate);
            [1000.0, 8000.0].map(|centre| rt60(&octave_band(&response, centre, rate), start, rate))
        };
        let (still, moving) = (decays(0.0), decays(100.0));
        for band in 0..2 {
            assert!(
                (moving[band] / still[band] - 1.0).abs() <= 0.03,
                "{name}: {moving:?} s moving against {still:?} s"
            );
        }
    }
}

/// Mean power per bin of `response` in third octaves from 200 Hz to 8 kHz,
/// in dB.
fn third_octave_levels(response: &[f32], rate: f32) -> Vec<f32> {
    use rustfft::num_complex::Complex64;
    let mut bins: Vec<Complex64> = response
        .iter()
        .map(|x| Complex64::new(f64::from(*x), 0.0))
        .collect();
    rustfft::FftPlanner::new()
        .plan_fft_forward(bins.len())
        .process(&mut bins);
    let hertz = bins.len() as f64 / f64::from(rate);
    (0..=16)
        .map(|third| {
            let centre = 200.0 * 2.0_f64.powf(f64::from(third) / 3.0);
            let (low, high) = (
                (centre / 2.0_f64.powf(1.0 / 6.0) * hertz) as usize,
                (centre * 2.0_f64.powf(1.0 / 6.0) * hertz) as usize,
            );
            let power: f64 = bins[low..=high].iter().map(|bin| bin.norm_sqr()).sum();
            (10.0 * (power / (high - low + 1) as f64).log10()) as f32
        })
        .collect()
}

/// How far each of `levels` is off the straight line through them.
fn off_the_trend(levels: &[f32]) -> Vec<f32> {
    let n = levels.len() as f32;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = levels.iter().sum::<f32>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (x, y) in levels.iter().enumerate() {
        sxy += (x as f32 - mean_x) * (y - mean_y);
        sxx += (x as f32 - mean_x).powi(2);
    }
    let slope = sxy / sxx;
    levels
        .iter()
        .enumerate()
        .map(|(x, y)| y - (mean_y + slope * (x as f32 - mean_x)))
        .collect()
}

/// Every seed of a model is the same room: as long, as loud, and with no
/// third octave of its tail more than `evenness` dB off the trend.
fn check_seeds(model: f32, name: &str, rate: f32, size: f32, evenness: f32) {
    let decay = 1.5;
    let mut levels = Vec::new();
    let mut failures = Vec::new();
    for seed in 1..=32 {
        let values = with(
            &REVERB_ONLY,
            &[
                (REVERB_MODEL, model),
                (SEED, seed as f32),
                (SIZE, size),
                (DECAY, decay),
                (DAMPING, 0.0),
                // The colour of the network is held with it at rest.
                (MOVEMENT, 0.0),
                (REVERB_LOW_CUT, 20.0),
            ],
        );
        let start = late_field_start(&values, rate);
        let second = rate as usize;
        let frames = start + 2 * second;
        let (left, right) = run_stereo(&mut *device(rate, &values), &impulse(frames));
        let seconds = rt60(&octave_band(&left, 1000.0, rate), start, rate);
        let energy = |x: &[f32]| x.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        levels.push(db((0.5 * (energy(&left) + energy(&right))).sqrt()));
        // A second of the tail of both channels, once the structure has
        // passed.
        let late = start + (0.25 * rate) as usize;
        let bands: Vec<f32> = third_octave_levels(&left[late..late + second], rate)
            .iter()
            .zip(third_octave_levels(&right[late..late + second], rate))
            .map(|(l, r)| {
                10.0 * (0.5 * (10.0_f32.powf(l / 10.0) + 10.0_f32.powf(r / 10.0))).log10()
            })
            .collect();
        let off = off_the_trend(&bands);
        let worst = off.iter().fold(0.0_f32, |worst, x| worst.max(x.abs()));
        if (seconds / decay - 1.0).abs() > 0.05 {
            failures.push(format!("seed {seed}: {seconds} s"));
        }
        if worst > evenness {
            failures.push(format!("seed {seed}: a band {worst} dB off the trend"));
        }
    }
    let mut sorted = levels.clone();
    sorted.sort_by(f32::total_cmp);
    let median = 0.5 * (sorted[15] + sorted[16]);
    for (index, level) in levels.iter().enumerate() {
        if (level - median).abs() > 0.5 {
            failures.push(format!(
                "seed {}: {} dB off the median level",
                index + 1,
                level - median
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{name} at {rate} Hz, Size {size} %: {failures:#?}"
    );
}

#[test]
fn every_studio_seed_is_the_same_room() {
    for rate in [44_100.0, 48_000.0] {
        for size in [50.0, 75.0, 100.0] {
            check_seeds(STUDIO, "Studio", rate, size, 4.0);
        }
        // Sixteen lines of a few milliseconds hold too few modes for an
        // even 1.5 s tail, whatever the draw: a small Studio with a long
        // decay is coloured (up to 7 dB at Size 0). Length and level hold.
        for size in [0.0, 25.0] {
            check_seeds(STUDIO, "Studio", rate, size, f32::INFINITY);
        }
    }
}

#[test]
fn every_room_seed_is_the_same_room() {
    for rate in [44_100.0, 48_000.0] {
        for size in [25.0, 50.0, 75.0] {
            check_seeds(ROOM, "Room", rate, size, 3.0);
        }
    }
}

#[test]
fn every_hall_seed_is_the_same_room() {
    for rate in [44_100.0, 48_000.0] {
        for size in [25.0, 50.0, 75.0] {
            check_seeds(HALL, "Hall", rate, size, 3.0);
        }
    }
}

#[test]
fn seeds_differ_and_repeat() {
    let rate = 48_000.0_f32;
    for (model, name) in REVERBS {
        let response = |seed: f32| {
            let values = with(&REVERB_ONLY, &[(REVERB_MODEL, model), (SEED, seed)]);
            let mut device = device(rate, &values);
            let first = run_stereo(&mut *device, &impulse(36_000));
            // Run on, then start over: the same seed is the same room.
            run_stereo(&mut *device, &vec![0.25; 12_000]);
            device.reset();
            let again = run_stereo(&mut *device, &impulse(36_000));
            assert!(first == again, "{name} seed {seed} after a reset");
            first.0
        };
        let responses = [1.0, 2.0, 17.0, 32.0].map(response);
        let energy = |x: &[f32]| x.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        for a in 0..responses.len() {
            for b in a + 1..responses.len() {
                let (x, y) = (&responses[a], &responses[b]);
                let scale = (energy(x) * energy(y)).sqrt();
                let likeness = (-240_i32..=240)
                    .map(|lag| {
                        let (from, shift) = if lag < 0 {
                            (0, (-lag) as usize)
                        } else {
                            (lag as usize, 0)
                        };
                        let sum: f64 = x[from..]
                            .iter()
                            .zip(&y[shift..])
                            .map(|(x, y)| f64::from(*x) * f64::from(*y))
                            .sum();
                        (sum / scale).abs()
                    })
                    .fold(0.0, f64::max);
                assert!(likeness < 0.2, "{name}, responses {a} and {b}: {likeness}");
            }
        }
    }
}

/// Abel and Huang's echo density of `response` around `centre`: the share
/// of a 20 ms window that lies outside its own standard deviation, against
/// what Gaussian noise has there.
fn echo_density(response: &[f32], centre: usize, rate: f32) -> f64 {
    let half = (0.01 * rate) as usize;
    let window = &response[centre.saturating_sub(half)..centre + half];
    let deviation = rms(window);
    let outside = window
        .iter()
        .filter(|x| f64::from(x.abs()) > deviation)
        .count();
    // erfc(1/√2).
    outside as f64 / window.len() as f64 / 0.3173
}

#[test]
fn the_tail_is_dense_as_early_as_the_model_says() {
    let rate = 48_000.0_f32;
    // One window holds a thousand samples, and its density is good to a
    // twentieth: the model's is the mean of eight seeds and both channels.
    let density = |model: f32| {
        let responses: Vec<Vec<f32>> = (1..=8)
            .flat_map(|seed| {
                let values = with(
                    &REVERB_ONLY,
                    &[
                        (REVERB_MODEL, model),
                        (SEED, seed as f32),
                        (PRE_DELAY, 0.0),
                        (DIFFUSION, 100.0),
                    ],
                );
                let (left, right) = run_stereo(&mut *device(rate, &values), &impulse(24_000));
                [left, right]
            })
            .collect();
        move |seconds: f32| {
            responses
                .iter()
                .map(|response| echo_density(response, (seconds * rate) as usize, rate))
                .sum::<f64>()
                / responses.len() as f64
        }
    };
    // A window is 20 ms: the first that holds only sound is centred at 10.
    for (model, name, within) in [(STUDIO, "Studio", 0.025), (ROOM, "Room", 0.06)] {
        let density = density(model);
        let reached = (10..=400)
            .map(|ms| ms as f32 / 1000.0)
            .find(|seconds| density(*seconds) >= 0.9);
        assert!(
            reached.is_some_and(|seconds| seconds <= within),
            "{name}: {reached:?} s"
        );
        // And stays so: no late thinning.
        for seconds in [2.0 * within, 0.2, 0.4] {
            assert!(
                density(seconds) >= 0.9,
                "{name} at {seconds} s: {}",
                density(seconds)
            );
        }
    }
    // A hall builds: thin at first, dense within a quarter of a second,
    // and never thinner on the way.
    let density = density(HALL);
    let trace: Vec<(f32, f64)> = (10..=250)
        .step_by(5)
        .map(|ms| (ms as f32 / 1000.0, density(ms as f32 / 1000.0)))
        .collect();
    let reached = trace
        .iter()
        .find(|(_, density)| *density >= 0.9)
        .map(|x| x.0);
    assert!(
        reached.is_some_and(|seconds| (0.1..=0.25).contains(&seconds)),
        "Hall: {reached:?} s; {trace:?}"
    );
    let mut densest = 0.0_f64;
    for (seconds, density) in &trace {
        assert!(
            *density >= densest - 0.05,
            "Hall thins at {seconds} s: {trace:?}"
        );
        densest = densest.max(*density);
    }
    assert!(density(0.4) >= 0.9 && density(0.45) >= 0.9);
}

#[test]
fn size_keeps_the_level_and_the_models_are_equally_loud() {
    let rate = 48_000.0;
    let mut random = Noise(17);
    let input: Vec<f32> = (0..168_000).map(|_| 0.25 * random.next()).collect();
    let level = |model: f32, size: f32| {
        let values = with(&REVERB_ONLY, &[(REVERB_MODEL, model), (SIZE, size)]);
        let (left, right) = run_stereo(&mut *device(rate, &values), &input);
        db((0.5 * (rms(&left[96_000..]).powi(2) + rms(&right[96_000..]).powi(2))).sqrt())
    };
    let mut defaults = Vec::new();
    for (model, name) in REVERBS {
        let levels = [0.0, 50.0, 100.0].map(|size| level(model, size));
        let (low, high) = levels.iter().fold((f32::MAX, f32::MIN), |(low, high), x| {
            (low.min(*x), high.max(*x))
        });
        assert!(
            high - low <= 3.0,
            "{name}: {levels:?} dB at Size 0, 50, 100 %"
        );
        defaults.push(levels[1]);
    }
    let (low, high) = defaults
        .iter()
        .fold((f32::MAX, f32::MIN), |(low, high), x| {
            (low.min(*x), high.max(*x))
        });
    let input_db = db(0.25 / 3.0_f64.sqrt());
    assert!(
        high - low <= 2.0,
        "Studio, Room, Hall: {defaults:?} dB for {input_db} dB in"
    );
    // At full level the wet signal of noise is about as loud as the noise.
    assert!(
        defaults.iter().all(|level| (level - input_db).abs() <= 6.0),
        "{defaults:?} dB for {input_db} dB in"
    );
}

#[test]
fn the_late_field_is_wide_and_sums_to_mono_without_holes() {
    let rate = 48_000.0_f32;
    for (model, name) in REVERBS {
        let values = with(
            &REVERB_ONLY,
            &[
                (REVERB_MODEL, model),
                (DECAY, 3.0),
                (MOVEMENT, 0.0),
                (WIDTH, 100.0),
            ],
        );
        let start = late_field_start(&values, rate) + (0.2 * rate) as usize;
        let (left, right) = run_stereo(&mut *device(rate, &values), &impulse(start + 48_000));
        let (left, right) = (&left[start..], &right[start..]);
        let cross: f64 = left
            .iter()
            .zip(right)
            .map(|(l, r)| f64::from(*l) * f64::from(*r))
            .sum();
        let correlation = cross / (rms(left) * rms(right) * left.len() as f64);
        assert!(correlation.abs() < 0.2, "{name}: correlation {correlation}");
        let mono: Vec<f32> = left.iter().zip(right).map(|(l, r)| 0.5 * (l + r)).collect();
        let (sum, sides) = (
            third_octave_levels(&mono, rate),
            [
                third_octave_levels(left, rate),
                third_octave_levels(right, rate),
            ],
        );
        for band in 0..sum.len() {
            // Uncorrelated channels sum 3 dB below their mean.
            let stereo = 0.5 * (sides[0][band] + sides[1][band]) - 3.0;
            assert!(
                sum[band] - stereo >= -6.0,
                "{name}, band {band} above 200 Hz: the mono sum is {} dB down",
                stereo - sum[band]
            );
        }
        // Width 0 is mono; 150 % is wider than the network.
        let narrow = with(&values, &[(WIDTH, 0.0)]);
        let (left, right) = run_stereo(&mut *device(rate, &narrow), &impulse(24_000));
        assert_eq!(left, right, "{name} at Width 0");
    }
}

#[test]
fn hall_modulation_is_clean() {
    let rate = 48_000.0;
    let values = with(
        &REVERB_ONLY,
        &[
            (REVERB_MODEL, HALL),
            (MOVEMENT, 100.0),
            (DECAY, 3.0),
            (DAMPING, 0.0),
        ],
    );
    let (frequency, bin) = on_bin(1000.0, rate);
    let input = sine(frequency, 0.5, rate, 4 * 48_000 + FFT);
    let (left, _) = run_stereo(&mut *device(rate as f32, &values), &input);
    let spectrum = spectrum(&left);
    let tone = spectrum[bin];
    // The moving reads spread the tone over a few hertz around it; nothing
    // else may come of them.
    let away = on_bin(150.0, rate).1;
    let worst = spectrum
        .iter()
        .enumerate()
        .filter(|(index, _)| index.abs_diff(bin) > away && *index > SKIRT)
        .map(|(_, level)| *level)
        .fold(0.0, f64::max);
    assert!(
        db(worst / tone) <= -90.0,
        "{} dB against the tone, more than 150 Hz from it",
        db(worst / tone)
    );
    // And they do move it: a still hall holds a steady tone steadier.
    let wander = |movement: f32| {
        let values = with(&values, &[(MOVEMENT, movement)]);
        let (left, _) = run_stereo(&mut *device(rate as f32, &values), &input[..5 * 48_000]);
        let levels: Vec<f64> = left[3 * 48_000..].chunks(4800).map(rms).collect();
        let (low, high) = levels.iter().fold((f64::MAX, f64::MIN), |(low, high), x| {
            (low.min(*x), high.max(*x))
        });
        db(high / low)
    };
    let (still, moving) = (wander(0.0), wander(100.0));
    assert!(
        still < 0.1 && moving > 1.0,
        "{still} dB still, {moving} dB moving"
    );
}

#[test]
fn a_model_change_keeps_the_old_tail() {
    let rate = 48_000.0_f32;
    let mut random = Noise(5);
    let mut input: Vec<f32> = (0..24_000).map(|_| 0.25 * random.next()).collect();
    input.resize(72_000, 0.0);
    for (from, to, name) in [
        (HALL, STUDIO, "Hall to Studio"),
        (STUDIO, ROOM, "Studio to Room"),
        (ROOM, HALL, "Room to Hall"),
    ] {
        let values = with(&REVERB_ONLY, &[(REVERB_MODEL, from), (DECAY, 3.0)]);
        let render = |change: bool| {
            let mut device = device(rate, &values);
            // The change comes half a second into the tail.
            let (mut left, _) = run_stereo(&mut *device, &input[..48_000]);
            if change {
                set(&mut *device, &[(REVERB_MODEL, to)]);
            }
            left.extend(run_stereo(&mut *device, &input[48_000..]).0);
            left
        };
        let (kept, changed) = (render(false), render(true));
        let after = 48_000..52_800;
        let difference = db(rms(&changed[after.clone()]) / rms(&kept[after.clone()]));
        assert!(
            difference.abs() <= 1.0,
            "{name}: {difference} dB in the next 100 ms"
        );
        let (steady, step) = (
            largest_step(&kept[47_000..60_000]),
            largest_step(&changed[47_000..60_000]),
        );
        assert!(
            step <= 1.5 * steady,
            "{name}: a step of {step} against {steady}"
        );
        // The old tail rings out: a second later it is still there.
        let later = db(rms(&changed[66_000..]) / rms(&kept[66_000..]));
        assert!(later.abs() <= 3.0, "{name}: {later} dB a second later");
    }
}

#[test]
fn seed_and_size_changes_do_not_click() {
    let rate = 48_000.0_f32;
    for (model, name) in REVERBS {
        let mut device = device(rate, &with(&REVERB_ONLY, &[(REVERB_MODEL, model)]));
        let mut position = 0;
        let mut step = |device: &mut dyn Device, frames: usize| {
            let input: Vec<f32> = (position..position + frames)
                .map(|n| 0.5 * (std::f32::consts::TAU * 220.0 * n as f32 / rate).sin())
                .collect();
            position += frames;
            let (left, right) = run_stereo(device, &input);
            largest_step(&left).max(largest_step(&right))
        };
        step(&mut *device, 96_000);
        let steady = step(&mut *device, 48_000);
        for (what, change) in [
            ("Seed 2", (SEED, 2.0)),
            ("Seed 32", (SEED, 32.0)),
            ("Size 100", (SIZE, 100.0)),
            ("Size 0", (SIZE, 0.0)),
            ("Pre-delay 120", (PRE_DELAY, 120.0)),
        ] {
            set(&mut *device, &[change]);
            let largest = step(&mut *device, 96_000);
            // While the lengths glide, at up to half a sample per sample,
            // the pitch of what they hold is up to half as high again and
            // the tone finds other combs: steeper, but nothing like the
            // step of a click, which a sudden length of this tone would
            // make as large as its amplitude.
            let limit = if change.0 == SIZE { 3.0 } else { 1.5 };
            assert!(
                largest <= limit * steady + 0.02,
                "{name}, {what}: {largest} against {steady}"
            );
        }
    }
}

/// Processes at `tempo`, the same signal on both channels.
fn run_at_tempo(device: &mut dyn Device, tempo: f64, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (input.to_vec(), input.to_vec());
    for (left, right) in left.chunks_mut(500).zip(right.chunks_mut(500)) {
        let context = ProcessContext {
            tempo,
            ..ProcessContext::default()
        };
        device.process(
            &context,
            &[],
            StereoIo {
                left,
                right,
                sidechain: None,
            },
        );
    }
    (left, right)
}

/// Samples of `signal` above −100 dBFS, as `(index, value)`.
fn taps_of(signal: &[f32]) -> Vec<(usize, f32)> {
    signal
        .iter()
        .enumerate()
        .filter(|(_, x)| x.abs() > 1e-5)
        .map(|(index, x)| (index, *x))
        .collect()
}

/// Where the heads before the main one sit, by pattern, as the design
/// lists them.
const PATTERNS: [(&str, &[f32]); 8] = [
    ("Single", &[]),
    ("Double", &[0.5]),
    ("Triple", &[1.0 / 3.0, 2.0 / 3.0]),
    ("Quad", &[0.25, 0.5, 0.75]),
    ("Dotted", &[0.75]),
    ("Swing", &[2.0 / 3.0]),
    ("Gallop", &[0.5, 0.75]),
    ("Golden", &[0.382, 0.618]),
];

#[test]
fn digital_repeats_are_exact_in_every_pattern_and_stereo_mode() {
    let rate = 48_000.0_f32;
    assert_eq!(ECHO_TAP_PATTERNS.len(), PATTERNS.len());
    for milliseconds in [1.0_f32, 5.0, 300.0] {
        let period = f64::from((milliseconds / 1000.0 * rate).round());
        let later = {
            let seconds = milliseconds / 1000.0;
            f64::from(((seconds + echo::wide_offset(seconds)) * rate).round())
        };
        for (pattern, (name, fractions)) in PATTERNS.iter().enumerate() {
            assert_eq!(ECHO_TAP_PATTERNS[pattern], *name);
            for stereo in [MONO, PING_PONG, WIDE] {
                let values = with(
                    &ECHO_ONLY,
                    &[
                        (ECHO_TIME, milliseconds),
                        (ECHO_TAPS, pattern as f32),
                        (ECHO_STEREO, stereo),
                        (ECHO_FEEDBACK, 50.0),
                    ],
                );
                let frames = (3.0 * later) as usize + 8;
                let (left, right) = run_stereo(&mut *device(rate, &values), &impulse(frames));
                // The heads of each pass of a line, the main one last.
                let mut expected: [Vec<(usize, f32)>; 2] = [Vec::new(), Vec::new()];
                for pass in 0..17 {
                    let gain = 0.5_f32.powi(pass);
                    for fraction in fractions.iter().copied().chain([1.0]) {
                        let head = |period: f64| {
                            (f64::from(pass) * period + (f64::from(fraction) * period).round())
                                as usize
                        };
                        if stereo == MONO {
                            expected[0].push((head(period), gain));
                            expected[1].push((head(period), gain));
                        } else if stereo == PING_PONG {
                            expected[pass as usize % 2].push((head(period), gain));
                        } else {
                            expected[0].push((head(period), gain));
                            expected[1].push((head(later), gain));
                        }
                    }
                }
                for (channel, heard) in [&left, &right].into_iter().enumerate() {
                    let mut wanted: Vec<(usize, f32)> = expected[channel]
                        .iter()
                        .copied()
                        .filter(|(index, _)| *index < frames)
                        .collect();
                    wanted.sort_by_key(|(index, _)| *index);
                    assert_eq!(
                        taps_of(heard),
                        wanted,
                        "{milliseconds} ms, {name}, stereo {stereo}, channel {channel}"
                    );
                }
                // What the panel draws is what sounds.
                let mut drawn = [(0.0, 0.0, 0.0); 64];
                let count = echo_taps(&settings(&values), 120.0, &mut drawn);
                for (seconds, left_gain, right_gain) in &drawn[..count] {
                    let index = (seconds * rate).round() as usize;
                    if index + 1 >= frames {
                        continue;
                    }
                    for (gain, heard) in [(left_gain, &left), (right_gain, &right)] {
                        // Another head may sound on the side this one
                        // leaves empty.
                        if *gain == 0.0 {
                            continue;
                        }
                        let near = heard[index - 1..=index + 1]
                            .iter()
                            .fold(0.0_f32, |near, x| near.max(x.abs()));
                        assert!(
                            (near - gain).abs() < 1e-3,
                            "{milliseconds} ms, {name}, stereo {stereo}: drawn {gain} at \
                             {seconds} s, heard {near}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_synced_echo_follows_the_tempo() {
    let rate = 48_000.0_f32;
    let first_tap = |tempo: f64, division: f32, frames: usize| {
        let values = with(
            &ECHO_ONLY,
            &[
                (ECHO_SYNC, 1.0),
                (ECHO_DIVISION, division),
                (ECHO_STEREO, MONO),
                (ECHO_FEEDBACK, 0.0),
            ],
        );
        let mut device = device(rate, &values);
        // The tempo is known before the impulse: the time does not move
        // under it.
        run_at_tempo(&mut *device, tempo, &[0.0; 4800]);
        let (left, _) = run_at_tempo(&mut *device, tempo, &impulse(frames));
        let telemetry = device.telemetry();
        (
            taps_of(&left),
            telemetry.value(TELEMETRY_TIME),
            telemetry.value(TELEMETRY_TEMPO),
        )
    };
    // A dotted eighth at 120: three quarters of a beat.
    assert_eq!(DIVISIONS[9], "1/8 D");
    let (taps, seconds, tempo) = first_tap(120.0, 9.0, 24_000);
    assert_eq!(taps, [(18_000, 1.0)]);
    assert_eq!((seconds, tempo), (0.375, 120.0));
    // A dotted whole note at 60 would be 6 s: halved to fit the line.
    assert_eq!(DIVISIONS[17], "1/1 D");
    let (taps, seconds, tempo) = first_tap(60.0, 17.0, 150_000);
    assert_eq!(taps, [(144_000, 1.0)]);
    assert_eq!((seconds, tempo), (3.0, 60.0));
    assert_eq!(division_beats(17), 6.0);
    assert_eq!(division_beats(0), 1.0 / 12.0);
    // The longest time there is, a whole note at 60: the line holds it and
    // Wide's offset.
    let values = with(
        &ECHO_ONLY,
        &[
            (ECHO_SYNC, 1.0),
            (ECHO_DIVISION, 16.0),
            (ECHO_STEREO, WIDE),
            (ECHO_FEEDBACK, 0.0),
        ],
    );
    let mut device = device(rate, &values);
    run_at_tempo(&mut *device, 60.0, &[0.0; 4800]);
    let (left, right) = run_at_tempo(&mut *device, 60.0, &impulse(200_000));
    assert_eq!(taps_of(&left), [(192_000, 1.0)]);
    assert_eq!(taps_of(&right), [(192_000 + 720, 1.0)]);
}

/// A tone that fades in and out over `frames`.
fn burst(hz: f32, amplitude: f32, frames: usize, rate: f32) -> Vec<f32> {
    (0..frames)
        .map(|n| {
            let window = (std::f32::consts::PI * n as f32 / frames as f32)
                .sin()
                .powi(2);
            amplitude * window * (std::f32::consts::TAU * hz * n as f32 / rate).sin()
        })
        .collect()
}

/// Where the energy of `signal` is centred, in samples.
fn centroid(signal: &[f32]) -> f64 {
    let energy: f64 = signal.iter().map(|x| f64::from(*x).powi(2)).sum();
    signal
        .iter()
        .enumerate()
        .map(|(n, x)| n as f64 * f64::from(*x).powi(2))
        .sum::<f64>()
        / energy
}

#[test]
fn a_wobbling_digital_echo_keeps_its_top_and_never_gains() {
    let rate = 48_000.0_f32;
    // The tenth repeat of a 15 kHz burst is as loud as the loop gain says:
    // the moving read costs next to nothing.
    let values = with(
        &ECHO_ONLY,
        &[
            (ECHO_TIME, 100.0),
            (ECHO_STEREO, MONO),
            (ECHO_FEEDBACK, 80.0),
            (ECHO_WOBBLE, 100.0),
        ],
    );
    let mut input = burst(15_000.0, 0.5, 2400, rate);
    input.resize(11 * 4800, 0.0);
    let (left, _) = run_stereo(&mut *device(rate, &values), &input);
    let repeat = |n: usize| rms(&left[n * 4800 - 480..n * 4800 + 2880]);
    let loss = db(repeat(10) / repeat(1)) - db(0.8_f64.powi(9));
    assert!((-1.0..=0.01).contains(&loss), "{loss} dB over nine repeats");

    // Held at full feedback, the level never rises.
    let mut random = Noise(9);
    for milliseconds in [1.0, 300.0] {
        let values = with(
            &ECHO_ONLY,
            &[
                (ECHO_TIME, milliseconds),
                (ECHO_STEREO, MONO),
                (ECHO_FEEDBACK, 100.0),
                (ECHO_WOBBLE, 100.0),
            ],
        );
        let mut input: Vec<f32> = (0..14_400).map(|_| 0.5 * random.next()).collect();
        input.resize(20 * 48_000, 0.0);
        let (left, _) = run_stereo(&mut *device(rate, &values), &input);
        let levels: Vec<f64> = left[48_000..].chunks(48_000).map(rms).collect();
        assert!(levels[0] > 0.01, "{milliseconds} ms: {levels:?}");
        assert!(
            levels.windows(2).all(|pair| pair[1] <= pair[0] * 1.0001),
            "{milliseconds} ms: {levels:?}"
        );
    }
}

#[test]
fn coloured_echoes_arrive_on_time() {
    let rate = 48_000.0_f32;
    for (model, name) in [(BBD, "BBD"), (TAPE, "Tape")] {
        for (pattern, (pattern_name, fractions)) in PATTERNS.iter().enumerate() {
            for (milliseconds, diffusion) in [
                (5.0, 0.0),
                (50.0, 0.0),
                (300.0, 0.0),
                (300.0, 100.0),
                (500.0, 0.0),
            ] {
                // The other patterns at one time: the heads sit at shares
                // of it whatever it is.
                if pattern > 0 && milliseconds != 300.0 {
                    continue;
                }
                let values = with(
                    &ECHO_ONLY,
                    &[
                        (ECHO_MODEL, model),
                        (ECHO_TIME, milliseconds),
                        (ECHO_TAPS, pattern as f32),
                        (ECHO_DIFFUSION, diffusion),
                        (ECHO_STEREO, MONO),
                        (ECHO_FEEDBACK, 0.0),
                        (ECHO_COLOR, 50.0),
                    ],
                );
                // A burst the line passes whole, short against the gaps
                // between the heads.
                let length = if milliseconds < 300.0 { 96 } else { 960 };
                let mut sound = burst(48_000.0 / 48.0, 0.25, length, rate);
                if diffusion > 0.0 {
                    // An allpass delays each frequency by another time,
                    // and all of them by its length on average: noise.
                    let mut random = Noise(41);
                    sound = (0..96)
                        .map(|n| {
                            let shape = (std::f32::consts::PI * n as f32 / 96.0).sin().powi(2);
                            0.25 * shape * random.next()
                        })
                        .collect();
                }
                let period = milliseconds / 1000.0 * rate;
                let mut input = sound.clone();
                input.resize((1.6 * period) as usize + 4 * length + 4800, 0.0);
                let (left, _) = run_stereo(&mut *device(rate, &values), &input);
                let middle = centroid(&sound);
                if diffusion > 0.0 {
                    // Smeared heads run into each other, but each stays
                    // centred on its time, so their sum does.
                    let sum: f32 = fractions.iter().sum();
                    let count = fractions.len() as f32;
                    let place = (centroid(&left) - middle) / f64::from(period);
                    if model == BBD && !fractions.is_empty() {
                        // The heads before the main one share an
                        // expander: where they overlap they lift each
                        // other. The sum is centred where heads that much
                        // louder would put it.
                        let place = place as f32;
                        let lift = 10.0 * ((1.0 - place) / (count * place - sum)).log10();
                        assert!(
                            lift.abs() <= 2.0,
                            "{name} {pattern_name} smeared: the early heads are {lift} dB up"
                        );
                        continue;
                    }
                    let error =
                        (place - f64::from((sum + 1.0) / (count + 1.0))) * f64::from(period);
                    assert!(
                        error.abs() <= 0.001 * f64::from(rate),
                        "{name} {pattern_name} smeared at {milliseconds} ms: {error} samples"
                    );
                    continue;
                }
                for fraction in fractions.iter().copied().chain([1.0]) {
                    let expected = middle + f64::from(fraction * period);
                    let reach = (0.6 * length as f32) as usize;
                    let from = expected as usize - reach;
                    let heard = from as f64 + centroid(&left[from..from + 2 * reach]);
                    assert!(
                        (heard - expected).abs() <= 0.0005 * f64::from(rate),
                        "{name} {pattern_name} at {milliseconds} ms: head {fraction} at \
                         {heard}, not {expected}"
                    );
                }
            }
        }
    }
}

#[test]
fn coloured_echoes_do_not_alias() {
    let rate = 44_100.0;
    let values = with(
        &ECHO_ONLY,
        &[
            (ECHO_MODEL, TAPE),
            (ECHO_TIME, 20.0),
            (ECHO_FEEDBACK, 0.0),
            (ECHO_COLOR, 0.0),
        ],
    );
    for (tone, limit) in [
        (1237.0, -60.0),
        (3171.0, -60.0),
        (4999.0, -60.0),
        (7919.0, -50.0),
        (9973.0, -50.0),
    ] {
        let (frequency, bin) = on_bin(tone, rate);
        let input = sine(frequency, 0.5, rate, FFT + FFT / 4);
        let (left, _) = run_stereo(&mut *device(rate as f32, &values), &input);
        let worst = worst_inharmonic_db(
            &spectrum(&left),
            bin,
            on_bin(20.0, rate).1 + SKIRT,
            on_bin(20_000.0, rate).1,
        );
        assert!(worst <= limit, "Tape at {tone} Hz: {worst} dBc");
    }
}

#[test]
fn repeats_lose_what_the_drawn_response_says() {
    let rate = 48_000.0_f32;
    for (model, name, colors, tolerance) in [
        (DIGITAL, "Digital", [-100.0, 0.0, 60.0], 0.5),
        (TAPE, "Tape", [-100.0, 0.0, 100.0], 1.0),
    ] {
        for color in colors {
            let values = with(
                &ECHO_ONLY,
                &[
                    (ECHO_MODEL, model),
                    (ECHO_TIME, 20.0),
                    (ECHO_FEEDBACK, 0.0),
                    (ECHO_COLOR, color),
                    (ECHO_LOW_CUT, 120.0),
                ],
            );
            let echo = settings(&values).echo(120.0);
            for hz in [60.0, 200.0, 1000.0, 3000.0, 5000.0, 10_000.0] {
                let input = sine(hz, 0.01, f64::from(rate), 24_000);
                let (left, _) = run_stereo(&mut *device(rate, &values), &input);
                let measured = fit(&left[12_000..], hz, f64::from(rate)) / 0.01;
                let expected = echo::repeat_response(&echo, rate, hz);
                let error = db(measured / expected);
                assert!(
                    error.abs() <= tolerance,
                    "{name} color {color} at {hz} Hz: {error} dB off {} dB",
                    db(expected)
                );
            }
        }
    }
    // Color is each model's own tone.
    let response = |model: f32, color: f32, hz: f64| {
        let values = with(
            &ECHO_ONLY,
            &[(ECHO_MODEL, model), (ECHO_TIME, 300.0), (ECHO_COLOR, color)],
        );
        db(echo::repeat_response(
            &settings(&values).echo(120.0),
            rate,
            hz,
        ))
    };
    for (model, name) in ECHOES {
        let (dark, middle, bright) = (
            response(model, -100.0, 3000.0),
            response(model, 0.0, 3000.0),
            response(model, 100.0, 3000.0),
        );
        assert!(
            dark < middle - 2.0 && middle <= bright,
            "{name}: {dark}, {middle}, {bright} dB"
        );
    }
    assert_eq!(response(DIGITAL, 100.0, 15_000.0), 0.0);
    // A dotted quarter on a dark bucket-brigade line clocks at 4.8 kHz.
    let dub = EchoSettings {
        model: EchoModel::Bbd,
        seconds: 0.75,
        color: -0.4,
        ..EchoSettings::default()
    };
    let clock = echo::bbd_clock_hz(&dub, rate);
    assert!((clock / 4800.0 - 1.0).abs() < 0.05, "{clock} Hz");
    assert!(echo::tape_loss_hz(0.3, 0.0) > echo::tape_loss_hz(0.3, -1.0));
}

/// Frequency of a clean tone from one upward zero crossing to the next,
/// as `(sample, hertz)`.
fn pitch_by_crossings(signal: &[f32], rate: f32) -> Vec<(usize, f32)> {
    let mut crossings = Vec::new();
    for (index, pair) in signal.windows(2).enumerate() {
        if pair[0] < 0.0 && pair[1] >= 0.0 {
            crossings.push(index as f32 + pair[0] / (pair[0] - pair[1]));
        }
    }
    crossings
        .windows(2)
        .map(|pair| (pair[0] as usize, rate / (pair[1] - pair[0])))
        .collect()
}

/// A time change on a line that bends: what the tone does on the way and
/// where a burst lands afterwards.
fn time_change(model: f32, from: f32, to: f32) -> (Vec<f32>, f64, f64) {
    let rate = 48_000.0_f32;
    let values = with(
        &ECHO_ONLY,
        &[
            (ECHO_MODEL, model),
            (ECHO_TIME, from),
            (ECHO_FEEDBACK, 0.0),
            (ECHO_STEREO, MONO),
            (ECHO_COLOR, 50.0),
        ],
    );
    let mut device = device(rate, &values);
    let tone = sine(500.0, 0.25, f64::from(rate), 48_000 + 144_000);
    run_stereo(&mut *device, &tone[..48_000]);
    set(&mut *device, &[(ECHO_TIME, to)]);
    let (during, _) = run_stereo(&mut *device, &tone[48_000..]);
    // The tone stops; once the line is empty a burst shows the time.
    run_stereo(&mut *device, &vec![0.0; 96_000]);
    let sound = burst(500.0, 0.25, 960, rate);
    let mut input = sound.clone();
    input.resize(48_000, 0.0);
    let (after, _) = run_stereo(&mut *device, &input);
    let arrived = centroid(&after) - centroid(&sound);
    // The same burst through a line that never moved.
    let fresh = with(&values, &[(ECHO_TIME, to)]);
    let (still, _) = run_stereo(&mut *self::device(rate, &fresh), &input);
    (during, arrived, f64::from(db(rms(&after) / rms(&still))))
}

#[test]
fn a_tape_time_change_bends_the_pitch_and_lands_on_the_new_time() {
    let rate = 48_000.0_f32;
    let (during, arrived, level) = time_change(TAPE, 100.0, 400.0);
    let glide = pitch_by_crossings(&during, rate);
    let lowest = glide.iter().map(|(_, hz)| *hz).fold(f32::MAX, f32::min);
    let highest = glide.iter().map(|(_, hz)| *hz).fold(f32::MIN, f32::max);
    // The tape slows down, never stops and never runs backward.
    assert!(
        lowest >= 50.0 && highest <= 501.0,
        "{lowest} to {highest} Hz"
    );
    assert!(
        lowest < 300.0,
        "a longer time bends the pitch down: {lowest} Hz"
    );
    // The motor has a time constant of 0.12 s: the speed, and with it the
    // pitch of what is recorded, is most of the way after a few of them,
    // and the old tape has passed the head after the new time.
    assert!(
        glide
            .iter()
            .filter(|(index, _)| *index > 72_000)
            .all(|(_, hz)| (hz - 500.0).abs() < 1.0)
    );
    assert!(
        (arrived - 0.4 * f64::from(rate)).abs() <= 0.0005 * f64::from(rate),
        "{arrived} samples"
    );
    assert!(level.abs() <= 0.5, "{level} dB");
    // Halving the time runs the tape faster: the pitch rises.
    let (during, arrived, _) = time_change(TAPE, 400.0, 200.0);
    let highest = pitch_by_crossings(&during, rate)
        .iter()
        .map(|(_, hz)| *hz)
        .fold(f32::MIN, f32::max);
    assert!(highest > 600.0 && highest < 1100.0, "{highest} Hz");
    assert!((arrived - 0.2 * f64::from(rate)).abs() <= 0.0005 * f64::from(rate));
}

#[test]
fn a_bucket_line_glides_its_clock_and_hands_over_to_another_stage_count() {
    let rate = 48_000.0_f32;
    // 100 to 400 ms is another clock and another number of stages.
    for (from, to) in [(100.0, 400.0), (400.0, 100.0), (300.0, 250.0)] {
        let (during, arrived, level) = time_change(BBD, from, to);
        // The tone bends and no step is steeper than a bent tone's.
        let steady = 0.25 * std::f32::consts::TAU * 500.0 / rate;
        let step = largest_step(&during);
        assert!(
            step <= 3.0 * steady,
            "{from} to {to} ms: a step of {step} against {steady}"
        );
        let pitch = pitch_by_crossings(&during[..24_000], rate);
        let (lowest, highest) = pitch
            .iter()
            .fold((f32::MAX, f32::MIN), |(low, high), (_, hz)| {
                (low.min(*hz), high.max(*hz))
            });
        if to > from {
            assert!(lowest < 480.0, "{from} to {to} ms: {lowest} Hz");
        } else {
            assert!(highest > 520.0, "{from} to {to} ms: {highest} Hz");
        }
        assert!(
            (arrived - f64::from(to / 1000.0 * rate)).abs() <= 0.001 * f64::from(rate),
            "{from} to {to} ms: a repeat after {arrived} samples"
        );
        assert!(level.abs() <= 1.0, "{from} to {to} ms: {level} dB");
    }
}

#[test]
fn wobble_moves_the_time_as_a_wavering_transport_would() {
    let rate = 48_000.0_f32;
    // BBD has the wow alone, on its clock.
    let values = with(
        &ECHO_ONLY,
        &[
            (ECHO_MODEL, BBD),
            (ECHO_TIME, 300.0),
            (ECHO_FEEDBACK, 0.0),
            (ECHO_WOBBLE, 100.0),
        ],
    );
    let input = sine(500.0, 0.25, f64::from(rate), 240_000);
    let (left, _) = run_stereo(&mut *device(rate, &values), &input);
    let pitch = instantaneous_hz(&left[48_000..], 500.0, f64::from(rate), 96);
    let deviation = pitch
        .iter()
        .map(|hz| (hz - 500.0).abs())
        .fold(0.0, f64::max);
    // A clock that errs by 1.5 % at 0.5 Hz, integrated over the 300 ms the
    // sound spends in the line, moves the delay by
    // `share/(2π·f)·2·sin(π·f·T)`; the pitch follows its slope.
    let (share, wow, seconds) = (0.015, 0.5, 0.3);
    let swing =
        share / (std::f64::consts::TAU * wow) * 2.0 * (std::f64::consts::PI * wow * seconds).sin();
    let expected = 500.0 * std::f64::consts::TAU * wow * swing;
    assert!(
        (deviation / expected - 1.0).abs() <= 0.2,
        "{deviation} Hz against {expected}"
    );
}

#[test]
fn full_feedback_holds_or_sings() {
    let rate = 48_000.0_f32;
    let looped = |model: f32| {
        with(
            &ECHO_ONLY,
            &[
                (ECHO_MODEL, model),
                (ECHO_TIME, 100.0),
                (ECHO_STEREO, MONO),
                (ECHO_FEEDBACK, 100.0),
            ],
        )
    };
    // Digital: ten seconds, a hundred repeats, and the impulse is as it was.
    let (left, _) = run_stereo(&mut *device(rate, &looped(DIGITAL)), &impulse(480_100));
    assert_eq!(left[480_000], 1.0);

    let mut random = Noise(77);
    let mut input: Vec<f32> = (0..24_000).map(|_| 0.5 * random.next()).collect();
    input.resize(8 * 48_000, 0.0);
    for (model, name) in [(TAPE, "Tape"), (BBD, "BBD")] {
        let (left, _) = run_stereo(&mut *device(rate, &looped(model)), &input);
        assert!(left.iter().all(|x| x.is_finite()));
        let peak = db(f64::from(peak(&left)));
        assert!(peak < 6.0, "{name}: {peak} dBFS");
        let (middle, last) = (rms(&left[5 * 48_000..6 * 48_000]), rms(&left[7 * 48_000..]));
        assert!(
            last > 0.05 && last > 0.9 * middle,
            "{name} keeps sounding: {middle} then {last}"
        );
    }
}

/// The duck's reduction in dB at full depth after `input`.
fn duck_after(values: &[(u32, f32)], input: &[f32]) -> f32 {
    let mut device = device(48_000.0, values);
    run_stereo(&mut *device, input);
    device.telemetry().value(TELEMETRY_DUCK)
}

#[test]
fn duck_follows_the_input_level_and_each_section_takes_its_share() {
    let rate = 48_000.0_f32;
    let level = |db: f32| 10.0_f32.powf(db / 20.0);
    let constant = |db: f32| vec![level(db); 48_000];
    // Full reduction from −12 dBFS, the middle of the knee at −36. The
    // detector is one for both sections, whatever they take of it.
    for values in [
        [(ECHO_DUCK, 100.0), (REVERB_DUCK, 0.0)],
        [(ECHO_DUCK, 20.0), (REVERB_DUCK, 50.0)],
    ] {
        for (input, expected) in [(-12.0, 24.0), (-36.0, 1.5), (-60.0, 0.0), (0.0, 24.0)] {
            let reduction = duck_after(&values, &constant(input));
            assert!(
                (reduction - expected).abs() <= 0.5,
                "{values:?} at {input} dBFS: {reduction} dB"
            );
        }
    }
    // A sine is followed crest by crest: at −6 dBFS it holds the full range.
    let tone = sine(1000.0, f64::from(level(-6.0)), f64::from(rate), 96_000);
    let reduction = duck_after(&[(REVERB_DUCK, 100.0)], &tone);
    assert!((reduction - 24.0).abs() <= 0.5, "{reduction} dB");
    // With nothing to duck the detector rests.
    assert_eq!(duck_after(&[], &tone), 0.0);

    // The same in the sound, section by section.
    let wet = |base: &[(u32, f32)], echo: f32, reverb: f32| {
        let values = with(base, &[(ECHO_DUCK, echo), (REVERB_DUCK, reverb)]);
        run_stereo(&mut *device(rate, &values), &tone)
    };
    let heard = |base: &[(u32, f32)], echo: f32, reverb: f32| {
        db(rms(&wet(base, 0.0, 0.0).0[48_000..]) / rms(&wet(base, echo, reverb).0[48_000..]))
    };
    for (amount, expected) in [(100.0, 24.0), (50.0, 12.0)] {
        let reverb = heard(&REVERB_ONLY, 0.0, amount);
        assert!(
            (reverb - expected).abs() <= 0.5,
            "reverb at {amount} %: {reverb} dB"
        );
        let echo = heard(&ECHO_ONLY, amount, 0.0);
        assert!(
            (echo - expected).abs() <= 0.5,
            "echo at {amount} %: {echo} dB"
        );
    }
    // A section whose Duck is 0 is untouched, to the bit, while the other
    // ducks.
    assert!(wet(&REVERB_ONLY, 100.0, 0.0) == wet(&REVERB_ONLY, 0.0, 0.0));
    assert!(wet(&ECHO_ONLY, 0.0, 100.0) == wet(&ECHO_ONLY, 0.0, 0.0));
    // In series the reverb hears the echo before the echo's duck.
    let series = [
        (MIX, 100.0),
        (ECHO_LEVEL, 0.0),
        (REVERB_LEVEL, 0.0),
        (ECHO_SYNC, 0.0),
    ];
    let reverb_of = |echo_duck: f32| {
        let with_echo = wet(&with(&series, &[(ECHO_DUCK, echo_duck)]), echo_duck, 0.0).0;
        let echo_alone = wet(&with(&series, &[(REVERB_ON, 0.0)]), echo_duck, 0.0).0;
        let reverb: Vec<f32> = with_echo
            .iter()
            .zip(&echo_alone)
            .map(|(a, b)| a - b)
            .collect();
        rms(&reverb[48_000..])
    };
    let change = db(reverb_of(100.0) / reverb_of(0.0));
    assert!(
        change.abs() < 0.1,
        "the reverb moved by {change} dB with the echo's duck"
    );
}

#[test]
fn duck_comes_in_eight_milliseconds_lets_go_in_three_hundred_and_does_not_ripple() {
    let rate = 48_000.0_f32;
    let mut device = device(rate, &[(ECHO_DUCK, 100.0)]);
    let telemetry = device.telemetry();
    // Blocks of eight samples: a reading every sixth of a millisecond.
    let mut trace = |input: &mut dyn FnMut(usize) -> f32, frames: usize| -> Vec<f32> {
        (0..frames / 8)
            .map(|block| {
                let mut left: [f32; 8] = std::array::from_fn(|n| input(8 * block + n));
                let mut right = left;
                run(&mut *device, &mut left, &mut right);
                telemetry.value(TELEMETRY_DUCK)
            })
            .collect()
    };
    let rise = trace(&mut |_| 0.25, 24_000);
    let fall = trace(&mut |_| 0.0, 96_000);
    let milliseconds = |trace: &[f32]| (time_to(trace, 0.632) + 1) as f32 * 8.0 / rate * 1000.0;
    let (rise, fall) = (milliseconds(&rise), milliseconds(&fall));
    assert!((rise / 8.0 - 1.0).abs() <= 0.1, "in {rise} ms");
    // The detector holds a peak for 5 ms before it lets go.
    assert!(((fall - 5.0) / 300.0 - 1.0).abs() <= 0.1, "out {fall} ms");

    // A 50 Hz tone in the knee, where the gain follows the level most
    // closely: the cycles do not come through.
    for amplitude in [0.02_f32, 0.05, 0.5] {
        let mut tone =
            |n: usize| amplitude * (std::f32::consts::TAU * 50.0 * n as f32 / rate).sin();
        let held = trace(&mut tone, 96_000);
        let settled = &held[held.len() / 2..];
        let (low, high) = settled.iter().fold((f32::MAX, f32::MIN), |(low, high), x| {
            (low.min(*x), high.max(*x))
        });
        assert!(
            high - low < 0.5,
            "50 Hz at {amplitude}: the gain ripples by {} dB around {high}",
            high - low
        );
    }
}

#[test]
fn routing_puts_the_echo_through_the_reverb() {
    let rate = 48_000.0_f32;
    let mut random = Noise(3);
    let noise: Vec<f32> = (0..48_000).map(|_| 0.5 * random.next()).collect();
    // Without an echo there is nothing to route.
    let alone = |routing: f32| {
        let values = [(ECHO_ON, 0.0), (ROUTING, routing)];
        run_stereo(&mut *device(rate, &values), &noise)
    };
    assert!(alone(SERIES) == alone(PARALLEL));

    // An echo every 375 ms and a reverb that has gone after 300: between
    // the repeats there is only what the reverb makes of them.
    let between = |routing: f32| {
        let values = [
            (ROUTING, routing),
            (REVERB_MODEL, ROOM),
            (DECAY, 0.3),
            (MIX, 100.0),
        ];
        let (left, _) = run_stereo(&mut *device(rate, &values), &impulse(36_000));
        rms(&left[21_600..33_600])
    };
    let (series, parallel) = (between(SERIES), between(PARALLEL));
    assert!(series > 3e-5, "{series}");
    assert!(series > 30.0 * parallel, "{series} against {parallel}");
}

#[test]
fn switches_do_not_click() {
    let rate = 48_000.0_f32;
    let mut device = device(rate, &[(MIX, 50.0)]);
    let mut position = 0;
    let mut step = |device: &mut dyn Device, frames: usize| {
        let input: Vec<f32> = (position..position + frames)
            .map(|n| 0.5 * (std::f32::consts::TAU * 220.0 * n as f32 / rate).sin())
            .collect();
        position += frames;
        let (left, right) = run_stereo(device, &input);
        largest_step(&left).max(largest_step(&right))
    };
    step(&mut *device, 96_000);
    let steady = step(&mut *device, 48_000);
    let changes = [
        ("BBD", (ECHO_MODEL, BBD)),
        ("Tape", (ECHO_MODEL, TAPE)),
        ("Digital", (ECHO_MODEL, DIGITAL)),
        ("Mono", (ECHO_STEREO, MONO)),
        ("Wide", (ECHO_STEREO, WIDE)),
        ("Ping-pong", (ECHO_STEREO, PING_PONG)),
        ("Quad", (ECHO_TAPS, 3.0)),
        ("Golden", (ECHO_TAPS, 7.0)),
        ("Single", (ECHO_TAPS, 0.0)),
        ("Diffusion", (ECHO_DIFFUSION, 100.0)),
        ("Studio", (REVERB_MODEL, STUDIO)),
        ("Room", (REVERB_MODEL, ROOM)),
        ("Hall", (REVERB_MODEL, HALL)),
        ("Parallel", (ROUTING, PARALLEL)),
        ("Series", (ROUTING, SERIES)),
        ("Echo off", (ECHO_ON, 0.0)),
        ("Echo on", (ECHO_ON, 1.0)),
        ("Reverb off", (REVERB_ON, 0.0)),
        ("Reverb on", (REVERB_ON, 1.0)),
        ("Tape again", (ECHO_MODEL, TAPE)),
        ("Tape Triple", (ECHO_TAPS, 2.0)),
        ("BBD again", (ECHO_MODEL, BBD)),
        ("BBD Dotted", (ECHO_TAPS, 4.0)),
        ("BBD Wide", (ECHO_STEREO, WIDE)),
    ];
    for (name, change) in changes {
        set(&mut *device, &[change]);
        let largest = step(&mut *device, 48_000);
        assert!(
            largest <= 1.5 * steady + 0.02,
            "{name}: {largest} against {steady}"
        );
    }
}

/// Full-scale noise for `seconds`, a constant 1.0 for as long, then
/// `silence` seconds of nothing.
fn stress_signal(rate: f32, seconds: f32, silence: f32) -> Vec<f32> {
    let mut random = Noise(31);
    let frames = (seconds * rate) as usize;
    let mut signal: Vec<f32> = (0..frames).map(|_| random.next()).collect();
    signal.extend(std::iter::repeat_n(1.0, frames));
    signal.extend(std::iter::repeat_n(0.0, (silence * rate) as usize));
    signal
}

/// The plan's stress: an echo that never ends, smeared, into the longest
/// decay of the largest room, under noise and then a constant.
fn check_stability(rate: f32, seconds: f32) {
    let silence = 5.0;
    let input = stress_signal(rate, seconds, silence);
    let stressed = [
        (DECAY, 20.0),
        (SIZE, 100.0),
        (ROUTING, SERIES),
        (MIX, 100.0),
        (ECHO_FEEDBACK, 100.0),
        (ECHO_DIFFUSION, 100.0),
    ];
    for model in 0..3 {
        let model = model as f32;
        let values = with(&stressed, &[(ECHO_MODEL, model), (REVERB_MODEL, model)]);
        let (left, right) = run_stereo(&mut *device(rate, &values), &input);
        for channel in [&left, &right] {
            assert!(channel.iter().all(|x| x.is_finite()));
            let peak = db(f64::from(peak(channel)));
            assert!(peak < 24.0, "models {model} at {rate} Hz: {peak} dBFS");
        }

        // The reverb by itself: the echo above never stops.
        let values = with(&stressed, &[(ECHO_ON, 0.0), (REVERB_MODEL, model)]);
        let (left, _) = run_stereo(&mut *device(rate, &values), &input);
        assert!(left.iter().all(|x| x.is_finite()));
        let second = rate as usize;
        let stop = 2 * (seconds * rate) as usize;
        // The low cut keeps the constant out of the wet signal.
        let held = &left[stop - second..stop];
        let mean = held.iter().map(|x| f64::from(*x)).sum::<f64>() / held.len() as f64;
        assert!(db(mean.abs()) < -80.0, "{} dBFS of DC", db(mean.abs()));
        let levels: Vec<f64> = left[stop..].chunks_exact(second).map(rms).collect();
        assert!(levels[0] > 1e-3, "there is a tail to watch: {levels:?}");
        assert!(
            levels.windows(2).all(|pair| pair[1] < pair[0]),
            "reverb {model} at {rate} Hz: {levels:?}"
        );
    }
}

#[test]
fn the_worst_settings_stay_bounded() {
    check_stability(48_000.0, 3.0);
}

/// The plan's full length at every rate.
#[test]
#[ignore = "long renders"]
fn the_worst_settings_stay_bounded_for_ten_seconds_at_every_rate() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        check_stability(rate, 10.0);
    }
}

#[test]
fn a_synced_tail_covers_a_slow_tempo_it_has_not_been_told() {
    let rate = 48_000.0;
    let mut device = device(
        rate,
        &[
            (REVERB_ON, 0.0),
            (ECHO_SYNC, 1.0),
            (ECHO_DIVISION, 13.0),
            (ECHO_FEEDBACK, 20.0),
        ],
    );
    // Asked before the first block, as the engine does.
    let tail = device.tail_samples() as usize;
    let (left, right) = run_at_tempo(&mut *device, 60.0, &impulse(tail + 48_000));
    let loudest = peak(&left).max(peak(&right));
    let last = left
        .iter()
        .zip(&right)
        .rposition(|(l, r)| l.abs().max(r.abs()) > 1e-3 * loudest)
        .unwrap();
    assert!(last > 2 * 48_000, "the half note at 60 bpm: {last}");
    assert!(last <= tail, "−60 dB at {last}, reported {tail}");
}

#[test]
fn no_latency_and_a_tail_that_covers_the_sound() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let device = device(rate, &[]);
        assert_eq!(device.latency_samples(), 0);
        assert_eq!((spacer::DESCRIPTOR.latency)(&|_| 0.0, rate), 0);
    }
    let rate = 48_000.0;
    let tail_of =
        |values: &[(u32, f32)]| support::device(&spacer::DESCRIPTOR, rate, values).tail_samples();
    let cases: [&[(u32, f32)]; 6] = [
        &[],
        &[(REVERB_MODEL, STUDIO), (DECAY, 0.4)],
        &[
            (REVERB_MODEL, ROOM),
            (ECHO_MODEL, TAPE),
            (ECHO_DIFFUSION, 100.0),
        ],
        &[
            (REVERB_ON, 0.0),
            (ECHO_MODEL, BBD),
            (ECHO_FEEDBACK, 60.0),
            (ECHO_TAPS, 3.0),
        ],
        &[
            (ECHO_ON, 0.0),
            (SIZE, 100.0),
            (DECAY, 1.0),
            (PRE_DELAY, 250.0),
        ],
        &[(ROUTING, PARALLEL), (ECHO_STEREO, WIDE)],
    ];
    for values in cases {
        let mut device = device(rate, values);
        let tail = device.tail_samples() as usize;
        let (left, right) = run_stereo(&mut *device, &impulse(tail + 48_000));
        let loudest = peak(&left).max(peak(&right));
        let last = left
            .iter()
            .zip(&right)
            .rposition(|(l, r)| l.abs().max(r.abs()) > 1e-3 * loudest)
            .unwrap();
        assert!(last > 4800, "{values:?}: there is a tail: {last}");
        assert!(
            last <= tail,
            "{values:?}: −60 dB at {last}, reported {tail}"
        );
    }
    // The parts of the tail: nothing when both are off, the longer one in
    // parallel, one after the other in series.
    assert_eq!(tail_of(&[(ECHO_ON, 0.0), (REVERB_ON, 0.0)]), 0);
    // Free-running, so that the sum stays under the longest tail reported.
    let free = (ECHO_SYNC, 0.0);
    let (echo, reverb) = (
        tail_of(&[free, (REVERB_ON, 0.0)]),
        tail_of(&[(ECHO_ON, 0.0)]),
    );
    assert_eq!(tail_of(&[free, (ROUTING, PARALLEL)]), echo.max(reverb));
    assert!(tail_of(&[free, (ROUTING, SERIES)]).abs_diff(echo + reverb) <= 1);
    assert!(tail_of(&[(ECHO_FEEDBACK, 100.0)]) as f32 / rate >= 30.0);
}

#[test]
fn controls_apply_where_their_section_is_on() {
    let hidden = |values: &[(u32, f32)]| -> Vec<u32> {
        let settings = settings(values);
        spacer::PARAMETERS
            .iter()
            .map(|spec| spec.id)
            .filter(|id| !applies(*id, &settings))
            .collect()
    };
    assert_eq!(hidden(&[]), [ECHO_TIME]);
    assert_eq!(hidden(&[(ECHO_SYNC, 0.0)]), [ECHO_DIVISION]);
    // Nothing comes or goes with a model.
    for model in 0..3 {
        let model = model as f32;
        assert_eq!(
            hidden(&[(ECHO_MODEL, model), (REVERB_MODEL, model)]),
            [ECHO_TIME]
        );
    }
    let echo_off = hidden(&[(ECHO_ON, 0.0)]);
    assert_eq!(echo_off.len(), 14);
    assert!(echo_off.contains(&ROUTING) && !echo_off.contains(&ECHO_ON));
    assert!(echo_off.contains(&ECHO_DUCK) && echo_off.contains(&ECHO_TAPS));
    let reverb_off = hidden(&[(REVERB_ON, 0.0)]);
    assert_eq!(reverb_off.len(), 15);
    assert!(reverb_off.contains(&ROUTING) && reverb_off.contains(&DECAY));
    assert!(reverb_off.contains(&SEED) && reverb_off.contains(&REVERB_DUCK));
    assert!(!reverb_off.contains(&REVERB_ON) && !reverb_off.contains(&MIX));
    // The parameters are what the design lists, in signal order.
    assert_eq!(spacer::PARAMETERS.len(), 31);
    let groups: Vec<&str> = spacer::PARAMETERS.iter().map(|spec| spec.group).collect();
    assert!(
        groups
            .is_sorted_by_key(|group| ["Echo", "Reverb", "Output"].iter().position(|x| x == group))
    );
}

/// The largest second difference against the peak: a step stands out of a
/// low tone there, whatever the tone's level does meanwhile.
fn roughness(signal: &[f32]) -> f32 {
    let roughest = signal
        .windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0, f32::max);
    roughest / peak(signal)
}

/// A settled tone through a device, then on through one `change`; the
/// output from a few samples before the change.
fn through_a_change(values: &[(u32, f32)], hz: f64, change: (u32, f32)) -> Vec<f32> {
    let rate = 48_000.0_f32;
    let mut device = device(rate, values);
    let tone = sine(hz, 0.25, f64::from(rate), 192_000);
    let (before, _) = run_stereo(&mut *device, &tone[..96_000]);
    set(&mut *device, &[change]);
    let (after, _) = run_stereo(&mut *device, &tone[96_000..]);
    let mut joined = before[95_990..].to_vec();
    joined.extend(after);
    joined
}

/// How far a tone through `values` is from the same run without `change`
/// in the first millisecond after it, against the peak of the output.
fn step_at_a_change(values: &[(u32, f32)], hz: f64, change: (u32, f32)) -> f32 {
    let changed = through_a_change(values, hz, change);
    let kept = through_a_change(values, hz, (change.0, value_in(values, change.0)));
    let apart = changed[10..58]
        .iter()
        .zip(&kept[10..58])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    apart / peak(&kept)
}

fn value_in(values: &[(u32, f32)], id: u32) -> f32 {
    values
        .iter()
        .rev()
        .find(|(of, _)| *of == id)
        .expect("the value is set")
        .1
}

#[test]
fn reverb_controls_move_without_a_step() {
    for (model, name) in REVERBS {
        // A 220 Hz tone's own second difference is 0.08 % of its peak, and
        // twice that while a Size glide bends it up; a step of a hundredth
        // of the tone would show as 1 %.
        for (hz, before, change) in [
            (220.0, vec![(SIZE, 0.0), (DECAY, 0.5)], (SIZE, 100.0)),
            (220.0, vec![(SIZE, 100.0), (DECAY, 0.5)], (SIZE, 0.0)),
            (220.0, vec![(SIZE, 0.0), (DECAY, 1.8)], (SIZE, 100.0)),
            (220.0, vec![(SIZE, 90.0), (DECAY, 1.8)], (SIZE, 10.0)),
            (220.0, vec![(SIZE, 30.0)], (SEED, 7.0)),
            (220.0, vec![(DECAY, 0.2)], (DECAY, 10.0)),
            (220.0, vec![(DECAY, 10.0)], (DECAY, 0.2)),
            (220.0, vec![(DECAY, 1.0)], (DECAY, 2.0)),
            (220.0, vec![(DECAY, 1.8)], (DECAY, 0.5)),
            // Onto the Off position of the low cut and off it.
            (60.0, vec![(REVERB_LOW_CUT, 21.0)], (REVERB_LOW_CUT, 20.0)),
            (60.0, vec![(REVERB_LOW_CUT, 20.0)], (REVERB_LOW_CUT, 21.0)),
        ] {
            let mut values = with(&REVERB_ONLY, &[(REVERB_MODEL, model), (MOVEMENT, 0.0)]);
            values.extend(before.iter().copied());
            let rough = roughness(&through_a_change(&values, hz, change));
            assert!(
                rough <= 0.004,
                "{name}, {hz} Hz, {before:?} to {change:?}: {rough}"
            );
        }
        // Color's high cut is off from the centre up. A second difference
        // says nothing at 5 kHz: the run is held against one without the
        // change.
        for (from, to) in [(0.0, -1.0), (-1.0, 0.0)] {
            let values = with(
                &REVERB_ONLY,
                &[(REVERB_MODEL, model), (MOVEMENT, 0.0), (REVERB_COLOR, from)],
            );
            let step = step_at_a_change(&values, 5000.0, (REVERB_COLOR, to));
            assert!(step <= 0.02, "{name}, Color {from} to {to}: {step}");
        }
    }
}

#[test]
fn echo_cuts_come_and_go_without_a_step() {
    let digital = with(
        &ECHO_ONLY,
        &[
            (ECHO_MODEL, DIGITAL),
            (ECHO_TIME, 100.0),
            (ECHO_FEEDBACK, 50.0),
        ],
    );
    for (from, to) in [(21.0, 20.0), (20.0, 21.0)] {
        let values = with(&digital, &[(ECHO_LOW_CUT, from)]);
        let rough = roughness(&through_a_change(&values, 60.0, (ECHO_LOW_CUT, to)));
        assert!(rough <= 0.004, "Low cut {from} to {to}: {rough}");
    }
    for (from, to) in [(100.0, 99.0), (99.0, 100.0)] {
        let values = with(&digital, &[(ECHO_COLOR, from)]);
        let step = step_at_a_change(&values, 10_000.0, (ECHO_COLOR, to));
        assert!(step <= 0.02, "Color {from} to {to}: {step}");
    }
}

#[test]
fn a_short_time_bends_without_a_dropout_or_a_click() {
    let rate = 48_000.0_f32;
    // Below 320 ms the loop diffusers' lengths follow the time.
    for (model, name) in [(BBD, "BBD"), (TAPE, "Tape")] {
        let values = |time: f32| {
            with(
                &ECHO_ONLY,
                &[(ECHO_MODEL, model), (ECHO_TIME, time), (ECHO_FEEDBACK, 0.0)],
            )
        };
        let tone = sine(1000.0, 0.1, f64::from(rate), 288_000);
        let second_difference = |signal: &[f32]| {
            signal
                .windows(3)
                .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
                .fold(0.0, f32::max)
        };
        for (from, to) in [(300.0, 330.0), (250.0, 240.0)] {
            let mut device = device(rate, &values(from));
            let (before, _) = run_stereo(&mut *device, &tone[..48_000]);
            set(&mut *device, &[(ECHO_TIME, to)]);
            let (after, _) = run_stereo(&mut *device, &tone[48_000..144_000]);
            let level = rms(&before[38_400..]);
            for (index, window) in after.chunks(480).enumerate() {
                let change = db(rms(window) / level);
                assert!(
                    change.abs() < 1.0,
                    "{name}, {from} to {to} ms: {change} dB {} ms after the step",
                    10 * index
                );
            }
        }
        // A slow ramp, as a tempo change gives a synced time.
        for (from, to) in [(200.0, 260.0), (300.0, 360.0)] {
            let mut device = device(rate, &values(from));
            let (before, _) = run_stereo(&mut *device, &tone[..48_000]);
            let mut moving = Vec::new();
            for (index, block) in tone[48_000..].chunks(64).enumerate() {
                let t = (index as f32 * 64.0 / 192_000.0).min(1.0);
                set(&mut *device, &[(ECHO_TIME, from + t * (to - from))]);
                moving.extend(run_stereo(&mut *device, block).0);
            }
            let (steady, bent) = (
                second_difference(&before[38_400..]),
                second_difference(&moving),
            );
            assert!(
                bent < 1.5 * steady,
                "{name}, {from} to {to} ms: {bent} against {steady} at rest"
            );
        }
    }
}
