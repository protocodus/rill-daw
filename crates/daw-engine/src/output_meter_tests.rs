use super::*;

#[test]
fn master_retains_short_transients_across_silent_callbacks_without_changing_pcm() {
    let controls = Controls::new();
    controls.playing.store(true, Ordering::Release);
    let queue = AudioQueue::new(512);
    for frame in 0..256 {
        queue.push([0.8, -0.25], frame as f64 / 24_000.0, 0);
    }
    let mut output = [0.0_f32; 512];
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| {
            fill_output(&mut output, 2, &queue, &controls);
        }),
        0,
        "output telemetry must not allocate in the callback"
    );
    assert!(
        output
            .as_chunks::<2>()
            .0
            .iter()
            .all(|frame| *frame == [0.8, -0.25])
    );
    assert_eq!(controls.output_meter.peaks(), [0.8, 0.25]);

    fill_output(&mut output, 2, &queue, &controls);
    assert!(output.iter().all(|sample| *sample == 0.0));
    let release = release_factor(256, 48_000);
    assert_eq!(
        controls.output_meter.peaks(),
        [0.8 * release, 0.25 * release]
    );
    assert!(controls.output_meter.peaks()[0] > 0.78);
    assert_eq!(controls.underruns.load(Ordering::Relaxed), 1);
}

#[test]
fn master_measures_finite_clipped_output_and_releases_at_the_same_rate_for_each_buffer_size() {
    let mut releases = Vec::new();
    for block in [64, 128, 256, 512] {
        let controls = Controls::new();
        controls.playing.store(true, Ordering::Release);
        let queue = AudioQueue::new(1024);
        queue.push([f32::NAN, f32::INFINITY], 0.0, 0);
        queue.push([2.0, -3.0], 0.0, 0);
        let mut attack = [0.0; 4];
        fill_output(&mut attack, 2, &queue, &controls);
        assert_eq!(attack, [0.0, 0.0, 1.0, -1.0]);
        assert_eq!(controls.output_meter.peaks(), [1.0, 1.0]);
        let mut silence = vec![0.0; block * 2];
        for _ in 0..(48_000 / block) {
            fill_output(&mut silence, 2, &queue, &controls);
        }
        let elapsed_frames = (48_000 / block) * block;
        let expected = release_factor(elapsed_frames, 48_000);
        let actual = controls.output_meter.peaks();
        for peak in actual {
            assert!(
                (peak - expected).abs() < 2e-6,
                "block={block}: {actual:?} != {expected}"
            );
        }
        releases.push(actual[0]);
    }
    // Every case is within one callback of one second's 24 dB attenuation.
    let one_second = 10.0_f32.powf(-24.0 / 20.0);
    assert!(
        releases
            .iter()
            .all(|value| (value - one_second).abs() < 0.002)
    );
}
