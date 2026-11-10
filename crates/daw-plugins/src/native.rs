//! The format seam inside the isolated helper. The parent speaks one command protocol
//! (`vst3_host::process_isolation`); each hosted format adapts it behind this trait.
//! Adding a format means one adapter module plus one arm in [`load`].
use crate::PluginFormat;
use anyhow::{Result, bail};
use std::path::Path;
use vst3_host::{
    AudioBusLayout, BusAudioBuffers, MidiChannel, MidiEvent, Plugin, Vst3Host,
    audio::{AudioBusDetailsLayout, BusDirection, MediaType},
    midi::{MidiController, MidiControllerAssignment, NoteExpressionInfo, PluginEvent},
    parameters::Parameter,
    plugin::{ProcessMode, StateContext, TransportPosition},
};

/// VST3 `RestartFlags` bits, reused as the wire vocabulary for every format.
pub(crate) const RESTART_IO_CHANGED: i32 = 1 << 1;
pub(crate) const RESTART_PARAM_VALUES_CHANGED: i32 = 1 << 2;
pub(crate) const RESTART_LATENCY_CHANGED: i32 = 1 << 3;
pub(crate) const RESTART_PARAM_TITLES_CHANGED: i32 = 1 << 4;

/// Plug-in GUI gestures and value changes, drained by the editor helper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ParameterEdit {
    Begin(u32),
    Value(u32, f64),
}

/// Identity reported to the parent after loading.
pub(crate) struct LoadedInfo {
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub category: String,
    pub uid: String,
    pub has_gui: bool,
    pub has_midi_input: bool,
    pub has_midi_output: bool,
}

/// Engine-facing operations every hosted format provides. All calls arrive on the
/// helper's main thread, one at a time; `process_bus_audio` is the only audio call.
pub(crate) trait NativeProcessor {
    fn loaded_info(&self) -> LoadedInfo;
    fn start_processing(&mut self) -> Result<()>;
    fn stop_processing(&mut self) -> Result<()>;
    fn is_processing(&self) -> bool;
    fn reconfigure(&mut self, sample_rate: f64, block_size: usize) -> Result<()>;
    fn set_process_mode(&mut self, mode: ProcessMode) -> Result<()>;
    /// VST3-style restart bits accumulated since the previous call.
    fn service_host_requests(&mut self) -> Result<i32>;
    fn audio_bus_layout(&self) -> Result<AudioBusLayout>;
    fn audio_bus_details(&self) -> Result<AudioBusDetailsLayout>;
    fn set_bus_active(
        &mut self,
        media: MediaType,
        direction: BusDirection,
        index: i32,
        active: bool,
    ) -> Result<()>;
    fn set_transport_position(&mut self, position: TransportPosition) -> Result<()>;
    fn transport_position(&self) -> Result<TransportPosition>;
    fn set_tempo(&mut self, bpm: f64) -> Result<()>;
    fn set_playing(&mut self, playing: bool) -> Result<()>;
    fn set_time_signature(&mut self, numerator: i32, denominator: i32) -> Result<()>;
    /// Parameter values are normalized to 0..=1 on the wire.
    fn get_parameters(&mut self) -> Result<Vec<Parameter>>;
    fn get_parameter(&mut self, id: u32) -> Result<f64>;
    fn set_parameter(&mut self, id: u32, value: f64) -> Result<()>;
    fn set_parameter_at(&mut self, id: u32, value: f64, offset: i32) -> Result<()>;
    fn format_parameter(&mut self, id: u32, value: f64) -> Result<String>;
    fn send_midi_event_at(&mut self, event: MidiEvent, offset: i32) -> Result<()>;
    fn send_plugin_event(&mut self, event: PluginEvent) -> Result<()>;
    fn send_midi_controller_at(
        &mut self,
        _bus: i32,
        _channel: MidiChannel,
        _controller: MidiController,
        _value: f64,
        _offset: i32,
    ) -> Result<()> {
        bail!("This plug-in has no mapped MIDI controllers")
    }
    fn note_expressions_on(&self, _bus: i32, _channel: u8) -> Result<Vec<NoteExpressionInfo>> {
        Ok(Vec::new())
    }
    fn midi_controller_assignments(
        &self,
        _direction: BusDirection,
    ) -> Result<Vec<MidiControllerAssignment>> {
        Ok(Vec::new())
    }
    fn midi_panic(&mut self) -> Result<()>;
    fn process_bus_audio(&mut self, buffers: &mut BusAudioBuffers) -> Result<()>;
    fn take_output_events(&mut self) -> Vec<PluginEvent> {
        Vec::new()
    }
    fn latency_samples(&self) -> u32;
    fn tail_samples(&self) -> u32;
    fn save_state(&mut self) -> Result<Vec<u8>>;
    fn load_state(&mut self, data: &[u8]) -> Result<()>;
    /// Main-thread housekeeping (callbacks, timers) before each helper command.
    fn service(&mut self) {}
    /// VST3 needs its concrete type for protocol commands no other format has.
    fn as_vst3(&mut self) -> Option<&mut Plugin> {
        None
    }
}

pub(crate) struct LoadRequest<'a> {
    pub path: &'a Path,
    pub class_id: Option<&'a str>,
    pub sample_rate: f64,
    pub block_size: usize,
    pub tempo: f64,
    pub time_signature: (i32, i32),
}

pub(crate) fn load(request: LoadRequest<'_>) -> Result<Box<dyn NativeProcessor>> {
    match PluginFormat::from_path(request.path) {
        PluginFormat::Vst3 => {
            let mut host = Vst3Host::builder()
                .with_process_isolation(false)
                .sample_rate(request.sample_rate)
                .block_size(request.block_size)
                .tempo(request.tempo)
                .time_signature(request.time_signature.0, request.time_signature.1)
                .build()?;
            Ok(Box::new(match request.class_id {
                Some(id) => host.load_plugin_class(request.path, id)?,
                None => host.load_plugin(request.path)?,
            }))
        }
        PluginFormat::Clap => Ok(Box::new(crate::clap::ClapProcessor::load(&request)?)),
        #[cfg(target_os = "linux")]
        PluginFormat::Lv2 => crate::lv2::load(&request),
        #[cfg(not(target_os = "linux"))]
        PluginFormat::Lv2 => bail!("LV2 plug-ins are supported only on Linux"),
        #[cfg(target_os = "macos")]
        PluginFormat::Au => Ok(Box::new(crate::au::AuProcessor::load(&request)?)),
        #[cfg(not(target_os = "macos"))]
        PluginFormat::Au => bail!("Audio Units are supported only on macOS"),
    }
}

impl NativeProcessor for Plugin {
    fn loaded_info(&self) -> LoadedInfo {
        let info = Plugin::info(self);
        LoadedInfo {
            name: info.name.clone(),
            vendor: info.vendor.clone(),
            version: info.version.clone(),
            category: info.category.clone(),
            uid: info.uid.clone(),
            has_gui: false,
            has_midi_input: info.has_midi_input,
            has_midi_output: info.has_midi_output,
        }
    }
    fn start_processing(&mut self) -> Result<()> {
        Ok(Plugin::start_processing(self)?)
    }
    fn stop_processing(&mut self) -> Result<()> {
        Ok(Plugin::stop_processing(self)?)
    }
    fn is_processing(&self) -> bool {
        Plugin::is_processing(self)
    }
    fn reconfigure(&mut self, sample_rate: f64, block_size: usize) -> Result<()> {
        Ok(Plugin::reconfigure(self, sample_rate, block_size)?)
    }
    fn set_process_mode(&mut self, mode: ProcessMode) -> Result<()> {
        Ok(Plugin::set_process_mode(self, mode)?)
    }
    fn service_host_requests(&mut self) -> Result<i32> {
        Ok(Plugin::service_host_requests(self)?.bits())
    }
    fn audio_bus_layout(&self) -> Result<AudioBusLayout> {
        Ok(Plugin::audio_bus_layout(self)?)
    }
    fn audio_bus_details(&self) -> Result<AudioBusDetailsLayout> {
        Ok(Plugin::audio_bus_details(self)?)
    }
    fn set_bus_active(
        &mut self,
        media: MediaType,
        direction: BusDirection,
        index: i32,
        active: bool,
    ) -> Result<()> {
        Ok(Plugin::set_bus_active(
            self, media, direction, index, active,
        )?)
    }
    fn set_transport_position(&mut self, position: TransportPosition) -> Result<()> {
        Ok(Plugin::set_transport_position(self, position)?)
    }
    fn transport_position(&self) -> Result<TransportPosition> {
        Ok(Plugin::transport_position(self)?)
    }
    fn set_tempo(&mut self, bpm: f64) -> Result<()> {
        Ok(Plugin::set_tempo(self, bpm)?)
    }
    fn set_playing(&mut self, playing: bool) -> Result<()> {
        Ok(Plugin::set_playing(self, playing)?)
    }
    fn set_time_signature(&mut self, numerator: i32, denominator: i32) -> Result<()> {
        Ok(Plugin::set_time_signature(self, numerator, denominator)?)
    }
    fn get_parameters(&mut self) -> Result<Vec<Parameter>> {
        Ok(Plugin::get_parameters(self)?)
    }
    fn get_parameter(&mut self, id: u32) -> Result<f64> {
        Ok(Plugin::get_parameter(self, id)?)
    }
    fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        Ok(Plugin::set_parameter(self, id, value)?)
    }
    fn set_parameter_at(&mut self, id: u32, value: f64, offset: i32) -> Result<()> {
        Ok(Plugin::set_parameter_at(self, id, value, offset)?)
    }
    fn format_parameter(&mut self, id: u32, value: f64) -> Result<String> {
        Ok(Plugin::format_parameter(self, id, value)?)
    }
    fn send_midi_event_at(&mut self, event: MidiEvent, offset: i32) -> Result<()> {
        Ok(Plugin::send_midi_event_at(self, event, offset)?)
    }
    fn send_plugin_event(&mut self, event: PluginEvent) -> Result<()> {
        Ok(Plugin::send_plugin_event(self, event)?)
    }
    fn send_midi_controller_at(
        &mut self,
        bus: i32,
        channel: MidiChannel,
        controller: MidiController,
        value: f64,
        offset: i32,
    ) -> Result<()> {
        Ok(Plugin::send_midi_controller_at(
            self, bus, channel, controller, value, offset,
        )?)
    }
    fn note_expressions_on(&self, bus: i32, channel: u8) -> Result<Vec<NoteExpressionInfo>> {
        Ok(Plugin::note_expressions_on(self, bus, channel)?)
    }
    fn midi_controller_assignments(
        &self,
        direction: BusDirection,
    ) -> Result<Vec<MidiControllerAssignment>> {
        Ok(Plugin::midi_controller_assignments(self, direction)?)
    }
    fn midi_panic(&mut self) -> Result<()> {
        Ok(Plugin::midi_panic(self)?)
    }
    fn process_bus_audio(&mut self, buffers: &mut BusAudioBuffers) -> Result<()> {
        Ok(Plugin::process_bus_audio(self, buffers)?)
    }
    fn take_output_events(&mut self) -> Vec<PluginEvent> {
        Plugin::take_output_events(self)
    }
    fn latency_samples(&self) -> u32 {
        Plugin::latency_samples(self)
    }
    fn tail_samples(&self) -> u32 {
        Plugin::tail_samples(self)
    }
    fn save_state(&mut self) -> Result<Vec<u8>> {
        Ok(Plugin::save_state(self)?)
    }
    fn load_state(&mut self, data: &[u8]) -> Result<()> {
        Ok(Plugin::load_state_with_context(
            self,
            data,
            &StateContext::Project,
        )?)
    }
    fn as_vst3(&mut self) -> Option<&mut Plugin> {
        Some(self)
    }
}

/// Map a normalized wire value onto a plain range, rounding stepped values.
pub(crate) fn plain_from_normalized(min: f64, max: f64, stepped: bool, value: f64) -> f64 {
    let plain = min + value.clamp(0.0, 1.0) * (max - min);
    if stepped {
        plain.round().clamp(min, max)
    } else {
        plain
    }
}

pub(crate) fn normalized_from_plain(min: f64, max: f64, plain: f64) -> f64 {
    if max > min && plain.is_finite() {
        ((plain - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Parameter metadata in wire form for a plain-valued (CLAP/LV2/AU) parameter.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plain_parameter(
    id: u32,
    name: String,
    unit: String,
    min: f64,
    max: f64,
    default: f64,
    value: f64,
    stepped: bool,
    automatable: bool,
    read_only: bool,
    bypass: bool,
) -> Parameter {
    Parameter {
        id,
        name,
        value: normalized_from_plain(min, max, value),
        min: 0.0,
        max: 1.0,
        default: normalized_from_plain(min, max, default),
        unit,
        step_count: if stepped && max > min {
            (max - min).round().min(i32::MAX as f64) as i32
        } else {
            0
        },
        can_automate: automatable && !read_only,
        is_read_only: read_only,
        is_bypass: bypass,
        flags: 0,
    }
}

/// CLAP, LV2 and AU transport bookkeeping, advanced after each successful playing block.
#[derive(Clone, Copy)]
pub(crate) struct Transport {
    pub position: TransportPosition,
    pub tempo: f64,
    pub playing: bool,
    pub numerator: i32,
    pub denominator: i32,
}

impl Transport {
    pub fn new(tempo: f64, (numerator, denominator): (i32, i32)) -> Self {
        Self {
            position: TransportPosition {
                samples: 0,
                quarter_notes: 0.0,
            },
            tempo,
            playing: false,
            numerator,
            denominator,
        }
    }

    pub fn advance(&mut self, frames: usize, sample_rate: f64) {
        if self.playing {
            self.position.samples = self.position.samples.saturating_add(frames as i64);
            self.position.quarter_notes += frames as f64 / sample_rate * (self.tempo / 60.0);
        }
    }

    pub fn validate_tempo(bpm: f64) -> Result<()> {
        anyhow::ensure!(bpm.is_finite() && bpm > 0.0, "Invalid tempo");
        Ok(())
    }
}
