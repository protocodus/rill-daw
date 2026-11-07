//! Immutable source ownership. Logical audio length is independent of resident PCM.
use anyhow::{Context, Result};
use daw_core::{AudioData, AudioFile};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub(crate) struct TempPathLease {
    path: PathBuf,
}
impl TempPathLease {
    pub fn new() -> Result<Arc<Self>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir();
        for _ in 0..128 {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let path = root.join(format!(
                "rill-audio-cache-{}-{stamp}-{sequence}.wav",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(_) => return Ok(Arc::new(Self { path })),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("Create private streamed audio cache"),
            }
        }
        anyhow::bail!("Could not reserve a private streamed audio cache")
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_cache_permissions_and_last_lease_cleanup() {
        let lease = TempPathLease::new().unwrap();
        let path = lease.path().to_owned();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
            "Prepared audio must be readable and writable only by its owner"
        );
        let retained = Arc::clone(&lease);
        drop(lease);
        assert!(path.exists(), "An active lease keeps its cache available");
        drop(retained);
        assert!(!path.exists(), "The last lease removes its private cache");
    }
}
impl Drop for TempPathLease {
    fn drop(&mut self) {
        daw_core::evict_cached_audio_file(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) enum AudioAsset {
    Resident(AudioData),
    Disk {
        file: AudioFile,
        _lease: Option<Arc<TempPathLease>>,
    },
}
impl AudioAsset {
    pub fn from_resident(audio: AudioData) -> Arc<Self> {
        Arc::new(Self::Resident(audio))
    }
    pub fn from_source_file(file: AudioFile) -> Arc<Self> {
        Arc::new(Self::Disk { file, _lease: None })
    }
    pub fn from_prepared_file(file: AudioFile, lease: Arc<TempPathLease>) -> Arc<Self> {
        Arc::new(Self::Disk {
            file,
            _lease: Some(lease),
        })
    }
    pub fn resident(&self) -> Option<&AudioData> {
        match self {
            Self::Resident(audio) => Some(audio),
            Self::Disk { .. } => None,
        }
    }
    pub fn file(&self) -> Option<&AudioFile> {
        match self {
            Self::Disk { file, .. } => Some(file),
            Self::Resident(_) => None,
        }
    }
    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::Resident(audio) => audio.sample_rate,
            Self::Disk { file, .. } => file.info().sample_rate,
        }
    }
    pub fn channels(&self) -> usize {
        match self {
            Self::Resident(audio) => audio.channels,
            Self::Disk { file, .. } => file.info().channels,
        }
    }
    pub fn frames(&self) -> usize {
        match self {
            Self::Resident(audio) => audio.frames(),
            Self::Disk { file, .. } => file.info().frames,
        }
    }
    pub fn resident_bytes(&self) -> usize {
        self.resident()
            .map_or(0, |audio| audio.samples.len() * size_of::<f32>())
    }
    pub fn is_streamed(&self) -> bool {
        matches!(self, Self::Disk { .. })
    }
    pub fn ensure_file(
        self: &Arc<Self>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Arc<Self>> {
        if self.is_streamed() {
            return Ok(Arc::clone(self));
        }
        let audio = self.resident().expect("resident asset");
        let mut writer = crate::stretch::StreamWriter::new(
            audio.sample_rate,
            audio.channels,
            audio.frames(),
            cancel,
        )?;
        for block in audio.samples.chunks(4096 * audio.channels) {
            writer.write(block)?;
        }
        writer.finish()
    }
}
