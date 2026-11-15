//! Background, isolated decoding and atomic import into the captured document.
use crate::app::DawApp;
use anyhow::{Context, Result, bail, ensure};
use daw_core::{AudioData, AudioFile, Clip, MAX_BEAT, TrackKind};
use eframe::egui;
use serde::Deserialize;
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_FILES: usize = 8;
const MAX_QUEUED: usize = 4;
const DECODE_BUDGET: usize = 8 * 1024 * 1024 * 1024;
const PREVIEW_BUDGET: usize = 128 * 1024 * 1024;
const MAX_LOG: u64 = 8192;
const MAX_IMPORT_OUTCOMES: usize = 64;

/// Stable identity for one admitted, rejected, or canceled import request.
/// A request remains tied to the document in which it was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImportTicket {
    pub id: u64,
    pub session: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct ImportedSourceReceipt {
    pub source: daw_ui::SampleDrop,
    pub path: PathBuf,
    pub name: String,
    pub project_sample_id: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ImportReceipt {
    pub sources: Vec<ImportedSourceReceipt>,
}

#[derive(Clone, Debug)]
pub(crate) enum ImportOutcome {
    Succeeded(ImportReceipt),
    Failed(String),
    Cancelled,
}

struct Request {
    ticket: ImportTicket,
    session: u64,
    sources: Vec<ImportSource>,
    destination: ImportDestination,
    scratch: PathBuf,
}

/// The document already owns admitted recordings; external sources must first
/// become private, verified media. This distinction is shared by all targets.
#[derive(Clone)]
enum ImportSource {
    External(daw_ui::SampleDrop),
    Managed(daw_ui::SampleDrop),
}

impl ImportSource {
    fn source(&self) -> &daw_ui::SampleDrop {
        match self {
            Self::External(source) | Self::Managed(source) => source,
        }
    }
}

enum ImportDestination {
    Audio {
        target: Option<u64>,
        beat: f64,
    },
    Device {
        track: u64,
        slot: u32,
        guard: SampleImportGuard,
        player: Option<PlayerImport>,
    },
    ProjectSamples,
}

impl ImportDestination {
    fn reserves_slot(&self, target_track: u64, target_slot: u32) -> bool {
        matches!(self, Self::Device { track, slot, .. }
            if *track == target_track && *slot == target_slot)
    }
}

struct PlayerImport {
    group: u32,
    expected: daw_core::PlayerNode,
    replacement: daw_core::PlayerNode,
}

#[derive(PartialEq)]
struct SampleImportGuard {
    device: String,
    binding: Option<daw_core::DeviceSample>,
    implicit: bool,
    players: Vec<(u32, daw_core::PlayerNode)>,
}

impl SampleImportGuard {
    fn capture(instrument: &daw_core::Instrument, slot: u32) -> Result<Self> {
        let daw_core::Instrument::Builtin {
            device,
            sample_players,
            ..
        } = instrument
        else {
            bail!("Sample target is no longer a built-in instrument");
        };
        let descriptor =
            daw_devices::descriptor(device).context("Sampling instrument is unavailable")?;
        ensure!(
            slot < descriptor.sample_capacity,
            "Sample destination is out of range"
        );
        Ok(Self {
            device: device.clone(),
            binding: instrument.sample(slot).cloned(),
            implicit: slot < descriptor.sample_slots && instrument.sample_player(slot).is_none(),
            players: sample_players
                .iter()
                .filter(|(_, root)| root.uses_sample(slot))
                .map(|(group, root)| (*group, root.clone()))
                .collect(),
        })
    }
}

struct Active {
    request: Request,
    receiver: mpsc::Receiver<Result<Batch>>,
    cancel: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
}

#[derive(Default)]
pub(crate) struct AudioImportState {
    active: Option<Active>,
    queued: VecDeque<Request>,
    last_error_status: Option<String>,
    pointer_was_down: bool,
    outcomes: VecDeque<(ImportTicket, ImportOutcome)>,
}

impl Drop for AudioImportState {
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
        let root = scratch.join("imports");
        fs::create_dir_all(&root).context("Creating managed audio folder")?;
        for _ in 0..16 {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let directory = root.join(format!(
                "import-{}-{nonce}-{}",
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
                Err(error) => return Err(error).context("Creating private import staging folder"),
            }
        }
        bail!("Could not create a unique managed audio folder")
    }
}

impl Drop for Assets {
    fn drop(&mut self) {
        if !self.retained {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

struct Imported {
    path: PathBuf,
    name: String,
    frames: u64,
    sample_rate: u32,
    preview: Option<AudioData>,
    source: AudioFile,
}

struct Batch {
    assets: Option<Assets>,
    files: Vec<Imported>,
}

#[derive(Deserialize)]
struct DecodeInfo {
    sample_rate: u32,
    channels: usize,
    frames: u64,
    decoded_bytes: u64,
}

struct Decoder(Child);
impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn test_environment() -> bool {
    std::env::var_os("RILL_AUTOMATION_DIR").is_some()
}

fn decoder_timeout() -> Duration {
    let millis = test_environment()
        .then(|| std::env::var("RILL_E2E_AUDIO_IMPORT_TIMEOUT_MS").ok())
        .flatten()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120_000);
    Duration::from_millis(millis.clamp(10, 600_000))
}

fn read_log(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_LOG + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_LOG,
        "Audio decoder response exceeded its limit"
    );
    Ok(bytes)
}

fn decode_one(
    input: &Path,
    output: &Path,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<DecodeInfo> {
    ensure!(!cancel.load(Ordering::Acquire), "Audio import cancelled");
    let stdout = output.with_extension("stdout");
    let stderr = output.with_extension("stderr");
    let executable = if test_environment() {
        std::env::var_os("RILL_E2E_AUDIO_IMPORT_HELPER").map(PathBuf::from)
    } else {
        None
    }
    .map_or_else(std::env::current_exe, Ok)?;
    let mut child = Decoder(
        Command::new(executable)
            .arg("--decode-audio-worker")
            .arg(input)
            .arg(output)
            .arg(budget.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::from(File::create(&stdout)?))
            .stderr(Stdio::from(File::create(&stderr)?))
            .spawn()
            .context("Starting isolated audio decoder")?,
    );
    let deadline = Instant::now() + decoder_timeout();
    let status = loop {
        ensure!(!cancel.load(Ordering::Acquire), "Audio import cancelled");
        ensure!(
            Instant::now() < deadline,
            "Audio decoder timed out; no clips were imported"
        );
        ensure!(
            fs::metadata(&stdout)?.len() <= MAX_LOG && fs::metadata(&stderr)?.len() <= MAX_LOG,
            "Audio decoder produced too much diagnostic output"
        );
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let diagnostic = read_log(&stderr)?;
    ensure!(
        status.success(),
        "Audio decoder failed ({status}): {}",
        String::from_utf8_lossy(&diagnostic).trim()
    );
    let info: DecodeInfo = serde_json::from_slice(&read_log(&stdout)?)
        .context("Audio decoder returned an invalid response")?;
    let _ = fs::remove_file(stdout);
    let _ = fs::remove_file(stderr);
    ensure!(
        (8_000..=384_000).contains(&info.sample_rate) && (1..=2).contains(&info.channels),
        "Audio must have one or two channels at 8–384 kHz"
    );
    ensure!(
        info.frames > 0
            && info
                .frames
                .checked_mul(info.channels as u64)
                .and_then(|samples| samples.checked_mul(4))
                == Some(info.decoded_bytes)
            && info.decoded_bytes <= budget as u64,
        "Audio decoder exceeded its decoded sample budget"
    );
    Ok(info)
}

fn decode_batch(
    sources: &[ImportSource],
    scratch: &Path,
    cancel: &AtomicBool,
    completed: &AtomicUsize,
) -> Result<Batch> {
    let mut assets = None;
    let mut remaining = DECODE_BUDGET;
    let mut preview_remaining = PREVIEW_BUDGET;
    let mut files = Vec::with_capacity(sources.len());
    for (index, imported) in sources.iter().enumerate() {
        ensure!(!cancel.load(Ordering::Acquire), "Audio import cancelled");
        let input = &imported.source().path;
        let name = imported.source().name.clone().unwrap_or_else(|| {
            input
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Audio".into())
        });
        if matches!(imported, ImportSource::Managed(_)) {
            let source = AudioFile::open(input).context("Opening project recording")?;
            let info = source.info();
            let bytes = usize::try_from(info.decoded_bytes).context("Audio size overflow")?;
            remaining = remaining
                .checked_sub(bytes)
                .context("Audio import batch exceeds the 8 GiB media budget")?;
            files.push(Imported {
                path: source.path().to_path_buf(),
                name,
                frames: info.frames as u64,
                sample_rate: info.sample_rate,
                preview: None,
                source,
            });
            completed.store(index + 1, Ordering::Release);
            continue;
        }
        if assets.is_none() {
            assets = Some(Assets::create(scratch)?);
        }
        let output = assets
            .as_ref()
            .expect("created above")
            .directory
            .join(format!("{}.wav", index + 1));
        let info = decode_one(
            input,
            &output,
            remaining.min(daw_core::MAX_IMPORTED_AUDIO_BYTES),
            cancel,
        )
        .with_context(|| format!("Importing {}", input.display()))?;
        // Verify the managed WAV, its full finite payload and metadata off the
        // UI thread before any document mutation. Never trust a partial output.
        let mut reader = hound::WavReader::open(&output).context("Verifying managed audio")?;
        let spec = reader.spec();
        let samples = info
            .frames
            .checked_mul(info.channels as u64)
            .context("Decoded sample count overflow")?;
        let bytes = usize::try_from(info.decoded_bytes).context("Decoded audio size overflow")?;
        ensure!(
            spec.sample_rate == info.sample_rate
                && usize::from(spec.channels) == info.channels
                && spec.bits_per_sample == 32
                && spec.sample_format == hound::SampleFormat::Float
                && u64::from(reader.len()) == samples
                && bytes <= remaining,
            "Decoded WAV does not match its completed response"
        );
        // The main process streams verification. Optional visualization memory
        // is much smaller than the decoder budget and allocation is fallible.
        let mut preview = None;
        if (bytes as u64) < daw_core::AUDIO_STREAM_THRESHOLD_BYTES && bytes <= preview_remaining {
            let mut data = Vec::new();
            if usize::try_from(samples).is_ok_and(|samples| data.try_reserve_exact(samples).is_ok())
            {
                preview = Some(data);
            }
        }
        let mut count = 0_u64;
        for sample in reader.samples::<f32>() {
            if count.is_multiple_of(4096) {
                ensure!(!cancel.load(Ordering::Acquire), "Audio import cancelled");
            }
            let sample = sample.context("Incomplete managed audio payload")?;
            ensure!(
                sample.is_finite() && count < samples,
                "Invalid managed audio payload"
            );
            if let Some(preview) = &mut preview {
                preview.push(sample);
            }
            count += 1;
        }
        ensure!(count == samples, "Incomplete managed audio payload");
        remaining = remaining
            .checked_sub(bytes)
            .context("Audio import batch exceeds the 8 GiB media budget")?;
        let preview = preview.map(|samples| {
            preview_remaining -= bytes;
            AudioData {
                sample_rate: info.sample_rate,
                channels: info.channels,
                samples,
            }
        });
        let source = AudioFile::open(&output).context("Admit verified managed audio")?;
        ensure!(
            source.info().decoded_bytes == info.decoded_bytes
                && source.info().frames as u64 == info.frames
                && source.info().sample_rate == info.sample_rate,
            "Managed audio changed after verification"
        );
        files.push(Imported {
            path: source.path().to_path_buf(),
            name,
            frames: info.frames,
            sample_rate: info.sample_rate,
            preview,
            source,
        });
        completed.store(index + 1, Ordering::Release);
    }
    ensure!(!cancel.load(Ordering::Acquire), "Audio import cancelled");
    Ok(Batch { assets, files })
}

/// Decode a single source for an ephemeral consumer using the same isolated,
/// bounded decoder as imports. Staging media is removed before returning.
pub(crate) fn decode_audio_preview(
    input: &Path,
    scratch: &Path,
    budget: usize,
    cancel: &AtomicBool,
) -> Result<AudioData> {
    let assets = Assets::create(scratch)?;
    let output = assets.directory.join("preview.wav");
    let info = decode_one(input, &output, budget, cancel)?;
    ensure!(!cancel.load(Ordering::Acquire), "Sample preview cancelled");
    let audio = AudioData::load_wav_with_limit(&output, budget)?;
    ensure!(
        audio.sample_rate == info.sample_rate
            && audio.channels == info.channels
            && audio.samples.len() as u64 == info.frames * info.channels as u64,
        "Preview audio does not match the completed decoder response"
    );
    ensure!(!cancel.load(Ordering::Acquire), "Sample preview cancelled");
    Ok(audio)
}

impl DawApp {
    fn new_import_ticket(&self) -> ImportTicket {
        ImportTicket {
            id: daw_core::new_id(),
            session: self.song_session.id,
        }
    }

    /// Completed outcomes are bounded independently of queued/active imports.
    /// Consumers may inspect synchronous completions immediately after queuing.
    pub(crate) fn audio_import_outcome(&self, ticket: ImportTicket) -> Option<&ImportOutcome> {
        if ticket.session != self.song_session.id {
            return Some(&ImportOutcome::Cancelled);
        }
        if let Some((_, outcome)) = self
            .audio_import
            .outcomes
            .iter()
            .find(|(candidate, _)| *candidate == ticket)
        {
            return Some(outcome);
        }
        let pending = self
            .audio_import
            .active
            .as_ref()
            .is_some_and(|active| active.request.ticket == ticket)
            || self
                .audio_import
                .queued
                .iter()
                .any(|request| request.ticket == ticket);
        if pending {
            None
        } else {
            Some(&ImportOutcome::Cancelled)
        }
    }

    fn finish_audio_import(&mut self, ticket: ImportTicket, outcome: ImportOutcome) {
        if self
            .audio_import
            .outcomes
            .iter()
            .any(|(candidate, _)| *candidate == ticket)
        {
            return;
        }
        self.audio_import.outcomes.push_back((ticket, outcome));
        while self.audio_import.outcomes.len() > MAX_IMPORT_OUTCOMES {
            self.audio_import.outcomes.pop_front();
        }
    }

    fn cancel_queued_audio_imports(&mut self) {
        let queued: Vec<_> = self
            .audio_import
            .queued
            .drain(..)
            .map(|request| request.ticket)
            .collect();
        for ticket in queued {
            self.finish_audio_import(ticket, ImportOutcome::Cancelled);
        }
    }

    fn audio_import_error(&mut self, error: anyhow::Error) {
        self.message(Err(error), "");
        self.audio_import.last_error_status = Some(self.status.clone());
    }

    pub(crate) fn import_audio_dialog(&mut self) {
        if !self.import_picker_ready() {
            return;
        }
        let beat = self.cursor;
        if let Some(path) = self.pick_document_path(crate::document_ui::DocumentDialog::ImportAudio)
        {
            self.queue_audio_import(vec![path], None, beat);
        }
    }

    pub(crate) fn import_picker_ready(&mut self) -> bool {
        let error = if self.recording_busy() {
            Some("Stop recording before importing audio or MIDI")
        } else if self.document_guard.is_some()
            || self.project_io_pending()
            || self.state_sync_pending()
            || self.freeze_busy()
        {
            Some("Finish the current document operation before importing audio or MIDI")
        } else {
            None
        };
        if let Some(error) = error {
            self.audio_import_error(anyhow::anyhow!(error));
            false
        } else {
            true
        }
    }

    pub(crate) fn audio_import_busy(&self) -> bool {
        self.audio_import.active.is_some() || !self.audio_import.queued.is_empty()
    }

    pub(crate) fn audio_import_message(&self) -> Option<String> {
        self.audio_import.active.as_ref().map(|active| {
            format!(
                "Importing audio · {} of {} decoded{}",
                active.completed.load(Ordering::Acquire),
                active.request.sources.len(),
                if active.cancel.load(Ordering::Acquire) {
                    " · Cancelling…"
                } else {
                    ""
                }
            )
        })
    }

    pub(crate) fn cancel_audio_import(&mut self) {
        let pending = self.audio_import_busy();
        self.cancel_queued_audio_imports();
        if let Some(active) = &self.audio_import.active {
            active.cancel.store(true, Ordering::Release);
        }
        if pending && !self.error {
            self.status = if self.audio_import.active.is_some() {
                "Cancelling audio import…"
            } else {
                "Audio import canceled"
            }
            .into();
        }
    }

    pub(crate) fn queue_audio_import(
        &mut self,
        paths: Vec<PathBuf>,
        target: Option<u64>,
        beat: f64,
    ) {
        self.queue_audio_source_import(paths.into_iter().map(Into::into).collect(), target, beat);
    }

    pub(crate) fn queue_audio_source_import(
        &mut self,
        sources: Vec<daw_ui::SampleDrop>,
        target: Option<u64>,
        beat: f64,
    ) {
        self.queue_audio_source_import_tracked(sources, target, beat);
    }

    pub(crate) fn queue_audio_source_import_tracked(
        &mut self,
        sources: Vec<daw_ui::SampleDrop>,
        target: Option<u64>,
        beat: f64,
    ) -> ImportTicket {
        self.queue_import(sources, ImportDestination::Audio { target, beat })
    }

    /// Decodes `path` like any import, then loads it into sample slot `slot`
    /// of the built-in instrument on `track`.
    pub(crate) fn queue_sample_import(&mut self, path: PathBuf, track: u64, slot: u32) {
        self.queue_sample_source_import(path.into(), track, slot);
    }

    pub(crate) fn queue_sample_source_import(
        &mut self,
        source: daw_ui::SampleDrop,
        track: u64,
        slot: u32,
    ) {
        self.queue_sample_source_import_tracked(source, track, slot);
    }

    pub(crate) fn queue_sample_source_import_tracked(
        &mut self,
        source: daw_ui::SampleDrop,
        track: u64,
        slot: u32,
    ) -> ImportTicket {
        self.queue_device_source_import(source, track, slot, None)
    }

    pub(crate) fn queue_project_sample_import_tracked(
        &mut self,
        sources: Vec<daw_ui::SampleDrop>,
    ) -> ImportTicket {
        self.queue_import(sources, ImportDestination::ProjectSamples)
    }

    fn queue_device_source_import(
        &mut self,
        source: daw_ui::SampleDrop,
        track: u64,
        slot: u32,
        player: Option<PlayerImport>,
    ) -> ImportTicket {
        let guard = self
            .project
            .track(track)
            .context("Sample target no longer exists")
            .and_then(|track| SampleImportGuard::capture(&track.instrument, slot));
        match guard {
            Ok(guard) => self.queue_import(
                vec![source],
                ImportDestination::Device {
                    track,
                    slot,
                    guard,
                    player,
                },
            ),
            Err(error) => {
                let ticket = self.new_import_ticket();
                self.finish_audio_import(ticket, ImportOutcome::Failed(format!("{error:#}")));
                self.audio_import_error(error);
                ticket
            }
        }
    }

    pub(crate) fn sample_import_reserves_slot(&self, track: u64, slot: u32) -> bool {
        self.audio_import
            .active
            .as_ref()
            .is_some_and(|active| active.request.destination.reserves_slot(track, slot))
            || self
                .audio_import
                .queued
                .iter()
                .any(|request| request.destination.reserves_slot(track, slot))
    }

    /// Dropping onto a player replaces a lone Sample or appends a recording
    /// to a composite. Its new leaf appears only after a successful import.
    pub(crate) fn queue_player_sample_import(&mut self, path: PathBuf, track: u64, group: u32) {
        self.queue_player_sample_source_import(path.into(), track, group);
    }

    pub(crate) fn queue_player_sample_source_import(
        &mut self,
        source: daw_ui::SampleDrop,
        track: u64,
        group: u32,
    ) {
        let prepared = (|| -> Result<(u32, Option<PlayerImport>)> {
            let instrument = &self
                .project
                .track(track)
                .context("Sample target no longer exists")?
                .instrument;
            let descriptor = instrument
                .builtin_device()
                .and_then(daw_devices::descriptor)
                .context("Choose a sampling instrument")?;
            ensure!(
                group < descriptor.sample_slots,
                "Sample player is out of range"
            );
            let Some(root) = instrument.sample_player(group) else {
                return Ok((group, None));
            };
            if let daw_core::PlayerKind::Sample { slot, .. } = root.kind {
                return Ok((slot, None));
            }
            let slot = (descriptor.sample_slots..descriptor.sample_capacity)
                .find(|slot| {
                    !self.sample_import_reserves_slot(track, *slot)
                        && instrument.sample_slot_available(
                            *slot,
                            descriptor.sample_slots,
                            descriptor.sample_capacity,
                        )
                })
                .context("The instrument's recording pool is full")?;
            let mut replacement = root.clone();
            let child = daw_core::PlayerNode::sample(replacement.next_id(), slot);
            match &mut replacement.kind {
                daw_core::PlayerKind::All { children }
                | daw_core::PlayerKind::RoundRobin { children }
                | daw_core::PlayerKind::Random { children } => children.push(child),
                daw_core::PlayerKind::Velocity { children } => {
                    children.push(daw_core::VelocityChild {
                        min: 1,
                        max: 127,
                        child,
                    })
                }
                daw_core::PlayerKind::Sample { .. } => unreachable!("handled above"),
            }
            replacement
                .validate(descriptor.sample_capacity)
                .map_err(anyhow::Error::msg)?;
            Ok((
                slot,
                Some(PlayerImport {
                    group,
                    expected: root.clone(),
                    replacement,
                }),
            ))
        })();
        match prepared {
            Ok((slot, player)) => {
                self.queue_device_source_import(source, track, slot, player);
            }
            Err(error) => self.audio_import_error(error),
        }
    }

    fn queue_import(
        &mut self,
        sources: Vec<daw_ui::SampleDrop>,
        mut destination: ImportDestination,
    ) -> ImportTicket {
        let ticket = self.new_import_ticket();
        let mut immediate_status = None;
        let mut immediate_receipt = None;
        let result = (|| -> Result<()> {
            ensure!(
                !sources.is_empty() && sources.len() <= MAX_FILES,
                "Import between one and eight audio files at a time"
            );
            ensure!(
                !self.recording_busy(),
                "Stop recording before importing audio"
            );
            ensure!(
                self.audio_import.queued.len() < MAX_QUEUED,
                "Audio import queue is full; wait or cancel the pending import"
            );
            ensure!(
                self.document_guard.is_none()
                    && !self.project_io_pending()
                    && !self.state_sync_pending()
                    && !self.freeze_busy(),
                "Finish the current document operation before importing audio"
            );
            if let ImportDestination::Audio { target, beat } = &mut destination {
                ensure!(
                    beat.is_finite() && (0.0..=MAX_BEAT).contains(beat),
                    "Invalid audio import position"
                );
                if let Some(id) = *target {
                    let track = self
                        .project
                        .track(id)
                        .context("Import target no longer exists")?;
                    *target = (track.kind == TrackKind::Audio).then_some(id);
                }
            }
            let sources: Vec<_> = sources
                .into_iter()
                .map(|mut source| {
                    if let Some(name) = &source.name {
                        daw_core::validate_project_sample_name(name)?;
                    }
                    if source.name.is_none() {
                        source.name = self
                            .project
                            .samples
                            .iter()
                            .find(|sample| Path::new(&sample.path) == source.path)
                            .map(|sample| sample.name.clone());
                    }
                    Ok(
                        if self
                            .project
                            .audio_paths()
                            .any(|path| Path::new(path) == source.path)
                        {
                            ImportSource::Managed(source)
                        } else {
                            ImportSource::External(source)
                        },
                    )
                })
                .collect::<Result<_>>()?;
            if let ImportDestination::Device {
                track,
                slot,
                player,
                ..
            } = &destination
                && let [ImportSource::Managed(source)] = sources.as_slice()
            {
                self.flush_pending_edit();
                let name = source.name.clone().unwrap_or_else(|| {
                    source
                        .path
                        .file_stem()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Audio".into())
                });
                self.set_device_sample_with_player(
                    *track,
                    *slot,
                    Some(daw_core::DeviceSample {
                        slot: *slot,
                        path: source
                            .path
                            .to_str()
                            .context("Audio path is not valid Unicode")?
                            .into(),
                        name: name.clone(),
                    }),
                    player
                        .as_ref()
                        .map(|player| (player.group, player.replacement.clone())),
                )
                .map_err(|error| anyhow::anyhow!(error.message))?;
                immediate_status = Some(format!("Loaded sample {name}"));
                immediate_receipt = Some(ImportReceipt {
                    sources: vec![ImportedSourceReceipt {
                        source: source.clone(),
                        path: source.path.clone(),
                        name,
                        project_sample_id: None,
                    }],
                });
                return Ok(());
            }
            if matches!(destination, ImportDestination::ProjectSamples)
                && sources.iter().all(|source| {
                    self.project.samples.iter().any(|sample| {
                        Path::new(&sample.path) == source.source().path
                            && source
                                .source()
                                .name
                                .as_ref()
                                .is_none_or(|name| *name == sample.name)
                    })
                })
            {
                immediate_status = Some("Already in Project Samples".into());
                immediate_receipt = Some(ImportReceipt {
                    sources: sources
                        .iter()
                        .map(|source| {
                            let original = source.source();
                            let sample = self
                                .project
                                .samples
                                .iter()
                                .find(|sample| {
                                    Path::new(&sample.path) == original.path
                                        && original
                                            .name
                                            .as_ref()
                                            .is_none_or(|name| *name == sample.name)
                                })
                                .expect("all sources matched above");
                            ImportedSourceReceipt {
                                source: original.clone(),
                                path: sample.path.clone().into(),
                                name: sample.name.clone(),
                                project_sample_id: Some(sample.id),
                            }
                        })
                        .collect(),
                });
                return Ok(());
            }
            self.audio_import.queued.push_back(Request {
                ticket,
                session: self.song_session.id,
                sources,
                destination,
                scratch: self.settings.scratch_dir.clone(),
            });
            Ok(())
        })();
        match result {
            Ok(()) => {
                if let Some(receipt) = immediate_receipt {
                    self.finish_audio_import(ticket, ImportOutcome::Succeeded(receipt));
                }
                if self.audio_import.last_error_status.as_deref() == Some(self.status.as_str()) {
                    self.error = false;
                }
                self.audio_import.last_error_status = None;
                if !self.error {
                    self.status =
                        immediate_status.unwrap_or_else(|| "Preparing audio import…".into());
                }
            }
            Err(error) => {
                self.finish_audio_import(ticket, ImportOutcome::Failed(format!("{error:#}")));
                self.audio_import_error(error);
            }
        }
        ticket
    }

    pub(crate) fn poll_audio_import(&mut self, ctx: &egui::Context) {
        let pointer_down = ctx.input(|input| input.pointer.any_down());
        let defer_commit =
            pointer_down || self.audio_import.pointer_was_down || self.recording_busy();
        self.audio_import.pointer_was_down = pointer_down;
        if let Some(active) = self.audio_import.active.take() {
            if defer_commit && !active.cancel.load(Ordering::Acquire) {
                self.audio_import.active = Some(active);
            } else {
                match active.receiver.try_recv() {
                    Ok(result) => {
                        if active.cancel.load(Ordering::Acquire)
                            || active.request.session != self.song_session.id
                        {
                            drop(result);
                            self.finish_audio_import(
                                active.request.ticket,
                                ImportOutcome::Cancelled,
                            );
                            if active.request.session == self.song_session.id
                                && self.document_guard.is_none()
                                && !self.project_io_pending()
                                && !self.error
                            {
                                self.status = "Audio import canceled".into();
                            }
                        } else {
                            match result
                                .and_then(|batch| self.commit_audio_import(&active.request, batch))
                            {
                                Ok(receipt) => self.finish_audio_import(
                                    active.request.ticket,
                                    ImportOutcome::Succeeded(receipt),
                                ),
                                Err(error) => self.fail_audio_import(active.request.ticket, error),
                            }
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => self.audio_import.active = Some(active),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if active.cancel.load(Ordering::Acquire)
                            || active.request.session != self.song_session.id
                        {
                            self.finish_audio_import(
                                active.request.ticket,
                                ImportOutcome::Cancelled,
                            );
                        } else {
                            self.fail_audio_import(active.request.ticket, anyhow::anyhow!(
                                "Audio import worker stopped unexpectedly; no clips were imported"
                            ));
                        }
                    }
                }
            }
        }
        while self.audio_import.active.is_none() {
            let Some(request) = self.audio_import.queued.pop_front() else {
                break;
            };
            if request.session != self.song_session.id {
                self.finish_audio_import(request.ticket, ImportOutcome::Cancelled);
                continue;
            }
            let (sender, receiver) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let completed = Arc::new(AtomicUsize::new(0));
            let worker_cancel = Arc::clone(&cancel);
            let worker_completed = Arc::clone(&completed);
            let sources = request.sources.clone();
            let scratch = request.scratch.clone();
            match std::thread::Builder::new()
                .name("rill-audio-import".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        decode_batch(&sources, &scratch, &worker_cancel, &worker_completed)
                    }))
                    .unwrap_or_else(|_| {
                        Err(anyhow::anyhow!(
                            "Audio import worker panicked; no clips were imported"
                        ))
                    });
                    let _ = sender.send(result);
                }) {
                Ok(_) => {
                    self.audio_import.active = Some(Active {
                        request,
                        receiver,
                        cancel,
                        completed,
                    })
                }
                Err(error) => self.fail_audio_import(request.ticket, error.into()),
            }
        }
        if self.audio_import_busy() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn fail_audio_import(&mut self, ticket: ImportTicket, error: anyhow::Error) {
        self.finish_audio_import(ticket, ImportOutcome::Failed(format!("{error:#}")));
        self.cancel_queued_audio_imports();
        self.audio_import_error(error);
    }

    fn commit_audio_import(
        &mut self,
        request: &Request,
        mut batch: Batch,
    ) -> Result<ImportReceipt> {
        ensure!(
            request.session == self.song_session.id,
            "Import belongs to a replaced document"
        );
        let mut receipt = ImportReceipt {
            sources: request
                .sources
                .iter()
                .zip(&batch.files)
                .map(|(source, file)| ImportedSourceReceipt {
                    source: source.source().clone(),
                    path: file.path.clone(),
                    name: file.name.clone(),
                    project_sample_id: None,
                })
                .collect(),
        };
        let (target, beat) = match &request.destination {
            ImportDestination::Device {
                track,
                slot,
                guard,
                player,
            } => {
                let instrument = &self
                    .project
                    .track(*track)
                    .context("Sample target no longer exists")?
                    .instrument;
                ensure!(
                    *guard == SampleImportGuard::capture(instrument, *slot)?,
                    "Sample destination changed while importing; the recording was not loaded"
                );
                self.commit_sample_import(*track, *slot, player.as_ref(), batch)?;
                return Ok(receipt);
            }
            ImportDestination::ProjectSamples => {
                let ids = self.commit_project_sample_import(batch)?;
                for (source, id) in receipt.sources.iter_mut().zip(ids) {
                    source.project_sample_id = Some(id);
                }
                return Ok(receipt);
            }
            ImportDestination::Audio { target, beat } => (*target, *beat),
        };
        if let Some(target) = target {
            ensure!(
                self.project
                    .track(target)
                    .is_some_and(|track| track.kind == TrackKind::Audio),
                "Audio import target was deleted or changed; no clips were imported"
            );
        }
        // Commit the preceding gesture (including MIDI overlap consolidation)
        // before cloning, so the import cannot restore its pre-merge state.
        self.flush_pending_edit();
        let mut candidate = self.project.clone();
        let mut selection = None;
        for file in &batch.files {
            let length = file.frames as f64 / f64::from(file.sample_rate) * candidate.tempo / 60.0;
            ensure!(
                length.is_finite() && length > 0.0 && beat + length <= MAX_BEAT,
                "Imported audio extends beyond the supported timeline"
            );
            let target = target.unwrap_or_else(|| candidate.add_audio_track(&file.name));
            let path = file.path.to_str().context("Managed audio folder is not valid Unicode; choose another Storage folder before importing")?;
            let clip = Clip::audio(&file.name, beat, length, path);
            selection = Some((target, clip.id));
            candidate
                .track_mut(target)
                .context("Import target disappeared")?
                .clips
                .push(clip);
        }
        candidate.validate()?;
        self.begin_edit();
        self.project = candidate;
        if let Some(assets) = &mut batch.assets {
            assets.retained = true;
        }
        if let Some((track, clip)) = selection {
            self.arranger_select_clips([(track, clip)], None);
            if self.view == crate::app::WorkspaceView::Arrangement {
                self.queue_arranger_channel_reveal(track);
            } else if self.view == crate::app::WorkspaceView::Mixer {
                self.mixer_reveal_channel = Some(track);
            }
        }
        self.selected_notes.clear();
        self.midi_editor.clear_hidden_selection();
        self.cache_imported_audio(&mut batch.files);
        self.edit_label = Some(
            if batch.files.len() == 1 {
                "Import audio"
            } else {
                "Import audio files"
            }
            .into(),
        );
        self.changed();
        self.flush_pending_edit();
        if !self.error {
            self.status = format!(
                "Imported {} audio file{}",
                batch.files.len(),
                if batch.files.len() == 1 { "" } else { "s" }
            );
        }
        Ok(receipt)
    }

    fn commit_sample_import(
        &mut self,
        track: u64,
        slot: u32,
        player: Option<&PlayerImport>,
        mut batch: Batch,
    ) -> Result<()> {
        if let Some(player) = player {
            let instrument = &self
                .project
                .track(track)
                .context("Sample target no longer exists")?
                .instrument;
            let descriptor = instrument
                .builtin_device()
                .and_then(daw_devices::descriptor)
                .context("Sample target changed while importing")?;
            ensure!(
                instrument.sample_player(player.group) == Some(&player.expected),
                "Sample player changed while importing; the recording was not loaded"
            );
            ensure!(
                instrument.sample_slot_available(
                    slot,
                    descriptor.sample_slots,
                    descriptor.sample_capacity
                ),
                "Sample destination changed while importing; the recording was not loaded"
            );
        }
        let file = batch.files.first_mut().context("Nothing was decoded")?;
        let path = file
            .path
            .to_str()
            .context("Managed audio folder is not valid Unicode; choose another Storage folder before importing")?
            .to_owned();
        self.flush_pending_edit();
        let name = file.name.clone();
        let sample = daw_core::DeviceSample {
            slot,
            path: path.clone(),
            name: name.clone(),
        };
        self.set_device_sample_with_player(
            track,
            slot,
            Some(sample),
            player.map(|player| (player.group, player.replacement.clone())),
        )
        .map_err(|error| anyhow::anyhow!(error.message))?;
        if let Some(assets) = &mut batch.assets {
            assets.retained = true;
        }
        self.cache_imported_audio(&mut batch.files);
        if !self.error {
            self.status = format!("Loaded sample {name}");
        }
        Ok(())
    }

    fn commit_project_sample_import(&mut self, mut batch: Batch) -> Result<Vec<u64>> {
        self.flush_pending_edit();
        let mut candidate = self.project.clone();
        let before = candidate.samples.len();
        let mut ids = Vec::with_capacity(batch.files.len());
        for file in &batch.files {
            let path = file
                .path
                .to_str()
                .context("Managed audio folder is not valid Unicode")?;
            let id = if let Some(sample) = candidate
                .samples
                .iter()
                .find(|sample| sample.path == path && sample.name == file.name)
            {
                sample.id
            } else {
                candidate.add_project_sample(&file.name, path)?
            };
            ids.push(id);
        }
        let added = candidate.samples.len() - before;
        self.commit_song_candidate(candidate, "Add to Project Samples")
            .map_err(|error| anyhow::anyhow!(error.message))?;
        if let Some(assets) = &mut batch.assets {
            assets.retained = true;
        }
        self.cache_imported_audio(&mut batch.files);
        if !self.error {
            self.status = if added == 0 {
                "Already in Project Samples".into()
            } else {
                format!(
                    "Added {added} recording{} to Project Samples",
                    if added == 1 { "" } else { "s" }
                )
            };
        }
        Ok(ids)
    }

    fn cache_imported_audio(&mut self, files: &mut [Imported]) {
        let mut used: usize = self
            .waveforms
            .values()
            .map(|audio| audio.samples.len() * 4)
            .sum();
        for file in files {
            self.audio_files.insert(
                file.path.to_string_lossy().into_owned(),
                file.source.clone(),
            );
            if let Some(preview) = file.preview.take() {
                let path = file.path.to_string_lossy().into_owned();
                let previous = self
                    .waveforms
                    .get(&path)
                    .map_or(0, |audio| audio.samples.len() * 4);
                let bytes = preview.samples.len() * 4;
                let replacement_used = used.saturating_sub(previous);
                if bytes <= crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(replacement_used)
                {
                    self.waveforms
                        .insert(file.path.to_string_lossy().into_owned(), preview);
                    used = replacement_used + bytes;
                }
            }
        }
    }

    pub(crate) fn audio_import_diagnostics(&self) -> serde_json::Value {
        let next = self
            .audio_import
            .active
            .as_ref()
            .map(|active| &active.request)
            .or_else(|| self.audio_import.queued.front());
        serde_json::json!({"busy":self.audio_import_busy(),"queued":self.audio_import.queued.len(),
        "files":self.audio_import.active.as_ref().map_or(0, |active| active.request.sources.len()),
        "completed":self.audio_import.active.as_ref().map_or(0, |active| active.completed.load(Ordering::Acquire)),
        "cancel_requested":self.audio_import.active.as_ref().is_some_and(|active| active.cancel.load(Ordering::Acquire)),
        "latest_outcome":self.audio_import.outcomes.back().map(|(ticket,outcome)| {
            let (state,error,ids) = match outcome {
                ImportOutcome::Succeeded(receipt) => ("succeeded", None, receipt.sources.iter().filter_map(|source| source.project_sample_id).collect::<Vec<_>>()),
                ImportOutcome::Failed(error) => ("failed", Some(error.as_str()), Vec::new()),
                ImportOutcome::Cancelled => ("cancelled", None, Vec::new()),
            };
            let sources = if let ImportOutcome::Succeeded(receipt) = outcome {
                receipt.sources.iter().map(|source| serde_json::json!({
                    "source_path":source.source.path,"source_name":source.source.name,
                    "path":source.path,"name":source.name,"project_sample_id":source.project_sample_id
                })).collect::<Vec<_>>()
            } else { Vec::new() };
            serde_json::json!({"ticket":ticket.id,"session":ticket.session,"state":state,"error":error,"project_sample_ids":ids,"sources":sources})
        }),
        "next":next.map(|request| {
            let (destination, target, beat) = match request.destination {
                ImportDestination::Audio { target, beat } => ("audio", target, Some(beat)),
                ImportDestination::Device { track, .. } => ("device", Some(track), None),
                ImportDestination::ProjectSamples => ("project_samples", None, None),
            };
            serde_json::json!({"ticket":request.ticket.id,"session":request.session,"destination":destination,
                "target":target,"beat":beat,"files":request.sources.len()})
        })})
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    #[test]
    fn cancellation_rejection_and_evicted_outcomes_never_look_pending() {
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, daw_core::Project::new(), None);
        let before = app.project.clone();
        let first = app
            .queue_project_sample_import_tracked(vec![PathBuf::from("/source/first.wav").into()]);
        let second = app
            .queue_project_sample_import_tracked(vec![PathBuf::from("/source/second.wav").into()]);
        assert!(app.audio_import_outcome(first).is_none());
        app.cancel_audio_import();
        assert!(matches!(
            app.audio_import_outcome(first),
            Some(ImportOutcome::Cancelled)
        ));
        assert!(matches!(
            app.audio_import_outcome(second),
            Some(ImportOutcome::Cancelled)
        ));
        let rejected = app.queue_project_sample_import_tracked(Vec::new());
        assert!(matches!(
            app.audio_import_outcome(rejected),
            Some(ImportOutcome::Failed(_))
        ));
        for _ in 0..MAX_IMPORT_OUTCOMES + 1 {
            app.queue_project_sample_import_tracked(Vec::new());
        }
        assert_eq!(app.audio_import.outcomes.len(), MAX_IMPORT_OUTCOMES);
        assert!(matches!(
            app.audio_import_outcome(rejected),
            Some(ImportOutcome::Cancelled)
        ));
        assert_eq!(app.project, before);
    }

    #[test]
    fn managed_no_op_receipt_identifies_the_exact_project_alias() {
        let ctx = egui::Context::default();
        let mut project = daw_core::Project::new();
        let id = project
            .add_project_sample("Soft kick", "/source/kick.wav")
            .unwrap();
        project
            .add_project_sample("Hard kick", "/source/kick.wav")
            .unwrap();
        let mut app = DawApp::with_context(&ctx, project, None);
        let source = daw_ui::SampleDrop {
            path: "/source/kick.wav".into(),
            name: Some("Soft kick".into()),
        };
        let ticket = app.queue_project_sample_import_tracked(vec![source.clone()]);
        let Some(ImportOutcome::Succeeded(receipt)) = app.audio_import_outcome(ticket) else {
            panic!("managed no-op did not complete synchronously")
        };
        assert_eq!(receipt.sources.len(), 1);
        assert_eq!(receipt.sources[0].project_sample_id, Some(id));
        assert_eq!(receipt.sources[0].source, source);
        assert!(!app.audio_import_busy());
    }

    #[test]
    fn a_worker_failure_terminalizes_the_following_queue() {
        let ctx = egui::Context::default();
        let mut app = DawApp::with_context(&ctx, daw_core::Project::new(), None);
        let failed = app
            .queue_project_sample_import_tracked(vec![PathBuf::from("/source/first.wav").into()]);
        let queued = app
            .queue_project_sample_import_tracked(vec![PathBuf::from("/source/second.wav").into()]);
        app.fail_audio_import(failed, anyhow::anyhow!("decoder stopped"));
        assert!(
            matches!(app.audio_import_outcome(failed), Some(ImportOutcome::Failed(message)) if message == "decoder stopped")
        );
        assert!(matches!(
            app.audio_import_outcome(queued),
            Some(ImportOutcome::Cancelled)
        ));
        assert!(!app.audio_import_busy());
    }
}
