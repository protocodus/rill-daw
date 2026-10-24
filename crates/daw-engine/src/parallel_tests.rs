use super::*;
use daw_core::{AutomationPoint, AutomationTarget, Clip, Interpolation, TrackKind};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

fn fixture() -> (tempfile::TempDir, Project, Vec<u64>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stereo.wav");
    AudioData {
        sample_rate: 22_050,
        channels: 2,
        samples: (0..44_100)
            .flat_map(|frame| {
                let phase = frame as f32 * 0.061;
                [phase.sin() * 0.17, (phase * 1.37).cos() * 0.13]
            })
            .collect(),
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 137.0;
    let first = project.add_bus("First sum");
    let second = project.add_bus("Second sum");
    let final_bus = project.add_bus("Final sum");
    project.set_output(first, Some(final_bus)).unwrap();
    project.set_output(second, Some(final_bus)).unwrap();
    // Two distinct edges to one node must both arrive before it can process.
    project.add_send(first, final_bus, 0.15, true).unwrap();
    project.track_mut(first).unwrap().latency_samples = 31;
    project.track_mut(second).unwrap().latency_samples = 97;
    project.track_mut(final_bus).unwrap().latency_samples = 127;
    let mut sources = Vec::new();
    for (index, latency) in [0, 1, 17, 63, 257, 1024].into_iter().enumerate() {
        let track = project.add_audio_track(format!("Source {index}"));
        let channel = project.track_mut(track).unwrap();
        channel.gain = 0.25 + index as f32 * 0.03;
        channel.pan = (index as f32 - 2.5) * 0.1;
        channel.latency_samples = latency;
        let mut clip = Clip::audio(
            "Trimmed",
            index as f64 * 0.0625,
            2.0,
            path.to_string_lossy(),
        );
        clip.offset = 0.125;
        channel.clips.push(clip);
        if index < 5 {
            project
                .set_output(track, Some(if index % 2 == 0 { first } else { second }))
                .unwrap();
        }
        project
            .add_send(
                track,
                if index % 2 == 0 { second } else { first },
                0.2,
                index % 2 == 0,
            )
            .unwrap();
        sources.push(track);
    }
    for (track, parameter, interpolation) in [
        (
            sources[0],
            AutomationParameter::TrackGain,
            Interpolation::Smooth,
        ),
        (
            sources[1],
            AutomationParameter::TrackPan,
            Interpolation::Linear,
        ),
        (
            sources[2],
            AutomationParameter::SendGain { bus: second },
            Interpolation::Step,
        ),
        (first, AutomationParameter::TrackGain, Interpolation::Linear),
    ] {
        let automation = project
            .add_automation_track("Curve", AutomationTarget { track, parameter })
            .unwrap();
        let lane = project.automation_lane_mut(automation).unwrap();
        lane.interpolation = interpolation;
        lane.points = vec![
            AutomationPoint::new(0.0, 0.1),
            AutomationPoint::new(0.5, 0.8),
            AutomationPoint::new(1.5, 0.35),
        ];
    }
    project.validate().unwrap();
    (directory, project, sources)
}

fn compare(serial: &mut Renderer, parallel: &mut Renderer, count: usize) -> Vec<f64> {
    let mut expected = vec![[0.0; 2]; count];
    let mut actual = expected.clone();
    let mut expected_positions = vec![0.0; count];
    let mut actual_positions = expected_positions.clone();
    serial
        .render(&mut expected, &mut expected_positions)
        .unwrap();
    parallel.render(&mut actual, &mut actual_positions).unwrap();
    assert_eq!(
        actual, expected,
        "parallel routing changed sample values or summation order"
    );
    assert_eq!(actual_positions, expected_positions);
    assert!(actual.iter().flatten().all(|sample| sample.is_finite()));
    actual_positions
}

#[test]
fn parallel_graph_is_sample_identical_with_pdc_sends_automation_and_seek() {
    let (_directory, project, _) = fixture();
    let mut serial = Renderer::new_with_workers(&project, 48_000, false, 1).unwrap();
    let mut parallel = Renderer::new_with_workers(&project, 48_000, false, 4).unwrap();
    assert_eq!(serial.end_sample, parallel.end_sample);
    for beat in [0.0, 0.25, 1.25, 0.0625] {
        serial.seek(beat).unwrap();
        parallel.seek(beat).unwrap();
        for count in [1, 17, 256, 511, 3, 128, 4093] {
            compare(&mut serial, &mut parallel, count);
        }
    }
}

#[test]
fn parallel_graph_preserves_repeated_loop_positions_and_sample_values() {
    let (_directory, mut project, _) = fixture();
    project.loop_enabled = true;
    project.loop_start = 0.125;
    project.loop_end = 0.375;
    let mut serial = Renderer::new_with_workers(&project, 48_000, true, 1).unwrap();
    let mut parallel = Renderer::new_with_workers(&project, 48_000, true, 4).unwrap();
    serial.seek(project.loop_start).unwrap();
    parallel.seek(project.loop_start).unwrap();
    let positions = compare(&mut serial, &mut parallel, 24_000);
    assert!(
        positions
            .windows(2)
            .filter(|pair| pair[1] < pair[0])
            .count()
            >= 3,
        "test must cross several loop seams"
    );
}

#[test]
fn dependency_levels_mix_every_parent_before_processing_a_bus() {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    let bus = project.add_bus("Sum");
    project.track_mut(bus).unwrap().gain = 0.5;
    project.track_mut(bus).unwrap().latency_samples = 127;
    for (index, value, latency) in [(0, 0.25, 17), (1, 0.125, 257), (2, 0.125, 3)] {
        let path = directory.path().join(format!("{index}.wav"));
        AudioData {
            sample_rate: 8_000,
            channels: 1,
            samples: vec![value; 8_000],
        }
        .write_wav(&path)
        .unwrap();
        let id = project.add_audio_track(format!("Constant {index}"));
        let track = project.track_mut(id).unwrap();
        track.gain = 1.0;
        track.latency_samples = latency;
        track
            .clips
            .push(Clip::audio("Constant", 0.0, 2.0, path.to_string_lossy()));
        if index < 2 {
            project.set_output(id, Some(bus)).unwrap();
        }
        if index == 0 {
            project.add_send(id, bus, 0.5, true).unwrap();
        }
    }
    for workers in [1, 2, 4] {
        let mut renderer = Renderer::new_with_workers(&project, 8_000, false, workers).unwrap();
        let mut audio = vec![[0.0; 2]; 1024];
        renderer.render(&mut audio, &mut vec![0.0; 1024]).unwrap();
        // ((0.25 main + 0.125 pre-send + 0.125 other) * 0.5 bus) + 0.125 master.
        assert!(
            audio.iter().flatten().all(|&sample| sample == 0.375),
            "a source was omitted, duplicated, or processed after its bus"
        );
    }
}

#[test]
fn independent_sources_actually_overlap_on_multiple_persistent_workers() {
    let (_directory, project, sources) = fixture();
    let mut renderer = Renderer::new_with_workers(&project, 48_000, false, 4).unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let threads = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let observed = (Arc::clone(&active), Arc::clone(&peak), Arc::clone(&threads));
    renderer.set_processing_observer(Arc::new(move |id| {
        if sources.contains(&id) {
            let count = observed.0.fetch_add(1, Ordering::SeqCst) + 1;
            observed.1.fetch_max(count, Ordering::SeqCst);
            observed
                .2
                .lock()
                .unwrap()
                .insert(std::thread::current().id());
            // Bounded delay makes overlap observable without a barrier deadlock
            // if dispatch accidentally regresses to serial execution.
            std::thread::sleep(std::time::Duration::from_millis(3));
            observed.0.fetch_sub(1, Ordering::SeqCst);
        }
    }));
    renderer
        .render(&mut [[0.0; 2]; 256], &mut [0.0; 256])
        .unwrap();
    assert!(
        peak.load(Ordering::SeqCst) >= 2,
        "node computation remained serial"
    );
    assert!(threads.lock().unwrap().len() >= 2);
    for _ in 0..4 {
        renderer
            .render(&mut [[0.0; 2]; 256], &mut [0.0; 256])
            .unwrap();
    }
    assert!(
        threads.lock().unwrap().len() <= 4,
        "render blocks created fresh threads"
    );
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
fn silent_and_muted_nodes_do_not_stall_the_dependency_graph() {
    let mut project = Project::new();
    let folder = project.add_folder("Muted");
    project.track_mut(folder).unwrap().mute = true;
    let bus = project.add_bus("Silent bus");
    for index in 0..4 {
        let track = project.add_midi_track(format!("Unassigned {index}"));
        project.set_output(track, Some(bus)).unwrap();
        if index % 2 == 0 {
            project.set_parent_folder(track, Some(folder)).unwrap();
        }
    }
    assert!(
        project
            .tracks
            .iter()
            .all(|track| track.kind != TrackKind::Automation)
    );
    let mut renderer = Renderer::new_with_workers(&project, 8_000, false, 4).unwrap();
    let mut audio = [[1.0; 2]; 513];
    renderer.render(&mut audio, &mut [0.0; 513]).unwrap();
    assert!(audio.iter().flatten().all(|sample| *sample == 0.0));
}

#[test]
fn worker_panic_restores_all_nodes_before_returning_an_offline_error() {
    let (_directory, project, sources) = fixture();
    let mut renderer = Renderer::new_with_workers(&project, 48_000, false, 4).unwrap();
    let failed = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&failed);
    renderer.set_processing_observer(Arc::new(move |id| {
        if id == sources[0] && !observed.swap(true, Ordering::SeqCst) {
            panic!("intentional graph worker failure");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }));
    let error = renderer
        .render(&mut [[0.0; 2]; 256], &mut [0.0; 256])
        .unwrap_err();
    assert!(error.to_string().contains("panicked"));
    assert!(failed.load(Ordering::SeqCst));
    assert!(renderer.tracks.iter().all(Option::is_some));
    // Recovery must not encounter a missing node or an undrained completion from
    // the failed block, even when another worker finished after the failure.
    renderer.set_processing_observer(Arc::new(|_| {}));
    renderer.seek(0.5).unwrap();
    let mut audio = [[0.0; 2]; 1024];
    renderer.render(&mut audio, &mut [0.0; 1024]).unwrap();
    assert!(audio.iter().flatten().any(|sample| sample.abs() > 0.001));
    assert_eq!(renderer.plugin_errors, 0);
}

#[test]
fn live_worker_panic_silences_only_its_path_and_keeps_the_pool_running() {
    let (_directory, project, sources) = fixture();
    let mut renderer = Renderer::new_with_workers(&project, 48_000, true, 4).unwrap();
    renderer.seek(0.5).unwrap();
    let failed_id = sources[0];
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    renderer.set_processing_observer(Arc::new(move |id| {
        if id == failed_id {
            observed.fetch_add(1, Ordering::SeqCst);
            panic!("intentional live graph failure");
        }
    }));
    for _ in 0..3 {
        let mut audio = [[0.0; 2]; 256];
        renderer.render(&mut audio, &mut [0.0; 256]).unwrap();
        assert!(renderer.tracks.iter().all(Option::is_some));
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        assert!(audio.iter().flatten().any(|sample| sample.abs() > 0.001));
    }
    let failed: Vec<_> = renderer
        .tracks
        .iter()
        .flatten()
        .filter(|track| track.plugin_failed)
        .map(|track| track.id)
        .collect();
    assert_eq!(failed, vec![failed_id]);
    assert_eq!(renderer.plugin_errors, 1);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn disconnected_worker_restores_dispatched_and_unsent_nodes_without_drop_deadlock() {
    let (completed, result) = std::sync::mpsc::sync_channel(1);
    let test = std::thread::spawn(move || {
        let (_directory, project, _) = fixture();
        let mut renderer = Renderer::new_with_workers(&project, 48_000, false, 4).unwrap();
        // Disconnect after preparation; worker 0 already owns a dispatched batch
        // when the send to worker 1 fails. Wider pools also retain unsent batches.
        renderer.workers.as_mut().unwrap().disconnect_for_test(1);
        let failure = renderer
            .render(&mut [[0.0; 2]; 256], &mut [0.0; 256])
            .unwrap_err();
        assert!(failure.to_string().contains("exited"));
        assert!(renderer.tracks.iter().all(Option::is_some));
        drop(renderer);
        completed.send(()).unwrap();
    });
    result
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("worker failure left in-flight work blocked during pool teardown");
    test.join().unwrap();
}

#[test]
fn prepared_parallel_dispatch_reuses_its_buffers_without_allocating() {
    let (_directory, project, _) = fixture();
    let mut renderer = Renderer::new_with_workers(&project, 48_000, false, 4).unwrap();
    let mut audio = [[0.0; 2]; 1024];
    let mut positions = [0.0; 1024];
    renderer.render(&mut audio, &mut positions).unwrap();
    // The counter is thread-local: this specifically checks the dispatcher,
    // ownership transfer, completion handling and ordered routing/mixing.
    let allocations = crate::allocation_test_support::count_allocations(|| {
        for _ in 0..8 {
            renderer.render(&mut audio, &mut positions).unwrap();
        }
    });
    assert_eq!(allocations, 0);
}

fn native_project(track_count: usize) -> anyhow::Result<Project> {
    use daw_core::{Effect, EffectSlot, Instrument, Note};
    let instrument = std::env::var("DAW_TEST_VST3")?;
    let effect = std::env::var("DAW_TEST_VST3_EFFECT")?;
    let mut project = Project::new();
    for index in 0..track_count {
        let id = project.add_midi_track(format!("Native {index}"));
        let track = project.track_mut(id).unwrap();
        track.gain = 1.0 / track_count as f32;
        track.instrument = Instrument::Vst3 {
            path: instrument.clone(),
            class_id: None,
        };
        track.inserts.push(EffectSlot::new(
            "Native insert",
            Effect::Vst3 {
                path: effect.clone(),
                class_id: None,
            },
        ));
        track.clips.push(Clip::midi(
            "Chords",
            0.0,
            8.0,
            (0..4)
                .flat_map(|beat| {
                    [48, 55, 60, 64].map(|pitch| Note::new(pitch, beat as f64 * 2.0, 1.8, 90))
                })
                .collect(),
        ));
    }
    Ok(project)
}

#[test]
#[ignore = "requires Surge XT and CHOWTapeModel in DAW_TEST_VST3 and DAW_TEST_VST3_EFFECT"]
fn native_first_beat_notes_survive_initial_latency_prefill() -> anyhow::Result<()> {
    let project = native_project(2)?;
    for automatic in [false, true] {
        let mut renderer = if automatic {
            Renderer::new(&project, 48_000, false)?
        } else {
            Renderer::new_with_workers(&project, 48_000, false, 1)?
        };
        for seeking in [false, true] {
            if seeking {
                renderer.seek(0.0)?;
            }
            // The first pass must not seek: the note-on arrives in the first
            // short block used to compensate the insert in a newly loaded graph.
            let mut audio = vec![[0.0; 2]; 12_000];
            renderer.render(&mut audio, &mut vec![0.0; 12_000])?;
            for channel in 0..2 {
                let peak = audio
                    .iter()
                    .fold(0.0_f32, |peak, sample| peak.max(sample[channel].abs()));
                assert!(
                    peak > 0.001,
                    "beat-zero note lost (auto={automatic}, seek={seeking}, channel={channel}, peak={peak})"
                );
            }
        }
    }
    Ok(())
}

/// Run with --release and installed native instruments/effects. Initialization,
/// warmup, allocation and diagnostic observers are outside the timed interval.
#[test]
#[ignore = "requires native plug-ins in DAW_TEST_VST3 and DAW_TEST_VST3_EFFECT"]
fn native_prepared_graph_uses_automatic_workers_and_reports_throughput() -> anyhow::Result<()> {
    anyhow::ensure!(
        std::thread::available_parallelism()?.get() > 1,
        "Requires multiple CPU cores"
    );
    for tracks in [4, 8, 16] {
        let project = native_project(tracks)?;
        let mut timings = Vec::new();
        for automatic in [false, true] {
            let mut renderer = if automatic {
                Renderer::new(&project, 48_000, false)?
            } else {
                Renderer::new_with_workers(&project, 48_000, false, 1)?
            };
            assert_eq!(renderer.workers.is_some(), automatic);
            if automatic {
                let threads = Arc::new(Mutex::new(std::collections::HashSet::new()));
                let observed = Arc::clone(&threads);
                renderer.set_processing_observer(Arc::new(move |_| {
                    observed.lock().unwrap().insert(std::thread::current().id());
                }));
                renderer.render(&mut [[0.0; 2]; 256], &mut [0.0; 256])?;
                assert!(
                    threads.lock().unwrap().len() > 1,
                    "automatic native graph remained serial"
                );
                for track in renderer.tracks.iter_mut().flatten() {
                    track.observer = None;
                }
            }
            renderer.seek(0.0)?;
            let mut audio = vec![[0.0; 2]; 192_000];
            let mut positions = vec![0.0; audio.len()];
            let started = std::time::Instant::now();
            renderer.render(&mut audio, &mut positions)?;
            let elapsed = started.elapsed().as_secs_f64();
            assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
            assert!(audio.iter().flatten().any(|sample| sample.abs() > 0.001));
            assert_eq!(renderer.plugin_errors, 0);
            timings.push(elapsed);
        }
        eprintln!(
            "Prepared {tracks} instrument+effect tracks ({} isolated VST instances), 4s/48k stereo: serial {:.4}s, automatic {:.4}s, {:.2}x speedup, {:.2}x realtime",
            tracks * 2,
            timings[0],
            timings[1],
            timings[0] / timings[1],
            4.0 / timings[1]
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
#[ignore = "kills only owned native helpers; requires DAW_TEST_VST3 and DAW_TEST_VST3_EFFECT"]
fn native_helper_crash_restores_parallel_nodes_and_contains_the_failed_live_path()
-> anyhow::Result<()> {
    let mut project = native_project(2)?;
    project.tracks[0].pan = -1.0;
    project.tracks[1].pan = 1.0;
    for track in &mut project.tracks {
        for clip in &mut track.clips {
            if let daw_core::ClipContent::Midi { notes } = &mut clip.content {
                for note in notes {
                    note.start += 0.25;
                }
            }
        }
    }
    for (live, fail_insert) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut renderer = Renderer::new(&project, 48_000, live)?;
        assert!(
            renderer.workers.is_some(),
            "native crash test must exercise automatic parallel processing"
        );
        let mut warmup = vec![[0.0; 2]; 24_000];
        renderer.render(&mut warmup, &mut vec![0.0; 24_000])?;
        assert!(warmup.iter().any(|sample| sample[0].abs() > 0.001));
        assert!(warmup.iter().any(|sample| sample[1].abs() > 0.001));
        let failed_id = project.tracks[0].id;
        let failed_track = renderer
            .tracks
            .iter()
            .flatten()
            .find(|track| track.id == failed_id)
            .unwrap();
        let Processor::Vst(plugin) = &failed_track.processor else {
            panic!("native instrument missing")
        };
        let pid = if fail_insert {
            let Insert::Vst(effect, _) = &failed_track.inserts[0];
            effect.isolation_pid().unwrap()
        } else {
            plugin.isolation_pid().unwrap()
        };
        assert_ne!(pid, std::process::id());
        assert!(
            std::process::Command::new("/bin/kill")
                .args(["-KILL", &pid.to_string()])
                .status()?
                .success()
        );
        let mut audio = [[0.0; 2]; 1024];
        let result = renderer.render(&mut audio, &mut [0.0; 1024]);
        assert!(renderer.tracks.iter().all(Option::is_some));
        if live {
            result?;
            assert_eq!(renderer.plugin_errors, 1);
            assert!(audio.iter().all(|sample| sample[0] == 0.0));
            assert!(
                audio.iter().any(|sample| sample[1].abs() > 0.000_01),
                "healthy path was silenced (live={live}, insert={fail_insert})"
            );
            renderer.render(&mut audio, &mut [0.0; 1024])?;
            assert_eq!(renderer.plugin_errors, 1);
            assert!(audio.iter().all(|sample| sample[0] == 0.0));
            assert!(
                audio.iter().any(|sample| sample[1].abs() > 0.000_01),
                "healthy path was silenced (live={live}, insert={fail_insert})"
            );
        } else {
            assert!(
                result.is_err(),
                "offline render must report a native helper crash"
            );
        }
    }
    Ok(())
}
