use daw_core::{
    AutomationParameter, AutomationTarget, Clip, DeviceSample, EditBatch, Effect, EffectSlot,
    Instrument, PluginParameterValue, Project, SongEdit,
};
use std::fs;

#[test]
fn builtin_devices_round_trip_and_take_automation() {
    let mut project = Project::new();
    let track = project.add_midi_track("Synth");
    project.track_mut(track).unwrap().instrument = Instrument::builtin("r1-mini");
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
    project
        .track_mut(track)
        .unwrap()
        .inserts
        .push(EffectSlot::new(
            "Bad",
            Effect::Builtin {
                device: "R1 Filter".into(),
            },
        ));
    assert!(project.validate().is_err());

    let mut project = Project::new();
    let track = project.add_midi_track("Synth");
    let channel = project.track_mut(track).unwrap();
    channel.instrument = Instrument::builtin("r1-mini");
    channel.instrument_state = vec![1];
    assert!(project.validate().is_err());
}

#[test]
fn new_instrument_tracks_start_with_the_default_synth() {
    let mut project = Project::new();
    let track = project.add_instrument_track("Synth");
    assert_eq!(
        project.track(track).unwrap().instrument,
        Instrument::builtin("r1-mini")
    );
    project.validate().unwrap();
}

fn sample_at(slot: u32, path: &str) -> DeviceSample {
    DeviceSample {
        slot,
        path: path.into(),
        name: "Sample".into(),
    }
}

fn sine_wav(path: &std::path::Path) {
    daw_core::AudioData {
        sample_rate: 44_100,
        channels: 1,
        samples: (0..4410).map(|n| (n as f32 * 0.06).sin() * 0.5).collect(),
    }
    .write_wav(path)
    .unwrap();
}

#[test]
fn projects_without_device_samples_still_load() {
    let old = r#"{"Builtin":{"device":"r1-mini"}}"#;
    let instrument: Instrument = serde_json::from_str(old).unwrap();
    assert_eq!(instrument, Instrument::builtin("r1-mini"));
    // No samples, no field: files stay as they were.
    assert_eq!(serde_json::to_string(&instrument).unwrap(), old);
}

#[test]
fn sample_slot_names_are_independent_bounded_and_undoable() {
    let mut project = Project::new();
    let track = project.add_instrument_track("Kit");
    project
        .set_instrument(track, Instrument::builtin("r1-drums"))
        .unwrap();
    let mut history = daw_core::History::new();
    history.commit_named(&project, "Name pad");
    EditBatch {
        label: "Name pad".into(),
        edits: vec![SongEdit::RenameDeviceSampleSlot {
            track: track.into(),
            slot: 3,
            name: "  Soft clap  ".into(),
        }],
    }
    .apply(&mut project)
    .unwrap();
    assert_eq!(
        project.track(track).unwrap().instrument.sample_name(3),
        Some("Soft clap")
    );
    assert!(history.undo(&mut project));
    assert_eq!(
        project.track(track).unwrap().instrument.sample_name(3),
        None
    );
    assert!(history.redo(&mut project));

    project
        .load_device_sample(track, sample_at(3, "/old.wav"))
        .unwrap();
    project
        .load_device_sample(track, sample_at(3, "/new.wav"))
        .unwrap();
    project
        .load_device_sample(track, sample_at(19, "/layer.wav"))
        .unwrap();
    project.clear_device_sample(track, 3).unwrap();
    assert_eq!(
        project.track(track).unwrap().instrument.sample_name(3),
        Some("Soft clap")
    );
    let before = project.clone();
    for name in ["x".repeat(65), "line\nbreak".into()] {
        assert!(project.rename_device_sample_slot(track, 3, &name).is_err());
        assert_eq!(project, before);
    }
    let encoded = serde_json::to_string(&project).unwrap();
    let restored: Project = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored, project);
    project.rename_device_sample_slot(track, 3, "   ").unwrap();
    assert_eq!(
        project.track(track).unwrap().instrument.sample_name(3),
        None
    );
}

#[test]
fn player_trees_validate_atomically_and_names_are_audio_neutral() {
    use daw_core::{PlayerKind, PlayerNode};
    let mut project = Project::new();
    let track = project.add_instrument_track("Sampler");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    let mut root = PlayerNode {
        id: 1,
        name: "Body".into(),
        kind: PlayerKind::All {
            children: vec![PlayerNode::sample(2, 0), PlayerNode::sample(3, 127)],
        },
    };
    project
        .set_sample_player(track, 0, Some(root.clone()))
        .unwrap();
    let original = project.track(track).unwrap().instrument.clone();
    root.name = "Renamed body".into();
    root.find_mut(3).unwrap().name = "Transient".into();
    project
        .set_sample_player(track, 0, Some(root.clone()))
        .unwrap();
    project.rename_device_sample_slot(track, 0, "Main").unwrap();
    assert!(original.audio_equivalent(&project.track(track).unwrap().instrument));
    assert_ne!(original, project.track(track).unwrap().instrument);
    let before = project.clone();
    root.find_mut(3).unwrap().id = 2;
    assert!(project.set_sample_player(track, 0, Some(root)).is_err());
    assert_eq!(project, before);
    let restored: Project =
        serde_json::from_value(serde_json::to_value(&project).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, project);
    project.set_sample_player(track, 0, None).unwrap();
    assert!(
        project
            .track(track)
            .unwrap()
            .instrument
            .sample_player(0)
            .is_none()
    );
}

#[test]
fn removed_leaf_cleanup_preserves_shared_legacy_and_staged_recordings() {
    use daw_core::PlayerNode;
    let mut project = Project::new();
    let track = project.add_instrument_track("Drums");
    project
        .set_instrument(track, Instrument::builtin("r1-drums"))
        .unwrap();
    for slot in [0, 16, 17, 18] {
        project
            .load_device_sample(track, sample_at(slot, &format!("/{slot}.wav")))
            .unwrap();
    }
    for (group, slot) in [(0, 16), (1, 16), (2, 17)] {
        project
            .set_sample_player(track, group, Some(PlayerNode::sample(1, slot)))
            .unwrap();
    }
    let before = project.track(track).unwrap().instrument.clone();
    project.set_sample_player(track, 0, None).unwrap();
    project.set_sample_player(track, 2, None).unwrap();
    let after = &mut project.track_mut(track).unwrap().instrument;
    after.release_removed_sample_bindings(&before, 16);
    assert!(after.sample(0).is_some(), "implicit legacy recording");
    assert!(
        after.sample(16).is_some(),
        "another player still references it"
    );
    assert!(after.sample(17).is_none(), "last reference was removed");
    assert!(
        after.sample(18).is_some(),
        "staged recording was never referenced"
    );
    assert!(after.sample_slot_available(17, 16, 128));
    assert!(!after.sample_slot_available(18, 16, 128));
}

#[test]
fn device_samples_are_bounded_unique_and_only_on_builtin_instruments() {
    let mut project = Project::new();
    let track = project.add_instrument_track("Sampler");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    project
        .load_device_sample(track, sample_at(0, "/a.wav"))
        .unwrap();
    project
        .load_device_sample(track, sample_at(0, "/b.wav"))
        .unwrap();
    let instrument = &project.track(track).unwrap().instrument;
    assert_eq!(instrument.samples().len(), 1, "a slot holds one sample");
    assert_eq!(instrument.sample(0).unwrap().path, "/b.wav");
    assert!(
        project
            .load_device_sample(track, sample_at(128, "/c.wav"))
            .is_err()
    );
    assert!(project.load_device_sample(track, sample_at(1, "")).is_err());
    project.clear_device_sample(track, 0).unwrap();
    assert!(
        project
            .track(track)
            .unwrap()
            .instrument
            .samples()
            .is_empty()
    );

    let duplicate = |samples: Vec<DeviceSample>| {
        let mut project = Project::new();
        let track = project.add_instrument_track("Drums");
        project.track_mut(track).unwrap().instrument = Instrument::Builtin {
            device: "r1-drums".into(),
            samples,
            sample_names: Default::default(),
            sample_players: Default::default(),
        };
        project.validate()
    };
    let sample = |slot| sample_at(slot, &format!("/{slot}.wav"));
    assert!(duplicate((0..16).map(sample).collect()).is_ok());
    assert!(duplicate(vec![sample(3), sample(3)]).is_err());
    assert!(duplicate((0..17).map(|n| sample(n % 16)).collect()).is_err());

    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    assert!(
        project
            .load_device_sample(track, sample_at(0, "/a.wav"))
            .is_err()
    );
}

#[test]
fn a_clip_becomes_a_sample_by_edit_and_both_travel_in_the_bundle() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("hit.wav");
    sine_wav(&source);
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio");
    let clip = Clip::audio("Hit", 0.0, 1.0, source.to_string_lossy());
    let clip_id = clip.id;
    project.track_mut(audio).unwrap().clips.push(clip);
    let track = project.add_instrument_track("Sampler");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    EditBatch {
        label: "Load sample".into(),
        edits: vec![SongEdit::SetDeviceSample {
            track: track.into(),
            slot: 0,
            clip: Some(clip_id.into()),
        }],
    }
    .apply(&mut project)
    .unwrap();
    let instrument = &project.track(track).unwrap().instrument;
    assert_eq!(instrument.sample(0).unwrap().path, source.to_string_lossy());
    assert_eq!(instrument.samples()[0].name, "Hit");
    // A MIDI clip has no recording to load.
    let midi = Clip::midi("Notes", 0.0, 1.0, Vec::new());
    let midi_id = midi.id;
    project.track_mut(track).unwrap().clips.push(midi);
    let mut copy = project.clone();
    assert!(
        EditBatch {
            label: "Load sample".into(),
            edits: vec![SongEdit::SetDeviceSample {
                track: track.into(),
                slot: 0,
                clip: Some(midi_id.into()),
            }],
        }
        .apply(&mut copy)
        .is_err()
    );

    // The sample travels in the bundle without the clip it came from.
    project.track_mut(audio).unwrap().clips.clear();
    let saved = temp.path().join("song.rill");
    project.save(&saved).unwrap();
    fs::remove_file(&source).unwrap();
    let restored = Project::load_with_assets(&saved, temp.path().join("scratch")).unwrap();
    let path = restored
        .track(track)
        .unwrap()
        .instrument
        .sample(0)
        .unwrap()
        .path
        .clone();
    assert_ne!(path, source.to_string_lossy());
    let data = daw_core::AudioData::load_wav(&path).unwrap();
    assert_eq!(data.samples.len(), 4410);
}

#[test]
fn a_bundle_naming_an_unbundled_sample_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let track = project.add_instrument_track("Sampler");
    project.track_mut(track).unwrap().instrument = Instrument::Builtin {
        device: "r1-sampler".into(),
        samples: vec![sample_at(0, "../outside.wav")],
        sample_names: Default::default(),
        sample_players: Default::default(),
    };
    let bytes = {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file("manifest.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(
                &serde_json::to_vec(&serde_json::json!({"version":1,"project":project})).unwrap(),
            )
            .unwrap();
        writer.finish().unwrap().into_inner()
    };
    let saved = temp.path().join("bad.rill");
    fs::write(&saved, bytes).unwrap();
    let error = Project::load_with_assets(&saved, temp.path().join("scratch")).unwrap_err();
    assert!(
        format!("{error:#}").contains("bundled audio reference"),
        "{error:#}"
    );
}
