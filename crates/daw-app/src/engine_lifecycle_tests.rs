//! Device-free completion/retirement races using the real application state.
use super::*;
use std::sync::{atomic::Ordering, mpsc};

#[test]
fn workspace_track_order_is_audio_equivalent_but_owned_audio_changes_are_not() {
    let mut original = Project::new();
    let source = original.add_audio_track("Source");
    let bus = original.add_bus("Bus");
    let folder = original.add_folder("Folder");
    original.set_output(source, Some(bus)).unwrap();
    let automation = original
        .add_automation_track(
            "Volume",
            daw_core::AutomationTarget {
                track: source,
                parameter: daw_core::AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let mut reordered = original.clone();
    reordered.tracks.reverse();
    assert!(audio_equivalent(&original, &reordered));
    reordered.track_mut(source).unwrap().name = "Renamed source".into();
    assert!(audio_equivalent(&original, &reordered));

    let mut changed = reordered.clone();
    changed.track_mut(bus).unwrap().gain = 0.25;
    assert!(!audio_equivalent(&original, &changed));
    changed = reordered.clone();
    changed.automation_lane_mut(automation).unwrap().points[0].value = 0.25;
    assert!(!audio_equivalent(&original, &changed));
    changed = reordered;
    changed.set_parent_folder(source, Some(folder)).unwrap();
    assert!(!audio_equivalent(&original, &changed));
}

#[test]
fn transport_stop_retains_pending_initialization_until_it_has_retired() {
    let ctx = egui::Context::default();
    let mut app = DawApp::with_context(&ctx, Project::new(), None);
    let (release, pending) = mpsc::channel();
    app.loading = Some(pending);
    app.desired_playing = true;
    app.cursor = 4.0;
    app.stop();
    assert!(!app.desired_playing);
    assert_eq!(app.cursor, 0.0);
    let retired = app.retiring.take().unwrap();
    assert!(matches!(retired.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release
        .send((0, Err(anyhow::anyhow!("cancelled"))))
        .unwrap();
    retired.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn stale_paused_completion_prepares_current_graph_and_cancel_prevents_device_open() {
    let ctx = egui::Context::default();
    let mut app = DawApp::with_context(&ctx, Project::new(), None);
    let (retired, gate) = mpsc::channel();
    app.retiring = Some(gate); // Simulate an old stream still shutting down.
    let (completed, preparing) = mpsc::channel();
    app.loading = Some(preparing);
    app.revision = 4;
    app.desired_playing = false;
    completed
        .send((3, Err(anyhow::anyhow!("obsolete graph"))))
        .unwrap();
    app.poll_engine();
    assert!(
        app.loading.is_some(),
        "Paused setup still needs the latest graph"
    );
    assert!(!app.desired_playing);
    assert!(
        !app.error,
        "A stale failure is not the current preparation result"
    );
    let cancel = app.loading_cancel.as_ref().unwrap().clone();
    app.retire_engine();
    assert!(cancel.load(Ordering::Acquire));
    assert!(app.loading.is_none());
    retired.send(()).unwrap();
    app.retiring
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(app.engine.is_none());
}

#[test]
fn retiring_superseded_preparations_keeps_every_owner_in_the_chain() {
    let ctx = egui::Context::default();
    let mut app = DawApp::with_context(&ctx, Project::new(), None);
    let (release, pending) = mpsc::channel();
    app.loading = Some(pending);
    app.prepare_engine(false);
    let first = app.loading_cancel.as_ref().unwrap().clone();
    app.prepare_engine(true);
    let second = app.loading_cancel.as_ref().unwrap().clone();
    assert!(first.load(Ordering::Acquire));
    assert!(!second.load(Ordering::Acquire));
    app.retire_engine();
    assert!(second.load(Ordering::Acquire));
    let retired = app.retiring.take().unwrap();
    assert!(
        matches!(retired.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "Retirement must wait for the original pending owner"
    );
    release
        .send((0, Err(anyhow::anyhow!("cancelled original"))))
        .unwrap();
    retired.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(app.engine.is_none());
    assert!(app.loading.is_none());
}
