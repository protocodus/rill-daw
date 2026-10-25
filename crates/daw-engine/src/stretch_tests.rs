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
    let source = tone(48000);
    for rate in [0.25, 0.5, 0.8, 1.0, 1.5, 2.0, 4.0] {
        let started = std::time::Instant::now();
        let result = prepare_stretched(&source, rate, None).unwrap();
        eprintln!(
            "1 s stereo stretch at source rate {rate}: {:?}",
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
            prepare_stretched(&source, rate, None).unwrap().samples
        );
    }
}

#[test]
fn native_stretch_compensates_transient_position_and_cancels_ongoing_preparation() {
    let mut source = tone(48000);
    source.samples.fill(0.0);
    for i in 24000..24048 {
        source.samples[i * 2] = 0.25;
        source.samples[i * 2 + 1] = 0.25;
    }
    for rate in [0.5, 2.0] {
        let output = prepare_stretched(&source, rate, None).unwrap();
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
            "rate {rate}: transient center {center}, expected {expected}"
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
        let error = prepare_stretched(&source, 0.25, Some(&cancel)).unwrap_err();
        assert!(error.to_string().contains("cancelled"), "{error:#}");
    });
}

#[test]
fn native_stretch_handles_short_silent_and_boundary_sources_with_bounded_errors() {
    for frames in [1, 10, 1024, 5000] {
        let mut source = tone(frames);
        source.samples.fill(0.0);
        for rate in [0.25, 4.0] {
            let silence = prepare_stretched(&source, rate, None).unwrap();
            assert_eq!(silence.frames(), (frames as f64 / rate).ceil() as usize);
            assert!(silence.samples.iter().all(|sample| *sample == 0.0));
        }
        source.samples[0] = 0.25;
        source.samples[(frames - 1) * 2] = 0.25;
        let result = prepare_stretched(&source, 0.5, None).unwrap();
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
        assert!(prepare_stretched(&source, rate, None).is_err());
    }
    assert!(prepare_stretched(&source, 0.5, Some(&AtomicBool::new(true))).is_err());
    assert_eq!(
        prepare_stretched(&source, 1.0, None).unwrap().samples,
        source.samples
    );
}
