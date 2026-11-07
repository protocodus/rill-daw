use daw_core::*;
use serde_json::json;

fn packet(group: u8, channel: u8, status: u8, index: u8, detail: u8, data: u32) -> UmpPacket {
    Midi2ChannelVoice {
        group,
        channel,
        status,
        index,
        detail,
        data,
    }
    .packet()
    .unwrap()
}
fn note(group: u8, channel: u8, index: u8, velocity: u16) -> Note {
    let mut note = Note::new(index, 0.0, 4.0, 90);
    let message = packet(group, channel, 9, index, 0, u32::from(velocity) << 16)
        .midi2_channel_voice()
        .unwrap();
    note.set_midi2(Midi2NoteData::from_note_on(message).unwrap())
        .unwrap();
    note
}
fn project() -> (Project, u64, u64, u64) {
    let mut project = Project::new();
    let track = project.add_midi_track("Expressive");
    let note = note(7, 4, 60, 32_769);
    let id = note.id;
    let clip = Clip::midi("Performance", 0.0, 8.0, vec![note]);
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    project
        .upsert_note_expression(
            track,
            clip_id,
            id,
            0.25,
            packet(7, 4, 10, 60, 0, 0x1234_5678),
        )
        .unwrap();
    project
        .set_midi_event(
            track,
            clip_id,
            None,
            0.75,
            packet(7, 4, 11, 11, 0, 0xffff_fffe),
        )
        .unwrap();
    (project, track, clip_id, id)
}

#[test]
fn ump_packet_boundaries_preserve_payloads_groups_and_reserved_bits() {
    let packets = [
        vec![0x0020_ffff],
        vec![0x2fbf_0b7f],
        vec![0x4f9f_be7a, 0xabcd_9876],
        vec![0x3f06_0102, 0x0304_0506],
        vec![0xd700_0000, 1, 2, 3],
        vec![0xf000_0000, 4, 5, 6],
    ];
    for words in packets {
        let packet = UmpPacket::from_words(&words).unwrap();
        packet.validate().unwrap();
        assert_eq!(&packet.words[..words.len()], words);
        if words.len() > 1 {
            assert!(UmpPacket::from_words(&words[..words.len() - 1]).is_err());
        }
    }
    let raw = UmpPacket::from_words(&[0x4f9f_be7a, 0xabcd_9876]).unwrap();
    let decoded = raw.midi2_channel_voice().unwrap();
    assert_eq!(
        (
            decoded.group,
            decoded.channel,
            decoded.status,
            decoded.index,
            decoded.detail
        ),
        (15, 15, 9, 0xbe, 0x7a)
    );
    assert_eq!(decoded.velocity16(), Some(0xabcd));
    assert_eq!(decoded.attribute_data(), Some(0x9876));
    assert_eq!(decoded.packet().unwrap(), raw);
    assert_eq!(UmpPacket::from_words(&[0x0010_ffff]).unwrap().group(), None);
    assert_eq!(
        UmpPacket::from_words(&[0xf000_0000, 0, 0, 0])
            .unwrap()
            .group(),
        None
    );
    for message_type in 0..16 {
        let count = [1, 1, 1, 2, 2, 4, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4][message_type];
        let mut words = vec![0; count];
        words[0] = (message_type as u32) << 28;
        assert_eq!(
            UmpPacket::from_words(&words).unwrap().word_count as usize,
            count
        );
    }
    assert!(
        UmpPacket {
            words: [0; 4],
            word_count: 255
        }
        .validate()
        .is_err()
    );
    assert!(
        UmpPacket {
            words: [0x4090_3c00, 0, 1, 0],
            word_count: 2
        }
        .validate()
        .is_err()
    );
    assert!(UmpPacket::from_words(&[]).is_err());
}

#[test]
fn all_sixteen_bit_velocities_including_real_zero_survive_editing() {
    let mut note = Note::new(60, 0.0, 1.0, 90);
    for velocity in 0..=u16::MAX {
        note.set_velocity16(velocity);
        validate_note(&note).unwrap();
        assert_eq!(note.velocity16(), velocity);
        assert!(note.velocity > 0);
    }
    note.set_velocity16(0);
    assert_eq!(note.velocity, 1);
    assert_eq!(note.velocity_normalized(), 0.0);
    assert_eq!(midi1_velocity_to_u16(0), 0);
    assert_eq!(midi1_velocity_to_u16(64), 0x8000);
    assert_eq!(midi1_velocity_to_u16(100), 0xc924);
    assert_eq!(midi1_velocity_to_u16(127), u16::MAX);
}

#[test]
fn manual_coarse_edits_update_precision_and_round_trip_packet_addresses() {
    let mut note = note(3, 5, 60, 32_769);
    note.midi2.as_mut().unwrap().attribute_type = 1;
    note.midi2.as_mut().unwrap().attribute_data = 0x7a3b;
    note.midi2
        .as_mut()
        .unwrap()
        .expressions
        .push(MidiNoteExpression::new(
            0.5,
            packet(3, 5, 0, 60, 74, 0xdead_beef),
        ));
    let original = note.clone();
    note.set_pitch_semitones(72.0).unwrap();
    assert_eq!(note.midi2.as_ref().unwrap().attribute_type, 1);
    assert_eq!(
        note.midi2.as_ref().unwrap().expressions[0]
            .packet
            .midi2_channel_voice()
            .unwrap()
            .index,
        72
    );
    note.set_pitch_semitones(60.0).unwrap();
    assert_eq!(note, original);
    note.set_velocity(65);
    note.set_velocity(64);
    assert_eq!(note.velocity16(), 32_768);
    assert_ne!(note.velocity16(), original.velocity16());
    note.velocity = 65;
    assert!(validate_note(&note).is_err());
    note.set_velocity(65);
    validate_note(&note).unwrap();
}

#[test]
fn fine_pitch_preserves_wire_index_and_legacy_attack_during_unrelated_edits() {
    let mut note = Note::new(60, 0.0, 2.0, 64);
    let mut data = Midi2NoteData::from_legacy(&note);
    data.channel = 4;
    note.set_midi2(data).unwrap();
    let attack = note.velocity_normalized();
    assert_eq!(attack, 64.0 / 127.0);
    note.set_release_velocity16(12_345);
    note.set_pitch_semitones(60.25).unwrap();
    assert_eq!(note.velocity_normalized(), attack);
    assert_eq!(note.midi2.as_ref().unwrap().protocol, 1);
    assert_eq!(note.midi2.as_ref().unwrap().note_index, 60);
    assert_eq!(note.pitch_semitones(), 60.25);
    note.set_pitch(72);
    assert_eq!(note.pitch_semitones(), 72.25);
    let before = note.clone();
    for value in [f64::NAN, -0.25, 128.0, f64::INFINITY] {
        assert!(note.set_pitch_semitones(value).is_err());
        assert_eq!(note, before);
    }
    note.set_velocity16(32_769);
    assert_eq!(note.midi2.as_ref().unwrap().protocol, 2);
    assert_eq!(note.velocity_normalized(), 32_769.0 / 65_535.0);
}

#[test]
fn timing_only_edits_and_coarse_noops_keep_every_velocity_bit_and_protocol() {
    let mut note = note(0, 0, 60, 0x8123);
    note.start = 0.12;
    note.midi2
        .as_mut()
        .unwrap()
        .expressions
        .push(MidiNoteExpression::new(
            0.5,
            packet(0, 0, 10, 60, 0, 0xffff_fffe),
        ));
    let data = note.midi2.clone();
    note.set_velocity(note.velocity);
    note.set_pitch(note.pitch);
    assert_eq!(note.midi2, data);
    let selected = std::collections::HashSet::from([note.id]);
    let randomized = randomize_notes(
        &[note],
        &selected,
        RandomizeSettings {
            velocity: 0,
            timing: 0.05,
            seed: 3,
        },
    )
    .unwrap();
    assert_eq!(randomized[0].midi2, data);
    let quantized = quantize_notes(
        &randomized,
        &selected,
        0.0,
        QuantizeSettings {
            grid: 0.25,
            strength: 1.0,
            swing: 0.5,
            ends: true,
        },
    )
    .unwrap();
    assert_eq!(quantized[0].midi2, data);
    let mut flipped = quantized[0].clone();
    flipped.set_pitch_semitones(72.0).unwrap();
    flipped.set_pitch_semitones(60.0).unwrap();
    assert_eq!(flipped.midi2, data);
}

#[test]
fn changing_the_start_edge_carries_state_or_moves_expression_time_without_touching_attack() {
    let mut note = note(0, 0, 60, 0x8123);
    note.start = 2.0;
    note.midi2.as_mut().unwrap().expressions = vec![
        MidiNoteExpression::new(0.0, packet(0, 0, 10, 60, 0, 1)),
        MidiNoteExpression::new(1.0, packet(0, 0, 10, 60, 0, 2)),
        MidiNoteExpression::new(3.0, packet(0, 0, 10, 60, 0, 3)),
    ];
    let original = note.clone();
    note.trim_start_preserving_end(3.5).unwrap();
    assert_eq!((note.start, note.length), (3.5, 2.5));
    assert_eq!(note.velocity16(), 0x8123);
    let points = &note.midi2.as_ref().unwrap().expressions;
    assert_eq!(
        points
            .iter()
            .map(|point| (point.beat, point.packet.words[1]))
            .collect::<Vec<_>>(),
        [(0.0, 2), (1.5, 3)]
    );
    let mut extended = original.clone();
    extended.trim_start_preserving_end(1.0).unwrap();
    for (old, new) in original
        .midi2
        .as_ref()
        .unwrap()
        .expressions
        .iter()
        .zip(&extended.midi2.as_ref().unwrap().expressions)
    {
        assert_eq!(original.start + old.beat, extended.start + new.beat);
        assert_eq!(old.id, new.id);
    }
    let before = extended.clone();
    assert!(extended.trim_start_preserving_end(6.0).is_err());
    assert_eq!(extended, before);
}

#[test]
fn ghost_copy_independent_copy_stretch_and_archive_keep_exact_performance() {
    let (mut project, track, clip, id) = project();
    let ghost = project.clone_clip(track, clip).unwrap();
    project
        .set_note_velocity16(track, clip, id, 40_001)
        .unwrap();
    assert_eq!(
        project.midi_notes(track, ghost).unwrap()[0].velocity16(),
        40_001
    );
    let before_events = project.clip(track, clip).unwrap().midi_events.clone();
    let before_expression = project.midi_notes(track, clip).unwrap()[0]
        .midi2
        .as_ref()
        .unwrap()
        .expressions
        .clone();
    project.stretch_clip(track, clip, 16.0).unwrap();
    assert_eq!(
        project.clip(track, clip).unwrap().midi_events,
        before_events
    );
    assert_eq!(
        project.midi_notes(track, clip).unwrap()[0]
            .midi2
            .as_ref()
            .unwrap()
            .expressions,
        before_expression
    );
    project.make_clip_independent(track, ghost).unwrap();
    assert_ne!(
        project.clip(track, ghost).unwrap().midi_events[0].id,
        before_events[0].id
    );
    assert_ne!(
        project.midi_notes(track, ghost).unwrap()[0]
            .midi2
            .as_ref()
            .unwrap()
            .expressions[0]
            .id,
        before_expression[0].id
    );
    project.validate().unwrap();
    let dir = tempfile::tempdir().unwrap();
    for extension in ["json", "rill"] {
        let path = dir.path().join(format!("midi2.{extension}"));
        project.save(&path).unwrap();
        let restored = Project::load(&path).unwrap();
        assert_eq!(restored, project);
    }
    assert!(project.contains_midi2());
}

#[test]
fn split_duplicate_and_paste_notes_preserve_expression_and_unique_identities() {
    let (mut project, track, clip, id) = project();
    project
        .upsert_note_expression(track, clip, id, 1.0, packet(7, 4, 10, 60, 0, 0x8000_0001))
        .unwrap();
    project
        .upsert_note_expression(track, clip, id, 3.0, packet(7, 4, 10, 60, 0, 0xffff_ffff))
        .unwrap();
    let right = project.split_note(track, clip, id, 2.0).unwrap();
    let notes = project.midi_notes(track, clip).unwrap();
    let left = notes.iter().find(|note| note.id == id).unwrap();
    assert_eq!(
        left.midi2.as_ref().unwrap().expressions.len(),
        3,
        "hidden data is retained beyond the shortened end"
    );
    let right_note = notes.iter().find(|note| note.id == right).unwrap();
    let points = &right_note.midi2.as_ref().unwrap().expressions;
    assert_eq!(
        points
            .iter()
            .map(|point| (point.beat, point.packet.words[1]))
            .collect::<Vec<_>>(),
        [(0.0, 0x8000_0001), (1.0, 0xffff_ffff)]
    );
    let duplicate = project.duplicate_note(track, clip, right).unwrap();
    let notes = project.midi_notes(track, clip).unwrap();
    let selected = std::collections::HashSet::from([duplicate]);
    let clipboard = copy_midi_notes(notes, &selected).unwrap();
    let (pasted, _) = paste_midi_notes(notes, &clipboard, 10.0).unwrap();
    *project.midi_notes_mut(track, clip).unwrap() = pasted;
    project.validate().unwrap();
    assert!(
        project
            .midi_notes(track, clip)
            .unwrap()
            .iter()
            .all(|note| note.velocity16() == 32_769)
    );
}

#[test]
fn packet_edit_batches_are_atomic_undoable_and_reject_duplicate_note_streams() {
    let mut project = Project::new();
    let track = project.add_midi_track("Keys");
    let note = Note::new(60, 0.0, 4.0, 90);
    let note_id = note.id;
    let clip = Clip::midi("Part", 0.0, 4.0, vec![note]);
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let before = project.clone();
    let edit:EditBatch=serde_json::from_value(json!({"label":"Expressive edit","edits":[
        {"op":"set_note_velocity16","track":track,"clip":clip_id,"note":note_id,"velocity":40001},
        {"op":"set_note_release_velocity16","track":track,"clip":clip_id,"note":note_id,"velocity":12345},
        {"op":"upsert_note_expression","track":track,"clip":clip_id,"note":note_id,"beat":0.5,"packet":packet(0,0,10,60,0,0x12345678),"key":"pressure"},
        {"op":"update_note_expression","track":track,"clip":clip_id,"note":note_id,"point":"pressure","beat":0.75,"packet":packet(0,0,10,60,0,0x12345679)},
        {"op":"set_midi_event","track":track,"clip":clip_id,"event":null,"beat":1.0,"packet":packet(0,0,11,11,0,0xffffffff)}
    ]})).unwrap();
    edit.apply(&mut project).unwrap();
    project.validate().unwrap();
    let changed = project.clone();
    let invalid:EditBatch=serde_json::from_value(json!({"label":"Bad packet","edits":[
        {"op":"set_note_velocity16","track":track,"clip":clip_id,"note":note_id,"velocity":0},
        {"op":"set_midi_event","track":track,"clip":clip_id,"event":null,"beat":1.0,"packet":packet(0,0,9,60,0,0xffff0000)}
    ]})).unwrap();
    assert!(invalid.apply(&mut project).is_err());
    assert_eq!(project, changed);
    let mut history = History::new();
    history.commit_named(&before, "Expressive edit");
    assert!(history.undo(&mut project));
    assert_eq!(project, before);
    assert!(history.redo(&mut project));
    assert_eq!(project, changed);
}

#[test]
fn legacy_midi_export_keeps_recorded_channel_and_release_without_claiming_ump() {
    let mut project = Project::new();
    let track = project.add_midi_track("Legacy");
    let mut note = Note::new(60, 0.0, 1.0, 64);
    let mut data = Midi2NoteData::from_legacy(&note);
    data.channel = 4;
    data.release_velocity = midi1_velocity_to_u16(77);
    note.set_midi2(data).unwrap();
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("Part", 0.0, 4.0, vec![note]));
    assert!(!project.contains_midi2());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.mid");
    project.export_midi(&path).unwrap();
    let data = std::fs::read(path).unwrap();
    let smf = midly::Smf::parse(&data).unwrap();
    let messages: Vec<_> = smf
        .tracks
        .iter()
        .flatten()
        .filter_map(|event| match event.kind {
            midly::TrackEventKind::Midi { channel, message } => Some((channel.as_int(), message)),
            _ => None,
        })
        .collect();
    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(|(channel, _)| *channel == 4));
    assert!(matches!(messages[0].1,midly::MidiMessage::NoteOn{vel,..}if vel.as_int()==64));
    assert!(matches!(messages[1].1,midly::MidiMessage::NoteOff{vel,..}if vel.as_int()==77));
}

#[test]
fn every_group_keeps_overlapping_same_pitch_notes_and_expression_identities_separate() {
    let mut project = Project::new();
    let track = project.add_midi_track("Groups");
    let mut notes = Vec::new();
    for group in 0..16 {
        for voice in 0..2 {
            let mut note = note(group, 3, 60, 0x8123 + voice);
            note.start = f64::from(voice) * 0.5;
            note.midi2
                .as_mut()
                .unwrap()
                .expressions
                .push(MidiNoteExpression::new(
                    0.25,
                    packet(
                        group,
                        3,
                        10,
                        60,
                        0,
                        0x8000_0000 + u32::from(group) * 2 + u32::from(voice),
                    ),
                ));
            notes.push(note);
        }
    }
    let ids: std::collections::HashSet<_> = notes.iter().map(|note| note.id).collect();
    assert_eq!(ids.len(), 32);
    project
        .track_mut(track)
        .unwrap()
        .clips
        .push(Clip::midi("Overlaps", 0.0, 8.0, notes));
    project.validate().unwrap();
    let clip = project.track(track).unwrap().clips[0].id;
    for note in project.midi_notes(track, clip).unwrap() {
        let data = note.midi2.as_ref().unwrap();
        let message = data.expressions[0].packet.midi2_channel_voice().unwrap();
        assert_eq!(
            (message.group, message.channel, message.index),
            (data.group, data.channel, data.note_index)
        );
        assert_eq!(
            message.data,
            0x8000_0000 + u32::from(data.group) * 2 + u32::from(note.velocity16() - 0x8123)
        );
    }
}

#[test]
fn expression_capacity_rejects_growth_before_mutation_but_allows_replacing_an_existing_point() {
    let mut project = Project::new();
    let track = project.add_midi_track("Bounded");
    let mut note = note(0, 0, 60, 0x8123);
    let id = note.id;
    let pressure = packet(0, 0, 10, 60, 0, 1);
    note.midi2.as_mut().unwrap().expressions = (0..MAX_NOTE_EXPRESSIONS)
        .map(|index| MidiNoteExpression::new(index as f64 / 128.0, pressure))
        .collect();
    let clip = Clip::midi("Performance", 0.0, 4.0, vec![note]);
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    project.validate().unwrap();
    let before = project.clone();
    assert!(
        project
            .upsert_note_expression(track, clip_id, id, 400.0, pressure)
            .is_err()
    );
    assert_eq!(project, before);
    project
        .upsert_note_expression(
            track,
            clip_id,
            id,
            0.0,
            packet(0, 0, 10, 60, 0, 0xffff_ffff),
        )
        .unwrap();
    assert_eq!(
        project.midi_notes(track, clip_id).unwrap()[0]
            .midi2
            .as_ref()
            .unwrap()
            .expressions
            .len(),
        MAX_NOTE_EXPRESSIONS
    );
    assert_eq!(
        project.midi_notes(track, clip_id).unwrap()[0]
            .midi2
            .as_ref()
            .unwrap()
            .expressions[0]
            .packet
            .words[1],
        u32::MAX
    );
}
