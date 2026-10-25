//! Portable, bounded project bundles. Never extracts archive-provided paths directly.
use crate::{ClipContent, Effect, Instrument, Project, model::ProjectFile};
use anyhow::{Context, Result, ensure};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const MAX_ENTRIES: usize = 4096;
const MAX_METADATA: u64 = 4 * 1024 * 1024;
const MAX_MANIFEST: u64 = 256 * 1024 * 1024;
const MAX_ASSET: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const MANIFEST: &str = "manifest.json";

pub(crate) fn is_bundle(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rill"))
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

pub(crate) fn save(project: &Project, path: &Path) -> Result<()> {
    let mut portable = project.clone();
    let base = std::env::current_dir()?;
    let mut names: HashMap<PathBuf, String> = HashMap::new();
    let mut assets = Vec::new();
    let mut total = 0u64;
    for track in &mut portable.tracks {
        if let Instrument::Vst3 { path, .. } = &mut track.instrument {
            absolute_plugin(path, &base);
        }
        for slot in &mut track.inserts {
            let Effect::Vst3 { path, .. } = &mut slot.effect;
            absolute_plugin(path, &base);
        }
        for clip in &mut track.clips {
            let ClipContent::Audio { path, .. } = &mut clip.content else {
                continue;
            };
            let source =
                fs::canonicalize(&*path).with_context(|| format!("Collect audio asset {path}"))?;
            if let Some(name) = names.get(&source) {
                *path = name.clone();
                continue;
            }
            ensure!(
                assets.len() + 1 < MAX_ENTRIES,
                "Project has too many bundled audio assets"
            );
            let metadata = source.metadata()?;
            ensure!(metadata.is_file(), "Audio asset is not a regular file");
            ensure!(metadata.len() <= MAX_ASSET, "Audio asset exceeds 2 GiB");
            total = total
                .checked_add(metadata.len())
                .context("Asset size overflow")?;
            ensure!(total <= MAX_TOTAL, "Project assets exceed 8 GiB");
            let name = format!("assets/{:08}.wav", assets.len());
            names.insert(source.clone(), name.clone());
            assets.push((source, name.clone(), metadata.len()));
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
        };
        serde_json::to_writer(
            &mut writer,
            &ProjectFile {
                version: 2,
                project: portable,
            },
        )?;
        total = total
            .checked_add(writer.written)
            .context("Project size overflow")?;
        ensure!(total <= MAX_TOTAL, "Expanded project exceeds 8 GiB");
        for (source, name, expected) in assets {
            archive.start_file(
                name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )?;
            let input = File::open(&source)?;
            let actual = io::copy(&mut input.take(expected + 1), &mut archive)?;
            ensure!(
                actual == expected && actual <= MAX_ASSET,
                "Audio asset changed while saving: {}",
                source.display()
            );
        }
        archive.finish()?;
    }
    ensure!(
        temporary.as_file().metadata()?.len() <= MAX_TOTAL,
        "Project archive exceeds 8 GiB"
    );
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .context("Atomically replace project bundle")?;
    Ok(())
}

pub(crate) fn load(path: &Path, scratch: &Path) -> Result<Project> {
    let input = File::open(path).with_context(|| format!("Open project {}", path.display()))?;
    ensure!(
        input.metadata()?.len() <= MAX_TOTAL,
        "Project archive exceeds 8 GiB"
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
        ensure!(total <= MAX_TOTAL, "Expanded project exceeds 8 GiB");
        if name == MANIFEST {
            manifest_index = Some(index);
        }
    }
    let manifest_index = manifest_index.context("Project bundle has no manifest.json")?;
    let manifest = archive.by_index(manifest_index)?;
    let declared_manifest = manifest.size();
    let mut bytes = Vec::new();
    manifest
        .take(declared_manifest + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == declared_manifest && bytes.len() as u64 <= MAX_MANIFEST,
        "Manifest size differs from ZIP metadata"
    );
    let envelope: ProjectFile = serde_json::from_slice(&bytes).context("Read project manifest")?;
    ensure!(
        (1..=2).contains(&envelope.version),
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
    let mut actual_total = bytes.len() as u64;
    for index in 0..archive.len() {
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
        let actual = io::copy(&mut (&mut file).take(declared + 1), &mut output)?;
        ensure!(
            actual == declared && actual <= MAX_ASSET,
            "Audio size differs from ZIP metadata"
        );
        actual_total = actual_total
            .checked_add(actual)
            .context("Expanded size overflow")?;
        ensure!(actual_total <= MAX_TOTAL, "Expanded project exceeds 8 GiB");
        output.sync_all()?;
    }
    let root = fs::canonicalize(directory.path())?;
    let project = project.finish_loading(&root, false)?;
    // Keep only fully verified assets. The application owns scratch cleanup after
    // projects using these paths are closed; dropping a failed load removes them.
    let _ = directory.keep();
    Ok(project)
}

struct LimitedWriter<W> {
    inner: W,
    written: u64,
    limit: u64,
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
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
