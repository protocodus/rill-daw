//! Real installed plug-ins only: RILL_TEST_VST_INSTRUMENT / RILL_TEST_VST_EFFECT.
//! Run with --ignored --nocapture. A desktop session (or Xvfb on Linux) is required.
use daw_plugins::{EditorEvent, EditorSession, PluginCatalog, PluginInfo, ThumbnailStatus};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn next(session: &mut EditorSession, wanted: impl Fn(&EditorEvent) -> bool) -> EditorEvent {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        assert!(Instant::now() < deadline, "Native editor event deadline");
        if let Some(event) = session.try_recv() {
            if let EditorEvent::Error { message } = &event {
                panic!("{message}");
            }
            if wanted(&event) {
                return event;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn exercise(variable: &str, parameter_name: &str) {
    let path = PathBuf::from(std::env::var(variable).expect(variable));
    let info = PluginInfo {
        name: "Native test".into(),
        path,
        vendor: String::new(),
        category: String::new(),
        class_id: None,
    };
    let directory = tempfile::tempdir().unwrap();
    let catalog = PluginCatalog::new(directory.path());
    let entry = catalog
        .capture(&info, daw_plugins::capture_editor_info)
        .unwrap();
    assert_eq!(entry.status, ThumbnailStatus::Ready);
    let mut reader = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(entry.thumbnail.unwrap()).unwrap(),
    ))
    .read_info()
    .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert!(frame.width >= 100 && frame.height >= 100);
    let mut histogram = [false; 256];
    for value in &pixels[..frame.buffer_size()] {
        histogram[*value as usize] = true;
    }
    assert!(
        histogram.iter().filter(|&&present| present).count() > 32,
        "Editor capture must contain real rendered detail"
    );

    let mut session = EditorSession::open_info(&info, &[], &[]).unwrap();
    assert_ne!(session.process_id(), std::process::id());
    let EditorEvent::Ready { parameters } =
        next(&mut session, |e| matches!(e, EditorEvent::Ready { .. }))
    else {
        unreachable!()
    };
    let parameter = parameters
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(parameter_name))
        .unwrap_or_else(|| panic!("Missing {parameter_name}"));
    let id = parameter.id;
    let value = 0.37;
    let earlier = session.save_state().unwrap();
    session.set_parameter(id, value).unwrap();
    let latest = session.save_state().unwrap();
    assert_ne!(earlier, latest);
    let old = next(
        &mut session,
        |e| matches!(e,EditorEvent::StateSaved {request_id:Some(request), ..} if *request == earlier),
    );
    next(
        &mut session,
        |e| matches!(e,EditorEvent::ParameterApplied {id:changed,value:actual} if *changed == id && (*actual - value).abs() < 1e-5),
    );
    let new = next(
        &mut session,
        |e| matches!(e,EditorEvent::StateSaved {request_id:Some(request), ..} if *request == latest),
    );
    let (EditorEvent::StateSaved { state: old, .. }, EditorEvent::StateSaved { state: new, .. }) =
        (old, new)
    else {
        unreachable!()
    };
    assert!(
        old != new,
        "Snapshot after the parameter edit must contain the newer state"
    );
    let EditorEvent::Ready { parameters } =
        next(&mut session, |e| matches!(e, EditorEvent::Ready { .. }))
    else {
        unreachable!()
    };
    assert!(
        (parameters.iter().find(|p| p.id == id).unwrap().value - value).abs() < 1e-5,
        "An explicit snapshot must refresh values before suppressing the automatic snapshot"
    );
    // Ordinary edits must persist without a separate Save state action. The
    // helper also refreshes metadata so preset changes cannot leave stale values.
    let automatic_value = 0.61;
    session.set_parameter(id, automatic_value).unwrap();
    next(
        &mut session,
        |e| matches!(e,EditorEvent::ParameterApplied {id:changed,value:actual} if *changed == id && (*actual - automatic_value).abs() < 1e-5),
    );
    let EditorEvent::StateSaved {
        state: automatic, ..
    } = next(&mut session, |e| {
        matches!(
            e,
            EditorEvent::StateSaved {
                request_id: None,
                ..
            }
        )
    })
    else {
        unreachable!()
    };
    assert_ne!(
        automatic, new,
        "Automatic snapshot must contain the new edit"
    );
    let EditorEvent::Ready { parameters } =
        next(&mut session, |e| matches!(e, EditorEvent::Ready { .. }))
    else {
        unreachable!()
    };
    assert!((parameters.iter().find(|p| p.id == id).unwrap().value - automatic_value).abs() < 1e-5);
    let close_id = session.close().unwrap();
    assert_eq!(session.close().unwrap(), close_id);
    let EditorEvent::StateSaved { request_id, state } = next(
        &mut session,
        |e| matches!(e, EditorEvent::StateSaved { request_id: Some(request), .. } if *request == close_id),
    ) else {
        unreachable!()
    };
    assert_eq!(request_id, Some(close_id));
    assert!(!state.is_empty());
    let closed = next(&mut session, |e| matches!(e, EditorEvent::Closed { .. }));
    assert!(matches!(closed, EditorEvent::Closed { request_id: Some(id) } if id == close_id));
    drop(session);

    let mut restored = EditorSession::open_info(&info, &automatic, &[]).unwrap();
    let EditorEvent::Ready { parameters } =
        next(&mut restored, |e| matches!(e, EditorEvent::Ready { .. }))
    else {
        unreachable!()
    };
    let actual = parameters.iter().find(|p| p.id == id).unwrap().value;
    assert!(
        (actual - automatic_value).abs() < 1e-5,
        "State restored {actual}, expected {automatic_value}"
    );
    restored.close().unwrap();
    next(&mut restored, |e| matches!(e, EditorEvent::Closed { .. }));
    eprintln!(
        "{parameter_name}: native PNG {}x{}, parameter event and {}-byte state round trip",
        frame.width,
        frame.height,
        state.len()
    );
}

#[test]
#[ignore = "Requires installed Surge XT and a desktop session"]
fn surge_native_editor_capture_parameter_and_state_round_trip() {
    exercise("RILL_TEST_VST_INSTRUMENT", "Global Volume");
}

#[test]
#[ignore = "Requires installed CHOWTapeModel and a desktop session"]
fn chow_native_editor_capture_parameter_and_state_round_trip() {
    exercise("RILL_TEST_VST_EFFECT", "Output Gain");
}
