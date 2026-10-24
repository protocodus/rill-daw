//! VST3 instrument discovery and processing, separate from the audio device backend.
//!
//! Scanning reads directory entries and optional `moduleinfo.json` files; it never loads
//! native plugin code. Each selected instrument runs in a separate `daw-plugin-host`
//! process, which contains crashes and hangs. This is process crash isolation, not an OS
//! permission sandbox: the child retains the user's file and network access. Native plugin
//! code is never loaded into the DAW process. No in-process fallback is available.
//!
//! IPC is synchronous and allocates while serializing blocks, so render on a worker/control
//! thread and give the device callback prepared audio. Never call this adapter on a hard
//! real-time device callback. A hung command is bounded by a timeout and the child is killed.
//! Only the first active output bus is routed, with mono duplicated to stereo.
//!
//! API references: <https://docs.rs/vst3-host/0.10.0/vst3_host/> and
//! <https://github.com/steinbergmedia/vst3_pluginterfaces>.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vst3_host::process_isolation::{HostCommand, HostResponse, PluginHostProcess};
use vst3_host::{BusAudioBuffers, MidiChannel, MidiEvent, Plugin, TransportPosition, Vst3Host};

pub mod catalog;
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
pub use catalog::{CatalogEntry, PluginCatalog, ThumbnailStatus};
pub use editor::{EditorEvent, EditorSession, capture_editor, capture_editor_info, launch_editor};

/// Leaves headroom in the host's 4,096-event queue for panic note-offs.
pub const MAX_MIDI_EVENTS_PER_BLOCK: usize = 2_048;

/// Catalogue entry. Missing metadata means the bundle has not been inspected by loading it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub path: PathBuf,
    pub vendor: String,
    pub category: String,
    /// Present when a bundle's metadata identifies a particular processor class.
    pub class_id: Option<String>,
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

/// Locations specified by the VST3 convention, plus `VST3_PATH` when set.
pub fn default_locations() -> Vec<PathBuf> {
    let mut paths = vst3_host::discovery::scan_standard_paths();
    if let Some(extra) = std::env::var_os("VST3_PATH") {
        paths.extend(std::env::split_paths(&extra));
    }
    paths
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

/// Recursively scan directories or individual `.vst3` bundles/files.
///
/// Missing roots are ignored. Symbolic-link cycles and duplicate roots are collapsed by
/// canonical path. Known effects are included so callers can decide how to display them.
/// Malformed or missing metadata falls back to the bundle name, allowing manual loading.
pub fn scan_paths(paths: &[PathBuf]) -> Result<Vec<PluginInfo>> {
    let mut pending = paths.to_vec();
    let mut visited = HashSet::new();
    let mut found = Vec::new();
    while let Some(path) = pending.pop() {
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
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("vst3"))
}

fn bundle_info(path: &Path) -> Vec<PluginInfo> {
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
        vendor: String::new(),
        category: String::new(),
        class_id: None,
    }]
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
        Self::load_processor(path, class_id, sample_rate, max_block, true)
    }

    fn load_processor(
        path: &Path,
        class_id: Option<&str>,
        sample_rate: f64,
        max_block: usize,
        require_midi: bool,
    ) -> Result<Self> {
        ensure!(
            sample_rate.is_finite() && (8_000.0..=768_000.0).contains(&sample_rate),
            "Invalid plugin sample rate"
        );
        ensure!(
            (1..=65_536).contains(&max_block),
            "Invalid plugin maximum block size"
        );
        ensure!(is_bundle(path), "Expected a .vst3 bundle or file");
        let mut host = Vst3Host::builder()
            .sample_rate(sample_rate)
            .block_size(max_block)
            .input_channels(if require_midi { 0 } else { 2 })
            .output_channels(2)
            .with_process_isolation(true)
            .helper_path(helper_path()?)
            .response_timeout(std::time::Duration::from_millis(100))
            .auto_recover_plugins(false)
            .build()?;
        let mut plugin = match class_id {
            Some(id) => host.load_plugin_class(path, id),
            None => host.load_plugin(path),
        }
        .with_context(|| format!("Loading VST3 processor {}", path.display()))?;
        ensure!(
            !require_midi || plugin.info().has_midi_input,
            "{} does not accept MIDI input",
            plugin.info().name
        );
        let buffers = plugin.create_bus_audio_buffers(max_block)?;
        let input_bus = buffers
            .inputs
            .iter()
            .position(|bus| bus.active && !bus.channels.is_empty());
        ensure!(
            require_midi || input_bus.is_some(),
            "Effect has no active audio input bus"
        );
        let output_bus = buffers
            .outputs
            .iter()
            .position(|bus| bus.active && !bus.channels.is_empty())
            .context("Plugin has no active audio output bus")?;
        let raw = plugin.info();
        let info = PluginInfo {
            name: raw.name.clone(),
            path: path.to_owned(),
            vendor: raw.vendor.clone(),
            category: raw.category.clone(),
            class_id: Some(raw.uid.clone()),
        };
        plugin.start_processing()?;
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

    /// Restart a failed child on a worker/control thread. The plugin returns to its default
    /// preset; call `load_state` with the saved session state to restore its settings.
    pub fn recover(&mut self) -> Result<()> {
        self.plugin.recover()?;
        self.buffers = self.plugin.create_bus_audio_buffers(self.max_block)?;
        self.input_bus = self
            .buffers
            .inputs
            .iter()
            .position(|bus| bus.active && !bus.channels.is_empty());
        self.output_bus = self
            .buffers
            .outputs
            .iter()
            .position(|bus| bus.active && !bus.channels.is_empty())
            .context("Recovered plugin has no active output bus")?;
        self.latency = self.plugin.latency_samples();
        self.tail = self.plugin.tail_samples();
        self.panic_pending = false;
        self.active_notes.fill(0);
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
        validate_block(events, left.len(), right.len(), self.max_block)?;
        if let Some((input_left, input_right)) = inputs {
            ensure!(
                input_left.len() == left.len() && input_right.len() == left.len(),
                "Plugin input and output lengths differ"
            );
            ensure!(
                self.input_bus.is_some(),
                "Plugin has no active audio input bus"
            );
        }
        if left.is_empty() {
            return Ok(());
        }
        let next_active = track_active_notes(&self.active_notes, self.panic_pending, events)?;
        self.buffers.block_size = left.len();
        for bus in self
            .buffers
            .inputs
            .iter_mut()
            .chain(&mut self.buffers.outputs)
        {
            for channel in &mut bus.channels {
                // Capacity was reserved to max_block while loading, including after a short block.
                channel.resize(left.len(), 0.0);
                channel.fill(0.0);
            }
        }
        if let Some((input_left, input_right)) = inputs {
            let bus_index = self
                .input_bus
                .context("Plugin has no active audio input bus")?;
            copy_stereo_input(
                &mut self.buffers.inputs[bus_index].channels,
                input_left,
                input_right,
            );
        }
        if self.panic_pending {
            for (pitch, active) in self.active_notes.iter_mut().enumerate() {
                for _ in 0..*active {
                    self.plugin.send_midi_event_at(
                        MidiEvent::NoteOff {
                            channel: MidiChannel::Ch1,
                            note: pitch as u8,
                            velocity: 0,
                        },
                        0,
                    )?;
                }
                *active = 0;
            }
            self.panic_pending = false;
        }
        for event in events {
            self.plugin
                .send_midi_event_at(event.as_host_event(), event.offset as i32)?;
        }
        self.active_notes = next_active;
        self.plugin.process_bus_audio(&mut self.buffers)?;
        if self.cached_playing == Some(true)
            && let (Some(position), Some(tempo)) = (&mut self.next_position, self.cached_tempo)
        {
            position.samples = position.samples.saturating_add(left.len() as i64);
            position.quarter_notes += left.len() as f64 / self.buffers.sample_rate * (tempo / 60.0);
        }
        let output = &self.buffers.outputs[self.output_bus].channels;
        left.copy_from_slice(&output[0]);
        right.copy_from_slice(output.get(1).unwrap_or(&output[0]));
        Ok(())
    }

    /// Enumerate parameters on a worker/control thread, with playback stopped.
    /// Large metadata replies get a separate 30-second helper budget; the active
    /// processing instance retains its 100 ms deadline. Current values are then
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
        let result = self.plugin.load_state(state);
        self.plugin.start_processing()?;
        result?;
        // A preset may change bus layout. Reallocate only here on the control thread.
        self.buffers = self.plugin.create_bus_audio_buffers(self.max_block)?;
        self.input_bus = self
            .buffers
            .inputs
            .iter()
            .position(|bus| bus.active && !bus.channels.is_empty());
        self.output_bus = self
            .buffers
            .outputs
            .iter()
            .position(|bus| bus.active && !bus.channels.is_empty())
            .context("Restored plugin has no active output bus")?;
        self.panic_pending = true;
        self.latency = self.plugin.latency_samples();
        self.tail = self.plugin.tail_samples();
        self.next_position = None;
        Ok(())
    }
}

/// A stereo VST3 insert or send effect, isolated exactly like an instrument.
/// Additional/sidechain buses are silent. Calls belong on the engine worker, never the
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
        Ok(Self {
            processor: VstInstrument::load_processor(
                path,
                class_id,
                sample_rate,
                max_block,
                false,
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

fn copy_stereo_input(channels: &mut [Vec<f32>], left: &[f32], right: &[f32]) {
    if channels.len() == 1 {
        for ((output, left), right) in channels[0].iter_mut().zip(left).zip(right) {
            *output = (left + right) * 0.5;
        }
    } else if channels.len() >= 2 {
        channels[0].copy_from_slice(left);
        channels[1].copy_from_slice(right);
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
