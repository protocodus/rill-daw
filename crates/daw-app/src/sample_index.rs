//! An on-demand file catalogue. Loading its cache never traverses sample roots.
//! Only `scan` starts directory I/O, in a cancellable worker; the UI reads snapshots.
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

const MAX_FILES: usize = 100_000;
const MAX_DIRECTORIES: usize = 25_000;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SampleEntry {
    pub root: PathBuf,
    pub relative: PathBuf,
    pub bytes: u64,
}
impl SampleEntry {
    pub fn path(&self) -> PathBuf {
        self.root.join(&self.relative)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Snapshot {
    roots: Vec<PathBuf>,
    entries: Vec<SampleEntry>,
    warnings: Vec<String>,
}

struct Scan {
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Snapshot, String>>,
}

pub(crate) struct SampleIndex {
    snapshot: Snapshot,
    scan: Option<Scan>,
    pub revision: u64,
    pub error: Option<String>,
}

impl Default for SampleIndex {
    fn default() -> Self {
        let snapshot = std::fs::read(cache_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Snapshot>(&bytes).ok())
            .filter(|snapshot| snapshot.entries.len() <= MAX_FILES)
            .unwrap_or_default();
        Self {
            snapshot,
            scan: None,
            revision: 0,
            error: None,
        }
    }
}

fn cache_path() -> PathBuf {
    crate::settings::config_dir().join("sample-index.json")
}

pub(crate) fn supported(path: &Path) -> bool {
    daw_core::is_supported_audio_path(path)
}

impl SampleIndex {
    pub fn entries(&self) -> &[SampleEntry] {
        &self.snapshot.entries
    }
    pub fn roots(&self) -> &[PathBuf] {
        &self.snapshot.roots
    }
    pub fn warnings(&self) -> &[String] {
        &self.snapshot.warnings
    }
    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }
    pub fn scan(&mut self, roots: Vec<PathBuf>, ctx: egui::Context) {
        if self.scan.is_some() {
            return;
        }
        self.error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, result) = mpsc::channel();
        std::thread::spawn(move || {
            let result = scan_roots(roots, &worker_cancel).and_then(|snapshot| {
                if worker_cancel.load(Ordering::Acquire) {
                    return Err("Sample scan canceled".into());
                }
                let path = cache_path();
                let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;
                let directory = path.parent().unwrap();
                std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
                let temporary = directory.join(format!("sample-index-{}.tmp", std::process::id()));
                std::fs::write(&temporary, bytes)
                    .and_then(|()| std::fs::rename(&temporary, &path))
                    .map_err(|error| {
                        let _ = std::fs::remove_file(&temporary);
                        error.to_string()
                    })?;
                Ok(snapshot)
            });
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.scan = Some(Scan { cancel, result });
    }
    pub fn cancel(&self) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Release);
        }
    }
    pub fn poll(&mut self) {
        let Some(result) = self
            .scan
            .as_ref()
            .and_then(|scan| match scan.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Sample scan worker stopped".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            })
        else {
            return;
        };
        self.scan = None;
        match result {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.revision = self.revision.wrapping_add(1);
            }
            Err(error) => self.error = Some(error),
        }
    }
}
impl Drop for SampleIndex {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn scan_roots(roots: Vec<PathBuf>, cancel: &AtomicBool) -> Result<Snapshot, String> {
    let mut snapshot = Snapshot {
        roots: roots.clone(),
        ..Default::default()
    };
    let mut seen = BTreeSet::new();
    let mut directories = 0usize;
    for root in roots {
        let mut queue = vec![(root.clone(), 0usize)];
        while let Some((directory, depth)) = queue.pop() {
            if cancel.load(Ordering::Acquire) {
                return Err("Sample scan canceled".into());
            }
            if directories >= MAX_DIRECTORIES || snapshot.entries.len() >= MAX_FILES {
                snapshot.warnings.push(format!("Index limit reached ({MAX_FILES} files or {MAX_DIRECTORIES} folders). Choose narrower sample folders."));
                return Ok(snapshot);
            }
            directories += 1;
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) => {
                    if snapshot.warnings.len() < 20 {
                        snapshot
                            .warnings
                            .push(format!("{}: {error}", directory.display()));
                    }
                    continue;
                }
            };
            for entry in entries.filter_map(Result::ok) {
                if cancel.load(Ordering::Acquire) {
                    return Err("Sample scan canceled".into());
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                // Do not follow symlinks: cycles and roots escaping the user's
                // chosen folder cannot silently expand a scan.
                if kind.is_dir() && depth < MAX_DEPTH {
                    if directories + queue.len() < MAX_DIRECTORIES {
                        queue.push((path, depth + 1));
                    } else if snapshot.warnings.len() < 20 {
                        snapshot
                            .warnings
                            .push("Folder limit reached; choose narrower sample folders.".into());
                    }
                } else if kind.is_file() && supported(&path) && seen.insert(path.clone()) {
                    let Ok(relative) = path.strip_prefix(&root) else {
                        continue;
                    };
                    snapshot.entries.push(SampleEntry {
                        root: root.clone(),
                        relative: relative.into(),
                        bytes: entry.metadata().map_or(0, |metadata| metadata.len()),
                    });
                    if snapshot.entries.len() >= MAX_FILES {
                        snapshot.warnings.push(format!(
                            "File limit reached ({MAX_FILES}); choose narrower sample folders."
                        ));
                        break;
                    }
                }
            }
        }
    }
    snapshot
        .entries
        .sort_by(|left, right| (&left.root, &left.relative).cmp(&(&right.root, &right.relative)));
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_scan_indexes_audio_tree_and_deduplicates_overlapping_roots() {
        let directory = tempfile::tempdir().unwrap();
        let sub = directory.path().join("Drums");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("Kick.WAV"), "audio").unwrap();
        std::fs::write(sub.join("notes.txt"), "text").unwrap();
        let index =
            scan_roots(vec![directory.path().into(), sub], &AtomicBool::new(false)).unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].relative, Path::new("Drums/Kick.WAV"));
        assert_eq!(index.entries[0].bytes, 5);
    }
    #[test]
    fn cancellation_never_publishes_partial_index() {
        assert!(
            scan_roots(
                vec![PathBuf::from("/does-not-matter")],
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .contains("canceled")
        );
    }
}
