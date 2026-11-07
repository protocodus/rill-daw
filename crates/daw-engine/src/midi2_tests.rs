use super::*;
use daw_core::{Midi2ChannelVoice, UmpPacket};
fn packet(status: u8, index: u8, detail: u8, data: u32) -> UmpPacket {
    Midi2ChannelVoice {
        group: 2,
        channel: 4,
        status,
        index,
        detail,
        data,
    }
    .packet()
    .unwrap()
}
fn recorded(events: &[(u64, UmpPacket)], frames: u64) -> MidiRecordingResult {
    assemble(
        Recording {
            routes: vec![MidiRoute {
                track: 1,
                group: 2,
                channel: Some(4),
                monitor: true,
            }],
            events: events
                .iter()
                .map(|(time, packet)| Event {
                    time: *time,
                    data: 0,
                    epoch: 1,
                    ump: Some(*packet),
                })
                .collect(),
            tempo: 120.0,
            loop_range: None,
        },
        frames,
        0,
        48000,
    )
}
#[test]
fn precise_capture_keeps_zero_velocity_pitch_attributes_and_actual_key_release_under_pedal() {
    let result = recorded(
        &[
            (0, packet(9, 72, 3, (32768 << 16) | (60 * 512 + 256))),
            (100, packet(11, 64, 0, u32::MAX)),
            (200, packet(10, 72, 0, 0x87654321)),
            (12000, packet(8, 72, 0, 50000 << 16)),
            (24000, packet(11, 64, 0, 0)),
            (26000, packet(9, 73, 0, 0)),
            (27000, packet(8, 73, 0, 0)),
        ],
        30000,
    );
    assert!(result.error.is_none(), "{:?}", result.error);
    let track = &result.tracks[0];
    assert_eq!(track.notes.len(), 2);
    let note = &track.notes[0];
    let data = note.midi2.as_ref().unwrap();
    assert_eq!(data.velocity, 32768);
    assert_eq!(data.release_velocity, 50000);
    assert_eq!((data.group, data.channel, data.note_index), (2, 4, 72));
    assert_eq!(note.pitch_semitones(), 60.5);
    assert_eq!(note.length, 0.5);
    assert_eq!(data.expressions[0].packet.words[1], 0x87654321);
    assert_eq!(track.midi_events.len(), 2);
    assert_eq!(track.notes[1].velocity16(), 0);
}
#[test]
fn setup_and_per_note_messages_attach_to_all_voices_until_detach() {
    let result = recorded(
        &[
            (0, packet(1, 60, 74, 123)),
            (100, packet(9, 60, 0, 30000 << 16)),
            (200, packet(9, 60, 0, 40000 << 16)),
            (300, packet(1, 60, 74, 456)),
            (400, packet(15, 60, 2, 0)),
            (500, packet(9, 60, 0, 50000 << 16)),
            (600, packet(1, 60, 74, 789)),
        ],
        1000,
    );
    assert!(result.error.is_none());
    let notes = &result.tracks[0].notes;
    assert_eq!(notes.len(), 3);
    for note in &notes[..2] {
        let points = &note.midi2.as_ref().unwrap().expressions;
        assert_eq!(points[0].beat, 0.0);
        assert_eq!(points[0].packet.words[1], 123);
        assert!(points.iter().any(|p| p.packet.words[1] == 456));
        assert!(!points.iter().any(|p| p.packet.words[1] == 789));
    }
    assert!(
        notes[2]
            .midi2
            .as_ref()
            .unwrap()
            .expressions
            .iter()
            .any(|p| p.packet.words[1] == 789)
    );
}
#[test]
fn legacy_channel_and_normalization_remain_explicit_with_captured_bend() {
    let legacy = |bytes: [u8; 3]| Event {
        time: 0,
        data: message(&bytes).unwrap(),
        epoch: 1,
        ump: None,
    };
    let mut off = legacy([0x84, 60, 0]);
    off.time = 100;
    let result = assemble(
        Recording {
            routes: vec![MidiRoute {
                track: 1,
                group: 0,
                channel: Some(4),
                monitor: true,
            }],
            events: vec![legacy([0x94, 60, 64]), legacy([0xe4, 12, 65]), off],
            tempo: 120.0,
            loop_range: None,
        },
        200,
        0,
        48000,
    );
    let track = &result.tracks[0];
    let note = &track.notes[0];
    assert_eq!(note.velocity, 64);
    let data = note.midi2.as_ref().unwrap();
    assert_eq!(data.protocol, 1);
    assert_eq!(data.channel, 4);
    assert_eq!(track.midi_events[0].packet.words[0], 0x20e40c41);
    let precise = crate::render::midi_expression::precise(data, note.velocity, 1);
    assert_eq!(precise.velocity, daw_plugins::ExpressiveVelocity::Midi1(64));
}
#[test]
fn global_only_capture_and_orphan_per_note_setup_are_not_silently_lost() {
    let result = recorded(&[(0, packet(11, 7, 0, 0x12345678))], 1000);
    assert!(result.error.is_none());
    assert!(result.tracks[0].notes.is_empty());
    assert_eq!(result.tracks[0].midi_events[0].packet.words[1], 0x12345678);
    let orphan = recorded(&[(0, packet(1, 60, 74, 123))], 1000);
    assert!(orphan.error.unwrap().contains("Unassociated"));
    let after_release = recorded(
        &[
            (0, packet(9, 60, 0, 40000 << 16)),
            (100, packet(8, 60, 0, 0)),
            (200, packet(1, 60, 74, 123)),
        ],
        1000,
    );
    assert!(after_release.error.unwrap().contains("Unassociated"));
    assert_eq!(after_release.tracks[0].notes.len(), 1);
}
#[test]
fn callback_ump_framing_rejects_truncation_and_keeps_bounded_prefix() {
    let shared = MidiShared::new();
    shared.ready.store(true, Ordering::Release);
    shared.enqueue_ump_words(&[0x42943c00, 32768 << 16, 0x42943d00], 100, 1);
    assert_eq!(shared.fault.load(Ordering::Acquire), 6);
    let event = shared.incoming.pop().unwrap();
    assert_eq!(event.packet().words[1], 32768 << 16);
    assert!(shared.incoming.pop().is_none());
}
#[test]
fn loop_capture_segments_have_disjoint_expression_windows_with_carried_state() {
    let mut events = vec![Event {
        time: 0,
        data: 0,
        epoch: 1,
        ump: Some(packet(9, 60, 0, 40000 << 16)),
    }];
    for index in 0..100 {
        events.push(Event {
            time: index * 100 + 50,
            data: 0,
            epoch: 1,
            ump: Some(packet(1, 60, 74, index as u32)),
        });
    }
    events.push(Event {
        time: 9999,
        data: 0,
        epoch: 1,
        ump: Some(packet(8, 60, 0, 30000 << 16)),
    });
    let result = assemble(
        Recording {
            routes: vec![MidiRoute {
                track: 1,
                group: 2,
                channel: Some(4),
                monitor: true,
            }],
            events,
            tempo: 120.0,
            loop_range: Some((0, 100)),
        },
        10000,
        0,
        48000,
    );
    assert!(result.error.is_none(), "{:?}", result.error);
    let notes = &result.tracks[0].notes;
    assert_eq!(notes.len(), 100);
    for note in notes {
        let points = &note.midi2.as_ref().unwrap().expressions;
        assert!(points.len() <= 2);
        assert!(points.iter().all(|point| point.beat < note.length));
    }
}
