//! Global sample bookmarks. Project-owned audio is retained in a durable vault
//! before publication, so closing a song never invalidates a favorite.
use anyhow::{Context, Result, ensure};
use daw_core::SampleFavorite;
use daw_ui::SampleDrop;
use eframe::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

const MAX_ENTRIES: usize = 10_000;
const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;
type SourceKey = (PathBuf, Option<String>);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SampleLibraryEntry {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub favorite: SampleFavorite,
    pub content_hash: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Catalog {
    version: u32,
    entries: Vec<SampleLibraryEntry>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

struct OwnedFile {
    path: PathBuf,
    retained: bool,
}
impl Drop for OwnedFile {
    fn drop(&mut self) {
        if !self.retained {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct Prepared {
    path: PathBuf,
    hash: Option<String>,
    asset: Option<OwnedFile>,
}

enum Operation {
    Add { retain: bool },
    Resolve,
}
struct Job {
    source: SampleDrop,
    operation: Operation,
}
struct Active {
    job: Job,
    result: mpsc::Receiver<Result<Prepared>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub(crate) struct SampleLibrary {
    directory: PathBuf,
    catalog: Catalog,
    by_path: BTreeMap<PathBuf, Vec<usize>>,
    by_id: BTreeMap<String, usize>,
    aliases: BTreeMap<SourceKey, String>,
    resolved: BTreeSet<SourceKey>,
    active: Option<Active>,
    queued: Option<Job>,
    revision: u64,
    errors: Vec<String>,
    load_error: Option<String>,
}
impl Default for SampleLibrary {
    fn default() -> Self {
        Self::load(crate::settings::config_dir())
    }
}

impl SampleLibrary {
    pub(crate) fn load(directory: PathBuf) -> Self {
        let directory = if directory.is_absolute() {
            directory
        } else {
            std::env::current_dir().unwrap_or_default().join(directory)
        };
        let path = directory.join("sample-library.json");
        let loaded = (|| -> Result<Catalog> {
            if !path.exists() {
                return Ok(Catalog::default());
            }
            let mut bytes = Vec::new();
            File::open(&path)?
                .take(MAX_CATALOG_BYTES + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= MAX_CATALOG_BYTES,
                "Sample library exceeds its size limit"
            );
            let catalog: Catalog = serde_json::from_slice(&bytes)?;
            validate_catalog(&catalog)?;
            Ok(catalog)
        })();
        let (catalog, errors) = match loaded {
            Ok(catalog) => (catalog, Vec::new()),
            Err(error) => (
                Catalog::default(),
                vec![format!("Sample favorites could not be loaded: {error:#}")],
            ),
        };
        let load_error = errors.first().cloned();
        let mut library = Self {
            directory,
            catalog,
            by_path: BTreeMap::new(),
            by_id: BTreeMap::new(),
            aliases: BTreeMap::new(),
            resolved: BTreeSet::new(),
            active: None,
            queued: None,
            revision: 0,
            errors,
            load_error,
        };
        library.index_entries();
        library
    }

    pub fn entries(&self) -> &[SampleLibraryEntry] {
        &self.catalog.entries
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.queued.is_some()
    }
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    pub fn favorite_for(&self, path: &Path, name: Option<&str>) -> Option<&SampleLibraryEntry> {
        if let Some(indices) = self.by_path.get(path)
            && let Some(entry) = indices
                .iter()
                .filter_map(|index| self.entries().get(*index))
                .find(|entry| name.is_none_or(|name| entry.name == name))
        {
            return Some(entry);
        }
        let key = (path.to_path_buf(), name.map(str::to_owned));
        let id = self.aliases.get(&key)?;
        self.by_id
            .get(id)
            .and_then(|index| self.entries().get(*index))
    }

    fn index_entries(&mut self) {
        self.by_path.clear();
        self.by_id.clear();
        for (index, entry) in self.catalog.entries.iter().enumerate() {
            self.by_path
                .entry(entry.path.clone())
                .or_default()
                .push(index);
            self.by_id.insert(entry.id.clone(), index);
        }
    }

    pub fn request_add(&mut self, source: SampleDrop, retain: bool, ctx: &egui::Context) {
        if self
            .favorite_for(&source.path, source.name.as_deref())
            .is_some()
        {
            return;
        }
        if self.busy() {
            self.errors
                .push("Wait for the current sample library operation to finish".into());
            return;
        }
        if self.entries().len() >= MAX_ENTRIES {
            self.errors.push(format!(
                "The favorites library holds at most {MAX_ENTRIES} samples"
            ));
            return;
        }
        self.start(
            Job {
                source,
                operation: Operation::Add { retain },
            },
            ctx,
        );
    }

    /// Resolve a selected project asset by content, never by its unpacked path.
    /// At most one worker and one latest selection exist; no directory is scanned.
    pub fn resolve(&mut self, source: SampleDrop, ctx: &egui::Context) {
        let key = (source.path.clone(), source.name.clone());
        if self.resolved.contains(&key)
            || self
                .favorite_for(&source.path, source.name.as_deref())
                .is_some()
            || !self
                .entries()
                .iter()
                .any(|entry| entry.content_hash.is_some())
            || self.active.as_ref().is_some_and(|active| {
                active.job.source == source && !active.cancel.load(Ordering::Acquire)
            })
        {
            return;
        }
        let job = Job {
            source,
            operation: Operation::Resolve,
        };
        if let Some(active) = &self.active {
            if matches!(active.job.operation, Operation::Resolve) {
                active.cancel.store(true, Ordering::Release);
            }
            self.queued = Some(job);
        } else {
            self.start(job, ctx);
        }
    }

    fn start(&mut self, job: Job, ctx: &egui::Context) {
        let (sender, result) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let source = job.source.path.clone();
        let directory = self.directory.clone();
        let retain = matches!(job.operation, Operation::Add { retain: true });
        let hash_only = matches!(job.operation, Operation::Resolve);
        let repaint = ctx.clone();
        match std::thread::Builder::new()
            .name("rill-sample-library".into())
            .spawn(move || {
                let prepared = prepare(&source, &directory, retain, hash_only, &worker_cancel);
                let _ = sender.send(prepared);
                repaint.request_repaint();
            }) {
            Ok(_) => {
                self.active = Some(Active {
                    job,
                    result,
                    cancel,
                })
            }
            Err(error) => self
                .errors
                .push(format!("Sample library worker could not start: {error}")),
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let result = self
            .active
            .as_ref()
            .and_then(|active| match active.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(anyhow::anyhow!("Sample library worker stopped")))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            let active = self.active.take().unwrap();
            if !active.cancel.load(Ordering::Acquire) {
                match result.and_then(|prepared| self.complete(&active.job, prepared)) {
                    Ok(()) => (),
                    Err(error) if matches!(active.job.operation, Operation::Resolve) => {
                        // Missing assets are reported by metadata/preview when used.
                        let _ = error;
                        self.resolved.insert((
                            active.job.source.path.clone(),
                            active.job.source.name.clone(),
                        ));
                    }
                    Err(error) => self
                        .errors
                        .push(format!("Sample favorite was not saved: {error:#}")),
                }
            }
        }
        if self.active.is_none()
            && let Some(job) = self.queued.take()
        {
            self.start(job, ctx);
        }
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn complete(&mut self, job: &Job, mut prepared: Prepared) -> Result<()> {
        let name = source_name(&job.source);
        let matching = self
            .entries()
            .iter()
            .find(|entry| {
                (entry.path == prepared.path
                    && job
                        .source
                        .name
                        .as_ref()
                        .is_none_or(|name| entry.name == *name))
                    || (prepared.hash.is_some()
                        && entry.content_hash == prepared.hash
                        && entry.name == name)
            })
            .map(|entry| entry.id.clone());
        let key = (job.source.path.clone(), job.source.name.clone());
        if matches!(job.operation, Operation::Resolve) {
            self.resolved.insert(key.clone());
            if let Some(id) = matching {
                self.aliases.insert(key, id);
                self.revision = self.revision.wrapping_add(1);
            }
            return Ok(());
        }
        let id = if let Some(id) = matching {
            id
        } else {
            let mut catalog = self.catalog.clone();
            let id = format!("sample-{}", daw_core::new_id());
            catalog.entries.push(SampleLibraryEntry {
                id: id.clone(),
                path: prepared.path,
                name,
                favorite: SampleFavorite::default(),
                content_hash: prepared.hash,
            });
            self.save(&catalog)?;
            self.catalog = catalog;
            self.index_entries();
            id
        };
        if let Some(asset) = &mut prepared.asset {
            asset.retained = self.entries().iter().any(|entry| entry.path == asset.path);
        }
        self.aliases.insert(key, id);
        self.resolved.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn update(&mut self, id: &str, favorite: SampleFavorite) -> Result<()> {
        favorite.validate()?;
        let mut catalog = self.catalog.clone();
        catalog
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .context("Sample favorite no longer exists")?
            .favorite = favorite;
        self.save(&catalog)?;
        self.catalog = catalog;
        self.index_entries();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        let mut catalog = self.catalog.clone();
        catalog.entries.retain(|entry| entry.id != id);
        self.save(&catalog)?;
        self.catalog = catalog;
        self.index_entries();
        self.aliases.retain(|_, value| value != id);
        self.resolved.clear();
        self.revision = self.revision.wrapping_add(1);
        // Removing a bookmark never deletes audio that another consumer may use.
        Ok(())
    }

    fn save(&self, catalog: &Catalog) -> Result<()> {
        ensure!(
            self.load_error.is_none(),
            "{}",
            self.load_error.as_deref().unwrap_or_default()
        );
        validate_catalog(catalog)?;
        let bytes = serde_json::to_vec_pretty(catalog)?;
        ensure!(
            bytes.len() as u64 <= MAX_CATALOG_BYTES,
            "Sample library exceeds its size limit"
        );
        fs::create_dir_all(&self.directory)?;
        let temporary = self
            .directory
            .join(format!(".sample-library-{}.tmp", daw_core::new_id()));
        let guard = OwnedFile {
            path: temporary.clone(),
            retained: false,
        };
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, self.directory.join("sample-library.json"))?;
        drop(guard);
        Ok(())
    }
}

fn source_name(source: &SampleDrop) -> String {
    source.name.clone().unwrap_or_else(|| {
        source
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    })
}

fn validate_catalog(catalog: &Catalog) -> Result<()> {
    ensure!(catalog.version == 1, "Unsupported sample library version");
    ensure!(
        catalog.entries.len() <= MAX_ENTRIES,
        "Too many sample favorites"
    );
    let mut ids = BTreeSet::new();
    for entry in &catalog.entries {
        ensure!(
            !entry.id.is_empty() && entry.id.len() <= 128 && ids.insert(&entry.id),
            "Invalid sample favorite identity"
        );
        ensure!(
            !entry.path.as_os_str().is_empty() && entry.path.is_absolute(),
            "Sample favorite path must be absolute"
        );
        ensure!(
            !entry.name.trim().is_empty() && entry.name.len() <= 1024 && !entry.name.contains('\0'),
            "Invalid sample favorite name"
        );
        ensure!(
            entry.content_hash.as_ref().is_none_or(
                |hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            ),
            "Invalid sample fingerprint"
        );
        entry.favorite.validate()?;
    }
    Ok(())
}

fn prepare(
    source: &Path,
    directory: &Path,
    retain: bool,
    hash_only: bool,
    cancel: &AtomicBool,
) -> Result<Prepared> {
    ensure!(
        !cancel.load(Ordering::Acquire),
        "Sample library operation canceled"
    );
    let source = source
        .canonicalize()
        .context("Sample file is unavailable")?;
    let metadata = source.metadata()?;
    ensure!(metadata.is_file(), "Choose a sample file");
    ensure!(
        metadata.len() <= daw_core::MAX_IMPORTED_AUDIO_BYTES as u64,
        "Sample exceeds the library file limit"
    );
    if !retain && !hash_only {
        return Ok(Prepared {
            path: source,
            hash: None,
            asset: None,
        });
    }
    let vault = directory.join("Sample Library").join("audio");
    let mut temporary = None;
    let mut output = if retain {
        fs::create_dir_all(&vault)?;
        let path = vault.join(format!(".retain-{}.tmp", daw_core::new_id()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        temporary = Some(OwnedFile {
            path,
            retained: false,
        });
        Some(file)
    } else {
        None
    };
    let mut input = File::open(&source)?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        ensure!(
            !cancel.load(Ordering::Acquire),
            "Sample library operation canceled"
        );
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        ensure!(
            bytes <= daw_core::MAX_IMPORTED_AUDIO_BYTES as u64,
            "Sample grew beyond the library file limit"
        );
        hash.update(&buffer[..count]);
        if let Some(output) = &mut output {
            output.write_all(&buffer[..count])?;
        }
    }
    ensure!(
        bytes == metadata.len() && input.metadata()?.modified().ok() == metadata.modified().ok(),
        "Sample changed while being retained"
    );
    let hash = format!("{:x}", hash.finalize());
    if let Some(output) = output {
        output.sync_all()?;
    }
    let Some(temporary) = temporary else {
        return Ok(Prepared {
            path: source,
            hash: Some(hash),
            asset: None,
        });
    };
    ensure!(
        !cancel.load(Ordering::Acquire),
        "Sample library operation canceled"
    );
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            extension.len() <= 12 && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .unwrap_or("audio")
        .to_ascii_lowercase();
    let destination = vault.join(format!("{hash}.{extension}"));
    let asset = match fs::hard_link(&temporary.path, &destination) {
        Ok(()) => Some(OwnedFile {
            path: destination.clone(),
            retained: false,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = prepare(&destination, directory, false, true, cancel)?;
            ensure!(
                existing.hash.as_ref() == Some(&hash),
                "Retained sample failed its content check"
            );
            None
        }
        Err(error) => return Err(error).context("Retaining sample in the library"),
    };
    drop(temporary);
    Ok(Prepared {
        path: destination,
        hash: Some(hash),
        asset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn drain(library: &mut SampleLibrary, ctx: &egui::Context) {
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while library.busy() {
            assert!(std::time::Instant::now() < until);
            library.poll(ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(library.take_errors().is_empty());
    }
    #[test]
    fn retained_favorites_survive_source_removal_and_resolve_new_archive_paths() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("unpacked.wav");
        fs::write(&source, b"immutable project source").unwrap();
        let ctx = egui::Context::default();
        let root = directory.path().join("config");
        let mut library = SampleLibrary::load(root.clone());
        library.request_add(
            SampleDrop {
                path: source.clone(),
                name: Some("My kick".into()),
            },
            true,
            &ctx,
        );
        drain(&mut library, &ctx);
        let first = library.entries()[0].clone();
        fs::remove_file(&source).unwrap();
        assert_eq!(fs::read(&first.path).unwrap(), b"immutable project source");
        let second = directory.path().join("reopened.wav");
        fs::write(&second, b"immutable project source").unwrap();
        let mut library = SampleLibrary::load(root);
        library.resolve(
            SampleDrop {
                path: second.clone(),
                name: Some("My kick".into()),
            },
            &ctx,
        );
        drain(&mut library, &ctx);
        assert_eq!(
            library.favorite_for(&second, Some("My kick")).unwrap().id,
            first.id
        );
        assert!(library.favorite_for(&second, Some("Other alias")).is_none());
        library.remove(&first.id).unwrap();
        assert!(
            first.path.exists(),
            "Removing a bookmark must preserve audio"
        );
    }
    #[test]
    fn referenced_favorites_persist_tags_without_copying_or_modifying_sources() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("external.flac");
        fs::write(&source, b"external recording").unwrap();
        let ctx = egui::Context::default();
        let root = directory.path().join("config");
        let mut library = SampleLibrary::load(root.clone());
        library.request_add(source.clone().into(), false, &ctx);
        drain(&mut library, &ctx);
        let entry = library.entries()[0].clone();
        let mut favorite = entry.favorite;
        favorite.tag = "Percussion".into();
        favorite.color = Some([200, 80, 40]);
        library.update(&entry.id, favorite.clone()).unwrap();
        let library = SampleLibrary::load(root);
        assert_eq!(library.entries()[0].favorite, favorite);
        assert_eq!(library.entries()[0].path, source.canonicalize().unwrap());
        assert_eq!(fs::read(source).unwrap(), b"external recording");
    }

    #[test]
    fn returning_to_a_canceled_selection_resolves_the_latest_source() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.wav");
        let first = directory.path().join("reopened-a.wav");
        let second = directory.path().join("reopened-b.wav");
        for path in [&original, &first, &second] {
            fs::write(path, b"same retained recording").unwrap();
        }
        let ctx = egui::Context::default();
        let mut library = SampleLibrary::load(directory.path().join("config"));
        library.request_add(
            SampleDrop {
                path: original,
                name: Some("Kick".into()),
            },
            true,
            &ctx,
        );
        drain(&mut library, &ctx);
        let expected = library.entries()[0].id.clone();
        let first_source = SampleDrop {
            path: first.clone(),
            name: Some("Kick".into()),
        };
        library.resolve(first_source.clone(), &ctx);
        library.resolve(
            SampleDrop {
                path: second.clone(),
                name: Some("Kick".into()),
            },
            &ctx,
        );
        // Do not poll between selections: the first result may already exist,
        // but its canceled request must not suppress the latest selection.
        library.resolve(first_source, &ctx);
        drain(&mut library, &ctx);
        assert_eq!(
            library.favorite_for(&first, Some("Kick")).unwrap().id,
            expected
        );
        assert!(library.favorite_for(&second, Some("Kick")).is_none());
        assert_eq!(library.entries().len(), 1);
    }

    #[test]
    fn a_new_favorite_invalidates_previous_unmatched_content_resolutions() {
        let directory = tempfile::tempdir().unwrap();
        let unrelated = directory.path().join("unrelated.wav");
        let first = directory.path().join("copy-a.wav");
        let second = directory.path().join("copy-b.wav");
        fs::write(&unrelated, b"another recording").unwrap();
        fs::write(&first, b"same new recording").unwrap();
        fs::write(&second, b"same new recording").unwrap();
        let ctx = egui::Context::default();
        let mut library = SampleLibrary::load(directory.path().join("config"));
        library.request_add(unrelated.into(), true, &ctx);
        drain(&mut library, &ctx);

        let first_source = SampleDrop {
            path: first.clone(),
            name: Some("Snare".into()),
        };
        library.resolve(first_source.clone(), &ctx);
        drain(&mut library, &ctx);
        assert!(library.favorite_for(&first, Some("Snare")).is_none());
        assert!(
            library
                .resolved
                .contains(&(first.clone(), Some("Snare".into())))
        );

        library.request_add(
            SampleDrop {
                path: second.clone(),
                name: Some("Snare".into()),
            },
            true,
            &ctx,
        );
        drain(&mut library, &ctx);
        let expected = library
            .favorite_for(&second, Some("Snare"))
            .unwrap()
            .id
            .clone();
        library.resolve(first_source, &ctx);
        drain(&mut library, &ctx);
        assert_eq!(
            library.favorite_for(&first, Some("Snare")).unwrap().id,
            expected
        );
        assert_eq!(
            library.entries().len(),
            2,
            "Resolving a copy never duplicates a bookmark"
        );
    }
}
