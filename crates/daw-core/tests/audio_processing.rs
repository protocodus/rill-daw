use daw_core::{
    AudioData, AudioProcessing, AudioProcessingAlgorithm, Clip, ClipContent, Note, Project,
};

fn audio_project(path: &str) -> (Project, u64, u64) {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    let clip = Clip::audio("Source", 0.0, 4.0, path);
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    (project, track, id)
}

fn processing(project: &Project, track: u64, clip: u64) -> AudioProcessing {
    let ClipContent::Audio { processing, .. } = project.clip(track, clip).unwrap().content else {
        panic!("expected audio clip")
    };
    processing
}

#[test]
fn transposition_and_engine_are_shared_by_ghosts_without_changing_instance_timing() {
    let (mut project, track, original) = audio_project("source.wav");
    let ghost = project.clone_clip(track, original).unwrap();
    project.stretch_clip(track, ghost, 8.0).unwrap();
    project.trim_clip(track, ghost, 5.0, 6.0).unwrap();
    let duplicate = project.duplicate_clip(track, ghost).unwrap();
    let before = project.clone();
    for algorithm in AudioProcessingAlgorithm::ALL {
        let value = AudioProcessing {
            algorithm,
            transpose_semitones: -7.25,
        };
        project.set_audio_processing(track, ghost, value).unwrap();
        for id in [original, ghost] {
            assert_eq!(processing(&project, track, id), value);
            let actual = project.clip(track, id).unwrap();
            let prior = before.clip(track, id).unwrap();
            assert_eq!(
                (
                    actual.start,
                    actual.length,
                    actual.offset,
                    actual.source_rate
                ),
                (prior.start, prior.length, prior.offset, prior.source_rate)
            );
        }
        assert_eq!(
            processing(&project, track, duplicate),
            AudioProcessing::default(),
            "an independent duplicate cannot follow ghost source edits"
        );
        project.validate().unwrap();
    }
    project.make_clip_independent(track, ghost).unwrap();
    project
        .set_audio_processing(track, ghost, AudioProcessing::default())
        .unwrap();
    assert_eq!(
        processing(&project, track, original).transpose_semitones,
        -7.25
    );
    assert_eq!(processing(&project, track, ghost).transpose_semitones, 0.0);
}

#[test]
fn invalid_pitch_and_non_audio_targets_are_rejected_without_mutation() {
    let (mut project, track, clip) = audio_project("source.wav");
    project.clone_clip(track, clip).unwrap();
    let midi = project.add_midi_track("MIDI");
    let notes = Clip::midi("Notes", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 100)]);
    let midi_clip = notes.id;
    project.track_mut(midi).unwrap().clips.push(notes);
    let before = project.clone();
    for pitch in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -24.01, 24.01] {
        let value = AudioProcessing {
            algorithm: AudioProcessingAlgorithm::Bungee,
            transpose_semitones: pitch,
        };
        assert!(project.set_audio_processing(track, clip, value).is_err());
        assert_eq!(project, before);
        let mut corrupt = before.clone();
        if let ClipContent::Audio { processing, .. } =
            &mut corrupt.clip_mut(track, clip).unwrap().content
        {
            *processing = value;
        }
        assert!(corrupt.validate().is_err());
    }
    for (track, clip) in [(midi, midi_clip), (track, midi_clip), (u64::MAX, clip)] {
        assert!(
            project
                .set_audio_processing(track, clip, AudioProcessing::default())
                .is_err()
        );
        assert_eq!(project, before);
    }
    for pitch in [-24.0, -0.01, 0.0, 0.01, 24.0] {
        project
            .set_audio_processing(
                track,
                clip,
                AudioProcessing {
                    transpose_semitones: pitch,
                    ..Default::default()
                },
            )
            .unwrap();
        project.validate().unwrap();
    }
}

#[test]
fn portable_project_keeps_original_audio_and_recalls_processing_and_local_stretch() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: (0..16_000).map(|i| (i as f32 * 0.1).sin() * 0.25).collect(),
    }
    .write_wav(&source)
    .unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let (mut project, track, original) = audio_project(&source.to_string_lossy());
    let ghost = project.clone_clip(track, original).unwrap();
    project.stretch_clip(track, ghost, 8.0).unwrap();
    let value = AudioProcessing {
        algorithm: AudioProcessingAlgorithm::Bungee,
        transpose_semitones: 12.37,
    };
    project.set_audio_processing(track, ghost, value).unwrap();
    let json = temp.path().join("processing.json");
    project.save(&json).unwrap();
    assert_eq!(Project::load(&json).unwrap(), project);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(json).unwrap()).unwrap();
    assert_eq!(json["version"], 3);
    let archive = temp.path().join("processing.rill");
    project.save(&archive).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    std::fs::remove_file(source).unwrap();
    let restored = Project::load_with_assets(&archive, temp.path().join("scratch")).unwrap();
    for (id, rate) in [(original, 1.0), (ghost, 0.5)] {
        assert_eq!(processing(&restored, track, id), value);
        let clip = restored.clip(track, id).unwrap();
        assert_eq!(clip.source_rate, rate);
        let ClipContent::Audio { path, .. } = &clip.content else {
            panic!()
        };
        assert_eq!(std::fs::read(path).unwrap(), original_bytes);
    }
    restored.validate().unwrap();
}

#[test]
fn legacy_projects_default_to_original_pitch_and_unknown_algorithms_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let (project, track, clip) = audio_project("source.wav");
    for version in [1, 2] {
        let mut data = serde_json::json!({"version":version,"project":project});
        data["project"]["tracks"][0]["clips"][0]["content"]["Audio"]
            .as_object_mut()
            .unwrap()
            .remove("processing");
        let path = temp.path().join(format!("legacy-{version}.json"));
        std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        let restored = Project::load(&path).unwrap();
        assert_eq!(
            processing(&restored, track, clip),
            AudioProcessing::default()
        );
    }
    let mut data = serde_json::json!({"version":3,"project":project});
    data["project"]["tracks"][0]["clips"][0]["content"]["Audio"]["processing"]["algorithm"] =
        "UnknownEngine".into();
    let path = temp.path().join("unknown.json");
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(Project::load(path).is_err());
}
