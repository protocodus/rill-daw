use super::*;
use daw_core::{Clip, Note, PluginParameterValue};

fn audio_project() -> (Project, tempfile::TempDir, u64) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 2,
        samples: (0..16_000).flat_map(|_| [0.5, 0.25]).collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let id = project.add_audio_track("Continuous audio");
    project.track_mut(id).unwrap().clips.push(Clip::audio(
        "Source",
        0.0,
        4.0,
        path.to_string_lossy(),
    ));
    (project, directory, id)
}

fn render_block(renderer: &mut Renderer) -> [[f32; 2]; BLOCK] {
    let mut audio = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    renderer.render(&mut audio, &mut positions).unwrap();
    audio
}

fn midi_tick(renderer: &mut Renderer) -> Vec<(u8, bool, u32)> {
    // Unassigned test track uses the exact scheduling/voice bookkeeping called
    // before VST processing. Real helper identity/audio is covered by app E2Es.
    let context = BlockContext {
        start: renderer.source_sample,
        count: BLOCK,
        sample_rate: renderer.sample_rate,
        tempo: renderer.tempo,
        timeline: true,
        meter_decay: 1.0,
    };
    let track = renderer.tracks[0].as_mut().unwrap();
    track.collect_midi(context);
    let result = track
        .midi
        .iter()
        .map(|event| (event.pitch, event.on, event.offset))
        .collect();
    render_block(renderer);
    result
}

#[test]
fn live_mixer_edit_preserves_parallel_graph_delay_buffers_and_position_without_allocating() {
    let (mut project, _directory, id) = audio_project();
    let copy = project.track(id).unwrap().clips[0].clone();
    let other = project.add_audio_track("Parallel source");
    project.track_mut(other).unwrap().clips.push(copy);
    // Give the clone a distinct identity as required by project validation.
    project.track_mut(other).unwrap().clips[0].id = daw_core::new_id();
    project.track_mut(other).unwrap().latency_samples = 37;
    let mut renderer = Renderer::new_with_workers(&project, 8_000, true, 2).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    render_block(&mut renderer);
    let source = renderer.source_sample;
    let logical = renderer.logical_sample;
    let worker = renderer.workers.as_ref().unwrap() as *const WorkerPool;
    let delayed = renderer
        .tracks
        .iter()
        .flatten()
        .find(|track| track.id == id)
        .unwrap();
    let route_buffer = delayed.routes[0].delay.frames.as_ptr();
    let delay_cursor = delayed.routes[0].delay.cursor;
    let audio = Arc::clone(&delayed.audio);
    project.track_mut(id).unwrap().gain = 0.25;
    project.track_mut(id).unwrap().pan = 0.5;
    let mut edit = prepare.prepare(&project).unwrap();
    let allocations = crate::allocation_test_support::count_allocations(|| {
        renderer.apply_live_edit(&mut edit).unwrap()
    });
    assert_eq!(allocations, 0, "Applying prepared edits is allocation-free");
    assert_eq!(
        (renderer.logical_sample, renderer.source_sample),
        (logical, source)
    );
    assert_eq!(
        renderer.workers.as_ref().unwrap() as *const WorkerPool,
        worker
    );
    let delayed = renderer
        .tracks
        .iter()
        .flatten()
        .find(|track| track.id == id)
        .unwrap();
    assert!(Arc::ptr_eq(&audio, &delayed.audio));
    assert_eq!(delayed.routes[0].delay.frames.as_ptr(), route_buffer);
    assert_eq!(delayed.routes[0].delay.cursor, delay_cursor);
    let block = render_block(&mut renderer);
    assert!(block.iter().all(|frame| frame[0] > 0.0 && frame[1] > 0.0));
    // The 5 ms manual ramp settles and the compensated path then exposes the
    // new mix, while the unchanged parallel source keeps its exact level.
    let block = render_block(&mut renderer);
    let a = project.track(id).unwrap();
    let b = project.track(other).unwrap();
    let expected = [
        0.5 * a.gain * (1.0 - a.pan) + 0.5 * b.gain,
        0.25 * a.gain + 0.25 * b.gain,
    ];
    for frame in block {
        assert!((frame[0] - expected[0]).abs() < 1e-6 && (frame[1] - expected[1]).abs() < 1e-6);
    }
}

#[test]
fn live_notes_keep_sustain_across_length_and_split_edits_and_reconcile_pending_changes() {
    let mut project = Project::new();
    project.tempo = 120.0;
    let id = project.add_midi_track("Held voice");
    let clip = Clip::midi("Phrase", 0.0, 4.0, vec![Note::new(60, 0.0, 4.0, 100)]);
    let clip_id = clip.id;
    project.track_mut(id).unwrap().clips.push(clip);
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    assert_eq!(midi_tick(&mut renderer), vec![(60, true, 0)]);
    if let ClipContent::Midi { notes } = &mut project.track_mut(id).unwrap().clips[0].content {
        notes[0].length = 8.0;
    }
    project.track_mut(id).unwrap().clips[0].length = 8.0;
    let mut edit = prepare.prepare(&project).unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| renderer
            .apply_live_edit(&mut edit)
            .unwrap()),
        0
    );
    assert!(
        midi_tick(&mut renderer).is_empty(),
        "Extending a held note must not retrigger"
    );
    let right = project.split_clip(id, clip_id, 0.05).unwrap();
    let mut edit = prepare.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut edit).unwrap();
    assert!(
        midi_tick(&mut renderer).is_empty(),
        "Splitting before the playhead preserves the sounding pitch despite new IDs"
    );
    if let ClipContent::Midi { notes } = &mut project.clip_mut(id, right).unwrap().content {
        notes[0].pitch = 62;
    }
    let mut edit = prepare.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut edit).unwrap();
    assert_eq!(
        midi_tick(&mut renderer),
        vec![(60, false, 0), (62, true, 0)]
    );
    project.track_mut(id).unwrap().clips.clear();
    let mut remove = prepare.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut remove).unwrap();
    // Replace again before another block. The intermediate removal has not sent
    // its note-off yet, so final reconciliation must still release the old voice.
    project.track_mut(id).unwrap().clips.push(Clip::midi(
        "Replacement",
        0.0,
        8.0,
        vec![Note::new(64, 0.0, 8.0, 90)],
    ));
    let mut replace = prepare.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut replace).unwrap();
    assert_eq!(
        midi_tick(&mut renderer),
        vec![(62, false, 0), (64, true, 0)]
    );
    project.track_mut(id).unwrap().clips.clear();
    let mut edit = prepare.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut edit).unwrap();
    assert_eq!(midi_tick(&mut renderer), vec![(64, false, 0)]);
    assert!(renderer.tracks[0].as_ref().unwrap().active_notes.is_empty());
}

#[test]
fn live_audio_edits_use_cached_sources_and_new_assets_without_restarting_the_timeline() {
    let (mut project, directory, id) = audio_project();
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    render_block(&mut renderer);
    std::fs::remove_file(directory.path().join("source.wav")).unwrap();
    project.track_mut(id).unwrap().clips[0].length = 2.0;
    let mut trim = prepare
        .prepare(&project)
        .expect("Existing decoded source remains usable after file removal");
    renderer.apply_live_edit(&mut trim).unwrap();
    assert!(
        render_block(&mut renderer)
            .iter()
            .all(|frame| frame[0] > 0.0)
    );
    let path = directory.path().join("new.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![0.2; 16_000],
    }
    .write_wav(&path)
    .unwrap();
    project.track_mut(id).unwrap().clips =
        vec![Clip::audio("New source", 0.0, 4.0, path.to_string_lossy())];
    let mut add = prepare.prepare(&project).unwrap();
    let position = renderer.logical_sample;
    renderer.apply_live_edit(&mut add).unwrap();
    assert_eq!(renderer.logical_sample, position);
    let output = render_block(&mut renderer);
    for frame in output {
        assert!((frame[0] - 0.2 * project.track(id).unwrap().gain).abs() < 1e-6);
    }
}

#[test]
fn live_update_reverts_superseded_preparations_and_rejects_foreign_graph_before_mutation() {
    let (project, _directory, id) = audio_project();
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    let mut changed = project.clone();
    changed.track_mut(id).unwrap().clips.clear();
    let _superseded = prepare.prepare(&changed).unwrap();
    let mut revert = prepare.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut revert).unwrap();
    assert!(
        render_block(&mut renderer)
            .iter()
            .all(|frame| frame[0] > 0.0)
    );
    let mut other = Renderer::new(&project, 8_000, true).unwrap();
    let before = other.logical_sample;
    let gain = other.tracks[0].as_ref().unwrap().gain.manual;
    let audio = Arc::clone(&other.tracks[0].as_ref().unwrap().audio);
    assert!(
        other
            .apply_live_edit(&mut prepare.prepare(&changed).unwrap())
            .is_err()
    );
    assert_eq!(other.logical_sample, before);
    assert_eq!(other.tracks[0].as_ref().unwrap().gain.manual, gain);
    assert!(Arc::ptr_eq(
        &audio,
        &other.tracks[0].as_ref().unwrap().audio
    ));
    changed.track_mut(id).unwrap().mute = true;
    assert!(!prepare.compatible(&changed));
    changed = project.clone();
    changed.tempo += 1.0;
    assert!(!prepare.compatible(&changed));
}

#[test]
fn accepted_live_parameters_are_part_of_compatibility_but_undo_is_not_ignored() {
    let (project, _directory, id) = audio_project();
    let renderer = Renderer::new(&project, 8_000, true).unwrap();
    let prepare = renderer.live_edit_preparer();
    let checker = prepare.compatibility();
    let mut changed = project.clone();
    changed
        .track_mut(id)
        .unwrap()
        .instrument_parameters
        .push(PluginParameterValue { id: 0, value: 0.75 });
    assert!(!checker.compatible(&changed));
    checker.acknowledge_parameter(id, None, 0, 0.75);
    assert!(checker.compatible(&changed));
    assert!(
        !checker.compatible(&project),
        "Undoing the parameter must use the state-changing path"
    );
}

#[test]
fn live_clip_edit_keeps_loop_timing_and_compensated_audio_running() {
    let (mut project, _directory, id) = audio_project();
    project.loop_enabled = true;
    project.loop_start = 0.0;
    project.loop_end = 0.125;
    project.track_mut(id).unwrap().latency_samples = 73;
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    render_block(&mut renderer);
    let before = (renderer.logical_sample, renderer.source_sample);
    project.track_mut(id).unwrap().clips[0].length = 1.0;
    let mut edit = prepare.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut edit).unwrap();
    assert_eq!((renderer.logical_sample, renderer.source_sample), before);
    let mut wrapped = false;
    let mut previous = renderer.logical_sample;
    for _ in 0..8 {
        let output = render_block(&mut renderer);
        assert!(output.iter().all(|frame| frame[0] > 0.0));
        wrapped |= renderer.logical_sample < previous;
        previous = renderer.logical_sample;
        assert!(!renderer.finished());
    }
    assert!(wrapped, "Live edit retains the transport loop");
}

#[test]
fn deleting_last_clip_delivers_note_off_and_runs_out_tail_before_finishing() {
    let mut project = Project::new();
    project.tempo = 120.0;
    let id = project.add_midi_track("Release");
    project.track_mut(id).unwrap().clips.push(Clip::midi(
        "Held",
        0.0,
        8.0,
        vec![Note::new(60, 0.0, 8.0, 100)],
    ));
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    midi_tick(&mut renderer);
    for _ in 0..16 {
        midi_tick(&mut renderer);
    }
    project.track_mut(id).unwrap().clips.clear();
    let mut edit = prepare.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut edit).unwrap();
    assert!(
        !renderer.finished(),
        "New nominal end cannot discard a pending note-off"
    );
    assert_eq!(midi_tick(&mut renderer), vec![(60, false, 0)]);
    for _ in 0..8 {
        render_block(&mut renderer);
    }
    assert!(renderer.finished(), "Release window is bounded");
    renderer.seek(0.0).unwrap();
    assert_eq!(
        renderer.end_sample, 1_600,
        "Explicit seek restores the empty project's normal 200 ms tail"
    );
}

#[test]
fn dense_live_note_edits_preserve_pitch_counts_and_apply_without_allocating() {
    let mut project = Project::new();
    project.tempo = 120.0;
    let id = project.add_midi_track("Dense sustained notes");
    project.track_mut(id).unwrap().clips.push(Clip::midi(
        "Dense",
        0.0,
        8.0,
        (0..8_192)
            .map(|index| Note::new((index % 128) as u8, 0.0, 8.0, 100))
            .collect(),
    ));
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    assert_eq!(midi_tick(&mut renderer).len(), 8_192);
    // Model a join/split that changes identities while keeping sounding pitches.
    let clip = &mut project.track_mut(id).unwrap().clips[0];
    clip.id = daw_core::new_id();
    if let ClipContent::Midi { notes } = &mut clip.content {
        for note in notes {
            note.id = daw_core::new_id();
        }
    }
    let mut remap = prepare.prepare(&project).unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| renderer
            .apply_live_edit(&mut remap)
            .unwrap()),
        0
    );
    assert!(
        midi_tick(&mut renderer).is_empty(),
        "Identity remap must not rearticulate existing pitches"
    );
    if let ClipContent::Midi { notes } = &mut project.track_mut(id).unwrap().clips[0].content {
        notes.retain(|note| note.pitch % 2 == 0);
    }
    let mut remove = prepare.prepare(&project).unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| renderer
            .apply_live_edit(&mut remove)
            .unwrap()),
        0
    );
    let context = BlockContext {
        start: renderer.source_sample,
        count: BLOCK,
        sample_rate: 8_000,
        tempo: 120.0,
        timeline: true,
        meter_decay: 1.0,
    };
    let track = renderer.tracks[0].as_mut().unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| track.collect_midi(context)),
        0
    );
    assert_eq!(track.midi.len(), 4_096);
    assert!(
        track
            .midi
            .iter()
            .all(|event| !event.on && event.pitch % 2 == 1)
    );
    assert_eq!(track.active_notes.len(), 4_096);
    assert_eq!(track.active_indices.len(), 4_096);
}

#[test]
fn dense_pitch_replacement_keeps_note_off_and_note_on_bookkeeping_allocation_free() {
    let mut project = Project::new();
    let id = project.add_midi_track("Dense pitch replacement");
    project.track_mut(id).unwrap().clips.push(Clip::midi(
        "Held",
        0.0,
        8.0,
        (0..8_192).map(|_| Note::new(60, 0.0, 8.0, 100)).collect(),
    ));
    let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
    let mut prepare = renderer.live_edit_preparer();
    assert_eq!(midi_tick(&mut renderer).len(), 8_192);
    if let ClipContent::Midi { notes } = &mut project.track_mut(id).unwrap().clips[0].content {
        for note in notes {
            note.pitch = 61;
        }
    }
    let mut update = prepare.prepare(&project).unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| renderer
            .apply_live_edit(&mut update)
            .unwrap()),
        0
    );
    let context = BlockContext {
        start: renderer.source_sample,
        count: BLOCK,
        sample_rate: 8_000,
        tempo: project.tempo,
        timeline: true,
        meter_decay: 1.0,
    };
    let track = renderer.tracks[0].as_mut().unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| track.collect_midi(context)),
        0
    );
    assert_eq!(track.midi.len(), 16_384);
    assert!(
        track.midi[..8_192]
            .iter()
            .all(|event| !event.on && event.pitch == 60)
    );
    assert!(
        track.midi[8_192..]
            .iter()
            .all(|event| event.on && event.pitch == 61)
    );
    assert_eq!(track.active_notes.len(), 8_192);
    assert_eq!(track.active_indices.len(), 8_192);
}
