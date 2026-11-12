mod support;

use daw_devices::devices::compressor::{self, *};
use daw_devices::dsp::{CurveKind, TruePeak4x};
use daw_devices::{Device, HISTORY_RATE, Preset};
use support::*;

const COMPRESSOR: (u32, f32) = (MODE, 0.0);
const LIMITER: (u32, f32) = (MODE, 1.0);
const EXPANDER: (u32, f32) = (MODE, 2.0);
const DIGITAL: f32 = 0.0;
const TUBE: f32 = 1.0;
const OPTO: f32 = 2.0;
const FET: f32 = 3.0;
const CHARACTER_NAMES: [(f32, &str); 4] = [
    (DIGITAL, "Digital"),
    (TUBE, "Tube"),
    (OPTO, "Opto"),
    (FET, "FET"),
];
const EXTERNAL: (u32, f32) = (SIDECHAIN, 1.0);
const RATE: f64 = 48_000.0;
/// Reduction in dB of a constant 0.5 at the defaults: 4:1 from −18 dB.
const HALF_SCALE_REDUCTION: f32 = 8.984_55;

fn device(rate: f32, values: &[(u32, f32)]) -> Box<dyn Device> {
    support::device(&compressor::DESCRIPTOR, rate, values)
}

fn settings(values: &[(u32, f32)]) -> Settings {
    Settings::from_normalized(value_of(&compressor::DESCRIPTOR, values))
}

fn curve_db(values: &[(u32, f32)], level_db: f32) -> f32 {
    let mut out = [0.0];
    transfer_db(&settings(values), &[level_db], &mut out);
    out[0]
}

fn amplitude(level_db: f32) -> f64 {
    10.0_f64.powf(f64::from(level_db) / 20.0)
}

/// Settled output peak in dB of a 1 kHz sine (`tone`) or of a constant at
/// each level, the levels played in order without a gap.
fn settled_peaks_db(
    values: &[(u32, f32)],
    levels_db: &[f32],
    seconds: f64,
    tone: bool,
) -> Vec<f32> {
    let segment = (seconds * RATE) as usize;
    let input: Vec<f32> = (0..segment * levels_db.len())
        .map(|n| {
            let phase = std::f64::consts::TAU * 1000.0 * n as f64 / RATE;
            let wave = if tone { phase.sin() } else { 1.0 };
            (amplitude(levels_db[n / segment]) * wave) as f32
        })
        .collect();
    let output = run_mono(&mut *device(RATE as f32, values), &input);
    (1..=levels_db.len())
        .map(|step| {
            let end = step * segment;
            // The last 10 ms of the segment, where the delayed signal is
            // still this level.
            db(f64::from(peak(&output[end - 480..end])).max(1e-9)).max(-180.0)
        })
        .collect()
}

fn rising_levels() -> Vec<f32> {
    (0..=35).map(|step| -70.0 + 2.0 * step as f32).collect()
}

#[test]
fn digital_static_curve_is_the_drawn_one() {
    let levels = rising_levels();
    for threshold in [-40.0, -18.0, -6.0] {
        for ratio in [2.0, 4.0, 20.0] {
            for knee in [0.0, 6.0, 24.0] {
                let values = [
                    (THRESHOLD, threshold),
                    (RATIO, ratio),
                    (KNEE, knee),
                    (ATTACK, 0.02),
                    (RELEASE, 2500.0),
                ];
                let measured = settled_peaks_db(&values, &levels, 0.04, true);
                for (level, measured) in levels.iter().zip(measured) {
                    let drawn = curve_db(&values, *level);
                    assert!(
                        (measured - drawn).abs() < 0.1,
                        "T {threshold} R {ratio} W {knee} at {level}: {measured} against {drawn}"
                    );
                }
            }
        }
    }
}

#[test]
fn every_character_settles_on_the_same_curve() {
    let levels = [-60.0, -30.0, -22.0, -16.0, -10.0, -4.0, 0.0];
    for (character, name) in &CHARACTER_NAMES[1..] {
        // The curve is what a steady level settles on. A sine never is one
        // to a detector with an attack time: Opto's 10 ms leave it 0.5 dB
        // short of the crests at 20:1. So the levels are constants, and
        // sines where the ripple is small.
        for (threshold, ratio, knee, tone) in [
            (-18.0, 4.0, 6.0, true),
            (-12.0, 2.0, 24.0, true),
            (-18.0, 4.0, 6.0, false),
            (-30.0, 20.0, 0.0, false),
            (-12.0, 2.0, 24.0, false),
        ] {
            let values = [
                (CHARACTER, *character),
                (THRESHOLD, threshold),
                (RATIO, ratio),
                (KNEE, knee),
                (ATTACK, 0.02),
                (RELEASE, 2500.0),
            ];
            let measured = settled_peaks_db(&values, &levels, 0.8, tone);
            for (level, measured) in levels.iter().zip(measured) {
                // Tube's knee floor is part of what is drawn.
                let drawn = curve_db(&values, *level);
                assert!(
                    (measured - drawn).abs() < 0.5,
                    "{name} T {threshold} R {ratio} W {knee} tone {tone} at {level}: {measured} against {drawn}"
                );
            }
        }
    }
    let tube = effective(&settings(&[(CHARACTER, TUBE), (KNEE, 3.0)]));
    assert_eq!(tube.curve.knee_db, 12.0);
}

#[test]
fn limiter_and_expander_follow_their_curves() {
    let levels: Vec<f32> = (0..=18).map(|step| -30.0 + 2.0 * step as f32).collect();
    for knee in [0.0, 6.0] {
        let values = [LIMITER, (CEILING, -6.0), (KNEE, knee)];
        let measured = settled_peaks_db(&values, &levels, 0.1, true);
        for (level, measured) in levels.iter().zip(measured) {
            let drawn = curve_db(&values, *level);
            assert!(drawn <= -6.0 + 1e-4);
            assert!(
                (measured - drawn).abs() < 0.1,
                "limiter W {knee} at {level}: {measured} against {drawn}"
            );
        }
    }
    let levels = rising_levels();
    for (ratio, knee) in [(2.0, 6.0), (10.0, 0.0), (10.0, 12.0)] {
        let values = [
            EXPANDER,
            (THRESHOLD, -20.0),
            (RATIO, ratio),
            (KNEE, knee),
            (ATTACK, 0.02),
            (RELEASE, 2500.0),
        ];
        let measured = settled_peaks_db(&values, &levels, 0.04, true);
        for (level, measured) in levels.iter().zip(measured) {
            let drawn = curve_db(&values, *level);
            let tolerance = if drawn < -60.0 { 0.5 } else { 0.1 };
            assert!(
                (measured - drawn).abs() < tolerance,
                "expander R {ratio} W {knee} at {level}: {measured} against {drawn}"
            );
        }
    }
    // The floor: no more than 80 dB are taken away.
    let gate = [EXPANDER, (THRESHOLD, -20.0), (RATIO, 10.0), (KNEE, 0.0)];
    assert_eq!(curve_db(&gate, -50.0), -130.0);
    assert_eq!(effective(&settings(&gate)).curve.kind, CurveKind::Expand);
}

/// Gain in dB (output against input) for each sample of a positive input.
fn gain_trace(values: &[(u32, f32)], rate: f64, input: &[f32]) -> Vec<f32> {
    let output = run_mono(&mut *device(rate as f32, values), input);
    output
        .iter()
        .zip(input)
        .map(|(y, x)| db(f64::from(*y / *x)))
        .collect()
}

/// Samples the colour stage's oversampler remembers. After a step down to
/// a probe level its residual of the louder signal is still passing, so
/// output against input is not the gain until then.
const COLOUR_MEMORY: usize = 64;

/// A constant that steps between levels: `(level, seconds)` in order.
fn steps(rate: f64, parts: &[(f32, f64)]) -> (Vec<f32>, Vec<usize>) {
    let mut signal = Vec::new();
    let mut starts = Vec::new();
    for &(level, seconds) in parts {
        starts.push(signal.len());
        signal.resize(signal.len() + (seconds * rate) as usize, level);
    }
    (signal, starts)
}

/// Samples the gain takes from `start` to cover 63 % of its way to where it
/// is at `end`.
fn time_to_63(trace: &[f32], start: usize, end: usize) -> usize {
    time_to(&trace[start - 1..end], 1.0 - (-1.0_f32).exp())
}

fn assert_time(samples: usize, expected_ms: f64, rate: f64, percent: f64, label: &str) {
    let expected = expected_ms * 0.001 * rate;
    let tolerance = (expected * percent / 100.0).max(2.0);
    assert!(
        (samples as f64 - expected).abs() <= tolerance,
        "{label}: {samples} samples, expected {expected:.1} ± {tolerance:.1}"
    );
}

#[test]
fn digital_attack_and_release_take_their_set_times() {
    for rate in [44_100.0, 96_000.0] {
        for (attack, release) in [(1.0, 50.0), (10.0, 120.0), (100.0, 1000.0)] {
            let values = [(ATTACK, attack), (RELEASE, release)];
            let hold = f64::from(attack) * 0.012;
            let tail = f64::from(release) * 0.012;
            let (input, starts) = steps(rate, &[(0.05, 0.05), (0.5, hold), (0.05, tail)]);
            let trace = gain_trace(&values, rate, &input);
            let reached = trace[starts[2] - 1];
            assert!((reached + HALF_SCALE_REDUCTION).abs() < 0.01, "{reached}");
            let label = format!("{attack} / {release} ms at {rate} Hz");
            assert_time(
                time_to_63(&trace, starts[1], starts[2]),
                f64::from(attack),
                rate,
                2.0,
                &label,
            );
            assert_time(
                time_to_63(&trace, starts[2], trace.len()),
                f64::from(release),
                rate,
                2.0,
                &label,
            );
        }
    }
}

#[test]
fn feedback_attack_is_the_set_time_above_the_knee() {
    // A closed loop is faster by its loop gain, about the ratio; the rise
    // coefficient is divided by it.
    for ratio in [2.0, 4.0, 20.0] {
        let values = [
            (CHARACTER, FET),
            (KNEE, 0.0),
            (ATTACK, 10.0),
            (RATIO, ratio),
        ];
        let (input, starts) = steps(RATE, &[(0.05, 0.05), (0.5, 0.3)]);
        let trace = gain_trace(&values, RATE, &input);
        let samples = time_to_63(&trace, starts[1], starts[1] + 4800);
        assert_time(samples, 10.0, RATE, 5.0, &format!("{ratio}:1"));
    }
}

#[test]
fn feedback_attack_is_slower_inside_the_knee() {
    let values = [
        (CHARACTER, FET),
        (THRESHOLD, -18.0),
        (RATIO, 20.0),
        (KNEE, 12.0),
        (ATTACK, 10.0),
        (RELEASE, 2500.0),
    ];
    let time = |level_db: f32| {
        let (input, starts) = steps(RATE, &[(0.01, 0.05), (amplitude(level_db) as f32, 3.0)]);
        let trace = gain_trace(&values, RATE, &input);
        time_to_63(&trace, starts[1], trace.len())
    };
    let (inside, above) = (time(-21.0), time(-6.0));
    assert!(inside >= 3 * above, "inside {inside}, above {above}");
}

#[test]
fn feedback_release_is_the_set_time_once_the_loop_is_open() {
    let values = [
        (CHARACTER, FET),
        (THRESHOLD, -18.0),
        (RATIO, 8.0),
        (KNEE, 0.0),
        (RELEASE, 120.0),
    ];
    // Samples until `left` of the reduction above `settled` is left. Not
    // measured against where the trace ends: FET's slow cell holds a
    // little of the reduction for seconds.
    let time = |after: f32, settled: f32, left: f32| {
        let (input, starts) = steps(RATE, &[(0.5, 0.1), (after, 0.6)]);
        let trace = gain_trace(&values, RATE, &input);
        let start = -trace[starts[1] - 1];
        let aim = settled + (start - settled) * left;
        trace[starts[1]..]
            .iter()
            .skip(COLOUR_MEMORY)
            .position(|gain| -gain <= aim)
            .unwrap()
            + COLOUR_MEMORY
    };
    // Far below the threshold nothing feeds back.
    let e = (-1.0_f32).exp();
    assert_time(time(1e-4, 0.0, e), 120.0, RATE, 5.0, "to silence");
    // Still 6 dB over it, the gain falls as freely until the output is
    // back at the threshold, 0.56 of the release time; there the loop
    // closes and holds it. So the comparison is the time to settle.
    let over = amplitude(-12.0) as f32;
    let settled = -12.0 - curve_db(&values, -12.0);
    let (open, closed) = (time(1e-4, 0.0, 0.1), time(over, settled, 0.1));
    assert!(3 * closed <= open, "closed {closed}, open {open}");
}

#[test]
fn opto_cannot_attack_faster_than_ten_milliseconds() {
    let (input, starts) = steps(RATE, &[(0.05, 0.05), (0.5, 0.3)]);
    let trace = |character: f32, attack: f32| {
        gain_trace(
            &[(CHARACTER, character), (ATTACK, attack), (KNEE, 0.0)],
            RATE,
            &input,
        )
    };
    let held = trace(OPTO, 0.02);
    assert_eq!(held, trace(OPTO, 10.0));
    let time = |trace: &[f32]| time_to_63(trace, starts[1], starts[1] + 9600);
    let (opto, fet) = (time(&held), time(&trace(FET, 0.02)));
    assert!(opto >= 20 * fet, "Opto {opto}, FET {fet}");
    assert_eq!(
        effective(&settings(&[(CHARACTER, OPTO), (ATTACK, 0.02)])).attack_ms,
        10.0
    );
    assert_eq!(
        effective(&settings(&[(CHARACTER, TUBE), (ATTACK, 0.02)])).attack_ms,
        0.5
    );
}

/// Samples after a burst of `seconds` until the reduction has recovered by
/// each share.
fn recovery(values: &[(u32, f32)], seconds: f64, shares: [f32; 2], tail: f64) -> [usize; 2] {
    let (input, starts) = steps(RATE, &[(0.5, seconds), (1e-4, tail)]);
    let trace = gain_trace(values, RATE, &input);
    let end = starts[1];
    let deepest = trace[end - 1];
    assert!(deepest < -1.0, "{deepest}");
    shares.map(|share| {
        trace[end..]
            .iter()
            .skip(COLOUR_MEMORY)
            .position(|gain| *gain >= deepest * (1.0 - share))
            .unwrap_or_else(|| panic!("no {share} recovery within {tail} s"))
            + COLOUR_MEMORY
    })
}

#[test]
fn release_depends_on_the_program() {
    for (character, name, slower) in [(OPTO, "Opto", 3.0), (FET, "FET", 1.5)] {
        let values = [(CHARACTER, character), (RELEASE, 120.0)];
        let short = recovery(&values, 0.02, [0.5, 0.9], 1.0);
        let long = recovery(&values, 2.0, [0.5, 0.9], 6.0);
        // The first half of the way is the fast release either way.
        let ratio = long[0] as f64 / short[0] as f64;
        assert!((0.7..=1.3).contains(&ratio), "{name}: {short:?} {long:?}");
        // The rest waits for the slow cell, if the burst charged it.
        assert!(
            long[1] as f64 >= slower * short[1] as f64,
            "{name}: {short:?} {long:?}"
        );
    }
}

#[test]
fn auto_release_is_slower_after_sustained_reduction() {
    let values = [(AUTO_RELEASE, 1.0)];
    let short = recovery(&values, 0.02, [0.5, 0.9], 1.0);
    let long = recovery(&values, 2.0, [0.5, 0.9], 8.0);
    assert!(long[1] >= 2 * short[1], "{short:?} {long:?}");
    assert_eq!(effective(&settings(&values)).release_ms, 60.0);
    assert!(!applies(RELEASE, &settings(&values)));
}

#[test]
fn hard_settings_stay_stable() {
    // 20:1 with the shortest attack is where a loop with a unit delay
    // would oscillate.
    let mut random = Noise(5);
    for rate in [44_100.0_f32, 48_000.0, 96_000.0, 192_000.0] {
        for (character, name) in CHARACTER_NAMES {
            let values = [
                (CHARACTER, character),
                (RATIO, 20.0),
                (ATTACK, 0.02),
                (THRESHOLD, -30.0),
            ];
            let mut device = device(rate, &values);
            let telemetry = device.telemetry();
            for _ in 0..125 {
                // Up to +12 dBFS, with silences.
                let level = 4.0 * random.next().abs().powi(2);
                let mut left: Vec<f32> = (0..256).map(|_| level * random.next()).collect();
                let mut right: Vec<f32> = (0..256).map(|_| level * random.next()).collect();
                run(&mut *device, &mut left, &mut right);
                assert!(left.iter().chain(&right).all(|x| x.is_finite()));
                let reduction = telemetry.value(TELEMETRY_REDUCTION);
                assert!((0.0..=120.0).contains(&reduction), "{name}: {reduction}");
            }
            // On a steady tone the gain must not flip direction from sample
            // to sample.
            let rate = f64::from(rate);
            let input = sine(500.0, 2.0, rate, (0.2 * rate) as usize);
            // Both channels, as the blocks before differed between them.
            let (mut output, mut right) = (input.clone(), input.clone());
            run(&mut *device, &mut output, &mut right);
            let skip = input.len() / 2;
            let gains: Vec<f32> = output[skip..]
                .iter()
                .zip(&input[skip..])
                .filter(|(_, x)| x.abs() > 0.2)
                .map(|(y, x)| y / x)
                .collect();
            let flips = gains
                .windows(3)
                .filter(|w| (w[1] - w[0]) * (w[2] - w[1]) < 0.0)
                .count();
            assert!(
                flips * 10 <= gains.len(),
                "{name} at {rate}: {flips} of {}",
                gains.len()
            );
        }
    }
}

/// The input level in dB that gets `reduction_db` at `values`.
fn level_for_reduction(values: &[(u32, f32)], reduction_db: f32) -> f32 {
    let (mut low, mut high) = (-60.0_f32, 40.0);
    for _ in 0..60 {
        let middle = 0.5 * (low + high);
        if middle - curve_db(values, middle) < reduction_db {
            low = middle;
        } else {
            high = middle;
        }
    }
    high
}

#[test]
fn colour_grows_with_the_reduction() {
    for (character, name, harmonic) in [(TUBE, "Tube", 2), (FET, "FET", 3)] {
        // A slow release holds the gain still, so the harmonics are the
        // colour's and not the gain riding the waveform.
        let values = [
            (CHARACTER, character),
            // Low enough that 12 dB of reduction are reached below full
            // scale, where the bounded shapes level off.
            (THRESHOLD, -18.0),
            (RATIO, 20.0),
            (ATTACK, 1.0),
            (RELEASE, 2500.0),
        ];
        // 64 samples a period. Off such a grid the crests the detector
        // sees wander against the samples, and the gain with them: side
        // bands a few hertz from the tone, in every character, that are
        // not the colour's.
        let (frequency, bin) = on_bin(750.0, RATE);
        let mut levels = Vec::new();
        for reduction in [0.0, 3.0, 6.0, 12.0] {
            let level = if reduction == 0.0 {
                -30.0
            } else {
                level_for_reduction(&values, reduction)
            };
            let input = sine(frequency, amplitude(level), RATE, FFT + FFT / 2);
            let mut device = device(RATE as f32, &values);
            let output = run_mono(&mut *device, &input);
            let measured = device.telemetry().value(TELEMETRY_REDUCTION);
            assert!(
                (measured - reduction).abs() < 0.5,
                "{name}: {measured} for {reduction}"
            );
            let spectrum = spectrum(&output);
            levels.push(db(spectrum[harmonic * bin] / spectrum[bin]));
            // Nothing that is not a harmonic.
            let worst = (on_bin(20.0, RATE).1 + SKIRT..on_bin(20_000.0, RATE).1)
                .filter(|index| (index + SKIRT) % bin > 2 * SKIRT)
                .map(|index| spectrum[index])
                .fold(0.0, f64::max);
            let alias = db(worst / spectrum[bin]);
            assert!(
                alias <= -80.0,
                "{name} at {reduction} dB: alias {alias} dBc"
            );
        }
        assert!(levels[0] <= -80.0, "{name}: {levels:?}");
        assert!(levels[1] > levels[0] + 20.0, "{name}: {levels:?}");
        assert!(levels[2] > levels[1] + 3.0, "{name}: {levels:?}");
        assert!(levels[3] >= levels[2] - 0.2, "{name}: {levels:?}");
    }
}

#[test]
fn colour_does_not_thin_out_under_deep_reduction() {
    // The input stays at −6 dBFS and the threshold falls: the stage is
    // driven by the level into the gain element.
    for (character, name, harmonic, expected) in [(TUBE, "Tube", 2, -36.0), (FET, "FET", 3, -50.5)]
    {
        let (frequency, bin) = on_bin(750.0, RATE);
        let input = sine(frequency, 0.5, RATE, FFT + FFT / 2);
        let mut levels = Vec::new();
        let mut reductions = Vec::new();
        for threshold in [-12.0, -18.0, -24.0, -30.0] {
            let values = [
                (CHARACTER, character),
                (THRESHOLD, threshold),
                (RATIO, 20.0),
                (KNEE, 0.0),
                (ATTACK, 1.0),
                (RELEASE, 2500.0),
            ];
            let mut device = device(RATE as f32, &values);
            let output = run_mono(&mut *device, &input);
            reductions.push(device.telemetry().value(TELEMETRY_REDUCTION));
            let spectrum = spectrum(&output);
            levels.push(db(spectrum[harmonic * bin] / spectrum[bin]));
        }
        assert!(
            reductions[0] > 4.0 && reductions[3] > 20.0,
            "{name}: {reductions:?}"
        );
        for pair in levels.windows(2) {
            assert!(pair[1] >= pair[0] - 0.5, "{name}: {levels:?}");
        }
        assert!(
            (levels[3] - expected).abs() < 3.0,
            "{name}: {levels:?} against {expected}"
        );
    }
}

#[test]
fn expander_holds_its_curve_whatever_the_times() {
    // 3 dB above the threshold: the curve takes nothing away.
    let input = sine(1000.0, amplitude(-17.0), RATE, 3 * RATE as usize);
    for (ratio, knee, attack, release) in [
        (4.0, 0.0, 50.0, 50.0),
        (10.0, 0.0, 100.0, 50.0),
        (10.0, 0.0, 250.0, 20.0),
        (4.0, 6.0, 30.0, 120.0),
    ] {
        let values = [
            EXPANDER,
            (THRESHOLD, -20.0),
            (RATIO, ratio),
            (KNEE, knee),
            (ATTACK, attack),
            (RELEASE, release),
        ];
        let output = run_mono(&mut *device(RATE as f32, &values), &input);
        let settled = db(f64::from(peak(&output[output.len() - 4800..])));
        let drawn = curve_db(&values, -17.0);
        assert!(
            (settled - drawn).abs() < 0.1,
            "R {ratio} W {knee} A {attack} R {release}: {settled} against {drawn}"
        );
    }
}

#[test]
fn expander_opens_in_its_attack_time() {
    let values = [EXPANDER, (THRESHOLD, -20.0), (RATIO, 10.0), (ATTACK, 100.0)];
    let silence = (0.2 * RATE) as usize;
    let mut input = vec![0.0; silence];
    input.extend(sine(1000.0, 0.5, RATE, RATE as usize));
    let output = run_mono(&mut *device(RATE as f32, &values), &input);
    // Peaks of 5 ms windows against the tone's 0.5.
    let envelope: Vec<f32> = output[silence..]
        .chunks(240)
        .map(|window| peak(window) / 0.5)
        .collect();
    let share = 1.0 - (-1.0_f32).exp();
    let windows = envelope.iter().position(|level| *level >= share).unwrap();
    assert!((18..=22).contains(&windows), "{windows} windows of 5 ms");
    assert!(envelope[1] < 0.15, "{envelope:?}");
}

#[test]
fn one_huge_sample_is_forgotten() {
    let clean = sine(1000.0, 0.25, RATE, 4 * RATE as usize);
    let tail = clean.len() - RATE as usize / 2;
    let mut cases: Vec<(String, Values)> = CHARACTER_NAMES
        .iter()
        .map(|(character, name)| (name.to_string(), vec![(CHARACTER, *character)]))
        .collect();
    cases.push(("Limiter".into(), vec![LIMITER]));
    cases.push(("Expander".into(), vec![EXPANDER]));
    for (name, mut values) in cases {
        values.push((MAKEUP, 24.0));
        let expected = rms(&run_mono(&mut *device(RATE as f32, &values), &clean)[tail..]);
        for huge in [1e20, f32::MAX, -f32::MAX] {
            let mut input = clean.clone();
            input[1000] = huge;
            let output = run_mono(&mut *device(RATE as f32, &values), &input);
            assert!(output.iter().all(|x| x.is_finite()), "{name} {huge}");
            let settled = rms(&output[tail..]);
            assert!(
                (settled / expected - 1.0).abs() < 1e-3,
                "{name} {huge}: {settled} against {expected}"
            );
        }
    }
}

/// Peak of `signal` as an independent interpolator sees it at four times
/// the rate: a 255-tap windowed sinc.
fn peak_4x(signal: &[f32]) -> f32 {
    const HALF: isize = 127;
    let tap = |offset: f64| {
        if offset == 0.0 {
            return 1.0;
        }
        let x = std::f64::consts::PI * offset;
        let window = 0.5 * (1.0 + (x / (HALF as f64 / 4.0 + 1.0)).cos());
        x.sin() / x * window
    };
    let taps: Vec<[f64; 4]> = (-HALF / 4 - 1..=HALF / 4 + 1)
        .map(|n| std::array::from_fn(|phase| tap(n as f64 - phase as f64 / 4.0)))
        .collect();
    let reach = (taps.len() / 2) as isize;
    let mut peak = 0.0_f64;
    for center in reach..signal.len() as isize - reach {
        for phase in 0..4 {
            let mut sum = 0.0;
            for (index, tap) in taps.iter().enumerate() {
                let offset = index as isize - reach;
                sum += tap[phase] * f64::from(signal[(center - offset) as usize]);
            }
            peak = peak.max(sum.abs());
        }
    }
    peak as f32
}

#[test]
fn limiter_holds_its_ceiling() {
    let ceiling = -1.0_f32;
    let values = [LIMITER, (CEILING, ceiling), (KNEE, 0.0), (LOOKAHEAD, 5.0)];
    let limit = amplitude(ceiling) as f32;
    let frames = 24_000;
    let tone = |cycles_per_sample: f64, phase: f64, amplitude: f64| -> Vec<f32> {
        (0..frames)
            .map(|n| {
                let angle = std::f64::consts::TAU * cycles_per_sample * n as f64 + phase;
                (amplitude * angle.sin()) as f32
            })
            .collect()
    };
    // A sine at a quarter of the rate, 45° off its crests: +3 dBFS on the
    // samples, +6 dBFS between them.
    let between = tone(0.25, std::f64::consts::FRAC_PI_4, 2.0);
    let high = tone(0.40, 0.1, 1.0);
    let higher = tone(0.45, 0.1, 1.0);
    // Bursts of a square wave clipped from +6 dBFS.
    let square: Vec<f32> = (0..frames)
        .map(|n| {
            let on = (n / 3000) % 2 == 0;
            let wave = if (n / 37) % 2 == 0 { 2.0 } else { -2.0 };
            if on { wave } else { 0.0 }
        })
        .collect();
    // A tone that starts at full level is a burst at its first samples;
    // the tones are measured once they are tones.
    for (name, input, margin_db, from) in [
        ("between samples", between, 0.05, 2400),
        ("0.40 fs", high, 0.05, 2400),
        ("0.45 fs", higher, 0.05, 2400),
        ("square bursts", square, 0.3, 0),
    ] {
        assert!(peak_4x(&input) > limit, "{name} is over the ceiling");
        let output = run_mono(&mut *device(RATE as f32, &values), &input);
        let sample_peak = peak(&output);
        assert!(sample_peak <= limit, "{name}: sample peak {sample_peak}");
        let true_peak = db(f64::from(peak_4x(&output[from..])));
        assert!(
            true_peak <= ceiling + margin_db,
            "{name}: true peak {true_peak} dBFS"
        );
        // It limits; it does not just turn everything down.
        assert!(
            db(f64::from(sample_peak)) > ceiling - 4.0,
            "{name}: {sample_peak}"
        );
    }
}

#[test]
fn limiter_attack_ends_at_the_delayed_peak() {
    let values = [
        LIMITER,
        (CEILING, -6.0),
        (KNEE, 0.0),
        (LOOKAHEAD, 5.0),
        (RELEASE, 500.0),
    ];
    let mut device = device(RATE as f32, &values);
    let latency = device.latency_samples() as usize;
    assert_eq!(latency, 240 + TruePeak4x::DELAY as usize);
    let (input, starts) = steps(RATE, &[(0.1, 0.05), (1.0, 0.05)]);
    let output = run_mono(&mut *device, &input);
    let step = starts[1] + latency;
    let gain = |n: usize| output[n] / input[n - latency];
    let target = amplitude(-6.0) as f32;
    // Untouched before the lookahead, fully down when the step arrives:
    // to the ceiling and by what a band-limited step overshoots between
    // the samples, which the detector counts.
    assert_eq!(gain(step - 250), 1.0);
    let floor = gain(step);
    assert!(floor <= target && floor > 0.84 * target, "{floor}");
    let last = (step - 250..=step)
        .rev()
        .find(|n| gain(*n) > floor * 1.00001)
        .unwrap();
    // `last` still moves; the gain is down one sample before the step.
    assert!(step - last <= 2, "ramp ends {} samples early", step - last);
    // An S: flat at both ends, steepest in between.
    let slope = |n: usize| (gain(n + 1) - gain(n)).abs();
    let first = (step - 250..step).find(|n| gain(*n) < 1.0).unwrap();
    let steepest = (first..last).map(slope).fold(0.0, f32::max);
    assert!(slope(first) < 0.1 * steepest && slope(last - 1) < 0.1 * steepest);
    assert!((first..last).all(|n| gain(n + 1) <= gain(n)), "monotone");
}

#[test]
fn latency_follows_mode_character_and_lookahead() {
    for rate in [44_100.0_f32, 48_000.0, 96_000.0, 192_000.0] {
        let ms = |ms: f32| (ms * 0.001 * rate).round() as u32;
        for (name, values, expected) in [
            ("Digital", vec![(RATIO, 1.0)], 0),
            (
                "Digital 5 ms",
                vec![(LOOKAHEAD, 5.0), (RATIO, 1.0)],
                ms(5.0),
            ),
            // 0 means the limiter's own 5 ms.
            ("Limiter", vec![LIMITER], ms(5.0) + 20),
            (
                "Limiter 2 ms",
                vec![LIMITER, (LOOKAHEAD, 2.0)],
                ms(2.0) + 20,
            ),
            (
                "Expander 2 ms",
                vec![EXPANDER, (LOOKAHEAD, 2.0), (RATIO, 1.0)],
                ms(2.0),
            ),
            // A feedback detector cannot look ahead.
            (
                "FET",
                vec![(CHARACTER, FET), (LOOKAHEAD, 5.0), (RATIO, 1.0)],
                0,
            ),
        ] {
            let mut device = device(rate, &values);
            let label = format!("{name} at {rate} Hz");
            assert_eq!(device.latency_samples(), expected, "{label}");
            assert_eq!(latency(&settings(&values), rate), expected, "{label}");
            assert_eq!(
                device.tail_samples(),
                expected + (0.5 * rate) as u32,
                "{label}"
            );
            let mut input = vec![0.0; 3000];
            input[100] = 0.5;
            let output = run_mono(&mut *device, &input);
            let position = (0..output.len())
                .max_by(|a, b| output[*a].abs().total_cmp(&output[*b].abs()))
                .unwrap();
            assert_eq!(position, 100 + expected as usize, "{label}");
            assert_eq!(output[position], 0.5, "{label}");
        }
    }
}

#[test]
fn stereo_link_shares_the_reduction() {
    // Left is loud, right a quiet copy scaled by a power of two, so equal
    // gains give bit-equal products.
    let scale = 1.0 / 4096.0;
    let frames = 9600;
    let gains = |link: f32| {
        let mut left = vec![0.5_f32; frames];
        let mut right = vec![0.5 * scale; frames];
        run(
            &mut *device(RATE as f32, &[(STEREO_LINK, link)]),
            &mut left,
            &mut right,
        );
        (left, right)
    };
    let (left, right) = gains(100.0);
    assert!(left[frames - 1] < 0.2);
    assert!(left.iter().zip(&right).all(|(l, r)| *l * scale == *r));
    let (left, right) = gains(0.0);
    assert!(left[frames - 1] < 0.2);
    assert!(right.iter().all(|r| *r == 0.5 * scale));
    let (left, right) = gains(50.0);
    let (loud, quiet) = (
        db(f64::from(left[frames - 1] / 0.5)),
        db(f64::from(right[frames - 1] / (0.5 * scale))),
    );
    assert!((loud + HALF_SCALE_REDUCTION).abs() < 0.01, "{loud}");
    assert!(
        quiet < -1.0 && quiet > loud + 1.0,
        "{quiet} between 0 and {loud}"
    );
}

/// Reduction in dB once settled, as the panel's meter reads it.
fn settled_reduction(values: &[(u32, f32)], input: &[f32]) -> f32 {
    let mut device = device(RATE as f32, values);
    run_mono(&mut *device, input);
    device.telemetry().value(TELEMETRY_REDUCTION)
}

#[test]
fn sidechain_high_pass_ignores_the_bass() {
    let frames = 24_000;
    let input: Vec<f32> = sine(60.0, 0.5, RATE, frames)
        .iter()
        .zip(sine(2000.0, 0.05, RATE, frames))
        .map(|(low, high)| low + high)
        .collect();
    let values = |high_pass: f32| vec![(THRESHOLD, -30.0), (SIDECHAIN_HIGH_PASS, high_pass)];
    let (off, on) = (
        settled_reduction(&values(20.0), &input),
        settled_reduction(&values(200.0), &input),
    );
    assert!(on <= off - 6.0, "off {off} dB, high-passed {on} dB");
    assert!(on > 1.0, "the 2 kHz part still counts: {on}");
    // At the bottom of its range the filter is out exactly: the static
    // curve tests rely on it.
    assert!(!settings(&values(20.0)).high_pass_on);
    assert!(settings(&values(21.0)).high_pass_on);
}

#[test]
fn external_sidechain_ducks_the_main_signal() {
    let frames = 48_000;
    let main = sine(1000.0, 0.1, RATE, frames);
    // Kick-like bursts every 250 ms.
    let key: Vec<f32> = sine(60.0, 0.8, RATE, frames)
        .iter()
        .enumerate()
        .map(|(n, x)| if n % 12_000 < 2400 { *x } else { 0.0 })
        .collect();
    let values = [
        EXTERNAL,
        (THRESHOLD, -30.0),
        (RATIO, 8.0),
        (ATTACK, 1.0),
        (RELEASE, 20.0),
    ];
    let render = |values: &[(u32, f32)], key: Option<&[f32]>| {
        let mut device = device(RATE as f32, values);
        let (mut left, mut right) = (main.clone(), main.clone());
        run_keyed(
            &mut *device,
            &mut left,
            &mut right,
            key.map(|key| (key, key)),
        );
        (left, device.telemetry().value(TELEMETRY_SIDECHAIN))
    };
    let level =
        |output: &[f32], from: usize| db(rms(&output[from..from + 1200]) / rms(&main[..1200]));
    let (ducked, flag) = render(&values, Some(&key));
    assert_eq!(flag, 1.0);
    // During the second burst, and well after it.
    let (during, between) = (level(&ducked, 12_000 + 1000), level(&ducked, 12_000 + 9000));
    assert!(during < -12.0, "{during} dB during the burst");
    assert!(between > -0.5, "{between} dB between bursts");
    // Without a routed source there is nothing to follow.
    let (unrouted, flag) = render(&values, None);
    assert_eq!(flag, 0.0);
    assert_eq!(unrouted, main);
    // Internal ignores the key: the main signal is below the threshold,
    // the key far over it.
    let internal = [(THRESHOLD, -10.0), (RATIO, 8.0)];
    let (own, flag) = render(&internal, Some(&key));
    assert_eq!(flag, 0.0);
    assert_eq!(own, main);
    // A key shorter than the block is silence beyond its end.
    let mut device = device(RATE as f32, &values);
    let (mut left, mut right) = (main[..500].to_vec(), main[..500].to_vec());
    device.process(
        &Default::default(),
        &[],
        daw_devices::StereoIo {
            left: &mut left,
            right: &mut right,
            sidechain: Some((&key[..100], &key[..10])),
        },
    );
    assert!(left.iter().all(|x| x.is_finite()));
    // Every feedback character follows an external key feed-forward.
    for (character, name) in &CHARACTER_NAMES[1..] {
        let values = [values.as_slice(), &[(CHARACTER, *character)]].concat();
        let (ducked, _) = render(&values, Some(&key));
        let during = level(&ducked, 12_000 + 1500);
        assert!(during < -6.0, "{name}: {during} dB");
    }
}

#[test]
fn lookahead_catches_the_transient() {
    let silence = 4800;
    let mut input = vec![0.0; silence];
    input.extend(sine(1000.0, 0.5, RATE, 9600));
    let overshoot = |lookahead: f32| {
        let values = [
            (THRESHOLD, -24.0),
            (RATIO, 20.0),
            (KNEE, 0.0),
            (ATTACK, 5.0),
            (LOOKAHEAD, lookahead),
        ];
        let mut device = device(RATE as f32, &values);
        let start = silence + device.latency_samples() as usize;
        let output = run_mono(&mut *device, &input);
        db(f64::from(peak(&output[start..start + 240])))
    };
    let (without, with) = (overshoot(0.0), overshoot(5.0));
    assert!(
        with <= without - 6.0,
        "{without} dB without, {with} dB with"
    );
}

#[test]
fn auto_makeup_is_half_the_reduction_at_full_scale() {
    for values in [
        vec![],
        vec![(THRESHOLD, -40.0), (RATIO, 10.0), (KNEE, 0.0)],
        vec![(CHARACTER, TUBE), (THRESHOLD, -3.0), (KNEE, 2.0)],
    ] {
        let expected = 0.5 * (0.0 - curve_db(&values, 0.0));
        let makeup = auto_makeup_db(&settings(&values));
        assert!(
            (makeup - expected).abs() < 0.01,
            "{makeup} against {expected}"
        );
        // Measured below every knee, where nothing else changes the level.
        let input = sine(1000.0, 1e-3, RATE, 9600);
        let level = |auto: f32| {
            let values = [values.as_slice(), &[(AUTO_MAKEUP, auto)]].concat();
            let output = run_mono(&mut *device(RATE as f32, &values), &input);
            db(rms(&output[4800..]) / rms(&input[4800..]))
        };
        let measured = level(1.0) - level(0.0);
        assert!(
            (measured - makeup).abs() < 0.05,
            "{measured} against {makeup}"
        );
    }
    assert_eq!(auto_makeup_db(&settings(&[LIMITER])), 0.0);
    assert!((auto_makeup_db(&settings(&[])) - 6.75).abs() < 1e-4);
}

#[test]
fn characters_are_level_matched() {
    let noise = pink(RATE, 96_000, amplitude(-12.0));
    let levels = CHARACTER_NAMES.map(|(character, _)| {
        let output = run_mono(&mut *device(RATE as f32, &[(CHARACTER, character)]), &noise);
        db(rms(&output[24_000..]) / rms(&noise[24_000..]))
    });
    let spread = levels.iter().fold(f32::MIN, |a, b| a.max(*b))
        - levels.iter().fold(f32::MAX, |a, b| a.min(*b));
    assert!(spread <= 1.0, "{levels:?}");
    // And they do compress.
    assert!(levels.iter().all(|level| *level < -1.0), "{levels:?}");
}

#[test]
fn switches_do_not_click() {
    let frames = 36_000;
    let input = sine(100.0, 0.5, RATE, frames);
    let key = sine(100.0, 0.9, RATE, frames);
    let render = |from: &[(u32, f32)], to: Option<&[(u32, f32)]>| {
        let mut device = device(RATE as f32, from);
        let (mut left, mut right) = (input.clone(), input.clone());
        let half = frames / 2 + 137;
        let first = Some((&key[..half], &key[..half]));
        run_keyed(&mut *device, &mut left[..half], &mut right[..half], first);
        if let Some(to) = to {
            set(&mut *device, to);
        }
        let rest = Some((&key[half..], &key[half..]));
        run_keyed(&mut *device, &mut left[half..], &mut right[half..], rest);
        assert!(left.iter().all(|x| x.is_finite()));
        largest_step(&left[half - 2000..])
    };
    for (label, from, to) in [
        ("to Limiter", vec![COMPRESSOR], vec![LIMITER]),
        ("from Limiter", vec![LIMITER], vec![COMPRESSOR]),
        ("to Expander", vec![COMPRESSOR], vec![EXPANDER]),
        (
            "from Expander",
            vec![EXPANDER, (THRESHOLD, 0.0)],
            vec![COMPRESSOR],
        ),
        ("to FET", vec![(CHARACTER, DIGITAL)], vec![(CHARACTER, FET)]),
        ("to Tube", vec![(CHARACTER, FET)], vec![(CHARACTER, TUBE)]),
        (
            "Tube to FET",
            vec![(CHARACTER, TUBE)],
            vec![(CHARACTER, FET)],
        ),
        (
            "from Tube",
            vec![(CHARACTER, TUBE)],
            vec![(CHARACTER, DIGITAL)],
        ),
        (
            "Tube to External",
            vec![(CHARACTER, TUBE)],
            vec![(CHARACTER, TUBE), EXTERNAL],
        ),
        (
            "to Digital",
            vec![(CHARACTER, OPTO)],
            vec![(CHARACTER, DIGITAL)],
        ),
        (
            "Lookahead up",
            vec![(LOOKAHEAD, 0.0)],
            vec![(LOOKAHEAD, 10.0)],
        ),
        (
            "Lookahead down",
            vec![(LOOKAHEAD, 10.0)],
            vec![(LOOKAHEAD, 0.0)],
        ),
        ("to External", vec![(SIDECHAIN, 0.0)], vec![EXTERNAL]),
        ("to Internal", vec![EXTERNAL], vec![(SIDECHAIN, 0.0)]),
    ] {
        // Where it ends up: what was set before and is not changed stays.
        let after = [from.as_slice(), &to].concat();
        let limit = 2.0 * render(&from, None).max(render(&after, None));
        let step = render(&from, Some(&to));
        assert!(step <= limit, "{label}: {step} against {limit}");
    }
}

#[test]
fn silence_follows_the_tail() {
    let mut cases: Vec<(String, Values)> = CHARACTER_NAMES
        .iter()
        .map(|(character, name)| (name.to_string(), vec![(CHARACTER, *character)]))
        .collect();
    cases.push(("Limiter".into(), vec![LIMITER, (MAKEUP, 12.0)]));
    cases.push(("Expander".into(), vec![EXPANDER, (LOOKAHEAD, 3.0)]));
    cases.push(("Lookahead".into(), vec![(LOOKAHEAD, 10.0)]));
    for (name, values) in cases {
        let mut device = device(RATE as f32, &values);
        let tail = device.tail_samples() as usize;
        let burst = 12_000;
        let mut input = sine(90.0, 1.0, RATE, burst);
        input.resize(burst + 2 * 48_000 + 1000, 0.0);
        let output = run_mono(&mut *device, &input);
        assert!(peak(&output[..burst]) > 0.05, "{name}");
        let after_tail = &output[burst + tail..];
        if name == "Tube" {
            // The DC blocker of its colour is still ringing out.
            assert!(db(f64::from(peak(after_tail))) < -120.0, "{name}");
            assert!(
                output[burst + 2 * 48_000..].iter().all(|x| *x == 0.0),
                "{name}"
            );
        } else {
            assert!(after_tail.iter().all(|x| *x == 0.0), "{name}");
        }
        assert!(output.iter().all(|x| *x == 0.0 || x.is_normal()), "{name}");
    }
}

#[test]
fn the_ceiling_is_the_limiter_stage_only() {
    let input = sine(1000.0, 2.0, RATE, 24_000);
    let peak_db = |values: &[(u32, f32)]| {
        let output = run_mono(&mut *device(RATE as f32, values), &input);
        assert!(output.iter().all(|x| x.is_finite()));
        db(f64::from(peak(&output[12_000..])))
    };
    let ceiling = peak_db(&[LIMITER, (CEILING, -3.0)]);
    assert!((ceiling + 3.0).abs() < 0.01, "{ceiling}");
    // Output is the trim after every effect.
    let trimmed = peak_db(&[LIMITER, (CEILING, -3.0), (OUTPUT, 6.0)]);
    assert!((trimmed - 3.0).abs() < 0.01, "{trimmed}");
    // Mix blends the unlimited signal back in.
    let mixed = peak_db(&[LIMITER, (CEILING, -3.0), (MIX, 50.0)]);
    assert!(mixed > -3.0, "{mixed}");
    // Makeup drives the limiter instead of following it.
    let driven = peak_db(&[LIMITER, (CEILING, -3.0), (MAKEUP, 12.0)]);
    assert!((driven + 3.0).abs() < 0.01, "{driven}");
    let quiet = sine(1000.0, 0.05, RATE, 24_000);
    let output = run_mono(
        &mut *device(RATE as f32, &[LIMITER, (MAKEUP, 12.0)]),
        &quiet,
    );
    let gain = db(rms(&output[12_000..]) / rms(&quiet[12_000..]));
    assert!((gain - 12.0).abs() < 0.01, "{gain}");
}

#[test]
fn parameters_apply_per_mode() {
    let applying = |values: &[(u32, f32)]| -> Vec<u32> {
        let settings = settings(values);
        PARAMETERS
            .iter()
            .map(|spec| spec.id)
            .filter(|id| !applies(*id, &settings))
            .collect()
    };
    assert_eq!(applying(&[COMPRESSOR]), [CEILING]);
    assert_eq!(applying(&[(CHARACTER, OPTO)]), [LOOKAHEAD, CEILING]);
    assert_eq!(applying(&[(AUTO_RELEASE, 1.0)]), [RELEASE, CEILING]);
    assert_eq!(
        applying(&[LIMITER]),
        [
            CHARACTER,
            THRESHOLD,
            RATIO,
            ATTACK,
            SIDECHAIN,
            SIDECHAIN_HIGH_PASS,
            AUTO_MAKEUP
        ]
    );
    assert_eq!(
        applying(&[EXPANDER, (AUTO_RELEASE, 1.0)]),
        [CHARACTER, AUTO_RELEASE, CEILING, AUTO_MAKEUP]
    );
    // What the readouts show.
    let limiter = effective(&settings(&[LIMITER, (CHARACTER, FET), EXTERNAL]));
    assert_eq!(limiter.path, Path::Limiter);
    assert_eq!(limiter.curve.ratio, f32::INFINITY);
    assert_eq!(limiter.lookahead_ms, 5.0);
    assert_eq!(
        effective(&settings(&[(CHARACTER, FET), (LOOKAHEAD, 4.0)])).lookahead_ms,
        0.0
    );
    assert_eq!(
        effective(&settings(&[EXPANDER, (CHARACTER, FET)])).path,
        Path::Expander
    );
}

#[test]
fn telemetry_carries_level_reduction_and_history() {
    let mut device = device(RATE as f32, &[]);
    let telemetry = device.telemetry();
    assert_eq!(telemetry.value(TELEMETRY_SAMPLE_RATE), 48_000.0);
    // One second at −6.02 dBFS.
    run_mono(&mut *device, &vec![0.5; 48_000]);
    assert!((telemetry.value(TELEMETRY_REDUCTION) - HALF_SCALE_REDUCTION).abs() < 0.01);
    assert!((telemetry.value(TELEMETRY_LEVEL) + 6.02).abs() < 0.01);
    assert_eq!(telemetry.history_count(), HISTORY_RATE as usize);
    let mut columns = [[0.0; 3]; 8];
    telemetry.read_history(&mut columns);
    for [input, output, reduction] in columns {
        assert_eq!(input, 0.5);
        assert!((db(f64::from(output / input)) + HALF_SCALE_REDUCTION).abs() < 0.01);
        assert!((reduction - HALF_SCALE_REDUCTION).abs() < 0.01);
    }
}

#[test]
fn factory_presets_do_what_their_names_say() {
    let values = |name: &str| -> Values {
        let text = compressor::DESCRIPTOR
            .factory_presets
            .iter()
            .find(|text| Preset::parse(text).unwrap().name == name)
            .unwrap_or_else(|| panic!("no preset {name}"));
        let resolved = Preset::parse(text)
            .unwrap()
            .resolve(&compressor::DESCRIPTOR)
            .unwrap();
        resolved
            .values
            .iter()
            .map(|&(id, normalized)| {
                let spec = compressor::DESCRIPTOR.parameter(id).unwrap();
                (id, spec.to_plain(normalized))
            })
            .collect()
    };
    let loud = sine(200.0, 1.5, RATE, 24_000);
    for name in ["Brickwall Limiter", "Loud Limiter"] {
        let values = values(name);
        let ceiling = settings(&values).ceiling_db;
        let output = run_mono(&mut *device(RATE as f32, &values), &loud);
        assert!(peak(&output) <= amplitude(ceiling) as f32, "{name}");
        assert_eq!(latency(&settings(&values), 48_000.0), 260, "{name}");
    }
    let gate = values("Noise Gate");
    let hiss = sine(3000.0, amplitude(-70.0), RATE, 24_000);
    let output = run_mono(&mut *device(RATE as f32, &gate), &hiss);
    assert!(db(rms(&output[12_000..]) / rms(&hiss[12_000..])) < -40.0);
    assert!(settings(&values("Sidechain Duck")).external);
    for text in compressor::DESCRIPTOR.factory_presets {
        let name = Preset::parse(text).unwrap().name;
        let values = values(&name);
        let noise = pink(RATE, 12_000, 0.2);
        let output = run_mono(&mut *device(RATE as f32, &values), &noise);
        assert!(output.iter().all(|x| x.is_finite()), "{name}");
        assert!(
            rms(&output[6000..]) > 1e-4 || name == "Sidechain Duck",
            "{name}"
        );
    }
}
