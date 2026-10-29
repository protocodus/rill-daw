//! Background catalog synchronization. One worker thread owns scanning, preview
//! capture and index writes, so the UI reads snapshots and never waits on the
//! filesystem or a native editor. Captures run one at a time, each in its own
//! isolated helper process supplied by the caller.

use crate::{
    PluginInfo,
    catalog::{CatalogEntry, PluginCatalog, ThumbnailStatus},
};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

pub type CaptureFn = fn(&PluginInfo, &Path) -> anyhow::Result<()>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncPhase {
    #[default]
    Idle,
    Scanning,
    Capturing,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncStatus {
    pub phase: SyncPhase,
    /// Previews finished and still waiting in the current pass.
    pub captured: usize,
    pub queued: usize,
    /// The plug-in whose editor is being captured.
    pub current: Option<String>,
    /// Completion time of the last scan, including one loaded from the index.
    pub synced_unix_ms: Option<u64>,
}

enum Request {
    Sync { paths: Vec<PathBuf>, capture: bool },
    Capture(PluginInfo),
}

#[derive(Default)]
struct Shared {
    entries: Vec<CatalogEntry>,
    revision: u64,
    status: SyncStatus,
    /// Requests sent but not yet taken by the worker; they count as work so
    /// callers never observe a false Idle right after asking for something.
    pending_scans: usize,
    pending_captures: usize,
    errors: Vec<String>,
    captured: Vec<String>,
}

pub struct CatalogSync {
    catalog: PluginCatalog,
    capture: CaptureFn,
    notify: Arc<dyn Fn() + Send + Sync>,
    shared: Arc<Mutex<Shared>>,
    requests: Option<mpsc::Sender<Request>>,
}

impl CatalogSync {
    /// `notify` runs on the worker after every published change, typically to
    /// request a UI repaint. Nothing starts until the first request.
    pub fn new(
        catalog: PluginCatalog,
        capture: CaptureFn,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            catalog,
            capture,
            notify: Arc::new(notify),
            shared: Default::default(),
            requests: None,
        }
    }

    /// Show the persisted index (first request only), rescan `paths`, save the
    /// index, then capture missing previews of verified plug-ins when `capture`
    /// is set. Unverified bundles are only captured on explicit request.
    pub fn sync(&mut self, paths: Vec<PathBuf>, capture: bool) {
        let mut shared = lock(&self.shared);
        shared.pending_scans += 1;
        shared.status.phase = SyncPhase::Scanning;
        drop(shared);
        self.send(Request::Sync { paths, capture });
    }

    /// Capture one preview ahead of any automatic work, retrying failures.
    pub fn capture(&mut self, plugin: PluginInfo) {
        let mut shared = lock(&self.shared);
        shared.pending_captures += 1;
        shared.status.queued += 1;
        if shared.status.phase == SyncPhase::Idle {
            shared.status.phase = SyncPhase::Capturing;
        }
        drop(shared);
        self.send(Request::Capture(plugin));
    }

    pub fn scanning(&self) -> bool {
        lock(&self.shared).pending_scans > 0
    }

    pub fn status(&self) -> SyncStatus {
        lock(&self.shared).status.clone()
    }

    /// Entries newer than `revision`, with the revision to pass next time.
    pub fn entries_since(&self, revision: u64) -> Option<(u64, Vec<CatalogEntry>)> {
        let shared = lock(&self.shared);
        (shared.revision != revision).then(|| (shared.revision, shared.entries.clone()))
    }

    /// Scan, cache and index failures since the last call. A plug-in whose
    /// editor fails is not an error here; its entry records `Failed`.
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut lock(&self.shared).errors)
    }

    /// Fingerprints whose preview image was rewritten since the last call. A
    /// refreshed capture keeps its path, so decoded copies must be dropped.
    pub fn take_captured(&self) -> Vec<String> {
        std::mem::take(&mut lock(&self.shared).captured)
    }

    fn send(&mut self, request: Request) {
        // A worker that died (it never panics by design) is replaced.
        let request = match &self.requests {
            Some(sender) => match sender.send(request) {
                Ok(()) => return,
                Err(mpsc::SendError(request)) => request,
            },
            None => request,
        };
        let (sender, receiver) = mpsc::channel();
        sender.send(request).expect("receiver is alive");
        let worker = Worker {
            catalog: self.catalog.clone(),
            capture: self.capture,
            notify: self.notify.clone(),
            shared: self.shared.clone(),
            automatic: VecDeque::new(),
            explicit: VecDeque::new(),
        };
        if let Err(error) = std::thread::Builder::new()
            .name("rill-plugin-catalog".into())
            .spawn(move || worker.run(receiver))
        {
            let mut shared = lock(&self.shared);
            shared.pending_scans = 0;
            shared.pending_captures = 0;
            shared.status.queued = 0;
            shared.status.phase = SyncPhase::Idle;
            shared
                .errors
                .push(format!("Starting plug-in catalog: {error}"));
            return;
        }
        self.requests = Some(sender);
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|error| error.into_inner())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_millis() as u64)
}

fn same_plugin(a: &PluginInfo, b: &PluginInfo) -> bool {
    a.path == b.path && a.class_id == b.class_id
}

struct Worker {
    catalog: PluginCatalog,
    capture: CaptureFn,
    notify: Arc<dyn Fn() + Send + Sync>,
    shared: Arc<Mutex<Shared>>,
    automatic: VecDeque<PluginInfo>,
    explicit: VecDeque<PluginInfo>,
}

impl Worker {
    fn run(mut self, requests: mpsc::Receiver<Request>) {
        if let Some((entries, synced)) = self.catalog.load_index() {
            let mut shared = lock(&self.shared);
            shared.entries = entries;
            shared.revision += 1;
            shared.status.synced_unix_ms = Some(synced);
            drop(shared);
            (self.notify)();
        }
        loop {
            // Handle every waiting request before starting the next capture,
            // so a rescan or an explicit capture never waits behind the queue.
            let request = if self.automatic.is_empty() && self.explicit.is_empty() {
                match requests.recv() {
                    Ok(request) => request,
                    Err(_) => return,
                }
            } else {
                match requests.try_recv() {
                    Ok(request) => request,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                    Err(mpsc::TryRecvError::Empty) => {
                        self.capture_next();
                        continue;
                    }
                }
            };
            match request {
                Request::Sync { paths, capture } => self.scan(&paths, capture),
                Request::Capture(plugin) => {
                    self.automatic
                        .retain(|queued| !same_plugin(queued, &plugin));
                    self.explicit.retain(|queued| !same_plugin(queued, &plugin));
                    self.explicit.push_back(plugin);
                    let mut shared = lock(&self.shared);
                    shared.pending_captures -= 1;
                    self.settle(&mut shared);
                }
            }
        }
    }

    fn scan(&mut self, paths: &[PathBuf], capture: bool) {
        let previous: HashMap<_, _> = lock(&self.shared)
            .entries
            .iter()
            .map(|entry| {
                (
                    (entry.plugin.path.clone(), entry.plugin.class_id.clone()),
                    entry.added_unix_ms,
                )
            })
            .collect();
        let result = self.catalog.scan(paths);
        let synced = now_unix_ms();
        let mut entries = match result {
            Ok(entries) => entries,
            Err(error) => {
                let mut shared = lock(&self.shared);
                shared.pending_scans -= 1;
                shared.errors.push(format!("{error:#}"));
                self.settle(&mut shared);
                drop(shared);
                (self.notify)();
                return;
            }
        };
        for entry in &mut entries {
            entry.added_unix_ms = previous
                .get(&(entry.plugin.path.clone(), entry.plugin.class_id.clone()))
                .copied()
                .filter(|added| *added > 0)
                .unwrap_or(synced);
        }
        self.automatic = if capture {
            entries
                .iter()
                .filter(|entry| {
                    entry.status == ThumbnailStatus::Missing
                        && (entry.plugin.is_instrument() == Some(true)
                            || entry.plugin.is_effect() == Some(true))
                        && !self
                            .explicit
                            .iter()
                            .any(|queued| same_plugin(queued, &entry.plugin))
                })
                .map(|entry| entry.plugin.clone())
                .collect()
        } else {
            VecDeque::new()
        };
        let saved = self.catalog.save_index(&entries, synced);
        let mut shared = lock(&self.shared);
        shared.entries = entries;
        shared.revision += 1;
        shared.pending_scans -= 1;
        shared.status.synced_unix_ms = Some(synced);
        shared.status.captured = 0;
        if let Err(error) = saved {
            shared
                .errors
                .push(format!("Saving plug-in index: {error:#}"));
        }
        self.settle(&mut shared);
        drop(shared);
        (self.notify)();
    }

    fn capture_next(&mut self) {
        let Some(plugin) = self
            .explicit
            .pop_front()
            .or_else(|| self.automatic.pop_front())
        else {
            return;
        };
        let mut shared = lock(&self.shared);
        self.settle(&mut shared);
        shared.status.phase = SyncPhase::Capturing;
        shared.status.current = Some(plugin.name.clone());
        drop(shared);
        (self.notify)();
        let result = self.catalog.capture(&plugin, self.capture);
        let mut guard = lock(&self.shared);
        let shared = &mut *guard;
        match result {
            Ok(mut entry) => {
                if let Some(slot) = shared
                    .entries
                    .iter_mut()
                    .find(|slot| same_plugin(&slot.plugin, &plugin))
                {
                    entry.added_unix_ms = slot.added_unix_ms;
                    shared.captured.push(entry.fingerprint.clone());
                    *slot = entry;
                    shared.revision += 1;
                }
            }
            Err(error) => shared.errors.push(format!("{error:#}")),
        }
        let entries = shared.entries.clone();
        let synced = shared.status.synced_unix_ms.unwrap_or_else(now_unix_ms);
        drop(guard);
        // Write before reporting Idle: a finished sync is always on disk.
        let saved = self.catalog.save_index(&entries, synced);
        let mut shared = lock(&self.shared);
        if let Err(error) = saved {
            shared
                .errors
                .push(format!("Saving plug-in index: {error:#}"));
        }
        shared.status.captured += 1;
        shared.status.current = None;
        self.settle(&mut shared);
        drop(shared);
        (self.notify)();
    }

    fn settle(&self, shared: &mut Shared) {
        shared.status.queued = self.automatic.len() + self.explicit.len() + shared.pending_captures;
        shared.status.phase = if shared.pending_scans > 0 {
            SyncPhase::Scanning
        } else if shared.status.queued > 0 {
            SyncPhase::Capturing
        } else {
            SyncPhase::Idle
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant},
    };

    fn bundle(root: &Path, bundle: &str, classes: &[(&str, &str, &str)]) {
        let resources = root.join(format!("{bundle}.vst3/Contents/Resources"));
        fs::create_dir_all(&resources).unwrap();
        fs::create_dir_all(root.join(format!("{bundle}.vst3/Contents/x86_64-linux"))).unwrap();
        let classes: Vec<_> = classes
            .iter()
            .map(|(name, category, id)| {
                serde_json::json!({"CID": id, "Category": "Audio Module Class", "Name": name,
                    "Sub Categories": [category], "Version": "2.1", "SDKVersion": "VST 3.7.9",
                    "Cardinality": 2147483647})
            })
            .collect();
        fs::write(
            resources.join("moduleinfo.json"),
            serde_json::json!({"Name": bundle, "Version": "1", "Factory Info": {"Vendor": "Acme"},
                "Classes": classes})
            .to_string(),
        )
        .unwrap();
    }

    fn png(_: &PluginInfo, path: &Path) -> anyhow::Result<()> {
        let mut encoder = png::Encoder::new(fs::File::create(path)?, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&[1, 2, 3])?;
        Ok(())
    }

    fn crash(_: &PluginInfo, _: &Path) -> anyhow::Result<()> {
        anyhow::bail!("editor crashed")
    }

    fn settled(sync: &CatalogSync) -> Vec<CatalogEntry> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if !sync.scanning() && sync.status().phase == SyncPhase::Idle {
                return sync.entries_since(u64::MAX).unwrap().1;
            }
            assert!(Instant::now() < deadline, "sync did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn named<'a>(entries: &'a [CatalogEntry], name: &str) -> &'a CatalogEntry {
        entries
            .iter()
            .find(|entry| entry.plugin.name == name)
            .unwrap()
    }

    #[test]
    fn sync_indexes_metadata_and_captures_only_verified_missing_previews() {
        let plugins = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        bundle(
            plugins.path(),
            "Pair",
            &[
                ("Keys", "Instrument", "11111111111111111111111111111111"),
                ("Echo", "Fx", "22222222222222222222222222222222"),
            ],
        );
        fs::create_dir_all(plugins.path().join("Unverified.vst3")).unwrap();
        let catalog = PluginCatalog::new(cache.path());
        let mut sync = CatalogSync::new(catalog.clone(), png, || {});
        sync.sync(vec![plugins.path().to_owned()], true);
        assert!(sync.scanning(), "a requested scan is visible immediately");
        let entries = settled(&sync);
        assert_eq!(entries.len(), 3);
        for name in ["Keys", "Echo"] {
            let entry = named(&entries, name);
            assert_eq!(entry.status, ThumbnailStatus::Ready);
            assert!(entry.thumbnail.as_ref().unwrap().is_file());
            assert_eq!(entry.plugin.version, "2.1");
            assert_eq!(entry.plugin.architectures, ["x86_64-linux"]);
            assert!(entry.added_unix_ms > 0);
        }
        assert_eq!(
            named(&entries, "Unverified").status,
            ThumbnailStatus::Missing
        );
        assert_eq!(sync.status().captured, 2);
        assert_eq!(sync.take_captured().len(), 2);
        assert!(sync.take_captured().is_empty());
        assert!(sync.take_errors().is_empty());

        let (indexed, synced) = catalog.load_index().unwrap();
        assert_eq!(indexed, entries);
        assert_eq!(sync.status().synced_unix_ms, Some(synced));

        sync.capture(named(&entries, "Unverified").plugin.clone());
        let entries = settled(&sync);
        assert_eq!(named(&entries, "Unverified").status, ThumbnailStatus::Ready);
        assert_eq!(catalog.load_index().unwrap().0, entries);
    }

    #[test]
    fn restarted_sync_shows_the_index_and_resync_tracks_installs_and_removals() {
        let plugins = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        bundle(
            plugins.path(),
            "Old",
            &[("Old Keys", "Instrument", "33333333333333333333333333333333")],
        );
        bundle(
            plugins.path(),
            "Gone",
            &[("Gone Echo", "Fx", "44444444444444444444444444444444")],
        );
        let catalog = PluginCatalog::new(cache.path());
        let mut first = CatalogSync::new(catalog.clone(), crash, || {});
        first.sync(vec![plugins.path().to_owned()], true);
        let original = settled(&first);
        assert!(
            original
                .iter()
                .all(|entry| matches!(entry.status, ThumbnailStatus::Failed { .. })),
            "editor failures are recorded per plug-in, not reported as errors"
        );
        assert!(first.take_errors().is_empty());
        drop(first);

        // A new session shows the index; a scan that cannot finish keeps it.
        #[cfg(unix)]
        {
            let broken = plugins.path().join("Broken.vst3");
            fs::create_dir(&broken).unwrap();
            std::os::unix::fs::symlink(broken.join("missing"), broken.join("link")).unwrap();
            let mut failing = CatalogSync::new(catalog.clone(), png, || {});
            failing.sync(vec![plugins.path().to_owned()], true);
            assert_eq!(settled(&failing), original);
            assert_eq!(failing.take_errors().len(), 1);
            drop(failing);
            fs::remove_dir_all(broken).unwrap();
        }

        fs::remove_dir_all(plugins.path().join("Gone.vst3")).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        bundle(
            plugins.path(),
            "New",
            &[("New Keys", "Instrument", "55555555555555555555555555555555")],
        );
        let mut next = CatalogSync::new(catalog.clone(), png, || {});
        next.sync(vec![plugins.path().to_owned()], true);
        let entries = settled(&next);
        let names: Vec<_> = entries
            .iter()
            .map(|entry| entry.plugin.name.as_str())
            .collect();
        assert_eq!(names, ["New Keys", "Old Keys"]);
        assert_eq!(
            named(&entries, "Old Keys").added_unix_ms,
            named(&original, "Old Keys").added_unix_ms,
            "an installed plug-in keeps its first-indexed time"
        );
        assert!(
            named(&entries, "New Keys").added_unix_ms > named(&entries, "Old Keys").added_unix_ms
        );
        assert_eq!(named(&entries, "New Keys").status, ThumbnailStatus::Ready);
        assert!(
            matches!(
                named(&entries, "Old Keys").status,
                ThumbnailStatus::Failed { .. }
            ),
            "a failed editor is not relaunched by every sync"
        );
    }

    #[test]
    fn index_never_references_files_outside_the_preview_cache() {
        let cache = tempfile::tempdir().unwrap();
        let catalog = PluginCatalog::new(cache.path());
        let entry = |fingerprint: &str| CatalogEntry {
            plugin: PluginInfo {
                name: fingerprint.into(),
                ..PluginInfo::default()
            },
            fingerprint: fingerprint.into(),
            thumbnail: Some("/etc/passwd".into()),
            thumbnail_size: Some([1, 1]),
            status: ThumbnailStatus::Ready,
            added_unix_ms: 1,
        };
        let valid = "ab".repeat(32);
        catalog
            .save_index(&[entry(&valid), entry("../../escape")], 42)
            .unwrap();
        let (entries, synced) = catalog.load_index().unwrap();
        assert_eq!(synced, 42);
        assert_eq!(entries.len(), 1);
        let thumbnail = entries[0].thumbnail.as_ref().unwrap();
        assert!(thumbnail.starts_with(cache.path()));
        assert!(thumbnail.ends_with(format!("{valid}.png")));
        fs::write(cache.path().join("plugin-index-v1.json"), b"{truncated").unwrap();
        assert!(catalog.load_index().is_none());
    }
}
