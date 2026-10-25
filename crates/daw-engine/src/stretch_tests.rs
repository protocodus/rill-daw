use super::*;

fn tone(frames: usize) -> AudioData {
    AudioData {
        sample_rate: 48000,
        channels: 2,
        samples: (0..frames)
            .flat_map(|i| {
                let sample =
                    (std::f64::consts::TAU * 440.0 * i as f64 / 48000.0).sin() as f32 * 0.25;
                [sample, sample * 0.5]
            })
            .collect(),
    }
}

#[test]
fn native_stretch_preserves_pitch_duration_stereo_and_determinism() {
    verify_stretch(AudioProcessingAlgorithm::Signalsmith);
    verify_stretch(AudioProcessingAlgorithm::Bungee);
}

fn verify_stretch(algorithm: AudioProcessingAlgorithm) {
    let source = tone(48000);
    for rate in [0.25, 0.5, 0.8, 1.0, 1.5, 2.0, 4.0] {
        let started = std::time::Instant::now();
        let result = prepare_processed(&source, rate, 0.0, algorithm, None).unwrap();
        eprintln!(
            "{algorithm:?}: 1 s stereo stretch at source rate {rate}: {:?}",
            started.elapsed()
        );
        assert_eq!(result.frames(), (48000.0 / rate).ceil() as usize);
        let first = result.frames() / 4;
        let end = result.frames() * 3 / 4;
        let samples: Vec<_> = result
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .skip(first)
            .take(end - first)
            .map(|f| f[0])
            .collect();
        let crossings = samples
            .windows(2)
            .filter(|v| v[0] <= 0.0 && v[1] > 0.0)
            .count();
        let hz = crossings as f64 * 48000.0 / (end - first) as f64;
        assert!((hz - 440.0).abs() < 12.0, "rate {rate}: {hz}Hz");
        let rms = (samples.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>()
            / samples.len() as f64)
            .sqrt();
        assert!(rms > 0.10 && rms < 0.3, "rate {rate}: RMS {rms}");
        let stereo_error = result
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| (f[0] * 0.5 - f[1]).abs())
            .fold(0.0_f32, f32::max);
        // Spectral reconstruction introduces tiny per-channel float residuals,
        // particularly at extreme ratios. Bound them below -60 dBFS.
        assert!(
            stereo_error < 0.001,
            "rate {rate}: stereo error {stereo_error}"
        );
        assert_eq!(
            result.samples,
            prepare_processed(&source, rate, 0.0, algorithm, None)
                .unwrap()
                .samples
        );
    }
}

#[test]
fn native_stretch_compensates_transient_position_and_cancels_ongoing_preparation() {
    for algorithm in AudioProcessingAlgorithm::ALL {
        verify_transients_and_cancel(algorithm);
    }
}

fn verify_transients_and_cancel(algorithm: AudioProcessingAlgorithm) {
    let mut source = tone(48000);
    source.samples.fill(0.0);
    for i in 24000..24048 {
        source.samples[i * 2] = 0.25;
        source.samples[i * 2 + 1] = 0.25;
    }
    for rate in [0.5, 2.0] {
        let output = prepare_processed(&source, rate, 0.0, algorithm, None).unwrap();
        let (energy, weighted) = output.samples.as_chunks::<2>().0.iter().enumerate().fold(
            (0.0, 0.0),
            |(e, w), (i, f)| {
                let power = f64::from(f[0]).powi(2);
                (e + power, w + power * i as f64)
            },
        );
        assert!(energy > 0.001);
        let center = weighted / energy;
        let expected = 24024.0 / rate;
        assert!(
            (center - expected).abs() < 48000.0 * 0.04,
            "{algorithm:?} rate {rate}: transient center {center}, expected {expected}"
        );
    }
    // Long enough to remain active after the signal, without a real-time speed
    // assertion. Cancellation is observed between bounded native process calls.
    let source = tone(48000 * 20);
    let cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(1));
            cancel.store(true, Ordering::Release);
        });
        let error = prepare_processed(&source, 0.25, 7.0, algorithm, Some(&cancel)).unwrap_err();
        assert!(error.to_string().contains("cancelled"), "{error:#}");
    });
}

#[test]
fn native_stretch_handles_short_silent_and_boundary_sources_with_bounded_errors() {
    for algorithm in AudioProcessingAlgorithm::ALL {
        verify_boundaries(algorithm);
    }
}

fn verify_boundaries(algorithm: AudioProcessingAlgorithm) {
    for frames in [1, 10, 1024, 5000] {
        let mut source = tone(frames);
        source.samples.fill(0.0);
        for rate in [0.25, 4.0] {
            let silence = prepare_processed(&source, rate, 7.0, algorithm, None).unwrap();
            assert_eq!(silence.frames(), (frames as f64 / rate).ceil() as usize);
            assert!(silence.samples.iter().all(|sample| *sample == 0.0));
        }
        source.samples[0] = 0.25;
        source.samples[(frames - 1) * 2] = 0.25;
        let result = prepare_processed(&source, 0.5, 0.0, algorithm, None).unwrap();
        assert!(
            result
                .samples
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() < 1.0)
        );
        assert!(result.samples.iter().any(|sample| sample.abs() > 1e-6));
    }
    let source = tone(48000);
    for rate in [0.0, 0.1, 4.1, f64::NAN, f64::INFINITY] {
        assert!(prepare_processed(&source, rate, 0.0, algorithm, None).is_err());
    }
    for semitones in [-24.01, 24.01, f64::NAN, f64::INFINITY] {
        assert!(prepare_processed(&source, 1.0, semitones, algorithm, None).is_err());
    }
    assert!(prepare_stretched(&source, 0.5, Some(&AtomicBool::new(true))).is_err());
    assert_eq!(
        prepare_stretched(&source, 1.0, None).unwrap().samples,
        source.samples
    );
}

#[test]
fn time_and_pitch_are_independent_for_every_backend_including_fine_transposition() {
    let source = tone(48000);
    for algorithm in AudioProcessingAlgorithm::ALL {
        for rate in [0.5, 1.0, 2.0] {
            for semitones in [-24.0, -7.25, 7.25, 24.0] {
                let output = prepare_processed(&source, rate, semitones, algorithm, None).unwrap();
                assert_eq!(
                    output.frames(),
                    (source.frames() as f64 / rate).ceil() as usize
                );
                assert_eq!(output.channels, source.channels);
                assert_eq!(output.sample_rate, source.sample_rate);
                let central = &output.samples[output.frames() / 4 * 2..output.frames() * 3 / 4 * 2];
                let crossings = central
                    .as_chunks::<2>()
                    .0
                    .windows(2)
                    .filter(|frames| frames[0][0] <= 0.0 && frames[1][0] > 0.0)
                    .count();
                let hz = crossings as f64 * 48000.0 / (central.len() / 2) as f64;
                let expected = 440.0 * 2.0_f64.powf(semitones / 12.0);
                assert!(
                    (hz - expected).abs() < 12.0,
                    "{algorithm:?} rate {rate}, {semitones} st: {hz} Hz, expected {expected}"
                );
                let rms = (central
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|frame| f64::from(frame[0]).powi(2))
                    .sum::<f64>()
                    / (central.len() / 2) as f64)
                    .sqrt();
                assert!(
                    rms > 0.08 && rms < 0.4,
                    "{algorithm:?} rate {rate}, {semitones} st: RMS {rms}"
                );
                let stereo_error = central
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|frame| (frame[0] * 0.5 - frame[1]).abs())
                    .fold(0.0_f32, f32::max);
                assert!(
                    stereo_error < 0.001,
                    "{algorithm:?} rate {rate}, {semitones} st: stereo {stereo_error}"
                );
            }
        }
    }
}

#[test]
fn odd_mono_lengths_and_rates_are_exact_without_losing_stereo_separation() {
    for algorithm in AudioProcessingAlgorithm::ALL {
        let mono = AudioData {
            sample_rate: 44100,
            channels: 1,
            samples: (0..44101)
                .map(|i| (std::f32::consts::TAU * i as f32 * 997.0 / 44100.0).sin() * 0.2)
                .collect(),
        };
        let result = prepare_processed(&mono, 1.37, 3.01, algorithm, None).unwrap();
        assert_eq!(result.frames(), (44101.0_f64 / 1.37).ceil() as usize);
        assert_eq!(result.channels, 1);
        assert!(result.samples.iter().any(|sample| sample.abs() > 0.1));
        let mut stereo = tone(48000);
        for frame in stereo.samples.as_chunks_mut::<2>().0 {
            frame[1] = 0.0;
        }
        let result = prepare_processed(&stereo, 0.7, 9.0, algorithm, None).unwrap();
        assert!(
            result
                .samples
                .as_chunks::<2>()
                .0
                .iter()
                .all(|frame| frame[1].abs() < 1e-6)
        );
        let empty = AudioData {
            sample_rate: 48000,
            channels: 2,
            samples: vec![],
        };
        assert_eq!(
            prepare_processed(&empty, 0.5, 12.0, algorithm, None)
                .unwrap()
                .frames(),
            0
        );
    }
}
