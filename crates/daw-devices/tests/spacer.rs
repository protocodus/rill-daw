mod support;

use daw_devices::devices::spacer::{self, *};
use daw_devices::dsp::{Svf, prewarp};
use daw_devices::{Device, ProcessContext, StereoIo};
use support::*;

const ROOM: f32 = 0.0;
const CHAMBER: f32 = 1.0;
const HALL: f32 = 2.0;
const PLATE: f32 = 3.0;
const REVERB_NAMES: [(f32, &str); 4] = [
    (ROOM, "Room"),
    (CHAMBER, "Chamber"),
    (HALL, "Hall"),
    (PLATE, "Plate"),
];
const DIGITAL: f32 = 0.0;
const TAPE: f32 = 1.0;
const ANALOG: f32 = 2.0;
const MONO: f32 = 0.0;
const PING_PONG: f32 = 1.0;
const WIDE: f32 = 2.0;
const SERIES: f32 = 0.0;
const PARALLEL: f32 = 1.0;

/// The reverb alone, at full level.
const REVERB_ONLY: [(u32, f32); 3] = [(ECHO_ON, 0.0), (MIX, 100.0), (REVERB_LEVEL, 0.0)];
/// The echo alone, at full level, with a free time and nothing cut.
const ECHO_ONLY: [(u32, f32); 6] = [
    (REVERB_ON, 0.0),
    (MIX, 100.0),
    (ECHO_LEVEL, 0.0),
    (ECHO_SYNC, 0.0),
    (ECHO_LOW_CUT, 20.0),
    (ECHO_HIGH_CUT, 20_000.0),
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

/// A third of an octave wide.
fn third_octave_band(signal: &[f32], centre: f32, rate: f32) -> Vec<f32> {
    band(signal, centre, 0.55, rate)
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

/// The reverb's impulse response, left channel, long enough for a decay of
/// `longest` seconds to fall by some 55 dB.
fn reverb_response(values: &[(u32, f32)], rate: f32, longest: f32) -> Vec<f32> {
    let frames = ((0.9 * longest + 0.4) * rate) as usize;
    run_stereo(&mut *device(rate, values), &impulse(frames)).0
}

/// Where the late field is on its own: after the pre-delay and the last
/// early reflection. A room's reflections are a burst of their own level
/// in front of the tail; a decay read from the direct sound on would
/// measure the step between the two, not the slope.
fn late_field_start(values: &[(u32, f32)], rate: f32) -> usize {
    let settings = settings(values);
    let mut taps = [[(0.0, 0.0); 8]; 2];
    early_taps(&settings, &mut taps);
    let last = taps[0]
        .iter()
        .filter(|(_, gain)| *gain > 0.0)
        .map(|(seconds, _)| *seconds)
        .fold(0.0, f32::max);
    ((settings.reverb.pre_delay_seconds + last) * rate) as usize
}

struct Decay {
    kind: f32,
    decay: f32,
    size: f32,
    movement: f32,
    damping: f32,
}

/// Holds the decay measured in the octave around 1 kHz against the Decay
/// knob.
fn check_decay(case: &Decay, rate: f32) {
    let values = with(
        &REVERB_ONLY,
        &[
            (REVERB_TYPE, case.kind),
            (DECAY, case.decay),
            (SIZE, case.size),
            (MOVEMENT, case.movement),
            (DAMPING, case.damping),
        ],
    );
    let response = reverb_response(&values, rate, case.decay);
    let start = late_field_start(&values, rate);
    let measured = rt60(&octave_band(&response, 1000.0, rate), start, rate);
    assert!(
        (measured / case.decay - 1.0).abs() <= 0.1,
        "type {} decay {} s size {} movement {} damping {} at {rate} Hz: {measured} s",
        case.kind,
        case.decay,
        case.size,
        case.movement,
        case.damping
    );
}

/// Every combination of the plan at one Decay, without those where Decay is
/// shorter than eight mean line lengths (0.3 s at Size 100 %): there the
/// response is a few discrete passes, not a slope.
fn check_decays(decay: f32) {
    for (kind, _) in REVERB_NAMES {
        for size in [0.0, 50.0, 100.0] {
            if decay < 0.5 && size == 100.0 {
                continue;
            }
            for movement in [0.0, 100.0] {
                for damping in [0.0, 40.0] {
                    let case = Decay {
                        kind,
                        decay,
                        size,
                        movement,
                        damping,
                    };
                    check_decay(&case, 48_000.0);
                }
            }
        }
    }
}

#[test]
fn short_decays_are_the_decay_knob() {
    check_decays(0.3);
}

#[test]
fn medium_decays_are_the_decay_knob() {
    check_decays(1.8);
}

#[test]
fn long_decays_are_the_decay_knob_in_every_type() {
    for (kind, _) in REVERB_NAMES {
        for (decay, damping) in [(8.0, 0.0), (8.0, 40.0)] {
            let case = Decay {
                kind,
                decay,
                size: 50.0,
                movement: 25.0,
                damping,
            };
            check_decay(&case, 48_000.0);
        }
    }
}

#[test]
fn decays_hold_at_other_sample_rates() {
    for rate in [44_100.0, 96_000.0] {
        for (kind, _) in REVERB_NAMES {
            for damping in [0.0, 40.0] {
                let case = Decay {
                    kind,
                    decay: 1.8,
                    size: 50.0,
                    movement: 25.0,
                    damping,
                };
                check_decay(&case, rate);
            }
        }
    }
}

/// The rest of the plan's matrix: 8 and 20 s at every size and movement.
/// Twelve minutes of audio, so not part of the routine run:
/// `cargo test -p daw-devices --test spacer -- --ignored`.
#[test]
#[ignore = "long renders"]
fn the_longest_decays_are_the_decay_knob() {
    check_decays(8.0);
    check_decays(20.0);
}

#[test]
fn bass_and_treble_decay_as_the_drawn_curve_says() {
    let rate = 48_000.0;
    // Flat damping at a long decay is where the interpolated reads of a
    // moving network cost the top most.
    let cases = [
        (1.8, 40.0, 0.0),
        (1.8, 100.0, 0.0),
        (1.8, 40.0, 25.0),
        (4.0, 0.0, 25.0),
    ];
    for (kind, name) in REVERB_NAMES {
        for (decay, damping, movement) in cases {
            let values = with(
                &REVERB_ONLY,
                &[
                    (REVERB_TYPE, kind),
                    (DECAY, decay),
                    (MOVEMENT, movement),
                    (DAMPING, damping),
                    (REVERB_LOW_CUT, 20.0),
                    (REVERB_HIGH_CUT, 20_000.0),
                ],
            );
            let settings = settings(&values);
            // A third of an octave: at full damping the decay changes by
            // more than the tolerance across a whole one, and the slow end
            // of the band is what is left to measure.
            let centres: &[f32] = if damping == 100.0 {
                &[125.0, 1000.0, 8000.0]
            } else {
                &[125.0, 8000.0]
            };
            let longest = decay_seconds_at(&settings, 125.0, rate);
            let response = reverb_response(&values, rate, longest);
            let start = late_field_start(&values, rate);
            for centre in centres {
                let expected = decay_seconds_at(&settings, *centre, rate);
                let measured = rt60(&third_octave_band(&response, *centre, rate), start, rate);
                assert!(
                    (measured / expected - 1.0).abs() <= 0.15,
                    "{name} decay {decay} damping {damping} movement {movement} at {centre} Hz: \
                     {measured} s against {expected}"
                );
            }
            // The curve is not flat: the top goes first.
            let top = decay_seconds_at(&settings, 8000.0, rate) / settings.reverb.decay_seconds;
            assert!(
                top < if damping == 0.0 { 0.95 } else { 0.75 },
                "{name}: {top}"
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

/// Full-scale noise for `noise` seconds, a constant 1.0 for as long, then
/// `silence` seconds of nothing.
fn stress_signal(rate: f32, seconds: f32, silence: f32) -> Vec<f32> {
    let mut random = Noise(31);
    let frames = (seconds * rate) as usize;
    let mut signal: Vec<f32> = (0..frames).map(|_| random.next()).collect();
    signal.extend(std::iter::repeat_n(1.0, frames));
    signal.extend(std::iter::repeat_n(0.0, (silence * rate) as usize));
    signal
}

/// The plan's stress: the longest decay, the largest room and an echo that
/// never ends, in series, under noise and then a constant.
fn check_stability(seconds: f32) {
    let rate = 48_000.0;
    let silence = 5.0;
    let input = stress_signal(rate, seconds, silence);
    let stressed = [
        (DECAY, 20.0),
        (SIZE, 100.0),
        (ROUTING, SERIES),
        (MIX, 100.0),
    ];
    for kind in [DIGITAL, TAPE, ANALOG] {
        let values = with(&stressed, &[(ECHO_TYPE, kind), (ECHO_FEEDBACK, 100.0)]);
        let (left, right) = run_stereo(&mut *device(rate, &values), &input);
        for channel in [&left, &right] {
            assert!(channel.iter().all(|x| x.is_finite()));
            let peak = db(f64::from(peak(channel)));
            assert!(peak < 18.0, "echo type {kind}: {peak} dBFS");
        }
    }

    // The reverb by itself: the echo above never stops.
    let values = with(&stressed, &[(ECHO_ON, 0.0)]);
    let (left, _) = run_stereo(&mut *device(rate, &values), &input);
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
        "{levels:?}"
    );
}

#[test]
fn the_worst_settings_stay_bounded() {
    check_stability(3.0);
}

/// The plan's full length.
#[test]
#[ignore = "long renders"]
fn the_worst_settings_stay_bounded_for_ten_seconds() {
    check_stability(10.0);
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
fn the_tail_is_dense_early() {
    let rate = 48_000.0;
    for (kind, name, within) in [(HALL, "Hall", 0.15), (PLATE, "Plate", 0.08)] {
        let values = with(
            &REVERB_ONLY,
            &[(REVERB_TYPE, kind), (PRE_DELAY, 0.0), (DIFFUSION, 80.0)],
        );
        let (left, _) = run_stereo(&mut *device(rate, &values), &impulse(24_000));
        let reached = (480..14_400)
            .step_by(48)
            .find(|centre| echo_density(&left, *centre, rate) >= 0.9)
            .map(|centre| centre as f32 / rate);
        assert!(
            reached.is_some_and(|seconds| seconds <= within),
            "{name}: {reached:?} s"
        );
        // And stays so: no late thinning.
        let later = echo_density(&left, (2.0 * within * rate) as usize, rate);
        assert!(later >= 0.85, "{name} at twice the time: {later}");
    }
}

#[test]
fn the_late_tail_is_evenly_coloured() {
    use rustfft::num_complex::Complex64;
    let rate = 48_000.0_f32;
    for (kind, name) in REVERB_NAMES {
        let values = with(
            &REVERB_ONLY,
            &[
                (REVERB_TYPE, kind),
                (DECAY, 4.0),
                (DAMPING, 0.0),
                // A moving line is read between its samples and loses a
                // little of its top on every pass: by design, and by 3 to
                // 6 dB at 8 kHz here. The colour of the network is held
                // with the lines at rest.
                (MOVEMENT, 0.0),
                (REVERB_LOW_CUT, 20.0),
                (REVERB_HIGH_CUT, 20_000.0),
            ],
        );
        let (left, _) = run_stereo(&mut *device(rate, &values), &impulse(96_000));
        let mut bins: Vec<Complex64> = left[48_000..]
            .iter()
            .map(|x| Complex64::new(f64::from(*x), 0.0))
            .collect();
        rustfft::FftPlanner::new()
            .plan_fft_forward(bins.len())
            .process(&mut bins);
        // One second: a bin per hertz.
        let levels: Vec<f32> = (0..=16)
            .map(|third| {
                let centre = 200.0 * 2.0_f64.powf(f64::from(third) / 3.0);
                let (low, high) = (
                    (centre / 2.0_f64.powf(1.0 / 6.0)) as usize,
                    (centre * 2.0_f64.powf(1.0 / 6.0)) as usize,
                );
                let power: f64 = bins[low..=high].iter().map(|bin| bin.norm_sqr()).sum();
                (10.0 * (power / (high - low + 1) as f64).log10()) as f32
            })
            .collect();
        let mean = levels.iter().sum::<f32>() / levels.len() as f32;
        for (third, level) in levels.iter().enumerate() {
            assert!(
                (level - mean).abs() <= 3.0,
                "{name}, band {third} above 200 Hz: {} dB off the mean",
                level - mean
            );
        }
    }
}

#[test]
fn size_does_not_change_the_level() {
    let rate = 48_000.0;
    let mut random = Noise(17);
    let input: Vec<f32> = (0..120_000).map(|_| 0.25 * random.next()).collect();
    for (kind, name) in REVERB_NAMES {
        let levels: Vec<f32> = [0.0, 50.0, 100.0]
            .iter()
            .map(|size| {
                let values = with(&REVERB_ONLY, &[(REVERB_TYPE, kind), (SIZE, *size)]);
                let (left, _) = run_stereo(&mut *device(rate, &values), &input);
                db(rms(&left[72_000..]))
            })
            .collect();
        let middle = 0.5
            * (levels.iter().copied().fold(f32::MIN, f32::max)
                + levels.iter().copied().fold(f32::MAX, f32::min));
        assert!(
            levels.iter().all(|level| (level - middle).abs() <= 2.0),
            "{name}: {levels:?}"
        );
    }
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

#[test]
fn digital_repeats_are_exact() {
    let rate = 48_000.0_f32;
    for (milliseconds, repeats) in [(1.0, 6), (350.0, 4), (2000.0, 2)] {
        let values = with(
            &ECHO_ONLY,
            &[
                (ECHO_TIME, milliseconds),
                (ECHO_STEREO, MONO),
                (ECHO_FEEDBACK, 50.0),
            ],
        );
        let spacing = (milliseconds / 1000.0 * rate).round() as usize;
        let frames = repeats * spacing + spacing / 2;
        let (left, right) = run_stereo(&mut *device(rate, &values), &impulse(frames));
        assert_eq!(left, right);
        let taps = taps_of(&left);
        assert_eq!(taps.len(), repeats, "{milliseconds} ms: {taps:?}");
        // What the panel draws is what sounds.
        let mut drawn = [(0.0, 0.0, 0.0); 8];
        echo_taps(&settings(&values), 120.0, &mut drawn);
        for (repeat, (index, value)) in taps.iter().enumerate() {
            assert_eq!(*index, (repeat + 1) * spacing, "{milliseconds} ms");
            let expected = 0.5_f64.powi(repeat as i32);
            let error = db(f64::from(*value) / expected);
            assert!(
                error.abs() <= 0.1,
                "{milliseconds} ms, repeat {repeat}: {error} dB"
            );
            let (seconds, gain, _) = drawn[repeat];
            assert!((seconds * rate - *index as f32).abs() < 0.5);
            assert!((gain - *value).abs() < 1e-3);
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
    assert_eq!(taps.len(), 1);
    assert_eq!(taps[0].0, 18_000);
    assert_eq!((seconds, tempo), (0.375, 120.0));
    // A dotted whole note at 60 would be 6 s: halved to fit the line.
    assert_eq!(DIVISIONS[17], "1/1 D");
    let (taps, seconds, tempo) = first_tap(60.0, 17.0, 150_000);
    assert_eq!(taps.len(), 1);
    assert_eq!(taps[0].0, 144_000);
    assert_eq!((seconds, tempo), (3.0, 60.0));
    assert_eq!(division_beats(17), 6.0);
    assert_eq!(division_beats(0), 1.0 / 12.0);
}

#[test]
fn stereo_modes_place_the_repeats() {
    let rate = 48_000.0_f32;
    let echo = |stereo: f32| {
        with(
            &ECHO_ONLY,
            &[
                (ECHO_TIME, 25.0),
                (ECHO_STEREO, stereo),
                (ECHO_FEEDBACK, 50.0),
            ],
        )
    };
    let indices = |signal: &[f32]| -> Vec<usize> {
        taps_of(signal).iter().map(|(index, _)| *index).collect()
    };
    let spacing = 1200;

    let (left, right) = run_stereo(&mut *device(rate, &echo(PING_PONG)), &impulse(5000));
    assert_eq!(indices(&left), [spacing, 3 * spacing]);
    assert_eq!(indices(&right), [2 * spacing, 4 * spacing]);

    let (left, right) = run_stereo(&mut *device(rate, &echo(MONO)), &impulse(5000));
    assert_eq!(left, right);
    assert_eq!(
        indices(&left),
        [spacing, 2 * spacing, 3 * spacing, 4 * spacing]
    );

    // Wide: the right line is 3 % longer, at least 3 and at most 15 ms.
    for (milliseconds, later) in [(25.0, 0.003), (350.0, 0.0105)] {
        let values = with(
            &echo(WIDE),
            &[(ECHO_TIME, milliseconds), (ECHO_FEEDBACK, 0.0)],
        );
        let spacing = (milliseconds / 1000.0 * rate).round() as usize;
        let (left, right) = run_stereo(&mut *device(rate, &values), &impulse(2 * spacing));
        assert_eq!(indices(&left), [spacing]);
        assert_eq!(
            indices(&right),
            [((milliseconds / 1000.0 + later) * rate).round() as usize]
        );
    }
    // The longest time there is, a whole note at 60: the line holds it and
    // the offset.
    let values = with(
        &echo(WIDE),
        &[
            (ECHO_SYNC, 1.0),
            (ECHO_DIVISION, 16.0),
            (ECHO_FEEDBACK, 0.0),
        ],
    );
    let mut device = device(rate, &values);
    run_at_tempo(&mut *device, 60.0, &[0.0; 4800]);
    let (left, right) = run_at_tempo(&mut *device, 60.0, &impulse(200_000));
    assert_eq!(indices(&left), [192_000]);
    assert_eq!(indices(&right), [192_000 + 720]);
}

#[test]
fn coloured_echoes_do_not_alias() {
    let rate = 48_000.0;
    for (kind, name) in [(TAPE, "Tape"), (ANALOG, "Analog")] {
        // 20 ms: a bucket-brigade line that short is as bright as tape.
        let values = with(
            &ECHO_ONLY,
            &[(ECHO_TYPE, kind), (ECHO_TIME, 20.0), (ECHO_FEEDBACK, 0.0)],
        );
        for tone in [1237.0, 3171.0, 4999.0] {
            let (frequency, bin) = on_bin(tone, rate);
            let input = sine(frequency, 0.5, rate, FFT + FFT / 4);
            let (left, _) = run_stereo(&mut *device(rate as f32, &values), &input);
            let worst = worst_inharmonic_db(
                &spectrum(&left),
                bin,
                on_bin(20.0, rate).1 + SKIRT,
                on_bin(20_000.0, rate).1,
            );
            assert!(worst <= -80.0, "{name} at {tone} Hz: {worst} dBc");
        }
    }
}

#[test]
fn coloured_echoes_are_the_drawn_filters_at_low_level() {
    let rate = 48_000.0_f32;
    for (kind, name) in [(TAPE, "Tape"), (ANALOG, "Analog")] {
        let values = with(
            &ECHO_ONLY,
            &[(ECHO_TYPE, kind), (ECHO_TIME, 20.0), (ECHO_FEEDBACK, 0.0)],
        );
        let echo = settings(&values).echo(120.0);
        for hz in [60.0, 200.0, 1000.0, 3000.0, 5000.0] {
            let input = sine(hz, 0.01, f64::from(rate), 24_000);
            let (left, _) = run_stereo(&mut *device(rate, &values), &input);
            let measured = fit(&left[12_000..], hz, f64::from(rate)) / 0.01;
            let expected = daw_devices::dsp::echo::repeat_response(&echo, rate, hz);
            let error = db(measured / expected);
            assert!(
                error.abs() <= 0.5,
                "{name} at {hz} Hz: {error} dB off {} dB",
                db(expected)
            );
        }
    }
}

/// The lag in samples at which `output` is most like `input`, searched
/// around `near`.
fn best_lag(input: &[f32], output: &[f32], near: usize) -> usize {
    (near - 20..=near + 20)
        .max_by(|a, b| {
            let like = |lag: usize| -> f64 {
                input
                    .iter()
                    .zip(&output[lag..])
                    .map(|(x, y)| f64::from(*x) * f64::from(*y))
                    .sum()
            };
            like(*a).total_cmp(&like(*b))
        })
        .unwrap()
}

#[test]
fn coloured_echoes_arrive_on_time() {
    let rate = 48_000.0_f32;
    let mut random = Noise(57);
    let burst: Vec<f32> = (0..2000).map(|_| 0.05 * random.next()).collect();
    // An impulse says little behind a compander, and at long times a
    // bucket-brigade line's own filters hold the sound back by more than
    // a sample: noise, and Analog only where it is bright.
    for (kind, name, times) in [
        (TAPE, "Tape", [1.0, 350.0]),
        (ANALOG, "Analog", [1.0, 30.0]),
    ] {
        for milliseconds in times {
            let values = with(
                &ECHO_ONLY,
                &[
                    (ECHO_TYPE, kind),
                    (ECHO_TIME, milliseconds),
                    (ECHO_FEEDBACK, 0.0),
                ],
            );
            let expected = (milliseconds / 1000.0 * rate).round() as usize;
            let mut input = burst.clone();
            input.resize(burst.len() + expected + 100, 0.0);
            let (left, _) = run_stereo(&mut *device(rate, &values), &input);
            let lag = best_lag(&burst, &left, expected);
            assert!(
                lag.abs_diff(expected) <= 1,
                "{name} at {milliseconds} ms: {lag} samples, not {expected}"
            );
        }
    }
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

#[test]
fn a_tape_glides_to_a_new_time() {
    let rate = 48_000.0_f32;
    let values = with(
        &ECHO_ONLY,
        &[(ECHO_TYPE, TAPE), (ECHO_TIME, 100.0), (ECHO_FEEDBACK, 0.0)],
    );
    let mut device = device(rate, &values);
    let tone = sine(1000.0, 0.25, f64::from(rate), 24_000 + 96_000);
    let (before, _) = run_stereo(&mut *device, &tone[..24_000]);
    set(&mut *device, &[(ECHO_TIME, 400.0)]);
    let (during, _) = run_stereo(&mut *device, &tone[24_000..]);
    // The tone stops; what is still in the line plays out.
    let (after, _) = run_stereo(&mut *device, &[0.0; 24_000]);

    let steady = pitch_by_crossings(&before[9600..], rate);
    assert!(steady.iter().all(|(_, hz)| (hz - 1000.0).abs() < 1.0));
    let glide = pitch_by_crossings(&during, rate);
    let lowest = glide.iter().map(|(_, hz)| *hz).fold(f32::MAX, f32::min);
    let highest = glide.iter().map(|(_, hz)| *hz).fold(f32::MIN, f32::max);
    // The tape slows down, never stops and never runs backward.
    assert!(
        lowest >= 99.9 && highest <= 1900.0,
        "{lowest} to {highest} Hz"
    );
    assert!(
        lowest < 500.0,
        "a longer time bends the pitch down: {lowest} Hz"
    );
    // Settled within two seconds: the pitch is back, and the line is 400 ms
    // long, so the tone ends that much after its source.
    assert!(
        glide
            .iter()
            .filter(|(index, _)| *index > 72_000)
            .all(|(_, hz)| (hz - 1000.0).abs() < 1.0)
    );
    let end = (0.4 * rate) as usize;
    assert!(rms(&after[end - 960..end - 96]) > 0.1);
    // What follows is the loop filters ringing on the stop, 30 dB down.
    assert!(rms(&after[end + 96..end + 960]) < 0.01);
}

#[test]
fn wobble_moves_the_time_as_a_wavering_transport_would() {
    let rate = 48_000.0_f32;
    // Analog has the wow alone; 300 ms leaves it 2.4 kHz of bandwidth.
    let values = with(
        &ECHO_ONLY,
        &[
            (ECHO_TYPE, ANALOG),
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
    // A speed error of 1.5 % at 0.5 Hz, integrated over the 300 ms the
    // sound spends in the line, moves the delay by
    // `share/(2π·f)·2·sin(π·f·T)`; the pitch follows its slope.
    let (share, wow, seconds) = (0.015, 0.5, 0.3);
    let swing =
        share / (std::f64::consts::TAU * wow) * 2.0 * (std::f64::consts::PI * wow * seconds).sin();
    let expected = 500.0 * std::f64::consts::TAU * wow * swing;
    assert!(
        (deviation / expected - 1.0).abs() <= 0.1,
        "{deviation} Hz against {expected}"
    );
}

#[test]
fn full_feedback_holds_or_sings() {
    let rate = 48_000.0_f32;
    let looped = |kind: f32| {
        with(
            &ECHO_ONLY,
            &[
                (ECHO_TYPE, kind),
                (ECHO_TIME, 100.0),
                (ECHO_STEREO, MONO),
                (ECHO_FEEDBACK, 100.0),
            ],
        )
    };
    // Digital: ten seconds, a hundred repeats, and the impulse is as it was.
    let (left, _) = run_stereo(&mut *device(rate, &looped(DIGITAL)), &impulse(480_100));
    let last = db(f64::from(left[480_000]));
    assert!(last.abs() <= 0.1, "{last} dB after 10 s");

    let mut random = Noise(77);
    let mut input: Vec<f32> = (0..24_000).map(|_| 0.5 * random.next()).collect();
    input.resize(8 * 48_000, 0.0);
    for (kind, name) in [(TAPE, "Tape"), (ANALOG, "Analog")] {
        let (left, _) = run_stereo(&mut *device(rate, &looped(kind)), &input);
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

/// The duck's reduction in dB after `seconds` of `input`.
fn duck_after(values: &[(u32, f32)], input: &[f32]) -> f32 {
    let mut device = device(48_000.0, values);
    run_stereo(&mut *device, input);
    device.telemetry().value(TELEMETRY_DUCK)
}

#[test]
fn duck_follows_the_input_level() {
    let rate = 48_000.0_f32;
    let level = |db: f32| 10.0_f32.powf(db / 20.0);
    let constant = |db: f32| vec![level(db); 48_000];
    for (amount, share) in [(100.0, 1.0), (50.0, 0.5)] {
        let values = [(DUCK, amount)];
        // Full reduction from −12 dBFS, the middle of the knee at −36.
        for (input, expected) in [(-12.0, 24.0), (-36.0, 1.5), (-60.0, 0.0), (0.0, 24.0)] {
            let reduction = duck_after(&values, &constant(input));
            assert!(
                (reduction - share * expected).abs() <= 0.5,
                "Duck {amount} % at {input} dBFS: {reduction} dB"
            );
        }
    }
    // A sine is followed crest by crest: at −6 dBFS it holds the full range.
    let tone = sine(1000.0, f64::from(level(-6.0)), f64::from(rate), 96_000);
    let reduction = duck_after(&[(DUCK, 100.0)], &tone);
    assert!((reduction - 24.0).abs() <= 0.5, "{reduction} dB");
    assert_eq!(duck_after(&[(DUCK, 0.0)], &tone), 0.0);

    // The same in the sound: the reverb under the tone is 24 dB down.
    let wet = |duck: f32| {
        let values = with(&REVERB_ONLY, &[(DUCK, duck)]);
        let (left, _) = run_stereo(&mut *device(rate, &values), &tone);
        rms(&left[48_000..])
    };
    let heard = db(wet(0.0) / wet(100.0));
    assert!((heard - 24.0).abs() <= 0.5, "{heard} dB");
}

#[test]
fn duck_comes_in_eight_milliseconds_and_lets_go_in_three_hundred() {
    let rate = 48_000.0_f32;
    let mut device = device(rate, &[(DUCK, 100.0)]);
    let telemetry = device.telemetry();
    // Blocks of eight samples: a reading every sixth of a millisecond.
    let mut trace = |level: f32, frames: usize| -> Vec<f32> {
        (0..frames / 8)
            .map(|_| {
                let (mut left, mut right) = ([level; 8], [level; 8]);
                run(&mut *device, &mut left, &mut right);
                telemetry.value(TELEMETRY_DUCK)
            })
            .collect()
    };
    let rise = trace(0.25, 24_000);
    let fall = trace(0.0, 96_000);
    let milliseconds = |trace: &[f32]| (time_to(trace, 0.632) + 1) as f32 * 8.0 / rate * 1000.0;
    let (rise, fall) = (milliseconds(&rise), milliseconds(&fall));
    assert!((rise / 8.0 - 1.0).abs() <= 0.1, "in {rise} ms");
    assert!((fall / 300.0 - 1.0).abs() <= 0.1, "out {fall} ms");
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
    assert_eq!(alone(SERIES), alone(PARALLEL));

    // An echo every 375 ms and a reverb that has gone after 300: between
    // the repeats there is only what the reverb makes of them.
    let between = |routing: f32| {
        let values = [(ROUTING, routing), (DECAY, 0.3), (MIX, 100.0)];
        let (left, _) = run_stereo(&mut *device(rate, &values), &impulse(36_000));
        rms(&left[21_600..33_600])
    };
    let (series, parallel) = (between(SERIES), between(PARALLEL));
    assert!(series > 1e-4, "{series}");
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
        ("Tape", (ECHO_TYPE, TAPE)),
        ("Analog", (ECHO_TYPE, ANALOG)),
        ("Digital", (ECHO_TYPE, DIGITAL)),
        ("Mono", (ECHO_STEREO, MONO)),
        ("Wide", (ECHO_STEREO, WIDE)),
        ("Ping-pong", (ECHO_STEREO, PING_PONG)),
        ("Room", (REVERB_TYPE, ROOM)),
        ("Plate", (REVERB_TYPE, PLATE)),
        ("Chamber", (REVERB_TYPE, CHAMBER)),
        ("Hall", (REVERB_TYPE, HALL)),
        ("Parallel", (ROUTING, PARALLEL)),
        ("Series", (ROUTING, SERIES)),
        ("Echo off", (ECHO_ON, 0.0)),
        ("Echo on", (ECHO_ON, 1.0)),
        ("Reverb off", (REVERB_ON, 0.0)),
        ("Reverb on", (REVERB_ON, 1.0)),
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

#[test]
fn no_latency_and_a_tail_that_covers_the_sound() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let device = device(rate, &[]);
        assert_eq!(device.latency_samples(), 0);
        assert_eq!((spacer::DESCRIPTOR.latency)(&|_| 0.0, rate), 0);
    }
    let rate = 48_000.0;
    let mut device = device(rate, &[]);
    let tail = device.tail_samples() as usize;
    let (left, right) = run_stereo(&mut *device, &impulse(tail + 48_000));
    let last = left
        .iter()
        .zip(&right)
        .rposition(|(l, r)| l.abs().max(r.abs()) > 1e-3)
        .unwrap();
    assert!(last > 24_000, "there is a tail: {last}");
    assert!(last <= tail, "−60 dB at {last}, reported {tail}");
    // The parts of the tail: nothing when both are off, the longer one in
    // parallel, one after the other in series.
    let tail_of =
        |values: &[(u32, f32)]| support::device(&spacer::DESCRIPTOR, rate, values).tail_samples();
    assert_eq!(tail_of(&[(ECHO_ON, 0.0), (REVERB_ON, 0.0)]), 0);
    let (echo, reverb) = (tail_of(&[(REVERB_ON, 0.0)]), tail_of(&[(ECHO_ON, 0.0)]));
    assert_eq!(tail_of(&[(ROUTING, PARALLEL)]), echo.max(reverb));
    assert!(tail_of(&[(ROUTING, SERIES)]).abs_diff(echo + reverb) <= 1);
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
    let echo_off = hidden(&[(ECHO_ON, 0.0)]);
    assert_eq!(echo_off.len(), 11);
    assert!(echo_off.contains(&ROUTING) && !echo_off.contains(&ECHO_ON));
    let reverb_off = hidden(&[(REVERB_ON, 0.0)]);
    assert!(reverb_off.contains(&ROUTING) && reverb_off.contains(&DECAY));
    assert!(!reverb_off.contains(&REVERB_ON) && !reverb_off.contains(&DUCK));
}
