use daw_engine::DeviceSettings;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: daw_ui::theme::Appearance,
    pub device_name: Option<String>,
    pub sample_rate: u32,
    pub buffer_frames: u32,
    pub resample_quality: daw_engine::ResampleQuality,
    pub scratch_dir: PathBuf,
    pub vst_dirs: Vec<PathBuf>,
}

fn config_dir() -> PathBuf {
    if let Some(directory) = std::env::var_os("RILL_CONFIG_DIR")
        && !directory.is_empty()
    {
        return PathBuf::from(directory);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Rill")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join("Rill")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("rill")
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: daw_ui::theme::Appearance::System,
            device_name: None,
            sample_rate: 48_000,
            buffer_frames: 256,
            resample_quality: daw_engine::ResampleQuality::High,
            scratch_dir: config_dir().join("Scratch"),
            vst_dirs: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from(&config_dir().join("settings.json"))
    }

    fn load_from(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|settings| settings.validate().is_ok())
            .unwrap_or_default()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (8_000..=192_000).contains(&self.sample_rate),
            "Invalid sample rate"
        );
        anyhow::ensure!(
            (32..=4096).contains(&self.buffer_frames),
            "Invalid buffer size"
        );
        anyhow::ensure!(
            !self.scratch_dir.as_os_str().is_empty(),
            "Choose a scratch directory"
        );
        anyhow::ensure!(
            self.device_name
                .as_ref()
                .is_none_or(|name| !name.trim().is_empty()),
            "Invalid output device name"
        );
        Ok(())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_in(&config_dir())
    }

    fn save_in(&self, directory: &Path) -> anyhow::Result<()> {
        self.validate()?;
        fs::create_dir_all(&self.scratch_dir)?;
        // Verify the scratch directory is writable before accepting the setup.
        let probe = temporary_path(&self.scratch_dir, "rill-write-check");
        drop(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe)?,
        );
        fs::remove_file(probe)?;
        fs::create_dir_all(directory)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        let temp = temporary_path(directory, "settings");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| -> anyhow::Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, directory.join("settings.json"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
    pub fn audio(&self) -> DeviceSettings {
        DeviceSettings {
            device_name: self.device_name.clone(),
            sample_rate: self.sample_rate,
            buffer_frames: self.buffer_frames,
            resample_quality: self.resample_quality,
        }
    }
}

fn temporary_path(directory: &Path, prefix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        ".{prefix}.{}.{timestamp}.{serial}.tmp",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let directory = temporary_path(&std::env::temp_dir(), "rill-settings-test");
            fs::create_dir(&directory).unwrap();
            Self(directory)
        }
        fn settings(&self) -> Settings {
            Settings {
                scratch_dir: self.0.join("scratch"),
                ..Settings::default()
            }
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn invalid_saved_settings_fall_back_before_reaching_the_ui() {
        let directory = TestDirectory::new();
        let path = directory.0.join("settings.json");
        for bytes in [
            br#"{"buffer_frames":4294967295}"#.as_slice(),
            br#"{"sample_rate":0}"#.as_slice(),
            br#"{"scratch_dir":""}"#.as_slice(),
            br#"{"device_name":" "}"#.as_slice(),
            b"not json".as_slice(),
        ] {
            fs::write(&path, bytes).unwrap();
            assert_eq!(Settings::load_from(&path), Settings::default());
        }
        fs::write(&path, br#"{"sample_rate":44100}"#).unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.sample_rate, 44_100);
        assert_eq!(loaded.buffer_frames, 256);
    }

    #[test]
    fn failed_save_preserves_existing_settings() {
        let directory = TestDirectory::new();
        let config = directory.0.join("config");
        let settings = directory.settings();
        settings.save_in(&config).unwrap();
        let before = fs::read(config.join("settings.json")).unwrap();
        let invalid_scratch = directory.0.join("not-a-directory");
        fs::write(&invalid_scratch, "existing file").unwrap();
        let invalid = Settings {
            scratch_dir: invalid_scratch,
            sample_rate: 44_100,
            ..settings
        };
        assert!(invalid.save_in(&config).is_err());
        assert_eq!(fs::read(config.join("settings.json")).unwrap(), before);
    }

    #[test]
    fn failed_atomic_replace_cleans_its_temporary_file() {
        let directory = TestDirectory::new();
        let config = directory.0.join("config");
        fs::create_dir_all(config.join("settings.json")).unwrap();
        assert!(directory.settings().save_in(&config).is_err());
        assert_eq!(fs::read_dir(&config).unwrap().count(), 1);
        assert!(config.join("settings.json").is_dir());
        assert_eq!(
            fs::read_dir(directory.0.join("scratch")).unwrap().count(),
            0
        );
    }

    #[test]
    fn concurrent_saves_use_independent_temporary_files() {
        let directory = TestDirectory::new();
        let config = directory.0.join("config");
        let first = directory.settings();
        let second = Settings {
            sample_rate: 44_100,
            ..first.clone()
        };
        std::thread::scope(|scope| {
            let a = scope.spawn(|| first.save_in(&config));
            let b = scope.spawn(|| second.save_in(&config));
            a.join().unwrap().unwrap();
            b.join().unwrap().unwrap();
        });
        let loaded = Settings::load_from(&config.join("settings.json"));
        assert!(loaded == first || loaded == second);
        assert_eq!(fs::read_dir(&config).unwrap().count(), 1);
    }
}
