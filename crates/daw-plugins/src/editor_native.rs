//! Native editor lifetime and IPC. This module runs only in the isolated helper.

use crate::{
    ParameterInfo,
    editor::{EditorCommand, EditorEvent},
};
use crate::{editor_context_menu::ContextMenuProcess, editor_platform::Platform};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use vst3_host::{
    BusAudioBuffers, HostNotification, ParameterEditKind, Plugin, PluginWindow, Vst3Host,
    process_isolation::ProtocolChannel,
};

const SNAPSHOT_DEBOUNCE: Duration = Duration::from_millis(350);
const SNAPSHOT_MIN_INTERVAL: Duration = Duration::from_secs(1);
const SNAPSHOT_FALLBACK: Duration = Duration::from_secs(2);

/// Runs only on the isolated editor's control thread. The fallback covers preset
/// changes which update opaque state without sending parameter/dirty callbacks.
struct SnapshotSchedule {
    last_snapshot: Instant,
    last_edit: Option<Instant>,
    digest: Option<[u8; 32]>,
}

impl SnapshotSchedule {
    fn new(now: Instant, state: &[u8]) -> Self {
        Self {
            last_snapshot: now,
            last_edit: None,
            digest: (!state.is_empty()).then(|| Sha256::digest(state).into()),
        }
    }

    fn edited(&mut self, now: Instant) {
        self.last_edit = Some(now);
    }

    fn due(&self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last_snapshot);
        elapsed >= SNAPSHOT_MIN_INTERVAL
            && (elapsed >= SNAPSHOT_FALLBACK
                || self
                    .last_edit
                    .is_some_and(|edit| now.saturating_duration_since(edit) >= SNAPSHOT_DEBOUNCE))
    }

    fn captured(&mut self, now: Instant, state: &[u8]) -> bool {
        let digest = Sha256::digest(state).into();
        let changed = self.digest != Some(digest);
        self.digest = Some(digest);
        self.last_snapshot = now;
        self.last_edit = None;
        changed
    }
}

fn current_parameters(plugin: &mut Plugin) -> Result<Vec<ParameterInfo>> {
    let parameters = plugin.get_parameters()?;
    // Configure before the editor opens: some plug-ins copy the host's entries
    // into their own menu instead of asking us to display a popup. Eligibility is
    // checked again by the host and when the selected action reaches this helper.
    let actions = parameters
        .iter()
        .filter(|parameter| parameter.can_automate && !parameter.is_read_only)
        .map(|parameter| (parameter.id, "Automate in Rill".to_owned()))
        .collect::<Vec<_>>();
    plugin.set_parameter_context_menu_actions(&actions)?;
    Ok(parameters
        .into_iter()
        .map(|p| ParameterInfo {
            id: p.id,
            name: p.name,
            value: p.value,
            default: p.default,
            unit: p.unit,
            steps: p.step_count,
            read_only: p.is_read_only,
        })
        .collect())
}

pub(crate) fn run() -> Result<()> {
    let mut protocol = ProtocolChannel::claim();
    if let Err(error) = run_inner(&mut protocol) {
        if peer_disconnected(&error) {
            return Ok(());
        }
        if let Err(error) = emit(
            &mut protocol,
            &EditorEvent::Error {
                message: format!("{error:#}"),
            },
        ) && !peer_disconnected(&error)
        {
            return Err(error);
        }
    }
    Ok(())
}

fn peer_disconnected(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe)
    })
}

fn emit(output: &mut impl Write, event: &EditorEvent) -> Result<()> {
    serde_json::to_writer(&mut *output, event)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

fn save_state(
    plugin: &Arc<Mutex<Plugin>>,
    buffers: &mut BusAudioBuffers,
    output: &mut impl Write,
    snapshots: &mut SnapshotSchedule,
    request_id: Option<u64>,
    automatic: bool,
) -> Result<()> {
    let mut plugin = plugin
        .lock()
        .map_err(|_| anyhow::anyhow!("Editor lock poisoned"))?;
    // Controller edits are queued for the processor's next block. Commit them
    // before acknowledging a snapshot, including back-to-back edit/save commands.
    plugin.process_bus_audio(buffers)?;
    let state = plugin.save_state()?;
    ensure!(
        state.len() <= 64 * 1024 * 1024,
        "Plugin state exceeds 64 MiB"
    );
    let changed = snapshots.captured(Instant::now(), &state);
    // Explicit save/close always acknowledges its own request, even when an
    // automatic snapshot already published these bytes.
    if !automatic || changed {
        emit(output, &EditorEvent::StateSaved { request_id, state })?;
        // An explicit save can arrive before the fallback sees a preset change.
        // Refresh metadata here too, otherwise digest dedup would suppress the
        // later automatic refresh and leave the owner's values stale.
        if changed {
            emit(
                output,
                &EditorEvent::Ready {
                    parameters: current_parameters(&mut plugin)?,
                },
            )?;
        }
    }
    Ok(())
}

fn run_inner(output: &mut impl Write) -> Result<()> {
    let (commands_tx, commands) = mpsc::sync_channel(32);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<EditorCommand>(&line) {
                Ok(command) => {
                    if commands_tx.try_send(command).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        // The owner vanished. Exit from this independent reader thread even when
        // third-party initialization or GUI code has hung the main thread.
        std::process::exit(0);
    });
    let EditorCommand::Open {
        path,
        class_id,
        state,
        parameters,
        visible,
    } = commands.recv_timeout(Duration::from_secs(30))?
    else {
        anyhow::bail!("First native editor command must initialize a plugin");
    };
    ensure!(
        state.len() <= 64 * 1024 * 1024,
        "Plugin state exceeds 64 MiB"
    );
    let platform = Platform::new()?;
    let mut host = Vst3Host::builder()
        .with_process_isolation(false)
        .sample_rate(48_000.0)
        .block_size(256)
        .build()?;
    let mut plugin = match class_id {
        Some(id) => host.load_plugin_class(&path, &id)?,
        None => host.load_plugin(&path)?,
    };
    ensure!(plugin.has_editor(), "This plug-in has no native editor");
    if !state.is_empty() {
        plugin.load_state(&state)?;
    }
    for (id, value) in parameters {
        ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "Invalid normalized parameter value"
        );
        plugin.set_parameter(id, value)?;
    }
    let parameters = current_parameters(&mut plugin)?;
    let mut buffers = plugin.create_bus_audio_buffers(256)?;
    plugin.start_processing()?;
    let title = format!("{} - VST3", plugin.info().name);
    let plugin = Arc::new(Mutex::new(plugin));
    let mut window = PluginWindow::new(Arc::clone(&plugin));
    window.open()?;
    platform.configure(&title, visible)?;
    emit(output, &EditorEvent::Ready { parameters })?;
    let mut snapshots = SnapshotSchedule::new(Instant::now(), &state);
    let mut heartbeat = Instant::now();
    let mut next_process = Instant::now();
    let mut capture: Option<(PathBuf, Instant)> = None;
    let mut context_menu: Option<ContextMenuProcess> = None;
    let mut closing = None;
    while closing.is_none() {
        for _ in 0..32 {
            match commands.try_recv() {
                Ok(EditorCommand::SetParameter { id, value }) => {
                    ensure!(
                        value.is_finite() && (0.0..=1.0).contains(&value),
                        "Invalid normalized parameter value"
                    );
                    plugin.lock().unwrap().set_parameter(id, value)?;
                    snapshots.edited(Instant::now());
                    emit(output, &EditorEvent::ParameterChanged { id, value })?;
                }
                Ok(EditorCommand::SaveState { request_id }) => save_state(
                    &plugin,
                    &mut buffers,
                    output,
                    &mut snapshots,
                    Some(request_id),
                    false,
                )?,
                #[cfg(debug_assertions)]
                Ok(EditorCommand::RestoreStateWithoutNotifications { state }) => {
                    ensure!(
                        state.len() <= 64 * 1024 * 1024,
                        "Plugin state exceeds 64 MiB"
                    );
                    let mut plugin = plugin.lock().unwrap();
                    plugin.load_state(&state)?;
                    // Deliberately suppress signals to prove the periodic opaque
                    // state fallback works independently of automation callbacks.
                    let _ = plugin.take_parameter_edits();
                    let _ = plugin.get_parameter_changes();
                    let _ = plugin.take_host_notifications();
                    let _ = plugin.take_restart_flags();
                }
                Ok(EditorCommand::Capture { path }) => {
                    capture = Some((path, Instant::now() + Duration::from_millis(700)))
                }
                Ok(EditorCommand::Close {
                    save_state,
                    request_id,
                }) => {
                    closing = Some((save_state, request_id));
                    break;
                }
                #[cfg(all(debug_assertions, target_os = "macos"))]
                Ok(EditorCommand::MouseDrag { from, to }) => {
                    platform.mouse_drag(&title, from, to)?;
                }
                Ok(EditorCommand::Open { .. }) => anyhow::bail!("Editor already initialized"),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    closing = Some((false, None));
                    break;
                }
            }
        }
        // Dispatch native events without the plug-in lock: controls may call the host.
        platform.pump()?;
        window.service_platform_events()?;
        if closing.is_none() && visible && (window.closed_by_user() || platform.closed_by_user()) {
            closing = Some((true, None));
        }
        // The native menu has its own process because OS popup calls can run a
        // nested, blocking event loop. Polling keeps editor IPC and processing
        // alive for as long as the user leaves a menu open.
        let menu_result = context_menu.as_mut().and_then(|menu| match menu.poll() {
            Ok(Some(selection)) => Some((menu.menu_id(), selection)),
            Ok(None) => None,
            Err(error) => {
                eprintln!("Native context menu closed: {error:#}");
                Some((menu.menu_id(), None))
            }
        });
        let mut menu_requests = Vec::new();
        let mut automation_requests = Vec::new();
        {
            let mut plugin = plugin.lock().unwrap();
            if let Some((menu_id, selection)) = menu_result {
                context_menu = None;
                let result = match selection.filter(|_| closing.is_none()) {
                    Some(item_id) => plugin.execute_context_menu_item(menu_id, item_id),
                    None => plugin.dismiss_context_menu(menu_id),
                };
                if let Err(error) = result {
                    // A preset/reload can invalidate a menu while it is open.
                    // Cancelling an obsolete menu must not close the editor.
                    let _ = plugin.dismiss_context_menu(menu_id);
                    eprintln!("Native context menu action ignored: {error}");
                }
            }
            plugin.service_run_loop();
            if Instant::now() >= next_process {
                plugin.process_bus_audio(&mut buffers)?;
                next_process = Instant::now() + Duration::from_millis(5);
            }
            // Only deliberate editor gestures should become "last touched". Generic
            // changes also contain processor meters/LFOs, so drain those separately.
            let _ = plugin.get_parameter_changes();
            for edit in plugin.take_parameter_edits() {
                if matches!(
                    edit.kind,
                    ParameterEditKind::ValueChange | ParameterEditKind::EndGesture
                ) {
                    snapshots.edited(Instant::now());
                }
                if edit.kind == ParameterEditKind::ValueChange
                    && let Some(value) = edit.value
                    && value.is_finite()
                    && (0.0..=1.0).contains(&value)
                {
                    emit(
                        output,
                        &EditorEvent::ParameterChanged { id: edit.id, value },
                    )?;
                }
            }
            for notification in plugin.take_host_notifications() {
                match notification {
                    HostNotification::DirtyChanged(true)
                    | HostNotification::ProgramListChanged { .. }
                    | HostNotification::GroupEditFinished => snapshots.edited(Instant::now()),
                    HostNotification::ContextMenuRequested {
                        menu_id,
                        x,
                        y,
                        items,
                        ..
                    } => {
                        if visible && closing.is_none() {
                            menu_requests.push((menu_id, x, y, items));
                        } else {
                            let _ = plugin.dismiss_context_menu(menu_id);
                        }
                    }
                    HostNotification::ParameterAutomationRequested { id }
                        if visible && closing.is_none() =>
                    {
                        automation_requests.push(id);
                    }
                    _ => {}
                }
            }
            let flags = plugin.service_host_requests()?;
            if flags.param_values_changed()
                || flags.param_titles_changed()
                || flags.reload_component()
            {
                snapshots.edited(Instant::now());
                emit(
                    output,
                    &EditorEvent::Ready {
                        parameters: current_parameters(&mut plugin)?,
                    },
                )?;
            }
            if flags.io_changed() {
                buffers = plugin.create_bus_audio_buffers(256)?;
            }
        }
        for id in automation_requests {
            // Commit opaque preset changes and queued controller edits before
            // creating a lane. Otherwise its playback rebuild and Undo entry
            // could precede an earlier preset change's periodic snapshot.
            save_state(&plugin, &mut buffers, output, &mut snapshots, None, true)?;
            let mut plugin = plugin.lock().unwrap();
            // Never infer a parameter from focus or "last touched". Revalidate
            // the exact callback ID after the snapshot, then sample its value.
            if plugin
                .get_parameters()?
                .iter()
                .any(|p| p.id == id && p.can_automate && !p.is_read_only)
                && let Ok(value) = plugin.get_parameter(id)
                && value.is_finite()
                && (0.0..=1.0).contains(&value)
            {
                emit(
                    output,
                    &EditorEvent::Ready {
                        parameters: current_parameters(&mut plugin)?,
                    },
                )?;
                emit(output, &EditorEvent::AutomateParameter { id, value })?;
            }
        }
        for (menu_id, x, y, items) in menu_requests {
            if let Some(previous) = context_menu.take() {
                let previous_id = previous.menu_id();
                drop(previous);
                let _ = plugin.lock().unwrap().dismiss_context_menu(previous_id);
            }
            let menu = platform
                .context_menu_position(&title, x, y)
                .and_then(|position| ContextMenuProcess::spawn(menu_id, items, position));
            match menu {
                Ok(menu) => context_menu = Some(menu),
                Err(error) => {
                    let _ = plugin.lock().unwrap().dismiss_context_menu(menu_id);
                    eprintln!("Unable to display native context menu: {error:#}");
                }
            }
        }
        if visible && closing.is_none() && snapshots.due(Instant::now()) {
            save_state(&plugin, &mut buffers, output, &mut snapshots, None, true)?;
        }
        if capture
            .as_ref()
            .is_some_and(|(_, ready)| Instant::now() >= *ready)
        {
            let (path, _) = capture.take().unwrap();
            let (width, height) = platform.capture(&title, &path)?;
            emit(
                output,
                &EditorEvent::Captured {
                    path,
                    width,
                    height,
                },
            )?;
        }
        if heartbeat.elapsed() >= Duration::from_millis(250) {
            emit(output, &EditorEvent::Heartbeat)?;
            heartbeat = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    if let Some(menu) = context_menu.take() {
        let menu_id = menu.menu_id();
        drop(menu);
        let _ = plugin.lock().unwrap().dismiss_context_menu(menu_id);
    }
    let (save_requested, request_id) = closing.expect("editor close was requested");
    if save_requested {
        save_state(
            &plugin,
            &mut buffers,
            output,
            &mut snapshots,
            request_id,
            false,
        )?;
    }
    window.close();
    plugin.lock().unwrap().stop_processing()?;
    emit(output, &EditorEvent::Closed { request_id })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_snapshots_debounce_gestures_and_limit_save_frequency() {
        let start = Instant::now();
        let mut schedule = SnapshotSchedule::new(start, b"initial");
        for millis in [100, 200, 600, 850] {
            schedule.edited(start + Duration::from_millis(millis));
            assert!(!schedule.due(start + Duration::from_millis(millis)));
        }
        assert!(!schedule.due(start + Duration::from_millis(1199)));
        assert!(schedule.due(start + Duration::from_millis(1200)));
        assert!(schedule.captured(start + Duration::from_millis(1200), b"edited"));
        schedule.edited(start + Duration::from_millis(1300));
        assert!(!schedule.due(start + Duration::from_millis(2199)));
        assert!(schedule.due(start + Duration::from_millis(2200)));
    }

    #[test]
    fn fallback_detects_opaque_changes_without_callbacks_and_suppresses_identical_bytes() {
        let start = Instant::now();
        let mut schedule = SnapshotSchedule::new(start, b"original preset");
        assert!(!schedule.due(start + Duration::from_millis(1999)));
        assert!(schedule.due(start + SNAPSHOT_FALLBACK));
        assert!(!schedule.captured(start + SNAPSHOT_FALLBACK, b"original preset"));
        let later = start + SNAPSHOT_FALLBACK * 2;
        assert!(schedule.due(later));
        assert!(schedule.captured(later, b"opaque preset edit"));
        assert!(!schedule.captured(later + SNAPSHOT_FALLBACK, b"opaque preset edit"));
    }
}
