use daw_core::{
    AudioCrossfade, AudioData, AudioEnvelope, Clip, ClipContent, CrossfadeCurve, FadeCurve,
    History, MAX_BEAT, Note, Project,
};

fn midi_project() -> (Project, u64, u64) {
    let mut project = Project::new();
    let track = project.add_midi_track("MIDI");
    let clip = Clip::midi(
        "Melody",
        0.0,
        4.0,
        vec![Note::new(60, 0.0, 1.0, 90), Note::new(64, 2.0, 1.0, 100)],
    );
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    (project, track, id)
}

fn audio_project() -> (Project, u64, u64) {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    let clip = Clip::audio("Sample", 0.0, 4.0, "source.wav");
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    (project, track, id)
}

#[test]
fn midi_clones_share_names_notes_and_stable_event_ids_from_any_instance() {
    let (mut project, track, original) = midi_project();
    let clone = project.clone_clip(track, original).unwrap();
    let third = project.clone_clip(track, clone).unwrap();
    let group = project.clip(track, original).unwrap().link_group.unwrap();
    assert!(![original, clone, third, track].contains(&group));
    assert_eq!(project.clip(track, clone).unwrap().start, 4.0);
    assert_eq!(project.clip(track, third).unwrap().start, 8.0);
    project.clip_mut(track, clone).unwrap().name = "Variation 🎹".into();
    project.midi_notes_mut(track, clone).unwrap()[0].pitch = 72;
    project.synchronize_linked_clip(track, clone).unwrap();
    let notes = project.midi_notes(track, clone).unwrap().to_vec();
    for id in [original, clone, third] {
        let clip = project.clip(track, id).unwrap();
        assert_eq!(clip.name, "Variation 🎹");
        assert_eq!(clip.link_group, Some(group));
        assert_eq!(project.midi_notes(track, id).unwrap(), notes);
    }
    // Core source editing helpers also publish without a UI synchronization step.
    project.split_note(track, third, notes[0].id, 0.5).unwrap();
    assert_eq!(project.midi_notes(track, original).unwrap().len(), 3);
    project.validate().unwrap();
}

#[test]
fn independent_duplicate_and_detach_preserve_values_without_sharing_ids_or_edits() {
    let (mut project, track, original) = midi_project();
    let clone = project.clone_clip(track, original).unwrap();
    let duplicate = project.duplicate_clip(track, clone).unwrap();
    assert_eq!(project.clip(track, duplicate).unwrap().link_group, None);
    let shared_id = project.midi_notes(track, original).unwrap()[0].id;
    assert_ne!(
        project.midi_notes(track, duplicate).unwrap()[0].id,
        shared_id
    );
    project.make_clip_independent(track, clone).unwrap();
    assert_ne!(project.midi_notes(track, clone).unwrap()[0].id, shared_id);
    project.clip_mut(track, clone).unwrap().name = "Own name".into();
    project.midi_notes_mut(track, clone).unwrap()[0].pitch = 40;
    project.synchronize_linked_clip(track, clone).unwrap();
    assert_eq!(project.clip(track, original).unwrap().name, "Melody");
    assert_eq!(project.midi_notes(track, original).unwrap()[0].pitch, 60);
    assert_eq!(project.midi_notes(track, duplicate).unwrap()[0].pitch, 60);
    project.validate().unwrap();
}

#[test]
fn movement_trimming_and_splitting_are_local_and_source_edits_still_propagate() {
    let (mut project, track, original) = midi_project();
    let clone = project.clone_clip(track, original).unwrap();
    project.move_clip(track, clone, 12.0).unwrap();
    project.trim_clip(track, clone, 13.0, 2.0).unwrap();
    let right = project.split_clip(track, clone, 14.0).unwrap();
    assert_eq!(
        (
            project.clip(track, original).unwrap().start,
            project.clip(track, original).unwrap().length
        ),
        (0.0, 4.0)
    );
    assert_eq!(
        (
            project.clip(track, right).unwrap().offset,
            project.clip(track, right).unwrap().length
        ),
        (2.0, 1.0)
    );
    project.midi_notes_mut(track, right).unwrap()[1].velocity = 33;
    project.synchronize_linked_clip(track, right).unwrap();
    for id in [original, clone, right] {
        assert_eq!(project.midi_notes(track, id).unwrap()[1].velocity, 33);
    }
    project.validate().unwrap();
}

#[test]
fn groups_survive_parent_deletion_and_movement_between_tracks() {
    let (mut project, first, original) = midi_project();
    let clone = project.clone_clip(first, original).unwrap();
    let third = project.clone_clip(first, clone).unwrap();
    let second = project.add_midi_track("Other instrument");
    let moved = project.delete_clip(first, clone).unwrap();
    project.track_mut(second).unwrap().clips.push(moved);
    project.delete_clip(first, original).unwrap();
    project.clip_mut(second, clone).unwrap().name = "Survivor".into();
    project.synchronize_linked_clip(second, clone).unwrap();
    assert_eq!(project.clip(first, third).unwrap().name, "Survivor");
    project.validate().unwrap();
}

#[test]
fn audio_gain_fades_reverse_are_shared_but_trim_envelope_and_crossfade_stay_local() {
    let (mut project, track, original) = audio_project();
    let clone = project.clone_clip(track, original).unwrap();
    project.trim_clip(track, clone, 5.0, 2.0).unwrap();
    let curve = AudioCrossfade {
        duration: 0.5,
        curve: CrossfadeCurve::SMOOTH,
    };
    project
        .set_audio_crossfade(track, clone, Some(curve))
        .unwrap();
    project
        .set_audio_fades(
            track,
            original,
            1.5,
            1.0,
            FadeCurve::SCurve,
            FadeCurve::EqualPower,
        )
        .unwrap();
    project.set_audio_gain(track, clone, 0.4).unwrap();
    project.reverse_audio_clip(track, clone).unwrap();
    let original_clip = project.clip(track, original).unwrap();
    let clone_clip = project.clip(track, clone).unwrap();
    assert_eq!(
        (clone_clip.start, clone_clip.length, clone_clip.offset),
        (5.0, 2.0, 1.0)
    );
    assert!(matches!(
        original_clip.content,
        ClipContent::Audio {
            gain: 0.4,
            fade_in: 1.5,
            reverse: true,
            crossfade: None,
            envelope: Some(AudioEnvelope {
                offset: 0.0,
                length: 4.0
            }),
            ..
        }
    ));
    assert!(
        matches!(clone_clip.content, ClipContent::Audio { gain: 0.4, fade_in: 1.5, reverse: true, crossfade: Some(value), envelope: Some(AudioEnvelope {offset: 1.0, length: 4.0}), .. } if value == curve)
    );
    let group = original_clip.link_group;
    // Editing a fade through a trimmed clone must keep the source-wide span
    // and every instance's independent envelope offset.
    project
        .set_audio_fades(
            track,
            clone,
            2.5,
            0.75,
            FadeCurve::EqualPower,
            FadeCurve::SCurve,
        )
        .unwrap();
    assert!(matches!(
        project.clip(track, original).unwrap().content,
        ClipContent::Audio {
            fade_in: 2.5,
            envelope: Some(AudioEnvelope {
                offset: 0.0,
                length: 4.0
            }),
            ..
        }
    ));
    assert!(matches!(
        project.clip(track, clone).unwrap().content,
        ClipContent::Audio {
            fade_in: 2.5,
            envelope: Some(AudioEnvelope {
                offset: 1.0,
                length: 4.0
            }),
            ..
        }
    ));
    let right = project.split_clip(track, clone, 6.0).unwrap();
    assert_eq!(project.clip(track, right).unwrap().link_group, group);
    project.validate().unwrap();
}

#[test]
fn audio_copy_paste_and_join_create_independent_sources() {
    let (mut project, track, original) = audio_project();
    let clone = project.clone_clip(track, original).unwrap();
    let clipboard = project.clip(track, clone).unwrap().clone();
    let paste = project.paste_audio_clip(track, &clipboard, 20.0).unwrap();
    assert_eq!(project.clip(track, paste).unwrap().link_group, None);
    let joined = project
        .replace_audio_clips_with_joined(track, &[original, clone], "joined.wav")
        .unwrap();
    assert_eq!(project.clip(track, joined).unwrap().link_group, None);
    project.validate().unwrap();

    let (mut project, track, original) = midi_project();
    let clone = project.clone_clip(track, original).unwrap();
    let third = project.clone_clip(track, clone).unwrap();
    let joined = project.join_midi_clips(track, &[original, clone]).unwrap();
    assert_eq!(project.clip(track, joined).unwrap().link_group, None);
    assert!(project.clip(track, third).unwrap().link_group.is_some());
    project.validate().unwrap();
}

#[test]
fn simultaneous_conflicting_edits_are_rejected_atomically_and_explicit_gestures_are_live() {
    let (mut project, track, original) = midi_project();
    let clone = project.clone_clip(track, original).unwrap();
    let before = project.clone();
    project.clip_mut(track, clone).unwrap().name = "A".into();
    project.synchronize_linked_clips(&before).unwrap();
    assert_eq!(project.clip(track, original).unwrap().name, "A");
    for pitch in 61..70 {
        project.midi_notes_mut(track, clone).unwrap()[0].pitch = pitch;
        project.synchronize_linked_clip(track, clone).unwrap();
        assert_eq!(project.midi_notes(track, original).unwrap()[0].pitch, pitch);
    }
    let before = project.clone();
    project.clip_mut(track, clone).unwrap().name = "B".into();
    project.clip_mut(track, original).unwrap().name = "C".into();
    let divergent = project.clone();
    assert!(project.synchronize_linked_clips(&before).is_err());
    assert_eq!(project, divergent);
}

#[test]
fn malformed_link_groups_and_cross_object_id_collisions_are_rejected() {
    let (mut valid, track, original) = midi_project();
    let clone = valid.clone_clip(track, original).unwrap();
    for group in [
        0,
        u64::MAX,
        track,
        original,
        valid.midi_notes(track, original).unwrap()[0].id,
    ] {
        let mut invalid = valid.clone();
        for clip in &mut invalid.track_mut(track).unwrap().clips {
            clip.link_group = Some(group);
        }
        assert!(invalid.validate().is_err(), "group {group}");
    }
    let mut invalid = valid.clone();
    invalid.clip_mut(track, clone).unwrap().name.push('!');
    assert!(invalid.validate().is_err());
    let mut invalid = valid.clone();
    invalid.midi_notes_mut(track, clone).unwrap()[0].pitch = 61;
    assert!(invalid.validate().is_err());
    let mut invalid = valid.clone();
    invalid.clip_mut(track, clone).unwrap().link_group = None;
    assert!(
        invalid.validate().is_err(),
        "unlinked notes must have distinct IDs"
    );
    let mut invalid = valid.clone();
    invalid.clip_mut(track, clone).unwrap().content = Clip::audio("x", 0.0, 4.0, "x.wav").content;
    assert!(invalid.validate().is_err());
    let (mut audio, track, original) = audio_project();
    audio.clone_clip(track, original).unwrap();
    if let ClipContent::Audio { envelope, .. } =
        &mut audio.clip_mut(track, original).unwrap().content
    {
        *envelope = None;
    }
    assert!(audio.validate().is_err());
}

#[test]
fn legacy_projects_default_to_independent_and_link_creation_rejects_overflow_without_mutation() {
    let (mut project, track, original) = midi_project();
    let json = serde_json::to_value(&project).unwrap();
    assert!(json["tracks"][0]["clips"][0].get("link_group").is_none());
    let loaded: Project = serde_json::from_value(json).unwrap();
    assert_eq!(loaded.clip(track, original).unwrap().link_group, None);
    project.move_clip(track, original, MAX_BEAT - 4.0).unwrap();
    let before = project.clone();
    assert!(project.clone_clip(track, original).is_err());
    assert_eq!(project, before);
}

#[test]
fn links_and_independent_sources_roundtrip_through_archive_and_history() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.wav");
    AudioData {
        sample_rate: 8000,
        channels: 1,
        samples: vec![0.25; 16000],
    }
    .write_wav(&source)
    .unwrap();
    let (mut project, track, original) = audio_project();
    if let ClipContent::Audio { path, .. } = &mut project.clip_mut(track, original).unwrap().content
    {
        *path = source.to_string_lossy().into_owned();
    }
    let mut history = History::default();
    history.commit_named(&project, "Clone clip");
    let clone = project.clone_clip(track, original).unwrap();
    let linked = project.clone();
    assert!(history.undo(&mut project));
    assert_eq!(project.track(track).unwrap().clips.len(), 1);
    assert!(project.clip(track, original).unwrap().link_group.is_none());
    assert!(history.redo(&mut project));
    assert_eq!(project, linked);
    let midi = project.add_midi_track("MIDI");
    let notes = Clip::midi("Pattern", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 90)]);
    let midi_original = notes.id;
    project.track_mut(midi).unwrap().clips.push(notes);
    let midi_clone = project.clone_clip(midi, midi_original).unwrap();
    let file = temp.path().join("clones.rill");
    project.save(&file).unwrap();
    std::fs::remove_file(source).unwrap();
    let mut recalled = Project::load_with_assets(&file, temp.path().join("assets")).unwrap();
    recalled.validate().unwrap();
    assert_eq!(
        recalled.clip(track, clone).unwrap().link_group,
        project.clip(track, clone).unwrap().link_group
    );
    recalled.set_audio_gain(track, clone, 0.8).unwrap();
    assert!(matches!(
        recalled.clip(track, original).unwrap().content,
        ClipContent::Audio { gain: 0.8, .. }
    ));
    recalled.midi_notes_mut(midi, midi_clone).unwrap()[0].pitch = 67;
    recalled.synchronize_linked_clip(midi, midi_clone).unwrap();
    assert_eq!(
        recalled.midi_notes(midi, midi_original).unwrap()[0].pitch,
        67
    );
    recalled.clone_clip(midi, midi_clone).unwrap();
    recalled.validate().unwrap();
}

#[test]
fn loading_reserves_group_ids_even_when_no_object_uses_the_same_high_id() {
    let temp = tempfile::tempdir().unwrap();
    let (mut project, track, original) = midi_project();
    project.clone_clip(track, original).unwrap();
    let group = daw_core::new_id() + 1_000_000;
    for clip in &mut project.track_mut(track).unwrap().clips {
        clip.link_group = Some(group);
    }
    let file = temp.path().join("links.json");
    project.save(&file).unwrap();
    Project::load(&file).unwrap();
    assert!(daw_core::new_id() > group);
}
