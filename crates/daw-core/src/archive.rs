//! Portable, bounded project bundles. Never extracts archive-provided paths directly.
use crate::{ClipContent, Effect, Instrument, Project, model::ProjectFile};
use anyhow::{Context, Result, ensure};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const MAX_ENTRIES: usize = 4096;
const MAX_METADATA: u64 = 4 * 1024 * 1024;
const MAX_MANIFEST: u64 = 256 * 1024 * 1024;
// Media currently uses RIFF WAV, whose data/container sizes are 32-bit. Leave
// room for headers rather than accepting a file the audio writer cannot create.
const MAX_ASSET: u64 = u32::MAX as u64 - 128;
const MAX_TOTAL: u64 = 64 * 1024 * 1024 * 1024;
const MANIFEST: &str = "manifest.json";
const COPY_BUFFER: usize = 64 * 1024;

pub(crate) fn is_bundle(signature: &[u8]) -> bool {
    matches!(signature, b"PK\x03\x04" | b"PK\x05\x06" | b"PK\x07\x08")
}

struct Asset {
    path: PathBuf,
    name: String,
    metadata: Metadata,
}

fn check_cancelled(cancel: Option<&AtomicBool>) -> Result<()> {
    ensure!(
        !cancel.is_some_and(|flag| flag.load(Ordering::Acquire)),
        "Project operation cancelled"
    );
    Ok(())
}

fn unchanged(before: &Metadata, after: &Metadata) -> bool {
    let same = before.is_file()
        && after.is_file()
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && before.created().ok() == after.created().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same && before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        same
    }
}

fn same_asset(left: &Asset, path: &Path, metadata: &Metadata) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Ok(left.metadata.dev() == metadata.dev() && left.metadata.ino() == metadata.ino())
    }
    #[cfg(not(unix))]
    {
        Ok(left.metadata.len() == metadata.len() && same_file::is_same_file(&left.path, path)?)
    }
}

/// Read through EOF so ZIP CRC checks run. Never allocate or trust a declared
/// media length, and never write even one byte beyond the admission budget.
fn copy_exact(
    input: &mut impl Read,
    output: &mut impl Write,
    expected: u64,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    let mut buffer = [0; COPY_BUFFER];
    let mut copied = 0_u64;
    loop {
        check_cancelled(cancel)?;
        let count = (expected.saturating_sub(copied) + 1).min(COPY_BUFFER as u64) as usize;
        let read = match input.read(&mut buffer[..count]) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .context("Audio size overflow")?;
        ensure!(
            copied <= expected,
            "Audio size differs from its declared length"
        );
        output.write_all(&buffer[..read])?;
    }
    ensure!(
        copied == expected,
        "Audio size differs from its declared length"
    );
    Ok(())
}

fn copy_asset(asset: &Asset, output: &mut impl Write, cancel: Option<&AtomicBool>) -> Result<()> {
    check_cancelled(cancel)?;
    let mut input = File::open(&asset.path)?;
    let before = input.metadata()?;
    ensure!(
        unchanged(&asset.metadata, &before),
        "Audio asset changed while saving: {}",
        asset.path.display()
    );
    let identity = same_file::Handle::from_file(input.try_clone()?)?;
    copy_exact(&mut input, output, before.len(), cancel)?;
    ensure!(
        unchanged(&before, &input.metadata()?)
            && same_file::Handle::from_path(&asset.path)? == identity,
        "Audio asset changed while saving: {}",
        asset.path.display()
    );
    Ok(())
}

fn asset_name(name: &str) -> bool {
    name.strip_prefix("assets/")
        .and_then(|name| name.strip_suffix(".wav"))
        .is_some_and(|stem| stem.len() == 8 && stem.bytes().all(|byte| byte.is_ascii_digit()))
}

fn absolute_plugin(path: &mut String, base: &Path) {
    if Path::new(path).is_relative() {
        *path = base.join(&*path).to_string_lossy().into_owned();
    }
}

pub(crate) fn save(project: &Project, path: &Path, cancel: Option<&AtomicBool>) -> Result<()> {
    check_cancelled(cancel)?;
    let mut portable = project.clone();
    let base = std::env::current_dir()?;
    let mut names: HashMap<PathBuf, String> = HashMap::new();
    let mut assets: Vec<Asset> = Vec::new();
    let mut total = 0u64;
    for track in portable.channels_mut() {
        if let Instrument::Vst3 { path, .. } = &mut track.instrument {
            absolute_plugin(path, &base);
        }
        for slot in &mut track.inserts {
            let Effect::Vst3 { path, .. } = &mut slot.effect;
            absolute_plugin(path, &base);
        }
        for clip in &mut track.clips {
            check_cancelled(cancel)?;
            let ClipContent::Audio { path, .. } = &mut clip.content else {
                continue;
            };
            let source =
                fs::canonicalize(&*path).with_context(|| format!("Collect audio asset {path}"))?;
            if let Some(name) = names.get(&source) {
                *path = name.clone();
                continue;
            }
            let metadata = source.metadata()?;
            ensure!(metadata.is_file(), "Audio asset is not a regular file");
            if let Some(asset) = assets
                .iter()
                .find_map(|asset| match same_asset(asset, &source, &metadata) {
                    Ok(true) => Some(Ok(asset)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                })
                .transpose()?
            {
                *path = asset.name.clone();
                names.insert(source, asset.name.clone());
                continue;
            }
            ensure!(
                assets.len() + 1 < MAX_ENTRIES,
                "Project has too many bundled audio assets"
            );
            ensure!(
                metadata.len() <= MAX_ASSET,
                "Audio asset exceeds the RIFF WAV size limit"
            );
            total = total
                .checked_add(metadata.len())
                .context("Asset size overflow")?;
            ensure!(total <= MAX_TOTAL, "Project assets exceed 64 GiB");
            let name = format!("assets/{:08}.wav", assets.len());
            names.insert(source.clone(), name.clone());
            assets.push(Asset {
                path: source,
                name: name.clone(),
                metadata,
            });
            *path = name;
        }
    }
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".rill-save-")
        .tempfile_in(parent)?;
    {
        let mut archive = ZipWriter::new(temporary.as_file_mut());
        archive.start_file(
            MANIFEST,
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )?;
        let mut writer = LimitedWriter {
            inner: &mut archive,
            written: 0,
            limit: MAX_MANIFEST,
            cancel,
        };
        serde_json::to_writer(
            &mut writer,
            &ProjectFile {
                version: 6,
                project: portable,
            },
        )?;
        total = total
            .checked_add(writer.written)
            .context("Project size overflow")?;
        ensure!(total <= MAX_TOTAL, "Expanded project exceeds 64 GiB");
        for asset in &assets {
            check_cancelled(cancel)?;
            archive.start_file(
                &asset.name,
                SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Stored)
                    .large_file(true),
            )?;
            copy_asset(asset, &mut archive, cancel)?;
        }
        archive.finish()?;
    }
    ensure!(
        temporary.as_file().metadata()?.len() <= MAX_TOTAL,
        "Project archive exceeds 64 GiB"
    );
    for asset in assets {
        check_cancelled(cancel)?;
        ensure!(
            unchanged(&asset.metadata, &asset.path.metadata()?),
            "Audio asset changed while saving: {}",
            asset.path.display()
        );
    }
    temporary.as_file().sync_all()?;
    check_cancelled(cancel)?;
    temporary
        .persist(path)
        .context("Atomically replace project bundle")?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub(crate) fn load(
    mut input: File,
    scratch: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<Project> {
    check_cancelled(cancel)?;
    input
        .seek(SeekFrom::Start(0))
        .context("Project ZIP must be seekable")?;
    ensure!(
        input.metadata()?.len() <= MAX_TOTAL,
        "Project archive exceeds 64 GiB"
    );
    // Bound metadata parsing before trusting the entry count or directory sizes.
    // The ZIP parser cannot allocate based on arbitrarily large names/directories.
    let quota = Rc::new(Cell::new(Some(MAX_METADATA)));
    let reader = MetadataReader {
        inner: input,
        quota: Rc::clone(&quota),
    };
    let mut archive = ZipArchive::with_config(
        zip::read::Config {
            archive_offset: zip::read::ArchiveOffset::Known(0),
        },
        reader,
    )
    .context("Read project ZIP")?;
    archive
        .validate_header_consistency()
        .context("Validate project ZIP headers")?;
    quota.set(None);
    ensure!(
        !archive.is_empty() && archive.len() <= MAX_ENTRIES,
        "Invalid project entry count"
    );
    let mut names = HashSet::new();
    let mut total = 0u64;
    let mut manifest_index = None;
    for index in 0..archive.len() {
        check_cancelled(cancel)?;
        let file = archive.by_index(index)?;
        let name =
            std::str::from_utf8(file.name_raw()).context("Project entry name must be UTF-8")?;
        ensure!(
            name == MANIFEST || asset_name(name),
            "Unsafe or unsupported project entry: {name}"
        );
        ensure!(
            names.insert(name.to_owned()),
            "Duplicate project entry: {name}"
        );
        ensure!(
            !file.encrypted(),
            "Encrypted project entries are unsupported"
        );
        ensure!(
            file.unix_mode()
                .is_none_or(|mode| mode & 0o170000 == 0 || mode & 0o170000 == 0o100000),
            "Project entries must be regular files"
        );
        ensure!(
            matches!(
                file.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            ),
            "Unsupported project compression"
        );
        let limit = if name == MANIFEST {
            MAX_MANIFEST
        } else {
            MAX_ASSET
        };
        ensure!(
            file.size() <= limit,
            "Project entry exceeds its expanded size limit"
        );
        total = total
            .checked_add(file.size())
            .context("Expanded size overflow")?;
        ensure!(total <= MAX_TOTAL, "Expanded project exceeds 64 GiB");
        if name == MANIFEST {
            manifest_index = Some(index);
        }
    }
    let manifest_index = manifest_index.context("Project bundle has no manifest.json")?;
    let manifest = archive.by_index(manifest_index)?;
    let declared_manifest = manifest.size();
    let mut manifest = ManifestReader {
        inner: manifest.take(declared_manifest + 1),
        read: 0,
        cancel,
    };
    let envelope: ProjectFile = serde_json::from_reader(io::BufReader::new(&mut manifest))
        .context("Read project manifest")?;
    ensure!(
        manifest.read == declared_manifest && manifest.read <= MAX_MANIFEST,
        "Manifest size differs from ZIP metadata"
    );
    drop(manifest);
    ensure!(
        (1..=6).contains(&envelope.version),
        "Unsupported project bundle version {}",
        envelope.version
    );
    let project = envelope.project;
    project.validate()?;
    let mut required = HashSet::new();
    for track in &project.tracks {
        for clip in &track.clips {
            if let ClipContent::Audio { path, .. } = &clip.content {
                ensure!(
                    asset_name(path) && names.contains(path),
                    "Missing or unsafe bundled audio reference: {path}"
                );
                required.insert(path.clone());
            }
        }
    }
    ensure!(
        required.len() + 1 == names.len(),
        "Project bundle contains unreferenced assets"
    );
    fs::create_dir_all(scratch).context("Create project asset scratch directory")?;
    let mut directory_builder = tempfile::Builder::new();
    directory_builder.prefix("rill-project-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        directory_builder.permissions(fs::Permissions::from_mode(0o700));
    }
    let directory = directory_builder.tempdir_in(scratch)?;
    fs::create_dir(directory.path().join("assets"))?;
    let mut actual_total = declared_manifest;
    for index in 0..archive.len() {
        check_cancelled(cancel)?;
        if index == manifest_index {
            continue;
        }
        let mut file = archive.by_index(index)?;
        let name = std::str::from_utf8(file.name_raw())?.to_owned();
        let declared = file.size();
        let destination = directory.path().join(&name);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        copy_exact(&mut file, &mut output, declared, cancel)?;
        actual_total = actual_total
            .checked_add(declared)
            .context("Expanded size overflow")?;
        ensure!(actual_total <= MAX_TOTAL, "Expanded project exceeds 64 GiB");
        output.sync_all()?;
    }
    let root = fs::canonicalize(directory.path())?;
    let project = project.finish_loading(&root, false)?;
    check_cancelled(cancel)?;
    // Keep only fully verified assets. The application owns scratch cleanup after
    // projects using these paths are closed; dropping a failed load removes them.
    let _ = directory.keep();
    Ok(project)
}

struct LimitedWriter<'a, W> {
    inner: W,
    written: u64,
    limit: u64,
    cancel: Option<&'a AtomicBool>,
}
impl<W: Write> Write for LimitedWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        check_cancelled(self.cancel).map_err(io::Error::other)?;
        if bytes.len() as u64 > self.limit.saturating_sub(self.written) {
            return Err(io::Error::other("Project manifest exceeds 256 MiB"));
        }
        let written = self.inner.write(bytes)?;
        self.written += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct ManifestReader<'a, R> {
    inner: R,
    read: u64,
    cancel: Option<&'a AtomicBool>,
}
impl<R: Read> Read for ManifestReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        check_cancelled(self.cancel).map_err(io::Error::other)?;
        let read = self.inner.read(bytes)?;
        self.read += read as u64;
        Ok(read)
    }
}

struct MetadataReader<R> {
    inner: R,
    quota: Rc<Cell<Option<u64>>>,
}
impl<R: Read> Read for MetadataReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let allowed = match self.quota.get() {
            Some(0) => return Err(io::Error::other("Project ZIP metadata exceeds 4 MiB")),
            Some(left) => bytes.len().min(left as usize),
            None => bytes.len(),
        };
        let read = self.inner.read(&mut bytes[..allowed])?;
        if let Some(left) = self.quota.get() {
            self.quota.set(Some(left - read as u64));
        }
        Ok(read)
    }
}
impl<R: Seek> Seek for MetadataReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ChangingOutput<'a> {
        path: &'a Path,
        first: bool,
        bytes: u64,
        truncate: bool,
    }
    impl Write for ChangingOutput<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.first {
                self.first = false;
                let mut input = OpenOptions::new().write(true).open(self.path)?;
                if self.truncate {
                    input.set_len(1)?;
                } else {
                    // Same-length mutation must be rejected as well as a
                    // changed declaration or short read.
                    input.seek(SeekFrom::Start(0))?;
                    input.write_all(&[1; 16])?;
                    input.sync_all()?;
                }
            }
            self.bytes += bytes.len() as u64;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn streamed_asset_copy_rejects_same_size_mutation_and_truncation() {
        for truncate in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("source.wav");
            fs::write(&path, vec![0; COPY_BUFFER * 3]).unwrap();
            let asset = Asset {
                metadata: path.metadata().unwrap(),
                path: path.clone(),
                name: "source".into(),
            };
            let mut output = ChangingOutput {
                path: &path,
                first: true,
                bytes: 0,
                truncate,
            };
            let error = copy_asset(&asset, &mut output, None).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(if truncate { "length" } else { "changed" }),
                "{error:#}"
            );
            assert!(output.bytes <= (COPY_BUFFER * 3) as u64);
        }
    }

    #[test]
    fn bounded_copy_observes_cancellation_between_chunks() {
        struct CancelOutput<'a>(&'a AtomicBool, usize);
        impl Write for CancelOutput<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                assert!(bytes.len() <= COPY_BUFFER);
                self.1 += bytes.len();
                self.0.store(true, Ordering::Release);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let cancel = AtomicBool::new(false);
        let mut output = CancelOutput(&cancel, 0);
        let mut source = io::repeat(0).take((COPY_BUFFER * 4) as u64);
        let error = copy_exact(
            &mut source,
            &mut output,
            (COPY_BUFFER * 4) as u64,
            Some(&cancel),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(output.1, COPY_BUFFER);
    }
}
