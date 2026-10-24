use daw_core::{AudioData, Clip, ClipContent, History, Note, Project, Scale, new_id};
use tempfile::tempdir;

fn fixture() -> (Project, u64, u64) {
    let mut project = Project::new();
    let track_id = project.add_midi_track("Piano");
    let clip = Clip::midi(
        "Phrase",
        4.0,
        8.0,
        vec![Note::new(60, 0.0, 7.0, 96), Note::new(64, 5.0, 1.0, 80)],
    );
    let clip_id = clip.id;
    project.track_mut(track_id).unwrap().clips.push(clip);
    (project, track_id, clip_id)
}

#[test]
fn split_and_trim_preserve_source_notes_and_offsets() {
    let (mut project, track, clip) = fixture();
    let original = project.clip(track, clip).unwrap().clone();
    let right = project.split_clip(track, clip, 7.0).unwrap();
    assert_eq!(project.clip(track, clip).unwrap().length, 3.0);
    let right_clip = project.clip(track, right).unwrap();
    assert_eq!(
        (right_clip.start, right_clip.offset, right_clip.length),
        (7.0, 3.0, 5.0)
    );
    if let (ClipContent::Midi { notes: left }, ClipContent::Midi { notes: right }) =
        (&original.content, &right_clip.content)
    {
        assert_eq!(left[0].start, right[0].start);
        assert_eq!(left[0].length, right[0].length);
        assert_ne!(left[0].id, right[0].id);
    } else {
        panic!("expected MIDI");
    }
    project.trim_clip(track, right, 8.0, 4.0).unwrap();
    assert_eq!(project.clip(track, right).unwrap().offset, 4.0);
    project.trim_clip(track, right, 6.0, 6.0).unwrap();
    assert_eq!(project.clip(track, right).unwrap().offset, 2.0);
    let before = project.clone();
    assert!(project.trim_clip(track, right, 1.0, 11.0).is_err());
    assert_eq!(
        before, project,
        "invalid trim must not partially mutate the model"
    );
    project.validate().unwrap();
}

#[test]
fn duplicate_move_delete_and_undo_are_reversible() {
    let (mut project, track, clip) = fixture();
    let initial = project.clone();
    let mut history = History::new();
    history.commit(&project);
    let duplicate = project.duplicate_clip(track, clip).unwrap();
    assert_eq!(project.clip(track, duplicate).unwrap().start, 12.0);
    project.move_clip(track, duplicate, 20.0).unwrap();
    project.delete_clip(track, clip).unwrap();
    project.validate().unwrap();
    let edited = project.clone();
    assert!(history.undo(&mut project));
    assert_eq!(project, initial);
    assert!(history.redo(&mut project));
    assert_eq!(project, edited);
    assert!(history.undo(&mut project));
    history.commit(&project);
    project.tempo = 100.0;
    assert!(!history.can_redo(), "a new edit invalidates the old future");
}

#[test]
fn note_cut_duplicate_and_glue_preserve_pitch_and_timing() {
    let (mut project, track, clip) = fixture();
    let original_id = project.midi_notes(track, clip).unwrap()[0].id;
    let right_id = project.split_note(track, clip, original_id, 3.0).unwrap();
    let notes = project.midi_notes(track, clip).unwrap();
    assert_eq!((notes[0].start, notes[0].length), (0.0, 3.0));
    assert_eq!((notes[1].start, notes[1].length), (3.0, 4.0));
    let glued_id = project
        .glue_notes(track, clip, &[original_id, right_id])
        .unwrap();
    let glued = project
        .midi_notes(track, clip)
        .unwrap()
        .iter()
        .find(|note| note.id == glued_id)
        .unwrap();
    assert_eq!(
        (glued.pitch, glued.start, glued.length, glued.velocity),
        (60, 0.0, 7.0, 96)
    );
    let duplicate_id = project.duplicate_note(track, clip, glued_id).unwrap();
    let duplicate = project
        .midi_notes(track, clip)
        .unwrap()
        .iter()
        .find(|note| note.id == duplicate_id)
        .unwrap();
    assert_eq!((duplicate.start, duplicate.length), (7.0, 7.0));
    let different_pitch = project
        .midi_notes(track, clip)
        .unwrap()
        .iter()
        .find(|note| note.pitch == 64)
        .unwrap()
        .id;
    let before = project.clone();
    assert!(
        project
            .glue_notes(track, clip, &[glued_id, different_pitch])
            .is_err()
    );
    assert_eq!(before, project);
    assert!(project.split_note(track, clip, glued_id, 0.0).is_err());
    project.delete_note(track, clip, duplicate_id).unwrap();
    project.validate().unwrap();
}

#[test]
fn glue_overlapping_notes_spans_the_union_without_adding_durations() {
    let (mut project, track, clip) = fixture();
    let notes = project.midi_notes_mut(track, clip).unwrap();
    notes.clear();
    notes.push(Note::new(60, 2.0, 4.0, 71));
    notes.push(Note::new(60, 3.0, 4.0, 110));
    notes.push(Note::new(60, 4.0, 1.0, 90));
    let ids: Vec<_> = notes.iter().map(|note| note.id).collect();
    project.glue_notes(track, clip, &ids).unwrap();
    let notes = project.midi_notes(track, clip).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (notes[0].start, notes[0].length, notes[0].velocity),
        (2.0, 5.0, 71)
    );
}

#[test]
fn excessive_timeline_values_are_rejected_before_sample_conversion() {
    let (mut project, track, clip) = fixture();
    let before = project.clone();
    assert!(project.move_clip(track, clip, daw_core::MAX_BEAT).is_err());
    assert_eq!(project, before);
    project.clip_mut(track, clip).unwrap().start = 1e30;
    assert!(project.validate().is_err());
    project = before;
    project.loop_end = f64::MAX;
    assert!(project.validate().is_err());
}

#[test]
fn generated_demo_audio_is_real_deterministic_and_matches_clip_length() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("media/demo.wav");
    let project = Project::demo_with_audio(&path).unwrap();
    let audio = AudioData::load_wav(&path).unwrap();
    project.validate().unwrap();
    assert_eq!(audio.sample_rate, 48_000);
    assert_eq!(audio.channels, 2);
    let audio_clip = &project.tracks.last().unwrap().clips[0];
    assert!(
        (audio.duration_seconds() - audio_clip.length * 60.0 / project.tempo).abs()
            <= 1.0 / 48_000.0
    );
    assert!(audio.samples.iter().any(|sample| sample.abs() > 0.8));
    assert!(audio.samples.iter().all(|sample| sample.abs() <= 0.851));
    Project::demo_with_audio(&path).unwrap();
    assert_eq!(audio.samples, AudioData::load_wav(&path).unwrap().samples);
}

#[test]
fn project_roundtrip_and_future_ids_are_safe() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.daw");
    let project = Project::demo();
    project.validate().unwrap();
    project.save(&path).unwrap();
    let loaded = Project::load(&path).unwrap();
    assert_eq!(loaded, project);
    let maximum = loaded
        .tracks
        .iter()
        .flat_map(|track| {
            std::iter::once(track.id).chain(track.clips.iter().flat_map(|clip| {
                let note_ids: Vec<_> = match &clip.content {
                    ClipContent::Midi { notes } => notes.iter().map(|note| note.id).collect(),
                    _ => Vec::new(),
                };
                std::iter::once(clip.id).chain(note_ids)
            }))
        })
        .max()
        .unwrap();
    assert!(new_id() > maximum);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "atomic save cleans up its temporary file"
    );
}

#[test]
fn relative_media_paths_resolve_against_project_directory() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.daw");
    let mut project = Project::new();
    let track = project.add_audio_track("Audio");
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::audio("Sample", 0.0, 4.0, "media/kick.wav"));
    project.save(&path).unwrap();
    let loaded = Project::load(&path).unwrap();
    let ClipContent::Audio { path: media, .. } = &loaded.tracks[0].clips[0].content else {
        panic!("expected audio")
    };
    assert_eq!(
        std::path::PathBuf::from(media),
        dir.path().canonicalize().unwrap().join("media/kick.wav")
    );
}

#[test]
fn corrupt_project_is_rejected_without_overwriting_saved_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("session.daw");
    let mut project = Project::demo();
    project.save(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    project.tempo = f64::NAN;
    assert!(project.save(&path).is_err());
    assert_eq!(original, std::fs::read(&path).unwrap());
    project.tempo = 120.0;
    project.tracks[1].id = project.tracks[0].id;
    assert!(project.validate().is_err());
}

#[test]
fn scales_transpose_and_wrap_without_u8_overflow() {
    for pitch in 0..=127 {
        assert!(Scale::Chromatic.contains(255, pitch));
    }
    assert!(Scale::Minor.contains(9, 60)); // C in A minor
    assert!(!Scale::Minor.contains(9, 61));
    assert!(Scale::Major.contains(11, 63)); // D# in B major
    assert!(!Scale::Major.contains(11, 62));
    assert_eq!(daw_core::note_name(60), "C4");
}

#[test]
fn wav_roundtrip_preserves_interleaving_and_headroom() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wav");
    let data = AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: vec![-1.0, 0.25, 0.5, 1.5],
    };
    data.write_wav(&path).unwrap();
    let loaded = AudioData::load_wav(&path).unwrap();
    assert_eq!(loaded.samples, data.samples);
    assert_eq!(loaded.channels, 2);
    assert_eq!(loaded.frames(), 2);
    assert_eq!(loaded.duration_seconds(), 2.0 / 48_000.0);
}

#[test]
fn pcm_wav_normalizes_correctly() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("pcm.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for sample in [i16::MIN, 0, i16::MAX] {
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    let audio = AudioData::load_wav(path).unwrap();
    assert_eq!(audio.samples, vec![-1.0, 0.0, 32767.0 / 32768.0]);
}

#[test]
fn midi_export_clips_source_notes_to_visible_range() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("phrase.mid");
    let (mut project, track, clip) = fixture();
    project.trim_clip(track, clip, 6.0, 4.0).unwrap(); // source range [2, 6]
    project.export_midi(&path).unwrap();
    let imported = daw_core::import_midi(&path).unwrap();
    let ClipContent::Midi { notes } = &imported.tracks[0].clips[0].content else {
        panic!("expected MIDI")
    };
    assert_eq!(notes.len(), 2);
    assert_eq!((notes[0].start, notes[0].length), (6.0, 4.0));
    assert_eq!((notes[1].start, notes[1].length), (9.0, 1.0));
    assert_eq!(imported.tempo, Some(120.0));
    assert_eq!(imported.beats_per_bar, Some(4));
}

#[test]
fn midi_rejects_tempo_maps_and_accepts_velocity_zero_note_off() {
    use midly::{Header, MetaMessage, MidiMessage, Smf, TrackEvent, TrackEventKind};
    let dir = tempdir().unwrap();
    let path = dir.path().join("input.mid");
    let meta = |delta: u32, tempo: u32| TrackEvent {
        delta: delta.into(),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(tempo.into())),
    };
    let note = |delta: u32, velocity: u8| TrackEvent {
        delta: delta.into(),
        kind: TrackEventKind::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOn {
                key: 60.into(),
                vel: velocity.into(),
            },
        },
    };
    let mut smf = Smf {
        header: Header::new(
            midly::Format::SingleTrack,
            midly::Timing::Metrical(480.into()),
        ),
        tracks: vec![vec![
            meta(0, 500_000),
            note(0, 96),
            note(480, 0),
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            },
        ]],
    };
    smf.save(&path).unwrap();
    let imported = daw_core::import_midi(&path).unwrap();
    let ClipContent::Midi { notes } = &imported.tracks[0].clips[0].content else {
        panic!("expected MIDI")
    };
    assert_eq!(notes[0].length, 1.0);
    smf.tracks[0].insert(2, meta(240, 600_000));
    smf.save(&path).unwrap();
    assert!(daw_core::import_midi(&path).is_err());
    // A lone tempo event after beat zero still changes the default 120 BPM.
    smf.tracks[0].remove(0);
    smf.save(&path).unwrap();
    assert!(daw_core::import_midi(&path).is_err());
}
