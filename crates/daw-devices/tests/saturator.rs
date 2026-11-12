mod support;

use daw_devices::Device;
use daw_devices::devices::saturator::{self, *};
use daw_devices::dsp::{Shape, Shelf1};
use support::*;

const TRANSISTOR: f32 = 0.0;
const TUBE: f32 = 1.0;
const TAPE: f32 = 2.0;
const MODEL_NAMES: [(f32, &str); 3] = [(TRANSISTOR, "Transistor"), (TUBE, "Tube"), (TAPE, "Tape")];
const X2: f32 = 0.0;
const X4: f32 = 1.0;
const X8: f32 = 2.0;
/// Focus at the bottom of its range is off.
const FOCUS_OFF: f32 = 20.0;

fn device(rate: f32, values: &[(u32, f32)]) -> Box<dyn Device> {
    support::device(&saturator::DESCRIPTOR, rate, values)
}

fn settings(values: &[(u32, f32)]) -> Settings {
    Settings::from_normalized(value_of(&saturator::DESCRIPTOR, values))
}

/// Level in dB of the worst component from 20 Hz to `top` that is not a
/// harmonic of the tone, against the tone, for a −6 dBFS sine.
fn worst_alias_db(values: &[(u32, f32)], frequency: f64, rate: f64, top: f64) -> f32 {
    let (frequency, bin) = on_bin(frequency, rate);
    let input = sine(frequency, 0.5, rate, FFT + FFT / 4);
    let output = run_mono(&mut *device(rate as f32, values), &input);
    let spectrum = spectrum(&output);
    let mut harmonic = vec![false; spectrum.len()];
    for k in 1.. {
        let center = k * bin;
        if center >= spectrum.len() + SKIRT {
            break;
        }
        for near in center.saturating_sub(SKIRT)..=center + SKIRT {
            if let Some(slot) = harmonic.get_mut(near) {
                *slot = true;
            }
        }
    }
    let (low, high) = (on_bin(20.0, rate).1 + SKIRT, on_bin(top, rate).1);
    let worst = (low..=high)
        .filter(|bin| !harmonic[*bin])
        .map(|bin| spectrum[bin])
        .fold(0.0, f64::max);
    db(worst / spectrum[bin])
}

/// The roadmap's aliasing setting: Drive 18 dB, Character 50 %.
fn alias_setting(model: f32, quality: f32) -> Values {
    vec![(MODEL, model), (QUALITY, quality), (DRIVE, 18.0)]
}

const ALIAS_TONES: [f64; 5] = [1000.0, 2000.0, 3000.0, 4000.0, 5000.0];

fn worst_alias_over_tones(values: &[(u32, f32)]) -> f32 {
    worst_alias_at(values, 48_000.0, 20_000.0)
}

fn worst_alias_at(values: &[(u32, f32)], rate: f64, top: f64) -> f32 {
    ALIAS_TONES
        .iter()
        .map(|tone| worst_alias_db(values, *tone, rate, top))
        .fold(f32::MIN, f32::max)
}

#[test]
fn aliasing_stays_seventy_decibels_down_at_four_times() {
    for (model, name) in MODEL_NAMES {
        let worst = worst_alias_over_tones(&alias_setting(model, X4));
        assert!(worst <= -70.0, "{name}: {worst} dBc");
    }
}

#[test]
fn aliasing_at_44_1_khz_is_confined_above_18_khz() {
    // The first half-band stage stops from 25.65 kHz at this rate: what
    // lies between Nyquist and there folds to above 18.4 kHz at any
    // Quality. Below, the claim holds.
    for (model, name) in MODEL_NAMES {
        let worst = worst_alias_at(&alias_setting(model, X4), 44_100.0, 18_000.0);
        assert!(worst <= -70.0, "{name}: {worst} dBc");
    }
}

#[test]
fn quality_orders_the_aliasing() {
    let worst =
        [X2, X4, X8].map(|quality| worst_alias_over_tones(&alias_setting(TRANSISTOR, quality)));
    assert!(
        worst[1] <= worst[0] - 10.0 && worst[2] <= worst[1] - 10.0,
        "{worst:?}"
    );
}

#[test]
fn latency_is_forty_one_samples_at_every_quality_and_rate() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for quality in [X2, X4, X8] {
            let mut device = device(rate, &[(QUALITY, quality), (DRIVE, 0.0)]);
            assert_eq!(device.latency_samples(), 41);
            assert_eq!(saturator::LATENCY, 41);
            let mut input = vec![0.0; 400];
            input[100] = 1e-3;
            let output = run_mono(&mut *device, &input);
            let position = (0..output.len())
                .max_by(|a, b| output[*a].abs().total_cmp(&output[*b].abs()))
                .unwrap();
            assert_eq!(position, 141, "{rate} Hz, quality {quality}");
            assert!((output[position] / 1e-3 - 1.0).abs() < 0.05);
        }
    }
}

/// Gain in dB of small settled sines, two per run (one per channel).
fn small_signal_db(values: &[(u32, f32)], rate: f32, frequencies: &[f64]) -> Vec<f32> {
    let rate64 = f64::from(rate);
    let (skip, frames) = ((0.3 * rate64) as usize, (0.5 * rate64) as usize);
    let mut gains = Vec::new();
    for pair in frequencies.chunks(2) {
        let tones = [pair[0], pair[pair.len() - 1]];
        let inputs = tones.map(|tone| sine(tone, 1e-4, rate64, frames));
        let [mut left, mut right] = inputs.clone();
        run(&mut *device(rate, values), &mut left, &mut right);
        for ((output, input), tone) in [left, right]
            .iter()
            .zip(&inputs)
            .zip(tones)
            .take(pair.len())
        {
            gains.push(db(
                fit(&output[skip..], tone, rate64) / fit(&input[skip..], tone, rate64)
            ));
        }
    }
    gains
}

#[test]
fn dry_and_wet_are_aligned_to_the_sample() {
    // One sample of misalignment would read −11.7 dB at 20 kHz.
    let frequencies = [20.0, 100.0, 1000.0, 5000.0, 10_000.0, 15_000.0, 20_000.0];
    for quality in [X2, X4, X8] {
        let values = [
            (QUALITY, quality),
            (DRIVE, 0.0),
            (MIX, 50.0),
            (AUTO_GAIN, 0.0),
        ];
        let gains = small_signal_db(&values, 48_000.0, &frequencies);
        for (frequency, gain) in frequencies.iter().zip(gains) {
            assert!(
                gain.abs() < 0.25,
                "quality {quality}, {frequency} Hz: {gain} dB"
            );
        }
    }
}

#[test]
fn small_signals_pass_flat() {
    for (rate, top, tolerance) in [(48_000.0_f32, 20_000.0, 0.2), (44_100.0, 18_000.0, 0.3)] {
        let frequencies = [20.0, 40.0, 100.0, 300.0, 1000.0, 5000.0, 12_000.0, top];
        // Tape's head bump is part of its small-signal response.
        let bump = Shelf1::low_shelf(1.5, 90.0, rate);
        for (model, name) in MODEL_NAMES {
            for quality in [X2, X4, X8] {
                for focus in [FOCUS_OFF, 300.0] {
                    let values = [
                        (MODEL, model),
                        (QUALITY, quality),
                        (FOCUS, focus),
                        (DRIVE, 0.0),
                        (AUTO_GAIN, 0.0),
                    ];
                    let gains = small_signal_db(&values, rate, &frequencies);
                    // The clean band below Focus is scaled like the
                    // saturated one is for the −18 dBFS reference, so the
                    // balance of a real signal holds; for a signal this
                    // small that is a step of a few tenths of a dB (Auto
                    // gain moves both bands back together).
                    let clean = -auto_gain_db(&settings(&values));
                    assert!((-0.4..=0.05).contains(&clean), "{name}: {clean} dB");
                    for (frequency, gain) in frequencies.iter().zip(gains) {
                        let label =
                            format!("{name} {quality} focus {focus} {rate}: {frequency} Hz");
                        let omega = std::f64::consts::TAU * frequency / f64::from(rate);
                        let bump = if model == TAPE {
                            db(bump.response(omega).norm())
                        } else {
                            0.0
                        };
                        let (expected, tolerance) = if focus == FOCUS_OFF {
                            if *frequency == 20.0 {
                                // The 5 Hz DC blocker.
                                (bump - 0.26, 0.1)
                            } else {
                                (bump, tolerance)
                            }
                        } else if *frequency <= 100.0 {
                            // Clean: past the bump and the DC blocker.
                            (clean, tolerance)
                        } else if *frequency >= 1000.0 {
                            (0.0, tolerance)
                        } else {
                            // The crossover, between the two.
                            (0.5 * clean, tolerance + 0.5 * clean.abs())
                        };
                        assert!((gain - expected).abs() < tolerance, "{label}: {gain} dB");
                    }
                }
            }
        }
    }
}

/// Amplitudes of harmonics 1–`count` of a settled sine through the device.
fn measured_harmonics(
    values: &[(u32, f32)],
    frequency: f64,
    amplitude: f64,
    count: usize,
) -> Vec<f64> {
    let rate = 48_000.0;
    let frames = (1.0 * rate) as usize;
    let input = sine(frequency, amplitude, rate, frames);
    let output = run_mono(&mut *device(rate as f32, values), &input);
    // A whole number of periods, after the DC blocker has settled.
    let period = (rate / frequency).round() as usize;
    let window = &output[frames - 20 * period..];
    (1..=count)
        .map(|k| fit(window, k as f64 * frequency, rate))
        .collect()
}

/// Harmonics 1–10 of `transfer` over one cosine cycle of `peak` (0–1).
fn transfer_harmonics(settings: &Settings, peak: f64) -> [f64; 10] {
    const POINTS: usize = 4096;
    let input: Vec<f32> = (0..POINTS)
        .map(|n| (peak * (std::f64::consts::TAU * n as f64 / POINTS as f64).cos()) as f32)
        .collect();
    let mut curve = vec![0.0; POINTS];
    transfer(settings, &input, &mut curve);
    std::array::from_fn(|k| {
        let sum: f64 = curve
            .iter()
            .enumerate()
            .map(|(n, y)| {
                let theta = std::f64::consts::TAU * n as f64 / POINTS as f64;
                f64::from(*y) * ((k + 1) as f64 * theta).cos()
            })
            .sum();
        (2.0 * sum / POINTS as f64).abs()
    })
}

#[test]
fn the_drawn_curve_is_the_audio() {
    // Transistor has no bloom, bump or compression. Spectra, not waveforms:
    // the DC blocker removes the asymmetric curve's mean and the de-emphasis
    // rounds the edges, so samples cannot match a static curve.
    let values = [(MODEL, TRANSISTOR), (DRIVE, 12.0)];
    let settings = settings(&values);
    let measured = measured_harmonics(&values, 50.0, 0.5, 5);
    let drawn = transfer_harmonics(&settings, 0.5);
    let fundamental = db(measured[0] / drawn[0]);
    assert!(
        fundamental.abs() < 0.3,
        "fundamental off by {fundamental} dB"
    );
    let mut levels = [0.0; 10];
    let shaper_peak = 0.5 * 10.0_f32.powf(12.0 / 20.0);
    harmonics_db(&settings, shaper_peak, &mut levels);
    for k in 1..5 {
        let relative = db(measured[k] / measured[0]);
        if levels[k] > -60.0 {
            assert!(
                (relative - levels[k]).abs() < 1.0,
                "harmonic {}: measured {relative}, drawn {}",
                k + 1,
                levels[k]
            );
        }
    }
}

#[test]
fn harmonics_view_and_transfer_plot_show_the_same_curve() {
    for (model, name) in MODEL_NAMES {
        for character in [0.0, 50.0, 100.0] {
            for drive in [0.0, 12.0, 30.0] {
                let values = [(MODEL, model), (CHARACTER, character), (DRIVE, drive)];
                let settings = settings(&values);
                let from_transfer = transfer_harmonics(&settings, 0.8);
                let mut levels = [0.0; 10];
                let shaper_peak = 0.8 * 10.0_f32.powf(drive / 20.0);
                harmonics_db(&settings, shaper_peak, &mut levels);
                assert_eq!(levels[0], 0.0);
                for k in 1..10 {
                    let relative = db(from_transfer[k] / from_transfer[0]);
                    if levels[k] > -80.0 {
                        assert!(
                            (relative - levels[k]).abs() < 0.1,
                            "{name} {character} {drive}, harmonic {}: {relative} against {}",
                            k + 1,
                            levels[k]
                        );
                    }
                    assert!(levels[k] >= -100.0);
                }
                // The audio path and both functions use the one mapping.
                let shape = curve(settings.model, settings.character);
                let gain = if settings.auto_gain {
                    10.0_f64.powf(f64::from(auto_gain_db(&settings)) / 20.0)
                } else {
                    1.0
                };
                let mut out = [0.0];
                transfer(&settings, &[0.37], &mut out);
                let expected = shape.value(0.37 * 10.0_f64.powf(f64::from(drive) / 20.0)) * gain;
                assert!((f64::from(out[0]) - expected).abs() < 1e-5 * expected.abs().max(1.0));
            }
        }
    }
}

fn assert_rising(values: &[f32], label: &str) {
    assert!(
        values.windows(2).all(|pair| pair[1] > pair[0]),
        "{label} is not rising: {values:?}"
    );
}

#[test]
fn character_does_what_its_caption_says() {
    let steps = [0.0, 25.0, 50.0, 75.0, 100.0];
    // Drive 6 on a −6 dBFS sine: a shaper peak of 1.0, the curves' knee. The
    // level is part of the claims.
    let harmonics = |model: f32, character: f32| {
        measured_harmonics(
            &[(MODEL, model), (CHARACTER, character), (DRIVE, 6.0)],
            100.0,
            0.5,
            5,
        )
    };
    // Tube bias: the second harmonic grows.
    let second: Vec<f32> = steps
        .iter()
        .map(|character| {
            let h = harmonics(TUBE, *character);
            db(h[1] / h[0])
        })
        .collect();
    assert_rising(&second, "Tube second harmonic");
    assert!(second[4] - second[0] > 10.0, "{second:?}");
    // Transistor hardness: the upper harmonics catch up with the third.
    let edge: Vec<f32> = steps
        .iter()
        .map(|character| {
            let h = harmonics(TRANSISTOR, *character);
            db(h[4] / h[2])
        })
        .collect();
    assert_rising(&edge, "Transistor fifth against third");
    assert!(edge[4] - edge[0] > 20.0, "{edge:?}");
}

#[test]
fn tape_character_compresses() {
    // Loud and quiet passages of the same tone: the level between them
    // shrinks as Character rises, and at 0 % the stage is out of the way.
    let rate = 48_000.0;
    let frames = (1.2 * rate) as usize;
    let half = frames / 2;
    let tone = sine(220.0, 1.0, rate, frames);
    let input: Vec<f32> = tone
        .iter()
        .enumerate()
        .map(|(n, x)| x * if n < half { 0.02 } else { 0.5 })
        .collect();
    let range = |character: f32| {
        let values = [
            (MODEL, TAPE),
            (CHARACTER, character),
            (DRIVE, 6.0),
            (AUTO_GAIN, 0.0),
        ];
        let output = run_mono(&mut *device(rate as f32, &values), &input);
        let quarter = half / 2;
        db(rms(&output[half + quarter..]) / rms(&output[quarter..half]))
    };
    let ranges: Vec<f32> = [0.0, 25.0, 50.0, 75.0, 100.0].map(range).to_vec();
    assert!(
        ranges.windows(2).all(|pair| pair[1] < pair[0]),
        "{ranges:?}"
    );
    assert!(ranges[0] - ranges[4] > 2.0, "{ranges:?}");
}

#[test]
fn tape_saturates_highs_before_lows() {
    // The fundamental's level, not THD: the curve is odd and the third
    // harmonic of 8 kHz is at Nyquist.
    let rate = 48_000.0_f32;
    let bump = Shelf1::low_shelf(1.5, 90.0, rate);
    let level = |frequency: f64, amplitude: f64| {
        let values = [(MODEL, TAPE), (CHARACTER, 0.0), (DRIVE, 12.0)];
        let frames = 24_000;
        let input = sine(frequency, amplitude, f64::from(rate), frames);
        let output = run_mono(&mut *device(rate, &values), &input);
        let gain = fit(&output[frames / 2..], frequency, f64::from(rate))
            / fit(&input[frames / 2..], frequency, f64::from(rate));
        let omega = std::f64::consts::TAU * frequency / f64::from(rate);
        db(gain / bump.response(omega).norm())
    };
    let driven = level(8000.0, 0.5) - level(200.0, 0.5);
    assert!(driven <= -3.0, "driven: {driven} dB");
    let small = level(8000.0, 1e-4) - level(200.0, 1e-4);
    assert!(small.abs() < 0.2, "small: {small} dB");
}

#[test]
fn focus_keeps_the_bass_clean() {
    let rate = 48_000.0;
    let frames = 48_000;
    let input: Vec<f32> = sine(60.0, 0.25, rate, frames)
        .iter()
        .zip(sine(2000.0, 0.25, rate, frames))
        .map(|(low, high)| low + high)
        .collect();
    // Harmonics 2–5 of the 60 Hz component against it.
    let distortion = |focus: f32| {
        let values = [(DRIVE, 24.0), (FOCUS, focus)];
        let output = run_mono(&mut *device(rate as f32, &values), &input);
        let window = &output[frames - 16_000..];
        let harmonics: f64 = (2..=5)
            .map(|k| fit(window, 60.0 * k as f64, rate).powi(2))
            .sum();
        db(harmonics.sqrt() / fit(window, 60.0, rate))
    };
    let (off, on) = (distortion(FOCUS_OFF), distortion(250.0));
    assert!(on <= off - 20.0, "off {off} dB, focused {on} dB");
}

/// Output level against input level in dB for pink noise at −18 dBFS RMS.
fn noise_gain_db(values: &[(u32, f32)], noise: &[f32]) -> f32 {
    let output = run_mono(&mut *device(48_000.0, values), noise);
    let skip = noise.len() / 4;
    db(rms(&output[skip..]) / rms(&noise[skip..]))
}

fn reference_noise() -> Vec<f32> {
    pink(48_000.0, 40_000, 10.0_f64.powf(-18.0 / 20.0))
}

#[test]
fn auto_gain_holds_the_level() {
    let noise = reference_noise();
    for (model, name) in MODEL_NAMES {
        let tolerance = if model == TAPE { 2.0 } else { 1.0 };
        for drive in [0.0, 6.0, 12.0, 18.0, 24.0, 30.0, 36.0] {
            for character in [0.0, 50.0, 100.0] {
                let values = [(MODEL, model), (DRIVE, drive), (CHARACTER, character)];
                let gain = noise_gain_db(&values, &noise);
                assert!(
                    gain.abs() <= tolerance,
                    "{name}, Drive {drive}, Character {character}: {gain} dB"
                );
                // The readout is the gain that was applied.
                let off = [values.as_slice(), &[(AUTO_GAIN, 0.0)]].concat();
                if drive == 18.0 {
                    let applied = gain - noise_gain_db(&off, &noise);
                    let readout = auto_gain_db(&settings(&values));
                    assert!(
                        (applied - readout).abs() < 0.05,
                        "{name}: {applied} {readout}"
                    );
                }
            }
        }
    }
}

#[test]
fn models_are_level_matched() {
    let noise = reference_noise();
    let gains = MODEL_NAMES
        .map(|(model, _)| noise_gain_db(&[(MODEL, model), (DRIVE, 6.0), (AUTO_GAIN, 0.0)], &noise));
    let spread = gains.iter().fold(f32::MIN, |a, b| a.max(*b))
        - gains.iter().fold(f32::MAX, |a, b| a.min(*b));
    assert!(spread <= 1.5, "{gains:?}");
}

#[test]
fn biased_curves_leave_no_dc() {
    let rate = 48_000.0;
    let values = [(MODEL, TUBE), (CHARACTER, 100.0), (DRIVE, 24.0)];
    let input = sine(1000.0, 0.5, rate, 48_000);
    let output = run_mono(&mut *device(rate as f32, &values), &input);
    // Whole periods of the tone.
    let window = &output[24_000..];
    let mean = window.iter().map(|x| f64::from(*x)).sum::<f64>() / window.len() as f64;
    assert!(db(mean.abs()) < -60.0, "{} dBFS", db(mean.abs()));
}

#[test]
fn silence_follows_the_tail() {
    let rate = 48_000.0;
    let values = [(MODEL, TUBE), (CHARACTER, 100.0), (DRIVE, 36.0)];
    let mut device = device(rate as f32, &values);
    let tail = device.tail_samples() as usize;
    assert_eq!(tail, 41 + 24_000);
    let burst = 12_000;
    let mut input = sine(90.0, 1.0, rate, burst);
    input.resize(burst + 2 * 48_000 + 1000, 0.0);
    let output = run_mono(&mut *device, &input);
    assert!(peak(&output[..burst]) > 0.1);
    let after_tail = peak(&output[burst + tail..]);
    assert!(db(f64::from(after_tail)) < -120.0, "{after_tail}");
    assert!(output[burst + 2 * 48_000..].iter().all(|x| *x == 0.0));
}

#[test]
fn switches_and_jumps_do_not_click() {
    let rate = 48_000.0;
    let frames = 36_000;
    let input = sine(100.0, 0.5, rate, frames);
    let steady = |values: &[(u32, f32)]| {
        let output = run_mono(&mut *device(rate as f32, values), &input);
        largest_step(&output[frames / 2..])
    };
    for (label, from, to) in [
        ("Model to Tape", vec![(MODEL, TUBE)], vec![(MODEL, TAPE)]),
        (
            "Model to Transistor",
            vec![(MODEL, TAPE)],
            vec![(MODEL, TRANSISTOR)],
        ),
        ("Quality up", vec![(QUALITY, X2)], vec![(QUALITY, X8)]),
        ("Quality down", vec![(QUALITY, X8)], vec![(QUALITY, X4)]),
        ("Drive up", vec![(DRIVE, 0.0)], vec![(DRIVE, 36.0)]),
        ("Drive down", vec![(DRIVE, 36.0)], vec![(DRIVE, 0.0)]),
        ("Focus on", vec![(FOCUS, FOCUS_OFF)], vec![(FOCUS, 1000.0)]),
        ("Focus off", vec![(FOCUS, 1000.0)], vec![(FOCUS, FOCUS_OFF)]),
        (
            "Auto gain",
            vec![(DRIVE, 24.0)],
            vec![(DRIVE, 24.0), (AUTO_GAIN, 0.0)],
        ),
    ] {
        let limit = 2.0 * steady(&from).max(steady(&to));
        let mut device = device(rate as f32, &from);
        let (mut left, mut right) = (input.clone(), input.clone());
        let half = frames / 2 + 137;
        run(&mut *device, &mut left[..half], &mut right[..half]);
        set(&mut *device, &to);
        run(&mut *device, &mut left[half..], &mut right[half..]);
        // From before the change to well after every ramp.
        let step = largest_step(&left[half - 2000..]);
        assert!(step <= limit, "{label}: {step} against {limit}");
        assert!(left.iter().all(|x| x.is_finite()));
    }
}

#[test]
fn nonfinite_input_does_not_spread() {
    let mut device = device(48_000.0, &[(DRIVE, 20.0)]);
    let mut input = sine(300.0, 0.5, 48_000.0, 4000);
    input[1000] = f32::NAN;
    input[1001] = f32::INFINITY;
    let output = run_mono(&mut *device, &input);
    assert!(output.iter().all(|x| x.is_finite()));
}

#[test]
fn telemetry_reports_the_shaper_level() {
    let rate = 48_000.0;
    let mut device = device(rate as f32, &[(DRIVE, 12.0)]);
    let telemetry = device.telemetry();
    run_mono(&mut *device, &sine(300.0, 0.25, rate, 9600));
    let level = telemetry.value(TELEMETRY_LEVEL);
    // 0.25 driven by 12 dB.
    assert!((level - 0.995).abs() < 0.01, "{level}");
    assert_eq!(telemetry.value(TELEMETRY_SAMPLE_RATE), 48_000.0);
    // Held, with a fall of 300 ms.
    run_mono(&mut *device, &vec![0.0; 14_400]);
    let held = telemetry.value(TELEMETRY_LEVEL);
    assert!((held / level - (-1.0_f32).exp()).abs() < 0.02, "{held}");
}

#[test]
fn one_huge_sample_is_forgotten() {
    let rate = 48_000.0;
    let clean = sine(1000.0, 0.25, rate, 4 * rate as usize);
    let tail = clean.len() - rate as usize / 2;
    for (model, name) in MODEL_NAMES {
        let values = [(MODEL, model), (DRIVE, 36.0)];
        let expected = rms(&run_mono(&mut *device(rate as f32, &values), &clean)[tail..]);
        for huge in [1e20, f32::MAX, -f32::MAX] {
            let mut input = clean.clone();
            input[1000] = huge;
            let output = run_mono(&mut *device(rate as f32, &values), &input);
            assert!(output.iter().all(|x| x.is_finite()), "{name} {huge}");
            let settled = rms(&output[tail..]);
            assert!(
                (settled / expected - 1.0).abs() < 1e-3,
                "{name} {huge}: {settled} against {expected}"
            );
        }
    }
}
