//! A reversible freeze prints selected source material through the existing
//! channel DSP, before volume/pan and routing. One graph owns every instrument
//! output throughout the pass; randomized instruments are never re-instantiated
//! separately for their outputs.
use super::*;
use std::{fs::OpenOptions, io::BufWriter, path::PathBuf};

#[derive(Clone, Debug)]
pub struct FreezeRenderRequest {
    pub track: u64,
    /// None prints all unmuted source clips; Some prints exactly these clips.
    pub clips: Option<Vec<u64>>,
    pub include_tails: bool,
}

#[derive(Clone, Debug)]
pub struct FreezeRenderOutput {
    pub source_track: u64,
    pub path: PathBuf,
    pub start_beat: f64,
    pub length_beats: f64,
    pub frames: u64,
    pub sample_rate: u32,
}

#[derive(Clone, Debug)]
pub struct FreezeRenderReport {
    pub outputs: Vec<FreezeRenderOutput>,
}

/// Render to new managed files inside the caller's private staging directory.
/// Files are never replaced; every file is finalized and synced before success.
/// Failed/cancelled batches remove all their files. The application commits the
/// returned group and source mutes together only after validating the report.
pub fn freeze_track_wav(
    project: &Project,
    directory: &Path,
    request: &FreezeRenderRequest,
    sample_rate: u32,
    options: RenderOptions,
    cancel: &AtomicBool,
) -> Result<FreezeRenderReport> {
    let check_cancelled = || -> Result<()> {
        ensure!(!cancel.load(Ordering::Acquire), "Freeze cancelled");
        Ok(())
    };
    check_cancelled()?;
    project.validate()?;
    ensure!(
        directory.is_dir(),
        "Freeze staging directory is unavailable"
    );
    let source = project
        .track(request.track)
        .context("Freeze source channel is missing")?;
    ensure!(
        matches!(source.kind, TrackKind::Audio | TrackKind::Midi),
        "Freeze requires an audio or MIDI source track"
    );
    let requested: Option<HashSet<_>> = request
        .clips
        .as_ref()
        .map(|clips| clips.iter().copied().collect());
    if let Some(ids) = &requested {
        ensure!(
            !ids.is_empty() && Some(ids.len()) == request.clips.as_ref().map(Vec::len),
            "Freeze requires unique source clip IDs"
        );
        ensure!(
            ids.iter().all(|id| source
                .clips
                .iter()
                .any(|clip| clip.id == *id && !clip.muted && !clip.is_take_source())),
            "Freeze source clip is missing or muted"
        );
    }
    let selected: Vec<_> = source
        .clips
        .iter()
        .filter(|clip| {
            !clip.muted
                && !clip.is_take_source()
                && requested.as_ref().is_none_or(|ids| ids.contains(&clip.id))
        })
        .collect();
    ensure!(!selected.is_empty(), "There are no unmuted clips to freeze");
    if source.kind == TrackKind::Audio && requested.is_some() {
        for clip in &selected {
            ensure!(
                !source.clips.iter().any(|other| !other.muted
                    && !other.is_take_source()
                    && !selected.iter().any(|selected| selected.id == other.id)
                    && other.start < clip.end()
                    && other.end() > clip.start),
                "Overlapping audio clips must be frozen together with Freeze track"
            );
        }
    }
    let start_beat = selected
        .iter()
        .map(|clip| clip.start)
        .fold(f64::INFINITY, f64::min);
    let end_beat = selected
        .iter()
        .map(|clip| clip.end())
        .fold(0.0_f64, f64::max);
    let mut output_ids = vec![source.id];
    output_ids.extend(
        project
            .tracks
            .iter()
            .filter(|track| {
                track
                    .instrument_output
                    .is_some_and(|output| output.track == source.id)
            })
            .map(|track| track.id),
    );
    ensure!(
        output_ids.len() <= 129,
        "Freeze has too many instrument outputs"
    );
    let mut prepared = project.clone();
    // Freeze ignores unrelated solos and the selected group's presentation mute
    // state. Other sidechain sources retain their mute and folder semantics.
    for track in &mut prepared.tracks {
        track.solo = false;
    }
    for &id in &output_ids {
        let track = prepared.track_mut(id).expect("freeze channel exists");
        track.mute = false;
        track.solo = true;
        track.output_bus = None;
        track.sends.clear();
        let mut parent = track.parent_folder;
        while let Some(folder) = parent.and_then(|id| prepared.track_mut(id)) {
            folder.mute = false;
            parent = folder.parent_folder;
        }
    }
    if let Some(ids) = requested {
        for clip in &mut prepared.track_mut(source.id).unwrap().clips {
            if !clip.is_take_source() && !ids.contains(&clip.id) {
                clip.muted = true;
            }
        }
    }
    prepared.master.inserts.clear();
    prepared.master.mute = false;
    prepared.master.input_gain = 1.0;
    prepared.master.gain = 1.0;
    prepared.master.pan = 0.0;
    prepared.master.latency_samples = 0;
    // Removed Master processors may have automation lanes. They are irrelevant
    // to this pre-Master print and cannot remain dangling during validation.
    prepared.tracks.retain(|track| {
        track.automation.as_ref().is_none_or(|lane| {
            lane.target.track != 0
                && !(output_ids.contains(&lane.target.track)
                    && matches!(lane.target.parameter, AutomationParameter::SendGain { .. }))
        })
    });
    let mut renderer = Renderer::new_inner(
        &prepared,
        sample_rate,
        false,
        None,
        options,
        Some(cancel),
        ProcessingMode::Offline,
        Some(&output_ids),
    )?;
    let samples_per_beat = sample_rate as f64 * 60.0 / project.tempo;
    let start = (start_beat * samples_per_beat).round() as u64;
    let base_end = (end_beat * samples_per_beat).round() as u64;
    let tail = if request.include_tails {
        renderer
            .tracks
            .iter()
            .flatten()
            .filter(|track| output_ids.contains(&track.id))
            .map(|track| track.tail_samples)
            .max()
            .unwrap_or(0)
    } else {
        0
    };
    let end = base_end
        .checked_add(tail)
        .context("Freeze duration is too large")?;
    let frames = end.saturating_sub(start);
    ensure!(
        frames > 0 && end <= u64::from(sample_rate) * 60 * 60 * 4,
        "Freeze exceeds the supported duration"
    );
    ensure!(
        frames
            .checked_mul(8)
            .and_then(|bytes| bytes.checked_mul(output_ids.len() as u64))
            .is_some_and(|bytes| bytes <= 512 * 1024 * 1024),
        "Freeze output exceeds the 512 MiB batch limit"
    );
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut created = Vec::new();
    let result = (|| -> Result<FreezeRenderReport> {
        let mut writers = Vec::new();
        for &id in &output_ids {
            check_cancelled()?;
            let path = directory.join(format!("channel-{id}.wav"));
            project.validate_audio_destination(&path)?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .context("Create freeze audio")?;
            created.push(path.clone());
            let sync = file.try_clone()?;
            writers.push((
                id,
                path,
                hound::WavWriter::new(BufWriter::new(file), spec)?,
                sync,
            ));
        }
        let mut block = [[0.0; 2]; BLOCK];
        let mut positions = [0.0; BLOCK];
        let mut rendered = 0;
        while rendered < end {
            check_cancelled()?;
            let count = (end - rendered).min(BLOCK as u64) as usize;
            renderer.render(&mut block[..count], &mut positions[..count])?;
            let first = start.saturating_sub(rendered).min(count as u64) as usize;
            for (id, _, writer, _) in &mut writers {
                let tap = renderer
                    .tracks
                    .iter()
                    .flatten()
                    .find(|track| track.id == *id)
                    .and_then(|track| track.freeze_tap.as_ref())
                    .context("Freeze capture channel is unavailable")?;
                for frame in &tap.frames[first..count] {
                    ensure!(
                        frame[0].is_finite() && frame[1].is_finite(),
                        "Freeze produced non-finite samples"
                    );
                    writer.write_sample(frame[0])?;
                    writer.write_sample(frame[1])?;
                }
            }
            rendered += count as u64;
        }
        let mut outputs = Vec::new();
        for (id, path, writer, sync) in writers {
            check_cancelled()?;
            writer.finalize()?;
            sync.sync_all()?;
            outputs.push(FreezeRenderOutput {
                source_track: id,
                path,
                start_beat,
                length_beats: frames as f64 / samples_per_beat,
                frames,
                sample_rate,
            });
        }
        check_cancelled()?;
        Ok(FreezeRenderReport { outputs })
    })();
    if result.is_err() {
        for path in created {
            let _ = std::fs::remove_file(path);
        }
    }
    result
}
