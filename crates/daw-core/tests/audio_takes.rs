use daw_core::{
    AudioData, AudioSchedule, AudioTakeMembership, AudioTakeRole, Clip, ClipContent, FadeCurve,
    History, Project, new_id,
};

fn fixture() -> (Project, u64, u64, u64) {
    let mut project = Project::new();
    let track = project.add_audio_track("Vocals");
    let a = Clip::audio("Take A", 0.0, 8.0, "a.wav");
    let b = Clip::audio("Take B", 0.0, 8.0, "b.wav");
    let ids = (a.id, b.id);
    project.track_mut(track).unwrap().clips.extend([a, b]);
    (project, track, ids.0, ids.1)
}

fn path(clip: &Clip) -> &str {
    let ClipContent::Audio { path, .. } = &clip.content else {
        panic!("audio")
    };
    path
}

fn audible(project: &Project, track: u64, beat: f64) -> (&Clip, Option<&Clip>, f32, f32) {
    let clips = &project.track(track).unwrap().clips;
    let schedule = AudioSchedule::new(clips).unwrap();
    let sample = schedule.sample(beat).expect("no unintended gap");
    (
        &clips[sample.incoming],
        sample.outgoing.map(|i| &clips[i]),
        sample.incoming_gain,
        sample.outgoing_gain,
    )
}

#[test]
fn grouping_retains_originals_and_first_argument_wins_without_extra_audio_or_export_tail() {
    let (mut project, track, a, b) = fixture();
    project.clip_mut(track, b).unwrap().length = 16.0;
    let before = project.clone();
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    assert_eq!(project.audio_take_sources(track, group).len(), 2);
    assert_eq!(project.audio_take_regions(track, group).len(), 1);
    assert_eq!(project.end_beat(), 8.0);
    for id in [a, b] {
        let original = before.clip(track, id).unwrap();
        let stored = project.clip(track, id).unwrap();
        assert!(stored.is_take_source());
        assert_eq!(
            (&stored.content, stored.start, stored.length),
            (&original.content, original.start, original.length)
        );
    }
    assert_eq!(path(audible(&project, track, 2.0).0), "a.wav");
    assert!(audible(&project, track, 2.0).1.is_none());
    assert!(
        AudioSchedule::new(&project.track(track).unwrap().clips)
            .unwrap()
            .sample(9.0)
            .is_none()
    );
    let chosen = project
        .select_audio_take(track, group, b, None, 0.02)
        .unwrap();
    assert_eq!(project.end_beat(), 16.0);
    assert_eq!(path(audible(&project, track, 9.0).0), "b.wav");
    assert_eq!(project.audio_take_regions(track, group)[0].id, chosen);

    let (mut reversed, track, a, b) = fixture();
    reversed.group_audio_takes(track, &[b, a]).unwrap();
    assert_eq!(path(audible(&reversed, track, 1.0).0), "b.wav");
}

#[test]
fn range_selection_preserves_outside_source_and_adds_smooth_crossfades_at_both_edges() {
    let (mut project, track, a, b) = fixture();
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    let sources: Vec<_> = project
        .audio_take_sources(track, group)
        .into_iter()
        .cloned()
        .collect();
    let selected = project
        .select_audio_take(track, group, b, Some((2.0, 4.0)), 0.125)
        .unwrap();
    let clips = &project.track(track).unwrap().clips;
    let schedule = AudioSchedule::new(clips).unwrap();
    assert_eq!(schedule.crossfades.len(), 2);
    assert_eq!(
        (schedule.crossfades[0].start, schedule.crossfades[0].end),
        (2.0, 2.125)
    );
    assert_eq!(
        (schedule.crossfades[1].start, schedule.crossfades[1].end),
        (4.0, 4.125)
    );
    assert_eq!(project.audio_take_regions(track, group).len(), 3);
    for (beat, expected) in [
        (0.0, "a.wav"),
        (1.0, "a.wav"),
        (3.0, "b.wav"),
        (5.0, "a.wav"),
        (7.999, "a.wav"),
    ] {
        let (clip, outgoing, gain, _) = audible(&project, track, beat);
        assert_eq!(path(clip), expected);
        assert_eq!(
            clip.audio_source_frame(beat - clip.start, 120.0, 8_000),
            beat * 4_000.0
        );
        assert!(outgoing.is_none());
        assert_eq!(gain, 1.0);
    }
    for (beat, incoming, outgoing) in [(2.0625, "b.wav", "a.wav"), (4.0625, "a.wav", "b.wav")] {
        let (clip, old, gain, old_gain) = audible(&project, track, beat);
        assert_eq!((path(clip), path(old.unwrap())), (incoming, outgoing));
        assert!((gain - 0.5).abs() < 1e-6);
        assert!((gain + old_gain - 1.0).abs() < 1e-6);
    }
    assert!(
        project
            .audio_take_regions(track, group)
            .iter()
            .any(|clip| clip.id == selected)
    );
    assert_eq!(
        project
            .audio_take_sources(track, group)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        sources
    );
    // Repeatedly changing the same passage must not accumulate crossfade tails
    // or mutate the stored candidates.
    for _ in 0..16 {
        project
            .select_audio_take(track, group, b, Some((2.0, 4.0)), 0.125)
            .unwrap();
        assert_eq!(project.audio_take_regions(track, group).len(), 3);
        let schedule = AudioSchedule::new(&project.track(track).unwrap().clips).unwrap();
        assert_eq!(schedule.crossfades.len(), 2);
        project.validate().unwrap();
    }
}

#[test]
fn reverse_stretch_and_fade_origins_survive_slicing_and_source_handle_extension() {
    let (mut project, track, a, b) = fixture();
    for id in [a, b] {
        let clip = project.clip_mut(track, id).unwrap();
        clip.offset = 3.0;
        clip.source_rate = 0.5;
        if let ClipContent::Audio {
            reverse,
            fade_in,
            fade_out,
            fade_in_curve,
            ..
        } = &mut clip.content
        {
            *reverse = true;
            *fade_in = 1.0;
            *fade_out = 1.0;
            *fade_in_curve = FadeCurve::SCurve;
        }
    }
    let original_a = project.clip(track, a).unwrap().clone();
    let original_b = project.clip(track, b).unwrap().clone();
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    project
        .select_audio_take(track, group, b, Some((2.0, 4.0)), 0.125)
        .unwrap();
    for beat in [0.0, 1.0, 2.0625, 3.0, 4.0625, 5.0, 7.5] {
        let (incoming, outgoing, _, _) = audible(&project, track, beat);
        for clip in std::iter::once(incoming).chain(outgoing) {
            let original = if path(clip) == "a.wav" {
                &original_a
            } else {
                &original_b
            };
            assert_eq!(
                clip.audio_source_frame(beat - clip.start, 120.0, 8_000),
                original.audio_source_frame(beat, 120.0, 8_000)
            );
            assert_eq!(
                clip.audio_envelope_gain(beat - clip.start),
                original.audio_envelope_gain(beat)
            );
        }
    }
    assert_eq!(
        AudioSchedule::new(&project.track(track).unwrap().clips)
            .unwrap()
            .crossfades
            .len(),
        2
    );
}

#[test]
fn missing_or_short_source_handles_clamp_crossfades_without_gaps_or_extra_duration() {
    let (mut project, track, a, b) = fixture();
    project.clip_mut(track, b).unwrap().length = 4.0;
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    project
        .select_audio_take(track, group, b, Some((2.0, 3.9375)), 0.125)
        .unwrap();
    let schedule = AudioSchedule::new(&project.track(track).unwrap().clips).unwrap();
    assert_eq!(schedule.crossfades.len(), 2);
    assert_eq!(
        (schedule.crossfades[1].start, schedule.crossfades[1].end),
        (3.9375, 4.0)
    );
    project
        .select_audio_take(track, group, b, Some((2.0, 4.0)), 0.125)
        .unwrap();
    let schedule = AudioSchedule::new(&project.track(track).unwrap().clips).unwrap();
    assert_eq!(
        schedule.crossfades.len(),
        1,
        "take ending at its source boundary has no outgoing handle"
    );
    assert_eq!(path(audible(&project, track, 4.0).0), "a.wav");
    assert_eq!(project.end_beat(), 8.0);
    for step in 0..8000 {
        assert!(schedule.sample(step as f64 / 1000.0).is_some());
    }
}

#[test]
fn comp_tools_remain_editable_while_copies_detach_and_sources_remain_safe() {
    let (mut project, track, a, b) = fixture();
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    let comp = project.audio_take_regions(track, group)[0].id;
    let right = project.split_clip(track, comp, 4.0).unwrap();
    assert_eq!(
        project.clip(track, right).unwrap().take_group(),
        Some(group)
    );
    project.move_clip(track, right, 5.0).unwrap();
    project.stretch_clip(track, right, 3.0).unwrap();
    project.set_audio_gain(track, right, 0.5).unwrap();
    assert_ne!(
        project.clip(track, right).unwrap().content,
        project.clip(track, a).unwrap().content
    );
    let duplicate = project.duplicate_clip_at(track, right, 12.0).unwrap();
    assert_eq!(project.clip(track, duplicate).unwrap().audio_take, None);
    let ghost = project.clone_clip(track, right).unwrap();
    assert_eq!(project.clip(track, ghost).unwrap().audio_take, None);
    assert_eq!(
        project.clip(track, ghost).unwrap().link_group,
        project.clip(track, right).unwrap().link_group
    );
    let overdub = project.add_audio_track("Overdub");
    let source = project.clip(track, b).unwrap().clone();
    let pasted = project.paste_audio_clip(overdub, &source, 0.0).unwrap();
    assert_eq!(project.clip(overdub, pasted).unwrap().audio_take, None);
    project.validate().unwrap();
    let before = project.clone();
    assert!(project.move_clip(track, a, 1.0).is_err());
    assert!(project.split_clip(track, a, 1.0).is_err());
    assert!(project.trim_clip(track, a, 1.0, 4.0).is_err());
    assert!(project.stretch_clip(track, a, 4.0).is_err());
    assert!(project.clone_clip(track, a).is_err());
    assert!(project.delete_clip(track, a).is_err());
    assert!(project.reverse_audio_clip(track, a).is_err());
    assert_eq!(project, before);
}

#[test]
fn comp_join_ignores_archived_candidates_and_keeps_them_available() {
    let (mut project, track, a, b) = fixture();
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    let comp = project.audio_take_regions(track, group)[0].id;
    let right = project.split_clip(track, comp, 4.0).unwrap();
    assert_eq!(
        project.audio_join_range(track, &[comp, right]).unwrap(),
        (0.0, 8.0)
    );
    let joined = project
        .replace_audio_clips_with_joined(track, &[comp, right], "joined.wav")
        .unwrap();
    assert_eq!(project.clip(track, joined).unwrap().audio_take, None);
    assert_eq!(project.audio_take_sources(track, group).len(), 2);
    assert!(project.audio_take_regions(track, group).is_empty());
    assert!(project.audio_join_range(track, &[a, b]).is_err());
    project.validate().unwrap();
}

#[test]
fn adding_and_detaching_takes_preserves_audio_and_undo_restores_candidates() {
    let (mut project, track, a, b) = fixture();
    let group = project.group_audio_takes(track, &[a]).unwrap();
    project.add_audio_take(track, group, b).unwrap();
    project
        .select_audio_take(track, group, b, Some((2.0, 4.0)), 0.125)
        .unwrap();
    let before = project.clone();
    let mut history = History::new();
    history.commit_named(&project, "Keep comp");
    project.detach_audio_take_group(track, group).unwrap();
    assert_eq!(project.track(track).unwrap().clips.len(), 3);
    assert!(
        project
            .track(track)
            .unwrap()
            .clips
            .iter()
            .all(|clip| clip.audio_take.is_none())
    );
    for beat in [0.0, 2.0, 2.0625, 3.0, 4.0625, 7.0] {
        let (new, old, gain, old_gain) = audible(&project, track, beat);
        let (prior, prior_old, prior_gain, prior_old_gain) = audible(&before, track, beat);
        assert_eq!(
            (new.id, old.map(|clip| clip.id), gain, old_gain),
            (
                prior.id,
                prior_old.map(|clip| clip.id),
                prior_gain,
                prior_old_gain
            )
        );
    }
    let detached = project.clone();
    assert!(history.undo(&mut project));
    assert_eq!(project, before);
    assert!(history.redo(&mut project));
    assert_eq!(project, detached);
}

#[test]
fn malformed_groups_ranges_and_references_are_rejected_atomically() {
    let (mut project, track, a, b) = fixture();
    let before = project.clone();
    assert!(project.group_audio_takes(track, &[]).is_err());
    assert!(project.group_audio_takes(track, &[a, a]).is_err());
    assert!(project.group_audio_takes(track, &[a, u64::MAX]).is_err());
    assert_eq!(project, before);
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    let before = project.clone();
    for range in [
        (f64::NAN, 2.0),
        (0.0, f64::INFINITY),
        (-1.0, 2.0),
        (1.0, 9.0),
        (3.0, 2.0),
        (2.0, 2.0),
    ] {
        assert!(
            project
                .select_audio_take(track, group, b, Some(range), 0.02)
                .is_err()
        );
        assert_eq!(project, before);
    }
    assert!(
        project
            .select_audio_take(track, group, b, None, f64::NAN)
            .is_err()
    );
    assert!(
        project
            .select_audio_take(track, group, u64::MAX, None, 0.02)
            .is_err()
    );
    assert!(project.detach_audio_take_group(track, u64::MAX).is_err());
    assert!(project.add_audio_take(track, group, b).is_err());
    assert_eq!(project, before);
    let comp = project.audio_take_regions(track, group)[0].id;
    let mut corrupt = before.clone();
    corrupt
        .clip_mut(track, comp)
        .unwrap()
        .audio_take
        .as_mut()
        .unwrap()
        .role = AudioTakeRole::Comp {
        source: b + 1_000_000,
    };
    assert!(corrupt.validate().is_err());
    let mut corrupt = before.clone();
    for clip in &mut corrupt.track_mut(track).unwrap().clips {
        clip.audio_take.as_mut().unwrap().group = track;
    }
    assert!(
        corrupt.validate().is_err(),
        "take group ID cannot collide with track ID"
    );
    let mut corrupt = before.clone();
    let another = corrupt.add_audio_track("Elsewhere");
    let mut source = corrupt.clip(track, a).unwrap().clone();
    source.id = new_id();
    corrupt.track_mut(another).unwrap().clips.push(source);
    assert!(corrupt.validate().is_err(), "take groups stay on one track");
    let mut corrupt = before;
    let midi = corrupt.add_midi_track("Notes");
    let mut clip = Clip::midi("No", 0.0, 4.0, vec![]);
    clip.audio_take = Some(AudioTakeMembership {
        group: new_id(),
        role: AudioTakeRole::Source,
    });
    corrupt.track_mut(midi).unwrap().clips.push(clip);
    assert!(corrupt.validate().is_err());
}

#[test]
fn portable_archive_preserves_inactive_sources_and_legacy_projects_have_no_takes() {
    let temp = tempfile::tempdir().unwrap();
    let (mut project, track, a, b) = fixture();
    for (id, value) in [(a, 0.1), (b, 0.2)] {
        let filename = temp.path().join(format!("{id}.wav"));
        AudioData {
            sample_rate: 8_000,
            channels: 1,
            samples: vec![value; 32_000],
        }
        .write_wav(&filename)
        .unwrap();
        let ClipContent::Audio { path, .. } = &mut project.clip_mut(track, id).unwrap().content
        else {
            panic!()
        };
        *path = filename.to_string_lossy().into_owned();
    }
    let legacy = temp.path().join("legacy.json");
    let mut old = serde_json::json!({"version":3,"project":project});
    for clip in old["project"]["tracks"][0]["clips"].as_array_mut().unwrap() {
        clip.as_object_mut().unwrap().remove("audio_take");
    }
    std::fs::write(&legacy, serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(Project::load(&legacy).unwrap(), project);
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    let json = temp.path().join("takes.json");
    project.save(&json).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(manifest["version"], 4);
    assert_eq!(Project::load(&json).unwrap(), project);
    let bundle = temp.path().join("takes.rill");
    project.save(&bundle).unwrap();
    let originals = project
        .audio_take_sources(track, group)
        .into_iter()
        .map(|clip| (clip.id, std::fs::read(path(clip)).unwrap()))
        .collect::<Vec<_>>();
    for clip in project.audio_take_sources(track, group) {
        std::fs::remove_file(path(clip)).unwrap();
    }
    let mut restored = Project::load_with_assets(&bundle, temp.path().join("scratch")).unwrap();
    for (id, bytes) in originals {
        assert_eq!(
            std::fs::read(path(restored.clip(track, id).unwrap())).unwrap(),
            bytes
        );
    }
    restored
        .select_audio_take(track, group, b, None, 0.02)
        .unwrap();
    assert_eq!(
        audible(&restored, track, 1.0).0.audio_take.unwrap().role,
        AudioTakeRole::Comp { source: b }
    );
    restored.validate().unwrap();
}

#[test]
fn loading_reserves_take_group_ids_and_untrusted_group_collisions_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let (mut project, track, a, b) = fixture();
    project.group_audio_takes(track, &[a, b]).unwrap();
    let high_group = new_id() + 10_000_000;
    for clip in &mut project.track_mut(track).unwrap().clips {
        clip.audio_take.as_mut().unwrap().group = high_group;
    }
    let file = temp.path().join("reserved.json");
    project.save(&file).unwrap();
    Project::load(&file).unwrap();
    assert!(new_id() > high_group);
    let comp = project.audio_take_regions(track, high_group)[0].id;
    let ghost = project.clone_clip(track, comp).unwrap();
    let link = project.clip(track, ghost).unwrap().link_group.unwrap();
    for clip in &mut project.track_mut(track).unwrap().clips {
        if let Some(take) = &mut clip.audio_take {
            take.group = link;
        }
    }
    assert!(
        project.validate().is_err(),
        "take and ghost groups cannot share an ID"
    );
}

#[test]
fn overlapping_comp_gestures_keep_exact_primary_source_mapping_and_never_sum_extra_takes() {
    let (mut project, track, a, b) = fixture();
    project.clip_mut(track, a).unwrap().source_rate = 0.5;
    project.reverse_audio_clip(track, b).unwrap();
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    let originals = [
        (a, project.clip(track, a).unwrap().clone()),
        (b, project.clip(track, b).unwrap().clone()),
    ];
    let mut expected = [a; 80];
    for (left, right, source) in [
        (10, 70, b),
        (35, 45, a),
        (20, 50, b),
        (0, 30, a),
        (65, 80, a),
        (40, 75, b),
        (15, 25, b),
        (5, 12, a),
    ] {
        project
            .select_audio_take(
                track,
                group,
                source,
                Some((left as f64 / 10.0, right as f64 / 10.0)),
                0.015,
            )
            .unwrap();
        expected[left..right].fill(source);
        let clips = &project.track(track).unwrap().clips;
        let schedule = AudioSchedule::new(clips).unwrap();
        for (index, source) in expected.iter().enumerate() {
            let beat = index as f64 / 10.0 + 0.05;
            let sample = schedule.sample(beat).expect("comp cannot create a gap");
            let clip = &clips[sample.incoming];
            assert_eq!(
                clip.audio_take.unwrap().role,
                AudioTakeRole::Comp { source: *source }
            );
            assert!(
                sample.outgoing.is_none(),
                "only boundary handles may overlap"
            );
            let original = &originals.iter().find(|(id, _)| id == source).unwrap().1;
            assert!(
                (clip.audio_source_frame(beat - clip.start, 120.0, 8_000)
                    - original.audio_source_frame(beat, 120.0, 8_000))
                .abs()
                    < 1e-6
            );
        }
        for (_, original) in &originals {
            assert_eq!(project.clip(track, original.id).unwrap(), original);
        }
        assert_eq!(project.end_beat(), 8.0);
        project.validate().unwrap();
    }
}

#[test]
fn comp_edits_use_source_coordinate_handles_after_reversing_and_stretching_the_audible_region() {
    let (mut project, track, a, b) = fixture();
    let group = project.group_audio_takes(track, &[a, b]).unwrap();
    let comp = project.audio_take_regions(track, group)[0].id;
    project.stretch_clip(track, comp, 4.0).unwrap();
    project.reverse_audio_clip(track, comp).unwrap();
    project
        .set_audio_fades(track, comp, 0.5, 0.75, FadeCurve::SCurve, FadeCurve::SCurve)
        .unwrap();
    let edited = project.clip(track, comp).unwrap().clone();
    let candidate = project.clip(track, b).unwrap().clone();
    project
        .select_audio_take(track, group, b, Some((2.0, 3.0)), 0.125)
        .unwrap();
    let clips = &project.track(track).unwrap().clips;
    let schedule = AudioSchedule::new(clips).unwrap();
    assert_eq!(schedule.crossfades.len(), 2);
    for beat in [0.25, 1.5, 2.0625, 2.5, 3.0625, 3.75] {
        let (incoming, outgoing, _, _) = audible(&project, track, beat);
        for clip in std::iter::once(incoming).chain(outgoing) {
            let original = if path(clip) == "a.wav" {
                &edited
            } else {
                &candidate
            };
            assert!(
                (clip.audio_source_frame(beat - clip.start, 120.0, 8_000)
                    - original.audio_source_frame(beat, 120.0, 8_000))
                .abs()
                    < 1e-6
            );
            assert!(
                (clip.audio_envelope_gain(beat - clip.start) - original.audio_envelope_gain(beat))
                    .abs()
                    < 1e-6
            );
        }
    }
    assert_eq!(project.end_beat(), 4.0, "unused source tails stay silent");
}
