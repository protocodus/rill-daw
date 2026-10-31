use super::*;
use daw_core::{AudioData, Clip};

#[test]
fn output_device_failure_retains_backend_cause_once_without_plugin_or_midi_recording_errors() {
    let mut engine = AudioEngine::start_virtual_cancellable(
        &Project::new(),
        DeviceSettings::default(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    engine.set_playing(true);
    let error = cpal::Error::with_message(
        cpal::ErrorKind::DeviceNotAvailable,
        "Output Ω disconnected: driver code 42 🎧",
    );
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| {
            for _ in 0..2048 {
                output_stream_failed(&engine.controls, &error);
            }
        }),
        0
    );
    let state = engine.snapshot();
    assert!(state.output_failed);
    assert!(!state.playing);
    assert_eq!(state.render_errors, 0);
    assert!(engine.take_render_error().is_none());
    assert!(engine.take_midi_error().is_none());
    let diagnostic = engine.take_input_error().unwrap();
    assert!(diagnostic.contains("Audio output stream failed"));
    assert!(diagnostic.contains("Output Ω disconnected: driver code 42 🎧"));
    assert!(diagnostic.contains("Retry playback or press Play"));
    assert!(engine.take_input_error().is_none());
    // A failed stream cannot be made healthy by merely setting playing=true,
    // including the engine-ready handoff racing an early device failure.
    engine.set_playing(true);
    assert!(!engine.snapshot().playing);
    output_stream_failed(&engine.controls, &error);
    assert!(engine.take_input_error().is_none());
    assert!(engine.take_midi_error().is_none());
    assert!(
        engine
            .pump_virtual(256)
            .unwrap()
            .iter()
            .all(|s| *s == [0.0; 2])
    );
}

#[test]
fn output_advisory_does_not_latch_or_stop_playback_but_a_real_fault_does() {
    let controls = Controls::new();
    controls.playing.store(true, Ordering::Release);
    for kind in [
        cpal::ErrorKind::RealtimeDenied,
        cpal::ErrorKind::DeviceChanged,
    ] {
        output_stream_failed(&controls, &cpal::Error::new(kind));
    }
    assert!(!controls.output_failed.load(Ordering::Acquire));
    assert!(controls.playing.load(Ordering::Acquire));
    output_stream_failed(&controls, &cpal::Error::new(cpal::ErrorKind::BackendError));
    assert!(controls.output_failed.load(Ordering::Acquire));
    assert!(!controls.playing.load(Ordering::Acquire));
    assert_eq!(controls.render_errors.load(Ordering::Relaxed), 0);
}

#[test]
fn unclassified_backend_and_other_errors_remain_fatal() {
    for kind in [
        cpal::ErrorKind::BackendError,
        cpal::ErrorKind::Other,
        cpal::ErrorKind::StreamInvalidated,
    ] {
        let controls = Controls::new();
        controls.playing.store(true, Ordering::Release);
        output_stream_failed(&controls, &cpal::Error::new(kind));
        assert!(controls.output_failed.load(Ordering::Acquire));
        assert!(!controls.playing.load(Ordering::Acquire));
        assert_eq!(controls.render_errors.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn output_failure_between_transport_check_and_publication_cannot_resurrect_playback() {
    let engine = AudioEngine::start_virtual_cancellable(
        &Project::new(),
        DeviceSettings::default(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let (entered, wait_for_check) = mpsc::channel();
    let (release, released) = mpsc::channel();
    PLAYING_PUBLICATION_TEST_GATE.with(|gate| *gate.borrow_mut() = Some((entered, released)));
    let controls = Arc::clone(&engine.controls);
    let failure = thread::spawn(move || {
        wait_for_check.recv_timeout(Duration::from_secs(5)).unwrap();
        output_stream_failed(
            &controls,
            &cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable),
        );
        release.send(()).unwrap();
    });
    engine.set_playing(true);
    failure.join().unwrap();
    assert!(engine.snapshot().output_failed);
    assert!(!engine.snapshot().playing);
    assert!(!engine.controls.playing.load(Ordering::Acquire));
}

#[test]
fn failed_output_rejects_audio_midi_and_audition_without_clearing_its_cause() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio input");
    let midi = project.add_midi_track("MIDI input");
    let mut engine = AudioEngine::start_virtual_cancellable(
        &project,
        DeviceSettings::default(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    engine
        .configure_input(
            &project,
            InputSettings {
                device_name: None,
                routes: vec![InputRoute {
                    track: audio,
                    channels: daw_core::AudioInputChannels::Mono { channel: 0 },
                    monitor: true,
                }],
            },
        )
        .unwrap();
    engine.fail_virtual_output().unwrap();
    // Reopening MIDI inputs is allowed, but must not reopen the failed output.
    engine
        .configure_midi(
            &project,
            MidiSettings {
                port_id: Some("rill-virtual-midi".into()),
                routes: vec![MidiRoute {
                    track: midi,
                    channel: None,
                    monitor: true,
                }],
            },
        )
        .unwrap();
    assert!(engine.recording_snapshot().input_ready);
    assert!(engine.midi_snapshot().input_ready);
    for error in [
        engine
            .start_recording(RecordingConfig {
                directory: directory.path().join("takes"),
                tracks: vec![audio],
            })
            .unwrap_err(),
        engine.start_midi_recording(vec![midi]).unwrap_err(),
        engine.audition_note(Some((midi, 60, 100))).unwrap_err(),
    ] {
        assert!(error.to_string().contains("press Play to reconnect"));
    }
    assert!(!engine.recording_snapshot().active);
    assert!(!engine.recording_snapshot().preparing);
    assert!(!engine.midi_snapshot().active);
    assert!(!directory.path().join("takes").exists());
    assert!(
        engine
            .take_input_error()
            .unwrap()
            .contains("Virtual output disconnected")
    );
    assert!(engine.take_input_error().is_none());
    engine.audition_note(None).unwrap();
    assert!(
        engine
            .pump_virtual(256)
            .unwrap()
            .iter()
            .all(|frame| *frame == [0.0; 2])
    );
}

#[test]
fn terminal_callback_cannot_emit_queued_preview_or_stale_play_requests() {
    let controls = Controls::new();
    let queue = AudioQueue::new(256);
    for frame in 0..256 {
        queue.push([0.6, -0.2], frame as f64, 0);
    }
    output_stream_failed(
        &controls,
        &cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable),
    );
    // A count-in/capture publication already in flight must not override the
    // terminal latch, even when the queue still contains real signal.
    controls.playing.store(true, Ordering::Release);
    controls.auditioning.store(true, Ordering::Release);
    let mut output = [1.0_f32; 512];
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| {
            fill_output(&mut output, 2, &queue, &controls);
        }),
        0
    );
    assert!(output.iter().all(|sample| *sample == 0.0));
    assert_eq!(
        queue.free(),
        0,
        "Failed stream must not consume the queued audio"
    );
    assert_eq!(controls.consumed_frames.load(Ordering::Relaxed), 0);
    assert!(!controls.playing.load(Ordering::Acquire));
    assert!(!controls.auditioning.load(Ordering::Acquire));
}

#[test]
fn output_xruns_keep_real_pcm_playing_and_count_once_per_callback_without_allocating() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000).flat_map(|_| [0.4, 0.2]).collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    let track = project.add_audio_track("Source");
    project.track_mut(track).unwrap().gain = 1.0;
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Source",
        0.0,
        2.0,
        path.to_string_lossy(),
    ));
    let mut engine = AudioEngine::start_virtual_cancellable(
        &project,
        DeviceSettings::default(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    engine.set_playing(true);
    let before = engine.pump_virtual(512).unwrap();
    let identity = engine.snapshot().engine_id;
    let xrun = cpal::Error::new(cpal::ErrorKind::Xrun);
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| {
            for _ in 0..3 {
                output_stream_failed(&engine.controls, &xrun);
            }
        }),
        0
    );
    assert!(engine.snapshot().playing);
    assert!(!engine.snapshot().output_failed);
    assert_eq!(engine.snapshot().output_dropouts, 3);
    assert_eq!(engine.snapshot().render_errors, 0);
    assert!(engine.take_input_error().is_none());
    assert!(engine.take_midi_error().is_none());
    assert!(engine.take_render_error().is_none());
    assert_eq!(engine.take_output_dropout_notice(), Some(3));
    assert_eq!(engine.take_output_dropout_notice(), None);
    let after = engine.pump_virtual(512).unwrap();
    assert_eq!(before, after);
    assert!(
        after
            .iter()
            .all(|frame| (frame[0] - 0.4).abs() < 1e-6 && (frame[1] - 0.2).abs() < 1e-6)
    );
    assert_eq!(engine.snapshot().engine_id, identity);
    assert_eq!(engine.snapshot().consumed_frames, 1024);
    assert_eq!(engine.snapshot().underruns, 0);
    engine.xrun_virtual_output().unwrap();
    assert_eq!(engine.snapshot().output_dropouts, 4);
    assert_eq!(
        engine.take_output_dropout_notice(),
        None,
        "Dismissed/consumed notice must not repeat on this stream"
    );
    // No generic recovery: a later actual I/O error remains terminal and retains its cause.
    output_stream_failed(
        &engine.controls,
        &cpal::Error::with_message(
            cpal::ErrorKind::BackendError,
            "snd_pcm_avail_delay: I/O error (5)",
        ),
    );
    assert!(engine.snapshot().output_failed);
    assert!(!engine.snapshot().playing);
    assert!(engine.take_input_error().unwrap().contains("I/O error (5)"));
    assert!(
        engine
            .pump_virtual(256)
            .unwrap()
            .iter()
            .all(|frame| *frame == [0.0; 2])
    );
}

#[test]
fn output_xrun_stops_audio_and_midi_capture_and_preserves_the_exact_recorded_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let audio = project.add_audio_track("Audio capture");
    let midi = project.add_midi_track("MIDI capture");
    let mut engine = AudioEngine::start_virtual_cancellable(
        &project,
        DeviceSettings::default(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    engine
        .configure_input(
            &project,
            InputSettings {
                device_name: None,
                routes: vec![InputRoute {
                    track: audio,
                    channels: daw_core::AudioInputChannels::Mono { channel: 0 },
                    monitor: false,
                }],
            },
        )
        .unwrap();
    engine
        .configure_midi(
            &project,
            MidiSettings {
                port_id: Some("rill-virtual-midi".into()),
                routes: vec![MidiRoute {
                    track: midi,
                    channel: None,
                    monitor: false,
                }],
            },
        )
        .unwrap();
    engine
        .start_recording(RecordingConfig {
            directory: directory.path().join("takes"),
            tracks: vec![audio],
        })
        .unwrap();
    engine.start_midi_recording(vec![midi]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !engine.recording_snapshot().active {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    engine.inject_virtual_midi(&[0x90, 60, 100], 0).unwrap();
    engine
        .pump_virtual_input(&[0.2, -0.1].repeat(512), 2)
        .unwrap();
    engine.xrun_virtual_output().unwrap();
    assert!(engine.snapshot().output_failed);
    assert!(!engine.snapshot().playing);
    assert_eq!(engine.snapshot().output_dropouts, 1);
    assert!(engine.take_output_dropout_notice().is_none());
    let mut recorded_audio = None;
    let mut recorded_midi = None;
    while recorded_audio.is_none() || recorded_midi.is_none() {
        if recorded_audio.is_none() {
            recorded_audio = engine.take_recording_result();
        }
        if recorded_midi.is_none() {
            recorded_midi = engine.take_midi_recording_result();
        }
        assert!(
            Instant::now() < deadline,
            "Captured prefix did not finalize"
        );
        thread::sleep(Duration::from_millis(1));
    }
    let audio = recorded_audio.unwrap();
    assert!(audio.error.as_deref().unwrap().contains("output dropout"));
    assert_eq!(audio.takes.len(), 1);
    assert_eq!(audio.takes[0].frames, 512);
    assert_eq!(
        AudioData::load_wav(&audio.takes[0].path).unwrap().samples,
        vec![0.2; 512]
    );
    let midi = recorded_midi.unwrap();
    assert!(midi.error.as_deref().unwrap().contains("output"));
    assert_eq!(midi.tracks.len(), 1);
    assert_eq!(midi.tracks[0].notes.len(), 1);
    assert_eq!(midi.tracks[0].notes[0].pitch, 60);
    assert!((midi.tracks[0].notes[0].length - 512.0 / 48_000.0 * 2.0).abs() < 1e-9);
    assert!(
        engine
            .take_input_error()
            .unwrap()
            .contains("A buffer underrun or overrun occurred.")
    );
}

#[test]
fn output_xrun_during_armed_precount_remains_a_capture_failure() {
    for (audio, midi) in [(true, false), (false, true), (true, true)] {
        let project = Project::new();
        let mut engine = AudioEngine::start_virtual_cancellable(
            &project,
            DeviceSettings::default(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        engine
            .configure_recording_timing(
                &project,
                RecordingTiming {
                    precount_bars: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        engine.arm_recording_timing(audio, midi).unwrap();
        assert!(engine.timing_snapshot().armed);
        engine.xrun_virtual_output().unwrap();
        assert!(engine.snapshot().output_failed);
        assert!(!engine.snapshot().playing);
        assert!(engine.take_output_dropout_notice().is_none());
        assert!(engine.take_input_error().unwrap().contains("recording"));
        assert!(
            engine
                .pump_virtual(256)
                .unwrap()
                .iter()
                .all(|frame| *frame == [0.0; 2])
        );
        assert_eq!(engine.timing_snapshot().captured_frames, 0);
    }
}
