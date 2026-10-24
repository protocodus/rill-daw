use daw_core::{Clip, Effect, EffectSlot, Project, Send, TrackKind, new_id};

#[test]
fn a_new_bus_has_unity_gain_for_transparent_routing() {
    assert_eq!(daw_core::Track::new("Bus", TrackKind::Bus).gain, 1.0);
}

#[test]
fn deleting_a_bus_restores_main_outputs_and_removes_incoming_sends() {
    let mut project = Project::new();
    let bus = project.add_bus("Effects");
    let audio = project.add_audio_track("Audio");
    let midi = project.add_midi_track("MIDI");
    project.set_output(audio, Some(bus)).unwrap();
    project.add_send(midi, bus, 0.5, false).unwrap();
    let removed = project.delete_track(bus).unwrap();
    assert_eq!(removed.id, bus);
    assert!(project.track(bus).is_none());
    assert_eq!(project.track(audio).unwrap().output_bus, None);
    assert!(project.track(midi).unwrap().sends.is_empty());
    project.validate().unwrap();
    let before = project.clone();
    assert!(project.delete_track(bus).is_err());
    assert_eq!(project, before);
}

#[test]
fn deleting_a_folder_promotes_direct_children_and_preserves_nested_groups() {
    let mut project = Project::new();
    let grandparent = project.add_folder("Band");
    let parent = project.add_folder("Rhythm");
    let child = project.add_folder("Percussion");
    let audio = project.add_audio_track("Kick");
    let midi = project.add_midi_track("Bass");
    project
        .set_parent_folder(parent, Some(grandparent))
        .unwrap();
    project.set_parent_folder(child, Some(parent)).unwrap();
    project.set_parent_folder(audio, Some(child)).unwrap();
    project.set_parent_folder(midi, Some(parent)).unwrap();
    project.delete_track(parent).unwrap();
    assert_eq!(
        project.track(child).unwrap().parent_folder,
        Some(grandparent)
    );
    assert_eq!(
        project.track(midi).unwrap().parent_folder,
        Some(grandparent)
    );
    assert_eq!(project.track(audio).unwrap().parent_folder, Some(child));
    project.delete_track(grandparent).unwrap();
    assert_eq!(project.track(child).unwrap().parent_folder, None);
    assert_eq!(project.track(midi).unwrap().parent_folder, None);
    project.validate().unwrap();
}

#[test]
fn deleting_a_track_rolls_back_when_the_candidate_is_invalid() {
    let mut project = Project::new();
    let first = project.add_audio_track("First");
    let invalid = project.add_audio_track("Invalid");
    project.track_mut(invalid).unwrap().pan = 2.0;
    let before = project.clone();
    assert!(project.delete_track(first).is_err());
    assert_eq!(project, before);
}

#[test]
fn routing_order_is_source_before_bus_and_preserves_parallel_paths() {
    let mut project = Project::new();
    let sum = project.add_bus("Mix");
    let reverb = project.add_bus("Reverb");
    let instrument = project.add_midi_track("Piano");
    let audio = project.add_audio_track("Audio");
    let folder = project.add_folder("Band");
    project.set_parent_folder(instrument, Some(folder)).unwrap();
    project.set_output(instrument, Some(sum)).unwrap();
    project.add_send(instrument, reverb, 0.3, false).unwrap();
    project.set_output(reverb, Some(sum)).unwrap();
    project.set_output(audio, Some(sum)).unwrap();
    // Main output and a send to the same bus are distinct contributions.
    project.add_send(audio, sum, 0.1, true).unwrap();
    let order = project.routing_order().unwrap();
    let position = |id| order.iter().position(|item| *item == id).unwrap();
    assert_eq!(order.len(), 4);
    assert!(!order.contains(&folder));
    assert!(position(instrument) < position(reverb));
    assert!(position(reverb) < position(sum));
    assert!(position(audio) < position(sum));
    project.validate().unwrap();
}

#[test]
fn output_and_send_cycles_are_rejected_atomically() {
    let mut project = Project::new();
    let a = project.add_bus("A");
    let b = project.add_bus("B");
    let c = project.add_bus("C");
    project.set_output(a, Some(b)).unwrap();
    project.add_send(b, c, 0.5, false).unwrap();
    let before = project.clone();
    assert!(project.set_output(c, Some(a)).is_err());
    assert_eq!(project, before);
    assert!(project.add_send(c, a, 0.5, true).is_err());
    assert_eq!(project, before);
    assert!(project.set_output(a, Some(a)).is_err());
    assert_eq!(project, before);
    assert!(project.add_send(b, c, 0.5, true).is_err());
    assert_eq!(project, before);
}

#[test]
fn output_choices_exclude_feedback_through_main_and_send_paths() {
    let mut project = Project::new();
    let current = project.add_bus("Current output");
    let source = project.add_bus("Source");
    let upstream = project.add_bus("Upstream output");
    let ancestor = project.add_bus("Upstream send");
    let parallel = project.add_bus("Existing send");
    let independent = project.add_bus("Independent");
    let folder = project.add_folder("Group");
    project.add_audio_track("Audio");
    project.add_midi_track("MIDI");
    project.set_output(upstream, Some(source)).unwrap();
    // A silent send and a muted bus still participate in routing validation.
    project.add_send(ancestor, upstream, 0.0, true).unwrap();
    project.track_mut(ancestor).unwrap().mute = true;
    project.set_output(source, Some(current)).unwrap();
    project.add_send(source, parallel, 0.5, false).unwrap();
    project.set_output(parallel, Some(current)).unwrap();
    project.set_parent_folder(source, Some(folder)).unwrap();
    let before = project.clone();
    let eligible: Vec<_> = project
        .eligible_output_buses(source)
        .unwrap()
        .into_iter()
        .map(|track| track.id)
        .collect();
    assert_eq!(eligible, vec![current, parallel, independent]);
    assert_eq!(project, before, "Query must not edit the routing graph");
    // The picker and final mutation must agree, including an existing output or
    // parallel send as a valid choice and transitive feedback as an invalid one.
    for bus in [current, source, upstream, ancestor, parallel, independent] {
        let mut candidate = project.clone();
        assert_eq!(
            candidate.set_output(source, Some(bus)).is_ok(),
            eligible.contains(&bus)
        );
    }
    project.set_output(source, None).unwrap();
    assert_eq!(project.track(source).unwrap().output_bus, None);
}

#[test]
fn output_choices_accept_signal_tracks_and_reject_non_signal_sources() {
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio");
    let midi = project.add_midi_track("MIDI");
    let first = project.add_bus("First");
    let second = project.add_bus("Second");
    let folder = project.add_folder("Folder");
    let automation_id = project
        .add_automation_track(
            "Automation",
            daw_core::AutomationTarget {
                track: audio,
                parameter: daw_core::AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    for source in [audio, midi] {
        let eligible: Vec<_> = project
            .eligible_output_buses(source)
            .unwrap()
            .into_iter()
            .map(|track| track.id)
            .collect();
        assert_eq!(eligible, vec![first, second]);
    }
    let before = project.clone();
    for source in [folder, automation_id, u64::MAX] {
        assert!(project.eligible_output_buses(source).is_err());
        assert!(project.set_output(source, None).is_err());
        assert!(project.set_output(source, Some(first)).is_err());
    }
    assert_eq!(project, before);
    project.validate().unwrap();
}

#[test]
fn invalid_destinations_and_folder_cycles_leave_project_unchanged() {
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio");
    let top = project.add_folder("Top");
    let inner = project.add_folder("Inner");
    let bus = project.add_bus("Bus");
    project.set_parent_folder(inner, Some(top)).unwrap();
    project.set_parent_folder(audio, Some(inner)).unwrap();
    let before = project.clone();
    assert!(project.set_parent_folder(top, Some(inner)).is_err());
    assert!(project.set_parent_folder(top, Some(top)).is_err());
    assert!(project.set_parent_folder(audio, Some(bus)).is_err());
    assert!(project.set_parent_folder(audio, Some(u64::MAX)).is_err());
    assert!(project.set_output(bus, Some(audio)).is_err());
    assert!(project.set_output(audio, Some(top)).is_err());
    assert!(project.set_output(top, Some(bus)).is_err());
    assert_eq!(project, before);
    project.validate().unwrap();
}

#[test]
fn routing_and_hierarchy_survive_project_save_load() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("routing.daw");
    let mut project = Project::new();
    let folder = project.add_folder("Rhythm");
    let bus = project.add_bus("Drums sum");
    let audio = project.add_audio_track("Kick");
    project.track_mut(folder).unwrap().collapsed = true;
    project.set_parent_folder(audio, Some(folder)).unwrap();
    project.set_parent_folder(bus, Some(folder)).unwrap();
    project.set_output(audio, Some(bus)).unwrap();
    let effect = EffectSlot::new(
        "Echo",
        Effect::Vst3 {
            class_id: None,
            path: directory
                .path()
                .join("echo.vst3")
                .to_string_lossy()
                .into_owned(),
        },
    );
    let effect_id = effect.id;
    project.track_mut(bus).unwrap().inserts.push(effect);
    project.save(&path).unwrap();
    let restored = Project::load(&path).unwrap();
    assert_eq!(restored, project);
    assert!(new_id() > effect_id);
}

#[test]
fn legacy_tracks_load_without_routing_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.daw");
    let project = Project::demo();
    let mut value = serde_json::to_value(&project).unwrap();
    for track in value["tracks"].as_array_mut().unwrap() {
        let object = track.as_object_mut().unwrap();
        for key in [
            "inserts",
            "sends",
            "output_bus",
            "parent_folder",
            "collapsed",
        ] {
            object.remove(key);
        }
    }
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({ "version": 1, "project": value })).unwrap(),
    )
    .unwrap();
    assert_eq!(Project::load(path).unwrap(), project);
}

#[test]
fn malformed_signal_graph_and_effect_parameters_are_rejected() {
    let mut project = Project::new();
    let folder = project.add_folder("Folder");
    let bus = project.add_bus("Bus");
    let audio = project.add_audio_track("Audio");
    project
        .track_mut(folder)
        .unwrap()
        .clips
        .push(Clip::audio("Invalid", 0.0, 4.0, "/unused.wav"));
    assert!(project.validate().is_err());
    project.track_mut(folder).unwrap().clips.clear();
    project.track_mut(audio).unwrap().sends.push(Send {
        target_bus: bus,
        gain: f32::NAN,
        pre_fader: false,
    });
    assert!(project.validate().is_err());
    project.track_mut(audio).unwrap().sends.clear();
    project
        .track_mut(bus)
        .unwrap()
        .inserts
        .push(EffectSlot::new(
            "Missing plugin",
            Effect::Vst3 {
                class_id: None,
                path: String::new(),
            },
        ));
    assert!(project.validate().is_err());
    project.track_mut(bus).unwrap().inserts[0].effect = Effect::Vst3 {
        class_id: None,
        path: "/fixture.vst3".into(),
    };
    project.validate().unwrap();
    project.track_mut(bus).unwrap().inserts[0].id = audio;
    assert!(project.validate().is_err());
    assert_eq!(project.track(bus).unwrap().kind, TrackKind::Bus);
}
