use daw_core::*;
use serde_json::json;

fn batch(edits: serde_json::Value) -> EditBatch {
    serde_json::from_value(json!({"label":"Test edit","edits":edits})).unwrap()
}

#[test]
fn composition_uses_host_ids_and_batch_references_in_one_history_step() {
    let mut p = Project::new();
    let before = p.clone();
    let mut history = History::new();
    let result = batch(json!([
        {"op":"add_track","kind":"midi","name":"Bass","key":"bass"},
        {"op":"add_midi_clip","track":"bass","name":"Phrase","start":8.0,"length":4.0,"notes":[{"pitch":36,"start":0.0,"length":1.0,"velocity":95}],"key":"phrase"},
        {"op":"transform_notes","track":"bass","clip":"phrase","semitones":12,"beats":0.5},
        {"op":"set_mix","channels":[{"track":"bass","gain_db":-6.0,"pan":-0.25}]}
    ])).apply(&mut p).unwrap();
    assert!(result.changed);
    assert_eq!(result.created.len(), 2);
    assert_eq!(p.tracks[0].id, result.created[0].id);
    let notes = p
        .midi_notes(p.tracks[0].id, p.tracks[0].clips[0].id)
        .unwrap();
    assert_eq!((notes[0].pitch, notes[0].start), (48, 0.5));
    history.commit_named(&before, "AI: Bass");
    assert!(history.undo(&mut p));
    assert_eq!(p, before);
    assert!(!history.can_undo());
}

#[test]
fn rejected_later_edit_leaves_every_part_of_song_unchanged() {
    let mut p = Project::new();
    let before = p.clone();
    let err = batch(json!([
        {"op":"add_track","kind":"midi","name":"Bass","key":"bass"},
        {"op":"set_tempo","bpm":100.0},
        {"op":"set_mix","channels":[{"track":"bass","pan":2.0}]}
    ]))
    .apply(&mut p)
    .unwrap_err();
    assert!(err.to_string().contains("pan"));
    assert_eq!(p, before);
    assert!(
        batch(json!([
            {"op":"add_track","kind":"audio","name":"A","key":"same"},
            {"op":"add_track","kind":"audio","name":"B","key":"same"}
        ]))
        .apply(&mut p)
        .is_err()
    );
    assert_eq!(p, before);
}

#[test]
fn editing_nonselected_ghost_updates_all_peers_but_keeps_placements() {
    let mut p = Project::new();
    let t = p.add_midi_track("Synth");
    let clip = Clip::midi("Part", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 100)]);
    let c = clip.id;
    p.track_mut(t).unwrap().clips.push(clip);
    let ghost = p.clone_clip(t, c).unwrap();
    batch(json!([
        {"op":"transform_notes","track":t,"clip":ghost,"semitones":7,"beats":0.0},
        {"op":"rename_clip","track":t,"clip":ghost,"name":"Updated"}
    ]))
    .apply(&mut p)
    .unwrap();
    assert_eq!(p.clip(t, c).unwrap().start, 0.0);
    assert_eq!(p.clip(t, ghost).unwrap().start, 4.0);
    assert_eq!(p.clip(t, c).unwrap().name, "Updated");
    assert_eq!(p.midi_notes(t, c).unwrap()[0].pitch, 67);
    assert_eq!(p.midi_notes(t, ghost), p.midi_notes(t, c));
    p.validate().unwrap();
}

#[test]
fn routing_cycles_are_rejected_atomically() {
    let mut p = Project::new();
    let a = p.add_bus("A");
    let b = p.add_bus("B");
    let before = p.clone();
    assert!(
        batch(json!([
            {"op":"set_output","track":a,"bus":b},
            {"op":"set_output","track":b,"bus":a}
        ]))
        .apply(&mut p)
        .is_err()
    );
    assert_eq!(p, before);
}

#[test]
fn malformed_fields_and_unbounded_notes_cannot_enter_commands() {
    for value in [
        json!({"op":"open","path":"/tmp/song.rill"}),
        json!({"op":"add_track","kind":"midi","name":"A","id":123}),
        json!({"op":"rename_song","name":"A","path":"/tmp/secret"}),
    ] {
        assert!(serde_json::from_value::<SongEdit>(value).is_err());
    }
    let mut p = Project::new();
    let data = NoteData {
        pitch: 60,
        start: 0.0,
        length: 1.0,
        velocity: 100,
    };
    let edits = EditBatch {
        label: "Too large".into(),
        edits: vec![SongEdit::AddMidiClip {
            track: 1.into(),
            name: "A".into(),
            start: 0.0,
            length: 4.0,
            notes: vec![data; MAX_BATCH_NOTES + 1],
            key: None,
        }],
    };
    assert!(
        edits
            .apply(&mut p)
            .unwrap_err()
            .to_string()
            .contains("Too many notes")
    );
}

#[test]
fn session_revision_advances_and_reset_revokes_earlier_session() {
    let mut session = SongSession::default();
    let original = session;
    session.bump();
    assert_eq!(session.revision, 1);
    assert_eq!(session.id, original.id);
    session.reset();
    assert_ne!(session.id, original.id);
    assert_eq!(session.revision, 0);
}

#[test]
fn quantize_preserves_unselected_sources_and_zero_strength_is_a_noop() {
    let mut p = Project::new();
    let track = p.add_midi_track("Quantize");
    let original = Note::new(60, 0.7, 0.3, 100);
    let selected = Note::new(64, 1.1, 0.4, 90);
    let mut clip = Clip::midi(
        "Stretched",
        0.125,
        4.0,
        vec![original.clone(), selected.clone()],
    );
    clip.source_rate = 1.3;
    clip.offset = 0.5;
    let id = clip.id;
    p.track_mut(track).unwrap().clips.push(clip);
    let before = p.clone();
    let noop = batch(
        json!([{"op":"quantize_notes","track":track,"clip":id,"note_ids":[selected.id],"grid":0.25,"strength":0.0,"swing":0.5,"ends":true}]),
    );
    assert!(!noop.apply(&mut p).unwrap().changed);
    assert_eq!(p, before);
    batch(json!([{"op":"quantize_notes","track":track,"clip":id,"note_ids":[selected.id],"grid":0.25,"strength":1.0,"swing":0.5,"ends":false}])).apply(&mut p).unwrap();
    let notes = p.midi_notes(track, id).unwrap();
    assert_eq!(notes[0], original);
    assert_eq!(notes[1].length, selected.length);
}

#[test]
fn randomize_stretched_linked_source_matches_preview_and_is_one_undoable_edit() {
    let mut p = Project::new();
    let track = p.add_midi_track("Randomize");
    let untouched = Note::new(60, 0.7, 0.3, 100);
    let chosen = Note::new(64, 1.1, 0.4, 90);
    let source = vec![untouched.clone(), chosen.clone()];
    let mut clip = Clip::midi("Stretched", 0.125, 4.0, source.clone());
    clip.source_rate = 1.3;
    clip.offset = 0.5;
    let id = clip.id;
    p.track_mut(track).unwrap().clips.push(clip);
    let ghost = p.clone_clip(track, id).unwrap();
    let before = p.clone();
    let settings = RandomizeSettings {
        velocity: 24,
        timing: 0.125,
        seed: 34,
    };
    let mut audible = source.clone();
    for note in &mut audible {
        note.start /= 1.3;
        note.length /= 1.3;
    }
    let expected = randomize_notes(
        &audible,
        &std::collections::HashSet::from([chosen.id]),
        settings,
    )
    .unwrap();
    let edit = batch(
        json!([{"op":"randomize_notes","track":track,"clip":ghost,"note_ids":[chosen.id],"velocity":24,"timing":0.125,"seed":34}]),
    );
    assert!(edit.apply(&mut p).unwrap().changed);
    let notes = p.midi_notes(track, id).unwrap();
    assert_eq!(notes, p.midi_notes(track, ghost).unwrap());
    assert_eq!(notes[0], untouched);
    assert_eq!(notes[1].start, expected[1].start * 1.3);
    assert_eq!(notes[1].velocity, expected[1].velocity);
    assert_eq!(notes[1].length.to_bits(), chosen.length.to_bits());
    assert_eq!(p.clip(track, id).unwrap().offset, 0.5);
    assert_eq!(p.clip(track, ghost).unwrap().source_rate, 1.3);
    assert_eq!(
        p.clip(track, ghost).unwrap().start,
        before.clip(track, ghost).unwrap().start
    );
    let mut replay = before.clone();
    edit.apply(&mut replay).unwrap();
    assert_eq!(replay, p);
    let mut history = History::new();
    history.commit_named(&before, "Randomize notes");
    assert_eq!(history.undo_labels().count(), 1);
    assert!(history.undo(&mut p));
    assert_eq!(p, before);
}

#[test]
fn randomize_defaults_noop_and_invalid_settings_are_atomic() {
    let mut p = Project::new();
    let track = p.add_midi_track("Humanize");
    let original = Note::new(60, 0.7, 0.3, 100);
    let mut clip = Clip::midi("Source", 0.0, 4.0, vec![original.clone()]);
    clip.source_rate = 1.3;
    let id = clip.id;
    p.track_mut(track).unwrap().clips.push(clip);
    let before = p.clone();
    assert!(
        !batch(json!([{"op":"randomize_notes","track":track,"clip":id,"velocity":0}]))
            .apply(&mut p)
            .unwrap()
            .changed
    );
    assert_eq!(p, before);
    for extra in [
        json!({"velocity":128}),
        json!({"timing":-0.1}),
        json!({"timing":0.251}),
        json!({"note_ids":[u64::MAX]}),
    ] {
        let mut value = json!({"op":"randomize_notes","track":track,"clip":id});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(
            batch(json!([{"op":"set_tempo","bpm":90},value]))
                .apply(&mut p)
                .is_err()
        );
        assert_eq!(p, before);
    }
    batch(json!([{"op":"randomize_notes","track":track,"clip":id}]))
        .apply(&mut p)
        .unwrap();
    let note = &p.midi_notes(track, id).unwrap()[0];
    assert_eq!(
        (
            note.id,
            note.pitch,
            note.start.to_bits(),
            note.length.to_bits()
        ),
        (
            original.id,
            original.pitch,
            original.start.to_bits(),
            original.length.to_bits()
        )
    );
    assert!((i16::from(note.velocity) - i16::from(original.velocity)).abs() <= 16);
}

#[test]
fn bulk_mix_can_set_exact_silence_and_rejects_ambiguous_gain_atomically() {
    let mut p = Project::new();
    let t = p.add_audio_track("Audio");
    batch(json!([{"op":"set_mix","channels":[{"track":t,"gain":0.0,"pan":0.25}]}]))
        .apply(&mut p)
        .unwrap();
    assert_eq!(p.track(t).unwrap().gain, 0.0);
    assert!(!p.track(t).unwrap().mute);
    let before = p.clone();
    assert!(
        batch(
            json!([{"op":"set_mix","channels":[{"track":t,"gain":1.0,"gain_db":-6.0,"pan":0.75}]}])
        )
        .apply(&mut p)
        .is_err()
    );
    assert_eq!(p, before);
    batch(json!([{"op":"rename_track","track":t,"name":"Ž".repeat(1024)}]))
        .apply(&mut p)
        .unwrap();
    assert_eq!(p.track(t).unwrap().name.chars().count(), 1024);
}

#[test]
fn moving_a_comp_to_another_track_detaches_only_the_audible_region() {
    let mut p = Project::new();
    let source = p.add_audio_track("Takes");
    let destination = p.add_audio_track("Comp edit");
    let a = Clip::audio("A", 0.0, 4.0, "retained-a.wav");
    let b = Clip::audio("B", 0.0, 4.0, "retained-b.wav");
    let ids = [a.id, b.id];
    p.track_mut(source).unwrap().clips = vec![a, b];
    let group = p.group_audio_takes(source, &ids).unwrap();
    let source_id = p.audio_take_sources(source, group)[0].id;
    let comp = p
        .select_audio_take(source, group, source_id, None, 0.0)
        .unwrap();
    batch(json!([{"op":"move_clip_to_track","track":source,"clip":comp,"destination":destination,"start":8.0}])).apply(&mut p).unwrap();
    let moved = p.clip(destination, comp).unwrap();
    assert_eq!(moved.start, 8.0);
    assert!(moved.audio_take.is_none());
    assert!(p.clip(source, comp).is_none());
    assert_eq!(p.audio_take_sources(source, group).len(), 2);
    p.validate().unwrap();
}

#[test]
fn trusted_source_durations_override_forged_slip_bounds_and_follow_tempo_edits() {
    let mut p = Project::new();
    let t = p.add_audio_track("Audio");
    let clip = Clip::audio("Source", 0.0, 4.0, "source.wav");
    let c = clip.id;
    p.track_mut(t).unwrap().clips.push(clip);
    let edit = batch(json!([
        {"op":"set_tempo","bpm":60.0},
        {"op":"duplicate_clip","track":t,"clip":c,"start":4.0,"key":"copy"},
        {"op":"slip_audio","track":t,"clip":"copy","delta":-1000.0,"source_beats":10000.0}
    ]));
    let result = edit
        .apply_with_source_durations(&mut p, &|path| (path == "source.wav").then_some(10.0))
        .unwrap();
    let id = result
        .created
        .iter()
        .find(|e| e.key.as_deref() == Some("copy"))
        .unwrap()
        .id;
    assert_eq!(p.clip(t, id).unwrap().offset, 6.0);
    let before = p.clone();
    assert!(
        batch(json!([{"op":"slip_audio","track":t,"clip":c,"delta":1.0,"source_beats":10000.0}]))
            .apply_with_source_durations(&mut p, &|_| None)
            .is_err()
    );
    assert_eq!(p, before);
}

#[test]
fn relative_track_commands_use_batch_references_and_reject_cycles_atomically() {
    let mut project = Project::new();
    let result = batch(json!([
        {"op":"add_track","kind":"folder","name":"Group","key":"folder"},
        {"op":"add_track","kind":"midi","name":"Part","key":"part"},
        {"op":"add_track","kind":"bus","name":"Room","key":"bus"},
        {"op":"move_track","track":"part","target":"folder","placement":"into_folder"},
        {"op":"move_track","track":"bus","target":"folder","placement":"before"}
    ]))
    .apply(&mut project)
    .unwrap();
    let (folder, part, bus) = (
        result.created[0].id,
        result.created[1].id,
        result.created[2].id,
    );
    assert_eq!(project.ordered_track_ids(), vec![bus, folder, part]);
    assert_eq!(project.track(part).unwrap().parent_folder, Some(folder));
    let before = project.clone();
    assert!(
        batch(json!([
            {"op":"rename_track","track":bus,"name":"Changed"},
            {"op":"move_track","track":folder,"target":part,"placement":"before"}
        ]))
        .apply(&mut project)
        .is_err()
    );
    assert_eq!(project, before);
    let no_op = batch(json!([
        {"op":"move_track","track":part,"target":folder,"placement":"into_folder"}
    ]))
    .apply(&mut project)
    .unwrap();
    assert!(!no_op.changed);
    assert_eq!(project, before);
}
