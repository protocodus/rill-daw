//! Plug-in format identity and per-format discovery conventions.
//!
//! A project or catalogue reference is a bundle/library path plus an optional
//! class identifier. The format is a function of the bundle's extension, so legacy
//! references and index records without an explicit format remain VST3.
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
}

impl PluginFormat {
    pub const ALL: [Self; 3] = [Self::Vst3, Self::Clap, Self::Lv2];

    /// Formats this build can host on the current platform.
    pub fn available() -> impl Iterator<Item = Self> {
        Self::ALL
            .into_iter()
            .filter(|format| *format != Self::Lv2 || cfg!(target_os = "linux"))
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Vst3 => "VST3",
            Self::Clap => "CLAP",
            Self::Lv2 => "LV2",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.label().eq_ignore_ascii_case(label))
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Vst3 => "vst3",
            Self::Clap => "clap",
            Self::Lv2 => "lv2",
        }
    }

    /// The format whose bundle extension `path` carries.
    pub fn of_bundle(path: &Path) -> Option<Self> {
        let extension = path.extension()?;
        Self::available().find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }

    /// Persisted references without a recognised extension keep their historic VST3 meaning.
    pub fn from_path(path: &Path) -> Self {
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
            _ => Ok(()),
        }
    }

    /// Standard install folders plus the format's search-path variable.
    pub fn default_locations(self) -> Vec<PathBuf> {
        let (variable, mut paths) = match self {
            Self::Vst3 => ("VST3_PATH", vst3_host::discovery::scan_standard_paths()),
            Self::Clap => ("CLAP_PATH", clap_standard_paths()),
            Self::Lv2 => ("LV2_PATH", lv2_standard_paths()),
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
