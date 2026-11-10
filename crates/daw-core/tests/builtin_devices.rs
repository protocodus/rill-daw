use daw_core::{
    AutomationParameter, AutomationTarget, Effect, EffectSlot, Instrument, PluginParameterValue,
    Project,
};

#[test]
fn builtin_devices_round_trip_and_take_automation() {
    let mut project = Project::new();
    let track = project.add_midi_track("Synth");
    project.track_mut(track).unwrap().instrument = Instrument::Builtin {
        device: "r1-mini".into(),
    };
    let mut slot = EffectSlot::new(
        "R1 Filter",
        Effect::Builtin {
            device: "r1-filter".into(),
        },
    );
    slot.parameters = vec![PluginParameterValue { id: 4, value: 0.3 }];
    let effect = slot.id;
    project.track_mut(track).unwrap().inserts.push(slot);
    project.validate().unwrap();
    project
        .add_automation_track_with_value(
            "Cutoff",
            AutomationTarget {
                track,
                parameter: AutomationParameter::VstParameter {
                    effect: Some(effect),
                    id: 4,
                },
            },
            0.3,
        )
        .unwrap();

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("song.rill");
    project.save(&path).unwrap();
    let restored = Project::load(&path).unwrap();
    let restored_track = restored.track(track).unwrap();
    assert_eq!(restored_track.instrument.builtin_device(), Some("r1-mini"));
    assert_eq!(
        restored_track.inserts[0].effect.builtin_device(),
        Some("r1-filter")
    );
    assert_eq!(restored_track.inserts[0].parameters[0].value, 0.3);
}

#[test]
fn builtin_device_ids_are_bounded_and_carry_no_opaque_state() {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    project.track_mut(track).unwrap().inserts.push(EffectSlot::new(
        "Bad",
        Effect::Builtin {
            device: "R1 Filter".into(),
        },
    ));
    assert!(project.validate().is_err());

    let mut project = Project::new();
    let track = project.add_midi_track("Synth");
    let channel = project.track_mut(track).unwrap();
    channel.instrument = Instrument::Builtin {
        device: "r1-mini".into(),
    };
    channel.instrument_state = vec![1];
    assert!(project.validate().is_err());
}
