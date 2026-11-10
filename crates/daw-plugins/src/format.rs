//! Plug-in format identity and per-format discovery conventions.
//!
//! A project or catalogue reference is a bundle/library path plus an optional
//! class identifier. The format is a function of the bundle's extension, so legacy
//! references and index records without an explicit format remain VST3. Audio
//! Units have no bundle per plug-in and share the [`AU_LOCATOR`] path instead.
use crate::audio_unit::{AU_LOCATOR, AudioUnitId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum PluginFormat {
    #[default]
    Vst3,
    Clap,
    /// Linux only; other platforms never discover or load LV2 bundles.
    Lv2,
    /// Audio Units, macOS only; the reference path is [`AU_LOCATOR`].
    Au,
}

impl PluginFormat {
    pub const ALL: [Self; 4] = [Self::Vst3, Self::Clap, Self::Lv2, Self::Au];

    /// Formats this build can host on the current platform.
    pub fn available() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|format| match format {
            Self::Lv2 => cfg!(target_os = "linux"),
            Self::Au => cfg!(target_os = "macos"),
            _ => true,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Vst3 => "VST3",
            Self::Clap => "CLAP",
            Self::Lv2 => "LV2",
            Self::Au => "AU",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.label().eq_ignore_ascii_case(label))
    }

    fn extension(self) -> Option<&'static str> {
        match self {
            Self::Vst3 => Some("vst3"),
            Self::Clap => Some("clap"),
            Self::Lv2 => Some("lv2"),
            Self::Au => None,
        }
    }

    /// The hostable format whose bundle extension (or the Audio Unit locator) `path` names.
    pub fn of_bundle(path: &Path) -> Option<Self> {
        if path == Path::new(AU_LOCATOR) {
            return Self::available().find(|format| *format == Self::Au);
        }
        let extension = path.extension()?;
        Self::available().find(|format| {
            format
                .extension()
                .is_some_and(|known| extension.eq_ignore_ascii_case(known))
        })
    }

    /// Persisted references without a recognised extension keep their historic VST3
    /// meaning. The Audio Unit locator names AU everywhere, so such references fail to
    /// load with a clear reason off macOS instead of being read as VST3.
    pub fn from_path(path: &Path) -> Self {
        if path == Path::new(AU_LOCATOR) {
            return Self::Au;
        }
        Self::of_bundle(path).unwrap_or_default()
    }

    /// Validate a persisted class identifier for this format.
    pub fn validate_class_id(self, class_id: Option<&str>) -> Result<(), &'static str> {
        let Some(id) = class_id else { return Ok(()) };
        match self {
            Self::Vst3 if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Err("VST3 processor class ID must contain exactly 32 hexadecimal characters")
            }
            Self::Clap | Self::Lv2
                if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) =>
            {
                Err("Plug-in ID must be 1-1,024 printable characters")
            }
            Self::Au => AudioUnitId::parse(id).map(|_| ()),
            _ => Ok(()),
        }
    }

    /// Standard install folders plus the format's search-path variable. Audio
    /// Units are enumerated from the system registry through their locator.
    pub fn default_locations(self) -> Vec<PathBuf> {
        let (variable, mut paths) = match self {
            Self::Vst3 => ("VST3_PATH", vst3_host::discovery::scan_standard_paths()),
            Self::Clap => ("CLAP_PATH", clap_standard_paths()),
            Self::Lv2 => ("LV2_PATH", lv2_standard_paths()),
            Self::Au => return vec![PathBuf::from(AU_LOCATOR)],
        };
        if let Some(extra) = std::env::var_os(variable) {
            paths.extend(std::env::split_paths(&extra).filter(|p| !p.as_os_str().is_empty()));
        }
        paths
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// <https://github.com/free-audio/clap/blob/main/include/clap/entry.h>
fn clap_standard_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if cfg!(target_os = "macos") {
        paths.extend(home().map(|home| home.join("Library/Audio/Plug-Ins/CLAP")));
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
    } else if cfg!(windows) {
        for (variable, suffix) in [
            ("COMMONPROGRAMFILES", "CLAP"),
            ("LOCALAPPDATA", "Programs\\Common\\CLAP"),
        ] {
            if let Some(base) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
                paths.push(PathBuf::from(base).join(suffix));
            }
        }
    } else {
        paths.extend(home().map(|home| home.join(".clap")));
        paths.push(PathBuf::from("/usr/lib/clap"));
        paths.push(PathBuf::from("/usr/local/lib/clap"));
    }
    paths
}

/// lilv's default `LV2_PATH` on Linux, plus the multiarch folder.
fn lv2_standard_paths() -> Vec<PathBuf> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    let mut paths: Vec<_> = home().map(|home| home.join(".lv2")).into_iter().collect();
    paths.push(PathBuf::from("/usr/local/lib/lv2"));
    paths.push(PathBuf::from("/usr/lib/lv2"));
    // Debian and Ubuntu multiarch packages.
    paths.push(PathBuf::from(format!(
        "/usr/lib/{}-linux-gnu/lv2",
        std::env::consts::ARCH
    )));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_select_formats_and_legacy_paths_stay_vst3() {
        assert_eq!(
            PluginFormat::from_path(Path::new("/x/Synth.vst3")),
            PluginFormat::Vst3
        );
        assert_eq!(
            PluginFormat::from_path(Path::new("/x/Synth.CLAP")),
            PluginFormat::Clap
        );
        assert_eq!(
            PluginFormat::from_path(Path::new("/x/legacy")),
            PluginFormat::Vst3
        );
        assert_eq!(
            PluginFormat::of_bundle(Path::new("/x/amp.lv2")),
            cfg!(target_os = "linux").then_some(PluginFormat::Lv2)
        );
        assert_eq!(PluginFormat::from_label("clap"), Some(PluginFormat::Clap));
        assert!(
            PluginFormat::Vst3
                .validate_class_id(Some("org.example"))
                .is_err()
        );
        assert!(
            PluginFormat::Clap
                .validate_class_id(Some("org.example.synth"))
                .is_ok()
        );
        assert!(PluginFormat::Lv2.validate_class_id(Some("")).is_err());
    }

    #[test]
    fn audio_units_use_their_locator_everywhere_but_are_hosted_only_on_macos() {
        let locator = Path::new(AU_LOCATOR);
        assert_eq!(PluginFormat::from_path(locator), PluginFormat::Au);
        assert_eq!(
            PluginFormat::of_bundle(locator),
            cfg!(target_os = "macos").then_some(PluginFormat::Au)
        );
        assert_eq!(
            PluginFormat::from_path(Path::new("/Library/Frameworks/Other.framework")),
            PluginFormat::Vst3
        );
        assert_eq!(PluginFormat::from_label("au"), Some(PluginFormat::Au));
        assert_eq!(
            PluginFormat::available().any(|f| f == PluginFormat::Au),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            PluginFormat::Au.default_locations(),
            [PathBuf::from(AU_LOCATOR)]
        );
        assert!(
            PluginFormat::Au
                .validate_class_id(Some("aumu:dls :appl"))
                .is_ok()
        );
        assert!(
            PluginFormat::Au
                .validate_class_id(Some("org.example.synth"))
                .is_err()
        );
    }

    #[test]
    fn missing_format_deserializes_as_vst3() {
        #[derive(Deserialize)]
        struct Record {
            #[serde(default)]
            format: PluginFormat,
        }
        let record: Record = serde_json::from_str("{}").unwrap();
        assert_eq!(record.format, PluginFormat::Vst3);
        let record: Record = serde_json::from_str(r#"{"format":"Clap"}"#).unwrap();
        assert_eq!(record.format, PluginFormat::Clap);
    }
}
