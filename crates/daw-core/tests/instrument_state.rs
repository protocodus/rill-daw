use daw_core::{
    History, Instrument, MAX_INSTRUMENT_STATE_BYTES, PluginParameterValue, Project, TrackKind,
};

fn fixture() -> (Project, u64) {
    let mut project = Project::new();
    let track = project.add_midi_track("Instrument");
    project
        .set_instrument(
            track,
            Instrument::Vst3 {
                class_id: None,
                path: std::env::temp_dir()
                    .join("rill-fixture.vst3")
                    .to_string_lossy()
                    .into_owned(),
            },
        )
        .unwrap();
    let instrument = project.track_mut(track).unwrap();
    instrument.instrument_state = vec![0, 1, 255, 32];
    instrument.instrument_parameters = vec![
        PluginParameterValue { id: 7, value: 0.25 },
        PluginParameterValue { id: 11, value: 1.0 },
    ];
    (project, track)
}

#[test]
fn instrument_state_persists_and_identical_assignment_preserves_it() {
    let (mut project, track) = fixture();
    let before = project.clone();
    project
        .set_instrument(track, project.track(track).unwrap().instrument.clone())
        .unwrap();
    assert_eq!(project, before);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("session.daw.json");
    project.save(&path).unwrap();
    assert_eq!(Project::load(path).unwrap(), project);
    let mut history = History::new();
    history.commit(&project);
    project
        .set_instrument(
            track,
            Instrument::Vst3 {
                class_id: None,
                path: "/different.vst3".into(),
            },
        )
        .unwrap();
    assert!(project.track(track).unwrap().instrument_state.is_empty());
    assert!(
        project
            .track(track)
            .unwrap()
            .instrument_parameters
            .is_empty()
    );
    assert!(history.undo(&mut project));
    assert_eq!(project, before);
    project.set_instrument(track, Instrument::None).unwrap();
    assert!(project.track(track).unwrap().instrument_state.is_empty());
    assert!(
        project
            .track(track)
            .unwrap()
            .instrument_parameters
            .is_empty()
    );
}

#[test]
fn instrument_state_rejects_oversize_duplicate_nonfinite_and_wrong_track_data() {
    let (project, track) = fixture();
    for value in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        let mut invalid = project.clone();
        invalid.track_mut(track).unwrap().instrument_parameters[0].value = value;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = project.clone();
    invalid.track_mut(track).unwrap().instrument_parameters[1].id = 7;
    assert!(invalid.validate().is_err());
    let mut invalid = project.clone();
    invalid.track_mut(track).unwrap().instrument_state = vec![0; MAX_INSTRUMENT_STATE_BYTES + 1];
    assert!(invalid.validate().is_err());
    invalid.track_mut(track).unwrap().instrument_state.pop();
    invalid.validate().unwrap();
    for kind in [TrackKind::Audio, TrackKind::Bus, TrackKind::Folder] {
        let mut invalid = project.clone();
        invalid.track_mut(track).unwrap().kind = kind;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = project;
    invalid.track_mut(track).unwrap().instrument = Instrument::None;
    assert!(invalid.validate().is_err());
}

#[test]
fn processor_class_is_persistent_and_changing_class_clears_obsolete_state() {
    let (mut project, track) = fixture();
    let path = match &project.track(track).unwrap().instrument {
        Instrument::Vst3 { path, .. } => path.clone(),
        Instrument::None => unreachable!(),
    };
    project
        .set_instrument(
            track,
            Instrument::Vst3 {
                path: path.clone(),
                class_id: Some("0123456789ABCDEF0123456789ABCDEF".into()),
            },
        )
        .unwrap();
    assert!(project.track(track).unwrap().instrument_state.is_empty());
    let channel = project.track_mut(track).unwrap();
    channel.instrument_state = vec![1, 2, 3];
    channel.instrument_parameters = vec![PluginParameterValue { id: 12, value: 0.7 }];
    let lane = project
        .add_automation_track_with_value(
            "Parameter",
            daw_core::AutomationTarget {
                track,
                parameter: daw_core::AutomationParameter::VstParameter {
                    effect: None,
                    id: 12,
                },
            },
            0.7,
        )
        .unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let saved = temporary.path().join("multi-class.daw.json");
    project.save(&saved).unwrap();
    assert_eq!(Project::load(saved).unwrap(), project);
    project
        .set_instrument(
            track,
            Instrument::Vst3 {
                path,
                class_id: Some("1123456789ABCDEF0123456789ABCDEF".into()),
            },
        )
        .unwrap();
    assert!(project.track(track).unwrap().instrument_state.is_empty());
    assert!(
        project
            .track(track)
            .unwrap()
            .instrument_parameters
            .is_empty()
    );
    assert!(project.track(lane).is_none());
    let legacy: Instrument =
        serde_json::from_value(serde_json::json!({"Vst3":{"path":"old.vst3"}})).unwrap();
    assert_eq!(
        legacy,
        Instrument::Vst3 {
            path: "old.vst3".into(),
            class_id: None
        }
    );
    for class_id in ["", "not-a-class", "Z123456789ABCDEF0123456789ABCDEF"] {
        let before = project.clone();
        assert!(
            project
                .set_instrument(
                    track,
                    Instrument::Vst3 {
                        path: "fixture.vst3".into(),
                        class_id: Some(class_id.into())
                    }
                )
                .is_err()
        );
        assert_eq!(
            project, before,
            "invalid class selection must not clear saved data"
        );
    }
}

#[test]
fn missing_state_fields_default_empty_and_removed_builtins_fail_explicitly() {
    let mut project = Project::new();
    let track = project.add_midi_track("Unassigned");
    assert_eq!(project.track(track).unwrap().instrument, Instrument::None);
    let mut value = serde_json::to_value(&project).unwrap();
    value["tracks"][0]
        .as_object_mut()
        .unwrap()
        .remove("instrument_state");
    value["tracks"][0]
        .as_object_mut()
        .unwrap()
        .remove("instrument_parameters");
    assert_eq!(
        serde_json::from_value::<Project>(value.clone()).unwrap(),
        project
    );
    for builtin in ["Sine", "Triangle", "Saw"] {
        value["tracks"][0]["instrument"] = builtin.into();
        let error = serde_json::from_value::<Project>(value.clone()).unwrap_err();
        assert!(error.to_string().contains("unknown variant"));
    }
    assert!(
        serde_json::from_value::<daw_core::Effect>(serde_json::json!({"Gain":{"gain":1.0}}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<daw_core::Effect>(
            serde_json::json!({"Delay":{"time_ms":250.0,"feedback":0.2,"mix":0.5}})
        )
        .is_err()
    );
}
