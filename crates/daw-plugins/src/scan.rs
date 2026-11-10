//! Metadata discovery for formats whose descriptors live in native code (CLAP), need a
//! host library (LV2) or a system registry (Audio Units). Each bundle, and the AU
//! registry as a whole, is read in a disposable helper with a deadline.
use crate::{PluginFormat, PluginInfo, helper_path};
use anyhow::{Context, Result, bail, ensure};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SCAN_DEADLINE: Duration = Duration::from_secs(15);
const MAX_SCAN_BYTES: u64 = 16 * 1024 * 1024;

/// `daw-plugin-host --scan <bundle>`: print the bundle's plug-ins as one JSON line.
pub(crate) fn helper_main() -> Result<()> {
    let mut protocol = vst3_host::process_isolation::ProtocolChannel::claim();
    let path = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .context("Missing plug-in bundle to scan")?,
    );
    let found = match PluginFormat::of_bundle(&path) {
        Some(PluginFormat::Clap) => crate::clap::scan(&path),
        #[cfg(target_os = "linux")]
        Some(PluginFormat::Lv2) => crate::lv2::scan(&path),
        #[cfg(target_os = "macos")]
        Some(PluginFormat::Au) => crate::au::scan(),
        _ => Err(anyhow::anyhow!("Unsupported plug-in bundle")),
    };
    let reply = found.map_err(|error| format!("{error:#}"));
    serde_json::to_writer(&mut protocol, &reply)?;
    writeln!(protocol)?;
    protocol.flush()?;
    Ok(())
}

/// Read one CLAP/LV2 bundle's descriptors, or the installed Audio Units, without
/// loading plug-in code into this process.
pub fn scan_bundle(path: &Path) -> Result<Vec<PluginInfo>> {
    scan_with(&helper_path()?, path, SCAN_DEADLINE)
}

fn scan_with(helper: &Path, path: &Path, deadline: Duration) -> Result<Vec<PluginInfo>> {
    let mut child = Command::new(helper)
        .arg("--scan")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Starting plug-in scan helper {}", helper.display()))?;
    let mut stdout = child.stdout.take().context("Missing scan output pipe")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stdout).take(MAX_SCAN_BYTES).read_to_end(&mut bytes);
        bytes
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "Scanning {} exceeded {} seconds; helper terminated",
                path.display(),
                deadline.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let bytes = reader.join().unwrap_or_default();
    ensure!(
        !bytes.is_empty(),
        "Plug-in scan helper exited without a result ({status})"
    );
    let reply: std::result::Result<Vec<PluginInfo>, String> =
        serde_json::from_slice(&bytes).context("Invalid plug-in scan reply")?;
    let mut found = reply.map_err(anyhow::Error::msg)?;
    ensure!(
        found.len() <= 4096,
        "Plug-in bundle declares too many classes"
    );
    for info in &mut found {
        // Never trust a path or format echoed by plug-in code.
        info.path = path.to_owned();
        info.format = PluginFormat::from_path(path);
        PluginFormat::from_path(path)
            .validate_class_id(info.class_id.as_deref())
            .map_err(anyhow::Error::msg)?;
    }
    Ok(found)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake(script: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scan-helper");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, path)
    }

    #[test]
    fn hung_or_crashing_scans_are_contained_and_paths_are_not_trusted() {
        let bundle = Path::new("/plugins/Synth.clap");
        let (_d, hang) = fake("exec sleep 30");
        let start = Instant::now();
        let error = scan_with(&hang, bundle, Duration::from_millis(100)).unwrap_err();
        assert!(error.to_string().contains("exceeded"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(5));
        let (_d, crash) = fake("kill -9 $$");
        assert!(scan_with(&crash, bundle, Duration::from_secs(5)).is_err());
        let (_d, liar) = fake(
            r#"printf '%s\n' '{"Ok":[{"name":"Synth","path":"/etc/passwd","vendor":"","category":"Instrument","class_id":"org.rill.synth","format":"Vst3"}]}'"#,
        );
        let found = scan_with(&liar, bundle, Duration::from_secs(5)).unwrap();
        assert_eq!(found[0].path, bundle);
        assert_eq!(found[0].format, PluginFormat::Clap);
    }
}
