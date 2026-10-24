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
        Effect::Delay {
            time_ms: 240.0,
            feedback: 0.4,
            mix: 0.3,
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
            "Unstable delay",
            Effect::Delay {
                time_ms: 100.0,
                feedback: 1.0,
                mix: 0.5,
            },
        ));
    assert!(project.validate().is_err());
    project.track_mut(bus).unwrap().inserts[0].effect = Effect::Gain { gain: 1.0 };
    project.validate().unwrap();
    project.track_mut(bus).unwrap().inserts[0].id = audio;
    assert!(project.validate().is_err());
    assert_eq!(project.track(bus).unwrap().kind, TrackKind::Bus);
}
