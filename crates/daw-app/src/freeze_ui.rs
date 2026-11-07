//! Freeze prints into private managed media, then publishes one undoable edit.
//! Original clips and plug-in state remain in the song until the user removes them.
use crate::app::{DawApp, WorkspaceView};
use anyhow::{Context, Result, bail, ensure};
use daw_core::{AudioData, AutomationParameter, Clip, MAX_BEAT, Project, Track, TrackKind};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const DECODED_BUDGET: u64 = 512 * 1024 * 1024;
const PREVIEW_BUDGET: usize = 128 * 1024 * 1024;

struct Request {
    session: u64,
    project: Arc<Project>,
    track: u64,
    clips: Vec<u64>,
    whole_track: bool,
    sample_rate: u32,
    options: daw_engine::RenderOptions,
    scratch: PathBuf,
}

struct Active {
    request: Request,
    receiver: mpsc::Receiver<Result<Rendered>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
}

#[derive(Default)]
pub(crate) struct FreezeState {
    active: Option<Active>,
    preparing: Option<u64>,
    cancel_preparing: bool,
    pointer_was_down: bool,
    last_error_status: Option<String>,
    completed: serde_json::Value,
}

impl Drop for FreezeState {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Release);
        }
    }
}

struct Assets {
    directory: PathBuf,
    retained: bool,
}

impl Assets {
    fn create(scratch: &Path) -> Result<Self> {
        let root = scratch.join("freezes");
        fs::create_dir_all(&root).context("Creating managed Freeze folder")?;
        for _ in 0..16 {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let directory = root.join(format!(
                "freeze-{}-{nonce}-{}",
                std::process::id(),
                daw_core::new_id()
            ));
            match fs::create_dir(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        directory,
                        retained: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("Creating private Freeze staging folder"),
            }
        }
        bail!("Could not create a private Freeze folder")
    }
}

impl Drop for Assets {
    fn drop(&mut self) {
        if !self.retained {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

struct Rendered {
    assets: Assets,
    outputs: Vec<daw_engine::FreezeRenderOutput>,
    previews: HashMap<PathBuf, AudioData>,
}

fn source_clips(project: &Project, track: u64, selected: Option<&[u64]>) -> Result<Vec<u64>> {
    let source = project
        .track(track)
        .context("Freeze source no longer exists")?;
    ensure!(
        matches!(source.kind, TrackKind::Audio | TrackKind::Midi),
        "Freeze an audio or MIDI source track"
    );
    ensure!(
        source.kind != TrackKind::Midi || !matches!(source.instrument, daw_core::Instrument::None),
        "Assign an instrument before freezing MIDI"
    );
    let ids: Vec<u64> = if let Some(selected) = selected {
        ensure!(!selected.is_empty(), "Select a clip to freeze");
        let mut seen = HashSet::new();
        for id in selected {
            ensure!(
                seen.insert(*id),
                "Freeze selection contains a duplicate clip"
            );
            ensure!(
                source
                    .clips
                    .iter()
                    .any(|clip| clip.id == *id && !clip.muted && !clip.is_take_source()),
                "Freeze selection contains a missing or muted clip"
            );
        }
        source
            .clips
            .iter()
            .filter(|clip| seen.contains(&clip.id))
            .map(|clip| clip.id)
            .collect()
    } else {
        source
            .clips
            .iter()
            .filter(|clip| !clip.muted && !clip.is_take_source())
            .map(|clip| clip.id)
            .collect()
    };
    ensure!(!ids.is_empty(), "The track has no unmuted clips to freeze");
    Ok(ids)
}

fn validate_clip_isolation(project: &Project, track: u64, selected: &[u64]) -> Result<()> {
    let source = project
        .track(track)
        .context("Freeze source no longer exists")?;
    if source.kind == TrackKind::Audio {
        for clip in source
            .clips
            .iter()
            .filter(|clip| selected.contains(&clip.id))
        {
            ensure!(
                !source.clips.iter().any(|other| !other.muted
                    && !other.is_take_source()
                    && !selected.contains(&other.id)
                    && clip.start < other.end()
                    && other.start < clip.end()),
                "Selected audio overlaps another clip; use Freeze track to preserve overlap playback"
            );
        }
    }
    Ok(())
}

fn render(request: &Request, cancel: &AtomicBool) -> Result<Rendered> {
    let assets = Assets::create(&request.scratch)?;
    // A child-scoped delay makes cancellation/stale-document E2Es deterministic;
    // ordinary launches never consult the test-only setting.
    if std::env::var_os("RILL_AUTOMATION_DIR").is_some() {
        let delay = std::env::var("RILL_E2E_FREEZE_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            .min(5_000);
        let deadline = Instant::now() + Duration::from_millis(delay);
        while Instant::now() < deadline {
            ensure!(!cancel.load(Ordering::Acquire), "Freeze canceled");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let report = daw_engine::freeze_track_wav(
        &request.project,
        &assets.directory,
        &daw_engine::FreezeRenderRequest {
            track: request.track,
            clips: Some(request.clips.clone()),
            include_tails: true,
        },
        request.sample_rate,
        request.options,
        cancel,
    )?;
    ensure!(
        !report.outputs.is_empty(),
        "Freeze produced no audio channels"
    );
    let root = assets.directory.canonicalize()?;
    let mut seen = HashSet::new();
    let mut decoded = 0u64;
    let mut preview_budget = PREVIEW_BUDGET;
    let mut previews = HashMap::new();
    for output in &report.outputs {
        ensure!(!cancel.load(Ordering::Acquire), "Freeze canceled");
        ensure!(
            seen.insert(output.source_track),
            "Freeze returned duplicate channels"
        );
        let channel = request
            .project
            .track(output.source_track)
            .context("Freeze returned an unknown source channel")?;
        ensure!(
            output.source_track == request.track
                || channel
                    .instrument_output
                    .as_ref()
                    .is_some_and(|output| output.track == request.track),
            "Freeze returned an unrelated source channel"
        );
        let canonical = output
            .path
            .canonicalize()
            .context("Freeze audio is missing")?;
        ensure!(
            canonical.parent() == Some(root.as_path()),
            "Invalid managed Freeze path"
        );
        ensure!(
            output.start_beat.is_finite()
                && output.start_beat >= 0.0
                && output.length_beats.is_finite()
                && output.length_beats > 0.0
                && output.start_beat + output.length_beats <= MAX_BEAT
                && output.frames > 0
                && output.sample_rate == request.sample_rate,
            "Freeze returned an invalid audio range"
        );
        let bytes = output
            .frames
            .checked_mul(8)
            .context("Freeze audio is too large")?;
        decoded = decoded
            .checked_add(bytes)
            .context("Freeze audio is too large")?;
        ensure!(
            decoded <= DECODED_BUDGET,
            "Freeze exceeds the 512 MiB audio budget"
        );
        let mut reader = hound::WavReader::open(&output.path).context("Checking frozen audio")?;
        let spec = reader.spec();
        ensure!(
            spec.channels == 2
                && spec.sample_rate == output.sample_rate
                && spec.bits_per_sample == 32
                && spec.sample_format == hound::SampleFormat::Float
                && u64::from(reader.duration()) == output.frames,
            "Freeze audio metadata does not match the rendered result"
        );
        for (index, sample) in reader.samples::<f32>().enumerate() {
            if index % 4096 == 0 {
                ensure!(!cancel.load(Ordering::Acquire), "Freeze canceled");
            }
            ensure!(sample?.is_finite(), "Freeze produced invalid audio samples");
        }
        if bytes < daw_core::AUDIO_STREAM_THRESHOLD_BYTES
            && usize::try_from(bytes).is_ok_and(|bytes| bytes <= preview_budget)
        {
            let preview = AudioData::load_wav_with_limit(&output.path, preview_budget)?;
            preview_budget -= preview.samples.len() * std::mem::size_of::<f32>();
            previews.insert(output.path.clone(), preview);
        }
    }
    ensure!(
        seen.contains(&request.track),
        "Freeze omitted the main source channel"
    );
    for child in request.project.tracks.iter().filter(|track| {
        track
            .instrument_output
            .as_ref()
            .is_some_and(|output| output.track == request.track)
    }) {
        ensure!(
            seen.contains(&child.id),
            "Freeze omitted an instrument output"
        );
    }
    Ok(Rendered {
        assets,
        outputs: report.outputs,
        previews,
    })
}

fn clone_mix_automation(source: &Track, target: u64) -> Option<Track> {
    let lane = source.automation.as_ref()?;
    if !matches!(
        lane.target.parameter,
        AutomationParameter::TrackGain
            | AutomationParameter::TrackPan
            | AutomationParameter::SendGain { .. }
    ) {
        return None;
    }
    let mut copied = source.clone();
    copied.id = daw_core::new_id();
    let lane = copied.automation.as_mut().unwrap();
    lane.target.track = target;
    for point in &mut lane.points {
        point.id = daw_core::new_id();
    }
    let mut groups = HashMap::new();
    for clip in &mut lane.clips {
        clip.id = daw_core::new_id();
        clip.link_group = clip
            .link_group
            .map(|id| *groups.entry(id).or_insert_with(daw_core::new_id));
        for point in &mut clip.points {
            point.id = daw_core::new_id();
        }
    }
    Some(copied)
}

fn unfreeze_owner(project: &Project, track: u64) -> Option<u64> {
    // A re-frozen print returns one generation at a time. Otherwise any output
    // from the same instrument resolves to its shared original source.
    if project
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .any(|clip| {
            clip.freeze
                .as_ref()
                .is_some_and(|link| link.active && link.source_track == track)
        })
    {
        return Some(track);
    }
    project.track(track)?.clips.iter().find_map(|clip| {
        clip.freeze
            .as_ref()
            .filter(|link| link.active)
            .map(|link| link.source_track)
    })
}

fn copy_sidechain_routes(
    track: u64,
    effect: Option<u64>,
    routes: &mut Vec<daw_core::PluginInputRoute>,
    replacements: &HashMap<u64, u64>,
    recorded: &mut Vec<daw_core::FreezeInputCopy>,
) {
    let copied: Vec<_> = routes
        .iter()
        .filter_map(|route| {
            let mut copied = route.clone();
            copied.source_track = *replacements.get(&route.source_track)?;
            recorded.push(daw_core::FreezeInputCopy {
                track,
                effect,
                route: copied.clone(),
            });
            Some(copied)
        })
        .collect();
    routes.extend(copied);
}

impl DawApp {
    pub(crate) fn freeze_busy(&self) -> bool {
        self.freeze.preparing.is_some() || self.freeze.active.is_some()
    }

    pub(crate) fn can_freeze(&self, track: u64, clips: Option<&[u64]>) -> bool {
        !self.freeze_busy()
            && !self.recording_busy()
            && !self.audio_import_busy()
            && !self.project_io_pending()
            && !self.state_sync_pending()
            && self.export_job.is_none()
            && self.document_guard.is_none()
            && source_clips(&self.project, track, clips).is_ok()
    }

    pub(crate) fn request_freeze(&mut self, track: u64, clips: Option<Vec<u64>>) {
        let result = (|| -> Result<()> {
            ensure!(!self.freeze_busy(), "A Freeze render is already running");
            ensure!(
                !self.recording_busy(),
                "Stop recording before freezing audio"
            );
            ensure!(
                !self.audio_import_busy()
                    && !self.project_io_pending()
                    && !self.state_sync_pending()
                    && self.document_guard.is_none()
                    && self.export_job.is_none(),
                "Finish the current project operation before freezing audio"
            );
            let ids = source_clips(&self.project, track, clips.as_deref())?;
            validate_clip_isolation(&self.project, track, &ids)?;
            self.flush_pending_edit();
            self.desired_playing = false;
            self.audition_request = None;
            if let Some(engine) = &self.engine {
                engine.set_playing(false);
                engine.audition_note(None)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.freeze_error(error);
            return;
        }
        if self.freeze.last_error_status.as_deref() == Some(self.status.as_str()) {
            self.error = false;
        }
        self.freeze.last_error_status = None;
        self.freeze.preparing = Some(self.song_session.id);
        self.freeze.cancel_preparing = false;
        self.request_freeze_snapshot(track, clips);
    }

    /// Called only after the acknowledged native-editor snapshot barrier.
    pub(crate) fn begin_freeze(&mut self, track: u64, clips: Option<Vec<u64>>) {
        let preparing = self.freeze.preparing.take();
        let cancelled = std::mem::take(&mut self.freeze.cancel_preparing);
        if cancelled || preparing.is_some_and(|session| session != self.song_session.id) {
            if !self.error {
                self.status = "Freeze canceled · Original clips kept".into();
            }
            return;
        }
        let result = (|| -> Result<Request> {
            ensure!(!self.freeze_busy(), "A Freeze render is already running");
            ensure!(
                !self.recording_busy(),
                "Stop recording before freezing audio"
            );
            let ids = source_clips(&self.project, track, clips.as_deref())?;
            validate_clip_isolation(&self.project, track, &ids)?;
            self.flush_pending_edit();
            Ok(Request {
                session: self.song_session.id,
                project: Arc::new(self.project.clone()),
                track,
                clips: ids,
                whole_track: clips.is_none(),
                sample_rate: self.settings.sample_rate,
                options: daw_engine::RenderOptions {
                    resample_quality: self.settings.resample_quality,
                },
                scratch: self.settings.scratch_dir.clone(),
            })
        })();
        let request = match result {
            Ok(request) => request,
            Err(error) => {
                self.freeze_error(error);
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_request = Request {
            project: Arc::clone(&request.project),
            clips: request.clips.clone(),
            scratch: request.scratch.clone(),
            ..request
        };
        match std::thread::Builder::new()
            .name("rill-freeze".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    render(&worker_request, &worker_cancel)
                }))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "Freeze worker panicked; no clips were changed"
                    ))
                });
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.freeze.completed = serde_json::Value::Null;
                self.freeze.active = Some(Active {
                    request,
                    receiver,
                    cancel,
                    started: Instant::now(),
                });
                if !self.error {
                    self.status = "Freezing audio in background…".into();
                }
            }
            Err(error) => self.freeze_error(error.into()),
        }
    }

    pub(crate) fn freeze_progress_message(&self) -> Option<String> {
        if self.freeze.preparing.is_some() {
            return Some(
                if self.freeze.cancel_preparing {
                    "Cancelling Freeze · Finishing plug-in state…"
                } else {
                    "Preparing Freeze · Saving plug-in state…"
                }
                .into(),
            );
        }
        let active = self.freeze.active.as_ref()?;
        let name = &active.request.project.track(active.request.track)?.name;
        Some(format!(
            "Freezing · {name} · {} s elapsed{}",
            active.started.elapsed().as_secs(),
            if active.cancel.load(Ordering::Acquire) {
                " · Cancelling…"
            } else {
                ""
            }
        ))
    }

    pub(crate) fn cancel_freeze(&mut self) {
        if self.freeze.preparing.is_some() {
            self.freeze.cancel_preparing = true;
        }
        if let Some(active) = &self.freeze.active {
            active.cancel.store(true, Ordering::Release);
            if !self.error {
                self.status = "Cancelling Freeze…".into();
            }
        }
    }

    fn freeze_error(&mut self, error: anyhow::Error) {
        self.message(
            Err(error.context("Freeze failed; original clips were kept")),
            "",
        );
        self.freeze.last_error_status = Some(self.status.clone());
    }

    pub(crate) fn poll_freeze(&mut self, ctx: &egui::Context) {
        if self.freeze.preparing.is_some()
            && !self.state_sync_pending()
            && self.freeze.active.is_none()
        {
            // A rejected/crashed native state barrier must not strand the
            // progress state or publish a print from an unacknowledged preset.
            self.freeze.preparing = None;
            let cancelled = std::mem::take(&mut self.freeze.cancel_preparing);
            if !self.error {
                if cancelled {
                    self.status = "Freeze canceled · Original clips kept".into();
                } else {
                    self.freeze_error(anyhow::anyhow!("Plug-in state could not be synchronized"));
                }
            }
        }
        let pointer_down = ctx.input(|input| input.pointer.any_down());
        let defer = pointer_down || self.freeze.pointer_was_down || self.recording_busy();
        self.freeze.pointer_was_down = pointer_down;
        if let Some(active) = self.freeze.active.take() {
            if defer && !active.cancel.load(Ordering::Acquire) {
                self.freeze.active = Some(active);
            } else {
                match active.receiver.try_recv() {
                    Ok(result) => {
                        if active.cancel.load(Ordering::Acquire) {
                            drop(result);
                            if active.request.session == self.song_session.id && !self.error {
                                self.status = "Freeze canceled · Original clips kept".into();
                            }
                        } else if active.request.session != self.song_session.id {
                            drop(result);
                        } else if let Err(error) = result
                            .and_then(|rendered| self.commit_freeze(&active.request, rendered))
                        {
                            self.freeze_error(error);
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => self.freeze.active = Some(active),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if !active.cancel.load(Ordering::Acquire) {
                            self.freeze_error(anyhow::anyhow!(
                                "Freeze worker stopped unexpectedly"
                            ));
                        }
                    }
                }
            }
        }
        if self.freeze_busy() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn commit_freeze(&mut self, request: &Request, mut rendered: Rendered) -> Result<()> {
        self.flush_pending_edit();
        ensure!(
            request.session == self.song_session.id && self.project == *request.project,
            "The song changed while freezing; retry Freeze to render the current edits"
        );
        let mut candidate = self.project.clone();
        rendered.outputs.sort_by_key(|output| {
            (
                output.source_track != request.track,
                request
                    .project
                    .tracks
                    .iter()
                    .position(|track| track.id == output.source_track)
                    .unwrap_or(usize::MAX),
            )
        });
        let source = candidate
            .track(request.track)
            .context("Freeze source was removed")?;
        let clip_name = if request.clips.len() == 1 {
            source
                .clips
                .iter()
                .find(|clip| clip.id == request.clips[0])
                .map(|clip| clip.name.clone())
                .unwrap_or_else(|| source.name.clone())
        } else {
            source.name.clone()
        };
        let captured: HashSet<_> = rendered
            .outputs
            .iter()
            .map(|output| output.source_track)
            .collect();
        let insertion = candidate
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| captured.contains(&track.id))
            .map(|(index, _)| index + 1)
            .max()
            .context("Freeze source is not an arranger track")?;
        let mut new_tracks = Vec::new();
        let mut automation = Vec::new();
        let mut selected = None;
        let mut completed = Vec::new();
        let mut replacements = HashMap::new();
        let group = daw_core::new_id();
        for clip in &mut candidate.track_mut(request.track).unwrap().clips {
            if request.clips.contains(&clip.id) {
                clip.muted = true;
            }
        }
        let mute_original_channels = candidate
            .track(request.track)
            .unwrap()
            .clips
            .iter()
            .filter(|clip| !clip.is_take_source())
            .all(|clip| clip.muted);
        let mut muted_tracks = Vec::new();
        if mute_original_channels {
            for source in &captured {
                let track = candidate.track_mut(*source).unwrap();
                if !track.mute {
                    muted_tracks.push(daw_core::FreezeTrackMute {
                        track: *source,
                        muted: false,
                    });
                    track.mute = true;
                }
            }
            muted_tracks.sort_by_key(|state| state.track);
        }
        for output in &rendered.outputs {
            let original = request
                .project
                .track(output.source_track)
                .context("Missing frozen channel")?;
            let mut track = Track::new(format!("{} · Frozen", original.name), TrackKind::Audio);
            track.color = original.color;
            track.parent_folder = original.parent_folder;
            track.input_gain = 1.0;
            track.gain = original.gain;
            track.pan = original.pan;
            track.mute = original.mute;
            track.solo = original.solo;
            track.output_bus = original.output_bus;
            track.sends = original.sends.clone();
            let name = if output.source_track == request.track {
                clip_name.clone()
            } else {
                original.name.clone()
            };
            let path = output.path.to_str().context(
                "Managed Freeze folder is not valid Unicode; choose another Storage folder",
            )?;
            let clip = Clip::audio(name, output.start_beat, output.length_beats, path);
            replacements.insert(output.source_track, track.id);
            selected.get_or_insert((track.id, clip.id));
            completed.push(serde_json::json!({"source_track":output.source_track,"track":track.id,"clip":clip.id,"path":path,
                "start_beat":output.start_beat,"length_beats":output.length_beats,"frames":output.frames}));
            track.clips.push(clip);
            for lane in request.project.tracks.iter().filter(|lane| {
                lane.automation
                    .as_ref()
                    .is_some_and(|lane| lane.target.track == output.source_track)
            }) {
                if let Some(copied) = clone_mix_automation(lane, track.id) {
                    automation.push(copied);
                }
            }
            new_tracks.push(track);
        }
        let mut input_copies = Vec::new();
        for channel in candidate.channels_mut() {
            copy_sidechain_routes(
                channel.id,
                None,
                &mut channel.instrument_inputs,
                &replacements,
                &mut input_copies,
            );
            for slot in &mut channel.inserts {
                copy_sidechain_routes(
                    channel.id,
                    Some(slot.id),
                    &mut slot.inputs,
                    &replacements,
                    &mut input_copies,
                );
            }
        }
        let link = daw_core::FreezeLink {
            group,
            source_track: request.track,
            source_clips: request.clips.clone(),
            muted_tracks,
            input_copies,
            active: true,
        };
        for track in &mut new_tracks {
            track.clips[0].freeze = Some(link.clone());
        }
        candidate.tracks.splice(insertion..insertion, new_tracks);
        candidate.tracks.extend(automation);
        candidate.validate()?;
        self.begin_edit();
        self.project = candidate;
        rendered.assets.retained = true;
        if let Some((track, clip)) = selected {
            self.arranger_select_clips([(track, clip)], None);
            if self.view == WorkspaceView::Arrangement {
                self.queue_arranger_channel_reveal(track);
            } else if self.view == WorkspaceView::Mixer {
                self.mixer_reveal_channel = Some(track);
            }
        }
        self.selected_notes.clear();
        self.midi_editor.clear_hidden_selection();
        let mut used: usize = self
            .waveforms
            .values()
            .map(|audio| audio.samples.len() * 4)
            .sum();
        for (path, preview) in rendered.previews.drain() {
            let bytes = preview.samples.len() * 4;
            if bytes <= crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(used) {
                self.waveforms
                    .insert(path.to_string_lossy().into_owned(), preview);
                used += bytes;
            }
        }
        self.edit_label = Some(
            if request.whole_track {
                "Freeze track"
            } else {
                "Freeze clip"
            }
            .into(),
        );
        self.changed();
        self.flush_pending_edit();
        self.freeze.completed = serde_json::json!({"source":request.track,"group":group,"clips":request.clips,"outputs":completed});
        if !self.error {
            self.status = format!(
                "Frozen {} channel{} · Original clips muted",
                rendered.outputs.len(),
                if rendered.outputs.len() == 1 { "" } else { "s" }
            );
        }
        Ok(())
    }

    pub(crate) fn can_unfreeze(&self, track: u64) -> bool {
        !self.freeze_busy()
            && !self.recording_busy()
            && !self.audio_import_busy()
            && !self.project_io_pending()
            && !self.state_sync_pending()
            && self.export_job.is_none()
            && self.document_guard.is_none()
            && unfreeze_owner(&self.project, track).is_some()
    }

    pub(crate) fn unfreeze(&mut self, track: u64) {
        if !self.can_unfreeze(track) {
            self.message(
                Err(anyhow::anyhow!(
                    "Finish the current operation, then choose a frozen track"
                )),
                "",
            );
            return;
        }
        if self.freeze.last_error_status.as_deref() == Some(self.status.as_str()) {
            self.error = false;
        }
        self.freeze.last_error_status = None;
        self.flush_pending_edit();
        self.desired_playing = false;
        self.audition_request = None;
        if let Some(engine) = &self.engine {
            engine.set_playing(false);
        }
        self.request_unfreeze_snapshot(track);
    }

    pub(crate) fn begin_unfreeze(&mut self, track: u64) {
        let result = (|| -> Result<Project> {
            ensure!(
                !self.freeze_busy() && !self.recording_busy(),
                "Wait for the current operation"
            );
            let owner =
                unfreeze_owner(&self.project, track).context("This track has no active Freeze")?;
            let mut groups = HashSet::new();
            let links: Vec<_> = self
                .project
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .filter_map(|clip| clip.freeze.as_ref())
                .filter(|link| {
                    link.active && link.source_track == owner && groups.insert(link.group)
                })
                .cloned()
                .collect();
            let mut clips = HashSet::new();
            let mut mutes = HashMap::new();
            for link in &links {
                for id in &link.source_clips {
                    let original = self.project.clip(owner, *id);
                    ensure!(
                        original.is_some(),
                        "An original clip was removed; restore it before returning to originals"
                    );
                    if let daw_core::ClipContent::Audio { path, .. } = &original.unwrap().content {
                        ensure!(
                            Path::new(path).is_file(),
                            "An original audio file is missing; restore it before returning to originals"
                        );
                    }
                    clips.insert(*id);
                }
                for state in &link.muted_tracks {
                    mutes.entry(state.track).or_insert(state.muted);
                }
            }
            for channel in self.project.tracks.iter().filter(|channel| {
                channel.id == owner
                    || channel
                        .instrument_output
                        .is_some_and(|output| output.track == owner)
            }) {
                if let daw_core::Instrument::Vst3 { path, .. } = &channel.instrument {
                    ensure!(
                        Path::new(path).exists(),
                        "The original instrument is unavailable; restore it before returning to originals"
                    );
                }
                for slot in channel.inserts.iter().filter(|slot| slot.enabled) {
                    let daw_core::Effect::Vst3 { path, .. } = &slot.effect;
                    ensure!(
                        Path::new(path).exists(),
                        "An original insert is unavailable; restore it before returning to originals"
                    );
                }
            }
            let mut candidate = self.project.clone();
            for clip in &mut candidate
                .track_mut(owner)
                .context("The original track was removed")?
                .clips
            {
                if clips.contains(&clip.id) {
                    clip.muted = false;
                }
            }
            for (track, muted) in mutes {
                if let Some(track) = candidate.track_mut(track) {
                    track.mute = muted;
                }
            }
            for copied in links.iter().flat_map(|link| &link.input_copies) {
                if let Some(channel) = candidate.track_mut(copied.track) {
                    let routes = if let Some(effect) = copied.effect {
                        channel
                            .inserts
                            .iter_mut()
                            .find(|slot| slot.id == effect)
                            .map(|slot| &mut slot.inputs)
                    } else {
                        Some(&mut channel.instrument_inputs)
                    };
                    if let Some(routes) = routes {
                        // Edited route copies are user work and remain in the song.
                        routes.retain(|route| route != &copied.route);
                    }
                }
            }
            for clip in candidate
                .tracks
                .iter_mut()
                .flat_map(|track| &mut track.clips)
            {
                if let Some(link) = &mut clip.freeze
                    && link.active
                    && link.source_track == owner
                {
                    clip.muted = true;
                    link.active = false;
                }
            }
            for channel in &mut candidate.tracks {
                if channel.clips.iter().any(|clip| {
                    clip.freeze
                        .as_ref()
                        .is_some_and(|link| !link.active && link.source_track == owner)
                }) && channel.clips.iter().all(|clip| clip.muted)
                {
                    channel.solo = false;
                }
            }
            candidate.validate()?;
            Ok(candidate)
        })();
        match result {
            Ok(candidate) => {
                self.flush_pending_edit();
                self.begin_edit();
                self.project = candidate;
                self.edit_label = Some("Return to originals".into());
                self.changed();
                self.flush_pending_edit();
                if !self.error {
                    self.status = "Originals restored · Frozen audio kept and muted".into();
                }
            }
            Err(error) => {
                self.message(Err(error.context("Could not return to originals")), "");
                self.freeze.last_error_status = Some(self.status.clone());
            }
        }
    }

    pub(crate) fn freeze_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"busy":self.freeze_busy(),"preparing":self.freeze.preparing,"cancel_preparing":self.freeze.cancel_preparing,"active":self.freeze.active.as_ref().map(|active| serde_json::json!({
            "track":active.request.track,"clips":active.request.clips,"session":active.request.session,
            "cancel_requested":active.cancel.load(Ordering::Acquire)})),"completed":self.freeze.completed})
    }
}
