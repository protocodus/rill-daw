use daw_core::{AudioData, DeviceSample, EditBatch, Instrument, Project, SongEdit};

fn source(path: &std::path::Path) {
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..480)
            .map(|frame| (frame as f32 * 0.1).sin() * 0.25)
            .collect(),
    }
    .write_wav(path)
    .unwrap();
}

#[test]
fn project_samples_are_portable_shared_sources_even_without_clips() {
    let temp = tempfile::tempdir().unwrap();
    let wav = temp.path().join("hit.wav");
    source(&wav);
    let mut project = Project::new();
    let id = project
        .add_project_sample("Warm hit", wav.to_string_lossy())
        .unwrap();
    let track = project.add_instrument_track("Drums");
    project
        .set_instrument(track, Instrument::builtin("r1-drums"))
        .unwrap();
    project
        .load_device_sample(
            track,
            DeviceSample {
                slot: 16,
                name: "Warm hit".into(),
                path: wav.to_string_lossy().into_owned(),
            },
        )
        .unwrap();
    assert_eq!(project.audio_paths().count(), 2);
    let saved = temp.path().join("song.rill");
    project.save(&saved).unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&saved).unwrap()).unwrap();
    assert_eq!(archive.len(), 2, "manifest plus one shared recording");
    assert_eq!(
        archive.by_index(1).unwrap().name().unwrap(),
        "assets/00000000.wav"
    );
    std::fs::remove_file(&wav).unwrap();
    let loaded = Project::load_with_assets(&saved, temp.path().join("loaded")).unwrap();
    let sample = loaded.project_sample(id).unwrap();
    assert_eq!(sample.name, "Warm hit");
    assert_eq!(
        sample.path,
        loaded
            .track(track)
            .unwrap()
            .instrument
            .sample(16)
            .unwrap()
            .path
    );
    assert_eq!(
        AudioData::load_wav(&sample.path).unwrap().samples.len(),
        480
    );

    let mut pool_only = loaded.clone();
    pool_only.tracks.clear();
    pool_only
        .save_optimized(temp.path().join("pool.rill"))
        .unwrap();
    let pool = Project::load_with_assets(
        temp.path().join("pool.rill"),
        temp.path().join("pool-loaded"),
    )
    .unwrap();
    assert_eq!(pool.samples.len(), 1);
    assert!(std::path::Path::new(&pool.samples[0].path).is_file());
}

#[test]
fn project_sample_edits_are_atomic_and_do_not_change_loaded_recordings() {
    let mut project = Project::new();
    let sample = project.add_project_sample("First", "/managed.wav").unwrap();
    let track = project.add_instrument_track("Sampler");
    project
        .set_instrument(track, Instrument::builtin("r1-sampler"))
        .unwrap();
    project
        .load_device_sample(
            track,
            DeviceSample {
                slot: 0,
                name: "First".into(),
                path: "/managed.wav".into(),
            },
        )
        .unwrap();
    let before = project.clone();
    let bad = EditBatch {
        label: "Rename samples".into(),
        edits: vec![
            SongEdit::RenameProjectSample {
                sample: sample.into(),
                name: "Second".into(),
            },
            SongEdit::RenameProjectSample {
                sample: sample.into(),
                name: "\n".into(),
            },
        ],
    };
    assert!(bad.apply(&mut project).is_err());
    assert_eq!(project, before);
    project.rename_project_sample(sample, " Second ").unwrap();
    assert_eq!(project.project_sample(sample).unwrap().name, "Second");
    EditBatch {
        label: "Remove sample".into(),
        edits: vec![SongEdit::RemoveProjectSample {
            sample: sample.into(),
        }],
    }
    .apply(&mut project)
    .unwrap();
    assert!(project.samples.is_empty());
    assert_eq!(
        project
            .track(track)
            .unwrap()
            .instrument
            .sample(0)
            .unwrap()
            .path,
        "/managed.wav"
    );
    assert!(project.remove_project_sample(sample).is_err());
}

#[test]
fn project_sample_identity_paths_and_names_are_validated() {
    let mut project = Project::new();
    assert!(project.add_project_sample("", "/sample.wav").is_err());
    assert!(
        project
            .add_project_sample("Line\nbreak", "/sample.wav")
            .is_err()
    );
    assert!(project.add_project_sample("Sample", "").is_err());
    assert!(project.add_project_sample("Sample", "bad\0path").is_err());
    let id = project.add_project_sample("Sample", "/sample.wav").unwrap();
    let track = project.add_audio_track("Audio");
    project.samples[0].id = track;
    assert!(project.validate().is_err());
    project.samples[0].id = id;
    project.validate().unwrap();

    let serialized = serde_json::to_value(Project::new()).unwrap();
    assert!(
        serialized.get("samples").is_none(),
        "legacy empty projects keep their schema"
    );
    let loaded: Project = serde_json::from_value(serialized).unwrap();
    assert!(loaded.samples.is_empty());
}

#[test]
fn pool_sources_are_protected_from_export_and_ids_are_reserved_on_load() {
    let temp = tempfile::tempdir().unwrap();
    let wav = temp.path().join("hit.wav");
    source(&wav);
    let mut project = Project::new();
    project
        .add_project_sample("Hit", wav.to_string_lossy())
        .unwrap();
    assert!(project.validate_audio_destination(&wav).is_err());
    assert!(project.save(&wav).is_err());
    project.samples[0].id = 9_000_000;
    let path = temp.path().join("song.json");
    project.save_legacy_json(&path).unwrap();
    let mut loaded = Project::load(&path).unwrap();
    assert!(
        loaded
            .add_project_sample("Next", wav.to_string_lossy())
            .unwrap()
            > 9_000_000
    );
}
