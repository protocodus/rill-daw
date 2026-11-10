//! VST3, CLAP, (on Linux) LV2 and (on macOS) Audio Unit discovery and processing,
//! separate from the audio device backend.
//!
//! VST3 scanning reads directory entries and optional `moduleinfo.json` files. CLAP and
//! LV2 descriptors are read by a disposable helper per bundle, and the Audio Unit
//! registry by one helper, so scanning never loads native plugin code into this process. Each selected instrument runs in a separate `daw-plugin-host`
//! process, which contains crashes and hangs. This is process crash isolation, not an OS
//! permission sandbox: the child retains the user's file and network access. Native plugin
//! code is never loaded into the DAW process. No in-process fallback is available.
//!
//! IPC is synchronous and allocates while serializing blocks, so render on a worker/control
//! thread and give the device callback prepared audio. Never call this adapter on a hard
//! real-time device callback. A hung command is bounded by a timeout and the child is killed.
//! Bus-aware processing preserves native bus indices and stereo pairs; legacy calls route
//! the first active output bus, with mono duplicated to stereo.
//!
//! API references: <https://docs.rs/vst3-host/0.10.0/vst3_host/> and
//! <https://github.com/steinbergmedia/vst3_pluginterfaces>.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
pub use vst3_host::audio::{AudioBusDetails, AudioBusDetailsLayout};
use vst3_host::process_isolation::{BlockCommand, HostCommand, HostResponse, PluginHostProcess};
pub use vst3_host::process_isolation::{BlockTransport, IpcStats};
pub use vst3_host::{AudioBusConfig, AudioBusLayout};
use vst3_host::{BusAudioBuffers, MidiChannel, MidiEvent, Plugin, TransportPosition, Vst3Host};

mod audio_buffers;
use audio_buffers::{
    MAX_AUDIO_CHANNELS, copy_stereo_input, validate_audio_bus_layout, validate_pair,
};

#[cfg(target_os = "macos")]
mod au;
pub mod audio_unit;
pub mod catalog;
mod clap;
mod expressive;
mod format;
#[cfg(target_os = "linux")]
mod lv2;
mod native;
mod scan;
pub use expressive::{
    ExpressiveController, ExpressiveMidiData, ExpressiveMidiMessage, ExpressiveVelocity,
};
pub use format::PluginFormat;
pub use scan::scan_bundle;
mod editor;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod editor_context_menu;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod editor_native;
#[cfg(target_os = "macos")]
#[path = "editor_platform_macos.rs"]
mod editor_platform;
#[cfg(target_os = "linux")]
#[path = "editor_platform_linux.rs"]
mod editor_platform;
#[cfg(target_os = "windows")]
#[path = "editor_platform_windows.rs"]
mod editor_platform;
pub mod helper;
mod inspection;
pub mod sync;
pub use catalog::{CatalogEntry, PluginCatalog, ThumbnailStatus};
pub use editor::{EditorEvent, EditorSession, capture_editor, capture_editor_info, launch_editor};
pub use inspection::{
    PluginAudioBus, PluginInspection, PluginParameterMetadata, inspect_plugin,
    inspect_plugin_with_configuration, inspect_plugin_with_state,
};

/// Leaves headroom in the host's 4,096-event queue for panic note-offs.
pub const MAX_MIDI_EVENTS_PER_BLOCK: usize = 2_048;

/// Live playback must fail promptly; an offline bounce can wait for a scheduled
/// worker or a plugin's higher-quality processing without mistaking it for a hang.
/// Both modes retain a finite per-command deadline and terminate hung children.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProcessingMode {
    #[default]
    Realtime,
    Offline,
}

impl ProcessingMode {
    fn response_timeout(self) -> Duration {
        match self {
            Self::Realtime => Duration::from_millis(100),
            Self::Offline => Duration::from_secs(5),
        }
    }
}

/// Catalogue entry. Missing metadata means the bundle has not been inspected by loading it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub path: PathBuf,
    pub vendor: String,
    pub category: String,
    /// Present when a bundle's metadata identifies a particular processor class.
    pub class_id: Option<String>,
    /// Declared class version; empty without bundle metadata.
    #[serde(default)]
    pub version: String,
    /// Binary folders inside the bundle, such as `x86_64-linux` or `MacOS`.
    #[serde(default)]
    pub architectures: Vec<String>,
    /// Absent in indexes and settings written before CLAP/LV2 support.
    #[serde(default)]
    pub format: PluginFormat,
}

impl PluginInfo {
    /// `None` means a bundle without declared metadata; loading will verify MIDI support.
    pub fn is_instrument(&self) -> Option<bool> {
        if self.category.is_empty() {
            None
        } else {
            Some(
                self.category
                    .split('|')
                    .any(|part| part.eq_ignore_ascii_case("Instrument")),
            )
        }
    }

    pub fn is_effect(&self) -> Option<bool> {
        if self.category.is_empty() {
            None
        } else {
            Some(
                self.category
                    .split('|')
                    .any(|part| part.eq_ignore_ascii_case("Fx")),
            )
        }
    }
}

/// Standard folders of every hostable format, plus `VST3_PATH`, `CLAP_PATH` and
/// (on Linux) `LV2_PATH` when set. On macOS this includes the Audio Unit locator.
pub fn default_locations() -> Vec<PathBuf> {
    PluginFormat::available()
        .flat_map(PluginFormat::default_locations)
        .collect()
}

/// Discover bundles without executing third-party plugin code.
pub fn discover() -> Result<Vec<PluginInfo>> {
    scan_paths(&default_locations())
}

/// Discover from user-configured roots, without implicitly adding system directories.
pub fn discover_in_paths(paths: &[PathBuf]) -> Result<Vec<PluginInfo>> {
    scan_paths(paths)
}

/// Resolve our matching helper executable. Distribution places it next to the DAW binary.
/// `DAW_PLUGIN_HOST` overrides that location; a missing helper is a hard loading error.
pub fn helper_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("DAW_PLUGIN_HOST") {
        let path = PathBuf::from(path);
        ensure!(
            path.is_file(),
            "DAW_PLUGIN_HOST does not point to a helper executable"
        );
        return Ok(path);
    }
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .context("Cannot locate application directory")?;
    let name = if cfg!(windows) {
        "daw-plugin-host.exe"
    } else {
        "daw-plugin-host"
    };
    let sibling = directory.join(name);
    if sibling.is_file() {
        return Ok(sibling);
    }
    // Cargo test executables live in target/{debug,release}/deps.
    if directory
        .file_name()
        .is_some_and(|name| name == "deps" || name == "examples")
        && let Some(parent) = directory.parent()
    {
        let sibling = parent.join(name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    anyhow::bail!(
        "Plugin isolation helper is missing. Run cargo build --workspace, or set DAW_PLUGIN_HOST to daw-plugin-host"
    )
}

/// Recursively scan directories or individual `.vst3`, `.clap` and `.lv2` bundles/files.
/// On macOS the [`audio_unit::AU_LOCATOR`] root lists the installed Audio Units.
///
/// Missing roots are ignored. Symbolic-link cycles and duplicate roots are collapsed by
/// canonical path. Known effects are included so callers can decide how to display them.
/// Malformed or missing metadata falls back to the bundle name, allowing manual loading.
pub fn scan_paths(paths: &[PathBuf]) -> Result<Vec<PluginInfo>> {
    let mut pending = paths.to_vec();
    let mut visited = HashSet::new();
    let mut found = Vec::new();
    let mut audio_units = false;
    while let Some(path) = pending.pop() {
        // The locator is matched as given; resolving it could change its spelling.
        if PluginFormat::of_bundle(&path) == Some(PluginFormat::Au) {
            if !std::mem::replace(&mut audio_units, true) {
                found.extend(bundle_info(&path));
            }
            continue;
        }
        if !path.exists() {
            continue;
        }
        let canonical = path
            .canonicalize()
            .with_context(|| format!("Resolving {}", path.display()))?;
        if !visited.insert(canonical.clone()) {
            continue;
        }
        if is_bundle(&canonical) {
            found.extend(bundle_info(&canonical));
        } else if canonical.is_dir() {
            for entry in std::fs::read_dir(&canonical)
                .with_context(|| format!("Scanning {}", canonical.display()))?
            {
                pending.push(entry?.path());
            }
        }
    }
    found.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.path.cmp(&b.path))
            .then(a.class_id.cmp(&b.class_id))
    });
    Ok(found)
}

fn is_bundle(path: &Path) -> bool {
    PluginFormat::of_bundle(path).is_some()
}

fn bundle_info(path: &Path) -> Vec<PluginInfo> {
    let format = PluginFormat::from_path(path);
    if format == PluginFormat::Au {
        // The registry has no bundle to list by name when enumeration fails.
        return scan::scan_bundle(path).unwrap_or_else(|error| {
            eprintln!("Audio Unit scan failed: {error:#}");
            Vec::new()
        });
    }
    if format != PluginFormat::Vst3 {
        // An unreadable bundle stays listed by name so loading can report the cause.
        return scan::scan_bundle(path).unwrap_or_else(|error| {
            eprintln!("Plug-in scan failed for {}: {error:#}", path.display());
            vec![PluginInfo {
                name: path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: path.to_owned(),
                format,
                ..PluginInfo::default()
            }]
        });
    }
    let architectures = bundle_architectures(path);
    if let Ok(Some(module)) = vst3_host::discovery::read_module_info(path) {
        let classes: Vec<_> = module
            .classes
            .into_iter()
            .filter(|class| class.category == "Audio Module Class")
            .map(|class| PluginInfo {
                name: class.name,
                path: path.to_owned(),
                vendor: if class.vendor.is_empty() {
                    module.factory.vendor.clone()
                } else {
                    class.vendor
                },
                category: class.sub_categories.join("|"),
                class_id: Some(class.class_id),
                version: if class.version.is_empty() {
                    module.version.clone()
                } else {
                    class.version
                },
                architectures: architectures.clone(),
                format: PluginFormat::Vst3,
            })
            .collect();
        if !classes.is_empty() {
            return classes;
        }
    }
    vec![PluginInfo {
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path: path.to_owned(),
        architectures,
        ..PluginInfo::default()
    }]
}

/// VST3 bundles keep one binary folder per platform under `Contents`. Listing
/// them reads directory names only; no binary is opened.
fn bundle_architectures(path: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(path.join("Contents")) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name == "MacOS" || name.ends_with("-linux") || name.ends_with("-win"))
        .collect();
    found.sort();
    found
}

/// MIDI channel 1 note event at a sample offset within the next block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiMessage {
    pub offset: u32,
    pub pitch: u8,
    pub velocity: u8,
    pub on: bool,
}

impl MidiMessage {
    fn as_host_event(self) -> MidiEvent {
        if self.on && self.velocity != 0 {
            MidiEvent::NoteOn {
                channel: MidiChannel::Ch1,
                note: self.pitch,
                velocity: self.velocity,
            }
        } else {
            MidiEvent::NoteOff {
                channel: MidiChannel::Ch1,
                note: self.pitch,
                velocity: self.velocity,
            }
        }
    }
}

/// Controller parameter values are normalized to 0..=1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterInfo {
    pub id: u32,
    pub name: String,
    pub value: f64,
    pub default: f64,
    pub unit: String,
    pub steps: i32,
    pub read_only: bool,
}

/// One stereo pair (or the final mono channel) on an indexed native input bus.
/// `first_channel` must be even. Mono destinations receive an equal-weight downmix.
#[derive(Clone, Copy)]
pub struct StereoBusInput<'a> {
    pub bus: usize,
    pub first_channel: usize,
    pub left: &'a [f32],
    pub right: &'a [f32],
}

/// Destination for one indexed native output pair. A final mono channel is duplicated.
pub struct StereoBusOutput<'a> {
    pub bus: usize,
    pub first_channel: usize,
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

#[derive(Clone)]
struct BusActivation {
    default_inputs: Vec<usize>,
    default_outputs: Vec<usize>,
    inputs: Vec<usize>,
    outputs: Vec<usize>,
}

fn allocate_bus_buffers(
    plugin: &mut Plugin,
    frames: usize,
    sample_rate: f64,
) -> Result<(BusAudioBuffers, Option<usize>, usize)> {
    let layout = plugin.audio_bus_layout()?;
    validate_audio_bus_layout(&layout, frames)?;
    let details = plugin.audio_bus_details()?;
    ensure!(
        details.inputs.len() == layout.inputs.len()
            && details.outputs.len() == layout.outputs.len(),
        "Plug-in bus layout changed during configuration"
    );
    let main_input = details
        .inputs
        .iter()
        .enumerate()
        .position(|(index, bus)| bus.main && layout.inputs[index].channel_count > 0);
    let main_output = details
        .outputs
        .iter()
        .enumerate()
        .position(|(index, bus)| bus.main && layout.outputs[index].channel_count > 0);
    for (direction, buses, main) in [
        (
            vst3_host::audio::BusDirection::Input,
            &layout.inputs,
            main_input,
        ),
        (
            vst3_host::audio::BusDirection::Output,
            &layout.outputs,
            main_output,
        ),
    ] {
        if let Some(index) = main
            && !buses[index].active
        {
            plugin.set_bus_active(
                vst3_host::audio::MediaType::Audio,
                direction,
                index as i32,
                true,
            )?;
        }
    }
    let layout = plugin.audio_bus_layout()?;
    validate_audio_bus_layout(&layout, frames)?;
    let active = |buses: &[AudioBusConfig], main: Option<usize>| {
        main.filter(|&index| {
            buses
                .get(index)
                .is_some_and(|bus| bus.active && bus.channel_count > 0)
        })
        .or_else(|| {
            buses
                .iter()
                .position(|bus| bus.active && bus.channel_count > 0)
        })
    };
    let input = active(&layout.inputs, main_input);
    let output =
        active(&layout.outputs, main_output).context("Plug-in has no active audio output bus")?;
    Ok((
        BusAudioBuffers::new(&layout, frames, sample_rate),
        input,
        output,
    ))
}

/// A proxy to a native VST3 instance in its own crash-isolated child process.
pub struct VstInstrument {
    plugin: Plugin,
    buffers: BusAudioBuffers,
    info: PluginInfo,
    input_bus: Option<usize>,
    output_bus: usize,
    max_block: usize,
    panic_pending: bool,
    active_notes: [u16; 128],
    latency: u32,
    tail: u32,
    cached_tempo: Option<f64>,
    cached_playing: Option<bool>,
    next_position: Option<TransportPosition>,
    block_commands: Vec<BlockCommand>,
    queued_transport: Option<(TransportPosition, f64, bool)>,
    bus_activation: Option<BusActivation>,
    expressive_notes: Vec<expressive::Voice>,
    next_expressive_notes: Vec<expressive::Voice>,
}

impl VstInstrument {
    /// Load and activate the first processor class from a VST3 bundle.
    ///
    /// Call on a worker/control thread. The plugin must accept MIDI and expose at least
    /// one active audio output. Loading fails if the isolation helper is unavailable.
    pub fn load(path: &Path, sample_rate: f64, max_block: usize) -> Result<Self> {
        Self::load_class(path, None, sample_rate, max_block)
    }

    /// Load the exact processor class represented by a discovery result.
    pub fn load_info(info: &PluginInfo, sample_rate: f64, max_block: usize) -> Result<Self> {
        Self::load_class(&info.path, info.class_id.as_deref(), sample_rate, max_block)
    }

    pub fn load_class(
        path: &Path,
        class_id: Option<&str>,
        sample_rate: f64,
        max_block: usize,
    ) -> Result<Self> {
        Self::load_class_with_mode(
            path,
            class_id,
            sample_rate,
            max_block,
            ProcessingMode::Realtime,
        )
    }

    /// Load for an explicit processing purpose. Offline instances advertise
    /// VST3 `kOffline` and use a bounded five-second block/IPC deadline; ordinary
    /// constructors retain the live 100 ms deadline.
    pub fn load_class_with_mode(
        path: &Path,
        class_id: Option<&str>,
        sample_rate: f64,
        max_block: usize,
        mode: ProcessingMode,
    ) -> Result<Self> {
        Self::load_processor(path, class_id, sample_rate, max_block, true, mode)
    }

    fn load_processor(
        path: &Path,
        class_id: Option<&str>,
        sample_rate: f64,
        max_block: usize,
        require_midi: bool,
        mode: ProcessingMode,
    ) -> Result<Self> {
        ensure!(
            sample_rate.is_finite() && (8_000.0..=768_000.0).contains(&sample_rate),
            "Invalid plugin sample rate"
        );
        ensure!(
            (1..=65_536).contains(&max_block),
            "Invalid plugin maximum block size"
        );
        ensure!(
            is_bundle(path),
            "Expected a .vst3, .clap{} plug-in bundle",
            if cfg!(target_os = "linux") {
                " or .lv2"
            } else if cfg!(target_os = "macos") {
                " or Audio Unit"
            } else {
                ""
            }
        );
        let mut host = Vst3Host::builder()
            .sample_rate(sample_rate)
            .block_size(max_block)
            .input_channels(if require_midi { 0 } else { 2 })
            .output_channels(2)
            .with_process_isolation(true)
            .helper_path(helper_path()?)
            .response_timeout(mode.response_timeout())
            .auto_recover_plugins(false)
            .build()?;
        let mut plugin = match class_id {
            Some(id) => host.load_plugin_class(path, id),
            None => host.load_plugin(path),
        }
        .with_context(|| format!("Loading VST3 processor {}", path.display()))?;
        if mode == ProcessingMode::Offline {
            plugin
                .set_process_mode(vst3_host::plugin::ProcessMode::Offline)
                .context("Configuring offline VST3 processing")?;
        }
        ensure!(
            !require_midi || plugin.info().has_midi_input,
            "{} does not accept MIDI input",
            plugin.info().name
        );
        let (buffers, input_bus, output_bus) =
            allocate_bus_buffers(&mut plugin, max_block, sample_rate)?;
        ensure!(
            require_midi || input_bus.is_some(),
            "Effect has no active audio input bus"
        );
        let raw = plugin.info();
        let info = PluginInfo {
            name: raw.name.clone(),
            path: path.to_owned(),
            vendor: raw.vendor.clone(),
            category: raw.category.clone(),
            class_id: Some(raw.uid.clone()),
            version: raw.version.clone(),
            format: PluginFormat::from_path(path),
            ..PluginInfo::default()
        };
        plugin
            .start_processing()
            .with_context(|| format!("Starting VST3 processor {}", path.display()))?;
        plugin.prepare_block_processing()?;
        let latency = plugin.latency_samples();
        let tail = plugin.tail_samples();
        ensure!(
            plugin.isolation_pid().is_some(),
            "Plugin was not loaded into an isolated process"
        );
        Ok(Self {
            plugin,
            buffers,
            info,
            input_bus,
            output_bus,
            max_block,
            panic_pending: false,
            active_notes: [0; 128],
            latency,
            tail,
            cached_tempo: None,
            cached_playing: None,
            next_position: None,
            block_commands: Vec::with_capacity(MAX_MIDI_EVENTS_PER_BLOCK * 2 + 256),
            queued_transport: None,
            bus_activation: None,
            expressive_notes: Vec::with_capacity(MAX_MIDI_EVENTS_PER_BLOCK),
            next_expressive_notes: Vec::with_capacity(MAX_MIDI_EVENTS_PER_BLOCK),
        })
    }

    pub fn info(&self) -> &PluginInfo {
        &self.info
    }

    pub fn latency_samples(&self) -> u32 {
        self.latency
    }

    pub fn tail_samples(&self) -> u32 {
        self.tail
    }

    pub fn isolation_pid(&self) -> Option<u32> {
        self.plugin.isolation_pid()
    }

    pub fn main_input_bus(&self) -> Option<usize> {
        self.input_bus
    }

    pub fn main_output_bus(&self) -> usize {
        self.output_bus
    }

    /// Current native bus indices and channel counts. Query on a control thread.
    pub fn audio_bus_layout(&self) -> AudioBusLayout {
        AudioBusLayout {
            inputs: self
                .buffers
                .inputs
                .iter()
                .map(|bus| AudioBusConfig {
                    channel_count: bus.channels.len(),
                    active: bus.active,
                })
                .collect(),
            outputs: self
                .buffers
                .outputs
                .iter()
                .map(|bus| AudioBusConfig {
                    channel_count: bus.channels.len(),
                    active: bus.active,
                })
                .collect(),
        }
    }
    pub fn audio_bus_details(&self) -> Result<AudioBusDetailsLayout> {
        Ok(self.plugin.audio_bus_details()?)
    }

    /// Activate additional audio buses without disabling the plug-in's original main/default
    /// buses. Reconfiguration belongs on the control thread and is retained across state and
    /// child-process recovery. Callers replace the render graph if native activation fails.
    pub fn configure_audio_buses(&mut self, inputs: &[usize], outputs: &[usize]) -> Result<()> {
        ensure!(
            inputs.len() <= MAX_AUDIO_CHANNELS && outputs.len() <= MAX_AUDIO_CHANNELS,
            "Too many requested plug-in audio buses"
        );
        let layout = self.plugin.audio_bus_layout()?;
        validate_audio_bus_layout(&layout, self.max_block)?;
        for (&index, buses) in inputs
            .iter()
            .map(|i| (i, &layout.inputs))
            .chain(outputs.iter().map(|i| (i, &layout.outputs)))
        {
            ensure!(
                buses.get(index).is_some_and(|bus| bus.channel_count > 0),
                "Invalid plug-in audio bus index {index}"
            );
        }
        let baseline = self
            .bus_activation
            .clone()
            .unwrap_or_else(|| BusActivation {
                default_inputs: layout
                    .inputs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, bus)| bus.active.then_some(index))
                    .collect(),
                default_outputs: layout
                    .outputs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, bus)| bus.active.then_some(index))
                    .collect(),
                inputs: Vec::new(),
                outputs: Vec::new(),
            });
        self.plugin.stop_processing()?;
        self.bus_activation = Some(BusActivation {
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
            ..baseline
        });
        self.refresh_bus_buffers()?;
        self.plugin.start_processing()?;
        self.latency = self.plugin.latency_samples();
        self.tail = self.plugin.tail_samples();
        self.next_position = None;
        Ok(())
    }

    fn refresh_bus_buffers(&mut self) -> Result<()> {
        let restart = self.plugin.service_host_requests()?;
        ensure!(
            !restart.reload_component(),
            "Plug-in requested a component reload while configuring audio buses"
        );
        let layout = self.plugin.audio_bus_layout()?;
        validate_audio_bus_layout(&layout, self.max_block)?;
        if let Some(activation) = &self.bus_activation {
            for (direction, buses, baseline, selected) in [
                (
                    vst3_host::audio::BusDirection::Input,
                    &layout.inputs,
                    &activation.default_inputs,
                    &activation.inputs,
                ),
                (
                    vst3_host::audio::BusDirection::Output,
                    &layout.outputs,
                    &activation.default_outputs,
                    &activation.outputs,
                ),
            ] {
                for &index in baseline.iter().chain(selected) {
                    ensure!(
                        buses.get(index).is_some_and(|bus| bus.channel_count > 0),
                        "Configured plug-in audio bus {index} disappeared"
                    );
                }
                for (index, bus) in buses.iter().enumerate() {
                    let active = baseline.contains(&index) || selected.contains(&index);
                    if active || bus.active != active {
                        self.plugin.set_bus_active(
                            vst3_host::audio::MediaType::Audio,
                            direction,
                            index as i32,
                            active,
                        )?;
                    }
                }
            }
        }
        let (buffers, input_bus, output_bus) =
            allocate_bus_buffers(&mut self.plugin, self.max_block, self.buffers.sample_rate)?;
        self.input_bus = input_bus;
        self.output_bus = output_bus;
        self.buffers = buffers;
        self.block_commands.clear();
        self.queued_transport = None;
        self.plugin.prepare_block_processing()?;
        Ok(())
    }

    /// Restart a failed child on a worker/control thread. The plugin returns to its default
    /// preset; call `load_state` with the saved session state to restore its settings.
    pub fn recover(&mut self) -> Result<()> {
        self.plugin.recover()?;
        self.plugin.stop_processing()?;
        self.refresh_bus_buffers()?;
        self.plugin.start_processing()?;
        self.latency = self.plugin.latency_samples();
        self.tail = self.plugin.tail_samples();
        self.panic_pending = false;
        self.active_notes.fill(0);
        self.expressive_notes.clear();
        self.next_expressive_notes.clear();
        self.cached_tempo = None;
        self.cached_playing = None;
        self.next_position = None;
        Ok(())
    }

    /// Release tracked active notes before the next block's note events. Explicit
    /// note-offs preserve ordering with chased note-ons; controller-based panic can
    /// arrive through a separate VST parameter stream and silence new notes.
    /// This does not reload the preset or suppress release/reverb tails.
    pub fn reset(&mut self) -> Result<()> {
        self.panic_pending = true;
        Ok(())
    }

    /// Queue automation for the next audio block. Ordinary control edits use
    /// `set_parameter`, which remains synchronous and reports errors immediately.
    pub fn queue_parameter_for_block(&mut self, id: u32, value: f64) -> Result<()> {
        ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "Parameter value must be normalized to 0..=1"
        );
        ensure!(
            self.block_commands.len() < vst3_host::parameters::MAX_PENDING_PARAM_CHANGES,
            "Too many block parameter changes"
        );
        self.block_commands
            .push(BlockCommand::SetParameter { id, value });
        Ok(())
    }

    /// Queue transport metadata with the next audio request. Cache state only
    /// after that block succeeds, so rejected or zero-frame blocks cannot lose it.
    pub fn queue_transport_for_block(
        &mut self,
        sample_position: i64,
        quarter_notes: f64,
        bpm: f64,
        playing: bool,
    ) -> Result<()> {
        ensure!(
            quarter_notes.is_finite() && bpm.is_finite() && bpm > 0.0,
            "Invalid plugin transport position or tempo"
        );
        self.queued_transport = Some((
            TransportPosition {
                samples: sample_position,
                quarter_notes,
            },
            bpm,
            playing,
        ));
        Ok(())
    }

    /// Choose the per-instance IPC policy on a control thread.
    pub fn set_block_transport(&mut self, policy: BlockTransport) -> Result<()> {
        self.plugin.set_block_transport(policy)?;
        Ok(())
    }

    /// Cumulative wire traffic; reads do not contact the native helper.
    pub fn ipc_stats(&self) -> Option<IpcStats> {
        self.plugin.ipc_stats()
    }

    pub fn set_transport(
        &mut self,
        sample_position: i64,
        quarter_notes: f64,
        bpm: f64,
        playing: bool,
    ) -> Result<()> {
        ensure!(
            quarter_notes.is_finite() && bpm.is_finite() && bpm > 0.0,
            "Invalid plugin transport position or tempo"
        );
        if !self.next_position.is_some_and(|position| {
            position.samples == sample_position
                && (position.quarter_notes - quarter_notes).abs() < 1.0e-9
        }) {
            let position = TransportPosition {
                samples: sample_position,
                quarter_notes,
            };
            self.plugin.set_transport_position(position)?;
            self.next_position = Some(position);
        }
        if self.cached_tempo != Some(bpm) {
            self.plugin.set_tempo(bpm)?;
            self.cached_tempo = Some(bpm);
        }
        if self.cached_playing != Some(playing) {
            self.plugin.set_playing(playing)?;
            self.cached_playing = Some(playing);
        }
        Ok(())
    }

    /// Render one block. Events must be ordered by offset and fit in the actual block.
    /// Stereo slices must have equal lengths no greater than the configured maximum.
    /// Invalid blocks are rejected before queuing any events; outputs are silenced on error.
    /// IPC allocates and waits for the child, so this method belongs off the device callback.
    pub fn process(
        &mut self,
        events: &[MidiMessage],
        left: &mut [f32],
        right: &mut [f32],
    ) -> Result<()> {
        self.process_block(events, None, left, right)
    }

    /// Render an instrument with stereo input on its first active audio input bus.
    /// Mono input buses receive an equal-weight downmix; additional buses receive silence.
    pub fn process_with_input(
        &mut self,
        events: &[MidiMessage],
        input_left: &[f32],
        input_right: &[f32],
        output_left: &mut [f32],
        output_right: &mut [f32],
    ) -> Result<()> {
        self.process_block(
            events,
            Some((input_left, input_right)),
            output_left,
            output_right,
        )
    }

    fn process_block(
        &mut self,
        events: &[MidiMessage],
        inputs: Option<(&[f32], &[f32])>,
        left: &mut [f32],
        right: &mut [f32],
    ) -> Result<()> {
        left.fill(0.0);
        right.fill(0.0);
        let mut outputs = [StereoBusOutput {
            bus: self.output_bus,
            first_channel: 0,
            left,
            right,
        }];
        if let Some((left, right)) = inputs {
            let bus = self
                .input_bus
                .context("Plugin has no active audio input bus")?;
            self.process_buses(
                events,
                &[StereoBusInput {
                    bus,
                    first_channel: 0,
                    left,
                    right,
                }],
                &mut outputs,
            )
        } else {
            self.process_buses(events, &[], &mut outputs)
        }
    }

    /// Process all active native buses once and copy only the requested output pairs.
    /// Omitted input pairs are silent. Duplicate pairs, inactive buses, invalid lengths,
    /// and odd pair offsets are rejected before any MIDI is sent. Destinations stay silent
    /// on error. This synchronous IPC operation belongs on the render/control worker.
    pub fn process_buses(
        &mut self,
        events: &[MidiMessage],
        inputs: &[StereoBusInput<'_>],
        outputs: &mut [StereoBusOutput<'_>],
    ) -> Result<()> {
        for output in outputs.iter_mut() {
            output.left.fill(0.0);
            output.right.fill(0.0);
        }
        ensure!(
            outputs.len() <= MAX_AUDIO_CHANNELS,
            "Too many requested plug-in output pairs"
        );
        let frames = outputs
            .first()
            .map(|bus| bus.left.len())
            .or_else(|| inputs.first().map(|bus| bus.left.len()))
            .unwrap_or(0);
        for (index, output) in outputs.iter().enumerate() {
            validate_block(&[], output.left.len(), output.right.len(), self.max_block)?;
            ensure!(
                output.left.len() == frames,
                "Plug-in bus block lengths differ"
            );
            validate_pair(&self.buffers.outputs, output.bus, output.first_channel)?;
            ensure!(
                !outputs[..index]
                    .iter()
                    .any(|other| other.bus == output.bus
                        && other.first_channel == output.first_channel),
                "Duplicate plug-in output pair"
            );
        }
        self.process_bus_inputs(events, inputs, frames)?;
        for output in outputs {
            self.copy_output_pair(output.bus, output.first_channel, output.left, output.right)?;
        }
        Ok(())
    }

    /// Render into retained native bus storage. Read any number of output pairs afterward
    /// with `copy_output_pair`; this does not run the plug-in more than once.
    pub fn process_bus_inputs(
        &mut self,
        events: &[MidiMessage],
        inputs: &[StereoBusInput<'_>],
        frames: usize,
    ) -> Result<()> {
        self.process_bus_inputs_expressive(events, &[], inputs, frames)
    }

    /// Render ordinary and high-resolution musical events in one native block.
    /// Both event lists must be ordered; offsets are merged before sending to the child.
    pub fn process_bus_inputs_expressive(
        &mut self,
        events: &[MidiMessage],
        expressive: &[ExpressiveMidiMessage],
        inputs: &[StereoBusInput<'_>],
        frames: usize,
    ) -> Result<()> {
        // Silence retained outputs even when validation rejects this block. Inputs
        // are cleared only once, after validation, before copying connected pairs.
        for bus in &mut self.buffers.outputs {
            for channel in &mut bus.channels {
                channel.fill(0.0);
            }
        }
        ensure!(
            inputs.len() <= MAX_AUDIO_CHANNELS,
            "Too many requested plug-in input pairs"
        );
        validate_block(events, frames, frames, self.max_block)?;
        ensure!(
            events.len() + expressive.len() <= MAX_MIDI_EVENTS_PER_BLOCK,
            "Too many combined MIDI events in one block"
        );
        self.prepare_expressive(expressive, frames)?;
        for (index, input) in inputs.iter().enumerate() {
            ensure!(
                input.left.len() == frames && input.right.len() == frames,
                "Plug-in bus block lengths differ"
            );
            validate_pair(&self.buffers.inputs, input.bus, input.first_channel)?;
            ensure!(
                !inputs[..index]
                    .iter()
                    .any(|other| other.bus == input.bus
                        && other.first_channel == input.first_channel),
                "Duplicate plug-in input pair"
            );
        }
        if frames == 0 {
            self.buffers.block_size = 0;
            return Ok(());
        }
        let next_active = track_active_notes(&self.active_notes, self.panic_pending, events)?;
        ensure!(
            next_active
                .iter()
                .map(|&count| usize::from(count))
                .sum::<usize>()
                + self.next_expressive_notes.len()
                <= MAX_MIDI_EVENTS_PER_BLOCK,
            "Too many held MIDI notes"
        );
        let mut commands = std::mem::take(&mut self.block_commands);
        let queued_transport = self.queued_transport.take();
        self.buffers.block_size = frames;
        for bus in &mut self.buffers.inputs {
            audio_buffers::prepare_channels(&mut bus.channels, frames);
        }
        for bus in &mut self.buffers.outputs {
            for channel in &mut bus.channels {
                // Existing samples were silenced above; growing adds silence too.
                channel.resize(frames, 0.0);
            }
        }
        for input in inputs {
            copy_stereo_input(
                &mut self.buffers.inputs[input.bus].channels[input.first_channel..],
                input.left,
                input.right,
            );
        }
        if let Some((position, bpm, playing)) = queued_transport {
            if !self.next_position.is_some_and(|cached| {
                cached.samples == position.samples
                    && (cached.quarter_notes - position.quarter_notes).abs() < 1.0e-9
            }) {
                commands.push(BlockCommand::SetTransportPosition { position });
            }
            if self.cached_tempo != Some(bpm) {
                commands.push(BlockCommand::SetTempo { bpm });
            }
            if self.cached_playing != Some(playing) {
                commands.push(BlockCommand::SetPlaying { playing });
            }
        }
        if self.panic_pending {
            for (pitch, active) in self.active_notes.iter_mut().enumerate() {
                for _ in 0..*active {
                    commands.push(BlockCommand::SendMidiAt {
                        event: MidiEvent::NoteOff {
                            channel: MidiChannel::Ch1,
                            note: pitch as u8,
                            velocity: 0,
                        },
                        sample_offset: 0,
                    });
                }
                *active = 0;
            }
            for voice in &self.expressive_notes {
                commands.push(
                    ExpressiveMidiMessage {
                        offset: 0,
                        data: ExpressiveMidiData::NoteOff {
                            channel: voice.channel,
                            pitch: voice.pitch,
                            velocity: ExpressiveVelocity::Midi2(0),
                            note_id: voice.note_id,
                            tuning: 0.0,
                        },
                    }
                    .block_command(),
                );
            }
            self.panic_pending = false;
        }
        let (mut legacy_index, mut expressive_index) = (0, 0);
        while legacy_index < events.len() || expressive_index < expressive.len() {
            if legacy_index < events.len()
                && (expressive_index == expressive.len()
                    || events[legacy_index].offset <= expressive[expressive_index].offset)
            {
                let event = events[legacy_index];
                commands.push(BlockCommand::SendMidiAt {
                    event: event.as_host_event(),
                    sample_offset: event.offset as i32,
                });
                legacy_index += 1;
            } else {
                commands.push(expressive[expressive_index].block_command());
                expressive_index += 1;
            }
        }
        self.active_notes = next_active;
        std::mem::swap(&mut self.expressive_notes, &mut self.next_expressive_notes);
        let result = self
            .plugin
            .process_bus_audio_block(&mut self.buffers, &mut commands);
        commands.clear();
        self.block_commands = commands;
        if let Err(error) = result {
            for bus in &mut self.buffers.outputs {
                for channel in &mut bus.channels {
                    channel.fill(0.0);
                }
            }
            return Err(error.into());
        }
        if let Some((position, bpm, playing)) = queued_transport {
            self.next_position = Some(position);
            self.cached_tempo = Some(bpm);
            self.cached_playing = Some(playing);
        }
        if self.cached_playing == Some(true)
            && let (Some(position), Some(tempo)) = (&mut self.next_position, self.cached_tempo)
        {
            position.samples = position.samples.saturating_add(frames as i64);
            position.quarter_notes += frames as f64 / self.buffers.sample_rate * (tempo / 60.0);
        }
        Ok(())
    }

    /// Copy one stereo pair or a duplicated final mono output without further processing.
    pub fn copy_output_pair(
        &self,
        bus: usize,
        first_channel: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) -> Result<()> {
        left.fill(0.0);
        right.fill(0.0);
        validate_block(&[], left.len(), right.len(), self.max_block)?;
        ensure!(
            left.len() == self.buffers.block_size,
            "Plug-in output length differs from last rendered block"
        );
        validate_pair(&self.buffers.outputs, bus, first_channel)?;
        let channels = &self.buffers.outputs[bus].channels;
        left.copy_from_slice(&channels[first_channel][..left.len()]);
        right.copy_from_slice(
            &channels
                .get(first_channel + 1)
                .unwrap_or(&channels[first_channel])[..right.len()],
        );
        Ok(())
    }

    /// Enumerate parameters on a worker/control thread, with playback stopped.
    /// Large metadata replies get a separate 30-second helper budget; the active
    /// processing instance retains its live/offline deadline. Current values are then
    /// read from that instance, without requiring plug-in state serialization.
    pub fn parameters(&self) -> Result<Vec<ParameterInfo>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut metadata = PluginHostProcess::new(Some(helper_path()?), Duration::from_secs(30))
            .map_err(anyhow::Error::msg)
            .context("Starting isolated parameter discovery")?;
        let loaded = metadata
            .send_command(HostCommand::LoadPlugin {
                path: self.info.path.to_string_lossy().into_owned(),
                sample_rate: self.buffers.sample_rate,
                block_size: self.max_block as u32,
                tempo: self.cached_tempo.unwrap_or(120.0),
                time_sig_numerator: 4,
                time_sig_denominator: 4,
                class_id: self.info.class_id.clone(),
            })
            .map_err(anyhow::Error::msg)
            .context("Loading isolated parameter metadata")?;
        ensure!(
            matches!(loaded, HostResponse::PluginInfo { .. }),
            "Metadata load failed: {loaded:?}"
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !remaining.is_zero(),
            "Parameter discovery exceeded its 30-second deadline"
        );
        metadata.set_timeout(remaining);
        let response = metadata
            .send_command(HostCommand::GetAllParameters)
            .map_err(anyhow::Error::msg)
            .context("Enumerating isolated parameter metadata")?;
        let HostResponse::Parameters { params } = response else {
            anyhow::bail!("Parameter discovery failed: {response:?}");
        };
        drop(metadata);
        params
            .into_iter()
            .map(|p| {
                ensure!(
                    Instant::now() < deadline,
                    "Parameter discovery exceeded its 30-second deadline"
                );
                let value = self
                    .plugin
                    .get_parameter(p.id)
                    .with_context(|| format!("Reading current value of parameter {}", p.name))?;
                Ok(ParameterInfo {
                    id: p.id,
                    name: p.name,
                    value,
                    default: p.default,
                    unit: p.unit,
                    steps: p.step_count,
                    read_only: p.is_read_only,
                })
            })
            .collect()
    }

    /// Set a normalized parameter serially between processing blocks on the owning
    /// render/control thread. The isolated helper applies it before the next block;
    /// this bounded IPC call must never run in the hardware audio callback.
    pub fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "Parameter value must be normalized to 0..=1"
        );
        self.plugin.set_parameter(id, value)?;
        Ok(())
    }

    /// Serialize opaque component and controller state on the original control thread.
    pub fn save_state(&self) -> Result<Vec<u8>> {
        Ok(self.plugin.save_state()?)
    }

    /// Restore state on the original control thread while the audio callback is stopped.
    pub fn load_state(&mut self, state: &[u8]) -> Result<()> {
        self.plugin.stop_processing()?;
        self.plugin.load_state(state)?;
        self.refresh_bus_buffers()?;
        self.plugin.start_processing()?;
        self.panic_pending = true;
        self.latency = self.plugin.latency_samples();
        self.tail = self.plugin.tail_samples();
        self.next_position = None;
        Ok(())
    }
}

/// A stereo VST3 insert or send effect, isolated exactly like an instrument.
/// Unrouted additional/sidechain buses are silent. Calls belong on the engine worker, never the
/// audio device callback, because each block uses synchronous IPC.
pub struct VstEffect {
    processor: VstInstrument,
}

impl VstEffect {
    pub fn load(path: &Path, sample_rate: f64, max_block: usize) -> Result<Self> {
        Self::load_class(path, None, sample_rate, max_block)
    }

    pub fn load_info(info: &PluginInfo, sample_rate: f64, max_block: usize) -> Result<Self> {
        Self::load_class(&info.path, info.class_id.as_deref(), sample_rate, max_block)
    }

    pub fn load_class(
        path: &Path,
        class_id: Option<&str>,
        sample_rate: f64,
        max_block: usize,
    ) -> Result<Self> {
        Self::load_class_with_mode(
            path,
            class_id,
            sample_rate,
            max_block,
            ProcessingMode::Realtime,
        )
    }

    /// Load an insert with the same explicit live/offline policy as instruments.
    pub fn load_class_with_mode(
        path: &Path,
        class_id: Option<&str>,
        sample_rate: f64,
        max_block: usize,
        mode: ProcessingMode,
    ) -> Result<Self> {
        Ok(Self {
            processor: VstInstrument::load_processor(
                path,
                class_id,
                sample_rate,
                max_block,
                false,
                mode,
            )?,
        })
    }

    pub fn info(&self) -> &PluginInfo {
        self.processor.info()
    }
    pub fn latency_samples(&self) -> u32 {
        self.processor.latency_samples()
    }
    pub fn tail_samples(&self) -> u32 {
        self.processor.tail_samples()
    }
    pub fn isolation_pid(&self) -> Option<u32> {
        self.processor.isolation_pid()
    }
    pub fn main_input_bus(&self) -> Option<usize> {
        self.processor.main_input_bus()
    }
    pub fn main_output_bus(&self) -> usize {
        self.processor.main_output_bus()
    }
    pub fn audio_bus_layout(&self) -> AudioBusLayout {
        self.processor.audio_bus_layout()
    }
    pub fn audio_bus_details(&self) -> Result<AudioBusDetailsLayout> {
        self.processor.audio_bus_details()
    }
    pub fn configure_audio_buses(&mut self, inputs: &[usize], outputs: &[usize]) -> Result<()> {
        self.processor.configure_audio_buses(inputs, outputs)
    }
    pub fn process_bus_inputs(
        &mut self,
        inputs: &[StereoBusInput<'_>],
        frames: usize,
    ) -> Result<()> {
        self.processor.process_bus_inputs(&[], inputs, frames)
    }
    pub fn copy_output_pair(
        &self,
        bus: usize,
        first_channel: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) -> Result<()> {
        self.processor
            .copy_output_pair(bus, first_channel, left, right)
    }
    pub fn process_buses(
        &mut self,
        inputs: &[StereoBusInput<'_>],
        outputs: &mut [StereoBusOutput<'_>],
    ) -> Result<()> {
        self.processor.process_buses(&[], inputs, outputs)
    }
    pub fn parameters(&self) -> Result<Vec<ParameterInfo>> {
        self.processor.parameters()
    }
    pub fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        self.processor.set_parameter(id, value)
    }
    pub fn save_state(&self) -> Result<Vec<u8>> {
        self.processor.save_state()
    }
    pub fn load_state(&mut self, state: &[u8]) -> Result<()> {
        self.processor.load_state(state)
    }
    pub fn recover(&mut self) -> Result<()> {
        self.processor.recover()
    }

    /// Deactivate and reactivate the effect to reset processing, keeping parameter state.
    pub fn reset(&mut self) -> Result<()> {
        self.processor.plugin.stop_processing()?;
        self.processor.plugin.start_processing()?;
        self.processor.panic_pending = false;
        self.processor.next_position = None;
        Ok(())
    }

    pub fn queue_parameter_for_block(&mut self, id: u32, value: f64) -> Result<()> {
        self.processor.queue_parameter_for_block(id, value)
    }
    pub fn queue_transport_for_block(
        &mut self,
        sample_position: i64,
        quarter_notes: f64,
        bpm: f64,
        playing: bool,
    ) -> Result<()> {
        self.processor
            .queue_transport_for_block(sample_position, quarter_notes, bpm, playing)
    }
    pub fn set_block_transport(&mut self, policy: BlockTransport) -> Result<()> {
        self.processor.set_block_transport(policy)
    }
    pub fn ipc_stats(&self) -> Option<IpcStats> {
        self.processor.ipc_stats()
    }

    pub fn set_transport(
        &mut self,
        sample_position: i64,
        quarter_notes: f64,
        bpm: f64,
        playing: bool,
    ) -> Result<()> {
        self.processor
            .set_transport(sample_position, quarter_notes, bpm, playing)
    }

    pub fn process(
        &mut self,
        input_left: &[f32],
        input_right: &[f32],
        output_left: &mut [f32],
        output_right: &mut [f32],
    ) -> Result<()> {
        self.processor
            .process_with_input(&[], input_left, input_right, output_left, output_right)
    }
}

/// Keep panic releases plus a maximum new block below the SDK's 4,096-event queue.
/// The count tracks overlapping notes at the same pitch, including zero-velocity offs.
fn track_active_notes(
    active: &[u16; 128],
    reset: bool,
    events: &[MidiMessage],
) -> Result<[u16; 128]> {
    let mut next = if reset { [0; 128] } else { *active };
    for event in events {
        let count = &mut next[event.pitch as usize];
        *count = if event.on && event.velocity > 0 {
            count.saturating_add(1)
        } else {
            count.saturating_sub(1)
        };
    }
    ensure!(
        next.iter().map(|&count| usize::from(count)).sum::<usize>() <= MAX_MIDI_EVENTS_PER_BLOCK,
        "Instrument exceeds the 2,048 held-note limit"
    );
    Ok(next)
}

fn validate_block(
    events: &[MidiMessage],
    left_len: usize,
    right_len: usize,
    max_block: usize,
) -> Result<()> {
    ensure!(left_len == right_len, "Plugin stereo output lengths differ");
    ensure!(
        left_len <= max_block,
        "Plugin block exceeds configured maximum"
    );
    ensure!(
        events.len() <= MAX_MIDI_EVENTS_PER_BLOCK,
        "Too many MIDI events in one block"
    );
    let mut previous_offset = 0;
    for event in events {
        ensure!(
            event.pitch <= 127 && event.velocity <= 127,
            "MIDI pitch and velocity must be 0..=127"
        );
        ensure!(
            (event.offset as usize) < left_len,
            "MIDI offset is outside the processing block"
        );
        ensure!(
            event.offset >= previous_offset,
            "MIDI events must be ordered by sample offset"
        );
        previous_offset = event.offset;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_audio_unit_locator_lists_nothing_off_macos() {
        let locator = PathBuf::from(audio_unit::AU_LOCATOR);
        assert!(scan_paths(&[locator]).unwrap().is_empty());
    }

    #[test]
    fn native_bus_layout_admission_rejects_unbounded_allocations() {
        let bus = AudioBusConfig {
            channel_count: 2,
            active: true,
        };
        let normal = AudioBusLayout {
            inputs: vec![bus; 2],
            outputs: vec![bus; 16],
        };
        assert!(validate_audio_bus_layout(&normal, 512).is_ok());
        let too_many = AudioBusLayout {
            inputs: vec![bus; 65],
            outputs: vec![bus],
        };
        assert!(validate_audio_bus_layout(&too_many, 64).is_err());
        let huge = AudioBusLayout {
            inputs: vec![],
            outputs: vec![AudioBusConfig {
                channel_count: usize::MAX,
                active: true,
            }],
        };
        assert!(validate_audio_bus_layout(&huge, 64).is_err());
        assert!(validate_audio_bus_layout(&normal, usize::MAX).is_err());
        let aggregate = AudioBusLayout {
            inputs: vec![AudioBusConfig {
                channel_count: 256,
                active: true,
            }],
            outputs: vec![bus],
        };
        assert!(validate_audio_bus_layout(&aggregate, 512).is_err());
    }

    #[test]
    fn overlapping_notes_preserve_release_counts_and_bound_reset_queue() {
        let on = MidiMessage {
            offset: 0,
            pitch: 60,
            velocity: 100,
            on: true,
        };
        let active = track_active_notes(&[0; 128], false, &[on, on]).unwrap();
        assert_eq!(active[60], 2);
        let zero_velocity = MidiMessage { velocity: 0, ..on };
        let remaining = track_active_notes(&active, false, &[zero_velocity]).unwrap();
        assert_eq!(remaining[60], 1);
        assert_eq!(track_active_notes(&active, true, &[on]).unwrap()[60], 1);
        let mut full = [0; 128];
        full[60] = MAX_MIDI_EVENTS_PER_BLOCK as u16;
        assert!(track_active_notes(&full, false, &[on]).is_err());
        assert_eq!(track_active_notes(&full, true, &[on]).unwrap()[60], 1);
    }

    #[test]
    fn instrument_is_send_without_an_unsafe_wrapper() {
        fn assert_send<T: Send>() {}
        assert_send::<VstInstrument>();
        assert_send::<VstEffect>();
    }

    #[test]
    fn audio_bus_input_routes_stereo_and_downmixes_mono() {
        let left = [0.5, -0.2, 0.8];
        let right = [-0.5, 0.4, 0.2];
        let mut mono = vec![vec![0.0; 3]];
        copy_stereo_input(&mut mono, &left, &right);
        assert_eq!(mono[0], [0.0, 0.1, 0.5]);
        let mut multichannel = vec![vec![0.0; 3]; 4];
        copy_stereo_input(&mut multichannel, &left, &right);
        assert_eq!(multichannel[0], left);
        assert_eq!(multichannel[1], right);
        assert_eq!(multichannel[2], [0.0; 3]);
    }

    /// DAW_TEST_VST3_EFFECT=/path/to/effect.vst3 cargo test -p daw-plugins installed_effect -- --ignored
    #[test]
    #[ignore = "requires a user-installed effect in DAW_TEST_VST3_EFFECT"]
    fn installed_effect_processes_input_and_roundtrips_state() -> Result<()> {
        let path = std::env::var_os("DAW_TEST_VST3_EFFECT")
            .context("Set DAW_TEST_VST3_EFFECT to an effect bundle")?;
        let mut effect = VstEffect::load(Path::new(&path), 48_000.0, 256)?;
        assert_ne!(effect.isolation_pid(), Some(std::process::id()));
        let state = effect.save_state()?;
        let mut input = [0.0; 256];
        let mut left = [0.0; 256];
        let mut right = [0.0; 256];
        let mut peak = 0.0_f32;
        let mut difference = 0.0_f32;
        for block in 0..200 {
            for (index, sample) in input.iter_mut().enumerate() {
                *sample = ((block * 256 + index) as f32 * 440.0 / 48_000.0 * std::f32::consts::TAU)
                    .sin()
                    * 0.2;
            }
            let sample_position = (block * 256) as i64;
            effect.set_transport(
                sample_position,
                sample_position as f64 / 24_000.0,
                120.0,
                true,
            )?;
            effect.process(&input, &input, &mut left, &mut right)?;
            assert!(left.iter().chain(&right).all(|sample| sample.is_finite()));
            peak = left
                .iter()
                .chain(&right)
                .fold(peak, |peak, sample| peak.max(sample.abs()));
            difference = left
                .iter()
                .zip(input)
                .fold(difference, |peak, (out, input)| {
                    peak.max((out - input).abs())
                });
        }
        assert!(peak > 0.000_01, "Effect rendered only silence");
        assert!(difference > 0.000_01, "Effect never modified its input");
        let position = effect.processor.plugin.transport_position()?;
        assert_eq!(position.samples, 51_200);
        assert!((position.quarter_notes - 51_200.0 / 24_000.0).abs() < 1.0e-9);
        effect.set_transport(1_000, 1_000.0 / 24_000.0, 120.0, false)?;
        effect.process(&input, &input, &mut left, &mut right)?;
        assert_eq!(effect.processor.plugin.transport_position()?.samples, 1_000);
        effect.reset()?;
        effect.process(
            &input[..17],
            &input[..17],
            &mut left[..17],
            &mut right[..17],
        )?;
        effect.load_state(&state)?;
        effect.process(&input, &input, &mut left, &mut right)?;
        eprintln!(
            "{}: peak={peak:.4}, changed_by={difference:.4}, latency={} samples",
            effect.info().name,
            effect.latency_samples()
        );
        Ok(())
    }

    #[test]
    fn scans_nested_bundles_and_direct_paths_without_loading_code() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let bundle = temp.path().join("Vendor").join("Fake Synth.vst3");
        std::fs::create_dir_all(bundle.join("nested.vst3"))?;
        let found = scan_paths(&[
            temp.path().to_owned(),
            bundle.clone(),
            temp.path().join("missing"),
        ])?;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Fake Synth");
        assert_eq!(found[0].path, bundle.canonicalize()?);
        assert_eq!(found[0].is_instrument(), None);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loops_and_duplicate_bundles_terminate() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::fs::create_dir(temp.path().join("Synth.vst3"))?;
        std::os::unix::fs::symlink(temp.path(), temp.path().join("loop"))?;
        std::os::unix::fs::symlink(
            temp.path().join("Synth.vst3"),
            temp.path().join("Alias.vst3"),
        )?;
        assert_eq!(scan_paths(&[temp.path().to_owned()])?.len(), 1);
        Ok(())
    }

    #[test]
    fn module_metadata_preserves_processor_classes() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let bundle = temp.path().join("Test.vst3");
        let resources = bundle.join("Contents/Resources");
        std::fs::create_dir_all(&resources)?;
        std::fs::write(
            resources.join("moduleinfo.json"),
            serde_json::json!({
                "Name": "Bundle", "Version": "1.0", "Factory Info": { "Vendor": "Acme" },
                "Classes": [{
                    "CID": "0123456789ABCDEF0123456789ABCDEF", "Category": "Audio Module Class",
                    "Name": "Declared Synth", "Sub Categories": ["Instrument", "Synth"],
                    "Version": "1.0", "SDKVersion": "VST 3.7.9", "Cardinality": 2147483647
                }]
            })
            .to_string(),
        )?;
        let found = scan_paths(&[bundle])?;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Declared Synth");
        assert_eq!(found[0].vendor, "Acme");
        assert_eq!(found[0].is_instrument(), Some(true));
        assert_eq!(
            found[0].class_id.as_deref(),
            Some("0123456789ABCDEF0123456789ABCDEF")
        );
        Ok(())
    }

    #[test]
    fn rejects_bad_blocks_before_the_plugin_sees_events() {
        let note = MidiMessage {
            offset: 17,
            pitch: 60,
            velocity: 100,
            on: true,
        };
        assert!(validate_block(&[note], 64, 64, 128).is_ok());
        assert!(validate_block(&[note], 17, 17, 128).is_err());
        assert!(validate_block(&[], 64, 32, 128).is_err());
        assert!(validate_block(&[], 256, 256, 128).is_err());
        assert!(validate_block(&[MidiMessage { pitch: 200, ..note }], 64, 64, 128).is_err());
        assert!(validate_block(&[note, MidiMessage { offset: 2, ..note }], 64, 64, 128).is_err());
        assert!(validate_block(&vec![note; MAX_MIDI_EVENTS_PER_BLOCK + 1], 64, 64, 128).is_err());
        assert!(validate_block(&[], 0, 0, 128).is_ok());
    }

    #[test]
    fn zero_velocity_note_on_is_a_note_off() {
        let note = MidiMessage {
            offset: 0,
            pitch: 60,
            velocity: 0,
            on: true,
        };
        assert!(matches!(
            note.as_host_event(),
            MidiEvent::NoteOff { note: 60, .. }
        ));
    }

    /// DAW_TEST_VST3=/path/to/instrument.vst3 cargo test -p daw-plugins installed_instrument -- --ignored
    /// Uses the installed plugin's current/default patch; choose one that emits sound.
    #[test]
    #[ignore = "requires a user-installed VST3 instrument in DAW_TEST_VST3"]
    fn installed_instrument_produces_audio_and_roundtrips_state() -> Result<()> {
        let path = std::env::var_os("DAW_TEST_VST3")
            .context("Set DAW_TEST_VST3 to an instrument bundle")?;
        let mut instrument = VstInstrument::load(Path::new(&path), 48_000.0, 256)?;
        assert_ne!(instrument.isolation_pid(), Some(std::process::id()));
        instrument.set_transport(0, 0.0, 120.0, true)?;
        let state = instrument.save_state()?;
        let mut left = [0.0; 256];
        let mut right = [0.0; 256];
        let note = MidiMessage {
            offset: 37,
            pitch: 60,
            velocity: 110,
            on: true,
        };
        let mut peak = 0.0_f32;
        for block in 0..200 {
            let events = if block == 0 {
                std::slice::from_ref(&note)
            } else {
                &[]
            };
            instrument.process(events, &mut left, &mut right)?;
            assert!(left.iter().chain(&right).all(|sample| sample.is_finite()));
            peak = left
                .iter()
                .chain(&right)
                .fold(peak, |peak, sample| peak.max(sample.abs()));
        }
        assert!(peak > 0.000_01, "Plugin rendered only silence");
        instrument.reset()?;
        instrument.process(&[], &mut left[..17], &mut right[..17])?;
        instrument.process(&[], &mut left, &mut right)?;
        instrument.load_state(&state)?;
        instrument.process(&[], &mut left, &mut right)?;
        #[cfg(unix)]
        {
            let original_pid = instrument.isolation_pid().unwrap();
            let status = std::process::Command::new("/bin/kill")
                .args(["-KILL", &original_pid.to_string()])
                .status()?;
            assert!(status.success());
            assert!(instrument.process(&[], &mut left, &mut right).is_err());
            assert!(left.iter().chain(&right).all(|sample| *sample == 0.0));
            instrument.recover()?;
            assert_ne!(instrument.isolation_pid(), Some(original_pid));
            instrument.load_state(&state)?;
            instrument.process(&[note], &mut left, &mut right)?;
        }
        eprintln!(
            "{}: peak={peak:.4}, latency={} samples",
            instrument.info().name,
            instrument.latency_samples()
        );
        Ok(())
    }
}
