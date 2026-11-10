//! CLAP hosting through `clack-host`, used only inside the isolated helper process.
//!
//! The helper's single thread is CLAP's main thread. Audio-thread entry points run on it
//! too, inside an explicit audio section that `thread-check` reports to the plug-in.
//! References: <https://github.com/free-audio/clap/tree/main/include/clap> and
//! <https://docs.rs/clack-host/0.2.0/clack_host/>.
use crate::PluginInfo;
use crate::native::{
    LoadRequest, LoadedInfo, NativeProcessor, RESTART_IO_CHANGED, RESTART_LATENCY_CHANGED,
    RESTART_PARAM_TITLES_CHANGED, RESTART_PARAM_VALUES_CHANGED, Transport, plain_from_normalized,
    plain_parameter,
};
use anyhow::{Context, Result, bail, ensure};
use clack_extensions::{
    audio_ports::{
        AudioPortFlags, AudioPortInfoBuffer, AudioPortRescanFlags, HostAudioPorts,
        HostAudioPortsImpl, PluginAudioPorts,
    },
    gui::{GuiConfiguration, GuiSize, HostGui, HostGuiImpl, PluginGui, Window},
    latency::{HostLatency, HostLatencyImpl, PluginLatency},
    log::{HostLog, HostLogImpl, LogSeverity},
    note_ports::{
        HostNotePorts, HostNotePortsImpl, NoteDialect, NoteDialects, NotePortInfoBuffer,
        NotePortRescanFlags, PluginNotePorts,
    },
    params::{
        HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags,
        ParamInfoBuffer, ParamInfoFlags, ParamRescanFlags, PluginParams,
    },
    render::{PluginRender, RenderMode},
    state::{HostState, HostStateImpl, PluginState},
    tail::{HostTail, HostTailImpl, PluginTail, TailLength},
    thread_check::{HostThreadCheck, HostThreadCheckImpl},
    timer::{HostTimer, HostTimerImpl, PluginTimer, TimerId},
};
use clack_host::events::event_types::{
    MidiEvent as ClapMidiEvent, NoteExpressionEvent, NoteExpressionType, NoteOffEvent, NoteOnEvent,
    ParamValueEvent, TransportEvent, TransportFlags,
};
use clack_host::events::spaces::CoreEventSpace;
use clack_host::events::{EventFlags, Match};
use clack_host::prelude::*;
use clack_host::utils::{BeatTime, SecondsTime};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ThreadId;
use std::time::{Duration, Instant};
use vst3_host::{
    AudioBusConfig, AudioBusLayout, BusAudioBuffers, MidiEvent,
    audio::{AudioBusDetails, AudioBusDetailsLayout, BusDirection, MediaType},
    midi::{NoteExpressionInfo, PluginEvent, PluginEventData},
    parameters::Parameter,
    plugin::{ProcessMode, TransportPosition},
};

/// Plug-in GUI gestures and value changes, drained by the editor helper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ParameterEdit {
    Begin(u32),
    Value(u32, f64),
}

struct ClapHost;

pub(crate) struct Shared {
    main_thread: ThreadId,
    audio_section: AtomicBool,
    restart: AtomicBool,
    callback: AtomicBool,
    flush: AtomicBool,
    tail_changed: AtomicBool,
    gui_resize: Mutex<Option<GuiSize>>,
    gui_closed: AtomicBool,
}

#[derive(Default)]
pub(crate) struct MainThread {
    timers: RefCell<Vec<(u32, Duration, Instant)>>,
    next_timer: Cell<u32>,
    #[cfg(unix)]
    fds: RefCell<Vec<(std::os::unix::io::RawFd, u32)>>,
    restart_bits: Cell<i32>,
    dirty: Cell<bool>,
}

pub(crate) struct AudioProcessor<'a> {
    shared: &'a Shared,
}

impl HostHandlers for ClapHost {
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread;
    type AudioProcessor<'a> = AudioProcessor<'a>;

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<HostLog>()
            .register::<HostThreadCheck>()
            .register::<HostParams>()
            .register::<HostState>()
            .register::<HostLatency>()
            .register::<HostTail>()
            .register::<HostAudioPorts>()
            .register::<HostNotePorts>()
            .register::<HostTimer>()
            .register::<HostGui>();
        #[cfg(unix)]
        builder.register::<clack_extensions::posix_fd::HostPosixFd>();
    }
}

impl SharedHandler<'_> for Shared {
    fn request_restart(&self) {
        self.restart.store(true, Ordering::Release);
    }
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback.store(true, Ordering::Release);
    }
}

impl MainThreadHandler<'_> for MainThread {}
impl<'a> AudioProcessorHandler<'a> for AudioProcessor<'a> {}

impl HostLogImpl for Shared {
    fn log(&self, severity: LogSeverity, message: &str) {
        eprintln!("CLAP {severity}: {message}");
    }
}

impl HostThreadCheckImpl for Shared {
    fn is_main_thread(&self) -> bool {
        std::thread::current().id() == self.main_thread
            && !self.audio_section.load(Ordering::Acquire)
    }
    fn is_audio_thread(&self) -> bool {
        std::thread::current().id() == self.main_thread
            && self.audio_section.load(Ordering::Acquire)
    }
}

impl HostParamsImplShared for Shared {
    fn request_flush(&self) {
        self.flush.store(true, Ordering::Release);
    }
}

impl HostParamsImplMainThread for MainThread {
    fn rescan(&self, flags: ParamRescanFlags) {
        let mut bits = RESTART_PARAM_VALUES_CHANGED;
        if flags.intersects(ParamRescanFlags::INFO | ParamRescanFlags::ALL | ParamRescanFlags::TEXT)
        {
            bits |= RESTART_PARAM_TITLES_CHANGED;
        }
        self.restart_bits.set(self.restart_bits.get() | bits);
    }
    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

impl HostStateImpl for MainThread {
    fn mark_dirty(&self) {
        self.dirty.set(true);
    }
}

impl HostLatencyImpl for MainThread {
    fn changed(&self) {
        self.restart_bits
            .set(self.restart_bits.get() | RESTART_LATENCY_CHANGED);
    }
}

impl HostTailImpl for AudioProcessor<'_> {
    fn changed(&mut self) {
        self.shared.tail_changed.store(true, Ordering::Release);
    }
}

impl HostAudioPortsImpl for MainThread {
    fn is_rescan_flag_supported(&self, _flag: AudioPortRescanFlags) -> bool {
        true
    }
    fn rescan(&self, _flags: AudioPortRescanFlags) {
        self.restart_bits
            .set(self.restart_bits.get() | RESTART_IO_CHANGED);
    }
}

impl HostNotePortsImpl for MainThread {
    fn supported_dialects(&self) -> NoteDialects {
        NoteDialects::CLAP | NoteDialects::MIDI
    }
    fn rescan(&self, _flags: NotePortRescanFlags) {
        self.restart_bits
            .set(self.restart_bits.get() | RESTART_IO_CHANGED);
    }
}

impl HostTimerImpl for MainThread {
    fn register_timer(&self, period_ms: u32) -> Result<TimerId, HostError> {
        let id = self.next_timer.get();
        self.next_timer.set(id.wrapping_add(1) % (u32::MAX - 1));
        // CLAP lets the host choose the effective period; keep GUIs responsive and bounded.
        let period = Duration::from_millis(u64::from(period_ms.clamp(10, 10_000)));
        self.timers
            .borrow_mut()
            .push((id, period, Instant::now() + period));
        Ok(TimerId(id))
    }
    fn unregister_timer(&self, timer_id: TimerId) -> Result<(), HostError> {
        let mut timers = self.timers.borrow_mut();
        let before = timers.len();
        timers.retain(|(id, ..)| *id != timer_id.0);
        if timers.len() == before {
            return Err(HostError::Message("Unknown timer"));
        }
        Ok(())
    }
}

#[cfg(unix)]
impl clack_extensions::posix_fd::HostPosixFdImpl for MainThread {
    fn register_fd(
        &self,
        fd: std::os::unix::io::RawFd,
        flags: clack_extensions::posix_fd::FdFlags,
    ) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        if fds.iter().any(|(existing, _)| *existing == fd) {
            return Err(HostError::Message("File descriptor already registered"));
        }
        fds.push((fd, flags.bits()));
        Ok(())
    }
    fn modify_fd(
        &self,
        fd: std::os::unix::io::RawFd,
        flags: clack_extensions::posix_fd::FdFlags,
    ) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        let entry = fds
            .iter_mut()
            .find(|(existing, _)| *existing == fd)
            .ok_or(HostError::Message("Unknown file descriptor"))?;
        entry.1 = flags.bits();
        Ok(())
    }
    fn unregister_fd(&self, fd: std::os::unix::io::RawFd) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        let before = fds.len();
        fds.retain(|(existing, _)| *existing != fd);
        if fds.len() == before {
            return Err(HostError::Message("Unknown file descriptor"));
        }
        Ok(())
    }
}

impl HostGuiImpl for Shared {
    fn resize_hints_changed(&self) {}
    fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
        *self.gui_resize.lock().unwrap_or_else(|p| p.into_inner()) = Some(new_size);
        Ok(())
    }
    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn closed(&self, _was_destroyed: bool) {
        self.gui_closed.store(true, Ordering::Release);
    }
}

fn host_info() -> Result<HostInfo> {
    Ok(HostInfo::new(
        "Rill",
        "Rill",
        "https://github.com/protocodus",
        env!("CARGO_PKG_VERSION"),
    )?)
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn cstr(value: Option<&CStr>) -> String {
    value
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Map CLAP features onto Rill's VST3-style category list.
fn category(features: &[String]) -> String {
    let mut parts = Vec::new();
    if features.iter().any(|f| f == "instrument") {
        parts.push("Instrument".to_owned());
    }
    if features
        .iter()
        .any(|f| f == "audio-effect" || f == "analyzer")
    {
        parts.push("Fx".to_owned());
    }
    parts.extend(
        features
            .iter()
            .filter(|f| !matches!(f.as_str(), "instrument" | "audio-effect"))
            .cloned(),
    );
    parts.join("|")
}

/// Run on the helper only: initializing the entry executes plug-in code.
pub(crate) fn scan(path: &Path) -> Result<Vec<PluginInfo>> {
    // SAFETY: loading third-party code is this isolated process's purpose; the parent
    // contains crashes and hangs.
    let entry = unsafe { PluginEntry::load(path) }
        .map_err(|error| anyhow::anyhow!("{error}"))
        .with_context(|| format!("Loading CLAP entry {}", path.display()))?;
    let factory = entry
        .get_plugin_factory()
        .context("CLAP entry has no plug-in factory")?;
    let architectures = vec![std::env::consts::ARCH.to_owned()];
    Ok(factory
        .plugin_descriptors()
        .filter_map(|descriptor| {
            let id = descriptor.id()?.to_string_lossy().into_owned();
            let features: Vec<String> = descriptor
                .features()
                .map(|f| f.to_string_lossy().into_owned())
                .collect();
            Some(PluginInfo {
                name: cstr(descriptor.name()),
                path: path.to_owned(),
                vendor: cstr(descriptor.vendor()),
                category: category(&features),
                class_id: Some(id),
                version: cstr(descriptor.version()),
                architectures: architectures.clone(),
                format: crate::PluginFormat::Clap,
            })
        })
        .collect())
}

struct ParamMeta {
    id: u32,
    name: String,
    min: f64,
    max: f64,
    default: f64,
    flags: ParamInfoFlags,
}

impl ParamMeta {
    fn stepped(&self) -> bool {
        self.flags.contains(ParamInfoFlags::IS_STEPPED)
    }
}

struct Port {
    name: String,
    channels: usize,
    main: bool,
    active: bool,
}

#[derive(Clone, Copy)]
enum Staged {
    NoteOn {
        key: i16,
        channel: i16,
        id: i32,
        velocity: f64,
    },
    NoteOff {
        key: i16,
        channel: i16,
        id: i32,
        velocity: f64,
    },
    Expression {
        key: i16,
        channel: i16,
        id: i32,
        kind: NoteExpressionType,
        value: f64,
    },
    Param {
        id: u32,
        value: f64,
    },
    Midi([u8; 3]),
}

fn note_pckn(key: i16, channel: i16, id: i32) -> Pckn {
    let wildcard16 = |value: i16| u16::try_from(value).map_or(Match::All, Match::Specific);
    Pckn::new(
        0u16,
        wildcard16(channel),
        wildcard16(key),
        u32::try_from(id).map_or(Match::All, Match::Specific),
    )
}

pub(crate) struct ClapProcessor {
    instance: PluginInstance<ClapHost>,
    processor: Option<PluginAudioProcessor<ClapHost>>,
    info: LoadedInfo,
    sample_rate: f64,
    max_block: usize,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
    params: Vec<ParamMeta>,
    clap_notes: bool,
    midi_notes: bool,
    latency: u32,
    tail: u32,
    transport: Transport,
    staged: Vec<(u32, Staged)>,
    events: EventBuffer,
    output_events: EventBuffer,
    edits: Vec<ParameterEdit>,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    steady_time: u64,
    render_mode: RenderMode,
}

impl ClapProcessor {
    pub(crate) fn load(request: &LoadRequest<'_>) -> Result<Self> {
        ensure!(
            request.sample_rate.is_finite() && request.sample_rate > 0.0,
            "Invalid sample rate"
        );
        // SAFETY: see `scan`.
        let entry = unsafe { PluginEntry::load(request.path) }
            .map_err(|error| anyhow::anyhow!("{error}"))
            .with_context(|| format!("Loading CLAP entry {}", request.path.display()))?;
        let factory = entry
            .get_plugin_factory()
            .context("CLAP entry has no plug-in factory")?;
        let descriptor = match request.class_id {
            Some(id) => factory.plugin_descriptors().find(|d| {
                d.id()
                    .is_some_and(|candidate| candidate.to_bytes() == id.as_bytes())
            }),
            None => factory.plugin_descriptors().find(|d| d.id().is_some()),
        }
        .with_context(|| {
            format!(
                "CLAP plug-in {} is not in {}",
                request.class_id.unwrap_or("(first)"),
                request.path.display()
            )
        })?;
        let id = CString::from(descriptor.id().context("CLAP plug-in has no ID")?);
        let features: Vec<String> = descriptor
            .features()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        let mut info = LoadedInfo {
            name: cstr(descriptor.name()),
            vendor: cstr(descriptor.vendor()),
            version: cstr(descriptor.version()),
            category: category(&features),
            uid: id.to_string_lossy().into_owned(),
            has_gui: false,
            has_midi_input: false,
            has_midi_output: false,
        };
        let main_thread = std::thread::current().id();
        let mut instance = PluginInstance::<ClapHost>::new(
            |_| Shared {
                main_thread,
                audio_section: AtomicBool::new(false),
                restart: AtomicBool::new(false),
                callback: AtomicBool::new(false),
                flush: AtomicBool::new(false),
                tail_changed: AtomicBool::new(false),
                gui_resize: Mutex::new(None),
                gui_closed: AtomicBool::new(false),
            },
            |_| MainThread::default(),
            &entry,
            &id,
            &host_info()?,
        )
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("Instantiating CLAP plug-in")?;
        let handle = instance.plugin_handle();
        info.has_gui = handle.get_extension::<PluginGui>().is_some();
        let mut processor = Self {
            instance,
            processor: None,
            info,
            sample_rate: request.sample_rate,
            max_block: request.block_size,
            inputs: Vec::new(),
            outputs: Vec::new(),
            params: Vec::new(),
            clap_notes: false,
            midi_notes: false,
            latency: 0,
            tail: 0,
            transport: Transport::new(request.tempo, request.time_signature),
            staged: Vec::with_capacity(4096),
            events: EventBuffer::with_capacity(4096),
            output_events: EventBuffer::with_capacity(1024),
            edits: Vec::new(),
            input_ports: AudioPorts::with_capacity(0, 0),
            output_ports: AudioPorts::with_capacity(0, 0),
            steady_time: 0,
            render_mode: RenderMode::Realtime,
        };
        processor.scan_ports();
        processor.scan_params();
        Ok(processor)
    }

    fn scan_ports(&mut self) {
        let handle = self.instance.plugin_handle();
        let scan_audio = |is_input: bool| -> Vec<Port> {
            let Some(ports) = handle.get_extension::<PluginAudioPorts>() else {
                return Vec::new();
            };
            let mut buffer = AudioPortInfoBuffer::new();
            (0..ports.count(&handle, is_input).min(64))
                .filter_map(|index| {
                    let port = ports.get(&handle, index, is_input, &mut buffer)?;
                    Some(Port {
                        name: text(port.name),
                        channels: port.channel_count.min(256) as usize,
                        main: port.flags.contains(AudioPortFlags::IS_MAIN),
                        active: true,
                    })
                })
                .collect()
        };
        let inputs = scan_audio(true);
        let outputs = scan_audio(false);
        let (mut clap_notes, mut midi_notes, mut note_outputs) = (false, false, false);
        if let Some(notes) = handle.get_extension::<PluginNotePorts>() {
            let mut buffer = NotePortInfoBuffer::new();
            if notes.count(&handle, true) > 0
                && let Some(port) = notes.get(&handle, 0, true, &mut buffer)
            {
                clap_notes = port.supported_dialects.supports(NoteDialect::Clap);
                midi_notes = port.supported_dialects.supports(NoteDialect::Midi);
            }
            note_outputs = notes.count(&handle, false) > 0;
        }
        self.inputs = inputs;
        self.outputs = outputs;
        self.clap_notes = clap_notes;
        self.midi_notes = midi_notes;
        self.info.has_midi_input = clap_notes || midi_notes;
        self.info.has_midi_output = note_outputs;
        let channels = |ports: &[Port]| ports.iter().map(|p| p.channels).sum::<usize>();
        self.input_ports = AudioPorts::with_capacity(channels(&self.inputs), self.inputs.len());
        self.output_ports = AudioPorts::with_capacity(channels(&self.outputs), self.outputs.len());
    }

    fn scan_params(&mut self) {
        let handle = self.instance.plugin_handle();
        let Some(params) = handle.get_extension::<PluginParams>() else {
            self.params.clear();
            return;
        };
        let mut buffer = ParamInfoBuffer::new();
        self.params = (0..params.count(&handle).min(65_536))
            .filter_map(|index| {
                let info = params.get_info(&handle, index, &mut buffer)?;
                (info.min_value.is_finite() && info.max_value.is_finite()).then(|| ParamMeta {
                    id: info.id.get(),
                    name: text(info.name),
                    min: info.min_value,
                    max: info.max_value,
                    default: info.default_value,
                    flags: info.flags,
                })
            })
            .collect();
    }

    fn param(&self, id: u32) -> Result<&ParamMeta> {
        self.params
            .iter()
            .find(|p| p.id == id)
            .with_context(|| format!("Unknown CLAP parameter {id}"))
    }

    fn shared<R>(&self, access: impl FnOnce(&Shared) -> R) -> R {
        self.instance.access_shared_handler(access)
    }

    fn audio_section<R>(&mut self, call: impl FnOnce(&mut Self) -> R) -> R {
        self.shared(|s| s.audio_section.store(true, Ordering::Release));
        let result = call(self);
        self.shared(|s| s.audio_section.store(false, Ordering::Release));
        result
    }

    fn activate(&mut self) -> Result<()> {
        if self.processor.is_some() {
            return Ok(());
        }
        let configuration = PluginAudioConfiguration {
            sample_rate: self.sample_rate,
            min_frames_count: 1,
            max_frames_count: self.max_block as u32,
        };
        let stopped = self
            .instance
            .activate(|shared, _| AudioProcessor { shared }, configuration)
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("Activating CLAP plug-in")?;
        self.processor = Some(stopped.into());
        let handle = self.instance.plugin_handle();
        self.latency = handle
            .get_extension::<PluginLatency>()
            .map_or(0, |latency| latency.get(&handle));
        if let Some(render) = handle.get_extension::<PluginRender>() {
            let _ = render.set(&handle, self.render_mode);
        }
        self.refresh_tail();
        Ok(())
    }

    fn refresh_tail(&mut self) {
        let Some(processor) = self.processor.as_mut() else {
            return;
        };
        let shared = processor.shared_plugin_handle();
        let Some(tail) = shared.get_extension::<PluginTail>() else {
            return;
        };
        self.tail = match tail.get(&processor.plugin_handle()) {
            TailLength::Finite(samples) => samples,
            TailLength::Infinite => u32::MAX,
        };
    }

    fn deactivate(&mut self) {
        if let Some(processor) = self.processor.take() {
            let stopped = self.audio_section(|_| processor.into_stopped());
            self.instance.deactivate(stopped);
        }
    }

    /// Service plug-in requests that must run on the main thread: callbacks,
    /// timers, file descriptors, parameter flushes and restarts.
    fn service_main_thread(&mut self) {
        if self.shared(|s| s.callback.swap(false, Ordering::AcqRel)) {
            self.instance.call_on_main_thread_callback();
        }
        let now = Instant::now();
        let due: Vec<u32> = self.instance.access_handler(|main| {
            main.timers
                .borrow_mut()
                .iter_mut()
                .filter(|(_, _, next)| *next <= now)
                .map(|(id, period, next)| {
                    *next = now + *period;
                    *id
                })
                .collect()
        });
        if !due.is_empty() {
            let handle = self.instance.plugin_handle();
            if let Some(timer) = handle.get_extension::<PluginTimer>() {
                for id in due {
                    timer.on_timer(&handle, TimerId(id));
                }
            }
        }
        #[cfg(unix)]
        self.service_fds();
        if self.shared(|s| s.flush.swap(false, Ordering::AcqRel))
            && !self.processor.as_ref().is_some_and(|p| p.is_started())
        {
            let _ = self.flush(&[]);
        }
        if self.shared(|s| s.restart.swap(false, Ordering::AcqRel)) && self.processor.is_some() {
            let started = self.processor.as_ref().is_some_and(|p| p.is_started());
            self.deactivate();
            let restarted = self.activate().and_then(|()| {
                if started {
                    self.start_processing()
                } else {
                    Ok(())
                }
            });
            if let Err(error) = restarted {
                eprintln!("CLAP restart failed: {error:#}");
            }
            self.mark(RESTART_LATENCY_CHANGED);
        }
        if self.shared(|s| s.tail_changed.swap(false, Ordering::AcqRel)) {
            self.refresh_tail();
        }
    }

    #[cfg(unix)]
    fn service_fds(&mut self) {
        let fds: Vec<(i32, u32)> = self
            .instance
            .access_handler(|main| main.fds.borrow().clone());
        if fds.is_empty() {
            return;
        }
        let mut polls: Vec<libc::pollfd> = fds
            .iter()
            .map(|&(fd, flags)| libc::pollfd {
                fd,
                events: (if flags & 1 != 0 { libc::POLLIN } else { 0 })
                    | (if flags & 2 != 0 { libc::POLLOUT } else { 0 }),
                revents: 0,
            })
            .collect();
        // SAFETY: `polls` is a valid, exclusively borrowed array for the call.
        let ready = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 0) };
        if ready <= 0 {
            return;
        }
        let handle = self.instance.plugin_handle();
        let Some(posix) = handle.get_extension::<clack_extensions::posix_fd::PluginPosixFd>()
        else {
            return;
        };
        for poll in polls.iter().filter(|poll| poll.revents != 0) {
            let mut flags = 0;
            if poll.revents & libc::POLLIN != 0 {
                flags |= 1;
            }
            if poll.revents & libc::POLLOUT != 0 {
                flags |= 2;
            }
            if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                flags |= 4;
            }
            posix.on_fd(
                &handle,
                poll.fd,
                clack_extensions::posix_fd::FdFlags::from_bits_truncate(flags),
            );
        }
    }

    fn mark(&self, bits: i32) {
        self.instance
            .access_handler(|main| main.restart_bits.set(main.restart_bits.get() | bits));
    }

    /// Deliver parameter values outside `process`, as CLAP's `params.flush` requires.
    fn flush(&mut self, values: &[(u32, f64)]) -> Result<()> {
        self.events.clear();
        for &(id, value) in values {
            let id = ClapId::from_raw(id).context("Invalid CLAP parameter ID")?;
            self.events
                .push(&ParamValueEvent::new(0, id, Pckn::match_all(), value));
        }
        self.output_events.clear();
        let input = self.events.as_input();
        let mut output = self.output_events.as_output();
        if let Some(processor) = self.processor.as_mut() {
            let shared = processor.shared_plugin_handle();
            if let Some(params) = shared.get_extension::<PluginParams>() {
                let instance = &self.instance;
                instance.access_shared_handler(|s| s.audio_section.store(true, Ordering::Release));
                params.flush_active(&mut processor.plugin_handle(), &input, &mut output);
                instance.access_shared_handler(|s| s.audio_section.store(false, Ordering::Release));
            }
        } else if let Some(mut handle) = self.instance.inactive_plugin_handle()
            && let Some(params) = handle.get_extension::<PluginParams>()
        {
            params.flush(&mut handle, &input, &mut output);
        }
        self.events.clear();
        self.collect_output_edits();
        Ok(())
    }

    fn collect_output_edits(&mut self) {
        for event in self.output_events.iter() {
            match event.as_core_event() {
                Some(CoreEventSpace::ParamValue(value)) => {
                    if let (Some(id), Ok(meta)) = (
                        value.param_id(),
                        self.param(value.param_id().map_or(u32::MAX, |i| i.get())),
                    ) {
                        let normalized =
                            crate::native::normalized_from_plain(meta.min, meta.max, value.value());
                        self.edits.push(ParameterEdit::Value(id.get(), normalized));
                    }
                }
                Some(CoreEventSpace::ParamGestureBegin(gesture)) => {
                    if let Some(id) = gesture.param_id() {
                        self.edits.push(ParameterEdit::Begin(id.get()));
                    }
                }
                _ => {}
            }
        }
        self.output_events.clear();
    }

    pub(crate) fn take_edits(&mut self) -> Vec<ParameterEdit> {
        std::mem::take(&mut self.edits)
    }

    pub(crate) fn take_dirty(&mut self) -> bool {
        self.instance
            .access_handler(|main| main.dirty.replace(false))
    }

    pub(crate) fn has_gui(&self) -> bool {
        self.info.has_gui
    }

    pub(crate) fn name(&self) -> &str {
        &self.info.name
    }

    fn stage_note(&mut self, offset: i32, note: Staged) -> Result<()> {
        ensure!(
            self.staged.len() < 16_384,
            "Too many CLAP events in one block"
        );
        let time = u32::try_from(offset).context("Negative CLAP event offset")?;
        let note = if self.clap_notes {
            note
        } else if self.midi_notes {
            match note {
                Staged::NoteOn {
                    key,
                    channel,
                    velocity,
                    ..
                } => Staged::Midi([
                    0x90 | (channel.clamp(0, 15) as u8),
                    key.clamp(0, 127) as u8,
                    ((velocity * 127.0).round() as u8).clamp(1, 127),
                ]),
                Staged::NoteOff {
                    key,
                    channel,
                    velocity,
                    ..
                } => Staged::Midi([
                    0x80 | (channel.clamp(0, 15) as u8),
                    key.clamp(0, 127) as u8,
                    ((velocity * 127.0).round() as u8).min(127),
                ]),
                Staged::Expression {
                    kind: NoteExpressionType::Pressure,
                    key,
                    channel,
                    value,
                    ..
                } if key >= 0 => Staged::Midi([
                    0xA0 | (channel.clamp(0, 15) as u8),
                    key.clamp(0, 127) as u8,
                    ((value * 127.0).round() as u8).min(127),
                ]),
                Staged::Expression { .. } => return Ok(()),
                other => other,
            }
        } else {
            return Ok(());
        };
        self.staged.push((time, note));
        Ok(())
    }

    fn transport_event(&self) -> TransportEvent {
        let t = &self.transport;
        let mut flags = TransportFlags::HAS_TEMPO
            | TransportFlags::HAS_BEATS_TIMELINE
            | TransportFlags::HAS_SECONDS_TIMELINE
            | TransportFlags::HAS_TIME_SIGNATURE;
        if t.playing {
            flags |= TransportFlags::IS_PLAYING;
        }
        let beats_per_bar = f64::from(t.numerator.max(1)) * 4.0 / f64::from(t.denominator.max(1));
        let bar = (t.position.quarter_notes / beats_per_bar).floor();
        TransportEvent {
            header: EventHeader::new_core(0, EventFlags::empty()),
            flags,
            song_pos_beats: BeatTime::from_float(t.position.quarter_notes),
            song_pos_seconds: SecondsTime::from_float(t.position.samples as f64 / self.sample_rate),
            tempo: t.tempo,
            tempo_inc: 0.0,
            loop_start_beats: BeatTime::from_int(0),
            loop_end_beats: BeatTime::from_int(0),
            loop_start_seconds: SecondsTime::from_int(0),
            loop_end_seconds: SecondsTime::from_int(0),
            bar_start: BeatTime::from_float(bar * beats_per_bar),
            bar_number: bar as i32,
            time_signature_numerator: t.numerator.clamp(1, i32::from(u16::MAX)) as u16,
            time_signature_denominator: t.denominator.clamp(1, i32::from(u16::MAX)) as u16,
        }
    }

    fn fill_events(&mut self) {
        self.staged.sort_by_key(|(time, _)| *time);
        self.events.clear();
        for &(time, event) in &self.staged {
            match event {
                Staged::NoteOn {
                    key,
                    channel,
                    id,
                    velocity,
                } => self.events.push(&NoteOnEvent::new(
                    time,
                    note_pckn(key, channel, id),
                    velocity,
                )),
                Staged::NoteOff {
                    key,
                    channel,
                    id,
                    velocity,
                } => self.events.push(&NoteOffEvent::new(
                    time,
                    note_pckn(key, channel, id),
                    velocity,
                )),
                Staged::Expression {
                    key,
                    channel,
                    id,
                    kind,
                    value,
                } => self.events.push(&NoteExpressionEvent::new(
                    time,
                    note_pckn(key, channel, id),
                    kind,
                    value,
                )),
                Staged::Param { id, value } => {
                    if let Some(id) = ClapId::from_raw(id) {
                        self.events
                            .push(&ParamValueEvent::new(time, id, Pckn::match_all(), value));
                    }
                }
                Staged::Midi(data) => self.events.push(&ClapMidiEvent::new(time, 0, data)),
            }
        }
        self.staged.clear();
    }

    /// Embed the editor into a native parent window owned by the editor helper.
    pub(crate) fn open_gui(&mut self, parent: Window<'_, '_>, title: &str) -> Result<(u32, u32)> {
        let handle = self.instance.plugin_handle();
        let gui = handle
            .get_extension::<PluginGui>()
            .context("This plug-in has no native editor")?;
        let configuration = GuiConfiguration {
            api_type: parent.api_type(),
            is_floating: false,
        };
        ensure!(
            gui.is_api_supported(&handle, configuration),
            "This plug-in cannot embed its editor on this platform"
        );
        gui.create(&handle, configuration)
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("Creating CLAP editor")?;
        let title = CString::new(title.replace('\0', "")).unwrap_or_default();
        gui.suggest_title(&handle, &title);
        let size = gui.get_size(&handle).unwrap_or(GuiSize {
            width: 640,
            height: 480,
        });
        // SAFETY: the parent window outlives the GUI; `close_gui` destroys it first.
        unsafe { gui.set_parent(&handle, parent) }
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("Embedding CLAP editor")?;
        let _ = gui.show(&handle);
        Ok((size.width.clamp(64, 8192), size.height.clamp(64, 8192)))
    }

    pub(crate) fn take_gui_resize(&self) -> Option<(u32, u32)> {
        self.shared(|s| {
            s.gui_resize
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
        })
        .map(|size| (size.width.clamp(64, 8192), size.height.clamp(64, 8192)))
    }

    /// Apply an accepted host resize so the plug-in's view follows its window.
    pub(crate) fn resize_gui(&mut self, width: u32, height: u32) {
        let handle = self.instance.plugin_handle();
        if let Some(gui) = handle.get_extension::<PluginGui>() {
            let _ = gui.set_size(&handle, GuiSize { width, height });
        }
    }

    pub(crate) fn set_gui_scale(&mut self, scale: f32) {
        let handle = self.instance.plugin_handle();
        if let Some(gui) = handle.get_extension::<PluginGui>() {
            let _ = gui.set_scale(&handle, f64::from(scale));
        }
    }

    pub(crate) fn gui_closed(&self) -> bool {
        self.shared(|s| s.gui_closed.load(Ordering::Acquire))
    }

    pub(crate) fn close_gui(&mut self) {
        let handle = self.instance.plugin_handle();
        if let Some(gui) = handle.get_extension::<PluginGui>() {
            gui.destroy(&handle);
        }
    }
}

impl Drop for ClapProcessor {
    fn drop(&mut self) {
        self.deactivate();
    }
}

impl NativeProcessor for ClapProcessor {
    fn loaded_info(&self) -> LoadedInfo {
        LoadedInfo {
            name: self.info.name.clone(),
            vendor: self.info.vendor.clone(),
            version: self.info.version.clone(),
            category: self.info.category.clone(),
            uid: self.info.uid.clone(),
            has_gui: self.info.has_gui,
            has_midi_input: self.info.has_midi_input,
            has_midi_output: self.info.has_midi_output,
        }
    }

    fn start_processing(&mut self) -> Result<()> {
        self.activate()?;
        let processor = self
            .processor
            .take()
            .context("CLAP plug-in is not active")?;
        let started = self.audio_section(|_| processor.into_started());
        match started {
            Ok(started) => {
                self.processor = Some(started.into());
                Ok(())
            }
            Err(error) => {
                self.processor = Some(error.into_stopped_processor().into());
                bail!("CLAP plug-in failed to start processing")
            }
        }
    }

    fn stop_processing(&mut self) -> Result<()> {
        self.deactivate();
        Ok(())
    }

    fn is_processing(&self) -> bool {
        self.processor.as_ref().is_some_and(|p| p.is_started())
    }

    fn reconfigure(&mut self, sample_rate: f64, block_size: usize) -> Result<()> {
        ensure!(
            sample_rate.is_finite() && sample_rate > 0.0 && block_size > 0,
            "Invalid CLAP processing configuration"
        );
        let started = self.is_processing();
        self.deactivate();
        self.sample_rate = sample_rate;
        self.max_block = block_size;
        if started {
            self.start_processing()?;
        }
        Ok(())
    }

    fn set_process_mode(&mut self, mode: ProcessMode) -> Result<()> {
        self.render_mode = match mode {
            ProcessMode::Offline => RenderMode::Offline,
            _ => RenderMode::Realtime,
        };
        let handle = self.instance.plugin_handle();
        if let Some(render) = handle.get_extension::<PluginRender>() {
            render
                .set(&handle, self.render_mode)
                .map_err(|_| anyhow::anyhow!("CLAP plug-in rejected the render mode"))?;
        }
        Ok(())
    }

    fn service(&mut self) {
        self.service_main_thread();
    }

    fn service_host_requests(&mut self) -> Result<i32> {
        self.service_main_thread();
        let bits = self
            .instance
            .access_handler(|main| main.restart_bits.replace(0));
        if bits & RESTART_PARAM_TITLES_CHANGED != 0 {
            self.scan_params();
        }
        if bits & RESTART_IO_CHANGED != 0 && self.processor.is_none() {
            let activation: Vec<_> = self
                .outputs
                .iter()
                .chain(&self.inputs)
                .map(|p| p.active)
                .collect();
            self.scan_ports();
            if activation.len() == self.outputs.len() + self.inputs.len() {
                for (port, active) in self
                    .outputs
                    .iter_mut()
                    .chain(&mut self.inputs)
                    .zip(activation)
                {
                    port.active = active;
                }
            }
        }
        Ok(bits)
    }

    fn audio_bus_layout(&self) -> Result<AudioBusLayout> {
        let config = |ports: &[Port]| {
            ports
                .iter()
                .map(|p| AudioBusConfig {
                    channel_count: p.channels,
                    active: p.active,
                })
                .collect()
        };
        Ok(AudioBusLayout {
            inputs: config(&self.inputs),
            outputs: config(&self.outputs),
        })
    }

    fn audio_bus_details(&self) -> Result<AudioBusDetailsLayout> {
        let details = |ports: &[Port], direction: &str| {
            ports
                .iter()
                .enumerate()
                .map(|(index, p)| AudioBusDetails {
                    name: if p.name.is_empty() {
                        format!("{direction} {}", index + 1)
                    } else {
                        p.name.clone()
                    },
                    channels: p.channels,
                    active: p.active,
                    main: p.main,
                })
                .collect()
        };
        Ok(AudioBusDetailsLayout {
            inputs: details(&self.inputs, "Input"),
            outputs: details(&self.outputs, "Output"),
        })
    }

    /// CLAP ports are always connected; inactive Rill buses receive silence and
    /// their outputs are discarded.
    fn set_bus_active(
        &mut self,
        media: MediaType,
        direction: BusDirection,
        index: i32,
        active: bool,
    ) -> Result<()> {
        if media == MediaType::Event {
            return Ok(());
        }
        let ports = match direction {
            BusDirection::Input => &mut self.inputs,
            BusDirection::Output => &mut self.outputs,
        };
        let port = usize::try_from(index)
            .ok()
            .and_then(|index| ports.get_mut(index))
            .context("Invalid CLAP audio port index")?;
        port.active = active;
        Ok(())
    }

    fn set_transport_position(&mut self, position: TransportPosition) -> Result<()> {
        ensure!(
            position.quarter_notes.is_finite(),
            "Invalid transport position"
        );
        self.transport.position = position;
        Ok(())
    }
    fn transport_position(&self) -> Result<TransportPosition> {
        Ok(self.transport.position)
    }
    fn set_tempo(&mut self, bpm: f64) -> Result<()> {
        Transport::validate_tempo(bpm)?;
        self.transport.tempo = bpm;
        Ok(())
    }
    fn set_playing(&mut self, playing: bool) -> Result<()> {
        self.transport.playing = playing;
        Ok(())
    }
    fn set_time_signature(&mut self, numerator: i32, denominator: i32) -> Result<()> {
        ensure!(numerator > 0 && denominator > 0, "Invalid time signature");
        self.transport.numerator = numerator;
        self.transport.denominator = denominator;
        Ok(())
    }

    fn get_parameters(&mut self) -> Result<Vec<Parameter>> {
        let visible: Vec<_> = self
            .params
            .iter()
            .filter(|meta| !meta.flags.contains(ParamInfoFlags::IS_HIDDEN))
            .map(|meta| {
                (
                    meta.id,
                    meta.name.clone(),
                    meta.min,
                    meta.max,
                    meta.default,
                    meta.flags,
                )
            })
            .collect();
        Ok(visible
            .into_iter()
            .map(|(id, name, min, max, default, flags)| {
                let value = self.plain_value(id).unwrap_or(default);
                plain_parameter(
                    id,
                    name,
                    String::new(),
                    min,
                    max,
                    default,
                    value,
                    flags.contains(ParamInfoFlags::IS_STEPPED),
                    flags.contains(ParamInfoFlags::IS_AUTOMATABLE),
                    flags.contains(ParamInfoFlags::IS_READONLY),
                    flags.contains(ParamInfoFlags::IS_BYPASS),
                )
            })
            .collect())
    }

    fn get_parameter(&mut self, id: u32) -> Result<f64> {
        let (min, max) = self.param(id).map(|meta| (meta.min, meta.max))?;
        let plain = self
            .plain_value(id)
            .context("CLAP plug-in did not report a parameter value")?;
        Ok(crate::native::normalized_from_plain(min, max, plain))
    }

    fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        ensure!(value.is_finite(), "Invalid parameter value");
        let meta = self.param(id)?;
        let plain = plain_from_normalized(meta.min, meta.max, meta.stepped(), value);
        self.flush(&[(id, plain)])
    }

    fn set_parameter_at(&mut self, id: u32, value: f64, offset: i32) -> Result<()> {
        ensure!(value.is_finite(), "Invalid parameter value");
        let meta = self.param(id)?;
        let plain = plain_from_normalized(meta.min, meta.max, meta.stepped(), value);
        let time = u32::try_from(offset).context("Negative CLAP event offset")?;
        self.staged.push((time, Staged::Param { id, value: plain }));
        Ok(())
    }

    fn format_parameter(&mut self, id: u32, value: f64) -> Result<String> {
        let meta = self.param(id)?;
        let plain = plain_from_normalized(meta.min, meta.max, meta.stepped(), value);
        Ok(self
            .value_text(id, plain)
            .unwrap_or_else(|| format!("{plain:.3}")))
    }

    fn send_midi_event_at(&mut self, event: MidiEvent, offset: i32) -> Result<()> {
        let staged = match event {
            MidiEvent::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => Staged::NoteOn {
                key: i16::from(note),
                channel: i16::from(channel.as_index()),
                id: -1,
                velocity: f64::from(velocity) / 127.0,
            },
            MidiEvent::NoteOn { channel, note, .. } => Staged::NoteOff {
                key: i16::from(note),
                channel: i16::from(channel.as_index()),
                id: -1,
                velocity: 0.0,
            },
            MidiEvent::NoteOff {
                channel,
                note,
                velocity,
            } => Staged::NoteOff {
                key: i16::from(note),
                channel: i16::from(channel.as_index()),
                id: -1,
                velocity: f64::from(velocity) / 127.0,
            },
            other => {
                if self.midi_notes
                    && let Some(data) = midi_bytes(other)
                {
                    let time = u32::try_from(offset).context("Negative CLAP event offset")?;
                    self.staged.push((time, Staged::Midi(data)));
                }
                return Ok(());
            }
        };
        self.stage_note(offset, staged)
    }

    fn send_plugin_event(&mut self, event: PluginEvent) -> Result<()> {
        let offset = event.sample_offset;
        match event.data {
            PluginEventData::NoteOn {
                channel,
                pitch,
                tuning,
                velocity,
                note_id,
                ..
            } => {
                self.stage_note(
                    offset,
                    Staged::NoteOn {
                        key: pitch,
                        channel,
                        id: note_id,
                        velocity: f64::from(velocity),
                    },
                )?;
                if tuning != 0.0 {
                    // VST3 tuning is in cents; CLAP tuning is in semitones.
                    self.stage_note(
                        offset,
                        Staged::Expression {
                            key: pitch,
                            channel,
                            id: note_id,
                            kind: NoteExpressionType::Tuning,
                            value: f64::from(tuning) / 100.0,
                        },
                    )?;
                }
                Ok(())
            }
            PluginEventData::NoteOff {
                channel,
                pitch,
                velocity,
                note_id,
                ..
            } => self.stage_note(
                offset,
                Staged::NoteOff {
                    key: pitch,
                    channel,
                    id: note_id,
                    velocity: f64::from(velocity),
                },
            ),
            PluginEventData::PolyPressure {
                channel,
                pitch,
                pressure,
                note_id,
            } => self.stage_note(
                offset,
                Staged::Expression {
                    key: pitch,
                    channel,
                    id: note_id,
                    kind: NoteExpressionType::Pressure,
                    value: f64::from(pressure).clamp(0.0, 1.0),
                },
            ),
            PluginEventData::NoteExpressionValue {
                type_id,
                note_id,
                value,
            } => {
                let Some((kind, value)) = clap_expression(type_id, value) else {
                    return Ok(());
                };
                self.stage_note(
                    offset,
                    Staged::Expression {
                        key: -1,
                        channel: -1,
                        id: note_id,
                        kind,
                        value,
                    },
                )
            }
            _ => Ok(()),
        }
    }

    fn note_expressions_on(&self, bus: i32, _channel: u8) -> Result<Vec<NoteExpressionInfo>> {
        if bus != 0 || !self.clap_notes {
            return Ok(Vec::new());
        }
        use vst3_host::midi::NoteExpressionType as Kind;
        Ok([
            (Kind::Volume, "Volume", 0.25),
            (Kind::Pan, "Pan", 0.5),
            (Kind::Tuning, "Tuning", 0.5),
            (Kind::Vibrato, "Vibrato", 0.0),
            (Kind::Expression, "Expression", 0.0),
            (Kind::Brightness, "Brightness", 0.0),
        ]
        .into_iter()
        .map(|(kind, title, default)| NoteExpressionInfo {
            kind,
            title: title.to_owned(),
            short_title: title.to_owned(),
            units: String::new(),
            default_value: default,
            min: 0.0,
            max: 1.0,
            step_count: 0,
            is_bipolar: matches!(kind, Kind::Pan | Kind::Tuning),
            is_one_shot: false,
            is_absolute: true,
        })
        .collect())
    }

    fn midi_panic(&mut self) -> Result<()> {
        if self.clap_notes {
            self.staged.push((
                0,
                Staged::NoteOff {
                    key: -1,
                    channel: -1,
                    id: -1,
                    velocity: 0.0,
                },
            ));
        } else if self.midi_notes {
            for channel in 0..16u8 {
                self.staged
                    .push((0, Staged::Midi([0xB0 | channel, 123, 0])));
            }
        }
        Ok(())
    }

    fn process_bus_audio(&mut self, buffers: &mut BusAudioBuffers) -> Result<()> {
        let frames = buffers.block_size;
        ensure!(
            frames > 0 && frames <= self.max_block,
            "Invalid CLAP block size"
        );
        ensure!(
            buffers.inputs.len() == self.inputs.len()
                && buffers.outputs.len() == self.outputs.len(),
            "CLAP audio port layout changed"
        );
        ensure!(self.is_processing(), "CLAP plug-in is not processing");
        for (bus, port) in buffers.inputs.iter_mut().zip(&self.inputs) {
            ensure!(
                bus.channels.len() == port.channels
                    && bus.channels.iter().all(|c| c.len() >= frames),
                "CLAP input buffer shape differs from its port"
            );
            if !port.active {
                bus.channels.iter_mut().for_each(|c| c.fill(0.0));
            }
        }
        for (bus, port) in buffers.outputs.iter().zip(&self.outputs) {
            ensure!(
                bus.channels.len() == port.channels
                    && bus.channels.iter().all(|c| c.len() >= frames),
                "CLAP output buffer shape differs from its port"
            );
        }
        self.fill_events();
        let transport = self.transport_event();
        self.output_events.clear();
        let steady_time = self.steady_time;
        let inputs = self
            .input_ports
            .with_input_buffers(buffers.inputs.iter_mut().map(|bus| {
                AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_input_only(
                        bus.channels
                            .iter_mut()
                            .map(|channel| InputChannel::variable(&mut channel[..frames])),
                    ),
                }
            }));
        let mut outputs = self
            .output_ports
            .with_output_buffers(buffers.outputs.iter_mut().map(|bus| {
                AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_output_only(
                        bus.channels
                            .iter_mut()
                            .map(|channel| &mut channel[..frames]),
                    ),
                }
            }));
        let input_events = self.events.as_input();
        let mut output_events = self.output_events.as_output();
        let processor = self
            .processor
            .as_mut()
            .and_then(|p| p.as_started_mut().ok())
            .context("CLAP plug-in is not processing")?;
        let shared = &self.instance;
        shared.access_shared_handler(|s| s.audio_section.store(true, Ordering::Release));
        let status = processor.process(
            &inputs,
            &mut outputs,
            &input_events,
            &mut output_events,
            Some(steady_time),
            Some(&transport),
        );
        shared.access_shared_handler(|s| s.audio_section.store(false, Ordering::Release));
        self.events.clear();
        status
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("CLAP processing failed")?;
        for (bus, port) in buffers.outputs.iter_mut().zip(&self.outputs) {
            if !port.active {
                bus.channels.iter_mut().for_each(|c| c.fill(0.0));
            }
        }
        self.collect_output_edits();
        self.steady_time = self.steady_time.wrapping_add(frames as u64);
        self.transport.advance(frames, self.sample_rate);
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        self.latency
    }

    fn tail_samples(&self) -> u32 {
        self.tail
    }

    fn save_state(&mut self) -> Result<Vec<u8>> {
        let handle = self.instance.plugin_handle();
        let Some(state) = handle.get_extension::<PluginState>() else {
            return Ok(Vec::new());
        };
        let mut data = Vec::new();
        state
            .save(&handle, &mut data)
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("Saving CLAP state")?;
        Ok(data)
    }

    fn load_state(&mut self, data: &[u8]) -> Result<()> {
        let handle = self.instance.plugin_handle();
        let Some(state) = handle.get_extension::<PluginState>() else {
            ensure!(
                data.is_empty(),
                "This CLAP plug-in cannot restore saved state"
            );
            return Ok(());
        };
        if data.is_empty() {
            return Ok(());
        }
        let mut reader = data;
        state
            .load(&handle, &mut reader)
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("Restoring CLAP state")?;
        self.mark(RESTART_PARAM_VALUES_CHANGED);
        Ok(())
    }
}

impl ClapProcessor {
    fn plain_value(&mut self, id: u32) -> Option<f64> {
        let handle = self.instance.plugin_handle();
        let params = handle.get_extension::<PluginParams>()?;
        params
            .get_value(&handle, ClapId::from_raw(id)?)
            .filter(|value| value.is_finite())
    }

    fn value_text(&mut self, id: u32, plain: f64) -> Option<String> {
        let handle = self.instance.plugin_handle();
        let params = handle.get_extension::<PluginParams>()?;
        let mut buffer = [0u8; 256];
        let written = params
            .value_to_text(&handle, ClapId::from_raw(id)?, plain, &mut buffer)
            .ok()?;
        Some(text(written))
    }
}

fn midi_bytes(event: MidiEvent) -> Option<[u8; 3]> {
    Some(match event {
        MidiEvent::ControlChange {
            channel,
            controller,
            value,
        } => [0xB0 | channel.as_index(), controller & 0x7F, value & 0x7F],
        MidiEvent::ProgramChange { channel, program } => {
            [0xC0 | channel.as_index(), program & 0x7F, 0]
        }
        MidiEvent::PitchBend { channel, value } => [
            0xE0 | channel.as_index(),
            (value & 0x7F) as u8,
            ((value >> 7) & 0x7F) as u8,
        ],
        MidiEvent::ChannelAftertouch { channel, pressure } => {
            [0xD0 | channel.as_index(), pressure & 0x7F, 0]
        }
        MidiEvent::PolyAftertouch {
            channel,
            note,
            pressure,
        } => [0xA0 | channel.as_index(), note & 0x7F, pressure & 0x7F],
        _ => return None,
    })
}

/// VST3 note-expression type IDs and normalized values mapped to CLAP's plain units.
fn clap_expression(type_id: u32, value: f64) -> Option<(NoteExpressionType, f64)> {
    let value = value.clamp(0.0, 1.0);
    Some(match type_id {
        0 => (NoteExpressionType::Volume, value * 4.0),
        1 => (NoteExpressionType::Pan, value),
        2 => (NoteExpressionType::Tuning, (value - 0.5) * 240.0),
        3 => (NoteExpressionType::Vibrato, value),
        4 => (NoteExpressionType::Expression, value),
        5 => (NoteExpressionType::Brightness, value),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_map_to_rill_categories() {
        let instrument = category(&["instrument".into(), "synthesizer".into()]);
        assert_eq!(instrument, "Instrument|synthesizer");
        let effect = category(&["audio-effect".into(), "delay".into()]);
        assert_eq!(effect, "Fx|delay");
    }

    #[test]
    fn vst3_expression_values_become_clap_units() {
        let (kind, value) = clap_expression(2, 0.75).unwrap();
        assert_eq!(kind, NoteExpressionType::Tuning);
        assert!((value - 60.0).abs() < 1e-9);
        assert_eq!(clap_expression(0, 0.25).unwrap().1, 1.0);
        assert!(clap_expression(99, 0.5).is_none());
    }
}
