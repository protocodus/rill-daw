//! The CLAP editor in the isolated editor helper, on a private Xvfb display:
//! cargo test -p daw-plugins --test clap_editor -- --ignored
#![cfg(target_os = "linux")]

#[path = "common/clap_probe_fixture.rs"]
mod clap_probe_fixture;

use clap_probe_fixture::{ClapProbeFixture, SYNTH_ID};
use daw_plugins::{EditorEvent, EditorSession, PluginFormat, PluginInfo};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Xvfb(Child);
impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn private_display() -> Xvfb {
    let mut child = Command::new("Xvfb")
        .args(["-displayfd", "1", "-screen", "0", "1024x768x24"])
        .args(["-nolisten", "tcp", "+extension", "Composite"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("Install Xvfb to run the CLAP editor test");
    let mut display = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut display)
        .unwrap();
    // SAFETY: this binary's only test sets it before starting any helper.
    unsafe { std::env::set_var("DISPLAY", format!(":{}", display.trim())) };
    Xvfb(child)
}

fn next(session: &mut EditorSession, wanted: impl Fn(&EditorEvent) -> bool) -> EditorEvent {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < deadline, "CLAP editor event deadline");
        if let Some(event) = session.try_recv() {
            if let EditorEvent::Error { message } = &event {
                panic!("{message}");
            }
            if wanted(&event) {
                return event;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn level(event: &EditorEvent) -> f64 {
    let EditorEvent::Ready { parameters } = event else {
        unreachable!()
    };
    parameters.iter().find(|p| p.id == 0).unwrap().value
}

#[test]
#[ignore = "Creates a private Xvfb display"]
fn clap_editor_gestures_pick_parameters_snapshot_state_and_capture() {
    let _display = private_display();
    let fixture = ClapProbeFixture::build();
    let info = PluginInfo {
        name: "Rill CLAP Synth".into(),
        path: fixture.path.clone(),
        class_id: Some(SYNTH_ID.into()),
        format: PluginFormat::Clap,
        ..PluginInfo::default()
    };

    let png = tempfile::tempdir().unwrap();
    let png = png.path().join("clap.png");
    daw_plugins::capture_editor_info(&info, &png).unwrap();
    let decoded = png::Decoder::new(BufReader::new(std::fs::File::open(&png).unwrap()))
        .read_info()
        .unwrap();
    assert!(decoded.info().width >= 100 && decoded.info().height >= 100);

    // Level is 0..2, so normalized 0.25 is 0.5 and the editor's gesture to 0.75 is 0.375.
    let mut session = EditorSession::open_info(&info, &[], &[(0, 0.25)]).unwrap();
    let pick = session.pick_parameter(true).unwrap().unwrap();
    let ready = next(&mut session, |e| matches!(e, EditorEvent::Ready { .. }));
    assert!((level(&ready) - 0.25).abs() < 1e-9);
    let changed = next(&mut session, |e| {
        matches!(e, EditorEvent::ParameterChanged { id: 0, .. })
    });
    assert!(
        matches!(changed, EditorEvent::ParameterChanged { value, .. } if (value - 0.375).abs() < 1e-9)
    );
    // A GUI gesture is persisted, without an explicit save, before it is picked.
    next(&mut session, |e| {
        matches!(
            e,
            EditorEvent::StateSaved {
                request_id: None,
                ..
            }
        )
    });
    let picked = next(&mut session, |e| {
        matches!(e, EditorEvent::ParameterPicked { .. })
    });
    assert!(
        matches!(picked, EditorEvent::ParameterPicked { request_id, id: 0, value } if request_id == pick && (value - 0.375).abs() < 1e-9),
        "{picked:?}"
    );
    session.set_parameter(1, 1.0).unwrap();
    next(&mut session, |e| {
        matches!(e, EditorEvent::ParameterApplied { id: 1, .. })
    });
    let close = session.close().unwrap();
    let EditorEvent::StateSaved { state, .. } = next(
        &mut session,
        |e| matches!(e, EditorEvent::StateSaved { request_id: Some(id), .. } if *id == close),
    ) else {
        unreachable!()
    };
    next(&mut session, |e| matches!(e, EditorEvent::Closed { .. }));
    drop(session);

    let mut restored = EditorSession::open_info(&info, &state, &[]).unwrap();
    let EditorEvent::Ready { parameters } =
        next(&mut restored, |e| matches!(e, EditorEvent::Ready { .. }))
    else {
        unreachable!()
    };
    let value = |id| parameters.iter().find(|p| p.id == id).unwrap().value;
    assert!((value(0) - 0.375).abs() < 1e-9 && (value(1) - 1.0).abs() < 1e-9);
    restored.close().unwrap();
    next(&mut restored, |e| matches!(e, EditorEvent::Closed { .. }));
}
