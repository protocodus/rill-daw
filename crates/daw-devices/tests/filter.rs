use daw_devices::devices::filter::{self, Settings};
use daw_devices::{Device, ParamSpec, ProcessContext, StereoIo, descriptor};

fn set(device: &mut dyn Device, values: &[(u32, f32)]) {
    for &(id, plain) in values {
        let spec: &ParamSpec = filter::DESCRIPTOR.parameter(id).unwrap();
        device.set_parameter(id, spec.to_normalized(plain));
    }
}

fn settings(values: &[(u32, f32)]) -> Settings {
    Settings::from_normalized(|id| {
        let spec = filter::DESCRIPTOR.parameter(id).unwrap();
        values
            .iter()
            .find(|(key, _)| *key == id)
            .map_or(spec.default_normalized(), |(_, plain)| {
                spec.to_normalized(*plain)
            })
    })
}

/// Small enough that the saturating models stay close to linear.
const AMPLITUDE: f32 = 0.05;

/// Runs a sine through the device and returns the settled output.
fn render_sine(values: &[(u32, f32)], frequency: f32, amplitude: f32, rate: f32) -> Vec<f32> {
    let mut device = (descriptor("r1-filter").unwrap().create)();
    device.prepare(rate, 512);
    set(&mut *device, values);
    let total = (rate as usize) / 2;
    let mut left: Vec<f32> = (0..total)
        .map(|n| {
            (std::f64::consts::TAU * f64::from(frequency) * n as f64 / f64::from(rate)).sin() as f32
                * amplitude
        })
        .collect();
    let mut right = left.clone();
    for (l, r) in left.chunks_mut(512).zip(right.chunks_mut(512)) {
        device.process(
            &ProcessContext::default(),
            &[],
            StereoIo {
                left: l,
                right: r,
                sidechain: None,
            },
        );
    }
    left.split_off(total / 2)
}

/// Amplitude of `output` at `frequency`, by projection.
fn amplitude_at(output: &[f32], frequency: f32, rate: f32) -> f64 {
    let (mut i, mut q) = (0.0_f64, 0.0_f64);
    // Phase from the projection start; only the magnitude matters.
    for (n, y) in output.iter().enumerate() {
        let phase = std::f64::consts::TAU * f64::from(frequency) * n as f64 / f64::from(rate);
        i += f64::from(*y) * phase.sin();
        q += f64::from(*y) * phase.cos();
    }
    2.0 * (i * i + q * q).sqrt() / output.len() as f64
}

/// The device's gain at `frequency` in dB, for a small sine.
fn measured_gain_db(values: &[(u32, f32)], frequency: f32, rate: f32) -> f32 {
    let output = render_sine(values, frequency, AMPLITUDE, rate);
    let amplitude = amplitude_at(&output, frequency, rate) / f64::from(AMPLITUDE);
    (20.0 * amplitude.log10()) as f32
}

#[test]
fn clean_models_match_their_drawn_response_within_a_tenth_of_a_db() {
    use filter::*;
    let cases: &[&[(u32, f32)]] = &[
        &[(CUTOFF, 1000.0), (RESONANCE, 0.0)],
        &[(CUTOFF, 5000.0), (RESONANCE, 50.0)],
        &[(TYPE, 1.0), (CUTOFF, 300.0), (RESONANCE, 30.0)],
        &[(TYPE, 2.0), (CUTOFF, 2000.0), (RESONANCE, 60.0)],
        &[(SLOPE, 1.0), (CUTOFF, 3000.0), (RESONANCE, 40.0)],
        &[(MODEL, 6.0), (CUTOFF, 1500.0), (MORPH, 70.0)],
        &[
            (CUTOFF, 12_000.0),
            (RESONANCE, 20.0),
            (MIX, 50.0),
            (OUTPUT, -6.0),
        ],
    ];
    for rate in [44_100.0, 48_000.0] {
        for case in cases {
            let settings = settings(case);
            let frequencies = [50.0, 200.0, 1000.0, 3000.0, 8000.0, 15_000.0, 20_000.0];
            let mut drawn = [0.0; 7];
            response_db(&settings, settings.cutoff, rate, &frequencies, &mut drawn);
            for (frequency, drawn) in frequencies.iter().zip(drawn) {
                // Below −60 dB the measurement is noise, not the filter.
                if drawn < -60.0 {
                    continue;
                }
                let measured = measured_gain_db(case, *frequency, rate);
                assert!(
                    (measured - drawn).abs() < 0.1,
                    "{case:?} at {frequency} Hz, {rate} Hz: measured {measured}, drawn {drawn}"
                );
            }
        }
    }
}

#[test]
fn nonlinear_models_follow_their_drawn_shape() {
    use filter::*;
    // Small signal, so the tanh stages are close to linear; the half-band
    // filter and saturation allow a looser match.
    for case in [
        &[(MODEL, 1.0), (CUTOFF, 800.0), (RESONANCE, 30.0)][..],
        &[(MODEL, 2.0), (CUTOFF, 800.0), (RESONANCE, 30.0)][..],
        &[
            (MODEL, 2.0),
            (TYPE, 1.0),
            (CUTOFF, 800.0),
            (RESONANCE, 30.0),
        ][..],
        &[(MODEL, 3.0), (CUTOFF, 800.0), (RESONANCE, 30.0)][..],
        &[
            (MODEL, 3.0),
            (SLOPE, 1.0),
            (CUTOFF, 800.0),
            (RESONANCE, 30.0),
        ][..],
        &[(MODEL, 3.0), (TYPE, 1.0), (SLOPE, 1.0), (CUTOFF, 800.0)][..],
        &[(MODEL, 3.0), (TYPE, 2.0), (SLOPE, 1.0), (CUTOFF, 800.0)][..],
        &[
            (MODEL, 3.0),
            (TYPE, 3.0),
            (CUTOFF, 800.0),
            (RESONANCE, 20.0),
        ][..],
        &[(MODEL, 4.0), (CUTOFF, 800.0), (RESONANCE, 30.0)][..],
        &[
            (MODEL, 4.0),
            (SLOPE, 1.0),
            (CUTOFF, 800.0),
            (RESONANCE, 20.0),
        ][..],
        &[
            (MODEL, 4.0),
            (TYPE, 2.0),
            (CUTOFF, 800.0),
            (RESONANCE, 60.0),
        ][..],
        &[(MODEL, 4.0), (TYPE, 1.0), (SLOPE, 1.0), (CUTOFF, 800.0)][..],
        &[(MODEL, 5.0), (CUTOFF, 800.0), (RESONANCE, 30.0)][..],
        &[
            (MODEL, 5.0),
            (TYPE, 1.0),
            (CUTOFF, 800.0),
            (RESONANCE, 30.0),
        ][..],
        &[
            (MODEL, 5.0),
            (TYPE, 2.0),
            (CUTOFF, 800.0),
            (RESONANCE, 60.0),
        ][..],
    ] {
        let settings = settings(case);
        for frequency in [100.0, 800.0, 4000.0] {
            let mut drawn = [0.0];
            response_db(
                &settings,
                settings.cutoff,
                48_000.0,
                &[frequency],
                &mut drawn,
            );
            // Deep in the stop band the measurement is noise, not the filter.
            if drawn[0] < -60.0 {
                continue;
            }
            let measured = measured_gain_db(case, frequency, 48_000.0);
            assert!(
                (measured - drawn[0]).abs() < 1.5,
                "{case:?} at {frequency} Hz: measured {measured}, drawn {}",
                drawn[0]
            );
        }
    }
}

#[test]
fn dry_path_is_aligned_with_the_reported_latency() {
    let mut device = (descriptor("r1-filter").unwrap().create)();
    device.prepare(48_000.0, 256);
    set(&mut *device, &[(filter::MIX, 0.0)]);
    device.reset();
    let mut left = vec![0.0; 256];
    left[0] = 1.0;
    let mut right = left.clone();
    device.process(
        &ProcessContext::default(),
        &[],
        StereoIo {
            left: &mut left,
            right: &mut right,
            sidechain: None,
        },
    );
    let latency = device.latency_samples() as usize;
    assert!((left[latency] - 1.0).abs() < 1e-3, "{}", left[latency]);
    assert!(
        left.iter()
            .enumerate()
            .all(|(n, x)| n == latency || x.abs() < 1e-3)
    );
}

#[test]
fn every_model_low_passes_high_frequencies() {
    // Every model up to State Variable.
    for model in [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
        let values = [(filter::MODEL, model), (filter::CUTOFF, 500.0)];
        let low = measured_gain_db(&values, 100.0, 48_000.0);
        let high = measured_gain_db(&values, 8000.0, 48_000.0);
        assert!(high < low - 30.0, "model {model}: {low} dB vs {high} dB");
    }
}

#[test]
fn resonant_models_self_oscillate_and_stay_bounded() {
    // Every circuit model.
    for model in [1.0, 2.0, 3.0, 4.0, 5.0] {
        let peak = ring_out(&[
            (filter::MODEL, model),
            (filter::RESONANCE, 100.0),
            (filter::CUTOFF, 1000.0),
        ]);
        assert!((0.05..4.0).contains(&peak), "model {model}: {peak}");
    }
}

/// Peak output long after a single short burst.
fn ring_out(values: &[(u32, f32)]) -> f32 {
    let mut device = (descriptor("r1-filter").unwrap().create)();
    device.prepare(48_000.0, 256);
    set(&mut *device, values);
    let mut peak = 0.0_f32;
    for block in 0..200 {
        let mut left = vec![if block == 0 { 0.1 } else { 0.0 }; 256];
        let mut right = left.clone();
        device.process(
            &ProcessContext::default(),
            &[],
            StereoIo {
                left: &mut left,
                right: &mut right,
                sidechain: None,
            },
        );
        if block > 150 {
            peak = left.iter().fold(peak, |peak, x| peak.max(x.abs()));
        }
    }
    peak
}

#[test]
fn circuit_models_add_color_at_default_drive_and_clean_stays_clean() {
    // A −6 dBFS 1 kHz sine well below an 8 kHz cutoff, Drive at 0 dB: the
    // circuit models saturate it audibly, Clean leaves it alone.
    for model in [0.0, 1.0, 2.0, 3.0, 4.0, 5.0] {
        let output = render_sine(
            &[(filter::MODEL, model), (filter::CUTOFF, 8000.0)],
            1000.0,
            0.5,
            48_000.0,
        );
        let fundamental = amplitude_at(&output, 1000.0, 48_000.0);
        let third = amplitude_at(&output, 3000.0, 48_000.0);
        let level = 20.0 * (third / fundamental).log10();
        if model == 0.0 {
            assert!(level < -90.0, "Clean: third harmonic at {level} dB");
        } else {
            assert!(level > -50.0, "model {model}: third harmonic at {level} dB");
        }
    }
}

/// Runs a sine through the device, with `key` on the sidechain input when
/// given, and returns both settled channels.
fn render_keyed(
    values: &[(u32, f32)],
    frequency: f32,
    amplitude: f32,
    key: Option<f32>,
) -> (Vec<f32>, Vec<f32>) {
    const RATE: f32 = 48_000.0;
    let mut device = (descriptor("r1-filter").unwrap().create)();
    device.prepare(RATE, 512);
    set(&mut *device, values);
    let total = (RATE as usize) / 2;
    let sine = |amplitude: f32| -> Vec<f32> {
        (0..total)
            .map(|n| {
                (std::f64::consts::TAU * f64::from(frequency) * n as f64 / f64::from(RATE)).sin()
                    as f32
                    * amplitude
            })
            .collect()
    };
    let mut left = sine(amplitude);
    let mut right = left.clone();
    let key = key.map(sine);
    for (index, (l, r)) in left.chunks_mut(512).zip(right.chunks_mut(512)).enumerate() {
        let key = key.as_ref().map(|key| {
            let block = &key[index * 512..(index * 512 + l.len())];
            (block, block)
        });
        device.process(
            &ProcessContext::default(),
            &[],
            StereoIo {
                left: l,
                right: r,
                sidechain: key,
            },
        );
    }
    (left.split_off(total / 2), right.split_off(total / 2))
}

fn level_db(signal: &[f32]) -> f32 {
    let rms =
        (signal.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / signal.len() as f64).sqrt();
    (20.0 * rms.max(1e-12).log10()) as f32
}

#[test]
fn spread_puts_the_left_cutoff_below_and_the_right_above() {
    use filter::*;
    let values = [(CUTOFF, 1000.0), (RESONANCE, 0.0), (SPREAD, 2.0)];
    let (left, right) = render_keyed(&values, 1000.0, AMPLITUDE, None);
    // A clean 12 dB low-pass at 500 Hz and 2 kHz, read at 1 kHz.
    let (left, right) = (level_db(&left), level_db(&right));
    assert!(right - left > 8.0, "left {left:.1} dB, right {right:.1} dB");
    // At no spread both sides are the same filter.
    let (left, right) = render_keyed(&values[..2], 1000.0, AMPLITUDE, None);
    assert_eq!(left, right);
}

#[test]
fn envelope_follows_the_sidechain_when_external() {
    use filter::*;
    // Shut low-pass that a full-scale key opens by five octaves.
    let values = [
        (CUTOFF, 100.0),
        (RESONANCE, 0.0),
        (ENV_AMOUNT, 5.0),
        (ENV_SIDECHAIN, 1.0),
    ];
    let quiet_key = level_db(&render_keyed(&values, 2000.0, AMPLITUDE, Some(0.0)).0);
    let loud_key = level_db(&render_keyed(&values, 2000.0, AMPLITUDE, Some(1.0)).0);
    assert!(
        loud_key - quiet_key > 20.0,
        "{quiet_key:.1} → {loud_key:.1} dB"
    );
    // External with nothing routed listens to silence, not to the input.
    let unrouted = level_db(&render_keyed(&values, 2000.0, 1.0, None).0);
    let internal = level_db(&render_keyed(&values[..3], 2000.0, 1.0, None).0);
    assert!(
        internal - unrouted > 20.0,
        "{unrouted:.1} vs {internal:.1} dB"
    );
}
