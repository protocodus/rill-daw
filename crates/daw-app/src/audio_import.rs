//! Background, isolated decoding and atomic import into the captured document.
use crate::app::DawApp;
use anyhow::{Context, Result, bail, ensure};
use daw_core::{AudioData, Clip, MAX_BEAT, TrackKind};
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
const DECODE_BUDGET: usize = 512 * 1024 * 1024;
const PREVIEW_BUDGET: usize = 128 * 1024 * 1024;
const MAX_LOG: u64 = 8192;

struct Request {
    session: u64,
    paths: Vec<PathBuf>,
    target: Option<u64>,
    beat: f64,
    scratch: PathBuf,
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
}

struct Batch {
    assets: Assets,
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
        .unwrap_or(30_000);
    Duration::from_millis(millis.clamp(10, 120_000))
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
    paths: &[PathBuf],
    scratch: &Path,
    cancel: &AtomicBool,
    completed: &AtomicUsize,
) -> Result<Batch> {
    let assets = Assets::create(scratch)?;
    let mut remaining = DECODE_BUDGET;
    let mut preview_remaining = PREVIEW_BUDGET;
    let mut files = Vec::with_capacity(paths.len());
    for (index, input) in paths.iter().enumerate() {
        let output = assets.directory.join(format!("{}.wav", index + 1));
        let info = decode_one(input, &output, remaining, cancel)
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
        if bytes <= crate::project_io::WAVEFORM_FILE_BYTES && bytes <= preview_remaining {
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
            .context("Audio import batch exceeds 512 MiB")?;
        let preview = preview.map(|samples| {
            preview_remaining -= bytes;
            AudioData {
                sample_rate: info.sample_rate,
                channels: info.channels,
                samples,
            }
        });
        files.push(Imported {
            path: fs::canonicalize(output)?,
            name: input
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Audio".into()),
            frames: info.frames,
            sample_rate: info.sample_rate,
            preview,
        });
        completed.store(index + 1, Ordering::Release);
    }
    ensure!(!cancel.load(Ordering::Acquire), "Audio import cancelled");
    Ok(Batch { assets, files })
}

impl DawApp {
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
                active.request.paths.len(),
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
        self.audio_import.queued.clear();
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
        let result = (|| -> Result<()> {
            ensure!(
                !paths.is_empty() && paths.len() <= MAX_FILES,
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
            ensure!(
                beat.is_finite() && (0.0..=MAX_BEAT).contains(&beat),
                "Invalid audio import position"
            );
            let target = match target {
                Some(id) => {
                    let track = self
                        .project
                        .track(id)
                        .context("Import target no longer exists")?;
                    (track.kind == TrackKind::Audio).then_some(id)
                }
                None => None,
            };
            self.audio_import.queued.push_back(Request {
                session: self.song_session.id,
                paths,
                target,
                beat,
                scratch: self.settings.scratch_dir.clone(),
            });
            Ok(())
        })();
        match result {
            Ok(()) => {
                if self.audio_import.last_error_status.as_deref() == Some(self.status.as_str()) {
                    self.error = false;
                }
                self.audio_import.last_error_status = None;
                if !self.error {
                    self.status = "Preparing audio import…".into();
                }
            }
            Err(error) => self.audio_import_error(error),
        }
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
                        if active.cancel.load(Ordering::Acquire) {
                            drop(result);
                            if active.request.session == self.song_session.id
                                && self.document_guard.is_none()
                                && !self.project_io_pending()
                                && !self.error
                            {
                                self.status = "Audio import canceled".into();
                            }
                        } else if active.request.session != self.song_session.id {
                            drop(result);
                        } else {
                            let result = result
                                .and_then(|batch| self.commit_audio_import(&active.request, batch));
                            if let Err(error) = result {
                                self.audio_import.queued.clear();
                                self.audio_import_error(error);
                            }
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => self.audio_import.active = Some(active),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.audio_import.queued.clear();
                        if !active.cancel.load(Ordering::Acquire) {
                            self.audio_import_error(anyhow::anyhow!(
                                "Audio import worker stopped unexpectedly; no clips were imported"
                            ));
                        }
                    }
                }
            }
        }
        if self.audio_import.active.is_none()
            && let Some(request) = self.audio_import.queued.pop_front()
            && request.session == self.song_session.id
        {
            let (sender, receiver) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let completed = Arc::new(AtomicUsize::new(0));
            let worker_cancel = Arc::clone(&cancel);
            let worker_completed = Arc::clone(&completed);
            let paths = request.paths.clone();
            let scratch = request.scratch.clone();
            match std::thread::Builder::new()
                .name("rill-audio-import".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        decode_batch(&paths, &scratch, &worker_cancel, &worker_completed)
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
                Err(error) => {
                    self.audio_import.queued.clear();
                    self.audio_import_error(error.into());
                }
            }
        }
        if self.audio_import_busy() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn commit_audio_import(&mut self, request: &Request, mut batch: Batch) -> Result<()> {
        ensure!(
            request.session == self.song_session.id,
            "Import belongs to a replaced document"
        );
        if let Some(target) = request.target {
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
                length.is_finite() && length > 0.0 && request.beat + length <= MAX_BEAT,
                "Imported audio extends beyond the supported timeline"
            );
            let target = request
                .target
                .unwrap_or_else(|| candidate.add_audio_track(&file.name));
            let path = file.path.to_str().context("Managed audio folder is not valid Unicode; choose another Storage folder before importing")?;
            let clip = Clip::audio(&file.name, request.beat, length, path);
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
        batch.assets.retained = true;
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
        let mut used: usize = self
            .waveforms
            .values()
            .map(|audio| audio.samples.len() * 4)
            .sum();
        for file in &mut batch.files {
            if let Some(preview) = file.preview.take() {
                let bytes = preview.samples.len() * 4;
                if bytes <= crate::project_io::WAVEFORM_CACHE_BYTES.saturating_sub(used) {
                    self.waveforms
                        .insert(file.path.to_string_lossy().into_owned(), preview);
                    used += bytes;
                }
            }
        }
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
        Ok(())
    }

    pub(crate) fn audio_import_diagnostics(&self) -> serde_json::Value {
        let next = self
            .audio_import
            .active
            .as_ref()
            .map(|active| &active.request)
            .or_else(|| self.audio_import.queued.front());
        serde_json::json!({"busy":self.audio_import_busy(),"queued":self.audio_import.queued.len(),
            "files":self.audio_import.active.as_ref().map_or(0, |active| active.request.paths.len()),
            "completed":self.audio_import.active.as_ref().map_or(0, |active| active.completed.load(Ordering::Acquire)),
            "cancel_requested":self.audio_import.active.as_ref().is_some_and(|active| active.cancel.load(Ordering::Acquire)),
            "next":next.map(|request| serde_json::json!({"session":request.session,"target":request.target,
                "beat":request.beat,"files":request.paths.len()}))})
    }
}
