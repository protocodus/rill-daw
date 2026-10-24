use daw_core::{
    AutomationParameter, AutomationTarget, Effect, EffectSlot, History, MAX_PLUGIN_STATE_BYTES,
    PluginParameterValue, Project,
};

fn fixture() -> (Project, u64, u64) {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    let mut slot = EffectSlot::new(
        "Effect",
        Effect::Vst3 {
            path: "/fixture.vst3".into(),
            class_id: None,
        },
    );
    slot.state = vec![0, 1, 255, 32];
    slot.parameters = vec![PluginParameterValue { id: 7, value: 0.25 }];
    let id = slot.id;
    project.track_mut(track).unwrap().inserts.push(slot);
    (project, track, id)
}

#[test]
fn effect_state_roundtrips_and_legacy_slots_default_empty() {
    let (mut project, _, _) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let Effect::Vst3 { path, .. } = &mut project.tracks[0].inserts[0].effect;
    *path = temp
        .path()
        .join("fixture.vst3")
        .to_string_lossy()
        .into_owned();
    let path = temp.path().join("effects.daw.json");
    project.save(&path).unwrap();
    assert_eq!(Project::load(path).unwrap(), project);
    let mut value = serde_json::to_value(&project).unwrap();
    let slot = value["tracks"][0]["inserts"][0].as_object_mut().unwrap();
    slot.remove("state");
    slot.remove("parameters");
    let legacy: Project = serde_json::from_value(value).unwrap();
    legacy.validate().unwrap();
    assert!(legacy.tracks[0].inserts[0].state.is_empty());
    assert!(legacy.tracks[0].inserts[0].parameters.is_empty());
    assert_eq!(
        legacy.tracks[0].inserts[0].id,
        project.tracks[0].inserts[0].id
    );
}

#[test]
fn effect_state_validates_normalized_unique_parameters_even_when_bypassed() {
    let (project, track, _) = fixture();
    for value in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        let mut invalid = project.clone();
        let slot = &mut invalid.track_mut(track).unwrap().inserts[0];
        slot.enabled = false;
        slot.parameters[0].value = value;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = project.clone();
    invalid.track_mut(track).unwrap().inserts[0]
        .parameters
        .push(PluginParameterValue { id: 7, value: 0.8 });
    assert!(invalid.validate().is_err());
    let mut oversized = project;
    oversized.track_mut(track).unwrap().inserts[0].state = vec![0; MAX_PLUGIN_STATE_BYTES + 1];
    assert!(oversized.validate().is_err());
    oversized.track_mut(track).unwrap().inserts[0].state.pop();
    oversized.validate().unwrap();
}

#[test]
fn effect_replacement_preserves_slot_identity_and_invalidates_only_its_state_and_lanes() {
    let (mut project, track, slot_id) = fixture();
    let lane = project
        .add_automation_track_with_value(
            "Effect parameter",
            AutomationTarget {
                track,
                parameter: AutomationParameter::VstParameter {
                    effect: Some(slot_id),
                    id: 7,
                },
            },
            0.25,
        )
        .unwrap();
    let gain_lane = project
        .add_automation_track(
            "Track gain",
            AutomationTarget {
                track,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    let before = project.clone();
    project
        .set_effect(
            track,
            slot_id,
            project.track(track).unwrap().inserts[0].effect.clone(),
        )
        .unwrap();
    assert_eq!(project, before);
    assert!(
        project
            .set_effect(
                track,
                slot_id,
                Effect::Vst3 {
                    path: String::new(),
                    class_id: None,
                }
            )
            .is_err()
    );
    assert_eq!(
        project, before,
        "Invalid replacement must retain the preset"
    );
    let mut history = History::new();
    history.commit(&project);
    project
        .set_effect(
            track,
            slot_id,
            Effect::Vst3 {
                path: "/fixture.vst3".into(),
                class_id: Some("0123456789ABCDEF0123456789ABCDEF".into()),
            },
        )
        .unwrap();
    let slot = &project.track(track).unwrap().inserts[0];
    assert_eq!(slot.id, slot_id);
    assert!(slot.state.is_empty());
    assert!(slot.parameters.is_empty());
    assert!(project.track(lane).is_none());
    assert!(project.track(gain_lane).is_some());
    assert!(history.undo(&mut project));
    assert_eq!(project, before);
}
