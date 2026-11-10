use super::*;
use crate::{EditBatch, EntityRef, SongEdit};

fn warped(reverse: bool) -> (Project, u64, u64) {
    let mut project = Project::new();
    let track = project.add_audio_track("Warp");
    let mut clip = Clip::audio("Loop", 4.0, 8.0, "loop.wav");
    clip.offset = 1.0;
    if let ClipContent::Audio { reverse: value, .. } = &mut clip.content {
        *value = reverse;
    }
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    // Source 3 plays at local 2 (rate 1 before), source 5 at local 6 (half
    // speed), then rate 1 again: the clip ends at source 7.
    project
        .set_warp_markers(
            track,
            id,
            vec![
                WarpMarker {
                    source: 3.0,
                    beat: 2.0,
                },
                WarpMarker {
                    source: 5.0,
                    beat: 6.0,
                },
            ],
        )
        .unwrap();
    (project, track, id)
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

/// The source heard at an arrangement beat, for comparing edits.
fn heard(clip: &Clip, beat: f64) -> f64 {
    clip.local_to_source(clip.forward_local(beat - clip.start))
}

#[test]
fn map_converts_both_ways_and_extrapolates_with_the_clip_rate() {
    let (project, track, id) = warped(false);
    let clip = project.clip(track, id).unwrap();
    for (local, source) in [
        (-1.0, 0.0),
        (0.0, 1.0),
        (2.0, 3.0),
        (4.0, 4.0),
        (6.0, 5.0),
        (8.0, 7.0),
    ] {
        close(clip.local_to_source(local), source);
        close(clip.source_to_local(source), local);
    }
    close(clip.offset, 1.0);
    close(clip.source_end(), 7.0);
    close(clip.local_rate(3.0), 0.5);
    close(clip.local_rate(7.0), 1.0);
    // Frame mapping drives waveform drawing and analysis.
    close(clip.audio_source_frame(4.0, 60.0, 10), 40.0);
}

#[test]
fn empty_markers_keep_whole_clip_stretch_and_old_files() {
    let mut clip = Clip::audio("Plain", 0.0, 4.0, "a.wav");
    clip.offset = 2.0;
    clip.source_rate = 2.0;
    close(clip.local_to_source(1.5), 5.0);
    close(clip.source_to_local(5.0), 1.5);
    close(clip.source_end(), 10.0);
    let json = serde_json::to_value(&clip).unwrap();
    assert!(json.get("warp_markers").is_none());
    let loaded: Clip = serde_json::from_value(json).unwrap();
    assert!(loaded.warp_markers.is_empty());
}

#[test]
fn validation_rejects_disorder_and_unsupported_segment_rates() {
    let marker = |source, beat| WarpMarker { source, beat };
    assert!(validate_warp_markers(&[marker(1.0, 1.0), marker(2.0, 2.0)]).is_ok());
    for bad in [
        vec![marker(2.0, 1.0), marker(1.0, 2.0)],
        vec![marker(1.0, 2.0), marker(2.0, 1.0)],
        vec![marker(1.0, 1.0), marker(1.0, 2.0)],
        vec![marker(0.0, 0.0), marker(5.0, 1.0)],
        vec![marker(0.0, 0.0), marker(1.0, 5.0)],
        vec![marker(f64::NAN, 0.0)],
        vec![marker(-1.0, 0.0)],
    ] {
        assert!(validate_warp_markers(&bad).is_err(), "{bad:?}");
    }
    assert!(validate_warp_markers(&[marker(0.0, 0.0), marker(4.0, 1.0)]).is_ok());
    assert!(validate_warp_markers(&[marker(0.0, 0.0), marker(1.0, 4.0)]).is_ok());

    // A failed batch leaves the song untouched.
    let (mut project, track, id) = warped(false);
    let before = project.clone();
    let batch = EditBatch {
        label: "Bad warp".into(),
        edits: vec![SongEdit::SetWarpMarkers {
            track: EntityRef::Id(track),
            clip: EntityRef::Id(id),
            markers: vec![marker(3.0, 1.0), marker(2.0, 2.0)],
        }],
    };
    assert!(batch.apply(&mut project).is_err());
    assert_eq!(project, before);
    // Markers may not pull the source window before the file starts.
    assert!(
        project
            .set_warp_markers(track, id, vec![marker(0.5, 2.0)])
            .is_err()
    );
}

#[test]
fn split_keeps_markers_on_both_sides_and_preserves_heard_audio() {
    for reverse in [false, true] {
        let (mut project, track, id) = warped(reverse);
        let original = project.clip(track, id).unwrap().clone();
        let right = project.split_clip(track, id, 7.0).unwrap();
        project.validate().unwrap();
        let left = project.clip(track, id).unwrap();
        let right = project.clip(track, right).unwrap();
        assert_eq!(left.warp_markers.len(), 2);
        assert_eq!(right.warp_markers.len(), 2);
        for beat in [4.0, 5.5, 6.9] {
            close(heard(left, beat), heard(&original, beat));
        }
        for beat in [7.1, 9.0, 11.9] {
            close(heard(right, beat), heard(&original, beat));
        }
    }
}

#[test]
fn trim_and_slip_keep_markers_and_follow_the_audio() {
    for reverse in [false, true] {
        let (mut project, track, id) = warped(reverse);
        let original = project.clip(track, id).unwrap().clone();
        project.trim_clip(track, id, 5.0, 6.0).unwrap();
        project.validate().unwrap();
        let trimmed = project.clip(track, id).unwrap();
        assert_eq!(trimmed.warp_markers.len(), 2);
        for beat in [5.0, 7.0, 10.9] {
            close(heard(trimmed, beat), heard(&original, beat));
        }
        // Trimming back out restores the original mapping exactly.
        project.trim_clip(track, id, 4.0, 8.0).unwrap();
        let restored = project.clip(track, id).unwrap();
        for beat in [4.0, 6.0, 8.0, 11.0] {
            close(heard(restored, beat), heard(&original, beat));
        }
        // Slip moves the audio and its markers right by one beat.
        project.slip_audio_clip(track, id, 1.0, 100.0).unwrap();
        project.validate().unwrap();
        let slipped = project.clip(track, id).unwrap();
        for beat in [6.0, 8.0, 10.0] {
            close(heard(slipped, beat + 1.0), heard(&original, beat));
        }
        // It stops at the source start.
        project.slip_audio_clip(track, id, 500.0, 100.0).unwrap();
        let limit = project.clip(track, id).unwrap();
        close(heard(limit, 4.0), if reverse { 100.0 } else { 0.0 });
    }
}

#[test]
fn reverse_mirrors_the_map_and_stretch_scales_it() {
    let (mut project, track, id) = warped(false);
    let forward = project.clip(track, id).unwrap().clone();
    project.reverse_audio_clip(track, id).unwrap();
    let reversed = project.clip(track, id).unwrap();
    assert_eq!(reversed.warp_markers, forward.warp_markers);
    for beat in [4.0, 6.0, 9.0] {
        close(heard(reversed, beat), heard(&forward, 4.0 + 12.0 - beat));
    }
    project.reverse_audio_clip(track, id).unwrap();
    project.stretch_clip(track, id, 16.0).unwrap();
    project.validate().unwrap();
    let stretched = project.clip(track, id).unwrap().clone();
    for local in [0.0, 1.0, 5.0, 8.0] {
        close(
            stretched.local_to_source(local * 2.0),
            forward.local_to_source(local),
        );
    }
    // The warped half-speed segment cannot be slowed further past 25%.
    assert!(project.stretch_clip(track, id, 40.0).is_err());
    assert_eq!(project.clip(track, id).unwrap(), &stretched);
}

#[test]
fn audio_regions_and_duplicates_carry_the_warp() {
    let (mut project, track, id) = warped(false);
    let original = project.clip(track, id).unwrap().clone();
    let region = original.audio_region(6.0, 10.0).unwrap();
    for beat in [6.0, 8.0, 9.9] {
        close(heard(&region, beat), heard(&original, beat));
    }
    let copy = project.duplicate_clip_at(track, id, 20.0).unwrap();
    let copy = project.clip(track, copy).unwrap();
    assert_eq!(copy.warp_markers, original.warp_markers);
    // Ghosts keep the warp local to each occurrence, like source_rate.
    let ghost = project.clone_clip(track, id).unwrap();
    project.set_warp_markers(track, id, Vec::new()).unwrap();
    assert_eq!(
        project.clip(track, ghost).unwrap().warp_markers,
        original.warp_markers
    );
    // Clearing keeps the audio at the clip start and reverts to source_rate.
    let cleared = project.clip(track, id).unwrap();
    close(cleared.offset, original.offset);
    close(cleared.source_end(), original.offset + 8.0);
}

#[test]
fn quantize_moves_hits_toward_the_grid_with_strength_and_threshold() {
    let mut project = Project::new();
    let track = project.add_audio_track("Drums");
    let clip = Clip::audio("Hits", 0.0, 4.0, "hits.wav");
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let clip = project.clip(track, id).unwrap().clone();
    let meter = project.meter_map();
    let settings = AudioQuantizeSettings {
        grid: 1.0,
        strength: 1.0,
        max_move: 0.3,
    };
    let plan = plan_warp_markers(&clip, &[1.1, 1.9, 2.5, 3.05], &meter, Some(settings)).unwrap();
    assert_eq!(plan.moved, 3);
    close(plan.largest_move, 0.1);
    let beats: Vec<_> = plan.markers.iter().map(|marker| marker.beat).collect();
    assert_eq!(beats.len(), 6);
    for (actual, expected) in beats.iter().zip([0.0, 1.0, 2.0, 2.5, 3.0, 4.0]) {
        close(*actual, expected);
    }
    // Source positions stay where the hits are; ends are pinned.
    for (marker, expected) in plan.markers.iter().zip([0.0, 1.1, 1.9, 2.5, 3.05, 4.0]) {
        close(marker.source, expected);
    }
    let half = plan_warp_markers(
        &clip,
        &[1.1],
        &meter,
        Some(AudioQuantizeSettings {
            strength: 0.5,
            ..settings
        }),
    )
    .unwrap();
    close(half.markers[1].beat, 1.05);
    // Without settings markers are placed at the hits without moving them.
    let pinned = plan_warp_markers(&clip, &[1.1, 2.5], &meter, None).unwrap();
    assert_eq!(pinned.moved, 0);
    close(pinned.markers[1].beat, 1.1);

    project.set_warp_markers(track, id, plan.markers).unwrap();
    let quantized = project.clip(track, id).unwrap();
    close(quantized.source_to_local(1.1), 1.0);
    close(quantized.source_end(), 4.0);
    // Meter changes: a one-bar grid follows real bar lines.
    project.set_meter_change(2, 3).unwrap();
    let meter = project.meter_map();
    close(snap_to_grid(&meter, 4.0, 6.4), 7.0);
}

#[test]
fn quantizing_a_reversed_clip_moves_what_is_heard() {
    let mut project = Project::new();
    let track = project.add_audio_track("Drums");
    let mut clip = Clip::audio("Back", 0.0, 4.0, "hits.wav");
    clip.offset = 2.0;
    if let ClipContent::Audio { reverse, .. } = &mut clip.content {
        *reverse = true;
    }
    let id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip.clone());
    let plan = plan_warp_markers(
        &clip,
        &[0.9],
        &project.meter_map(),
        Some(AudioQuantizeSettings {
            grid: 1.0,
            strength: 1.0,
            max_move: 1.0,
        }),
    )
    .unwrap();
    let heard_source = heard(&clip, 0.9);
    project.set_warp_markers(track, id, plan.markers).unwrap();
    let quantized = project.clip(track, id).unwrap();
    close(heard(quantized, 1.0), heard_source);
    close(heard(quantized, 0.0), heard(&clip, 0.0));
    close(heard(quantized, 4.0), heard(&clip, 4.0));
}
