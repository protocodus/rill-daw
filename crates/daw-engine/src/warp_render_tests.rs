use super::*;
use crate::stretch::{TimeMap, prepare_file_mapped, prepare_mapped};
use daw_core::{
    AudioProcessing, AudioProcessingAlgorithm, AudioQuantizeSettings, Clip, TransientSettings,
    WarpMarker,
};

const RATE: u32 = 48_000;

fn sine(frames: usize, frequency: f64) -> AudioData {
    AudioData {
        sample_rate: RATE,
        channels: 1,
        samples: (0..frames)
            .map(|i| {
                (std::f64::consts::TAU * frequency * i as f64 / f64::from(RATE)).sin() as f32 * 0.5
            })
            .collect(),
    }
}

/// Dominant frequency from upward zero crossings, interpolated to sub-sample.
fn frequency(samples: &[f32]) -> f64 {
    let crossings: Vec<f64> = samples
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] < 0.0 && pair[1] >= 0.0)
        .map(|(index, pair)| index as f64 + f64::from(-pair[0] / (pair[1] - pair[0])))
        .collect();
    let span = crossings.last().unwrap() - crossings.first().unwrap();
    (crossings.len() - 1) as f64 / span * f64::from(RATE)
}

fn set_algorithm(
    project: &mut Project,
    track: u64,
    clip: u64,
    algorithm: AudioProcessingAlgorithm,
) {
    project
        .set_audio_processing(
            track,
            clip,
            AudioProcessing {
                algorithm,
                transpose_semitones: 0.0,
            },
        )
        .unwrap();
}

#[test]
fn warped_preparation_keeps_pitch_has_no_seams_and_matches_disk_streaming() {
    let directory = tempfile::tempdir().unwrap();
    let source = sine(RATE as usize * 3, 440.0);
    let path = directory.path().join("sine.wav");
    source.write_wav(&path).unwrap();
    let file = daw_core::AudioFile::open(&path).unwrap();
    // 0.8x then 2x through the markers, the clip rate (1x) after the last.
    let map = TimeMap::warped(
        1.0,
        vec![[0.0, 0.0], [48_000.0, 60_000.0], [96_000.0, 84_000.0]],
    );
    let natural_step = 0.5 * std::f64::consts::TAU * 440.0 / f64::from(RATE);
    for algorithm in AudioProcessingAlgorithm::ALL {
        let output = prepare_mapped(&source, &map, 0.0, algorithm, None).unwrap();
        assert_eq!(output.frames(), 132_000, "{algorithm:?}");
        for (start, end) in [(6_000, 54_000), (63_000, 81_000), (90_000, 126_000)] {
            let hz = frequency(&output.samples[start..end]);
            assert!((hz - 440.0).abs() < 4.0, "{algorithm:?} {start}: {hz} Hz");
        }
        // Segment joins at 60 000 and 84 000 come from one continuous stretcher:
        // no step exceeds what the sine itself produces, with stretch tolerance.
        for boundary in [60_000_usize, 84_000] {
            let jump = output.samples[boundary - 2_000..boundary + 2_000]
                .windows(2)
                .map(|pair| f64::from((pair[1] - pair[0]).abs()))
                .fold(0.0, f64::max);
            assert!(
                jump < natural_step * 1.6,
                "{algorithm:?} at {boundary}: {jump}"
            );
            let level = |range: std::ops::Range<usize>| {
                (output.samples[range.clone()]
                    .iter()
                    .map(|sample| f64::from(*sample).powi(2))
                    .sum::<f64>()
                    / range.len() as f64)
                    .sqrt()
            };
            let before = level(boundary - 2_400..boundary - 400);
            let after = level(boundary + 400..boundary + 2_400);
            assert!(
                (before / after - 1.0).abs() < 0.15,
                "{algorithm:?} level {before} {after}"
            );
        }
        let streamed = prepare_file_mapped(&file, &map, 0.0, algorithm, None).unwrap();
        assert!(streamed.is_streamed());
        let mut disk = vec![0.0; streamed.frames()];
        let mut reader = streamed.file().unwrap().reader().unwrap();
        let mut read = 0;
        while read < disk.len() {
            let count = (disk.len() - read).min(65_536);
            assert_eq!(
                reader
                    .read_frames(read, &mut disk[read..read + count])
                    .unwrap(),
                count
            );
            read += count;
        }
        assert!(
            disk == output.samples,
            "{algorithm:?}: streamed and resident differ"
        );
    }
    // The pitch remains independent: a transposed warp still lands an octave up.
    let up = prepare_mapped(
        &source,
        &map,
        12.0,
        AudioProcessingAlgorithm::Signalsmith,
        None,
    )
    .unwrap();
    let hz = frequency(&up.samples[6_000..54_000]);
    assert!((hz - 880.0).abs() < 8.0, "{hz}");
}

/// Off-grid clicks at 120 BPM: one beat is 24 000 frames.
fn click_project(directory: &std::path::Path) -> (Project, u64, u64, AudioData) {
    let hits = [0.9, 2.1, 2.95, 4.08];
    let mut samples = vec![0.0_f32; 24_000 * 6];
    for hit in hits {
        let start = (hit * 24_000.0) as usize;
        for index in 0..480 {
            let decay = (-(index as f64) / 80.0).exp();
            samples[start + index] =
                ((std::f64::consts::TAU * 2_000.0 * index as f64 / 48_000.0).sin() * decay * 0.8)
                    as f32;
        }
    }
    let source = AudioData {
        sample_rate: RATE,
        channels: 1,
        samples,
    };
    let path = directory.join("clicks.wav");
    source.write_wav(&path).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Clicks");
    project.track_mut(track).unwrap().gain = 1.0;
    let clip = Clip::audio("Clicks", 0.0, 6.0, path.to_string_lossy());
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    (project, track, id, source)
}

fn quantize(project: &mut Project, track: u64, id: u64, source: &AudioData) -> usize {
    let clip = project.clip(track, id).unwrap().clone();
    let onsets = daw_core::detect_transient_onsets(
        &clip,
        source,
        project.tempo,
        TransientSettings::default(),
    )
    .unwrap();
    assert_eq!(onsets.len(), 4, "{onsets:?}");
    let plan = daw_core::plan_warp_markers(
        &clip,
        &onsets,
        &project.meter_map(),
        Some(AudioQuantizeSettings {
            grid: 1.0,
            strength: 1.0,
            max_move: 0.5,
        }),
    )
    .unwrap();
    let moved = plan.moved;
    project.set_warp_markers(track, id, plan.markers).unwrap();
    moved
}

/// First frame reaching half of the local peak, the same rule the onset
/// refinement uses to place markers.
fn onset(left: &[f32], center: usize) -> usize {
    let window = &left[center - 4_800..center + 4_800];
    let peak = window
        .iter()
        .fold(0.0_f32, |peak, value| peak.max(value.abs()));
    center - 4_800
        + window
            .iter()
            .position(|value| value.abs() >= peak * 0.5)
            .unwrap()
}

/// Bungee keeps attacks at warp markers within 1 ms. Signalsmith's 120 ms
/// spectral grains straddle each marker, so an attack where the rate changes
/// is pulled toward the neighbouring segment by a few milliseconds.
fn tolerance(algorithm: AudioProcessingAlgorithm) -> i64 {
    match algorithm {
        AudioProcessingAlgorithm::Bungee => 48,
        AudioProcessingAlgorithm::Signalsmith => 384,
    }
}

#[test]
fn quantized_clicks_render_on_the_grid_for_both_engines_including_reversed() {
    let directory = tempfile::tempdir().unwrap();
    for algorithm in AudioProcessingAlgorithm::ALL {
        let (mut project, track, id, source) = click_project(directory.path());
        set_algorithm(&mut project, track, id, algorithm);
        assert_eq!(quantize(&mut project, track, id, &source), 4);
        let rendered = render(&project, RATE, 6.0).unwrap();
        let left: Vec<f32> = rendered.chunks(2).map(|frame| frame[0]).collect();
        for beat in [1, 2, 3, 4] {
            let error = onset(&left, beat * 24_000) as i64 - (beat * 24_000) as i64;
            assert!(
                error.abs() <= tolerance(algorithm),
                "{algorithm:?} beat {beat}: {error} frames"
            );
        }
        // Reversed playback quantizes what is heard: the click tails now lead.
        project.set_warp_markers(track, id, Vec::new()).unwrap();
        project.reverse_audio_clip(track, id).unwrap();
        let clip = project.clip(track, id).unwrap().clone();
        let onsets = daw_core::detect_transient_onsets(
            &clip,
            &source,
            project.tempo,
            TransientSettings::default(),
        )
        .unwrap();
        let plan = daw_core::plan_warp_markers(
            &clip,
            &onsets,
            &project.meter_map(),
            Some(AudioQuantizeSettings {
                grid: 1.0,
                strength: 1.0,
                max_move: 0.5,
            }),
        )
        .unwrap();
        assert!(plan.moved >= 3, "{plan:?}");
        project.set_warp_markers(track, id, plan.markers).unwrap();
        let rendered = render(&project, RATE, 6.0).unwrap();
        let left: Vec<f32> = rendered.chunks(2).map(|frame| frame[0]).collect();
        for (_, target) in plan.hits {
            let frame = (target * 24_000.0).round() as usize;
            let error = onset(&left, frame) as i64 - frame as i64;
            assert!(
                error.abs() <= tolerance(algorithm),
                "{algorithm:?} reverse {target}: {error}"
            );
        }
    }
}

#[test]
fn live_warp_edits_match_export_share_split_variants_and_refine_previews() {
    let directory = tempfile::tempdir().unwrap();
    let (mut project, track, id, source) = click_project(directory.path());
    let mut renderer = Renderer::new(&project, RATE, true).unwrap();
    let original = Arc::clone(&renderer.tracks[0].as_ref().unwrap().audio[0].data);
    let mut preparer = renderer.live_edit_preparer();
    quantize(&mut project, track, id, &source);
    let mut plan = preparer.prepare(&project).unwrap();
    assert_eq!(
        crate::allocation_test_support::count_allocations(|| renderer
            .apply_live_edit(&mut plan)
            .unwrap()),
        0
    );
    let live = Arc::clone(&renderer.tracks[0].as_ref().unwrap().audio[0].data);
    assert!(!Arc::ptr_eq(&live, &original));
    let offline = Renderer::new(&project, RATE, false).unwrap();
    let exported = &offline.tracks[0].as_ref().unwrap().audio[0];
    assert_eq!(
        exported.data.resident().unwrap().samples,
        live.resident().unwrap().samples
    );
    assert_eq!(
        exported.source_offset,
        renderer.tracks[0].as_ref().unwrap().audio[0].source_offset
    );
    // Both halves of a split keep the same map and reuse one prepared variant,
    // and the split render matches the unsplit one.
    let whole = render(&project, RATE, 6.0).unwrap();
    project.split_clip(track, id, 2.5).unwrap();
    let mut plan = preparer.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut plan).unwrap();
    let clips = &renderer.tracks[0].as_ref().unwrap().audio;
    assert!(Arc::ptr_eq(&clips[0].data, &live));
    assert!(Arc::ptr_eq(&clips[1].data, &live));
    let split = render(&project, RATE, 6.0).unwrap();
    let error = whole
        .iter()
        .zip(&split)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(error < 1e-5, "split changed warped audio by {error}");
    // Clearing restores the original source without a prepared variant.
    let mut plain = project.track(track).unwrap().clips[0].clone();
    plain.warp_markers.clear();
    plain.offset = 0.0;
    plain.length = 6.0;
    project.track_mut(track).unwrap().clips = vec![plain];
    let mut plan = preparer.prepare(&project).unwrap();
    renderer.apply_live_edit(&mut plan).unwrap();
    assert!(Arc::ptr_eq(
        &renderer.tracks[0].as_ref().unwrap().audio[0].data,
        &original
    ));

    // A long warped source previews interactively, then refines to the
    // exported preparation from the original.
    let path = directory.path().join("long.wav");
    sine(RATE as usize * 9, 330.0).write_wav(&path).unwrap();
    let mut long = Clip::audio("Long", 0.0, 18.0, path.to_string_lossy());
    long.warp_markers = vec![
        WarpMarker {
            source: 2.0,
            beat: 2.0,
        },
        WarpMarker {
            source: 4.0,
            beat: 5.0,
        },
    ];
    project.track_mut(track).unwrap().clips[0] = long;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut preview = preparer.prepare_cancelled(&project, &cancel, true).unwrap();
    assert!(preview.needs_refinement());
    renderer.apply_live_edit(&mut preview).unwrap();
    assert!(renderer.tracks[0].as_ref().unwrap().audio[0].is_preview);
    let preview_frames = renderer.tracks[0].as_ref().unwrap().audio[0].data.frames();
    let mut refined = preparer
        .prepare_cancelled(&project, &cancel, false)
        .unwrap();
    renderer.apply_live_edit(&mut refined).unwrap();
    let live = &renderer.tracks[0].as_ref().unwrap().audio[0];
    assert!(!live.is_preview);
    assert_eq!(live.data.frames(), preview_frames);
    let offline = Renderer::new(&project, RATE, false).unwrap();
    assert_eq!(
        offline.tracks[0].as_ref().unwrap().audio[0]
            .data
            .resident()
            .unwrap()
            .samples,
        live.data.resident().unwrap().samples
    );
}
