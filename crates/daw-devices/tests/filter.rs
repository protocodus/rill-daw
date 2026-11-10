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

/// Runs a sine through the device and returns its gain in dB, measured by
/// projecting the settled output onto the input frequency.
fn measured_gain_db(values: &[(u32, f32)], frequency: f32, rate: f32) -> f32 {
    let mut device = (descriptor("r1-filter").unwrap().create)();
    device.prepare(rate, 512);
    set(&mut *device, values);
    let total = (rate as usize) / 2;
    let mut left: Vec<f32> = (0..total)
        .map(|n| {
            (std::f64::consts::TAU * f64::from(frequency) * n as f64 / f64::from(rate)).sin() as f32
                * AMPLITUDE
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
    let start = total / 2;
    let (mut i, mut q) = (0.0_f64, 0.0_f64);
    for (n, y) in left.iter().enumerate().skip(start) {
        let phase = std::f64::consts::TAU * f64::from(frequency) * n as f64 / f64::from(rate);
        i += f64::from(*y) * phase.sin();
        q += f64::from(*y) * phase.cos();
    }
    let amplitude = 2.0 * (i * i + q * q).sqrt() / (total - start) as f64 / f64::from(AMPLITUDE);
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
            (MODEL, 4.0),
            (SLOPE, 1.0),
            (CUTOFF, 2000.0),
            (RESONANCE, 20.0),
        ],
        &[
            (MODEL, 4.0),
            (TYPE, 2.0),
            (CUTOFF, 800.0),
            (RESONANCE, 30.0),
        ],
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
    // Clean, M, K, X, V, SP and State Variable.
    for model in [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
        let values = [(filter::MODEL, model), (filter::CUTOFF, 500.0)];
        let low = measured_gain_db(&values, 100.0, 48_000.0);
        let high = measured_gain_db(&values, 8000.0, 48_000.0);
        assert!(high < low - 30.0, "model {model}: {low} dB vs {high} dB");
    }
}

#[test]
fn resonant_models_self_oscillate_and_stay_bounded() {
    // M, K, X and SP.
    for model in [1.0, 2.0, 3.0, 5.0] {
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
