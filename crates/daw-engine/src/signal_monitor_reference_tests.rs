//! Independent numerical oracles: FFmpeg's ebur128 filter, fed stereo f32 PCM.
//! Values are its 0.1-LU rounded settled output; tolerances include rounding and
//! the monitor's explicitly bounded histogram gate approximation.
use super::*;

fn feed(analysis: &mut Analysis, frames: usize, signal: impl Fn(u64) -> [f32; 2]) {
    let start = analysis.observed;
    let mut block = [[0.0; 2]; crate::render::BLOCK];
    let mut offset = 0;
    while offset < frames {
        let count = (frames - offset).min(block.len());
        for (index, frame) in block[..count].iter_mut().enumerate() {
            *frame = signal(start + (offset + index) as u64);
        }
        analysis.push(&block[..count]);
        offset += count;
    }
}

fn sine(frame: u64, rate: u32, frequency: f64, amplitude: f64) -> f32 {
    (amplitude * (2.0 * std::f64::consts::PI * frequency * frame as f64 / rate as f64).sin()) as f32
}

fn near(actual: Option<f32>, expected: f32, tolerance: f32, context: &str) {
    let actual = actual.unwrap_or_else(|| panic!("Missing {context}"));
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}: {actual} != {expected} ± {tolerance}"
    );
}

#[test]
fn k_weighted_loudness_matches_independent_ebur128_frequency_and_rate_oracles() {
    for rate in [44_100, 48_000, 96_000] {
        for (frequency, expected) in [(100.0, -21.8), (1000.0, -20.0), (10_000.0, -16.6)] {
            let mut analysis = Analysis::new(rate);
            feed(&mut analysis, rate as usize * 4, |frame| {
                [sine(frame, rate, frequency, 0.1); 2]
            });
            let snapshot = analysis.snapshot(SignalMonitorTarget::Master);
            let context = format!("{frequency} Hz at {rate} Hz");
            near(
                snapshot.lufs_momentary,
                expected,
                0.16,
                &format!("M {context}"),
            );
            near(
                snapshot.lufs_short_term,
                expected,
                0.16,
                &format!("S {context}"),
            );
            near(
                snapshot.lufs_integrated,
                expected,
                0.16,
                &format!("I {context}"),
            );
        }
    }
}

#[test]
fn relative_gate_excludes_long_quiet_passages_like_independent_ebur128() {
    // Four seconds at -20 LUFS, then four at -60 LUFS: the quiet passage is
    // above the absolute gate but below the relative gate. An ungated average
    // would finish near -23 LUFS; ebur128 finishes at -20.2 after overlap blocks.
    for rate in [44_100, 48_000, 96_000] {
        let mut analysis = Analysis::new(rate);
        feed(&mut analysis, rate as usize * 8, |frame| {
            let amplitude = if frame < u64::from(rate) * 4 {
                0.1
            } else {
                0.001
            };
            [sine(frame, rate, 1000.0, amplitude); 2]
        });
        let snapshot = analysis.snapshot(SignalMonitorTarget::Master);
        near(snapshot.lufs_momentary, -60.0, 0.16, "Quiet final M");
        near(snapshot.lufs_short_term, -60.0, 0.16, "Quiet final S");
        near(snapshot.lufs_integrated, -20.2, 0.16, "Relative-gated I");
    }
}

#[test]
fn absolute_gate_excludes_sustained_subthreshold_audio() {
    let rate = 48_000;
    let mut analysis = Analysis::new(rate);
    feed(&mut analysis, rate as usize * 4, |frame| {
        [sine(frame, rate, 1000.0, 0.00001); 2]
    });
    let snapshot = analysis.snapshot(SignalMonitorTarget::Master);
    near(
        snapshot.lufs_momentary,
        -100.0,
        0.16,
        "Subthreshold M remains measurable",
    );
    assert!(
        snapshot.lufs_integrated.is_none(),
        "Subthreshold blocks must not enter gated I"
    );
}

#[test]
fn loudness_windows_require_real_elapsed_samples_and_reset_without_old_energy() {
    let rate = 48_000;
    let mut analysis = Analysis::new(rate);
    feed(&mut analysis, 19_199, |frame| {
        [sine(frame, rate, 1000.0, 0.1); 2]
    });
    let early = analysis.snapshot(SignalMonitorTarget::Master);
    assert!(early.lufs_momentary.is_none());
    assert!(early.lufs_short_term.is_none());
    assert!(early.lufs_integrated.is_none());
    feed(&mut analysis, 1, |frame| {
        [sine(frame, rate, 1000.0, 0.1); 2]
    });
    let momentary = analysis.snapshot(SignalMonitorTarget::Master);
    near(momentary.lufs_momentary, -20.0, 0.16, "Exactly 400 ms");
    assert!(momentary.lufs_short_term.is_none());
    feed(&mut analysis, 144_000 - 19_200, |frame| {
        [sine(frame, rate, 1000.0, 0.1); 2]
    });
    near(
        analysis
            .snapshot(SignalMonitorTarget::Master)
            .lufs_short_term,
        -20.0,
        0.16,
        "Exactly 3 seconds",
    );
    analysis.reset();
    let reset = analysis.snapshot(SignalMonitorTarget::Master);
    assert_eq!(reset.observed_frames, 0);
    assert!(reset.lufs_momentary.is_none());
    assert!(reset.lufs_short_term.is_none());
    assert!(reset.lufs_integrated.is_none());
    assert!(reset.peak_dbfs.is_none());
    assert!(reset.rms_dbfs.is_none());
    assert!(reset.correlation.is_none());
    assert!(reset.phase.is_empty());
}

#[test]
fn stereo_power_preserves_antiphase_and_correlation_has_valid_polarity_and_quiet_states() {
    let rate = 48_000;
    for (right_sign, correlation) in [(1.0, 1.0), (-1.0, -1.0), (0.0, 0.0)] {
        let mut analysis = Analysis::new(rate);
        feed(&mut analysis, rate as usize * 4, |frame| {
            let left = sine(frame, rate, 1000.0, 0.1);
            [left, left * right_sign]
        });
        let snapshot = analysis.snapshot(SignalMonitorTarget::Master);
        near(
            snapshot.lufs_momentary,
            if right_sign == 0.0 { -23.0 } else { -20.0 },
            0.16,
            "Stereo channel weighting",
        );
        near(snapshot.peak_dbfs, -20.0, 0.02, "Sample peak");
        if right_sign == 0.0 {
            assert!(
                snapshot.correlation.is_none(),
                "One silent leg has undefined correlation"
            );
        } else {
            near(
                snapshot.correlation,
                correlation,
                0.0001,
                "Correlation polarity",
            );
        }
        let loudest = snapshot
            .spectrum_db
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            loudest > -25.0,
            "Antiphase audio must not cancel in the spectrum"
        );
    }
    let mut quadrature = Analysis::new(rate);
    feed(&mut quadrature, rate as usize, |frame| {
        [
            sine(frame, rate, 1000.0, 0.1),
            (0.1 * (2.0 * std::f64::consts::PI * 1000.0 * frame as f64 / rate as f64).cos()) as f32,
        ]
    });
    near(
        quadrature.snapshot(SignalMonitorTarget::Master).correlation,
        0.0,
        0.005,
        "Quadrature correlation",
    );
    let mut analysis = Analysis::new(rate);
    feed(&mut analysis, rate as usize, |_| [f32::NAN, f32::INFINITY]);
    let silent = analysis.snapshot(SignalMonitorTarget::Master);
    assert!(silent.lufs_momentary.is_none());
    assert!(silent.lufs_integrated.is_none());
    assert!(silent.correlation.is_none());
    assert!(silent.peak_dbfs.is_none());
    assert!(silent.rms_dbfs.is_none());
    assert!(silent.spectrum_db.iter().all(|value| value.is_finite()));
    assert!(silent.phase.iter().flatten().all(|value| value.is_finite()));
}

#[test]
fn filters_and_public_values_remain_finite_at_supported_sample_rate_extremes() {
    for rate in [8_000, 192_000] {
        let mut analysis = Analysis::new(rate);
        feed(&mut analysis, rate as usize, |frame| {
            [sine(frame, rate, 1000.0, 0.1); 2]
        });
        let snapshot = analysis.snapshot(SignalMonitorTarget::Master);
        assert_eq!(snapshot.sample_rate, rate);
        assert!(
            snapshot
                .lufs_momentary
                .is_some_and(|value| value.is_finite())
        );
        assert!(
            snapshot
                .lufs_integrated
                .is_some_and(|value| value.is_finite())
        );
        assert!(snapshot.peak_dbfs.is_some_and(|value| value.is_finite()));
        assert!(
            snapshot
                .correlation
                .is_some_and(|value| (-1.0..=1.0).contains(&value))
        );
        assert!(snapshot.spectrum_db.iter().all(|value| value.is_finite()));
    }
}

#[test]
fn phase_selection_preserves_real_quadrature_samples_without_fixed_stride_aliasing() {
    use std::collections::HashSet;

    let rate = 48_000;
    let signal = |frame: u64| {
        [
            sine(frame, rate, 1000.0, 0.1),
            (0.1 * (2.0 * std::f64::consts::PI * 1000.0 * frame as f64 / rate as f64).cos()) as f32,
        ]
    };
    let mut analysis = Analysis::new(rate);
    feed(&mut analysis, rate as usize, signal);
    let snapshot = analysis.snapshot(SignalMonitorTarget::Master);
    assert!(!snapshot.phase.is_empty());
    assert!(snapshot.phase.len() <= 256, "Phase display stays bounded");

    let input: HashSet<_> = (u64::from(rate) - FFT_SIZE as u64..u64::from(rate))
        .map(|frame| signal(frame).map(f32::to_bits))
        .collect();
    for point in &snapshot.phase {
        assert!(
            input.contains(&point.map(f32::to_bits)),
            "Displayed point must be an actual recent stereo sample"
        );
    }
    // A 1 kHz signal at 48 kHz has 48 physical phase positions. A fixed stride
    // of 16 collapses this ellipse to three points; retain most of the circle.
    let positions: HashSet<_> = snapshot
        .phase
        .iter()
        .map(|point| point.map(|value| (value * 10_000.0).round() as i32))
        .collect();
    assert!(
        positions.len() >= 40,
        "Only {} distinct phase positions survived",
        positions.len()
    );
}

#[test]
fn loudness_windows_recover_after_an_extreme_finite_transient_leaves_the_window() {
    let rate = 48_000;
    let mut analysis = Analysis::new(rate);
    analysis.push(&[[1e20, -1e20]]);
    feed(&mut analysis, rate as usize * 4, |frame| {
        [sine(frame, rate, 1000.0, 0.1); 2]
    });
    let snapshot = analysis.snapshot(SignalMonitorTarget::Master);
    near(
        snapshot.lufs_momentary,
        -20.0,
        0.16,
        "Momentary recovered after finite spike",
    );
    near(
        snapshot.lufs_short_term,
        -20.0,
        0.16,
        "Short-term recovered after finite spike",
    );
    // Integrated deliberately includes the historical spike and is not an
    // oracle for the level of the current 400 ms/3 s window.
    assert!(snapshot.spectrum_db.iter().all(|value| value.is_finite()));
}
