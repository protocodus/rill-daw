use daw_core::{AudioData, Clip, ClipContent, FadeCurve, Note, Project, normalization_gain};

#[test]
fn duplicate_at_validates_atomically_and_can_copy_left_from_the_timeline_limit() {
    let mut project = Project::new();
    let track = project.add_midi_track("Boundary");
    let clip = Clip::midi(
        "Shared source",
        0.0,
        1.0,
        vec![Note::new(60, 0.0, 1.0, 100)],
    );
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let ghost = project.clone_clip(track, id).unwrap();
    project
        .move_clip(track, id, daw_core::MAX_BEAT - 1.0)
        .unwrap();
    let before = project.clone();
    assert!(project.duplicate_clip(track, id).is_err());
    for start in [-1.0, f64::NAN, f64::INFINITY, daw_core::MAX_BEAT] {
        assert!(project.duplicate_clip_at(track, id, start).is_err());
        assert_eq!(project, before);
    }
    let duplicate = project.duplicate_clip_at(track, id, 4.0).unwrap();
    let original = project.clip(track, id).unwrap();
    let copied = project.clip(track, duplicate).unwrap();
    assert_eq!(copied.start, 4.0);
    assert_eq!(copied.name, original.name);
    assert_eq!(copied.source_rate, original.source_rate);
    assert_eq!(copied.link_group, None);
    assert_eq!(
        original.link_group,
        project.clip(track, ghost).unwrap().link_group
    );
    let (ClipContent::Midi { notes: original }, ClipContent::Midi { notes: copied }) =
        (&original.content, &copied.content)
    else {
        panic!()
    };
    assert_ne!(original[0].id, copied[0].id);
    assert_eq!(
        (original[0].pitch, original[0].start, original[0].length),
        (copied[0].pitch, copied[0].start, copied[0].length)
    );
    project.validate().unwrap();
}

#[test]
fn transient_detection_keeps_narrow_source_attacks_at_compressed_rates() {
    let mut data = AudioData {
        sample_rate: 8000,
        channels: 1,
        samples: vec![0.0; 32000],
    };
    data.samples[16001] = 0.5;
    for rate in [0.25, 1.0, 4.0] {
        let mut clip = Clip::audio("Transient", 0.0, 8.0 / rate, "source.wav");
        clip.source_rate = rate;
        for algorithm in daw_core::TransientAlgorithm::ALL {
            let positions = daw_core::detect_transients(
                &clip,
                &data,
                120.0,
                daw_core::TransientSettings {
                    algorithm,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                positions.len(),
                1,
                "rate {rate}, {algorithm:?}: {positions:?}"
            );
            assert!((positions[0] - 4.0 / rate).abs() <= 0.01);
        }
    }
}

#[test]
fn stretching_ghost_timing_keeps_shared_notes_and_source_windows() {
    let mut project = Project::new();
    let track = project.add_midi_track("Phrase");
    let mut clip = Clip::midi("Source", 4.0, 4.0, vec![Note::new(60, 2.0, 3.0, 100)]);
    clip.offset = 1.0;
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let ghost = project.clone_clip(track, id).unwrap();
    let content = project.clip(track, id).unwrap().content.clone();
    project.stretch_clip(track, ghost, 8.0).unwrap();
    let stretched = project.clip(track, ghost).unwrap();
    assert_eq!(stretched.source_rate, 0.5);
    assert_eq!(stretched.source_end(), 5.0);
    assert_eq!(stretched.source_to_local(2.0), 2.0);
    assert_eq!(stretched.local_to_source(2.0), 2.0);
    assert_eq!(stretched.content, content);
    assert_eq!(project.clip(track, id).unwrap().source_rate, 1.0);
    project.trim_clip(track, ghost, 10.0, 6.0).unwrap();
    assert_eq!(project.clip(track, ghost).unwrap().offset, 2.0);
    let right = project.split_clip(track, ghost, 12.0).unwrap();
    assert_eq!(project.clip(track, right).unwrap().offset, 3.0);
    assert_eq!(project.clip(track, right).unwrap().source_rate, 0.5);
    assert_eq!(project.clip(track, right).unwrap().content, content);
    project.validate().unwrap();
}

#[test]
fn scaled_audio_split_and_trim_preserve_reverse_and_fade_coordinates() {
    for reversed in [false, true] {
        let mut project = Project::new();
        let track = project.add_audio_track("Audio");
        let mut clip = Clip::audio("Source", 2.0, 4.0, "source.wav");
        clip.offset = 1.0;
        let id = clip.id;
        if let ClipContent::Audio {
            reverse,
            fade_in,
            fade_out,
            fade_in_curve,
            ..
        } = &mut clip.content
        {
            *reverse = reversed;
            *fade_in = 1.0;
            *fade_out = 1.0;
            *fade_in_curve = FadeCurve::EqualPower;
        }
        project.track_mut(track).unwrap().clips.push(clip);
        project.stretch_clip(track, id, 8.0).unwrap();
        let original = project.clip(track, id).unwrap().clone();
        let right_id = project.split_clip(track, id, 5.0).unwrap();
        for (part_id, shift) in [(id, 0.0), (right_id, 3.0)] {
            let part = project.clip(track, part_id).unwrap();
            for step in 0..32 {
                let local = part.length * step as f64 / 32.0;
                assert!(
                    (part.audio_source_frame(local, 120.0, 8000)
                        - original.audio_source_frame(shift + local, 120.0, 8000))
                    .abs()
                        < 1e-8
                );
                assert!(
                    (part.audio_envelope_gain(local) - original.audio_envelope_gain(shift + local))
                        .abs()
                        < 1e-6
                );
            }
        }
        project.trim_clip(track, right_id, 6.0, 3.0).unwrap();
        let trimmed = project.clip(track, right_id).unwrap();
        assert_eq!(
            trimmed.audio_source_frame(0.0, 120.0, 8000),
            original.audio_source_frame(4.0, 120.0, 8000)
        );
        project.validate().unwrap();
    }
}

#[test]
fn joining_scaled_midi_bakes_visible_timing_into_an_independent_clip() {
    let mut project = Project::new();
    let track = project.add_midi_track("MIDI");
    let mut a = Clip::midi("Slow", 0.0, 2.0, vec![Note::new(60, 1.5, 1.0, 100)]);
    a.offset = 1.0;
    let mut b = Clip::midi("Fast", 6.0, 2.0, vec![Note::new(62, 0.5, 1.0, 100)]);
    let (a_id, b_id) = (a.id, b.id);
    b.offset = 0.25;
    project.track_mut(track).unwrap().clips = vec![a, b];
    project.stretch_clip(track, a_id, 4.0).unwrap();
    project.stretch_clip(track, b_id, 1.0).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let midi = temp.path().join("stretched.mid");
    daw_core::export_midi(&project, &midi).unwrap();
    let imported = daw_core::import_midi(&midi).unwrap();
    let ClipContent::Midi { notes: exported } = &imported.tracks[0].clips[0].content else {
        panic!()
    };
    assert_eq!((exported[0].start, exported[0].length), (1.0, 2.0));
    assert_eq!((exported[1].start, exported[1].length), (6.125, 0.5));
    let joined = project.join_midi_clips(track, &[a_id, b_id]).unwrap();
    let clip = project.clip(track, joined).unwrap();
    assert_eq!(clip.source_rate, 1.0);
    let ClipContent::Midi { notes } = &clip.content else {
        panic!()
    };
    assert_eq!((notes[0].start, notes[0].length), (1.0, 2.0));
    assert_eq!((notes[1].start, notes[1].length), (6.125, 0.5));
}

#[test]
fn invalid_stretch_is_atomic_and_audio_normalization_uses_the_original_window() {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    let mut clip = Clip::audio("Window", 0.0, 1.0, "source.wav");
    clip.offset = 1.0;
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let original = project.clone();
    for length in [0.0, -1.0, f64::NAN, f64::INFINITY, 0.125, 8.0] {
        assert!(project.stretch_clip(track, id, length).is_err());
        assert_eq!(project, original);
    }
    project.stretch_clip(track, id, 2.0).unwrap();
    let data = AudioData {
        sample_rate: 8000,
        channels: 1,
        samples: (0..16000)
            .map(|i| if (4000..8000).contains(&i) { 0.25 } else { 1.0 })
            .collect(),
    };
    assert_eq!(
        normalization_gain(project.clip(track, id).unwrap(), &data, 120.0, 0.0).unwrap(),
        4.0
    );
    let clip = project.clip_mut(track, id).unwrap();
    clip.offset = 0.0;
    clip.source_rate = 1.0;
    clip.length = daw_core::MAX_BEAT;
    let before = project.clone();
    project.validate().unwrap();
    assert!(project.stretch_clip(track, id, 579500.0).is_err());
    assert_eq!(project, before);
}

#[test]
fn versioned_projects_round_trip_rates_and_version_one_defaults_to_unstretched() {
    let temp = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let track = project.add_midi_track("MIDI");
    let clip = Clip::midi("Notes", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 100)]);
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    project.stretch_clip(track, id, 8.0).unwrap();
    for extension in ["json", "rill"] {
        let path = temp.path().join(format!("session.{extension}"));
        project.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), project);
        if extension == "json" {
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(value["version"], 4);
        }
    }
    let mut legacy = serde_json::json!({"version":1,"project":project});
    legacy["project"]["tracks"][0]["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_rate");
    let path = temp.path().join("legacy.json");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(
        Project::load(&path)
            .unwrap()
            .clip(track, id)
            .unwrap()
            .source_rate,
        1.0
    );
    legacy["version"] = 5.into();
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(Project::load(&path).is_err());
}
