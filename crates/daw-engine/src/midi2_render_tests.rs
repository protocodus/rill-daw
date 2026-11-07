//! Real native VST precision, seek/chase, trim, controller reset and group policy.
use super::multi_io_master_tests::fixture;
use super::*;
use daw_core::{Clip, Midi2ChannelVoice, Midi2NoteData, MidiClipEvent, MidiNoteExpression, Note};
use fixture::{CLASS_ID, EditorProbeFixture};
fn packet(
    group: u8,
    channel: u8,
    status: u8,
    index: u8,
    detail: u8,
    data: u32,
) -> daw_core::UmpPacket {
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
fn note(group: u8, channel: u8, index: u8, velocity: u16, length: f64) -> Note {
    let mut note = Note::new(index, 0.0, length, 64);
    note.set_midi2(
        Midi2NoteData::from_note_on(
            packet(
                group,
                channel,
                9,
                index,
                3,
                (u32::from(velocity) << 16) | (60 * 512 + 256),
            )
            .midi2_channel_voice()
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    note
}
fn project(fixture: &EditorProbeFixture, notes: Vec<Note>) -> (Project, u64) {
    let mut project = Project::new();
    let id = project.add_midi_track("Precise native");
    let track = project.track_mut(id).unwrap();
    track.gain = 1.0;
    track.instrument = Instrument::Vst3 {
        path: fixture.path.to_string_lossy().into(),
        class_id: Some(CLASS_ID.into()),
    };
    track.clips.push(Clip::midi("Performance", 0.0, 3.0, notes));
    (project, id)
}
fn render(renderer: &mut Renderer, count: usize) -> Vec<[f32; 2]> {
    let mut result = vec![[0.0; 2]; count];
    let mut positions = vec![0.0; count];
    renderer.render(&mut result, &mut positions).unwrap();
    result
}
fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}
#[test]
fn native_precise_overlapping_pitch_expression_and_held_note_seek_chase() {
    let fixture = EditorProbeFixture::build_expressive();
    let mut first = note(2, 4, 72, 32768, 1.0);
    first
        .midi2
        .as_mut()
        .unwrap()
        .expressions
        .push(MidiNoteExpression::new(
            0.25,
            packet(2, 4, 1, 72, 74, 0x80000001),
        ));
    let second = note(2, 4, 73, 32769, 2.0);
    let (project, _) = project(&fixture, vec![first, second]);
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    let full = (32768.0_f32 + 32769.0) / 65535.0 * 0.5;
    for frame in render(&mut renderer, 256) {
        near(frame[0], full);
        near(frame[1], 0.0);
    }
    renderer.seek(0.5).unwrap();
    let expected = (32768.0_f32 / 65535.0 * 0.5 + 32769.0_f32 / 65535.0) * 0.5;
    for frame in render(&mut renderer, 256) {
        near(frame[0], expected);
    }
    renderer.seek(1.5).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 32769.0_f32 / 65535.0 * 0.5);
    }
}
#[test]
fn native_legacy_channel_five_bend_keeps_seven_bit_attack_and_seek_restores_baseline() {
    let fixture = EditorProbeFixture::build_expressive();
    let mut legacy = Note::new(60, 0.0, 2.0, 64);
    let mut meta = Midi2NoteData::from_legacy(&legacy);
    meta.channel = 4;
    legacy.set_midi2(meta).unwrap();
    let (mut project, id) = project(&fixture, vec![legacy]);
    let clip = &mut project.track_mut(id).unwrap().clips[0];
    clip.midi_events.push(MidiClipEvent::new(
        0.25,
        daw_core::UmpPacket::from_words(&[0x20e40060]).unwrap(),
    ));
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 64.0 / 127.0 * 0.5);
    }
    renderer.seek(0.5).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 64.0 / 127.0 * (12288.0 / 16383.0));
    }
    renderer.seek(0.0).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 64.0 / 127.0 * 0.5);
    }
}
#[test]
fn dense_trimmed_curves_carry_only_latest_state_and_hidden_points_do_not_play() {
    let fixture = EditorProbeFixture::build_expressive();
    let mut rich = note(0, 0, 60, 65535, 10.0);
    for index in 0..5000 {
        rich.midi2
            .as_mut()
            .unwrap()
            .expressions
            .push(MidiNoteExpression::new(
                index as f64 / 1000.0,
                packet(0, 0, 1, 60, 74, 0x80000000),
            ));
    }
    rich.midi2
        .as_mut()
        .unwrap()
        .expressions
        .push(MidiNoteExpression::new(9.5, packet(0, 0, 1, 60, 74, 0)));
    let (mut project, id) = project(&fixture, vec![rich]);
    let clip = &mut project.track_mut(id).unwrap().clips[0];
    clip.offset = 8.0;
    clip.length = 1.0;
    for index in 0..5000 {
        clip.midi_events.push(MidiClipEvent::new(
            index as f64 / 1000.0,
            packet(0, 0, 11, 7, 0, 0x80000000),
        ));
    }
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    let track = renderer
        .tracks
        .iter()
        .flatten()
        .find(|track| track.id == id)
        .unwrap();
    assert_eq!(
        track.expression.content.events.len(),
        2,
        "Dense hidden histories collapse to their last state"
    );
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.25);
    }
    assert_eq!(renderer.plugin_errors, 0);
    renderer.seek(0.5).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.25);
    }
}
#[test]
fn stored_group_is_independent_of_input_and_extra_groups_are_retained_with_advisory() {
    let fixture = EditorProbeFixture::build_expressive();
    let (first, second) = (note(3, 0, 60, 65535, 2.0), note(5, 1, 61, 65535, 2.0));
    let (mut project, id) = project(&fixture, vec![first, second]);
    project.track_mut(id).unwrap().midi_input_group = 10;
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.5);
        near(frame[1], 0.0);
    }
    assert_eq!(renderer.unsupported_midi(), (0, 1));
    assert!(
        matches!(&project.track(id).unwrap().clips[0].content,ClipContent::Midi{notes}if notes.len()==2)
    );
}
#[test]
fn held_live_controller_wins_seek_chase_and_disappearing_input_restores_baseline() {
    use daw_plugins::{
        ExpressiveController as Controller, ExpressiveMidiData as Data,
        ExpressiveVelocity as Velocity,
    };
    let fixture = EditorProbeFixture::build_expressive();
    let (project, id) = project(&fixture, vec![]);
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    let on = Data::NoteOn {
        channel: 0,
        pitch: 60,
        velocity: Velocity::Midi2(65535),
        note_id: 0x40000001,
        tuning: 0.0,
    };
    let control = Data::Controller {
        channel: 0,
        controller: Controller::ControlChange(7),
        value: 0.8,
    };
    let state = [(id, 0, 0, on), (id, 0, 0, control)];
    renderer.set_expressive_input(&state, &[], 0).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.8);
    }
    renderer.seek(0.0).unwrap();
    renderer.set_expressive_input(&state, &[], 0).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.8);
    }
    // Different channel addresses may intentionally map to one native parameter.
    let alias = Data::Controller {
        channel: 1,
        controller: Controller::ControlChange(7),
        value: 0.6,
    };
    renderer
        .set_expressive_input(
            &[(id, 0, 0, on), (id, 0, 0, control), (id, 0, 1, alias)],
            &[],
            0,
        )
        .unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.6);
    }
    renderer
        .set_expressive_input(&[(id, 0, 0, on), (id, 0, 1, alias)], &[], 0)
        .unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.6);
    }
    renderer.set_expressive_input(&[], &[], 0).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.0);
    }
    renderer
        .set_expressive_input(&[(id, 0, 0, on)], &[], 0)
        .unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.5);
    }
}
#[test]
fn changing_stored_group_releases_a_previously_admitted_live_voice() {
    use daw_plugins::{ExpressiveMidiData as Data, ExpressiveVelocity as Velocity};
    let fixture = EditorProbeFixture::build_expressive();
    let (mut project, id) = project(&fixture, vec![]);
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    let on = Data::NoteOn {
        channel: 0,
        pitch: 60,
        velocity: Velocity::Midi2(65535),
        note_id: 0x40000001,
        tuning: 0.0,
    };
    let state = [(id, 0, 0, on)];
    renderer.set_expressive_input(&state, &[], 0).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.5);
    }
    let mut rich = note(1, 1, 61, 65535, 1.0);
    rich.start = 1.0;
    if let ClipContent::Midi { notes } = &mut project.track_mut(id).unwrap().clips[0].content {
        notes.push(rich);
    }
    let mut preparer = renderer.live_edit_preparer();
    let mut edit = preparer.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut edit).unwrap();
    renderer.set_expressive_input(&state, &[], 0).unwrap();
    for frame in render(&mut renderer, 256) {
        near(frame[0], 0.0);
        near(frame[1], 0.0);
    }
    assert_eq!(renderer.unsupported_midi().1, 1);
}

#[test]
fn rapid_legacy_monitor_retriggers_keep_native_id_release_order_in_preview_and_timeline() {
    use daw_plugins::{ExpressiveMidiData as Data, ExpressiveVelocity as Velocity};
    let fixture = EditorProbeFixture::build_expressive();
    for preview in [false, true] {
        let (project, id) = project(&fixture, vec![]);
        let mut renderer = Renderer::new(&project, 8000, false).unwrap();
        renderer.set_preview(preview, 0.0).unwrap();
        let on = |pitch, note_id| Data::NoteOn {
            channel: 0,
            pitch,
            velocity: Velocity::Midi1(100),
            note_id,
            tuning: 0.0,
        };
        // Match a monitor poll containing a short note and two rapid presses.
        // The final edges intentionally share a sample, as coalesced input can.
        renderer
            .set_expressive_input(&[(id, 0, 0, on(67, 0x40000001))], &[], 0)
            .unwrap();
        renderer.set_expressive_input(&[], &[], 128).unwrap();
        renderer
            .set_expressive_input(&[(id, 0, 0, on(69, 0x40000002))], &[], 255)
            .unwrap();
        renderer
            .set_expressive_input(&[(id, 0, 0, on(69, 0x40000003))], &[], 255)
            .unwrap();
        renderer.set_expressive_input(&[], &[], 255).unwrap();
        let mut frames = [[0.0; 2]; 256];
        let mut positions = [0.0; 256];
        if preview {
            renderer
                .render_preview(&mut frames, &mut positions)
                .unwrap();
        } else {
            renderer.render(&mut frames, &mut positions).unwrap();
        }
        for frame in &frames[..128] {
            near(frame[0], 100.0 / 127.0 * 0.5);
        }
        for frame in &frames[128..] {
            near(frame[0], 0.0);
            near(frame[1], 0.0);
        }
        if preview {
            renderer
                .render_preview(&mut frames, &mut positions)
                .unwrap();
        } else {
            renderer.render(&mut frames, &mut positions).unwrap();
        }
        assert!(
            frames.iter().flatten().all(|sample| *sample == 0.0),
            "Native voices must remain released in preview={preview}"
        );
        assert!(renderer.expressive_input.is_empty());
        // Reuse a retired native ID: both adapter ownership and the native DSP
        // voice must have been released, rather than merely hidden in the UI.
        renderer
            .set_expressive_input(&[(id, 0, 0, on(69, 0x40000002))], &[], 0)
            .unwrap();
        renderer.set_expressive_input(&[], &[], 1).unwrap();
        if preview {
            renderer
                .render_preview(&mut frames, &mut positions)
                .unwrap();
        } else {
            renderer.render(&mut frames, &mut positions).unwrap();
        }
        near(frames[0][0], 100.0 / 127.0 * 0.5);
        assert!(frames[1..].iter().flatten().all(|sample| *sample == 0.0));
        assert_eq!(renderer.plugin_errors, 0);
    }
}
