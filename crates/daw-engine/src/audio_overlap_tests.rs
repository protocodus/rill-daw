use super::*;
use daw_core::{AudioCrossfade, AudioSchedule, Clip, CrossfadeCurve};

fn project() -> (Project, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Exclusive takes");
    project.track_mut(track).unwrap().gain = 1.0;
    for (index, (start, end, level)) in [
        (0.0, 8.0, 0.08_f32),
        (2.0, 6.0, 0.19),
        (3.0, 4.0, 0.31),
        (2.0, 5.0, 0.43),
    ]
    .into_iter()
    .enumerate()
    {
        let path = directory.path().join(format!("{index}.wav"));
        AudioData {
            sample_rate: 8000,
            channels: 2,
            samples: (0..32000).flat_map(|_| [level, -level * 0.5]).collect(),
        }
        .write_wav(&path)
        .unwrap();
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            format!("Take{index}"),
            start,
            end - start,
            path.to_string_lossy(),
        ));
    }
    (project, directory)
}
fn level(winner: usize) -> f32 {
    [0.08, 0.19, 0.31, 0.43][winner]
}
fn expected(project: &Project, beat: f64) -> f32 {
    let schedule = AudioSchedule::new(&project.tracks[0].clips).unwrap();
    schedule.sample(beat).map_or(0.0, |mix| {
        level(mix.incoming) * mix.incoming_gain
            + mix.outgoing.map_or(0.0, |i| level(i) * mix.outgoing_gain)
    })
}

#[test]
fn exclusive_takes_crossfades_and_resumed_audio_match_sample_oracle_and_wav_export() {
    let (mut project, directory) = project();
    for curve in [
        None,
        Some(CrossfadeCurve::LINEAR),
        Some(CrossfadeCurve::SMOOTH),
        Some(CrossfadeCurve {
            control1: [0.1, 0.85],
            control2: [0.8, 0.2],
        }),
    ] {
        for clip in &mut project.tracks[0].clips {
            if let ClipContent::Audio { crossfade, .. } = &mut clip.content {
                *crossfade = curve.map(|curve| AudioCrossfade {
                    duration: 2.0,
                    curve,
                });
            }
        }
        let audio = crate::render(&project, 8000, 8.0).unwrap();
        let schedule = AudioSchedule::new(&project.tracks[0].clips).unwrap();
        for frame in 0..32000 {
            let mix = schedule.sample(frame as f64 / 4000.0).unwrap();
            let expected = level(mix.incoming) * mix.incoming_gain
                + mix.outgoing.map_or(0.0, |i| level(i) * mix.outgoing_gain);
            assert!(
                (audio[frame * 2] - expected).abs() < 3e-7,
                "frame{frame}, curve{curve:?}"
            );
            assert!((audio[frame * 2 + 1] + expected * 0.5).abs() < 3e-7);
        }
        let path = directory.path().join("export.wav");
        crate::export_wav(&project, &path, 8000, 8.0).unwrap();
        assert_eq!(AudioData::load_wav(path).unwrap().samples, audio);
    }
}

#[test]
fn overlap_seek_loop_pdc_and_prepared_render_are_consistent_and_allocation_free() {
    let (mut project, _directory) = project();
    for clip in &mut project.tracks[0].clips {
        if let ClipContent::Audio { crossfade, .. } = &mut clip.content {
            *crossfade = Some(AudioCrossfade {
                duration: 2.0,
                curve: CrossfadeCurve::SMOOTH,
            });
        }
    }
    project.tracks[0].latency_samples = 37;
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    let mut audio = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    for beat in [0.25, 2.25, 3.25, 4.0, 5.0, 6.0] {
        renderer.seek(beat).unwrap();
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| renderer
                .render(&mut audio, &mut positions)
                .unwrap()),
            0
        );
        for (index, sample) in audio.iter().enumerate() {
            let value = expected(&project, beat + index as f64 / 4000.0);
            assert!((sample[0] - value).abs() < 4e-7, "seek{beat} index{index}");
        }
    }
    project.loop_enabled = true;
    project.loop_start = 2.25;
    project.loop_end = 2.3;
    let mut renderer = Renderer::new(&project, 8000, true).unwrap();
    renderer.seek(2.25).unwrap();
    renderer.render(&mut audio, &mut positions).unwrap();
    for (index, sample) in audio.iter().enumerate() {
        let value = expected(&project, 2.25 + (index % 200) as f64 / 4000.0);
        assert!((sample[0] - value).abs() < 4e-7, "loop index{index}");
    }
    project.loop_enabled = false;
    let mut renderer = Renderer::new(&project, 8000, true).unwrap();
    renderer.seek(2.25).unwrap();
    renderer.render(&mut audio, &mut positions).unwrap();
    assert!(positions[BLOCK - 1] > project.loop_end);
    for (index, sample) in audio.iter().enumerate() {
        let value = expected(&project, 2.25 + index as f64 / 4000.0);
        assert!(
            (sample[0] - value).abs() < 4e-7,
            "disabled loop index{index}"
        );
    }
}

#[test]
fn seek_and_audio_boundaries_use_identical_half_sample_rounding() {
    let (mut project, _directory) = project();
    project.tempo = 123.0;
    project.tracks[0].clips.truncate(1);
    let clip = &mut project.tracks[0].clips[0];
    clip.start = 1098.199734375;
    clip.length = 1.0;
    let beat = clip.start;
    let mut renderer = Renderer::new(&project, 8000, false).unwrap();
    let start = renderer.tracks[0].as_ref().unwrap().audio[0].start;
    assert_eq!(start, 4_285_658);
    renderer.seek(beat).unwrap();
    assert_eq!(renderer.logical_sample, start);
    let mut output = [[0.0; 2]; 1];
    let mut positions = [0.0; 1];
    renderer.render(&mut output, &mut positions).unwrap();
    assert_eq!(output[0], [0.08, -0.04]);
}

#[test]
fn cancelled_wav_render_stops_between_blocks_and_preserves_existing_file() {
    use std::sync::atomic::AtomicUsize;

    let (project, directory) = project();
    let path = directory.path().join("keep.wav");
    std::fs::write(&path, b"previous audio").unwrap();
    // Cancel during both a final block and a non-final block. Neither may
    // replace the destination, and a non-final cancellation must stop promptly.
    for frames in [BLOCK as u64, BLOCK as u64 * 4] {
        let mut renderer = Renderer::new_with_workers(&project, 8000, false, 1).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_cancel = Arc::clone(&cancel);
        let worker_calls = Arc::clone(&calls);
        renderer.set_processing_observer(Arc::new(move |_| {
            worker_calls.fetch_add(1, Ordering::Relaxed);
            worker_cancel.store(true, Ordering::Relaxed);
        }));
        let error = write_wav_cancelled(renderer, &path, frames, &cancel).unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"previous audio");
        assert!(std::fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }
    let renderer = Renderer::new(&project, 8000, false).unwrap();
    let absent = directory.path().join("cancelled.wav");
    assert!(write_wav_cancelled(renderer, &absent, 1, &AtomicBool::new(true)).is_err());
    assert!(!absent.exists());
}
