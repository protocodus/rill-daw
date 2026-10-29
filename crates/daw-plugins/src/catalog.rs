//! Persistent editor thumbnails. Discovery and fingerprinting never load native
//! plug-in code; callers supply the isolated editor capture operation.

use crate::PluginInfo;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{BufReader, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const CACHE_VERSION: u32 = 1;
const INDEX_VERSION: u32 = 1;
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PNG_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThumbnailStatus {
    Missing,
    Ready,
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub plugin: PluginInfo,
    pub fingerprint: String,
    pub thumbnail: Option<PathBuf>,
    pub thumbnail_size: Option<[u32; 2]>,
    pub status: ThumbnailStatus,
    /// When a catalog sync first indexed this plug-in; 0 until it has.
    #[serde(default)]
    pub added_unix_ms: u64,
}

#[derive(Clone, Debug)]
pub struct PluginCatalog {
    root: PathBuf,
}

impl Default for PluginCatalog {
    fn default() -> Self {
        Self::new(cache_dir())
    }
}

/// Cache root override is independent of session settings and scratch storage.
/// `RILL_CACHE_DIR` selects the complete Rill cache directory, useful for tests.
pub fn cache_dir() -> PathBuf {
    if let Some(path) = nonempty_env("RILL_CACHE_DIR") {
        return path;
    }
    let home = nonempty_env("HOME")
        .or_else(|| nonempty_env("USERPROFILE"))
        .unwrap_or_else(std::env::temp_dir);
    if cfg!(target_os = "macos") {
        home.join("Library/Caches/Rill")
    } else if cfg!(target_os = "windows") {
        nonempty_env("LOCALAPPDATA")
            .unwrap_or(home)
            .join("Rill/Cache")
    } else {
        nonempty_env("XDG_CACHE_HOME")
            .unwrap_or_else(|| home.join(".cache"))
            .join("rill")
    }
}

fn nonempty_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    plugin: PluginInfo,
    fingerprint: String,
    status: ThumbnailStatus,
    size: Option<[u32; 2]>,
    png_sha256: Option<String>,
    updated_unix_ms: u128,
}

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    synced_unix_ms: u64,
    entries: Vec<CatalogEntry>,
}

impl PluginCatalog {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Read discovery metadata and cached results only. This does not create
    /// directories, launch a helper, or load native plug-in code.
    pub fn scan(&self, paths: &[PathBuf]) -> Result<Vec<CatalogEntry>> {
        crate::scan_paths(paths)?
            .iter()
            .map(|plugin| self.lookup(plugin))
            .collect()
    }

    pub fn lookup(&self, plugin: &PluginInfo) -> Result<CatalogEntry> {
        let fingerprint = fingerprint(plugin)?;
        let mut entry = CatalogEntry {
            plugin: plugin.clone(),
            fingerprint,
            thumbnail: None,
            thumbnail_size: None,
            status: ThumbnailStatus::Missing,
            added_unix_ms: 0,
        };
        let record_path = self.path(&entry.fingerprint, "json");
        let record = fs::metadata(&record_path)
            .ok()
            .filter(|metadata| metadata.len() <= 1024 * 1024)
            .and_then(|_| fs::read(record_path).ok())
            .and_then(|bytes| serde_json::from_slice::<Record>(&bytes).ok());
        let Some(record) = record.filter(|record| {
            record.version == CACHE_VERSION
                && record.fingerprint == entry.fingerprint
                && record.plugin.class_id == plugin.class_id
        }) else {
            return Ok(entry);
        };
        entry.status = record.status;
        if entry.status == ThumbnailStatus::Ready {
            let path = self.path(&entry.fingerprint, "png");
            // A digest detects truncated/replaced cache images without decoding
            // every thumbnail during each directory scan.
            let digest = read_png_bytes(&path).ok().map(|bytes| hash(&bytes));
            if record.size.is_some() && digest.is_some() && digest == record.png_sha256 {
                entry.thumbnail = Some(path);
                entry.thumbnail_size = record.size;
            } else {
                entry.status = ThumbnailStatus::Missing;
            }
        }
        Ok(entry)
    }

    /// Run an explicit isolated native-editor capture and atomically publish a
    /// validated PNG. The callback owns helper timeouts/crash isolation and must
    /// return only after it has finished writing `output`. Capture failures are
    /// persistent entries, so scanning does not repeatedly launch a broken plug-in.
    /// Filesystem/cache failures return `Err`; plug-in failures return `Failed`.
    pub fn capture(
        &self,
        plugin: &PluginInfo,
        capture: impl FnOnce(&PluginInfo, &Path) -> Result<()>,
    ) -> Result<CatalogEntry> {
        let fingerprint = fingerprint(plugin)?;
        let directory = self.directory();
        fs::create_dir_all(&directory)
            .with_context(|| format!("Creating plug-in cache {}", directory.display()))?;
        let temporary = TemporaryFile(temporary_path(&directory, "capture", "png"));
        let result = capture(plugin, &temporary.0).and_then(|()| {
            let size = validate_png(&temporary.0)?;
            let bytes = read_png_bytes(&temporary.0)?;
            Ok((size, hash(&bytes)))
        });
        ensure!(
            fingerprint == self::fingerprint(plugin)?,
            "Plug-in changed during editor capture; rescan before retrying"
        );
        let (status, size, png_sha256) = match result {
            Ok((size, digest)) => {
                // Commit the image first: a Ready record can never reference an
                // image whose write is still in progress.
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&temporary.0)?
                    .sync_all()?;
                fs::rename(&temporary.0, self.path(&fingerprint, "png"))?;
                (ThumbnailStatus::Ready, Some(size), Some(digest))
            }
            Err(error) => (
                ThumbnailStatus::Failed {
                    message: format!("{error:#}").chars().take(8192).collect(),
                },
                None,
                None,
            ),
        };
        let record = Record {
            version: CACHE_VERSION,
            plugin: plugin.clone(),
            fingerprint: fingerprint.clone(),
            status,
            size,
            png_sha256,
            updated_unix_ms: now().as_millis(),
        };
        atomic_write(
            &self.path(&fingerprint, "json"),
            &serde_json::to_vec_pretty(&record)?,
        )?;
        self.lookup(plugin)
    }

    /// The last synchronized catalog and its scan time, shown before a new
    /// scan finishes. Previews are only referenced by their fingerprinted cache
    /// path; the index can never point the library at another file.
    pub fn load_index(&self) -> Option<(Vec<CatalogEntry>, u64)> {
        let path = self.index_path();
        let bytes = fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.len() <= MAX_INDEX_BYTES)
            .and_then(|_| fs::read(path).ok())?;
        let index = serde_json::from_slice::<Index>(&bytes)
            .ok()
            .filter(|index| index.version == INDEX_VERSION)?;
        let entries = index
            .entries
            .into_iter()
            .filter(|entry| {
                entry.fingerprint.len() == 64
                    && entry
                        .fingerprint
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
            })
            .map(|mut entry| {
                entry.thumbnail = (entry.status == ThumbnailStatus::Ready)
                    .then(|| self.path(&entry.fingerprint, "png"));
                entry
            })
            .collect();
        Some((entries, index.synced_unix_ms))
    }

    pub fn save_index(&self, entries: &[CatalogEntry], synced_unix_ms: u64) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("Creating plug-in cache {}", self.root.display()))?;
        atomic_write(
            &self.index_path(),
            &serde_json::to_vec(&Index {
                version: INDEX_VERSION,
                synced_unix_ms,
                entries: entries.to_vec(),
            })?,
        )
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("plugin-index-v1.json")
    }

    fn directory(&self) -> PathBuf {
        self.root.join("plugin-thumbnails-v1")
    }

    fn path(&self, fingerprint: &str, extension: &str) -> PathBuf {
        self.directory().join(format!("{fingerprint}.{extension}"))
    }
}

/// Fingerprint canonical identity, processor class, metadata and all bundle file
/// sizes/modification times. Module metadata content is also hashed, including
/// version fields even when an installer preserves its filesystem timestamp.
/// Symlink targets are tracked and directory cycles are not followed twice.
pub fn fingerprint(plugin: &PluginInfo) -> Result<String> {
    let canonical = plugin
        .path
        .canonicalize()
        .with_context(|| format!("Fingerprinting {}", plugin.path.display()))?;
    let mut digest = Sha256::new();
    digest.update(b"rill-plugin-thumbnail-v1\0");
    digest.update(canonical.as_os_str().as_encoded_bytes());
    digest.update(serde_json::to_vec(&(
        &plugin.class_id,
        &plugin.name,
        &plugin.vendor,
        &plugin.category,
    ))?);
    let mut pending = vec![canonical.clone()];
    let mut visited = HashSet::new();
    while let Some(path) = pending.pop() {
        let link = fs::symlink_metadata(&path)?;
        let resolved = path.canonicalize()?;
        let metadata = fs::metadata(&path)?;
        digest.update(
            path.strip_prefix(&canonical)
                .unwrap_or(&path)
                .as_os_str()
                .as_encoded_bytes(),
        );
        digest.update([0]);
        digest.update(resolved.as_os_str().as_encoded_bytes());
        digest.update(metadata.len().to_le_bytes());
        stamp(&mut digest, &link);
        stamp(&mut digest, &metadata);
        if metadata.is_dir() {
            if visited.insert(resolved) {
                let mut children = fs::read_dir(&path)?
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<std::io::Result<Vec<_>>>()?;
                children.sort();
                pending.extend(children.into_iter().rev());
            }
        } else if path
            .file_name()
            .is_some_and(|name| name == "moduleinfo.json")
        {
            ensure!(
                metadata.len() <= 16 * 1024 * 1024,
                "Plug-in metadata exceeds 16 MiB"
            );
            digest.update(fs::read(path)?);
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn stamp(digest: &mut Sha256, metadata: &fs::Metadata) {
    if let Ok(modified) = metadata.modified() {
        match modified.duration_since(UNIX_EPOCH) {
            Ok(value) => {
                digest.update([1]);
                digest.update(value.as_nanos().to_le_bytes());
            }
            Err(value) => {
                digest.update([0]);
                digest.update(value.duration().as_nanos().to_le_bytes());
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        digest.update(metadata.ctime().to_le_bytes());
        digest.update(metadata.ctime_nsec().to_le_bytes());
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_png_bytes(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path).context("Editor capture did not produce a PNG")?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_PNG_BYTES,
        "Editor PNG exceeds 64 MiB"
    );
    Ok(fs::read(path)?)
}

fn validate_png(path: &Path) -> Result<[u32; 2]> {
    let bytes = read_png_bytes(path)?;
    let mut decoder = png::Decoder::new(BufReader::new(std::io::Cursor::new(bytes)));
    decoder.set_limits(png::Limits {
        bytes: MAX_PNG_BYTES as usize,
    });
    let mut reader = decoder
        .read_info()
        .context("Editor capture is not a valid PNG")?;
    let info = reader.info();
    ensure!(
        info.width > 0
            && info.height > 0
            && info.width <= 8192
            && info.height <= 8192
            && u64::from(info.width) * u64::from(info.height) <= MAX_PIXELS,
        "Editor image dimensions exceed the supported limit"
    );
    let size = [info.width, info.height];
    let buffer_size = reader
        .output_buffer_size()
        .context("PNG image size overflow")?;
    ensure!(
        buffer_size <= MAX_PNG_BYTES as usize,
        "Decoded editor image exceeds 64 MiB"
    );
    reader
        .next_frame(&mut vec![0; buffer_size])
        .context("Editor PNG is incomplete or corrupt")?;
    reader
        .finish()
        .context("Editor PNG is incomplete or corrupt")?;
    Ok(size)
}

fn now() -> std::time::Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn temporary_path(directory: &Path, prefix: &str, extension: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    directory.join(format!(
        ".{prefix}.{}.{}.{}.{}",
        std::process::id(),
        now().as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        extension
    ))
}

struct TemporaryFile(PathBuf);
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().context("Cache file has no directory")?;
    let temporary = TemporaryFile(temporary_path(directory, "record", "tmp"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary.0)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary.0, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(root: &Path) -> PluginInfo {
        let path = root.join("unsafe .. plug-in !.vst3");
        fs::create_dir_all(path.join("Contents/Resources")).unwrap();
        fs::write(path.join("Contents/processor"), b"native fixture").unwrap();
        fs::write(
            path.join("Contents/Resources/moduleinfo.json"),
            b"{\"Version\":\"1\"}",
        )
        .unwrap();
        PluginInfo {
            name: "../../Unsafe name".into(),
            path,
            vendor: "Vendor".into(),
            category: "Instrument|Synth".into(),
            class_id: Some("ABC".into()),
            ..PluginInfo::default()
        }
    }

    fn png(path: &Path) -> Result<()> {
        let mut encoder = png::Encoder::new(fs::File::create(path)?, 2, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()?
            .write_image_data(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255])?;
        Ok(())
    }

    #[test]
    fn capture_persists_validated_png_and_uses_safe_filenames() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let info = plugin(temp.path());
        let cache = PluginCatalog::new(temp.path().join("cache"));
        assert_eq!(cache.lookup(&info)?.status, ThumbnailStatus::Missing);
        assert!(
            !cache.root().exists(),
            "lookup must not write cache directories"
        );
        let entry = cache.capture(&info, |_, path| png(path))?;
        assert_eq!(entry.status, ThumbnailStatus::Ready);
        assert_eq!(entry.thumbnail_size, Some([2, 2]));
        let path = entry.thumbnail.as_ref().unwrap();
        assert!(path.starts_with(cache.root()));
        assert_eq!(path.file_stem().unwrap().to_string_lossy().len(), 64);
        assert!(
            path.file_stem()
                .unwrap()
                .to_string_lossy()
                .chars()
                .all(|c| c.is_ascii_hexdigit())
        );
        assert_eq!(PluginCatalog::new(cache.root()).lookup(&info)?, entry);
        assert!(fs::read_dir(cache.directory())?.all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.')
        }));
        Ok(())
    }

    #[test]
    fn updates_and_processor_classes_invalidate_independently() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let info = plugin(temp.path());
        let cache = PluginCatalog::new(temp.path().join("cache"));
        let original = cache.capture(&info, |_, path| png(path))?;
        let mut other = info.clone();
        other.class_id = Some("DEF".into());
        assert_ne!(fingerprint(&other)?, original.fingerprint);
        assert_eq!(cache.lookup(&other)?.status, ThumbnailStatus::Missing);
        let processor = info.path.join("Contents/processor");
        let original_contents = fs::read(&processor)?;
        // Windows requires a writable handle to change file timestamps. Opening
        // without create/truncate preserves the mtime-only invalidation fixture.
        OpenOptions::new().write(true).open(&processor)?.set_times(
            fs::FileTimes::new().set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1234)),
        )?;
        assert_eq!(fs::read(&processor)?, original_contents);
        assert_ne!(
            fingerprint(&info)?,
            original.fingerprint,
            "mtime-only updates must invalidate"
        );
        let original_binary = fingerprint(&info)?;
        fs::write(
            info.path.join("Contents/processor"),
            b"updated native fixture",
        )?;
        assert_ne!(fingerprint(&info)?, original_binary);
        assert_eq!(cache.lookup(&info)?.status, ThumbnailStatus::Missing);
        let binary = fingerprint(&info)?;
        fs::write(
            info.path.join("Contents/Resources/moduleinfo.json"),
            b"{\"Version\":\"2\"}",
        )?;
        assert_ne!(fingerprint(&info)?, binary);
        Ok(())
    }

    #[test]
    fn scan_retains_instruments_effects_and_unknown_metadata_without_initializing() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let info = plugin(temp.path());
        let metadata = serde_json::json!({
            "Name": "Bundle", "Version": "1.0", "Factory Info": { "Vendor": "Acme" },
            "Classes": [
                {"CID":"0123456789ABCDEF0123456789ABCDEF", "Category":"Audio Module Class", "Name":"Synth", "Sub Categories":["Instrument","Synth"], "Version":"1.0", "SDKVersion":"VST 3.7.9", "Cardinality":2147483647},
                {"CID":"1123456789ABCDEF0123456789ABCDEF", "Category":"Audio Module Class", "Name":"Effect", "Sub Categories":["Fx"], "Version":"1.0", "SDKVersion":"VST 3.7.9", "Cardinality":2147483647}
            ]
        });
        fs::write(
            info.path.join("Contents/Resources/moduleinfo.json"),
            metadata.to_string(),
        )?;
        fs::create_dir(temp.path().join("Unknown.vst3"))?;
        let cache = PluginCatalog::new(temp.path().join("cache"));
        let entries = cache.scan(&[temp.path().to_path_buf()])?;
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.plugin.is_instrument() == Some(true))
                .count(),
            1
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.plugin.is_effect() == Some(true))
                .count(),
            1
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.plugin.is_instrument().is_none())
                .count(),
            1
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.status == ThumbnailStatus::Missing)
        );
        assert!(
            !cache.root().exists(),
            "metadata scan must not initialize a plug-in or create a preview"
        );
        Ok(())
    }

    #[test]
    fn errors_are_persistent_retryable_and_never_publish_fake_images() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let info = plugin(temp.path());
        let cache = PluginCatalog::new(temp.path().join("cache"));
        let failed = cache.capture(&info, |_, path| {
            fs::write(path, b"partial")?;
            anyhow::bail!("editor crashed")
        })?;
        assert_eq!(
            failed.status,
            ThumbnailStatus::Failed {
                message: "editor crashed".into()
            }
        );
        assert!(failed.thumbnail.is_none());
        assert_eq!(cache.lookup(&info)?, failed);
        let invalid = cache.capture(&info, |_, path| {
            fs::write(path, b"not a PNG")?;
            Ok(())
        })?;
        assert!(matches!(invalid.status, ThumbnailStatus::Failed { .. }));
        assert!(invalid.thumbnail.is_none());
        assert_eq!(
            cache.capture(&info, |_, path| png(path))?.status,
            ThumbnailStatus::Ready
        );
        Ok(())
    }

    #[test]
    fn corrupt_records_images_and_updates_during_capture_are_not_reused() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let info = plugin(temp.path());
        let cache = PluginCatalog::new(temp.path().join("cache"));
        let entry = cache.capture(&info, |_, path| png(path))?;
        fs::write(entry.thumbnail.unwrap(), b"truncated")?;
        assert_eq!(cache.lookup(&info)?.status, ThumbnailStatus::Missing);
        fs::write(cache.path(&entry.fingerprint, "json"), b"partial JSON")?;
        assert_eq!(cache.lookup(&info)?.status, ThumbnailStatus::Missing);
        let result = cache.capture(&info, |info, path| {
            png(path)?;
            fs::write(
                info.path.join("Contents/processor"),
                b"replaced while running",
            )?;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(cache.lookup(&info)?.status, ThumbnailStatus::Missing);
        Ok(())
    }
}
