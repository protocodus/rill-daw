use super::*;
use crate::{RenderOptions, ResampleQuality, allocation_test_support::count_allocations, render};
use daw_core::Clip;

fn fixture() -> (tempfile::TempDir, Vec<String>) {
    let directory = tempfile::tempdir().unwrap();
    let paths = [16_777_216.0_f32, 1.0, -16_777_216.0]
        .into_iter()
        .enumerate()
        .map(|(index, amplitude)| {
            let path = directory.path().join(format!("{index}.wav"));
            AudioData {
                sample_rate: 8_000,
                channels: 1,
                samples: vec![amplitude; 4_000],
            }
            .write_wav(&path)
            .unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect();
    (directory, paths)
}

#[test]
fn f64_accumulates_bus_inputs_pre_fader_sends_and_master() {
    let (_directory, paths) = fixture();
    for topology in 1..4 {
        let mut project = Project::new();
        let bus = if topology >= 2 {
            let bus = project.add_bus("Sum");
            project.track_mut(bus).unwrap().gain = 1.0;
            Some(bus)
        } else {
            None
        };
        for (index, path) in paths.iter().enumerate() {
            let id = project.add_audio_track(format!("Source {index}"));
            let track = project.track_mut(id).unwrap();
            track.gain = 1.0;
            track.pan = 0.0;
            track
                .clips
                .push(Clip::audio("Cancellation", 0.0, 1.0, path));
            if let Some(bus) = bus {
                if topology == 3 {
                    project.track_mut(id).unwrap().gain = 0.0;
                    project.add_send(id, bus, 1.0, true).unwrap();
                } else {
                    project.set_output(id, Some(bus)).unwrap();
                }
            }
        }
        let output = render(&project, 8_000, 1.0).unwrap();
        assert!(
            output.iter().all(|sample| *sample == 1.0),
            "topology {topology}: lost small contribution between cancelling large signals: {:?}",
            &output[..8]
        );
    }
}

#[test]
fn source_conversion_is_shared_and_processing_remains_allocation_free() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.wav");
    AudioData {
        sample_rate: 44_100,
        channels: 2,
        samples: vec![0.1; 88_200],
    }
    .write_wav(&path)
    .unwrap();
    let mut project = Project::new();
    for i in 0..3 {
        let track = project.add_audio_track(format!("Copy {i}"));
        project.track_mut(track).unwrap().clips.push(Clip::audio(
            "Source",
            0.0,
            1.0,
            path.to_string_lossy(),
        ));
    }
    let mut renderer = Renderer::new_with_options(
        &project,
        48_000,
        false,
        RenderOptions {
            resample_quality: ResampleQuality::High,
        },
    )
    .unwrap();
    let first = &renderer.tracks[0].as_ref().unwrap().audio[0].data;
    for track in renderer.tracks.iter().flatten() {
        assert!(Arc::ptr_eq(first, &track.audio[0].data));
        assert_eq!(track.audio[0].data.sample_rate, 48_000);
    }
    let mut output = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    renderer.render(&mut output, &mut positions).unwrap();
    assert_eq!(
        count_allocations(|| renderer.render(&mut output, &mut positions).unwrap()),
        0
    );
}

#[test]
fn cancelled_preparation_stops_before_opening_assets() {
    let mut project = Project::new();
    let track = project.add_audio_track("Cancelled source");
    project.track_mut(track).unwrap().clips.push(Clip::audio(
        "Missing source",
        0.0,
        1.0,
        "source-must-not-be-opened.wav",
    ));
    let cancelled = AtomicBool::new(true);
    let result = Renderer::new_with_options_cancelled(
        &project,
        48_000,
        false,
        RenderOptions::default(),
        &cancelled,
    );
    assert_eq!(
        result.err().unwrap().to_string(),
        "Audio preparation cancelled"
    );
    cancelled.store(false, Ordering::Release);
    let result = Renderer::new_with_options_cancelled(
        &project,
        48_000,
        false,
        RenderOptions::default(),
        &cancelled,
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("Loading audio clip")
    );
}
