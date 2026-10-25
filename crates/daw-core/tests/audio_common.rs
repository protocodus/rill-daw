use daw_core::{AudioData, Clip, ClipContent, FadeCurve, Project, nearest_audio_zero_crossing};

fn fixture(reverse: bool) -> (Project, u64, u64) {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    let mut clip = Clip::audio("Source", 2.0, 4.0, "source.wav");
    clip.offset = 1.0;
    clip.source_rate = 0.5;
    if let ClipContent::Audio {
        reverse: value,
        fade_in,
        fade_out,
        fade_in_curve,
        gain,
        ..
    } = &mut clip.content
    {
        *value = reverse;
        *fade_in = 1.0;
        *fade_out = 0.5;
        *fade_in_curve = FadeCurve::SCurve;
        *gain = 0.4;
    }
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    (project, track, id)
}

#[test]
fn audio_range_copy_and_gap_delete_preserve_stretched_reverse_sources_and_envelopes() {
    for reverse in [false, true] {
        let (mut project, track, clip) = fixture(reverse);
        let original = project.clip(track, clip).unwrap().clone();
        let copy = project.copy_audio_range(track, clip, 3.0, 4.5).unwrap();
        assert_eq!((copy.start, copy.length), (3.0, 1.5));
        for i in 0..150 {
            let beat = i as f64 * 0.01;
            assert!(
                (copy.audio_source_frame(beat, 120.0, 48000)
                    - original.audio_source_frame(1.0 + beat, 120.0, 48000))
                .abs()
                    < 1e-8
            );
            assert!(
                (copy.audio_envelope_gain(beat) - original.audio_envelope_gain(1.0 + beat)).abs()
                    < 1e-6
            );
        }
        let ids = project.delete_audio_range(track, clip, 3.0, 4.5).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], clip);
        for id in ids {
            let region = project.clip(track, id).unwrap();
            for i in 0..100 {
                let local = region.length * i as f64 / 100.0;
                let original_local = region.start - original.start + local;
                assert!(
                    (region.audio_source_frame(local, 120.0, 48000)
                        - original.audio_source_frame(original_local, 120.0, 48000))
                    .abs()
                        < 1e-8
                );
                assert!(
                    (region.audio_envelope_gain(local)
                        - original.audio_envelope_gain(original_local))
                    .abs()
                        < 1e-6
                );
            }
        }
        project.validate().unwrap();
    }
}

#[test]
fn deletion_is_atomic_and_keeps_ghosts_and_retained_takes() {
    let (mut project, track, clip) = fixture(false);
    let ghost = project.clone_clip(track, clip).unwrap();
    let before_ghost = project.clip(track, ghost).unwrap().clone();
    project.delete_audio_range(track, clip, 3.0, 4.0).unwrap();
    assert_eq!(project.clip(track, ghost).unwrap(), &before_ghost);
    let before = project.clone();
    assert!(project.delete_audio_range(track, clip, 2.5, 4.0).is_err());
    assert_eq!(project, before);

    let (mut project, track, clip) = fixture(false);
    let second = project.duplicate_clip_at(track, clip, 2.0).unwrap();
    let group = project.group_audio_takes(track, &[clip, second]).unwrap();
    let sources: Vec<_> = project
        .audio_take_sources(track, group)
        .into_iter()
        .cloned()
        .collect();
    let comp = project.audio_take_regions(track, group)[0].id;
    project.delete_audio_range(track, comp, 3.0, 4.0).unwrap();
    assert_eq!(project.audio_take_regions(track, group).len(), 2);
    assert_eq!(
        project
            .audio_take_sources(track, group)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        sources
    );
    assert!(project.delete_audio_range(track, clip, 3.0, 4.0).is_err());
    let copy = project
        .copy_audio_range(
            track,
            project.audio_take_regions(track, group)[0].id,
            2.0,
            3.0,
        )
        .unwrap();
    assert!(copy.audio_take.is_none());
    assert!(copy.link_group.is_none());
}

#[test]
fn slip_respects_reverse_stretch_source_bounds_and_ghost_locality() {
    for reverse in [false, true] {
        let (mut project, track, clip) = fixture(reverse);
        let ghost = project.clone_clip(track, clip).unwrap();
        let before = project.clip(track, clip).unwrap().clone();
        let other = project.clip(track, ghost).unwrap().clone();
        project.slip_audio_clip(track, clip, 0.5, 8.0).unwrap();
        let slipped = project.clip(track, clip).unwrap();
        assert_eq!(
            (slipped.start, slipped.length),
            (before.start, before.length)
        );
        assert_eq!(slipped.content, before.content);
        assert_eq!(slipped.offset, if reverse { 1.25 } else { 0.75 });
        project.synchronize_linked_clip(track, clip).unwrap();
        assert_eq!(project.clip(track, ghost).unwrap(), &other);
        project.slip_audio_clip(track, clip, 100.0, 8.0).unwrap();
        assert_eq!(
            project.clip(track, clip).unwrap().offset,
            if reverse { 6.0 } else { 0.0 }
        );
        project.slip_audio_clip(track, clip, -100.0, 8.0).unwrap();
        assert_eq!(
            project.clip(track, clip).unwrap().offset,
            if reverse { 0.0 } else { 6.0 }
        );
        let before = project.clone();
        assert!(project.slip_audio_clip(track, clip, 0.5, 1.0).is_err());
        assert_eq!(project, before);
    }
}

#[test]
fn zero_crossings_use_one_multichannel_source_frame_and_bounded_search() {
    let mut data = AudioData {
        sample_rate: 8000,
        channels: 2,
        samples: vec![0.3; 8000 * 2],
    };
    // The left-only crossing at frame 4000 is loud in the right channel.
    data.samples[4000 * 2] = -0.3;
    data.samples[4000 * 2 + 1] = 0.9;
    data.samples[4008 * 2] = 0.0;
    data.samples[4008 * 2 + 1] = 0.0;
    let mut clip = Clip::audio("Stereo", 2.0, 2.0, "source.wav");
    let beat = nearest_audio_zero_crossing(&clip, &data, 120.0, 3.0).unwrap();
    assert!((beat - 3.002).abs() < 1e-9);
    assert_eq!(
        clip.audio_source_frame(beat - clip.start, 120.0, 8000)
            .round(),
        4008.0
    );
    assert!(nearest_audio_zero_crossing(&clip, &data, 120.0, 2.5).is_none());
    // Trimming left can expose a valid source handle outside current edges.
    let mut trimmed = clip.clone();
    trimmed.offset = 1.25;
    assert_eq!(
        nearest_audio_zero_crossing(&trimmed, &data, 120.0, 1.75),
        Some(1.752)
    );
    if let ClipContent::Audio { reverse, .. } = &mut clip.content {
        *reverse = true;
    }
    clip.source_rate = 0.5;
    clip.length = 4.0;
    let target = clip.start + clip.length - 4001.0 / 4000.0 / clip.source_rate;
    let beat = nearest_audio_zero_crossing(&clip, &data, 120.0, target).unwrap();
    assert_eq!(
        clip.audio_source_frame(beat - clip.start, 120.0, 8000)
            .round(),
        4008.0
    );
}

#[test]
fn ripple_closes_selected_track_time_and_keeps_alternate_takes_aligned() {
    let (mut project, track, clip) = fixture(false);
    let second = project.duplicate_clip_at(track, clip, 2.0).unwrap();
    let group = project.group_audio_takes(track, &[clip, second]).unwrap();
    let original_sources: Vec<_> = project
        .audio_take_sources(track, group)
        .into_iter()
        .cloned()
        .collect();
    let later = Clip::audio("Later", 8.0, 1.0, "later.wav");
    let later_id = later.id;
    project.track_mut(track).unwrap().clips.push(later);
    let other = project.add_audio_track("Unchanged");
    project
        .track_mut(other)
        .unwrap()
        .clips
        .push(Clip::audio("Other", 8.0, 1.0, "other.wav"));
    let before_other = project.track(other).unwrap().clone();
    project.ripple_delete_audio_range(track, 3.0, 4.0).unwrap();
    assert_eq!(project.clip(track, later_id).unwrap().start, 7.0);
    assert_eq!(project.track(other).unwrap(), &before_other);
    assert_eq!(
        project
            .audio_take_sources(track, group)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        original_sources
    );
    let shifted = project
        .track(track)
        .unwrap()
        .clips
        .iter()
        .find(|clip| !clip.is_take_source() && clip.start == 3.0)
        .unwrap()
        .clone();
    assert_ne!(shifted.take_group(), Some(group));
    let right_group = shifted.take_group().unwrap();
    let sources = project.audio_take_sources(track, right_group);
    assert_eq!(sources.len(), 2);
    assert_eq!((sources[1].start, sources[1].end()), (3.0, 5.0));
    let candidate = sources[1].id;
    let chosen = project
        .select_audio_take(track, right_group, candidate, Some((3.0, 5.0)), 0.0)
        .unwrap();
    let replacement = project.clip(track, chosen).unwrap();
    for local in [0.0, 0.5, 1.99] {
        assert_eq!(
            replacement.audio_source_frame(local, 120.0, 48000),
            shifted.audio_source_frame(local, 120.0, 48000)
        );
    }
    project.validate().unwrap();
    let before = project.clone();
    assert!(project.ripple_delete_audio_range(track, 4.0, 3.0).is_err());
    assert_eq!(project, before);
}

#[test]
fn ripple_preserves_moved_slipped_stretched_and_reversed_comp_take_mapping() {
    for reverse in [false, true] {
        let (mut project, track, source) = fixture(reverse);
        let alternate = project.duplicate_clip_at(track, source, 2.0).unwrap();
        let group = project
            .group_audio_takes(track, &[source, alternate])
            .unwrap();
        let comp = project.audio_take_regions(track, group)[0].id;
        project.trim_clip(track, comp, 2.5, 3.0).unwrap();
        project.stretch_clip(track, comp, 1.5).unwrap();
        project.slip_audio_clip(track, comp, 0.125, 8.0).unwrap();
        project.reverse_audio_clip(track, comp).unwrap();
        project.move_clip(track, comp, 8.0).unwrap();
        let before = project.clip(track, comp).unwrap().clone();
        let originals: Vec<_> = project
            .audio_take_sources(track, group)
            .into_iter()
            .cloned()
            .collect();
        project.ripple_delete_audio_range(track, 6.0, 7.0).unwrap();
        let shifted = project.clip(track, comp).unwrap().clone();
        assert_eq!(
            (
                shifted.start,
                shifted.length,
                shifted.offset,
                shifted.source_rate
            ),
            (7.0, before.length, before.offset, before.source_rate)
        );
        assert_eq!(
            project
                .audio_take_sources(track, group)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
            originals
        );
        let new_group = shifted.take_group().unwrap();
        let candidate = project.audio_take_sources(track, new_group)[1].id;
        let chosen = project
            .select_audio_take(
                track,
                new_group,
                candidate,
                Some((shifted.start, shifted.end())),
                0.0,
            )
            .unwrap();
        let replacement = project.clip(track, chosen).unwrap();
        for local in [0.0, 0.25, 1.499] {
            assert!(
                (replacement.audio_source_frame(local, 120.0, 48000)
                    - before.audio_source_frame(local, 120.0, 48000))
                .abs()
                    < 1e-7
            );
        }
        project.validate().unwrap();
    }
}

#[test]
fn ripple_keeps_latest_start_winner_when_overlaps_collapse_to_same_boundary() {
    let mut project = Project::new();
    let track = project.add_audio_track("Overlaps");
    // Vector order deliberately differs from start order, as after moving clips.
    let newest = Clip::audio("Newest start", 2.0, 6.0, "a.wav");
    let earlier = Clip::audio("Earlier start", 0.0, 10.0, "b.wav");
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([newest, earlier]);
    project.ripple_delete_audio_range(track, 1.0, 3.0).unwrap();
    let clips = &project.track(track).unwrap().clips;
    let schedule = daw_core::AudioSchedule::new(clips).unwrap();
    assert_eq!(
        clips[schedule.sample(2.0).unwrap().incoming].name,
        "Newest start"
    );
    assert_eq!(
        clips[schedule.sample(6.5).unwrap().incoming].name,
        "Earlier start"
    );
}

#[test]
fn ripple_retains_alternates_that_cannot_conform_to_the_current_stretch_rate() {
    let mut project = Project::new();
    let track = project.add_audio_track("Stretch extremes");
    let mut reference = Clip::audio("Slow", 0.0, 16.0, "a.wav");
    reference.source_rate = 0.25;
    let mut alternate = Clip::audio("Fast", 0.0, 1.0, "b.wav");
    alternate.source_rate = 4.0;
    let ids = [reference.id, alternate.id];
    project
        .track_mut(track)
        .unwrap()
        .clips
        .extend([reference, alternate]);
    let group = project.group_audio_takes(track, &ids).unwrap();
    let originals: Vec<_> = project
        .audio_take_sources(track, group)
        .into_iter()
        .cloned()
        .collect();
    let comp = project.audio_take_regions(track, group)[0].id;
    project.stretch_clip(track, comp, 4.0).unwrap();
    project.move_clip(track, comp, 8.0).unwrap();
    project.ripple_delete_audio_range(track, 6.0, 7.0).unwrap();
    assert_eq!(project.clip(track, comp).unwrap().start, 7.0);
    assert_eq!(
        project
            .audio_take_sources(track, group)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        originals
    );
    project.validate().unwrap();
}
