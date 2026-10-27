//! Bake selected source regions without channel processing. Absolute positions
//! stay intact until rendering, preserving sample rounding on unsnapped clips.
use crate::{
    RenderOptions,
    render::{BLOCK, Renderer, write_wav_cancelled},
};
use anyhow::{Result, ensure};
use daw_core::{AudioData, Project, Track, TrackKind};
use std::{path::Path, sync::atomic::AtomicBool};

fn prepare(
    project: &Project,
    track: u64,
    ids: &[u64],
    sample_rate: u32,
    options: RenderOptions,
    byte_limit: usize,
    cancel: Option<&AtomicBool>,
) -> Result<(Renderer, usize)> {
    ensure!(
        (8_000..=384_000).contains(&sample_rate),
        "Unsupported join sample rate"
    );
    ensure!(
        project.tempo.is_finite() && project.tempo > 0.0,
        "Invalid project tempo"
    );
    let selection = project.audio_join_selection(track, ids)?;
    let frames_per_beat = f64::from(sample_rate) * 60.0 / project.tempo;
    let start = (selection.start * frames_per_beat).round();
    let end = ((selection.start + selection.length) * frames_per_beat).round();
    let frames = end - start;
    ensure!(
        frames >= 1.0 && frames.is_finite(),
        "Join range is shorter than one sample"
    );
    ensure!(
        frames * 2.0 * size_of::<f32>() as f64 <= byte_limit as f64,
        "Joined audio exceeds the {} MiB source limit; join shorter sections",
        byte_limit / 1024 / 1024
    );
    let mut isolated = Project::new();
    isolated.tempo = project.tempo;
    isolated.beats_per_bar = project.beats_per_bar;
    let mut source = Track::new("Join sources", TrackKind::Audio);
    source.gain = 1.0;
    source.clips = selection.clips;
    // Comp membership refers to retained takes in the original document. The
    // isolated render needs only the audible regions and their source windows;
    // detach these copies without changing the document or its alternatives.
    for clip in &mut source.clips {
        clip.audio_take = None;
    }
    isolated.tracks.push(source);
    let mut renderer = match cancel {
        Some(cancel) => {
            Renderer::new_offline_with_options_cancelled(&isolated, sample_rate, options, cancel)?
        }
        None => Renderer::new_offline_with_options(&isolated, sample_rate, options)?,
    };
    renderer.seek(selection.start)?;
    Ok((renderer, frames as usize))
}

/// Bounded convenience renderer, useful for analysis and tests. The UI should
/// prefer `export_audio_join`, which streams to disk without holding the result.
pub fn render_audio_join(
    project: &Project,
    track: u64,
    ids: &[u64],
    sample_rate: u32,
) -> Result<AudioData> {
    let (mut renderer, frames) = prepare(
        project,
        track,
        ids,
        sample_rate,
        RenderOptions::default(),
        256 * 1024 * 1024,
        None,
    )?;
    let mut samples = Vec::new();
    samples.try_reserve_exact(frames * 2)?;
    let mut block = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    let mut remaining = frames;
    while remaining > 0 {
        let count = remaining.min(BLOCK);
        renderer.render(&mut block[..count], &mut positions[..count])?;
        samples.extend(block[..count].iter().flatten().copied());
        remaining -= count;
    }
    Ok(AudioData {
        sample_rate,
        channels: 2,
        samples,
    })
}

/// Atomically write a 32-bit float source at the project/device rate. Its decoded
/// size must fit the renderer's per-source budget so it remains usable afterward.
pub fn export_audio_join(
    project: &Project,
    track: u64,
    ids: &[u64],
    path: impl AsRef<Path>,
    sample_rate: u32,
    options: RenderOptions,
) -> Result<()> {
    export_audio_join_cancelled(
        project,
        track,
        ids,
        path,
        sample_rate,
        options,
        &AtomicBool::new(false),
    )
}

/// Cooperatively cancel source preparation and streamed writing. A cancellation
/// removes the temporary output and never replaces an existing destination.
pub fn export_audio_join_cancelled(
    project: &Project,
    track: u64,
    ids: &[u64],
    path: impl AsRef<Path>,
    sample_rate: u32,
    options: RenderOptions,
    cancel: &AtomicBool,
) -> Result<()> {
    project.validate_audio_destination(path.as_ref())?;
    let (renderer, frames) = prepare(
        project,
        track,
        ids,
        sample_rate,
        options,
        crate::quality::MAX_ASSET_BYTES,
        Some(cancel),
    )?;
    write_wav_cancelled(renderer, path.as_ref(), frames as u64, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::{
        AudioCrossfade, Clip, ClipContent, CrossfadeCurve, Effect, EffectSlot, FadeCurve,
        Instrument, Send,
    };

    #[test]
    fn join_bakes_source_edits_priority_and_crossfade_at_exact_absolute_sample_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("ramp.wav");
        AudioData {
            sample_rate: 8000,
            channels: 2,
            samples: (0..24000)
                .flat_map(|frame| {
                    [
                        0.1 + frame as f32 / 100000.0,
                        -0.05 - frame as f32 / 200000.0,
                    ]
                })
                .collect(),
        }
        .write_wav(&source)
        .unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Source");
        project.tempo = 120.0;
        project.track_mut(track).unwrap().gain = 1.0;
        let mut a = Clip::audio("A", 1.00015, 2.005, source.to_string_lossy());
        a.offset = 0.2;
        let mut b = Clip::audio("B", 2.25005, 2.0, source.to_string_lossy());
        b.offset = 0.5;
        if let ClipContent::Audio {
            gain,
            fade_in,
            fade_out,
            fade_in_curve,
            ..
        } = &mut a.content
        {
            *gain = 0.5;
            *fade_in = 0.2;
            *fade_out = 0.3;
            *fade_in_curve = FadeCurve::SCurve;
        }
        if let ClipContent::Audio {
            gain,
            reverse,
            crossfade,
            ..
        } = &mut b.content
        {
            *gain = 0.75;
            *reverse = true;
            *crossfade = Some(AudioCrossfade {
                duration: 0.5,
                curve: CrossfadeCurve::SMOOTH,
            });
        }
        let ids = [a.id, b.id];
        let start_frame = (a.start * 4000.0).round() as usize;
        let end_frame = (b.end() * 4000.0).round() as usize;
        project.track_mut(track).unwrap().clips = vec![a, b];
        let expected = crate::render(&project, 8000, 5.0).unwrap();
        let mut dry = project.clone();
        // These controls and nonexistent plugins must never enter consolidation.
        let bus = project.add_bus("Bus");
        let source_track = project.track_mut(track).unwrap();
        source_track.gain = 0.1;
        source_track.pan = -0.8;
        source_track.mute = true;
        source_track.latency_samples = 777;
        source_track.output_bus = Some(bus);
        source_track.sends.push(Send {
            target_bus: bus,
            gain: 0.5,
            pre_fader: true,
        });
        source_track.instrument = Instrument::Vst3 {
            path: "missing-instrument.vst3".into(),
            class_id: None,
        };
        source_track.inserts.push(EffectSlot::new(
            "Missing",
            Effect::Vst3 {
                path: "missing-effect.vst3".into(),
                class_id: None,
            },
        ));
        let before = project.clone();
        let joined = render_audio_join(&project, track, &ids, 8000).unwrap();
        assert_eq!(joined.frames(), end_frame - start_frame);
        assert!(
            joined
                .samples
                .iter()
                .zip(&expected[start_frame * 2..end_frame * 2])
                .all(|(a, b)| (a - b).abs() < 1e-6)
        );
        let output = directory.path().join("joined.wav");
        export_audio_join(
            &project,
            track,
            &ids,
            &output,
            8000,
            RenderOptions::default(),
        )
        .unwrap();
        assert_eq!(
            AudioData::load_wav(&output).unwrap().samples,
            joined.samples
        );
        assert_eq!(project, before);
        project
            .replace_audio_clips_with_joined(track, &ids, output.to_string_lossy())
            .unwrap();
        let channel = project.track(track).unwrap();
        assert_eq!(
            (
                channel.gain,
                channel.pan,
                channel.mute,
                channel.latency_samples
            ),
            (0.1, -0.8, true, 777)
        );
        assert_eq!(channel.inserts, before.track(track).unwrap().inserts);
        dry.track_mut(track).unwrap().clips = channel.clips.clone();
        assert_eq!(crate::render(&dry, 8000, 5.0).unwrap(), expected);
        let archive = directory.path().join("unsnapped-join.rill");
        dry.save(&archive).unwrap();
        std::fs::remove_file(&output).unwrap();
        let reopened = Project::load(&archive).unwrap();
        assert_eq!(crate::render(&reopened, 8000, 5.0).unwrap(), expected);
    }

    #[test]
    fn joining_split_comp_regions_preserves_source_windows_and_retained_takes() {
        let directory = tempfile::tempdir().unwrap();
        let mut project = Project::new();
        project.tempo = 120.0;
        let track = project.add_audio_track("Loop takes");
        project.track_mut(track).unwrap().gain = 1.0;
        for take in 0..2 {
            let path = directory.path().join(format!("take-{take}.wav"));
            AudioData {
                sample_rate: 8000,
                channels: 2,
                samples: (0..24000)
                    .flat_map(|frame| {
                        let value = 0.1 + take as f32 * 0.2 + frame as f32 / 100000.0;
                        [value, -value * 0.5]
                    })
                    .collect(),
            }
            .write_wav(&path)
            .unwrap();
            let mut clip = Clip::audio(format!("Take {take}"), 1.0, 4.0, path.to_string_lossy());
            clip.offset = 0.5;
            if let ClipContent::Audio {
                gain,
                reverse,
                fade_in,
                fade_out,
                ..
            } = &mut clip.content
            {
                *gain = 0.75;
                *reverse = true;
                *fade_in = 0.25;
                *fade_out = 0.5;
            }
            project.track_mut(track).unwrap().clips.push(clip);
        }
        let sources: Vec<_> = project
            .track(track)
            .unwrap()
            .clips
            .iter()
            .map(|clip| clip.id)
            .collect();
        let group = project.group_audio_takes(track, &sources).unwrap();
        let left = project.audio_take_regions(track, group)[0].id;
        let right = project.split_clip(track, left, 3.0).unwrap();
        project.validate().unwrap();
        let before = project.clone();
        let retained: Vec<_> = project
            .audio_take_sources(track, group)
            .into_iter()
            .cloned()
            .collect();
        let expected = crate::render(&project, 8000, 6.0).unwrap();
        let joined = render_audio_join(&project, track, &[left, right], 8000).unwrap();
        assert_eq!(joined.frames(), 16000);
        assert_eq!(joined.samples, expected[4000 * 2..20000 * 2]);
        assert_eq!(project, before);
        let path = directory.path().join("joined.wav");
        export_audio_join(
            &project,
            track,
            &[left, right],
            &path,
            8000,
            RenderOptions::default(),
        )
        .unwrap();
        assert_eq!(AudioData::load_wav(&path).unwrap().samples, joined.samples);
        assert_eq!(project, before);
        let id = project
            .replace_audio_clips_with_joined(track, &[left, right], path.to_string_lossy())
            .unwrap();
        assert!(project.clip(track, id).unwrap().audio_take.is_none());
        assert_eq!(
            project
                .audio_take_sources(track, group)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
            retained
        );
        project.validate().unwrap();
        assert_eq!(crate::render(&project, 8000, 6.0).unwrap(), expected);
    }

    #[test]
    fn failed_or_oversized_join_preserves_destination_and_document() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("keep.wav");
        std::fs::write(&output, b"previous source").unwrap();
        let mut project = Project::new();
        let track = project.add_audio_track("Missing");
        let a = Clip::audio("A", 0.0, 1.0, "missing.wav");
        let b = Clip::audio("B", 1.0, 1.0, "missing.wav");
        let ids = [a.id, b.id];
        project.track_mut(track).unwrap().clips = vec![a, b];
        let before = project.clone();
        assert!(
            export_audio_join(
                &project,
                track,
                &ids,
                &output,
                48000,
                RenderOptions::default()
            )
            .is_err()
        );
        assert_eq!(project, before);
        assert_eq!(std::fs::read(&output).unwrap(), b"previous source");
        // Cancellation is checked before attempting to open a missing source.
        let error = export_audio_join_cancelled(
            &project,
            track,
            &ids,
            &output,
            48000,
            RenderOptions::default(),
            &AtomicBool::new(true),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(std::fs::read(&output).unwrap(), b"previous source");
        project.track_mut(track).unwrap().clips[1].length = 100000.0;
        let error = export_audio_join(
            &project,
            track,
            &ids,
            &output,
            48000,
            RenderOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("512 MiB"), "{error}");
        assert_eq!(std::fs::read(output).unwrap(), b"previous source");
    }
}
