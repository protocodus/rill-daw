//! Native editor control stays in a separate helper process, including capture.

use crate::{ParameterInfo, helper_path};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

fn request_id() -> Result<u64> {
    let mut current = NEXT_REQUEST.load(Ordering::Relaxed);
    loop {
        let next = current
            .checked_add(1)
            .context("Native editor request IDs exhausted")?;
        match NEXT_REQUEST.compare_exchange_weak(
            current,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(current),
            Err(actual) => current = actual,
        }
    }
}

const DEADLINE: Duration = Duration::from_secs(30);
const MAX_WIRE_BYTES: u64 = 256 * 1024 * 1024 + 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EditorEvent {
    /// Initial metadata, and refreshed values following a changed state
    /// snapshot. Preserve a selected parameter while its ID remains present.
    Ready {
        parameters: Vec<ParameterInfo>,
    },
    ParameterChanged {
        id: u32,
        value: f64,
    },
    /// The user chose the host automation action in a native parameter menu.
    /// The value is sampled in the editor sandbox when the action is selected.
    AutomateParameter {
        id: u32,
        value: f64,
    },
    StateSaved {
        /// `None` is automatic/native-window-close state, never an explicit Save
        /// acknowledgement. Match the ID returned by `save_state` or `close`.
        request_id: Option<u64>,
        state: Vec<u8>,
    },
    Captured {
        path: PathBuf,
        width: u32,
        height: u32,
    },
    Closed {
        request_id: Option<u64>,
    },
    Error {
        message: String,
    },
    #[doc(hidden)]
    Heartbeat,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(crate) enum EditorCommand {
    Open {
        path: PathBuf,
        class_id: Option<String>,
        state: Vec<u8>,
        parameters: Vec<(u32, f64)>,
        visible: bool,
    },
    SetParameter {
        id: u32,
        value: f64,
    },
    SaveState {
        request_id: u64,
    },
    /// Debug-only regression hook for an opaque preset change without callbacks.
    #[cfg(debug_assertions)]
    RestoreStateWithoutNotifications {
        state: Vec<u8>,
    },
    Capture {
        path: PathBuf,
    },
    Close {
        save_state: bool,
        request_id: Option<u64>,
    },
    /// Debug-only native UI regression hook. No parameter setter is involved.
    #[cfg(all(debug_assertions, target_os = "macos"))]
    MouseDrag {
        from: [f64; 2],
        to: [f64; 2],
    },
}

/// A Send handle. Open it on a worker; polling and control writes are nonblocking.
/// Keep it alive until `Closed` after requesting close so the final state is retained.
pub struct EditorSession {
    child: Child,
    commands: SyncSender<EditorCommand>,
    events: Receiver<EditorEvent>,
    pending: Option<EditorEvent>,
    last_response: Instant,
    response_timeout: Duration,
    close_request: Cell<Option<u64>>,
    closed: bool,
}

impl EditorSession {
    pub fn open(path: &Path, state: &[u8], parameters: &[(u32, f64)]) -> Result<Self> {
        Self::open_class(path, None, state, parameters)
    }

    pub fn open_info(
        info: &crate::PluginInfo,
        state: &[u8],
        parameters: &[(u32, f64)],
    ) -> Result<Self> {
        Self::open_class(&info.path, info.class_id.as_deref(), state, parameters)
    }

    pub fn open_class(
        path: &Path,
        class_id: Option<&str>,
        state: &[u8],
        parameters: &[(u32, f64)],
    ) -> Result<Self> {
        Self::open_config(path, class_id, state, parameters, true)
    }

    fn open_config(
        path: &Path,
        class_id: Option<&str>,
        state: &[u8],
        parameters: &[(u32, f64)],
        visible: bool,
    ) -> Result<Self> {
        ensure!(
            cfg!(any(
                target_os = "macos",
                target_os = "linux",
                target_os = "windows"
            )),
            "Native VST editors require macOS, Windows, or Linux/X11"
        );
        ensure!(
            state.len() <= 64 * 1024 * 1024,
            "Plugin state exceeds 64 MiB"
        );
        ensure!(
            parameters
                .iter()
                .all(|(_, value)| value.is_finite() && (0.0..=1.0).contains(value)),
            "Invalid normalized parameter value"
        );
        Self::spawn(
            EditorCommand::Open {
                path: path.to_owned(),
                class_id: class_id.map(str::to_owned),
                state: state.to_vec(),
                parameters: parameters.to_vec(),
                visible,
            },
            &helper_path()?,
            DEADLINE,
        )
    }

    fn spawn(command: EditorCommand, helper: &Path, timeout: Duration) -> Result<Self> {
        let mut child = Command::new(helper)
            .arg("--editor")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut input = child.stdin.take().context("Missing editor command pipe")?;
        let output = child
            .stdout
            .take()
            .context("Missing editor response pipe")?;
        let (commands, commands_rx) = mpsc::sync_channel(32);
        let (events_tx, events) = mpsc::sync_channel(256);
        std::thread::spawn(move || {
            while let Ok(command) = commands_rx.recv() {
                if serde_json::to_writer(&mut input, &command).is_err()
                    || writeln!(&mut input).is_err()
                    || input.flush().is_err()
                {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut line = String::new();
                match (&mut reader).take(MAX_WIRE_BYTES + 1).read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() as u64 > MAX_WIRE_BYTES => {
                        let _ = events_tx.send(EditorEvent::Error {
                            message: "Editor response exceeded its size limit".into(),
                        });
                        break;
                    }
                    Ok(_) => {
                        let event = serde_json::from_str(&line).unwrap_or_else(|error| {
                            EditorEvent::Error {
                                message: format!("Invalid native editor response: {error}"),
                            }
                        });
                        if events_tx.send(event).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut session = Self {
            child,
            commands,
            events,
            pending: None,
            last_response: Instant::now(),
            response_timeout: timeout,
            close_request: Cell::new(None),
            closed: false,
        };
        session.send(command)?;
        let deadline = Instant::now() + timeout;
        loop {
            let event = session
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .context("Native editor initialization timed out or exited")?;
            match event {
                EditorEvent::Ready { .. } => {
                    session.last_response = Instant::now();
                    session.pending = Some(event);
                    return Ok(session);
                }
                EditorEvent::Heartbeat => {}
                EditorEvent::Error { message } => bail!(message),
                _ => bail!("Native editor exited before initialization completed"),
            }
        }
    }

    fn send(&self, command: EditorCommand) -> Result<()> {
        ensure!(!self.closed, "Native editor is closed");
        self.commands
            .try_send(command)
            .map_err(|error| anyhow::anyhow!("Native editor command queue unavailable: {error}"))
    }

    pub fn set_parameter(&self, id: u32, value: f64) -> Result<()> {
        ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "Parameter value must be normalized to 0..=1"
        );
        self.send(EditorCommand::SetParameter { id, value })
    }

    /// Request a snapshot; only StateSaved carrying this ID acknowledges it.
    pub fn save_state(&self) -> Result<u64> {
        ensure!(
            self.close_request.get().is_none(),
            "Native editor is closing"
        );
        let request_id = request_id()?;
        self.send(EditorCommand::SaveState { request_id })?;
        Ok(request_id)
    }

    /// Request a saved-state close. Repeated calls share the outstanding close ID.
    /// Completion requires both StateSaved and Closed carrying that ID.
    pub fn close(&self) -> Result<u64> {
        ensure!(!self.closed, "Native editor is closed");
        if let Some(id) = self.close_request.get() {
            return Ok(id);
        }
        let request_id = request_id()?;
        self.send(EditorCommand::Close {
            save_state: true,
            request_id: Some(request_id),
        })?;
        self.close_request.set(Some(request_id));
        Ok(request_id)
    }

    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    pub fn try_recv(&mut self) -> Option<EditorEvent> {
        if let Some(event) = self.pending.take() {
            return Some(event);
        }
        if self.closed {
            return None;
        }
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    self.last_response = Instant::now();
                    if matches!(event, EditorEvent::Heartbeat) {
                        continue;
                    }
                    if matches!(event, EditorEvent::Closed { .. }) {
                        self.closed = true;
                    }
                    if matches!(event, EditorEvent::Error { .. }) {
                        // A failure is terminal: queued or malformed Closed events
                        // can never turn a failed state operation into success.
                        self.closed = true;
                        let _ = self.child.kill();
                    }
                    return Some(event);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.closed = true;
                    return Some(EditorEvent::Error {
                        message: "Native editor process exited unexpectedly".into(),
                    });
                }
                Err(mpsc::TryRecvError::Empty)
                    if self.last_response.elapsed() > self.response_timeout =>
                {
                    self.closed = true;
                    let _ = self.child.kill();
                    return Some(EditorEvent::Error {
                        message: "Native editor stopped responding and was terminated".into(),
                    });
                }
                Err(mpsc::TryRecvError::Empty) => return None,
            }
        }
    }
}

impl Drop for EditorSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn launch_editor(path: &Path) -> Result<EditorSession> {
    EditorSession::open(path, &[], &[])
}

/// Render and capture the real native view, with a bounded lifetime and no DAW-side loading.
pub fn capture_editor(path: &Path, output_png: &Path) -> Result<()> {
    capture_session(
        EditorSession::open_config(path, None, &[], &[], false)?,
        output_png,
    )
}

pub fn capture_editor_info(info: &crate::PluginInfo, output_png: &Path) -> Result<()> {
    capture_session(
        EditorSession::open_config(&info.path, info.class_id.as_deref(), &[], &[], false)?,
        output_png,
    )
}

fn capture_session(mut session: EditorSession, output_png: &Path) -> Result<()> {
    session.send(EditorCommand::Capture {
        path: output_png.to_owned(),
    })?;
    let deadline = Instant::now() + DEADLINE;
    let mut captured = false;
    loop {
        ensure!(Instant::now() < deadline, "Native editor capture timed out");
        if let Some(event) = session.try_recv() {
            match event {
                EditorEvent::Captured { .. } => {
                    session.send(EditorCommand::Close {
                        save_state: false,
                        request_id: None,
                    })?;
                    captured = true;
                }
                EditorEvent::Error { message } => bail!(message),
                EditorEvent::Closed { .. } if captured => return Ok(()),
                EditorEvent::Closed { .. } => bail!("Native editor closed before capture"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn helper_main() -> Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        super::editor_native::run()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("Native VST editors require macOS, Windows, or Linux/X11")
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(unix, debug_assertions))]
    use super::*;

    #[test]
    fn editor_session_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<super::EditorSession>();
    }

    #[cfg(unix)]
    fn fake_helper(script: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("editor-helper");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, path)
    }

    #[cfg(unix)]
    fn wait_for_failure(session: &mut EditorSession) -> String {
        let start = Instant::now();
        loop {
            if let Some(EditorEvent::Error { message }) = session.try_recv() {
                return message;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_editor_crash_is_contained() {
        let (_directory, helper) =
            fake_helper("printf '%s\\n' '{\"event\":\"ready\",\"parameters\":[]}'\nexec sleep 30");
        let mut session = EditorSession::spawn(
            EditorCommand::SaveState { request_id: 1 },
            &helper,
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(matches!(
            session.try_recv(),
            Some(EditorEvent::Ready { .. })
        ));
        session.child.kill().unwrap();
        assert!(wait_for_failure(&mut session).contains("exited unexpectedly"));
        assert!(session.try_recv().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn native_editor_hang_is_terminated_at_deadline() {
        let (_directory, helper) =
            fake_helper("printf '%s\\n' '{\"event\":\"ready\",\"parameters\":[]}'\nexec sleep 30");
        let mut session = EditorSession::spawn(
            EditorCommand::SaveState { request_id: 1 },
            &helper,
            Duration::from_secs(5),
        )
        .unwrap();
        session.response_timeout = Duration::from_millis(80);
        assert!(matches!(
            session.try_recv(),
            Some(EditorEvent::Ready { .. })
        ));
        assert!(wait_for_failure(&mut session).contains("stopped responding"));
        session.child.wait().unwrap();
        assert!(session.try_recv().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn native_editor_startup_hang_is_bounded() {
        let (_directory, helper) = fake_helper("exec sleep 30");
        let start = Instant::now();
        let error = EditorSession::spawn(
            EditorCommand::SaveState { request_id: 1 },
            &helper,
            Duration::from_millis(80),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("initialization timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn snapshots_and_close_keep_distinct_request_ids() {
        let (_directory, helper) = fake_helper(
            r#"
printf '%s\n' '{"event":"ready","parameters":[]}'
IFS= read -r initial
while IFS= read -r command; do
    id=${command##*:}
    id=${id%\}}
    case "$command" in
        *'"command":"save_state"'*)
            printf '%s\n' '{"event":"state_saved","request_id":null,"state":[1]}'
            printf '{"event":"state_saved","request_id":%s,"state":[1]}\n' "$id" ;;
        *'"command":"close"'*)
            printf '{"event":"state_saved","request_id":%s,"state":[2]}\n' "$id"
            printf '{"event":"closed","request_id":%s}\n' "$id"
            exit 0 ;;
    esac
done
"#,
        );
        let mut session = EditorSession::spawn(
            EditorCommand::SaveState { request_id: 0 },
            &helper,
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(matches!(
            session.try_recv(),
            Some(EditorEvent::Ready { .. })
        ));
        let first = session.save_state().unwrap();
        let second = session.save_state().unwrap();
        let closing = session.close().unwrap();
        assert_ne!(first, second);
        assert_ne!(second, closing);
        assert_eq!(session.close().unwrap(), closing);
        assert!(session.save_state().is_err());
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut snapshots = Vec::new();
        loop {
            assert!(
                Instant::now() < deadline,
                "Missing correlated editor acknowledgement"
            );
            match session.try_recv() {
                Some(EditorEvent::StateSaved { request_id, state }) => {
                    snapshots.push((request_id, state))
                }
                Some(EditorEvent::Closed { request_id }) => {
                    assert_eq!(request_id, Some(closing));
                    break;
                }
                Some(EditorEvent::Error { message }) => panic!("{message}"),
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert_eq!(
            snapshots,
            vec![
                (None, vec![1]),
                (Some(first), vec![1]),
                (None, vec![1]),
                (Some(second), vec![1]),
                (Some(closing), vec![2])
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_snapshot_cannot_be_followed_by_successful_close() {
        let (_directory, helper) = fake_helper(
            r#"
printf '%s\n' '{"event":"ready","parameters":[]}'
IFS= read -r initial
printf '%s\n' '{"event":"error","message":"state failed"}' '{"event":"closed","request_id":7}'
exec sleep 30
"#,
        );
        let mut session = EditorSession::spawn(
            EditorCommand::SaveState { request_id: 7 },
            &helper,
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(matches!(
            session.try_recv(),
            Some(EditorEvent::Ready { .. })
        ));
        assert_eq!(wait_for_failure(&mut session), "state failed");
        assert!(session.try_recv().is_none());
        assert!(session.close().is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    #[ignore = "Requires installed CHOWTapeModel in RILL_TEST_VST_EFFECT and a desktop"]
    fn opaque_effect_preset_without_notifications_is_automatically_saved() {
        fn next(session: &mut EditorSession, wanted: impl Fn(&EditorEvent) -> bool) -> EditorEvent {
            let deadline = Instant::now() + DEADLINE;
            loop {
                assert!(
                    Instant::now() < deadline,
                    "Missing automatic preset snapshot"
                );
                if let Some(event) = session.try_recv() {
                    if let EditorEvent::Error { message } = &event {
                        panic!("{message}");
                    }
                    assert!(
                        !matches!(event, EditorEvent::ParameterChanged { .. }),
                        "This preset regression must not depend on parameter callbacks"
                    );
                    if wanted(&event) {
                        return event;
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let path = PathBuf::from(std::env::var("RILL_TEST_VST_EFFECT").unwrap());
        let mut effect = crate::VstEffect::load(&path, 48_000.0, 256).unwrap();
        let parameter = effect
            .parameters()
            .unwrap()
            .into_iter()
            .find(|p| p.name == "Output Gain")
            .unwrap();
        let baseline = effect.save_state().unwrap();
        effect.set_parameter(parameter.id, 0.73).unwrap();
        // A processor block makes the effect's opaque component state current.
        let input = [0.0; 256];
        effect
            .process(&input, &input, &mut [0.0; 256], &mut [0.0; 256])
            .unwrap();
        let preset = effect.save_state().unwrap();
        drop(effect);
        let mut session = EditorSession::open(&path, &baseline, &[]).unwrap();
        next(&mut session, |e| matches!(e, EditorEvent::Ready { .. }));
        session
            .send(EditorCommand::RestoreStateWithoutNotifications { state: preset })
            .unwrap();
        let EditorEvent::StateSaved { state, .. } = next(&mut session, |e| {
            matches!(
                e,
                EditorEvent::StateSaved {
                    request_id: None,
                    ..
                }
            )
        }) else {
            unreachable!()
        };
        let EditorEvent::Ready { parameters } =
            next(&mut session, |e| matches!(e, EditorEvent::Ready { .. }))
        else {
            unreachable!()
        };
        assert!(
            (parameters
                .iter()
                .find(|p| p.id == parameter.id)
                .unwrap()
                .value
                - 0.73)
                .abs()
                < 1e-5
        );
        let mut restored = crate::VstEffect::load(&path, 48_000.0, 256).unwrap();
        restored.load_state(&state).unwrap();
        assert!(
            (restored
                .parameters()
                .unwrap()
                .iter()
                .find(|p| p.id == parameter.id)
                .unwrap()
                .value
                - 0.73)
                .abs()
                < 1e-5
        );
        let requested = session.save_state().unwrap();
        next(
            &mut session,
            |e| matches!(e, EditorEvent::StateSaved { request_id: Some(id), .. } if *id == requested),
        );
        let closing = session.close().unwrap();
        next(
            &mut session,
            |e| matches!(e, EditorEvent::Closed { request_id: Some(id) } if *id == closing),
        );
    }

    #[cfg(all(debug_assertions, target_os = "macos"))]
    #[test]
    #[ignore = "Requires installed Surge XT in RILL_TEST_VST_INSTRUMENT and a desktop"]
    fn native_surgeslider_gesture_reports_last_touched_parameter() {
        let path = PathBuf::from(std::env::var("RILL_TEST_VST_INSTRUMENT").unwrap());
        let mut session = EditorSession::open(&path, &[], &[]).unwrap();
        let Some(EditorEvent::Ready { parameters }) = session.try_recv() else {
            panic!("No ready event")
        };
        let parameter = parameters
            .iter()
            .find(|p| p.name == "Global Volume")
            .unwrap();
        std::thread::sleep(Duration::from_millis(700));
        // Drag the real Global Volume slider in Surge's default skin; this never
        // invokes Plugin::set_parameter and therefore requires performEdit feedback.
        session
            .send(EditorCommand::MouseDrag {
                from: [0.96, 0.071],
                to: [0.87, 0.071],
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < deadline,
                "Native slider did not report a parameter gesture"
            );
            if let Some(event) = session.try_recv() {
                match event {
                    EditorEvent::ParameterChanged { id, value } if id == parameter.id => {
                        assert!((value - parameter.value).abs() > 0.01);
                        eprintln!(
                            "Surge native mouse gesture: parameter {id}, {} -> {value}",
                            parameter.value
                        );
                        break;
                    }
                    EditorEvent::Error { message } => panic!("{message}"),
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        session.close().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(session.try_recv(), Some(EditorEvent::Closed { .. })) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(all(debug_assertions, target_os = "macos"))]
    #[test]
    #[ignore = "Requires installed Surge XT in RILL_TEST_VST_INSTRUMENT and a desktop"]
    fn native_editor_exits_when_owner_closes_pipe_after_ready() {
        let path = PathBuf::from(std::env::var("RILL_TEST_VST_INSTRUMENT").unwrap());
        let mut session = EditorSession::open(&path, &[], &[]).unwrap();
        assert!(matches!(
            session.try_recv(),
            Some(EditorEvent::Ready { .. })
        ));
        // Close only the owner's command pipe, without sending Close or killing
        // the child. The helper's independent stdin monitor must terminate it.
        let (replacement, _receiver) = mpsc::sync_channel(1);
        session.commands = replacement;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = session.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Orphaned native editor remains alive"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
