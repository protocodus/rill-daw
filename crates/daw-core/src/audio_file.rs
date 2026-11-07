//! Seekable, bounded WAV access. Descriptors retain metadata, not decoded PCM
//! or an unbounded number of OS handles. Each reader has its own cursor; the
//! small shared handle cache uses positional reads and never shared file seeks.
use anyhow::{Context, Result, ensure};
use std::{
    collections::VecDeque,
    fs::{File, Metadata, OpenOptions},
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

pub const AUDIO_STREAM_THRESHOLD_BYTES: u64 = 32 * 1024 * 1024;
pub const AUDIO_FILE_READ_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Leave room for RIFF/fmt/data headers in a canonical 32-bit float WAV.
pub const MAX_STREAMED_AUDIO_BYTES: u64 = u32::MAX as u64 - 4096;
const READER_BYTES: usize = 64 * 1024;
const MAX_CACHED_HANDLES: usize = 32;
const MAX_HEADER_BYTES: u64 = 4 * 1024 * 1024;

fn open_regular_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // An external path can become a FIFO between stat and open. Do not let
        // that race block either metadata admission or unrelated source reads.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Audio source is not a regular file",
        ));
    }
    Ok(file)
}

struct HeaderBudget<R> {
    inner: R,
    remaining: u64,
}
impl<R: Read> Read for HeaderBudget<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WAV header exceeds the 4 MiB metadata limit",
            ));
        }
        let length = bytes.len().min(self.remaining as usize);
        let count = self.inner.read(&mut bytes[..length])?;
        self.remaining -= count as u64;
        Ok(count)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFileInfo {
    pub sample_rate: u32,
    pub channels: usize,
    pub frames: usize,
    pub decoded_bytes: u64,
}
impl AudioFileInfo {
    pub fn should_stream(self) -> bool {
        self.decoded_bytes >= AUDIO_STREAM_THRESHOLD_BYTES
    }
    pub fn duration_seconds(self) -> f64 {
        self.frames as f64 / f64::from(self.sample_rate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
impl Stamp {
    fn read(metadata: &Metadata) -> Result<Self> {
        ensure!(metadata.is_file(), "Audio source is not a regular file");
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            bytes: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        })
    }
}

#[derive(Debug)]
struct Source {
    path: PathBuf,
    stamp: Stamp,
    info: AudioFileInfo,
}

impl Drop for Source {
    fn drop(&mut self) {
        // In particular, release handles to private derived WAVs before their
        // media leases remove them. Otherwise an unlinked multi-GiB cache file
        // could occupy disk until an unrelated LRU eviction or application exit.
        if let Some(cache) = HANDLES.get()
            && let Ok(mut cache) = cache.lock()
        {
            cache
                .files
                .retain(|(path, stamp, _)| path != &self.path || stamp != &self.stamp);
        }
    }
}

/// An admitted source generation. Replacing/truncating its path is an error,
/// rather than silently changing an existing clip's sound. Managed media must
/// remain available while active projects, history or background jobs use it.
#[derive(Clone, Debug)]
pub struct AudioFile(Arc<Source>);

#[derive(Default)]
struct HandleCache {
    files: VecDeque<(PathBuf, Stamp, Arc<File>)>,
}
impl HandleCache {
    fn get(&mut self, source: &Source) -> Option<Arc<File>> {
        let index = self
            .files
            .iter()
            .position(|(path, stamp, _)| path == &source.path && stamp == &source.stamp)?;
        let entry = self.files.remove(index)?;
        let file = Arc::clone(&entry.2);
        self.files.push_back(entry);
        Some(file)
    }
}
static HANDLES: OnceLock<Mutex<HandleCache>> = OnceLock::new();
fn handles() -> &'static Mutex<HandleCache> {
    HANDLES.get_or_init(|| Mutex::new(HandleCache::default()))
}

impl AudioFile {
    /// Header-only admission. Payload decoding and finite checks are performed
    /// in bounded windows; imports additionally verify their entire output.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = std::fs::canonicalize(path.as_ref())
            .with_context(|| format!("Open audio source {}", path.as_ref().display()))?;
        // Reject devices and FIFOs before opening: opening a FIFO for reading
        // could otherwise block the single metadata-admission worker forever.
        ensure!(
            std::fs::metadata(&path)?.is_file(),
            "Audio source is not a regular file"
        );
        let file = open_regular_file(&path)?;
        let stamp = Stamp::read(&file.metadata()?)?;
        let reader = hound::WavReader::new(BufReader::with_capacity(
            READER_BYTES,
            HeaderBudget {
                inner: &file,
                remaining: MAX_HEADER_BYTES,
            },
        ))
        .with_context(|| format!("Read WAV header {}", path.display()))?;
        let spec = reader.spec();
        ensure!(
            (1..=32).contains(&spec.channels) && (8_000..=384_000).contains(&spec.sample_rate),
            "Audio must have 1–32 channels at 8–384 kHz"
        );
        ensure!(
            match spec.sample_format {
                hound::SampleFormat::Float => spec.bits_per_sample == 32,
                hound::SampleFormat::Int => (1..=32).contains(&spec.bits_per_sample),
            },
            "Unsupported WAV sample format"
        );
        let channels = usize::from(spec.channels);
        let samples = reader.len() as usize;
        ensure!(samples.is_multiple_of(channels), "Incomplete WAV frame");
        let decoded_bytes = (samples as u64)
            .checked_mul(4)
            .context("Audio source size overflow")?;
        ensure!(
            decoded_bytes <= MAX_STREAMED_AUDIO_BYTES,
            "Audio exceeds the supported float-WAV source size"
        );
        let info = AudioFileInfo {
            sample_rate: spec.sample_rate,
            channels,
            frames: samples / channels,
            decoded_bytes,
        };
        drop(reader);
        ensure!(
            Stamp::read(&file.metadata()?)? == stamp,
            "Audio source changed during admission"
        );
        Ok(Self(Arc::new(Source { path, stamp, info })))
    }

    pub fn info(&self) -> AudioFileInfo {
        self.0.info
    }
    pub fn path(&self) -> &Path {
        &self.0.path
    }
    /// Distinguish admitted file generations without touching the filesystem.
    pub fn same_source(&self, other: &Self) -> bool {
        self.0.path == other.0.path && self.0.stamp == other.0.stamp
    }
    pub fn check_unchanged(&self) -> Result<()> {
        let stamp = Stamp::read(
            &std::fs::metadata(self.path())
                .with_context(|| format!("Read audio source {}", self.path().display()))?,
        )?;
        ensure!(
            stamp == self.0.stamp,
            "Audio source changed: {}",
            self.path().display()
        );
        Ok(())
    }
    fn file(&self) -> io::Result<Arc<File>> {
        {
            let mut cache = handles()
                .lock()
                .map_err(|_| io::Error::other("Audio handle cache unavailable"))?;
            if let Some(file) = cache.get(&self.0) {
                return Ok(file);
            }
        }
        // Opening and inspecting a cold file must not hold the global LRU lock.
        let file = Arc::new(open_regular_file(self.path())?);
        let stamp = Stamp::read(&file.metadata()?).map_err(io::Error::other)?;
        if stamp != self.0.stamp {
            return Err(io::Error::other("Audio source changed since admission"));
        }
        let mut cache = handles()
            .lock()
            .map_err(|_| io::Error::other("Audio handle cache unavailable"))?;
        if let Some(existing) = cache.get(&self.0) {
            return Ok(existing);
        }
        while cache.files.len() >= MAX_CACHED_HANDLES {
            cache.files.pop_front();
        }
        cache
            .files
            .push_back((self.0.path.clone(), stamp, Arc::clone(&file)));
        Ok(file)
    }

    pub fn reader(&self) -> Result<AudioFileReader> {
        self.check_unchanged()?;
        let header_only = Arc::new(AtomicBool::new(true));
        let reader = hound::WavReader::new(BufReader::with_capacity(
            READER_BYTES,
            Cursor {
                source: self.clone(),
                position: 0,
                header_only: Arc::clone(&header_only),
            },
        ))?;
        header_only.store(false, Ordering::Relaxed);
        ensure!(
            reader.len() as u64 * 4 == self.info().decoded_bytes
                && usize::from(reader.spec().channels) == self.info().channels
                && reader.spec().sample_rate == self.info().sample_rate,
            "Audio header changed since admission"
        );
        Ok(AudioFileReader {
            source: self.clone(),
            reader,
        })
    }
}

struct Cursor {
    source: AudioFile,
    position: u64,
    header_only: Arc<AtomicBool>,
}
impl Read for Cursor {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let length = if self.header_only.load(Ordering::Relaxed) {
            if self.position >= MAX_HEADER_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WAV header exceeds the 4 MiB metadata limit",
                ));
            }
            bytes.len().min((MAX_HEADER_BYTES - self.position) as usize)
        } else {
            bytes.len()
        };
        let bytes = &mut bytes[..length];
        let file = self.source.file()?;
        #[cfg(unix)]
        let count = {
            use std::os::unix::fs::FileExt;
            file.read_at(bytes, self.position)?
        };
        #[cfg(windows)]
        let count = {
            use std::os::windows::fs::FileExt;
            file.seek_read(bytes, self.position)?
        };
        #[cfg(not(any(unix, windows)))]
        compile_error!("Positional audio reads require a Unix or Windows file API");
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("Audio cursor overflow"))?;
        Ok(count)
    }
}
impl Seek for Cursor {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.source.0.stamp.bytes) + i128::from(offset),
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid audio seek"))?;
        Ok(self.position)
    }
}

pub struct AudioFileReader {
    source: AudioFile,
    reader: hound::WavReader<BufReader<Cursor>>,
}
impl AudioFileReader {
    pub fn info(&self) -> AudioFileInfo {
        self.source.info()
    }
    pub fn check_unchanged(&self) -> Result<()> {
        self.source.check_unchanged()
    }
    /// Read at most four MiB into caller-owned interleaved storage. The result
    /// is the number of complete frames; the remainder of the output is zero.
    /// Disk faults, incomplete payloads and nonfinite data are never successful
    /// silence, including when an offline renderer reaches a cold page.
    pub fn read_frames(&mut self, start: usize, output: &mut [f32]) -> Result<usize> {
        let info = self.info();
        ensure!(
            output.len().is_multiple_of(info.channels),
            "Incomplete read destination frame"
        );
        ensure!(
            output.len() <= AUDIO_FILE_READ_MAX_BYTES / size_of::<f32>(),
            "Audio read exceeds bounded window size"
        );
        self.check_unchanged()?;
        output.fill(0.0);
        let frames = (output.len() / info.channels).min(info.frames.saturating_sub(start));
        if frames == 0 {
            return Ok(0);
        }
        self.reader
            .seek(u32::try_from(start).context("Audio seek exceeds WAV frame range")?)?;
        let count = frames * info.channels;
        match self.reader.spec().sample_format {
            hound::SampleFormat::Float => {
                let mut samples = self.reader.samples::<f32>();
                for value in &mut output[..count] {
                    *value = samples.next().context("Audio source is truncated")??;
                    ensure!(value.is_finite(), "Audio contains non-finite samples");
                }
            }
            hound::SampleFormat::Int => {
                let scale = 2.0_f64.powi(i32::from(self.reader.spec().bits_per_sample) - 1);
                let mut samples = self.reader.samples::<i32>();
                for value in &mut output[..count] {
                    let sample = samples.next().context("Audio source is truncated")??;
                    *value = (f64::from(sample) / scale) as f32;
                }
            }
        }
        self.check_unchanged()?;
        Ok(frames)
    }
}

/// Diagnostics concern handles retained by this cache; active positional reads
/// may temporarily retain an evicted handle until their syscall completes.
pub fn cached_audio_file_handles() -> usize {
    handles().lock().map_or(0, |cache| cache.files.len())
}

/// Release a retired private media file even if a reusable reader window still
/// holds its descriptor. In-flight positional reads keep their own handle alive.
pub fn evict_cached_audio_file(path: &Path) {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(cache) = HANDLES.get()
        && let Ok(mut cache) = cache.lock()
    {
        cache.files.retain(|(source, _, _)| source != &canonical);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn open_guard_cannot_block_on_a_fifo_replaced_after_metadata_admission() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("replaced.wav");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            // Deliberately bypass the caller's earlier path metadata check:
            // this is exactly the cold-cache stat/open replacement race.
            sender.send(open_regular_file(&path).is_err()).unwrap();
        });
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap()
        );
        worker.join().unwrap();
    }
}
