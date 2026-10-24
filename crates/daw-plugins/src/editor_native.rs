//! Native editor lifetime and IPC. This module runs only in the isolated helper.

use crate::editor_platform::Platform;
use crate::{
    ParameterInfo,
    editor::{EditorCommand, EditorEvent},
};
use anyhow::{Result, ensure};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use vst3_host::{
    BusAudioBuffers, ParameterEditKind, Plugin, PluginWindow, Vst3Host,
    process_isolation::ProtocolChannel,
};

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
    request_id: Option<u64>,
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
    emit(output, &EditorEvent::StateSaved { request_id, state })
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
    let parameters = plugin
        .get_parameters()?
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
        .collect();
    let mut buffers = plugin.create_bus_audio_buffers(256)?;
    plugin.start_processing()?;
    let title = format!("{} - VST3", plugin.info().name);
    let plugin = Arc::new(Mutex::new(plugin));
    let mut window = PluginWindow::new(Arc::clone(&plugin));
    window.open()?;
    platform.configure(&title, visible)?;
    emit(output, &EditorEvent::Ready { parameters })?;
    let mut heartbeat = Instant::now();
    let mut next_process = Instant::now();
    let mut capture: Option<(PathBuf, Instant)> = None;
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
                    emit(output, &EditorEvent::ParameterChanged { id, value })?;
                }
                Ok(EditorCommand::SaveState { request_id }) => {
                    save_state(&plugin, &mut buffers, output, Some(request_id))?
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
        {
            let mut plugin = plugin.lock().unwrap();
            plugin.service_run_loop();
            if Instant::now() >= next_process {
                plugin.process_bus_audio(&mut buffers)?;
                next_process = Instant::now() + Duration::from_millis(5);
            }
            // Only deliberate editor gestures should become "last touched". Generic
            // changes also contain processor meters/LFOs, so drain those separately.
            let _ = plugin.get_parameter_changes();
            for edit in plugin.take_parameter_edits() {
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
    let (save_requested, request_id) = closing.expect("editor close was requested");
    if save_requested {
        save_state(&plugin, &mut buffers, output, request_id)?;
    }
    window.close();
    plugin.lock().unwrap().stop_processing()?;
    emit(output, &EditorEvent::Closed { request_id })?;
    Ok(())
}
