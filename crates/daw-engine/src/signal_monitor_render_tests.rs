use super::*;
use crate::{SignalMonitorTarget, signal_monitor::SignalMonitor};
use daw_core::Clip;
use std::time::{Duration, Instant};

fn fixture() -> (tempfile::TempDir, Project, [u64; 3]) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("signal.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [0.8, 0.8]).collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    let bus = project.add_bus("Bus");
    let send = project.add_bus("Send return");
    let track = project.track_mut(source).unwrap();
    track.gain = 0.5;
    track.pan = -0.5;
    track
        .clips
        .push(Clip::audio("Signal", 0.0, 2.0, path.to_string_lossy()));
    project.track_mut(bus).unwrap().gain = 0.25;
    project.track_mut(send).unwrap().gain = 0.5;
    project.set_output(source, Some(bus)).unwrap();
    project.add_send(source, send, 0.5, true).unwrap();
    (directory, project, [source, bus, send])
}

fn capture(
    renderer: &mut Renderer,
    monitor: &SignalMonitor,
    target: SignalMonitorTarget,
) -> Arc<crate::SignalMonitorSnapshot> {
    monitor.set_target(Some(target)).unwrap();
    renderer.signal_monitor_transport(7, true);
    let mut output = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    for _ in 0..24 {
        renderer.render(&mut output, &mut positions).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(snapshot) = monitor.snapshot() {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "Signal analysis did not publish");
        thread::sleep(Duration::from_millis(1));
    }
}

fn peaks(snapshot: &crate::SignalMonitorSnapshot) -> [f32; 2] {
    snapshot.phase.iter().fold([0.0_f32; 2], |peaks, frame| {
        [peaks[0].max(frame[0].abs()), peaks[1].max(frame[1].abs())]
    })
}

#[test]
fn analyzer_captures_own_post_fader_pan_bus_and_send_signals_and_master_sum() {
    let (_directory, project, [source, bus, send]) = fixture();
    for (target, expected) in [
        (SignalMonitorTarget::Channel(source), [0.4, 0.2]),
        (SignalMonitorTarget::Channel(bus), [0.1, 0.05]),
        (SignalMonitorTarget::Channel(send), [0.2, 0.2]),
        (SignalMonitorTarget::Master, [0.3, 0.25]),
    ] {
        let monitor = Arc::new(SignalMonitor::new(48_000));
        let mut renderer = Renderer::new(&project, 48_000, false).unwrap();
        renderer.attach_signal_monitor(Arc::clone(&monitor));
        let snapshot = capture(&mut renderer, &monitor, target);
        assert_eq!(snapshot.target, target);
        let actual = peaks(&snapshot);
        for channel in 0..2 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 1e-6,
                "{target:?}: {actual:?}"
            );
        }
        assert_eq!(snapshot.correlation, Some(1.0));
        assert!(snapshot.observed_frames <= 24 * BLOCK as u64);
    }
}

#[test]
fn analyzer_captures_silence_for_muted_pruned_and_missing_channels() {
    let (_directory, mut project, [source, bus, _send]) = fixture();
    project.track_mut(source).unwrap().mute = true;
    for id in [source, bus, u64::MAX] {
        let monitor = Arc::new(SignalMonitor::new(48_000));
        let mut renderer = Renderer::new(&project, 48_000, false).unwrap();
        renderer.attach_signal_monitor(Arc::clone(&monitor));
        let snapshot = capture(&mut renderer, &monitor, SignalMonitorTarget::Channel(id));
        assert_eq!(peaks(&snapshot), [0.0; 2]);
        assert!(snapshot.peak_dbfs.is_none());
        assert!(snapshot.correlation.is_none());
    }
}

#[test]
fn telemetry_does_not_change_rendered_audio_and_priming_does_not_publish() {
    let (_directory, project, [source, _, _]) = fixture();
    let monitor = Arc::new(SignalMonitor::new(48_000));
    monitor
        .set_target(Some(SignalMonitorTarget::Channel(source)))
        .unwrap();
    let mut renderer = Renderer::new(&project, 48_000, false).unwrap();
    renderer.attach_signal_monitor(Arc::clone(&monitor));
    renderer.signal_monitor_transport(1, true);
    let mut output = [[0.0; 2]; BLOCK];
    for _ in 0..24 {
        renderer.segment(&mut output, false).unwrap();
    }
    assert!(monitor.snapshot().is_none());
    renderer.seek(0.0).unwrap();
    let mut reference = Renderer::new(&project, 48_000, false).unwrap();
    let mut positions = [0.0; BLOCK];
    let mut expected = output;
    let mut expected_positions = positions;
    for _ in 0..24 {
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| {
                renderer.render(&mut output, &mut positions).unwrap();
            }),
            0
        );
        reference
            .render(&mut expected, &mut expected_positions)
            .unwrap();
        assert_eq!(output, expected);
        assert_eq!(positions, expected_positions);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while monitor.snapshot().is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(monitor.snapshot().unwrap().observed_frames <= 24 * BLOCK as u64);
    monitor.set_target(None).unwrap();
    assert!(monitor.snapshot().is_none());
}
