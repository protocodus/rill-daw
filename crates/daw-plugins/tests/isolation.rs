use std::path::PathBuf;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;
use vst3_host::process_isolation::{HostCommand, HostResponse, PluginHostProcess};

#[test]
fn native_editor_exits_when_owner_pipe_closes_before_initialization() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_daw-plugin-host"))
        .arg("--editor")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdin.take());
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Orphaned editor did not exit after its owner pipe closed");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

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
    // A concurrent fork can briefly inherit a writable descriptor for this fresh
    // script. Retry only exec's ETXTBSY, before starting the protocol deadline.
    // The SDK exposes spawn errors as strings, retaining the numeric OS code.
    let spawn_deadline = Instant::now() + Duration::from_secs(2);
    let mut child = loop {
        match PluginHostProcess::new(Some(path.clone()), Duration::from_millis(80)) {
            Ok(child) => break child,
            Err(error) if error.contains("(os error 26)") && Instant::now() < spawn_deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("Failed to start hung-worker fixture: {error}"),
        }
    };
    let start = Instant::now();
    let error = child
        .send_command(HostCommand::Process {
            inputs: vec![],
            frames: 64,
        })
        .unwrap_err();
    assert!(error.to_lowercase().contains("timed out"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(!child.is_alive(), "Timed-out worker must be terminated");
    assert!(child.send_command(HostCommand::StartProcessing).is_err());
}
