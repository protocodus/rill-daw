use std::path::PathBuf;
use std::time::{Duration, Instant};
use vst3_host::process_isolation::{HostCommand, HostResponse, PluginHostProcess};

#[test]
fn helper_reports_errors_without_exiting() {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_daw-plugin-host"));
    let mut child = PluginHostProcess::new(Some(path), Duration::from_secs(5)).unwrap();
    for _ in 0..2 {
        let response = child.send_command(HostCommand::StartProcessing).unwrap();
        assert!(matches!(response, HostResponse::Error { .. }));
        assert!(child.is_alive());
    }
}

#[cfg(unix)]
#[test]
fn a_killed_worker_returns_an_error_and_parent_survives() {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_daw-plugin-host"));
    let mut child = PluginHostProcess::new(Some(path), Duration::from_millis(100)).unwrap();
    let pid = child.helper_pid().unwrap();
    assert!(
        std::process::Command::new("/bin/kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let start = Instant::now();
    assert!(child.send_command(HostCommand::StartProcessing).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(child.send_command(HostCommand::StartProcessing).is_err());
}

#[cfg(unix)]
#[test]
fn a_hung_worker_is_killed_at_the_response_deadline() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hung-helper");
    std::fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = PluginHostProcess::new(Some(path), Duration::from_millis(80)).unwrap();
    let start = Instant::now();
    let error = child
        .send_command(HostCommand::StartProcessing)
        .unwrap_err();
    assert!(error.to_lowercase().contains("timed out"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(child.send_command(HostCommand::StartProcessing).is_err());
}
