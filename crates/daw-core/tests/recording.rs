use daw_core::{
    AudioData, AudioInputChannels, AudioSchedule, AudioTakeMembership, AudioTakeRole, Clip,
    ClipContent, History, Project, RecordedAudioPass, RecordingPlacement, TrackKind,
};

fn pass(name: &str, start: f64, length: f64, complete: bool) -> RecordedAudioPass {
    RecordedAudioPass {
        clip: Clip::audio(name, start, length, format!("{name}.wav")),
        complete,
    }
}

fn audible_path(project: &Project, track: u64, beat: f64) -> Option<&str> {
    let clips = &project.track(track).unwrap().clips;
    let index = AudioSchedule::new(clips).unwrap().sample(beat)?.incoming;
    match &clips[index].content {
        ClipContent::Audio { path, .. } => Some(path),
        _ => unreachable!(),
    }
}

#[test]
fn input_channels_are_validated_against_real_devices_and_round_trip_without_arming() {
    let mono = AudioInputChannels::Mono { channel: 3 };
    let stereo = AudioInputChannels::Stereo { left: 1, right: 3 };
    assert_eq!(mono.channel_count(), 1);
    assert_eq!(stereo.channel_count(), 2);
    assert!(mono.validate(4).is_ok());
    assert!(mono.validate(3).is_err());
    assert!(stereo.validate(4).is_ok());
    assert!(stereo.validate(3).is_err());
    assert!(
        AudioInputChannels::Stereo { left: 2, right: 2 }
            .validate(4)
            .is_err()
    );
    assert!(AudioInputChannels::default().validate(0).is_err());
    let mut project = Project::new();
    let track = project.add_audio_track("Vocal");
    assert_eq!(
        project.track(track).unwrap().audio_input,
        Some(AudioInputChannels::default())
    );
    project.set_audio_input(track, Some(stereo)).unwrap();
    let value = serde_json::to_value(&project).unwrap();
    let restored: Project = serde_json::from_value(value.clone()).unwrap();
    restored.validate().unwrap();
    assert_eq!(project, restored);
    for field in ["armed", "record_arm", "monitor", "recording"] {
        assert!(
            value["tracks"][0].get(field).is_none(),
            "{field} must remain session-local"
        );
    }
    let mut legacy = value;
    legacy["tracks"][0]
        .as_object_mut()
        .unwrap()
        .remove("audio_input");
    let restored: Project = serde_json::from_value(legacy).unwrap();
    restored.validate().unwrap();
    assert!(restored.track(track).unwrap().audio_input.is_none());
    project.set_audio_input(track, None).unwrap();
    let midi = project.add_midi_track("MIDI");
    let before = project.clone();
    assert!(project.set_audio_input(midi, Some(mono)).is_err());
    assert_eq!(project, before);
    assert!(
        project
            .set_audio_input(track, Some(AudioInputChannels::Mono { channel: u16::MAX }))
            .is_err()
    );
    assert_eq!(project, before);
}

#[test]
fn nonloop_recording_is_an_independent_clip_with_exact_timing_and_preserves_existing_audio() {
    let mut project = Project::new();
    let track = project.add_audio_track("Vocal");
    let guide = Clip::audio("Guide", 0.0, 16.0, "guide.wav");
    project.track_mut(track).unwrap().clips.push(guide.clone());
    let recorded = pass("Capture", 4.25, 2.0 + 1.0 / 24000.0, false);
    let source = recorded.clip.clone();
    let result = project
        .insert_recorded_audio(track, vec![recorded], RecordingPlacement::Take, false)
        .unwrap();
    assert_eq!(result.track, track);
    assert_eq!(result.clip, source.id);
    assert_eq!(result.group, None);
    assert_eq!(project.clip(track, guide.id), Some(&guide));
    assert_eq!(project.clip(track, source.id), Some(&source));
    assert_eq!(audible_path(&project, track, 4.0), Some("guide.wav"));
    assert_eq!(audible_path(&project, track, 5.0), Some("Capture.wav"));
    assert_eq!(audible_path(&project, track, 7.0), Some("guide.wav"));
    let clip = project.clip(track, source.id).unwrap();
    assert_eq!(clip.audio_source_frame(1.0 / 24000.0, 120.0, 48000), 1.0);
    assert_eq!(clip.audio_source_frame(2.0, 120.0, 48000), 48000.0);
    assert!(clip.audio_take.is_none() && clip.link_group.is_none());
}

#[test]
fn loop_passes_keep_partial_alternatives_and_choose_the_latest_complete_recording() {
    let mut project = Project::new();
    let track = project.add_audio_track("Loop vocal");
    let passes = vec![
        pass("Started midway", 5.0, 3.0, false),
        pass("Full A", 4.0, 4.0, true),
        pass("Full B", 4.0, 4.0, true),
        pass("Stopped midway", 4.0, 0.5, false),
    ];
    let originals: Vec<_> = passes.iter().map(|pass| pass.clip.clone()).collect();
    let result = project
        .insert_recorded_audio(track, passes, RecordingPlacement::Take, true)
        .unwrap();
    let group = result.group.unwrap();
    let sources = project.audio_take_sources(track, group);
    assert_eq!(sources.len(), 4);
    assert_eq!(project.audio_take_regions(track, group).len(), 1);
    for original in &originals {
        let source = project.clip(track, original.id).unwrap();
        assert!(source.is_take_source());
        assert_eq!(
            (&source.content, source.start, source.length),
            (&original.content, original.start, original.length)
        );
    }
    assert_eq!(
        project.clip(track, result.clip).unwrap().audio_take,
        Some(AudioTakeMembership {
            group,
            role: AudioTakeRole::Comp {
                source: originals[2].id
            },
        })
    );
    for beat in [4.0, 4.25, 5.0, 7.999] {
        assert_eq!(audible_path(&project, track, beat), Some("Full B.wav"));
    }
    assert_eq!(project.end_beat(), 8.0);
    // The stopped fragment can still be deliberately comped into the beginning.
    project
        .select_audio_take(track, group, originals[3].id, Some((4.0, 4.5)), 0.02)
        .unwrap();
    assert_eq!(
        audible_path(&project, track, 4.25),
        Some("Stopped midway.wav")
    );
    assert_eq!(audible_path(&project, track, 7.0), Some("Full B.wav"));
}

#[test]
fn incomplete_only_loop_uses_longest_coverage_and_latest_tie_without_losing_short_pass() {
    let mut project = Project::new();
    let track = project.add_audio_track("Loop");
    let result = project
        .insert_recorded_audio(
            track,
            vec![
                pass("Earlier long", 0.0, 3.0, false),
                pass("Latest long", 0.0, 3.0, false),
                pass("Short final", 0.0, 0.25, false),
            ],
            RecordingPlacement::Take,
            true,
        )
        .unwrap();
    assert_eq!(
        project
            .audio_take_sources(track, result.group.unwrap())
            .len(),
        3
    );
    assert_eq!(audible_path(&project, track, 2.5), Some("Latest long.wav"));
    assert_eq!(project.end_beat(), 3.0);
    let mut only = Project::new();
    let track = only.add_audio_track("Only partial");
    let result = only
        .insert_recorded_audio(
            track,
            vec![pass("Fragment", 1.25, 0.75, false)],
            RecordingPlacement::Take,
            true,
        )
        .unwrap();
    assert_eq!(
        only.audio_take_sources(track, result.group.unwrap()).len(),
        1
    );
    assert_eq!(audible_path(&only, track, 1.5), Some("Fragment.wav"));
}

#[test]
fn overdub_layer_keeps_original_arrangement_routes_and_folder_and_starts_neutral() {
    let mut project = Project::new();
    let bus = project.add_bus("Vocals bus");
    let folder = project.add_folder("Vocals");
    let track = project.add_audio_track("Lead");
    let owner = project.track_mut(track).unwrap();
    owner.output_bus = Some(bus);
    owner.parent_folder = Some(folder);
    owner.gain = 0.42;
    owner.pan = -0.3;
    owner.audio_input = Some(AudioInputChannels::Stereo { left: 2, right: 3 });
    owner
        .clips
        .push(Clip::audio("Lead comp", 0.0, 8.0, "lead.wav"));
    let before = project.track(track).unwrap().clone();
    let mut history = History::new();
    history.commit_named(&project, "Record overdub");
    let result = project
        .insert_recorded_audio(
            track,
            vec![pass("Harmony", 0.0, 8.0, false)],
            RecordingPlacement::Overdub,
            false,
        )
        .unwrap();
    let layer = project.track(result.track).unwrap();
    assert_ne!(result.track, track);
    assert_eq!(layer.kind, TrackKind::Audio);
    assert_eq!(layer.output_bus, before.output_bus);
    assert_eq!(layer.parent_folder, before.parent_folder);
    assert_eq!(layer.audio_input, before.audio_input);
    assert_eq!(layer.gain, before.gain);
    assert_eq!(layer.pan, before.pan);
    assert!(layer.inserts.is_empty() && layer.sends.is_empty());
    assert_eq!(layer.color, [148, 153, 162]);
    assert_eq!(project.track(track), Some(&before));
    assert_eq!(audible_path(&project, track, 2.0), Some("lead.wav"));
    assert_eq!(
        audible_path(&project, result.track, 2.0),
        Some("Harmony.wav")
    );
    let after = project.clone();
    assert!(history.undo(&mut project));
    assert!(project.track(result.track).is_none());
    assert_eq!(project.track(track), Some(&before));
    assert!(history.redo(&mut project));
    assert_eq!(project, after);
}

#[test]
fn repeated_recordings_preserve_old_comp_edits_as_a_separate_recoverable_group() {
    let mut project = Project::new();
    let track = project.add_audio_track("Vocal");
    let first = project
        .insert_recorded_audio(
            track,
            vec![pass("Session A", 0.0, 4.0, true)],
            RecordingPlacement::Take,
            true,
        )
        .unwrap();
    project.split_clip(track, first.clip, 2.0).unwrap();
    project.set_audio_gain(track, first.clip, 0.5).unwrap();
    let original_clips = project.track(track).unwrap().clips.clone();
    let second = project
        .insert_recorded_audio(
            track,
            vec![pass("Session B", 0.0, 4.0, true)],
            RecordingPlacement::Take,
            true,
        )
        .unwrap();
    assert_ne!(first.group, second.group);
    for original in original_clips {
        assert_eq!(project.clip(track, original.id), Some(&original));
    }
    assert_eq!(
        project
            .audio_take_sources(track, first.group.unwrap())
            .len(),
        1
    );
    assert_eq!(
        project
            .audio_take_regions(track, first.group.unwrap())
            .len(),
        2
    );
    assert_eq!(
        project
            .audio_take_sources(track, second.group.unwrap())
            .len(),
        1
    );
    project.validate().unwrap();
}

#[test]
fn invalid_recording_batches_roll_back_the_entire_transaction_including_new_layers() {
    let mut project = Project::new();
    let track = project.add_audio_track("Lead");
    let midi = project.add_midi_track("MIDI");
    let before = project.clone();
    let malformed = [
        vec![],
        vec![pass("Negative", -1.0, 1.0, false)],
        vec![pass("Empty", 0.0, 0.0, false)],
        vec![pass("Infinite", 0.0, f64::INFINITY, false)],
        vec![RecordedAudioPass {
            clip: Clip::midi("Wrong kind", 0.0, 1.0, vec![]),
            complete: true,
        }],
        (0..65)
            .map(|i| pass(&format!("Take {i}"), 0.0, 1.0, true))
            .collect(),
    ];
    for passes in malformed {
        assert!(
            project
                .insert_recorded_audio(track, passes, RecordingPlacement::Overdub, true)
                .is_err()
        );
        assert_eq!(project, before);
    }
    let capture = pass("Duplicate ID", 0.0, 1.0, true);
    assert!(
        project
            .insert_recorded_audio(
                track,
                vec![capture.clone(), capture.clone()],
                RecordingPlacement::Take,
                true
            )
            .is_err()
    );
    assert!(
        project
            .insert_recorded_audio(midi, vec![capture.clone()], RecordingPlacement::Take, false)
            .is_err()
    );
    assert!(
        project
            .insert_recorded_audio(
                u64::MAX,
                vec![capture.clone()],
                RecordingPlacement::Take,
                false
            )
            .is_err()
    );
    assert!(
        project
            .insert_recorded_audio(
                track,
                vec![capture, pass("Other", 0.0, 1.0, true)],
                RecordingPlacement::Take,
                false
            )
            .is_err()
    );
    assert_eq!(project, before);
}

#[test]
fn recorded_complete_and_partial_assets_survive_portable_save_and_undo_without_file_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Portable recording");
    project
        .set_audio_input(
            track,
            Some(AudioInputChannels::Stereo { left: 0, right: 1 }),
        )
        .unwrap();
    let mut passes = Vec::new();
    let mut paths = Vec::new();
    for (name, frames, value, complete) in [
        ("Complete", 16000, 0.25, true),
        ("Partial", 2000, 0.5, false),
    ] {
        let path = directory.path().join(format!("{name}.wav"));
        AudioData {
            sample_rate: 8000,
            channels: 2,
            samples: (0..frames).flat_map(|_| [value, -value]).collect(),
        }
        .write_wav(&path)
        .unwrap();
        passes.push(RecordedAudioPass {
            clip: Clip::audio(name, 4.0, frames as f64 / 4000.0, path.to_string_lossy()),
            complete,
        });
        paths.push(path);
    }
    let mut history = History::new();
    history.commit_named(&project, "Record loop takes");
    let result = project
        .insert_recorded_audio(track, passes, RecordingPlacement::Take, true)
        .unwrap();
    assert!(history.undo(&mut project));
    assert!(paths.iter().all(|path| path.exists()));
    assert!(history.redo(&mut project));
    let bundle = directory.path().join("recording.rill");
    project.save(&bundle).unwrap();
    for path in &paths {
        std::fs::remove_file(path).unwrap();
    }
    let restored = Project::load_with_assets(&bundle, directory.path().join("restored")).unwrap();
    restored.validate().unwrap();
    assert_eq!(
        restored.track(track).unwrap().audio_input,
        project.track(track).unwrap().audio_input
    );
    let sources = restored.audio_take_sources(track, result.group.unwrap());
    assert_eq!(sources.len(), 2);
    for source in sources {
        let ClipContent::Audio { path, .. } = &source.content else {
            unreachable!()
        };
        let audio = AudioData::load_wav(path).unwrap();
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.sample_rate, 8000);
        let (frames, value) = if source.name == "Complete" {
            (16000, 0.25)
        } else {
            (2000, 0.5)
        };
        assert_eq!(audio.frames(), frames);
        assert_eq!(audio.samples[0], value);
        assert_eq!(audio.samples[1], -value);
    }
    assert_eq!(restored.end_beat(), 8.0);
}
