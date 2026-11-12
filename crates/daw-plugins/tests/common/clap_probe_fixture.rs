//! Native CLAP fixture shared by plug-in isolation and running-app E2E tests.
use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
pub const SYNTH_ID: &str = "org.rill.test.synth";
#[allow(dead_code)]
pub const GAIN_ID: &str = "org.rill.test.gain";
#[allow(dead_code)]
pub const GAIN_LATENCY: u32 = 32;

pub struct ClapProbeFixture {
    pub path: PathBuf,
    _directory: tempfile::TempDir,
}

impl ClapProbeFixture {
    /// Compile `clap_probe_plugin.rs` into `RillProbe.clap` in a fresh directory.
    pub fn build() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = Self::build_in(directory.path());
        Self {
            path,
            _directory: directory,
        }
    }

    /// Build into a caller-owned folder, for scan-folder tests.
    pub fn build_in(folder: &Path) -> PathBuf {
        let source = folder.join("clap_probe_plugin.rs");
        std::fs::write(&source, include_str!("clap_probe_plugin.rs")).unwrap();
        let path = folder.join("RillProbe.clap");
        let binary = if cfg!(target_os = "macos") {
            let contents = path.join("Contents");
            std::fs::create_dir_all(contents.join("MacOS")).unwrap();
            std::fs::write(
                contents.join("Info.plist"),
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>RillProbe</string>
<key>CFBundleIdentifier</key><string>app.rill.tests.clap-probe</string>
<key>CFBundlePackageType</key><string>BNDL</string>
</dict></plist>"#,
            )
            .unwrap();
            contents.join("MacOS/RillProbe")
        } else {
            path.clone()
        };
        let deps = dependency_directories();
        let mut bindings: Vec<_> = deps
            .iter()
            .flat_map(|folder| std::fs::read_dir(folder).unwrap().flatten())
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_string_lossy();
                name.starts_with("libclap_sys-") && name.ends_with(".rlib")
            })
            .collect();
        bindings.sort_by_key(|path| std::fs::metadata(path).unwrap().modified().unwrap());
        assert!(!bindings.is_empty(), "Missing clap-sys rlib in {deps:?}");
        let mut errors = Vec::new();
        for binding in bindings.iter().rev() {
            let mut compiler =
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
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
                        "--crate-name=clap_probe_plugin",
                        "-C",
                        "opt-level=1",
                    ])
                    .arg(&source)
                    .arg("--extern")
                    .arg(format!("clap_sys={}", binding.display()))
                    .args(deps.iter().flat_map(|folder| {
                        ["-L".into(), format!("dependency={}", folder.display())]
                    }))
                    .arg("-o")
                    .arg(&binary)
                    .output()
                    .unwrap();
            if output.status.success() {
                return path;
            }
            errors.push(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        panic!(
            "Native CLAP fixture did not compile:\n{}",
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
