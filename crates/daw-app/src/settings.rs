use daw_engine::DeviceSettings;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ai: AiSettings,
    pub appearance: daw_ui::theme::Appearance,
    pub track_density: daw_ui::TrackDensity,
    pub export: ExportSettings,
    pub device_name: Option<String>,
    pub input_device_name: Option<String>,
    pub midi_input_device: Option<String>,
    pub sample_rate: u32,
    pub buffer_frames: u32,
    pub resample_quality: daw_engine::ResampleQuality,
    pub history_limit: usize,
    pub scratch_dir: PathBuf,
    pub vst_dirs: Vec<PathBuf>,
    /// Successful project saves seed the next new project's picker too.
    pub last_project_directory: Option<PathBuf>,
}

/// Presentation defaults for delivery. Exact loop bounds belong to each
/// captured export request, rather than leaking between unrelated projects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportRangeMode {
    #[default]
    Arrangement,
    Loop,
}

/// One mix file, or one file per selected channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportWhat {
    #[default]
    Mix,
    Stems,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    /// None follows playback; a chosen rate never reconfigures the audio device.
    pub sample_rate: Option<u32>,
    /// Sample depth for WAV and FLAC.
    pub encoding: daw_engine::WavEncoding,
    pub format: daw_engine::ExportFormat,
    pub dither: bool,
    pub include_effect_tails: bool,
    pub range_mode: ExportRangeMode,
    pub what: ExportWhat,
    /// Stems are taken before Master unless this is on.
    pub stems_through_master: bool,
    pub stems_include_mix: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            sample_rate: None,
            encoding: daw_engine::WavEncoding::Float32,
            format: daw_engine::ExportFormat::Wav,
            dither: true,
            include_effect_tails: true,
            range_mode: ExportRangeMode::Arrangement,
            what: ExportWhat::Mix,
            stems_through_master: false,
            stems_include_mix: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub provider: daw_ai::Provider,
    pub model: String,
    pub api_key: Option<daw_ai::EncryptedSecret>,
}

impl Default for AiSettings {
    fn default() -> Self {
        let provider = daw_ai::Provider::default();
        Self {
            provider,
            model: provider.default_model().into(),
            api_key: None,
        }
    }
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
            ai: AiSettings::default(),
            appearance: daw_ui::theme::Appearance::System,
            track_density: daw_ui::TrackDensity::default(),
            export: ExportSettings::default(),
            device_name: None,
            input_device_name: None,
            midi_input_device: None,
            sample_rate: 48_000,
            buffer_frames: 256,
            resample_quality: daw_engine::ResampleQuality::High,
            history_limit: daw_core::HISTORY_STEPS,
            scratch_dir: config_dir().join("Scratch"),
            vst_dirs: Vec::new(),
            last_project_directory: None,
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
            (1..=daw_core::MAX_HISTORY_STEPS).contains(&self.history_limit),
            "Undo history must keep between 1 and {} actions",
            daw_core::MAX_HISTORY_STEPS
        );
        anyhow::ensure!(
            (8_000..=192_000).contains(&self.sample_rate),
            "Invalid sample rate"
        );
        anyhow::ensure!(
            self.export
                .sample_rate
                .is_none_or(|rate| (8_000..=192_000).contains(&rate)),
            "Invalid export sample rate"
        );
        self.export
            .format
            .validate(daw_engine::WavEncoding::Pcm16, 44_100)
            .map_err(|_| anyhow::anyhow!("Invalid export format"))?;
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
        anyhow::ensure!(
            self.input_device_name
                .as_ref()
                .is_none_or(|name| !name.trim().is_empty()),
            "Invalid input device name"
        );
        anyhow::ensure!(
            self.midi_input_device
                .as_ref()
                .is_none_or(|id| !id.trim().is_empty()),
            "Invalid MIDI input device"
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
        write_atomically(
            directory,
            "settings.json",
            &serde_json::to_vec_pretty(self)?,
        )
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

/// How the user organizes the plug-in library. Keys come from
/// [`PluginLibrary::key`], so they survive plug-in updates and cache resets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginLibrary {
    pub favorites: BTreeSet<String>,
    /// Stable plug-in keys, most recently assigned/inserted first.
    pub recently_used: Vec<String>,
    /// User categories replacing the plug-in's own VST3 subcategories.
    pub categories: BTreeMap<String, Vec<String>>,
}

impl PluginLibrary {
    pub fn record_use(&mut self, plugin: &daw_plugins::PluginInfo) {
        let key = Self::key(plugin);
        self.recently_used.retain(|existing| existing != &key);
        self.recently_used.insert(0, key);
        self.recently_used.truncate(50);
    }

    pub fn key(plugin: &daw_plugins::PluginInfo) -> String {
        format!(
            "{}#{}",
            plugin.path.display(),
            plugin.class_id.as_deref().unwrap_or_default()
        )
    }

    pub fn load() -> Self {
        fs::read(config_dir().join("plugin-library.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        write_atomically(
            &config_dir(),
            "plugin-library.json",
            &serde_json::to_vec_pretty(self)?,
        )
    }
}

fn write_atomically(directory: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    fs::create_dir_all(directory)?;
    let temp = temporary_path(directory, name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| -> anyhow::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, directory.join(name))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
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
            br#"{"history_limit":0}"#.as_slice(),
            br#"{"history_limit":4097}"#.as_slice(),
            br#"{"export":{"sample_rate":0}}"#.as_slice(),
            b"not json".as_slice(),
        ] {
            fs::write(&path, bytes).unwrap();
            assert_eq!(Settings::load_from(&path), Settings::default());
        }
        fs::write(&path, br#"{"sample_rate":44100}"#).unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.sample_rate, 44_100);
        assert_eq!(loaded.buffer_frames, 256);
        assert_eq!(loaded.last_project_directory, None);
        assert_eq!(loaded.history_limit, daw_core::HISTORY_STEPS);
        assert_eq!(loaded.track_density, daw_ui::TrackDensity::Compact);
        assert_eq!(loaded.export, ExportSettings::default());
    }

    #[test]
    fn delivery_and_density_preferences_roundtrip_without_reconfiguring_audio() {
        let directory = TestDirectory::new();
        let config = directory.0.join("config");
        let settings = Settings {
            track_density: daw_ui::TrackDensity::Detailed,
            export: ExportSettings {
                sample_rate: Some(44_100),
                encoding: daw_engine::WavEncoding::Pcm24,
                dither: true,
                include_effect_tails: false,
                range_mode: ExportRangeMode::Loop,
                format: daw_engine::ExportFormat::Mp3 { kbps: 256 },
                what: ExportWhat::Stems,
                stems_through_master: true,
                stems_include_mix: true,
            },
            ..directory.settings()
        };
        let audio = settings.audio();
        settings.save_in(&config).unwrap();
        let restored = Settings::load_from(&config.join("settings.json"));
        assert_eq!(restored, settings);
        let restored_audio = restored.audio();
        assert_eq!(restored_audio.sample_rate, audio.sample_rate);
        assert_eq!(restored_audio.buffer_frames, audio.buffer_frames);
        assert_eq!(restored_audio.device_name, audio.device_name);
        assert_eq!(restored_audio.resample_quality, audio.resample_quality);
    }

    #[test]
    fn custom_history_limit_roundtrips_without_changing_other_settings() {
        let directory = TestDirectory::new();
        let config = directory.0.join("config");
        let settings = Settings {
            history_limit: 75,
            ..directory.settings()
        };
        settings.save_in(&config).unwrap();
        assert_eq!(Settings::load_from(&config.join("settings.json")), settings);
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
