use super::*;

fn renderer(loop_frames: usize, latency: u32) -> (Renderer, u64) {
    let mut project = Project::new();
    project.tempo = 120.0;
    project.loop_enabled = true;
    project.loop_end = loop_frames as f64 / 4_000.0;
    let id = project.add_midi_track("Monitored input");
    // Unassigned instruments exercise render scheduling/state without a native
    // plug-in. Tests explicitly call collect_midi where processing would do so;
    // app E2Es verify the resulting events/audio with an isolated instrument.
    project.track_mut(id).unwrap().latency_samples = latency;
    (Renderer::new(&project, 8_000, true).unwrap(), id)
}

fn context(start: u64, count: usize) -> BlockContext {
    BlockContext {
        start,
        count,
        sample_rate: 8_000,
        tempo: 120.0,
        timeline: true,
        live_input: true,
        meter_decay: 1.0,
    }
}

fn edges(events: &[MidiMessage]) -> Vec<(u32, u8, bool)> {
    events
        .iter()
        .map(|event| (event.offset, event.pitch, event.on))
        .collect()
}

#[test]
fn internal_loop_wrap_keeps_live_chord_at_midblock_and_block_start() {
    for loop_frames in [128, BLOCK] {
        for latency in [0, 37] {
            let (mut renderer, id) = renderer(loop_frames, latency);
            let chord = [(id, 60, 100), (id, 64, 80)];
            renderer.set_midi_input_notes(&chord, 0).unwrap();
            let mut output = [[0.0; 2]; BLOCK];
            let mut positions = [0.0; BLOCK];
            for _ in 0..3 {
                assert_eq!(
                    crate::allocation_test_support::count_allocations(|| {
                        renderer.render(&mut output, &mut positions).unwrap();
                    }),
                    0
                );
                assert_eq!(renderer.midi_input_notes, chord);
            }
            renderer.set_midi_input_notes(&[], 0).unwrap();
            assert_eq!(
                edges(&renderer.tracks[0].as_ref().unwrap().live_midi),
                vec![(0, 60, false), (0, 64, false)],
                "Releasing the chord after a wrap must still release both remembered voices"
            );
        }
    }
}

#[test]
fn live_midi_edges_beyond_a_short_loop_segment_keep_their_exact_offsets() {
    let (mut renderer, _) = renderer(128, 0);
    let track = renderer.tracks[0].as_mut().unwrap();
    for (offset, pitch, on) in [
        (20, 60, true),
        (180, 60, false),
        (180, 64, true),
        (250, 64, false),
    ] {
        track.live_midi.push(MidiMessage {
            offset,
            pitch,
            velocity: if on { 100 } else { 0 },
            on,
        });
    }
    track.collect_midi(context(0, 128));
    assert_eq!(edges(&track.midi), vec![(20, 60, true)]);
    assert_eq!(
        edges(&track.live_midi),
        vec![(52, 60, false), (52, 64, true), (122, 64, false)]
    );
    track.collect_midi(context(0, 128));
    assert_eq!(
        edges(&track.midi),
        vec![(52, 60, false), (52, 64, true), (122, 64, false)]
    );
    assert!(track.live_midi.is_empty());
}

#[test]
fn loop_wrap_releases_only_sequencer_voices_and_defers_live_edges_during_priming() {
    for latency in [0, 37] {
        let (mut renderer, id) = renderer(128, latency);
        // With Instrument::None this remembers an already held chord without
        // queuing an initial NoteOn, allowing the pending release to be isolated.
        renderer.set_midi_input_notes(&[(id, 72, 100)], 0).unwrap();
        let track = renderer.tracks[0].as_mut().unwrap();
        let note = ScheduledNote {
            clip: 1,
            id: 2,
            start: 0,
            end: 256,
            pitch: 60,
            velocity: 90,
        };
        record_active_note(
            &mut track.active_notes,
            &mut track.active_indices,
            Event {
                sample: 0,
                note,
                on: true,
            },
        );
        track.live_midi.push(MidiMessage {
            offset: 80,
            pitch: 72,
            velocity: 0,
            on: false,
        });
        renderer.seek_sample(0, true).unwrap();
        assert_eq!(renderer.midi_input_notes, [(id, 72, 100)]);
        let track = renderer.tracks[0].as_mut().unwrap();
        assert_eq!(edges(&track.live_midi), [(80, 72, false)]);
        // Unassigned tracks skip MIDI processing during seek's priming. Drive
        // that same scheduler explicitly here, then the next audible segment.
        let mut prime = context(0, 37);
        prime.live_input = false;
        track.collect_midi(prime);
        assert_eq!(edges(&track.midi), [(0, 60, false)]);
        assert_eq!(edges(&track.live_midi), [(80, 72, false)]);
        track.collect_midi(context(0, 128));
        assert_eq!(edges(&track.midi), [(80, 72, false)]);
        assert!(track.active_notes.is_empty());

        // Explicit seeks retain their existing reset behavior. The render
        // worker will restore whichever keys are still physically held.
        renderer.seek(0.0).unwrap();
        assert!(renderer.midi_input_notes.is_empty());
        assert!(renderer.tracks[0].as_ref().unwrap().live_midi.is_empty());
    }
}
