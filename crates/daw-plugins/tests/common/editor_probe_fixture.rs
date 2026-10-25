//! Native VST fixture shared by plugin isolation and running-app E2E tests.
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

pub const CLASS_ID: &str = "11111111111111111111111111111111";

pub struct EditorProbeFixture {
    pub path: PathBuf,
    _directory: tempfile::TempDir,
}

impl EditorProbeFixture {
    pub fn build() -> Self {
        Self::build_with_delays(Duration::ZERO, Duration::ZERO)
    }

    pub fn build_with_lifecycle_delay(delay: Duration) -> Self {
        Self::build_with_delays(delay, Duration::ZERO)
    }

    pub fn build_with_delays(lifecycle_delay: Duration, process_delay: Duration) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("editor_probe_plugin.rs");
        let code = include_str!("editor_probe_plugin.rs")
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
            );
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
}"#,
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
        let deps = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let mut bindings: Vec<_> = std::fs::read_dir(&deps)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("libvst3-")
                    && path
                        .extension()
                        .is_some_and(|extension| extension == "rlib")
            })
            .collect();
        bindings.sort_by_key(|path| std::fs::metadata(path).unwrap().modified().unwrap());
        assert!(
            !bindings.is_empty(),
            "Missing VST3 SDK rlib in {}",
            deps.display()
        );
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
            let output = compiler
                .args([
                    "--edition=2024",
                    "--crate-type=cdylib",
                    "--crate-name=editor_probe_plugin",
                    "-C",
                    "opt-level=1",
                ])
                .arg(&source)
                .arg("--extern")
                .arg(format!("vst3={}", binding.display()))
                .arg("-L")
                .arg(format!("dependency={}", deps.display()))
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
