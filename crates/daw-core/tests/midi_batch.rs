use daw_core::{
    Clip, Midi2NoteData, MidiNoteExpression, MidiNoteLengthChange, Note, Project, UmpPacket,
};
use std::collections::HashSet;

fn fixture() -> (Project, u64, u64, u64, Vec<u64>) {
    let mut project = Project::new();
    let track = project.add_midi_track("Batch lengths");
    let mut notes = vec![
        Note::new(60, 1.0, 0.25, 81),
        Note::new(64, 1.5, 0.5, 91),
        Note::new(67, 4.0, 0.75, 103),
    ];
    let mut data = Midi2NoteData::from_legacy(&notes[0]);
    data.protocol = 2;
    data.velocity = 43_001;
    data.release_velocity = 11_113;
    let packet = UmpPacket::from_words(&[0x40a0_3c00, 0xdead_beef]).unwrap();
    data.expressions = vec![
        MidiNoteExpression::new(0.1, packet),
        MidiNoteExpression::new(0.4, packet),
    ];
    notes[0].set_midi2(data).unwrap();
    let ids = notes.iter().map(|note| note.id).collect();
    let mut clip = Clip::midi("Source", 2.0, 4.0, notes);
    clip.offset = 0.75;
    clip.source_rate = 0.5;
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let ghost = project.clone_clip(track, id).unwrap();
    let linked = project.clip_mut(track, ghost).unwrap();
    linked.start = 10.0;
    linked.offset = 0.5;
    linked.source_rate = 1.3;
    (project, track, id, ghost, ids)
}

#[test]
fn fixed_and_additive_lengths_use_audible_beats_without_losing_hidden_expression_or_ghost_bounds() {
    let (mut p, t, c, g, ids) = fixture();
    let original = p.clone();
    let selection = HashSet::from([ids[0], ids[1]]);
    let result = p
        .edit_midi_note_lengths(t, c, &selection, MidiNoteLengthChange::Set { length: 0.25 })
        .unwrap();
    assert_eq!(result.changed, 2);
    assert_eq!(result.applied_delta, None);
    for id in [c, g] {
        let now = p.clip(t, id).unwrap();
        let before = original.clip(t, id).unwrap();
        assert_eq!(
            (now.start, now.length, now.offset, now.source_rate),
            (
                before.start,
                before.length,
                before.offset,
                before.source_rate
            )
        );
        let after = p.midi_notes(t, id).unwrap();
        let before = original.midi_notes(t, id).unwrap();
        for (index, note) in after.iter().enumerate() {
            assert_eq!(note.start, before[index].start);
            assert_eq!(note.midi2, before[index].midi2);
            assert_eq!(note.velocity, before[index].velocity);
            assert_eq!(note.pitch, before[index].pitch);
            assert_eq!(
                note.length,
                if index < 2 {
                    0.125
                } else {
                    before[index].length
                }
            );
        }
    }
    let result = p
        .edit_midi_note_lengths(
            t,
            c,
            &selection,
            MidiNoteLengthChange::Add { delta: -100.0 },
        )
        .unwrap();
    assert!((result.applied_delta.unwrap() + 0.234375).abs() < 1e-12);
    assert!(
        p.midi_notes(t, c).unwrap()[0]
            .midi2
            .as_ref()
            .unwrap()
            .expressions
            .iter()
            .any(|point| point.beat > p.midi_notes(t, c).unwrap()[0].length)
    );
    p.validate().unwrap();
}

#[test]
fn group_shrink_preserves_length_differences_and_does_not_grow_imported_tiny_notes() {
    let (mut p, t, c, _, ids) = fixture();
    let selected = HashSet::from([ids[0], ids[1]]);
    let result = p
        .edit_midi_note_lengths(t, c, &selected, MidiNoteLengthChange::Add { delta: -1.0 })
        .unwrap();
    assert!((result.applied_delta.unwrap() + 0.484375).abs() < 1e-12);
    let after = p.midi_notes(t, c).unwrap();
    assert_eq!(after[1].length - after[0].length, 0.25);
    p.midi_notes_mut(t, c).unwrap()[0].length = 0.001;
    p.synchronize_linked_clip(t, c).unwrap();
    let before = p.clone();
    let result = p
        .edit_midi_note_lengths(t, c, &selected, MidiNoteLengthChange::Add { delta: -0.25 })
        .unwrap();
    assert_eq!(result.changed, 0);
    assert_eq!(result.applied_delta, Some(0.0));
    assert_eq!(p, before);
}

#[test]
fn invalid_lengths_missing_selections_and_source_overflow_are_atomic() {
    let (mut p, t, c, _, ids) = fixture();
    p.clip_mut(t, c).unwrap().source_rate = 4.0;
    let original = p.clone();
    for edit in [
        MidiNoteLengthChange::Set { length: f64::NAN },
        MidiNoteLengthChange::Set { length: 0.0 },
        MidiNoteLengthChange::Set {
            length: daw_core::MAX_BEAT,
        },
        MidiNoteLengthChange::Add {
            delta: f64::INFINITY,
        },
    ] {
        assert!(
            p.edit_midi_note_lengths(t, c, &HashSet::from([ids[0]]), edit)
                .is_err()
        );
        assert_eq!(p, original);
    }
    assert!(
        p.edit_midi_note_lengths(
            t,
            c,
            &HashSet::from([u64::MAX - 1]),
            MidiNoteLengthChange::Set { length: 1.0 }
        )
        .is_err()
    );
    assert_eq!(p, original);
    let result = p
        .edit_midi_note_lengths(
            t,
            c,
            &HashSet::from([ids[0], ids[1]]),
            MidiNoteLengthChange::Add { delta: 0.0 },
        )
        .unwrap();
    assert_eq!(result.changed, 0);
    assert_eq!(p, original);
}
