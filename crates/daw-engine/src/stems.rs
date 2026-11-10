//! Stem export renders several delivery files of one song in one operation.
//! Each stem is the song rendered with a single channel soloed through the
//! ordinary mute/solo rules, so its inserts, fader, sends and downstream buses
//! sound exactly as they do in the mix. Every file shares one start, length
//! and format. All files are staged first and moved into place together; a
//! failure or cancellation leaves none of them behind.
use super::*;

#[derive(Clone, Debug)]
pub struct StemRequest {
    pub directory: PathBuf,
    /// Base of every file name, usually the song name.
    pub song: String,
    /// Tracks, buses and folders to render, in any order.
    pub channels: Vec<u64>,
    pub include_mix: bool,
    /// Keep Master inserts, fader and pan on each stem. Off by default: stems
    /// are taken before Master so they can be mixed elsewhere.
    pub through_master: bool,
    /// Without this, an existing file at any destination fails the export
    /// before rendering starts.
    pub replace_existing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StemPlan {
    /// None for the full mix.
    pub channel: Option<u64>,
    pub label: String,
    pub file_name: String,
}

#[derive(Clone, Debug)]
pub struct StemFile {
    pub path: PathBuf,
    pub plan: StemPlan,
    pub report: ExportReport,
}

/// Stems are numbered in project order, after the optional mix as `00`. The
/// number keeps names unique even when channels share a name.
pub fn plan_stems(
    project: &Project,
    song: &str,
    channels: &[u64],
    include_mix: bool,
    format: ExportFormat,
) -> Result<Vec<StemPlan>> {
    let selected: HashSet<u64> = channels.iter().copied().collect();
    ensure!(
        selected.len() == channels.len(),
        "Stem channels must be unique"
    );
    for id in &selected {
        let track = project.track(*id).context("Stem channel is missing")?;
        ensure!(
            track.kind != TrackKind::Automation,
            "Automation lanes cannot be exported as stems"
        );
    }
    ensure!(
        include_mix || !selected.is_empty(),
        "Choose at least one stem to export"
    );
    let song = file_component(song, "Untitled");
    let extension = format.extension();
    let ordered: Vec<_> = project
        .tracks
        .iter()
        .filter(|track| selected.contains(&track.id))
        .collect();
    let width = if ordered.len() >= 100 { 3 } else { 2 };
    let mut plans = Vec::new();
    if include_mix {
        plans.push(StemPlan {
            channel: None,
            label: "Mix".into(),
            file_name: format!("{song} - {:0width$} Mix.{extension}", 0),
        });
    }
    for (index, track) in ordered.iter().enumerate() {
        let name = file_component(&track.name, "Channel");
        plans.push(StemPlan {
            channel: Some(track.id),
            label: track.name.clone(),
            file_name: format!("{song} - {:0width$} {name}.{extension}", index + 1),
        });
    }
    Ok(plans)
}

/// A single path component that is safe on every desktop file system.
fn file_component(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let mut cleaned = cleaned.trim().trim_end_matches('.').trim_end().to_owned();
    // File systems limit names to 255 bytes; leave room for the rest of the name.
    while cleaned.len() > 60 {
        cleaned.pop();
    }
    let cleaned = cleaned.trim_end().trim_end_matches('.').to_owned();
    if cleaned.is_empty() {
        fallback.to_owned()
    } else {
        cleaned
    }
}

/// The project with Master reduced to a transparent sum, unless the stems are
/// meant to pass through it.
fn without_master_processing(mut project: Project) -> Project {
    project.master.inserts.clear();
    project.master.mute = false;
    project.master.input_gain = 1.0;
    project.master.gain = 1.0;
    project.master.pan = 0.0;
    project.master.latency_samples = 0;
    // Lanes of removed Master processors would dangle.
    project.tracks.retain(|track| {
        track
            .automation
            .as_ref()
            .is_none_or(|lane| lane.target.track != 0)
    });
    project
}

fn unmute_with_folders(project: &mut Project, id: u64) {
    let mut next = Some(id);
    while let Some(track) = next.and_then(|id| project.track_mut(id)) {
        track.mute = false;
        next = track.parent_folder;
    }
}

pub fn export_stems(
    project: &Project,
    request: &StemRequest,
    sample_rate: u32,
    options: ExportOptions,
    cancel: &AtomicBool,
) -> Result<Vec<StemFile>> {
    let check_cancelled = || -> Result<()> {
        ensure!(!cancel.load(Ordering::Acquire), "Audio export cancelled");
        Ok(())
    };
    check_cancelled()?;
    options.format.validate(options.encoding, sample_rate)?;
    ensure!(
        request.directory.is_dir(),
        "The stem folder does not exist: {}",
        request.directory.display()
    );
    let plans = plan_stems(
        project,
        &request.song,
        &request.channels,
        request.include_mix,
        options.format,
    )?;
    let destinations: Vec<PathBuf> = plans
        .iter()
        .map(|plan| request.directory.join(&plan.file_name))
        .collect();
    let mut existing = Vec::new();
    for (plan, path) in plans.iter().zip(&destinations) {
        project.validate_audio_destination(path)?;
        if std::fs::symlink_metadata(path).is_ok() {
            existing.push(plan.file_name.as_str());
        }
    }
    ensure!(
        request.replace_existing || existing.is_empty(),
        "Files already exist in the stem folder: {}",
        existing.join(", ")
    );

    // One extent for every file: the longest tail anything in the export has.
    let spb = sample_rate as f64 * 60.0 / project.tempo;
    let (start, end) = if let Some(range) = options.range {
        export_range_samples(project, sample_rate, range)?
    } else if options.include_effect_tails {
        let mut reference = project.clone();
        for track in &mut reference.tracks {
            track.solo = false;
        }
        for &id in &request.channels {
            unmute_with_folders(&mut reference, id);
        }
        let mut variants = vec![if request.through_master {
            reference
        } else {
            without_master_processing(reference)
        }];
        if request.include_mix && !request.through_master {
            variants.push(project.clone());
        }
        let mut end = 0;
        for variant in &variants {
            end = end.max(
                Renderer::new_offline_with_options_cancelled(
                    variant,
                    sample_rate,
                    options.render,
                    cancel,
                )?
                .end_sample,
            );
        }
        (0, end)
    } else {
        (0, (project.end_beat() * spb).round() as u64)
    };
    ensure!(end > start, "Export contains no samples");
    ensure!(
        end <= u64::from(sample_rate) * 60 * 60 * 4,
        "Export exceeds the four-hour processing limit"
    );

    let delivery = options.delivery();
    let mut staged: Vec<(PathBuf, ExportReport)> = Vec::new();
    let result = (|| -> Result<()> {
        for (plan, destination) in plans.iter().zip(&destinations) {
            check_cancelled()?;
            let mut variant = project.clone();
            if let Some(id) = plan.channel {
                for track in &mut variant.tracks {
                    track.solo = false;
                }
                unmute_with_folders(&mut variant, id);
                variant.track_mut(id).expect("planned channel exists").solo = true;
                if !request.through_master {
                    variant = without_master_processing(variant);
                }
            }
            let renderer = Renderer::new_offline_with_options_cancelled(
                &variant,
                sample_rate,
                options.render,
                cancel,
            )?;
            drop(variant);
            staged.push(
                render_to_temp(renderer, destination, start, end, delivery, Some(cancel))
                    .with_context(|| format!("Rendering stem \"{}\"", plan.label))?,
            );
        }
        check_cancelled()
    })();
    if let Err(error) = result {
        for (temp, _) in &staged {
            let _ = std::fs::remove_file(temp);
        }
        return Err(error);
    }

    let mut files = Vec::new();
    let mut created = Vec::new();
    for (index, ((temp, report), (plan, destination))) in staged
        .iter()
        .zip(plans.iter().zip(&destinations))
        .enumerate()
    {
        let existed = std::fs::symlink_metadata(destination).is_ok();
        if let Err(error) = std::fs::rename(temp, destination) {
            for (temp, _) in &staged[index..] {
                let _ = std::fs::remove_file(temp);
            }
            for path in &created {
                let _ = std::fs::remove_file(path);
            }
            return Err(
                anyhow::Error::new(error).context(format!("Writing {}", destination.display()))
            );
        }
        if !existed {
            created.push(destination.clone());
        }
        files.push(StemFile {
            path: destination.clone(),
            plan: plan.clone(),
            report: *report,
        });
    }
    Ok(files)
}
