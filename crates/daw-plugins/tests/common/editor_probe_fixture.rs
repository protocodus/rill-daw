//! Native VST fixture shared by plugin isolation and running-app E2E tests.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const CLASS_ID: &str = "11111111111111111111111111111111";

pub struct EditorProbeFixture {
    pub path: PathBuf,
    _directory: tempfile::TempDir,
}

/// Deterministic real DSP work, compiled only into a native test bundle. Tracing
/// is optional because filesystem work must not contaminate timing benchmarks.
#[derive(Default)]
pub struct DspProbeOptions<'a> {
    pub stages: usize,
    pub effect: bool,
    pub trace_directory: Option<&'a Path>,
    pub fail_after_blocks: Option<u64>,
}

impl EditorProbeFixture {
    #[allow(dead_code)]
    pub fn build_cpu_load(options: DspProbeOptions<'_>) -> Self {
        assert!(options.stages <= 4096, "CPU fixture stage limit exceeded");
        Self::build_source(
            Duration::ZERO,
            Duration::ZERO,
            false,
            options.effect,
            options.effect,
            false,
            false,
            Some(&options),
        )
    }
    /// Polyphonic test-only instrument: channels are separated in stereo and
    /// precise note velocity / targeted expression values affect actual audio.
    #[allow(dead_code)]
    pub fn build_expressive() -> Self {
        Self::build_with_expression(
            Duration::ZERO,
            Duration::ZERO,
            false,
            false,
            false,
            false,
            true,
        )
    }
    pub fn build() -> Self {
        Self::build_with_delays(Duration::ZERO, Duration::ZERO)
    }

    /// Main and auxiliary inputs, multiple outputs, a wide five-channel output,
    /// and a final mono output, all routed through real VST3 bus activation.
    #[allow(dead_code)]
    pub fn build_multi_bus() -> Self {
        Self::build_configured_buses(Duration::ZERO, Duration::ZERO, false, true)
    }

    /// Fx-category variant. Main output is the routed main/sidechain mix multiplied
    /// by the saved Level parameter (0.5 by default); no MIDI is required for audio.
    #[allow(dead_code)]
    pub fn build_multi_bus_effect() -> Self {
        Self::build_configured_kind(Duration::ZERO, Duration::ZERO, false, true, true, false)
    }
    #[allow(dead_code)]
    pub fn build_multi_bus_nonzero_main() -> Self {
        Self::build_configured_kind(Duration::ZERO, Duration::ZERO, false, true, false, true)
    }
    #[allow(dead_code)]
    pub fn build_multi_bus_effect_nonzero_main() -> Self {
        Self::build_configured_kind(Duration::ZERO, Duration::ZERO, false, true, true, true)
    }

    pub fn build_with_lifecycle_delay(delay: Duration) -> Self {
        Self::build_with_delays(delay, Duration::ZERO)
    }

    pub fn build_with_delays(lifecycle_delay: Duration, process_delay: Duration) -> Self {
        Self::build_configured(lifecycle_delay, process_delay, false)
    }

    /// The first process block may be expensive; later blocks run normally. The
    /// processor rejects audio unless the host actually advertises VST3 kOffline.
    #[allow(dead_code)] // Shared fixture is compiled by several independent test targets.
    pub fn build_for_offline(first_block_delay: Duration) -> Self {
        Self::build_configured(Duration::ZERO, first_block_delay, true)
    }

    fn build_configured(
        lifecycle_delay: Duration,
        process_delay: Duration,
        offline_only: bool,
    ) -> Self {
        Self::build_configured_buses(lifecycle_delay, process_delay, offline_only, false)
    }

    fn build_configured_buses(
        lifecycle_delay: Duration,
        process_delay: Duration,
        offline_only: bool,
        multi_bus: bool,
    ) -> Self {
        Self::build_configured_kind(
            lifecycle_delay,
            process_delay,
            offline_only,
            multi_bus,
            false,
            false,
        )
    }

    fn build_configured_kind(
        lifecycle_delay: Duration,
        process_delay: Duration,
        offline_only: bool,
        multi_bus: bool,
        effect: bool,
        nonzero_main: bool,
    ) -> Self {
        Self::build_with_expression(
            lifecycle_delay,
            process_delay,
            offline_only,
            multi_bus,
            effect,
            nonzero_main,
            false,
        )
    }

    fn build_with_expression(
        lifecycle_delay: Duration,
        process_delay: Duration,
        offline_only: bool,
        multi_bus: bool,
        effect: bool,
        nonzero_main: bool,
        expressive: bool,
    ) -> Self {
        Self::build_source(
            lifecycle_delay,
            process_delay,
            offline_only,
            multi_bus,
            effect,
            nonzero_main,
            expressive,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_source(
        lifecycle_delay: Duration,
        process_delay: Duration,
        offline_only: bool,
        multi_bus: bool,
        effect: bool,
        nonzero_main: bool,
        expressive: bool,
        dsp: Option<&DspProbeOptions<'_>>,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("editor_probe_plugin.rs");
        let mut code = include_str!("editor_probe_plugin.rs")
            .replace(
                "const EXPRESSIVE: bool = false;",
                &format!("const EXPRESSIVE: bool = {expressive};"),
            )
            .replace(
                "const MULTI_BUS: bool = false;",
                &format!("const MULTI_BUS: bool = {multi_bus};"),
            )
            .replace(
                "const MULTI_EFFECT: bool = false;",
                &format!("const MULTI_EFFECT: bool = {effect};"),
            )
            .replace(
                "const NONZERO_MAIN: bool = false;",
                &format!("const NONZERO_MAIN: bool = {nonzero_main};"),
            )
            .replace(
                "const LIFECYCLE_DELAY_MS: u64 = 0;",
                &format!(
                    "const LIFECYCLE_DELAY_MS: u64 = {};",
                    lifecycle_delay.as_millis()
                ),
            )
            .replace(
                "const PROCESS_DELAY_MS: u64 = 0;",
                &format!(
                    "const PROCESS_DELAY_MS: u64 = {};",
                    process_delay.as_millis()
                ),
            )
            .replace(
                "const FIRST_PROCESS_DELAY_ONLY: bool = false;",
                &format!("const FIRST_PROCESS_DELAY_ONLY: bool = {offline_only};"),
            )
            .replace(
                "const REQUIRE_OFFLINE: bool = false;",
                &format!("const REQUIRE_OFFLINE: bool = {offline_only};"),
            );
        if let Some(dsp) = dsp {
            code = code
                .replace(
                    "const DSP_STAGES: usize = 0;",
                    &format!("const DSP_STAGES: usize = {};", dsp.stages),
                )
                .replace(
                    "const FAIL_AFTER_BLOCKS: u64 = u64::MAX;",
                    &format!(
                        "const FAIL_AFTER_BLOCKS: u64 = {};",
                        dsp.fail_after_blocks.unwrap_or(u64::MAX)
                    ),
                );
            if let Some(directory) = dsp.trace_directory {
                code = code.replace(
                    "const TRACE_DIRECTORY: &str = \"\";",
                    &format!(
                        "const TRACE_DIRECTORY: &str = {:?};",
                        directory.to_string_lossy()
                    ),
                );
            }
        }
        std::fs::write(&source, code).unwrap();
        let path = directory.path().join("NoEditorProbe.vst3");
        let contents = path.join("Contents");
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        std::fs::write(
            contents.join("Resources/moduleinfo.json"),
            r#"{
  "Name": "No Editor Probe", "Version": "1.0",
  "Factory Info": { "Vendor": "RILL tests", "URL": "", "E-Mail": "", "Flags": {} },
  "Classes": [{
    "CID": "11111111111111111111111111111111", "Category": "Audio Module Class",
    "Name": "No Editor Probe", "Vendor": "RILL tests", "Version": "1.0",
    "SDKVersion": "VST 3.7", "Class Flags": 0, "Cardinality": 2147483647,
    "Sub Categories": ["Instrument", "Synth"]
  }]
}"#
            .replace(
                "[\"Instrument\", \"Synth\"]",
                if effect {
                    "[\"Fx\"]"
                } else {
                    "[\"Instrument\", \"Synth\"]"
                },
            ),
        )
        .unwrap();
        let binary = if cfg!(target_os = "macos") {
            std::fs::create_dir_all(contents.join("MacOS")).unwrap();
            std::fs::write(
                contents.join("Info.plist"),
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>NoEditorProbe</string>
<key>CFBundleIdentifier</key><string>app.rill.tests.no-editor-probe</string>
<key>CFBundlePackageType</key><string>BNDL</string>
</dict></plist>"#,
            )
            .unwrap();
            contents.join("MacOS/NoEditorProbe")
        } else {
            let (folder, name) = if cfg!(target_os = "windows") {
                let folder = match std::env::consts::ARCH {
                    "x86_64" => "x86_64-win",
                    "aarch64" => "arm64-win",
                    "x86" => "x86-win",
                    arch => panic!("Unsupported Windows VST3 test architecture: {arch}"),
                };
                (folder.to_owned(), "NoEditorProbe.vst3")
            } else {
                let arch = match std::env::consts::ARCH {
                    "x86" => "i386",
                    arch => arch,
                };
                (format!("{arch}-linux"), "NoEditorProbe.so")
            };
            std::fs::create_dir_all(contents.join(&folder)).unwrap();
            contents.join(folder).join(name)
        };
        // Reuse Cargo's exact SDK bindings and dependencies; no network, nested Cargo
        // invocation, C++ SDK, or additional build-tool dependency is needed in CI.
        let deps = dependency_directories();
        let mut bindings: Vec<_> = deps
            .iter()
            .flat_map(|folder| std::fs::read_dir(folder).unwrap().flatten())
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_string_lossy();
                name.starts_with("libvst3-") && name.ends_with(".rlib")
            })
            .collect();
        bindings.sort_by_key(|path| std::fs::metadata(path).unwrap().modified().unwrap());
        assert!(!bindings.is_empty(), "Missing VST3 SDK rlib in {deps:?}");
        let mut errors = Vec::new();
        for binding in bindings.iter().rev() {
            let mut compiler =
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
            // Windows CI builds the host and SDK rlibs against the static MSVC runtime.
            // Direct rustc invocations do not inherit Cargo's RUSTFLAGS processing.
            if cfg!(all(
                target_os = "windows",
                target_env = "msvc",
                target_feature = "crt-static"
            )) {
                compiler.args(["-C", "target-feature=+crt-static"]);
            }
            let output =
                compiler
                    .args([
                        "--edition=2024",
                        "--crate-type=cdylib",
                        "--crate-name=editor_probe_plugin",
                        "-C",
                        if dsp.is_some() {
                            "opt-level=3"
                        } else {
                            "opt-level=1"
                        },
                    ])
                    .arg(&source)
                    .arg("--extern")
                    .arg(format!("vst3={}", binding.display()))
                    .args(deps.iter().flat_map(|folder| {
                        ["-L".into(), format!("dependency={}", folder.display())]
                    }))
                    .arg("-o")
                    .arg(&binary)
                    .output()
                    .unwrap();
            if output.status.success() {
                return Self {
                    path,
                    _directory: directory,
                };
            }
            errors.push(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        panic!(
            "Native VST3 fixture did not compile:\n{}",
            errors.join("\n")
        );
    }
}

/// The folders that hold the rlibs this test was linked with: Cargo's one
/// `deps` folder, or, where Cargo builds every package into a folder of its
/// own (`build/<package>/<hash>/out`), each of those.
fn dependency_directories() -> Vec<PathBuf> {
    let executable = std::env::current_exe().unwrap();
    let folder = executable.parent().unwrap();
    let shared = if folder.file_name().is_some_and(|name| name == "examples") {
        folder.parent().unwrap().join("deps")
    } else {
        folder.to_path_buf()
    };
    let per_package = folder.file_name().is_some_and(|name| name == "out");
    let build = match folder.ancestors().nth(3) {
        Some(build) if per_package && build.file_name().is_some_and(|name| name == "build") => {
            build.to_path_buf()
        }
        _ if shared.is_dir() => return vec![shared],
        _ => shared.with_file_name("build"),
    };
    let folders = |of: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(of)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect()
    };
    folders(&build)
        .iter()
        .flat_map(|package| folders(package))
        .map(|unit| unit.join("out"))
        .filter(|out| out.is_dir())
        .collect()
}
