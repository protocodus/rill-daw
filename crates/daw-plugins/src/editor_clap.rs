//! Native editor loop for formats that embed a view into Rill's own host window
//! (CLAP and, on macOS, Audio Units); the counterpart of the VST3 loop in
//! `editor_native`. It speaks the same editor protocol, so the owner cannot tell the
//! formats apart.
use super::{SnapshotSchedule, emit};
use crate::{
    ParameterInfo,
    clap::ClapProcessor,
    editor::{EditorCommand, EditorEvent},
    editor_platform::Platform,
    native::{
        LoadRequest, NativeProcessor, ParameterEdit, RESTART_IO_CHANGED,
        RESTART_PARAM_TITLES_CHANGED,
    },
};
use anyhow::{Result, ensure};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
use vst3_host::{BusAudioBuffers, HostWindow};

const BLOCK: usize = 256;
const SAMPLE_RATE: f64 = 48_000.0;

/// A processor whose editor embeds into a [`HostWindow`] and reports gestures.
pub(super) trait EmbeddedEditor: NativeProcessor + Sized {
    /// Suffix of the window title, such as "CLAP".
    const FORMAT: &'static str;
    fn load_editor(request: &LoadRequest<'_>) -> Result<Self>;
    fn has_gui(&self) -> bool;
    fn name(&self) -> &str;
    fn open_gui(&mut self, window: &HostWindow, title: &str) -> Result<(u32, u32)>;
    fn take_gui_resize(&mut self) -> Option<(u32, u32)>;
    fn resize_gui(&mut self, width: u32, height: u32);
    fn set_gui_scale(&mut self, scale: f32);
    fn gui_closed(&self) -> bool;
    fn close_gui(&mut self);
    fn take_edits(&mut self) -> Vec<ParameterEdit>;
    fn take_dirty(&mut self) -> bool;
}

impl EmbeddedEditor for ClapProcessor {
    const FORMAT: &'static str = "CLAP";
    fn load_editor(request: &LoadRequest<'_>) -> Result<Self> {
        ClapProcessor::load(request)
    }
    fn has_gui(&self) -> bool {
        ClapProcessor::has_gui(self)
    }
    fn name(&self) -> &str {
        ClapProcessor::name(self)
    }
    fn open_gui(&mut self, window: &HostWindow, title: &str) -> Result<(u32, u32)> {
        ClapProcessor::open_gui(self, parent(window), title)
    }
    fn take_gui_resize(&mut self) -> Option<(u32, u32)> {
        ClapProcessor::take_gui_resize(self)
    }
    fn resize_gui(&mut self, width: u32, height: u32) {
        ClapProcessor::resize_gui(self, width, height)
    }
    fn set_gui_scale(&mut self, scale: f32) {
        ClapProcessor::set_gui_scale(self, scale)
    }
    fn gui_closed(&self) -> bool {
        ClapProcessor::gui_closed(self)
    }
    fn close_gui(&mut self) {
        ClapProcessor::close_gui(self)
    }
    fn take_edits(&mut self) -> Vec<ParameterEdit> {
        ClapProcessor::take_edits(self)
    }
    fn take_dirty(&mut self) -> bool {
        ClapProcessor::take_dirty(self)
    }
}

/// Cocoa views (or the generic view) are embedded in the window's container view.
#[cfg(target_os = "macos")]
impl EmbeddedEditor for crate::au::AuProcessor {
    const FORMAT: &'static str = "AU";
    fn load_editor(request: &LoadRequest<'_>) -> Result<Self> {
        Self::load(request)
    }
    fn has_gui(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        Self::name(self)
    }
    fn open_gui(&mut self, window: &HostWindow, _title: &str) -> Result<(u32, u32)> {
        Self::open_gui(self, window.raw_handle())
    }
    fn take_gui_resize(&mut self) -> Option<(u32, u32)> {
        Self::take_gui_resize(self)
    }
    fn resize_gui(&mut self, width: u32, height: u32) {
        Self::resize_gui(self, width, height)
    }
    fn set_gui_scale(&mut self, _scale: f32) {}
    fn gui_closed(&self) -> bool {
        false
    }
    fn close_gui(&mut self) {
        Self::close_gui(self)
    }
    fn take_edits(&mut self) -> Vec<ParameterEdit> {
        Self::take_edits(self)
    }
    fn take_dirty(&mut self) -> bool {
        Self::take_dirty(self)
    }
}

fn parameters(plugin: &mut impl NativeProcessor) -> Result<Vec<ParameterInfo>> {
    Ok(plugin
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
        .collect())
}

fn automatable(plugin: &mut impl NativeProcessor, id: u32) -> Result<bool> {
    Ok(plugin
        .get_parameters()?
        .iter()
        .any(|p| p.id == id && p.can_automate && !p.is_read_only))
}

fn buffers(plugin: &impl NativeProcessor) -> Result<BusAudioBuffers> {
    Ok(BusAudioBuffers::new(
        &plugin.audio_bus_layout()?,
        BLOCK,
        SAMPLE_RATE,
    ))
}

fn save_state(
    plugin: &mut impl NativeProcessor,
    buffers: &mut BusAudioBuffers,
    output: &mut impl Write,
    snapshots: &mut SnapshotSchedule,
    request_id: Option<u64>,
    automatic: bool,
) -> Result<()> {
    // Apply staged edits before reading the state they belong to.
    plugin.process_bus_audio(buffers)?;
    let state = plugin.save_state()?;
    ensure!(
        state.len() <= 64 * 1024 * 1024,
        "Plugin state exceeds 64 MiB"
    );
    let changed = snapshots.captured(Instant::now(), &state);
    if !automatic || changed {
        emit(output, &EditorEvent::StateSaved { request_id, state })?;
        if changed {
            emit(
                output,
                &EditorEvent::Ready {
                    parameters: parameters(plugin)?,
                },
            )?;
        }
    }
    Ok(())
}

/// The host window is a CLAP editor's embedding parent.
fn parent(window: &HostWindow) -> clack_extensions::gui::Window<'static, 'static> {
    let raw = window.raw_handle();
    #[cfg(target_os = "macos")]
    // SAFETY: `raw` is the window's live container NSView; the GUI is destroyed first.
    return unsafe { clack_extensions::gui::Window::from_cocoa_nsview(raw) };
    #[cfg(target_os = "windows")]
    // SAFETY: `raw` is the window's live HWND; the GUI is destroyed first.
    return unsafe { clack_extensions::gui::Window::from_win32_hwnd(raw) };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    clack_extensions::gui::Window::from_x11_handle(raw as usize as _)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run<P: EmbeddedEditor>(
    output: &mut impl Write,
    commands: &mpsc::Receiver<EditorCommand>,
    platform: &Platform,
    path: &Path,
    class_id: Option<&str>,
    state: &[u8],
    initial: Vec<(u32, f64)>,
    visible: bool,
) -> Result<()> {
    let mut plugin = P::load_editor(&LoadRequest {
        path,
        class_id,
        sample_rate: SAMPLE_RATE,
        block_size: BLOCK,
        tempo: 120.0,
        time_signature: (4, 4),
    })?;
    ensure!(plugin.has_gui(), "This plug-in has no native editor");
    if !state.is_empty() {
        plugin.load_state(state)?;
    }
    for (id, value) in initial {
        ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "Invalid normalized parameter value"
        );
        plugin.set_parameter(id, value)?;
    }
    let mut buffers = buffers(&plugin)?;
    plugin.start_processing()?;
    let title = format!("{} - {}", plugin.name(), P::FORMAT);
    let window = HostWindow::create(&title, 640, 480)?;
    let (width, height) = plugin.open_gui(&window, &title)?;
    window.resize(width as i32, height as i32);
    window.show();
    platform.configure(&title, visible)?;
    emit(
        output,
        &EditorEvent::Ready {
            parameters: parameters(&mut plugin)?,
        },
    )?;
    let mut snapshots = SnapshotSchedule::new(Instant::now(), state);
    let mut heartbeat = Instant::now();
    let mut next_process = Instant::now();
    let mut capture: Option<(PathBuf, Instant)> = None;
    let mut closing = None;
    let mut picking = None;
    while closing.is_none() {
        for _ in 0..32 {
            match commands.try_recv() {
                Ok(EditorCommand::PickParameter { request_id }) => picking = request_id,
                Ok(EditorCommand::SetParameter { id, value }) => {
                    ensure!(
                        value.is_finite() && (0.0..=1.0).contains(&value),
                        "Invalid normalized parameter value"
                    );
                    plugin.set_parameter(id, value)?;
                    snapshots.edited(Instant::now());
                    emit(output, &EditorEvent::ParameterApplied { id, value })?;
                }
                Ok(EditorCommand::SaveState { request_id }) => save_state(
                    &mut plugin,
                    &mut buffers,
                    output,
                    &mut snapshots,
                    Some(request_id),
                    false,
                )?,
                #[cfg(debug_assertions)]
                Ok(EditorCommand::RestoreStateWithoutNotifications { state }) => {
                    plugin.load_state(&state)?;
                    let _ = plugin.take_edits();
                    let _ = plugin.take_dirty();
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
        platform.pump()?;
        if let Some(scale) = window.take_pending_scale_factor() {
            plugin.set_gui_scale(scale);
        }
        if closing.is_none()
            && visible
            && (window.closed_by_user() || platform.closed_by_user() || plugin.gui_closed())
        {
            closing = Some((true, None));
        }
        // Timers, fds and main-thread callbacks drive the editor between blocks.
        plugin.service();
        if let Some((width, height)) = plugin.take_gui_resize() {
            window.resize(width as i32, height as i32);
            plugin.resize_gui(width, height);
        }
        if Instant::now() >= next_process {
            plugin.process_bus_audio(&mut buffers)?;
            next_process = Instant::now() + Duration::from_millis(5);
        }
        if plugin.take_dirty() {
            snapshots.edited(Instant::now());
        }
        let mut automation_requests = Vec::new();
        for edit in plugin.take_edits() {
            match edit {
                ParameterEdit::Begin(id) | ParameterEdit::Value(id, _)
                    if picking.is_some()
                        && closing.is_none()
                        && visible
                        && automatable(&mut plugin, id)? =>
                {
                    automation_requests.push((id, picking.take()));
                }
                _ => {}
            }
            if let ParameterEdit::Value(id, value) = edit {
                snapshots.edited(Instant::now());
                if value.is_finite() && (0.0..=1.0).contains(&value) {
                    emit(output, &EditorEvent::ParameterChanged { id, value })?;
                }
            }
        }
        let flags = plugin.service_host_requests()?;
        if flags & RESTART_PARAM_TITLES_CHANGED != 0 {
            snapshots.edited(Instant::now());
            emit(
                output,
                &EditorEvent::Ready {
                    parameters: parameters(&mut plugin)?,
                },
            )?;
        }
        if flags & RESTART_IO_CHANGED != 0 {
            buffers = self::buffers(&plugin)?;
        }
        for (id, pick_request) in automation_requests {
            save_state(
                &mut plugin,
                &mut buffers,
                output,
                &mut snapshots,
                None,
                true,
            )?;
            if automatable(&mut plugin, id)?
                && let Ok(value) = plugin.get_parameter(id)
                && value.is_finite()
                && (0.0..=1.0).contains(&value)
            {
                emit(
                    output,
                    &EditorEvent::Ready {
                        parameters: parameters(&mut plugin)?,
                    },
                )?;
                emit(
                    output,
                    &match pick_request {
                        Some(request_id) => EditorEvent::ParameterPicked {
                            request_id,
                            id,
                            value,
                        },
                        None => EditorEvent::AutomateParameter { id, value },
                    },
                )?;
            }
        }
        if visible && closing.is_none() && snapshots.due(Instant::now()) {
            save_state(
                &mut plugin,
                &mut buffers,
                output,
                &mut snapshots,
                None,
                true,
            )?;
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
        save_state(
            &mut plugin,
            &mut buffers,
            output,
            &mut snapshots,
            request_id,
            false,
        )?;
    }
    // The plug-in's view must be gone before its parent window is destroyed.
    plugin.close_gui();
    drop(window);
    plugin.stop_processing()?;
    emit(output, &EditorEvent::Closed { request_id })?;
    Ok(())
}
