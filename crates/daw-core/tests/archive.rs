use daw_core::{
    AudioData, Clip, ClipContent, Effect, EffectSlot, Instrument, PluginParameterValue, Project,
};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

fn manifest(project: &Project) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"version":1,"project":project})).unwrap()
}

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn audio_project(path: impl AsRef<Path>) -> Project {
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Source",
        0.0,
        1.0,
        path.as_ref().to_string_lossy(),
    ));
    project
}

#[test]
fn portable_bundle_preserves_audio_and_presets_after_originals_are_removed() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    fs::create_dir_all(original.join("a")).unwrap();
    fs::create_dir_all(original.join("b")).unwrap();
    let first = original.join("a/same.wav");
    let second = original.join("b/same.wav");
    for (path, value) in [(&first, 0.25), (&second, -0.5)] {
        AudioData {
            sample_rate: 8_000,
            channels: 1,
            samples: vec![value; 4_000],
        }
        .write_wav(path)
        .unwrap();
    }
    let mut project = audio_project(&first);
    project.tracks[0].clips.push(Clip::audio(
        "Duplicate source",
        1.0,
        1.0,
        first.to_string_lossy(),
    ));
    project.tracks[0].clips.push(Clip::audio(
        "Distinct same name",
        2.0,
        1.0,
        second.to_string_lossy(),
    ));
    let mut effect = EffectSlot::new(
        "Effect",
        Effect::Vst3 {
            path: "relative-effect.vst3".into(),
            class_id: None,
        },
    );
    effect.state = vec![0, 255, 17];
    effect.parameters = vec![PluginParameterValue { id: 9, value: 0.2 }];
    project.tracks[0].inserts.push(effect);
    let midi = project.add_midi_track("Instrument");
    let instrument = project.track_mut(midi).unwrap();
    instrument.instrument = Instrument::Vst3 {
        path: "relative-instrument.vst3".into(),
        class_id: None,
    };
    instrument.instrument_state = vec![1, 2, 3];
    let saved = temp.path().join("song.rill");
    project.save(&saved).unwrap();
    let zip = ZipArchive::new(fs::File::open(&saved).unwrap()).unwrap();
    assert_eq!(
        zip.len(),
        3,
        "Duplicate clip sources must share a bundled asset"
    );
    drop(zip);
    fs::remove_dir_all(original).unwrap();
    let moved = temp.path().join("moved.rill");
    fs::rename(saved, &moved).unwrap();
    let scratch = temp.path().join("scratch");
    let restored = Project::load_with_assets(&moved, &scratch).unwrap();
    let paths: Vec<_> = restored.tracks[0]
        .clips
        .iter()
        .map(|clip| {
            let ClipContent::Audio { path, .. } = &clip.content else {
                unreachable!()
            };
            assert!(Path::new(path).starts_with(fs::canonicalize(&scratch).unwrap()));
            path.clone()
        })
        .collect();
    assert_eq!(paths[0], paths[1]);
    assert_ne!(paths[0], paths[2]);
    assert_eq!(
        AudioData::load_wav(&paths[0]).unwrap().samples,
        vec![0.25; 4_000]
    );
    assert_eq!(
        AudioData::load_wav(&paths[2]).unwrap().samples,
        vec![-0.5; 4_000]
    );
    assert_eq!(restored.tracks[0].inserts[0].state, vec![0, 255, 17]);
    assert_eq!(
        restored.track(midi).unwrap().instrument_state,
        vec![1, 2, 3]
    );
    let Effect::Vst3 { path, .. } = &restored.tracks[0].inserts[0].effect;
    assert!(Path::new(path).is_absolute());
    assert!(!Path::new(path).starts_with(&scratch));
    let resaved = temp.path().join("again.rill");
    restored.save(&resaved).unwrap();
    let reopened = Project::load_with_assets(&resaved, &scratch).unwrap();
    let ClipContent::Audio { path, .. } = &reopened.tracks[0].clips[0].content else {
        unreachable!()
    };
    assert_eq!(
        AudioData::load_wav(path).unwrap().samples,
        vec![0.25; 4_000]
    );
}

#[test]
fn missing_media_preserves_existing_bundle_and_cleans_temporary_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("saved.rill");
    Project::new().save(&path).unwrap();
    let original = fs::read(&path).unwrap();
    assert!(
        audio_project(temp.path().join("missing.wav"))
            .save(&path)
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    let mut replacement = Project::new();
    replacement.name = "Replacement".into();
    replacement.save(&path).unwrap();
    assert_eq!(
        Project::load_with_assets(&path, temp.path().join("scratch"))
            .unwrap()
            .name,
        "Replacement"
    );
}

#[test]
fn rejects_traversal_absolute_and_unbundled_manifest_media_before_extraction() {
    let temp = tempfile::tempdir().unwrap();
    let saved = temp.path().join("unsafe.rill");
    let scratch = temp.path().join("scratch");
    for name in [
        "../escape.wav",
        "/absolute.wav",
        "assets/../escape.wav",
        "assets\\escape.wav",
        "C:/escape.wav",
    ] {
        let data = manifest(&Project::new());
        fs::write(
            &saved,
            archive(&[("manifest.json", &data), (name, b"audio")]),
        )
        .unwrap();
        assert!(
            Project::load_with_assets(&saved, &scratch).is_err(),
            "{name}"
        );
        assert!(!scratch.exists());
    }
    for reference in [
        "../escape.wav",
        "/absolute.wav",
        "C:\\external.wav",
        "assets/00000000.wav",
    ] {
        let data = manifest(&audio_project(reference));
        fs::write(&saved, archive(&[("manifest.json", &data)])).unwrap();
        assert!(
            Project::load_with_assets(&saved, &scratch).is_err(),
            "{reference}"
        );
        assert!(!scratch.exists());
    }
}

#[test]
fn rejects_symlinks_duplicate_headers_and_oversized_declarations() {
    let temp = tempfile::tempdir().unwrap();
    let saved = temp.path().join("unsafe.rill");
    let scratch = temp.path().join("scratch");
    let data = manifest(&audio_project("assets/00000000.wav"));
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&data).unwrap();
    writer
        .add_symlink(
            "assets/00000000.wav",
            "../../outside.wav",
            SimpleFileOptions::default(),
        )
        .unwrap();
    fs::write(&saved, writer.finish().unwrap().into_inner()).unwrap();
    assert!(Project::load_with_assets(&saved, &scratch).is_err());
    assert!(!scratch.exists());

    let mut duplicate = archive(&[
        ("manifest.json", &data),
        ("assets/00000000.wav", b"a"),
        ("assets/00000001.wav", b"b"),
    ]);
    for offset in 0..duplicate.len().saturating_sub(18) {
        if &duplicate[offset..offset + 19] == b"assets/00000001.wav" {
            duplicate[offset + 14] = b'0';
        }
    }
    fs::write(&saved, duplicate).unwrap();
    assert!(Project::load_with_assets(&saved, &scratch).is_err());

    let mut oversized = archive(&[("manifest.json", &data), ("assets/00000000.wav", b"a")]);
    // Matching local/central declarations test the application size bound,
    // independent of the ZIP library's consistency check.
    let central = oversized
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    let local = oversized
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x03\x04")
        .unwrap();
    oversized[central + 24..central + 28].copy_from_slice(&(2_147_483_649u32).to_le_bytes());
    oversized[local + 22..local + 26].copy_from_slice(&(2_147_483_649u32).to_le_bytes());
    fs::write(&saved, oversized).unwrap();
    assert!(Project::load_with_assets(&saved, &scratch).is_err());
    assert!(!scratch.exists());
}

#[test]
fn corrupt_asset_removes_partial_extraction_and_does_not_touch_existing_scratch() {
    let temp = tempfile::tempdir().unwrap();
    let scratch = temp.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    fs::write(scratch.join("keep.txt"), b"user data").unwrap();
    let data = manifest(&audio_project("assets/00000000.wav"));
    let mut bytes = archive(&[
        ("manifest.json", &data),
        ("assets/00000000.wav", b"original audio"),
    ]);
    let payload = bytes
        .windows(14)
        .position(|value| value == b"original audio")
        .unwrap();
    bytes[payload] ^= 0xff;
    let saved = temp.path().join("corrupt.rill");
    fs::write(&saved, bytes).unwrap();
    assert!(Project::load_with_assets(&saved, &scratch).is_err());
    assert_eq!(fs::read(scratch.join("keep.txt")).unwrap(), b"user data");
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 1);
}

#[test]
fn rejects_decompression_beyond_declared_size_and_excess_entry_count() {
    let temp = tempfile::tempdir().unwrap();
    let saved = temp.path().join("bomb.rill");
    let scratch = temp.path().join("scratch");
    let data = manifest(&audio_project("assets/00000000.wav"));
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&data).unwrap();
    writer
        .start_file(
            "assets/00000000.wav",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
    writer.write_all(&vec![0; 1024 * 1024]).unwrap();
    let mut bytes = writer.finish().unwrap().into_inner();
    let central = bytes
        .windows(4)
        .rposition(|value| value == b"PK\x01\x02")
        .unwrap();
    let local = bytes
        .windows(4)
        .rposition(|value| value == b"PK\x03\x04")
        .unwrap();
    bytes[central + 24..central + 28].copy_from_slice(&32u32.to_le_bytes());
    bytes[local + 22..local + 26].copy_from_slice(&32u32.to_le_bytes());
    fs::write(&saved, bytes).unwrap();
    assert!(Project::load_with_assets(&saved, &scratch).is_err());
    assert!(!scratch.exists() || fs::read_dir(&scratch).unwrap().count() == 0);

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&data).unwrap();
    for index in 0..4096 {
        writer
            .start_file(
                format!("assets/{index:08}.wav"),
                SimpleFileOptions::default(),
            )
            .unwrap();
    }
    fs::write(&saved, writer.finish().unwrap().into_inner()).unwrap();
    assert!(Project::load_with_assets(&saved, &scratch).is_err());
    assert!(!scratch.exists() || fs::read_dir(&scratch).unwrap().count() == 0);
}
