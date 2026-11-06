use super::*;
use std::time::{Duration, Instant};

fn capture(monitor: &SignalMonitor) -> Arc<SignalMonitorSnapshot> {
    let (target, epoch) = monitor.target().unwrap();
    for _ in 0..24 {
        monitor.capture(target, epoch, 0, &[[0.2, -0.2]; crate::render::BLOCK]);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(snapshot) = monitor.snapshot() {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "Signal monitor did not publish");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn analysis_starts_on_demand_and_target_reset_never_exposes_previous_channel() {
    let monitor = SignalMonitor::new(48_000);
    assert!(monitor.sender.get().is_none());
    monitor.set_target(None).unwrap();
    assert!(monitor.sender.get().is_none());
    monitor
        .set_target(Some(SignalMonitorTarget::Master))
        .unwrap();
    let first = capture(&monitor);
    assert_eq!(first.target, SignalMonitorTarget::Master);
    monitor
        .set_target(Some(SignalMonitorTarget::Channel(17)))
        .unwrap();
    assert!(monitor.snapshot().is_none());
    let second = capture(&monitor);
    assert_eq!(second.target, SignalMonitorTarget::Channel(17));
    assert!(second.generation > first.generation);
    assert!(second.observed_frames <= 24 * crate::render::BLOCK as u64);
    monitor.reset();
    assert!(monitor.snapshot().is_none());
    monitor.set_target(None).unwrap();
    assert!(monitor.snapshot().is_none());
}

#[test]
fn ui_snapshot_never_waits_for_worker_publication_and_allocates_nothing() {
    let monitor = SignalMonitor::new(48_000);
    monitor
        .set_target(Some(SignalMonitorTarget::Master))
        .unwrap();
    let _snapshot = capture(&monitor);
    let guard = monitor.latest.lock().unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| {
            assert!(monitor.snapshot().is_none());
        }),
        0
    );
    drop(guard);
    assert!(monitor.snapshot().is_some());
}
