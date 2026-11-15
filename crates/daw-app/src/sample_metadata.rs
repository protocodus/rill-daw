//! Selected-source inspection: one isolated parser at a time, a latest-request
//! queue, and a bounded cache. No directory scanning or decoding happens here.
use daw_core::AudioMetadata;
use eframe::egui;
use std::{
    collections::VecDeque,
    ffi::OsString,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const CACHE_CAPACITY: usize = 256;
const WORKER_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) enum SampleMetadataStatus<'a> {
    Loading,
    Ready(&'a AudioMetadata),
    Failed(&'a str),
}

struct Pending {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<AudioMetadata, String>>,
}

#[derive(Default)]
pub(crate) struct SampleMetadataState {
    selected: Option<PathBuf>,
    pending: Option<Pending>,
    queued: Option<PathBuf>,
    cache: VecDeque<(PathBuf, Result<AudioMetadata, String>)>,
}

impl SampleMetadataState {
    /// Retry just this source; folder indexing and other cached entries stay intact.
    pub fn retry(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.cache.retain(|(cached, _)| cached != &path);
        if self.selected.as_ref() == Some(&path) {
            self.selected = None;
        }
        self.request(path, ctx);
    }

    pub fn request(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.selected.as_ref() == Some(&path) {
            return;
        }
        self.selected = Some(path.clone());
        self.queued = None;
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Release);
        }
        if let Some(index) = self.cache.iter().position(|(cached, _)| cached == &path) {
            let cached = self.cache.remove(index).unwrap();
            self.cache.push_back(cached);
            return;
        }
        self.queued = Some(path);
        self.start_queued(ctx);
    }

    fn start_queued(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let Some(path) = self.queued.take() else {
            return;
        };
        let (sender, result) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_path = path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = probe_isolated(&worker_path, &worker_cancel);
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.pending = Some(Pending {
            path,
            cancel,
            result,
        });
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let result = self
            .pending
            .as_ref()
            .and_then(|pending| match pending.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Sample metadata worker stopped".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            let pending = self.pending.take().unwrap();
            if !pending.cancel.load(Ordering::Acquire)
                && self.selected.as_ref() == Some(&pending.path)
            {
                self.cache.retain(|(path, _)| path != &pending.path);
                self.cache.push_back((pending.path, result));
                while self.cache.len() > CACHE_CAPACITY {
                    self.cache.pop_front();
                }
            }
        }
        self.start_queued(ctx);
        if self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    pub fn selected(&self, path: &Path) -> Option<SampleMetadataStatus<'_>> {
        if let Some((_, result)) = self.cache.iter().find(|(cached, _)| cached == path) {
            return Some(match result {
                Ok(metadata) => SampleMetadataStatus::Ready(metadata),
                Err(error) => SampleMetadataStatus::Failed(error),
            });
        }
        (self.queued.as_deref() == Some(path)
            || self.pending.as_ref().is_some_and(|pending| {
                pending.path == path && !pending.cancel.load(Ordering::Acquire)
            }))
        .then_some(SampleMetadataStatus::Loading)
    }

    /// Called after an explicit rescan/document replacement, so stale details
    /// do not survive replacement of a file at the same path.
    pub fn invalidate(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Release);
        }
        self.selected = None;
        self.queued = None;
        self.cache.clear();
    }
}

impl Drop for SampleMetadataState {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Release);
        }
    }
}

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn probe_isolated(path: &Path, cancel: &AtomicBool) -> Result<AudioMetadata, String> {
    let run = || -> anyhow::Result<AudioMetadata> {
        anyhow::ensure!(!cancel.load(Ordering::Acquire), "Sample metadata cancelled");
        let mut worker = Worker(
            Command::new(std::env::current_exe()?)
                .arg("--probe-audio-worker")
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
        );
        let deadline = Instant::now() + WORKER_TIMEOUT;
        let status = loop {
            anyhow::ensure!(!cancel.load(Ordering::Acquire), "Sample metadata cancelled");
            anyhow::ensure!(
                Instant::now() < deadline,
                "Sample metadata inspection timed out"
            );
            if let Some(status) = worker.0.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // The one-shot worker emits a tiny fixed-schema JSON record or a
        // bounded error. Retain a hard read cap at the supervisor boundary.
        let mut bytes = Vec::new();
        worker
            .0
            .stdout
            .take()
            .unwrap()
            .take(8193)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 8192, "Sample metadata response is too large");
        if !status.success() {
            let mut error = String::new();
            worker
                .0
                .stderr
                .take()
                .unwrap()
                .take(2048)
                .read_to_string(&mut error)?;
            anyhow::bail!(
                "{}",
                if error.trim().is_empty() {
                    "Sample metadata is unavailable"
                } else {
                    error.trim()
                }
            );
        }
        Ok(serde_json::from_slice(&bytes)?)
    };
    run().map_err(|error| error.to_string())
}

/// Internal one-shot mode, before any window, settings or audio device opens.
pub fn run_worker(args: impl Iterator<Item = OsString>) -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<_> {
        let args: Vec<_> = args.take(2).collect();
        anyhow::ensure!(args.len() == 1, "Usage: --probe-audio-worker input");
        daw_core::probe_audio_metadata(Path::new(&args[0]))
    }))
    .unwrap_or_else(|_| Err(anyhow::anyhow!("Sample metadata parser failed")));
    match result {
        Ok(metadata) => {
            serde_json::to_writer(std::io::stdout().lock(), &metadata)?;
            println!();
            Ok(())
        }
        Err(error) => {
            let bounded: String = format!("{error:#}").chars().take(1000).collect();
            let _ = writeln!(std::io::stderr().lock(), "{bounded}");
            std::process::exit(1);
        }
    }
}
