use crate::automation::CompiledAutomation;
use crate::meter::{ChannelMeter, ChannelMeters};
use crate::performance::{RenderPerformanceSnapshot, TrackRenderPerformance, nanoseconds};
use crate::quality::{self, RenderOptions};
use anyhow::{Context, Result, bail, ensure};
#[cfg(test)]
use daw_core::AudioData;
use daw_core::{
    AutomationLane, AutomationParameter, ClipContent, Effect, FadeCurve, Instrument, Project,
    TrackKind,
};
use daw_plugins::{MidiMessage, ProcessingMode, StereoBusInput, VstEffect, VstInstrument};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};

#[path = "schedule.rs"]
mod schedule;

#[path = "worker_pool.rs"]
mod worker_pool;
use worker_pool::WorkerPool;

#[cfg(test)]
#[path = "signal_monitor_render_tests.rs"]
mod signal_monitor_render_tests;

#[path = "audio_schedule.rs"]
mod audio_schedule;

#[path = "live_edit.rs"]
pub(crate) mod live_edit;
use live_edit::LiveEditContext;

#[path = "freeze.rs"]
mod freeze;
pub use freeze::{FreezeRenderOutput, FreezeRenderReport, FreezeRenderRequest, freeze_track_wav};

#[path = "encoded.rs"]
mod encoded;
pub use encoded::ExportFormat;
use encoded::{Delivery, Encoder};

#[path = "stems.rs"]
mod stems;
pub use stems::{StemFile, StemPlan, StemRequest, export_stems, plan_stems};

#[cfg(test)]
#[path = "live_edit_tests.rs"]
mod live_edit_tests;

#[cfg(test)]
#[path = "automation_render_tests.rs"]
mod automation_render_tests;

#[cfg(test)]
#[path = "stretch_render_tests.rs"]
mod stretch_render_tests;
#[cfg(test)]
#[path = "warp_render_tests.rs"]
mod warp_render_tests;

#[cfg(test)]
#[path = "effect_state_tests.rs"]
mod effect_state_tests;

#[cfg(test)]
#[path = "audio_stream_tests.rs"]
mod audio_stream_tests;
#[cfg(test)]
#[path = "quality_tests.rs"]
mod quality_tests;

#[cfg(test)]
#[path = "audio_overlap_tests.rs"]
mod audio_overlap_tests;

#[cfg(test)]
#[path = "builtin_render_tests.rs"]
mod builtin_render_tests;

#[cfg(test)]
#[path = "midi_loop_tests.rs"]
mod midi_loop_tests;

#[cfg(test)]
#[path = "midi2_render_tests.rs"]
mod midi2_render_tests;
#[cfg(test)]
#[path = "multi_io_master_tests.rs"]
mod multi_io_master_tests;

#[path = "midi_expression.rs"]
pub(crate) mod midi_expression;
use midi_expression::{PreciseNote, Runtime as ExpressionRuntime};

pub(crate) const BLOCK: usize = 256;

#[derive(Clone, Copy)]
struct ScheduledNote {
    clip: u64,
    id: u64,
    start: u64,
    end: u64,
    pitch: u8,
    velocity: u8,
    precise: Option<PreciseNote>,
}

#[derive(Clone, Copy)]
struct Event {
    sample: u64,
    note: ScheduledNote,
    on: bool,
}

impl Event {
    fn message(self, offset: u32) -> MidiMessage {
        MidiMessage {
            offset,
            pitch: self.note.pitch,
            velocity: self.note.velocity,
            on: self.on,
        }
    }
}

type NoteKey = (u64, u64, u8);

impl ScheduledNote {
    fn key(self) -> NoteKey {
        (self.clip, self.id, self.pitch)
    }
}

fn record_active_note(
    active: &mut Vec<ScheduledNote>,
    indices: &mut HashMap<NoteKey, usize>,
    event: Event,
) {
    let key = event.note.key();
    if event.on {
        if let Some(&index) = indices.get(&key) {
            active[index] = event.note;
        } else {
            indices.insert(key, active.len());
            active.push(event.note);
        }
    } else if let Some(index) = indices.remove(&key) {
        active.swap_remove(index);
        if let Some(moved) = active.get(index) {
            *indices
                .get_mut(&moved.key())
                .expect("active voice has an index") = index;
        }
    }
}

struct AudioClip {
    source_path: std::path::PathBuf,
    source_alias: std::path::PathBuf,
    source_rate: f64,
    processing: daw_core::AudioProcessing,
    is_preview: bool,
    warp: u64,
    original: Arc<crate::audio_asset::AudioAsset>,
    start: u64,
    source_offset: f64,
    gain: f32,
    fade_in: f64,
    fade_out: f64,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
    envelope_offset: f64,
    envelope_length: f64,
    boundary_fades: Option<daw_core::AudioBoundaryFades>,
    source_length: f64,
    reverse: bool,
    data: Arc<crate::audio_asset::AudioAsset>,
    id: u64,
}

impl AudioClip {
    fn source_position(&self, position: u64, sample_rate: u32) -> f64 {
        let elapsed = position - self.start;
        let delta = elapsed as f64 * self.data.sample_rate() as f64 / sample_rate as f64;
        self.source_offset
            + if self.reverse {
                self.source_length - 1.0 - delta
            } else {
                delta
            }
    }
    fn read_block(
        &self,
        position: u64,
        sample_rate: u32,
        mut window: Option<&mut crate::audio_stream::StreamWindow>,
        output: &mut [[f32; 2]],
    ) -> Result<()> {
        output.fill([0.0; 2]);
        let frames = self.data.frames();
        if output.is_empty() || frames == 0 {
            return Ok(());
        }
        if let Some(file) = self.data.file() {
            let first = self.source_position(position, sample_rate);
            let last = self.source_position(position + output.len() as u64 - 1, sample_rate);
            let lower = first.min(last);
            let upper = first.max(last);
            if upper < -1e-8 || lower >= frames as f64 {
                return Ok(());
            }
            let first = lower.max(0.0).floor() as usize;
            let last = (upper.max(0.0).floor() as usize)
                .min(frames - 1)
                .saturating_add(1)
                .min(frames - 1);
            window
                .as_mut()
                .context("Streamed audio window was not prepared")?
                .prepare(file, first.min(frames - 1), last)?;
        }
        for (index, output) in output.iter_mut().enumerate() {
            let elapsed = position + index as u64 - self.start;
            let source = self.source_position(position + index as u64, sample_rate);
            if source < -1e-8 {
                continue;
            }
            let source = source.max(0.0);
            let frame = source.floor() as usize;
            if frame >= frames {
                continue;
            }
            let next = (frame + 1).min(frames - 1);
            let fraction = (source - frame as f64) as f32;
            let time = elapsed as f64 + self.envelope_offset;
            let fade_in = if self.fade_in > 0.0 {
                self.fade_in_curve.value(time / self.fade_in)
            } else {
                1.0
            };
            let fade_out = if self.fade_out > 0.0 {
                self.fade_out_curve
                    .value((self.envelope_length - time) / self.fade_out)
            } else {
                1.0
            };
            let gain = self.gain
                * fade_in
                * fade_out
                * self
                    .boundary_fades
                    .map_or(1.0, |boundary| boundary.gain(elapsed as f64));
            for (channel, destination) in output.iter_mut().enumerate() {
                let channel = channel.min(self.data.channels() - 1);
                let (a, b) = if let Some(data) = self.data.resident() {
                    (
                        data.samples[frame * data.channels + channel],
                        data.samples[next * data.channels + channel],
                    )
                } else {
                    let window = window.as_ref().expect("prepared streamed source");
                    (window.sample(frame, channel), window.sample(next, channel))
                };
                *destination = (a + (b - a) * fraction) * gain;
            }
        }
        Ok(())
    }
}

/// A preallocated stereo delay used to align paths with different processing latency.
pub struct CompensationDelay {
    frames: Vec<[f32; 2]>,
    cursor: usize,
}

impl CompensationDelay {
    pub fn new(samples: usize) -> Self {
        Self {
            frames: vec![[0.0; 2]; samples],
            cursor: 0,
        }
    }
    pub fn latency_samples(&self) -> usize {
        self.frames.len()
    }
    pub fn reset(&mut self) {
        self.frames.fill([0.0; 2]);
        self.cursor = 0;
    }
    #[inline]
    pub fn process(&mut self, frame: [f32; 2]) -> [f32; 2] {
        if self.frames.is_empty() {
            return frame;
        }
        let output = self.frames[self.cursor];
        self.frames[self.cursor] = frame;
        self.cursor = (self.cursor + 1) % self.frames.len();
        output
    }
}

enum Processor {
    None,
    Vst(Box<VstInstrument>),
    Builtin(Box<crate::builtin::BuiltinDevice>),
}

/// Compiled lane ownership keeps graph lookup and allocation out of rendering.
struct AutomatedValue {
    manual: f32,
    smooth_value: f32,
    smooth_remaining: u32,
    lane: Option<CompiledAutomation>,
}

fn lane_enabled(project: &Project, track: &daw_core::Track) -> bool {
    if track.mute {
        return false;
    }
    let mut parent = track.parent_folder;
    while let Some(folder) = parent.and_then(|id| project.track(id)) {
        if folder.mute {
            return false;
        }
        parent = folder.parent_folder;
    }
    true
}

impl AutomatedValue {
    fn new(project: &Project, track: u64, parameter: AutomationParameter, manual: f32) -> Self {
        let lane = active_automation_lane(project, track, parameter).map(CompiledAutomation::new);
        Self {
            manual,
            lane,
            smooth_value: manual,
            smooth_remaining: 0,
        }
    }

    fn manual(manual: f32) -> Self {
        Self {
            manual,
            lane: None,
            smooth_value: manual,
            smooth_remaining: 0,
        }
    }

    fn set_manual(&mut self, value: f32, sample_rate: u32) {
        if self.manual != value {
            self.manual = value;
            self.smooth_remaining = (sample_rate / 200).max(1);
        }
    }

    /// Evaluate settled manual controls once per block. A ramp still advances
    /// for every sample, even while an automation lane takes precedence.
    fn constant(&self) -> Option<f32> {
        (self.lane.is_none() && self.smooth_remaining == 0).then_some(self.smooth_value)
    }

    #[inline]
    fn next_value(&mut self, beat: f64) -> f32 {
        if self.smooth_remaining > 0 {
            self.smooth_value += (self.manual - self.smooth_value) / self.smooth_remaining as f32;
            self.smooth_remaining -= 1;
        }
        self.lane
            .as_ref()
            .and_then(|lane| {
                lane.effective_value_at(beat)
                    .map(|value| lane.parameter().denormalize(value))
            })
            .unwrap_or(self.smooth_value)
    }

    #[inline]
    fn value(&self, beat: f64) -> f32 {
        self.lane
            .as_ref()
            .and_then(|lane| {
                lane.effective_value_at(beat)
                    .map(|value| lane.parameter().denormalize(value))
            })
            .unwrap_or(self.manual)
    }
}

fn active_automation_lane(
    project: &Project,
    track: u64,
    parameter: AutomationParameter,
) -> Option<&AutomationLane> {
    project.tracks.iter().find_map(|candidate| {
        let lane = candidate.automation.as_ref()?;
        (lane.target.track == track
            && lane.target.parameter == parameter
            && (!lane.points.is_empty()
                || lane.clips.iter().any(|clip| !clip.points.is_empty())
                || lane.override_value.is_some())
            && lane_enabled(project, candidate))
        .then_some(lane)
    })
}

#[inline]
/// Mid/side width: 0 folds to mono, 1 is unchanged, 2 doubles the side.
pub(crate) fn stereo_width([left, right]: [f32; 2], width: f32) -> [f32; 2] {
    let mid = (left + right) * 0.5;
    let side = (left - right) * 0.5 * width;
    [mid + side, mid - side]
}

pub(crate) fn polarity_sign(track: &daw_core::Track) -> f32 {
    if track.invert_polarity { -1.0 } else { 1.0 }
}

fn fader_factors(gain: f32, pan: f32) -> [f32; 2] {
    let pan = pan.clamp(-1.0, 1.0);
    [gain * (1.0 - pan.max(0.0)), gain * (1.0 + pan.min(0.0))]
}

fn send_active(project: &Project, track: u64, send: &daw_core::Send) -> bool {
    send.gain > 0.0
        || active_automation_lane(
            project,
            track,
            AutomationParameter::SendGain {
                bus: send.target_bus,
            },
        )
        .is_some()
}

struct PluginAutomation {
    id: u32,
    lane: CompiledAutomation,
    previous: Option<f32>,
    action_sequence: u64,
    manual: Option<f32>,
}

fn plugin_automation(project: &Project, track: u64, effect: Option<u64>) -> Vec<PluginAutomation> {
    let mut parameters: Vec<_> = project
        .tracks
        .iter()
        .filter(|candidate| lane_enabled(project, candidate))
        .filter_map(|candidate| {
            let lane = candidate.automation.as_ref()?;
            let AutomationParameter::VstParameter { effect: target, id } = lane.target.parameter
            else {
                return None;
            };
            (lane.target.track == track
                && target == effect
                && (!lane.points.is_empty()
                    || lane.override_value.is_some()
                    || lane.clips.iter().any(|clip| !clip.points.is_empty())))
            .then(|| PluginAutomation {
                id,
                lane: CompiledAutomation::new(lane),
                previous: None,
                action_sequence: 0,
                manual: project.track(track).and_then(|track| {
                    let values = match effect {
                        Some(effect) => {
                            &track
                                .inserts
                                .iter()
                                .find(|slot| slot.id == effect)?
                                .parameters
                        }
                        None => &track.instrument_parameters,
                    };
                    values
                        .iter()
                        .find(|parameter| parameter.id == id)
                        .map(|parameter| parameter.value as f32)
                }),
            })
        })
        .collect();
    // Automation rows are workspace order, not parameter processing order.
    // Canonical identities keep live swaps linear and reload/export behavior
    // independent of dragging those rows. Sorting stays off the render thread.
    parameters.sort_unstable_by_key(|parameter| parameter.id);
    parameters
}

fn initialize_automation_defaults(
    parameters: &mut [PluginAutomation],
    read: impl FnOnce() -> Result<Vec<daw_plugins::ParameterInfo>>,
) -> Result<()> {
    if !parameters
        .iter()
        .any(|parameter| parameter.manual.is_none() && parameter.lane.pattern_only())
    {
        return Ok(());
    }
    let defaults = read()?;
    for parameter in parameters
        .iter_mut()
        .filter(|parameter| parameter.manual.is_none() && parameter.lane.pattern_only())
    {
        parameter.manual = Some(
            defaults
                .iter()
                .find(|info| info.id == parameter.id)
                .with_context(|| {
                    format!(
                        "Plugin does not expose the default value for automated parameter {}",
                        parameter.id
                    )
                })?
                .value as f32,
        );
    }
    Ok(())
}

/// Built-in devices know their defaults without asking a helper process.
fn initialize_builtin_defaults(parameters: &mut [PluginAutomation], device: Option<&str>) {
    let Some(descriptor) = device.and_then(daw_devices::descriptor) else {
        return;
    };
    for parameter in parameters
        .iter_mut()
        .filter(|parameter| parameter.manual.is_none())
    {
        parameter.manual = descriptor
            .parameter(parameter.id)
            .map(|spec| spec.default_normalized());
    }
}

fn apply_plugin_automation(
    parameters: &mut [PluginAutomation],
    beat: f64,
    mut set: impl FnMut(u32, f64) -> Result<()>,
) -> Result<()> {
    for parameter in parameters {
        if let Some(value) = parameter.lane.effective_value_at(beat).or(parameter.manual)
            && parameter.previous != Some(value)
        {
            set(parameter.id, f64::from(value))?;
            parameter.previous = Some(value);
        }
    }
    Ok(())
}

fn apply_plugin_parameter_change(
    parameters: &mut [PluginAutomation],
    id: u32,
    value: f64,
    override_sequence: Option<u64>,
    mut set: impl FnMut(u32, f64) -> Result<()>,
) -> Result<()> {
    if let Some(sequence) = override_sequence
        && parameters
            .iter()
            .any(|parameter| parameter.id == id && parameter.action_sequence > sequence)
    {
        // A newer project already resumed or changed this lane while this
        // immediate gesture waited for the render worker.
        return Ok(());
    }
    set(id, value)?;
    for parameter in parameters.iter_mut().filter(|parameter| parameter.id == id) {
        parameter.manual = Some(value as f32);
        if let Some(sequence) = override_sequence {
            parameter.lane.set_override_value(Some(value as f32));
            parameter.previous = Some(value as f32);
            parameter.action_sequence = sequence;
        } else {
            parameter.previous = None;
        }
    }
    Ok(())
}

struct Insert {
    id: u64,
    effect: InsertProcessor,
    parameters: Vec<PluginAutomation>,
    inputs: Vec<AuxInput>,
    arrival_latency: u32,
    main_delay: CompensationDelay,
}

enum InsertProcessor {
    Vst(Box<VstEffect>),
    Builtin(Box<crate::builtin::BuiltinDevice>),
}

impl InsertProcessor {
    fn latency_samples(&self) -> u32 {
        match self {
            Self::Vst(effect) => effect.latency_samples(),
            Self::Builtin(device) => device.latency_samples(),
        }
    }
}

/// Every selected native input pair has one retained, double-precision sum.
/// Multiple sends to the same pair are combined before its single f32 conversion.
struct AuxInput {
    bus: usize,
    first_channel: usize,
    left: Vec<f32>,
    right: Vec<f32>,
    mix_left: Vec<f64>,
    mix_right: Vec<f64>,
}

impl AuxInput {
    fn new(bus: usize, first_channel: usize) -> Self {
        Self {
            bus,
            first_channel,
            left: vec![0.0; BLOCK],
            right: vec![0.0; BLOCK],
            mix_left: vec![0.0; BLOCK],
            mix_right: vec![0.0; BLOCK],
        }
    }

    fn clear(&mut self, count: usize) {
        self.mix_left[..count].fill(0.0);
        self.mix_right[..count].fill(0.0);
    }

    fn prepare(&mut self, count: usize) {
        for index in 0..count {
            self.left[index] = self.mix_left[index] as f32;
            self.right[index] = self.mix_right[index] as f32;
        }
    }

    fn borrowed(&self, count: usize) -> StereoBusInput<'_> {
        StereoBusInput {
            bus: self.bus,
            first_channel: self.first_channel,
            left: &self.left[..count],
            right: &self.right[..count],
        }
    }
}

fn aux_inputs(
    routes: &[daw_core::PluginInputRoute],
    layout: &daw_plugins::AudioBusLayout,
    main: Option<usize>,
) -> Result<Vec<AuxInput>> {
    let mut inputs = Vec::<AuxInput>::new();
    for route in routes {
        let bus = route.bus as usize;
        let first_channel = route.first_channel as usize;
        let native = layout
            .inputs
            .get(bus)
            .context("Plug-in input bus is unavailable")?;
        ensure!(
            native.active && first_channel < native.channel_count,
            "Plug-in input channel pair is unavailable"
        );
        ensure!(
            main != Some(bus) || first_channel != 0,
            "Additional input routing cannot replace an insert's main input"
        );
        if !inputs
            .iter()
            .any(|input| input.bus == bus && input.first_channel == first_channel)
        {
            inputs.push(AuxInput::new(bus, first_channel));
        }
    }
    ensure!(
        inputs.len() <= 128,
        "Plug-in has too many routed input pairs"
    );
    Ok(inputs)
}

fn unmuted_channel(project: &Project, track: &daw_core::Track) -> bool {
    if track.mute {
        return false;
    }
    let mut parent = track.parent_folder;
    while let Some(folder) = parent.and_then(|id| project.track(id)) {
        if folder.mute {
            return false;
        }
        parent = folder.parent_folder;
    }
    true
}

/// Solo selects audible channels, while sidechain feeds and raw instrument
/// owners may still need DSP without leaking their ordinary output into Master.
struct ProcessingAdmission {
    processors: HashSet<u64>,
    channels: HashSet<u64>,
}

fn processing_tracks(project: &Project, audible: &HashSet<u64>) -> ProcessingAdmission {
    let mut channels = audible.clone();
    if !project.master.mute {
        channels.insert(0);
    }
    let mut processors = channels.clone();
    loop {
        let before = (processors.len(), channels.len());
        for target in project.channels() {
            if !processors.contains(&target.id) {
                continue;
            }
            if let Some(output) = target.instrument_output {
                processors.insert(output.track);
            }
            let process_channel = channels.contains(&target.id);
            for input in target.instrument_inputs.iter().chain(
                target
                    .inserts
                    .iter()
                    .filter(|slot| process_channel && slot.enabled)
                    .flat_map(|slot| &slot.inputs),
            ) {
                if let Some(source) = project.track(input.source_track)
                    && unmuted_channel(project, source)
                {
                    channels.insert(source.id);
                    processors.insert(source.id);
                }
            }
            if process_channel && target.id != 0 && !audible.contains(&target.id) {
                for source in &project.tracks {
                    if unmuted_channel(project, source)
                        && (source.output_bus == Some(target.id)
                            || source.sends.iter().any(|send| {
                                send.target_bus == target.id
                                    && send_active(project, source.id, send)
                            }))
                    {
                        channels.insert(source.id);
                        processors.insert(source.id);
                    }
                }
            }
        }
        if (processors.len(), channels.len()) == before {
            return ProcessingAdmission {
                processors,
                channels,
            };
        }
    }
}

struct RawOutput {
    bus: usize,
    first_channel: usize,
    left: Vec<f32>,
    right: Vec<f32>,
}

#[derive(Clone, Copy)]
enum RouteKind {
    Audio,
    PluginInput { effect: Option<usize>, input: usize },
    InstrumentOutput { output: usize },
}

struct FreezeTap {
    delay: CompensationDelay,
    frames: Vec<[f32; 2]>,
}

struct Route {
    target: Option<usize>,
    gain: AutomatedValue,
    pre_fader: bool,
    delay: CompensationDelay,
    output: Vec<[f32; 2]>,
    kind: RouteKind,
    signal_latency: u32,
    send_bus: Option<u64>,
}

pub(crate) fn audible_tracks(project: &Project, order: &[u64]) -> HashSet<u64> {
    let mut unmuted = HashSet::new();
    let mut selected = HashSet::new();
    let any_solo = project
        .tracks
        .iter()
        .any(|track| track.kind != TrackKind::Automation && track.solo);
    for track in &project.tracks {
        let mut muted = track.mute;
        let mut solo = track.solo;
        let mut parent = track.parent_folder;
        while let Some(folder) = parent.and_then(|id| project.track(id)) {
            muted |= folder.mute;
            solo |= folder.solo;
            parent = folder.parent_folder;
        }
        if !muted {
            unmuted.insert(track.id);
            if !any_solo || solo {
                selected.insert(track.id);
            }
        }
    }
    if any_solo {
        // A solo bus includes its upstream feeds. Do this before propagating solo downstream.
        for id in order.iter().rev() {
            let track = project.track(*id).unwrap();
            if unmuted.contains(id)
                && (track
                    .output_bus
                    .is_some_and(|target| selected.contains(&target))
                    || track
                        .sends
                        .iter()
                        .any(|send| selected.contains(&send.target_bus)))
            {
                selected.insert(*id);
            }
        }
        for id in order {
            if !selected.contains(id) {
                continue;
            }
            let track = project.track(*id).unwrap();
            for target in track
                .output_bus
                .into_iter()
                .chain(track.sends.iter().map(|send| send.target_bus))
            {
                if unmuted.contains(&target) {
                    selected.insert(target);
                }
            }
        }
    }
    selected
}

struct RenderTrack {
    id: u64,
    name: String,
    pending_error: Option<String>,
    audio: Arc<Vec<AudioClip>>,
    audio_segments: Arc<Vec<audio_schedule::Segment>>,
    // A short source-only correction bridges edits/refinement before inserts.
    // Preallocated at graph construction; no retained asset is dropped by DSP.
    audio_transition: Vec<[f32; 2]>,
    audio_transition_cursor: usize,
    stream_windows: Option<Box<crate::audio_stream::WindowPair>>,
    audio_incoming: Vec<[f32; 2]>,
    audio_outgoing: Vec<[f32; 2]>,
    notes: Arc<Vec<ScheduledNote>>,
    events: Arc<Vec<Event>>,
    note_order: Arc<Vec<usize>>,
    event_cursor: usize,
    processor: Processor,
    expression: ExpressionRuntime,
    midi: Vec<MidiMessage>,
    chase: Vec<Event>,
    chase_delay: u32,
    active_notes: Vec<ScheduledNote>,
    active_indices: HashMap<NoteKey, usize>,
    live_midi: Vec<MidiMessage>,
    left: Vec<f32>,
    right: Vec<f32>,
    mix_left: Vec<f64>,
    mix_right: Vec<f64>,
    input_gain: AutomatedValue,
    // Manual-only values, smoothed so switching polarity or width cannot click.
    polarity: AutomatedValue,
    width: AutomatedValue,
    gain: AutomatedValue,
    pan: AutomatedValue,
    instrument_automation: Vec<PluginAutomation>,
    delay: CompensationDelay,
    latency: u32,
    input_latency: u32,
    source_latency: u32,
    source_arrival: u32,
    instrument_input_latency: u32,
    instrument_inputs: Vec<AuxInput>,
    raw_outputs: Vec<RawOutput>,
    post_fader: Vec<[f32; 2]>,
    freeze_tap: Option<FreezeTap>,
    tail_samples: u64,
    plugin_failed: bool,
    channel_failed: bool,
    processing_channel: bool,
    processing_frames: usize,
    enabled: bool,
    channel_active: bool,
    inserts: Vec<Insert>,
    insert_left: Vec<f32>,
    insert_right: Vec<f32>,
    routes: Vec<Route>,
    meter: Option<Arc<ChannelMeter>>,
    #[cfg(test)]
    observer: Option<Arc<dyn Fn(u64) + Send + Sync>>,
}

#[derive(Clone, Copy)]
struct BlockContext {
    start: u64,
    count: usize,
    sample_rate: u32,
    tempo: f64,
    timeline: bool,
    live_input: bool,
}

fn finish_processing(track: &mut RenderTrack, result: Result<()>, tolerate: bool) -> Result<u64> {
    if let Err(error) = result {
        if !tolerate {
            return Err(error);
        }
        if track.processing_channel {
            // The shared instrument already rendered successfully. Only this
            // channel's insert path failed; its independently mixed raw outputs
            // must keep sounding, including the block which exposed the fault.
            track.channel_failed = true;
            track.channel_active = false;
            let count = track.processing_frames;
            track.left[..count].fill(0.0);
            track.right[..count].fill(0.0);
            track.post_fader[..count].fill([0.0; 2]);
            if let Some(tap) = &mut track.freeze_tap {
                tap.frames[..count].fill([0.0; 2]);
            }
            for route in &mut track.routes {
                if let RouteKind::InstrumentOutput { output } = route.kind {
                    for index in 0..count {
                        route.output[index] = route.delay.process([
                            track.raw_outputs[output].left[index],
                            track.raw_outputs[output].right[index],
                        ]);
                    }
                } else {
                    route.delay.reset();
                    route.output[..count].fill([0.0; 2]);
                }
            }
        } else {
            track.plugin_failed = true;
        }
        track.pending_error = Some(format!(
            "Plugin processing failed on \"{}\"\n\nChannel: \"{}\" (ID {})\n{error:#}",
            track.name, track.name, track.id
        ));
        track.delay.reset();
        if let Some(meter) = &track.meter {
            meter.clear();
        }
        return Ok(1);
    }
    Ok(0)
}

/// Stable merge of two timestamp-ordered runs without scratch allocation.
/// Input batches are bounded; the already sorted sequencer run stays untouched
/// in the common case without live input or when all input offsets are zero.
fn merge_midi_events(messages: &mut [MidiMessage], middle: usize) {
    if middle == 0
        || middle == messages.len()
        || messages[middle - 1].offset <= messages[middle].offset
    {
        return;
    }
    if messages.len() == 2 {
        messages.swap(0, 1);
        return;
    }
    let (left_cut, right_cut) = if middle > messages.len() - middle {
        let left = middle / 2;
        let right = middle
            + messages[middle..].partition_point(|event| event.offset < messages[left].offset);
        (left, right)
    } else {
        let right = middle + (messages.len() - middle) / 2;
        let left =
            messages[..middle].partition_point(|event| event.offset <= messages[right].offset);
        (left, right)
    };
    messages[left_cut..right_cut].rotate_left(middle - left_cut);
    let next_middle = left_cut + right_cut - middle;
    let (left, right) = messages.split_at_mut(next_middle);
    merge_midi_events(left, left_cut);
    merge_midi_events(right, right_cut - next_middle);
}

impl RenderTrack {
    fn collect_midi(&mut self, context: BlockContext) {
        let start = context.start;
        let end = start + context.count as u64;
        self.midi.clear();
        self.expression.messages.clear();
        // Driver clock resets can make a bounded input batch locally unordered.
        // Binary insertion retains equal-offset off/on articulation ordering.
        for index in 1..self.live_midi.len() {
            let offset = self.live_midi[index].offset;
            if self.live_midi[index - 1].offset > offset {
                let before =
                    self.live_midi[..index].partition_point(|event| event.offset <= offset);
                self.live_midi[before..=index].rotate_right(1);
            }
        }
        // A loop can split one worker block into shorter processing segments.
        // Keep later input edges for the next segment, in audible sample time.
        // Latency priming discards output, so it must not consume these edges.
        let live_count = if context.live_input {
            self.live_midi
                .partition_point(|event| event.offset < context.count as u32)
        } else {
            0
        };
        self.midi.extend(self.live_midi.drain(..live_count));
        if context.live_input {
            for event in &mut self.live_midi {
                event.offset -= context.count as u32;
            }
        }
        if context.timeline && self.chase_delay < context.count as u32 {
            for event in self.chase.drain(..) {
                if let Some(precise) = event.note.precise {
                    self.expression.push(
                        precise.channel,
                        daw_plugins::ExpressiveMidiMessage {
                            offset: self.chase_delay,
                            data: precise.data(event.note.pitch, event.on),
                        },
                    );
                } else {
                    self.midi.push(event.message(self.chase_delay));
                }
                record_active_note(&mut self.active_notes, &mut self.active_indices, event);
            }
        }
        self.chase_delay = self.chase_delay.saturating_sub(context.count as u32);
        while context.timeline
            && let Some(event) = self.events.get(self.event_cursor)
        {
            let sample = event
                .sample
                .saturating_add(u64::from(self.instrument_input_latency));
            if sample >= end {
                break;
            }
            if let Some(precise) = event.note.precise {
                self.expression.push(
                    precise.channel,
                    daw_plugins::ExpressiveMidiMessage {
                        offset: sample.saturating_sub(start) as u32,
                        data: precise.data(event.note.pitch, event.on),
                    },
                );
            } else {
                self.midi
                    .push(event.message(sample.saturating_sub(start) as u32));
            }
            record_active_note(&mut self.active_notes, &mut self.active_indices, *event);
            self.event_cursor += 1;
        }
        // Input events can land between sequencer events within this block.
        // Stable ordering preserves a retrigger's note-off/note-on edge order.
        merge_midi_events(&mut self.midi, live_count);
        // Start/chase scheduled voices before their same-sample dimensions.
        // Live batches already retain their own causal note/pressure ordering.
        self.expression
            .collect(context, self.instrument_input_latency);
        midi_expression::sort_messages(&mut self.expression.messages);
    }

    fn process(&mut self, context: BlockContext) -> Result<()> {
        if !self.enabled || self.plugin_failed {
            return Ok(());
        }
        // Failure containment must describe this invocation, including a panic
        // at its entry, rather than the last successful block or seek prefill.
        self.processing_channel = false;
        self.processing_frames = context.count;
        #[cfg(test)]
        if let Some(observer) = &self.observer {
            observer(self.id);
        }
        let BlockContext { start, count, .. } = context;
        let end = start + count as u64;
        let beats_per_sample = context.tempo / (60.0 * context.sample_rate as f64);
        if !matches!(self.processor, Processor::None) {
            self.collect_midi(context);
        }
        match &mut self.processor {
            Processor::None => {}
            Processor::Vst(plugin) => {
                let position = start as i64 - i64::from(self.instrument_input_latency);
                for input in &mut self.instrument_inputs {
                    input.prepare(count);
                }
                let mut inputs = [StereoBusInput {
                    bus: 0,
                    first_channel: 0,
                    left: &[],
                    right: &[],
                }; 128];
                for (destination, input) in inputs.iter_mut().zip(&self.instrument_inputs) {
                    *destination = input.borrowed(count);
                }
                let process = apply_plugin_automation(
                    &mut self.instrument_automation,
                    position as f64 * beats_per_sample,
                    |id, value| plugin.queue_parameter_for_block(id, value),
                )
                .and_then(|()| {
                    plugin.queue_transport_for_block(
                        position,
                        position as f64 * beats_per_sample,
                        context.tempo,
                        context.timeline,
                    )
                })
                .and_then(|()| {
                    plugin.process_bus_inputs_expressive(
                        &self.midi,
                        &self.expression.messages,
                        &inputs[..self.instrument_inputs.len()],
                        count,
                    )?;
                    plugin.copy_output_pair(
                        plugin.main_output_bus(),
                        0,
                        &mut self.left[..count],
                        &mut self.right[..count],
                    )?;
                    for output in &mut self.raw_outputs {
                        plugin.copy_output_pair(
                            output.bus,
                            output.first_channel,
                            &mut output.left[..count],
                            &mut output.right[..count],
                        )?;
                    }
                    Ok(())
                });
                process.with_context(|| {
                    format!(
                        "Processing instrument \"{}\" ({})",
                        plugin.info().name,
                        plugin.info().path.display()
                    )
                })?;
                for index in 0..count {
                    self.mix_left[index] += f64::from(self.left[index]);
                    self.mix_right[index] += f64::from(self.right[index]);
                }
            }
            Processor::Builtin(device) => {
                let position = start as i64 - i64::from(self.instrument_input_latency);
                let beat = position as f64 * beats_per_sample;
                apply_plugin_automation(&mut self.instrument_automation, beat, |id, value| {
                    device.set_parameter(id, value);
                    Ok(())
                })?;
                device.process_instrument(
                    &daw_devices::ProcessContext {
                        tempo: context.tempo,
                        beat,
                        playing: context.timeline,
                    },
                    &self.midi,
                    &self.expression.messages,
                    &mut self.left[..count],
                    &mut self.right[..count],
                )?;
                for index in 0..count {
                    self.mix_left[index] += f64::from(self.left[index]);
                    self.mix_right[index] += f64::from(self.right[index]);
                }
            }
        }
        self.processing_channel = true;
        // A muted primary channel must not stop its independently mixed output
        // children. Render the shared instrument once, then bypass only this
        // channel's processing and ordinary routes.
        if !self.channel_active {
            for route in &mut self.routes {
                if let RouteKind::InstrumentOutput { output } = route.kind {
                    for index in 0..count {
                        route.output[index] = route.delay.process([
                            self.raw_outputs[output].left[index],
                            self.raw_outputs[output].right[index],
                        ]);
                    }
                }
            }
            if let Some(meter) = &self.meter {
                meter.publish_audio(&self.post_fader[..count], context.sample_rate);
            }
            return Ok(());
        }
        if context.timeline {
            let first_segment = self
                .audio_segments
                .partition_point(|segment| segment.end <= start);
            for segment in &self.audio_segments[first_segment..] {
                if segment.start >= end {
                    break;
                }
                let first = segment.start.saturating_sub(start) as usize;
                let last = (segment.end - start).min(count as u64) as usize;
                let incoming = &self.audio[segment.incoming];
                incoming
                    .read_block(
                        start + first as u64,
                        context.sample_rate,
                        self.stream_windows.as_mut().map(|windows| &mut windows[0]),
                        &mut self.audio_incoming[first..last],
                    )
                    .with_context(|| {
                        format!("Read audio source {}", incoming.source_path.display())
                    })?;
                if let (Some(outgoing), Some(curve)) = (segment.outgoing, &segment.fade) {
                    let outgoing = &self.audio[outgoing];
                    outgoing
                        .read_block(
                            start + first as u64,
                            context.sample_rate,
                            self.stream_windows.as_mut().map(|windows| &mut windows[1]),
                            &mut self.audio_outgoing[first..last],
                        )
                        .with_context(|| {
                            format!("Read audio source {}", outgoing.source_path.display())
                        })?;
                    let duration = (segment.end - segment.start) as f64;
                    let mut curve_cursor =
                        curve.cursor((start + first as u64 - segment.start) as f64 / duration);
                    for index in first..last {
                        let position = start + index as u64;
                        let gain = curve.value(
                            (position - segment.start) as f64 / duration,
                            &mut curve_cursor,
                        );
                        let a = self.audio_outgoing[index];
                        let b = self.audio_incoming[index];
                        self.mix_left[index] += f64::from(a[0]) * f64::from(1.0 - gain)
                            + f64::from(b[0]) * f64::from(gain);
                        self.mix_right[index] += f64::from(a[1]) * f64::from(1.0 - gain)
                            + f64::from(b[1]) * f64::from(gain);
                    }
                } else {
                    for index in first..last {
                        let sample = self.audio_incoming[index];
                        self.mix_left[index] += f64::from(sample[0]);
                        self.mix_right[index] += f64::from(sample[1]);
                    }
                }
            }
            let transition_frames = count.min(
                self.audio_transition
                    .len()
                    .saturating_sub(self.audio_transition_cursor),
            );
            for index in 0..transition_frames {
                let cursor = self.audio_transition_cursor + index;
                let gain = 1.0
                    - cursor as f64 / self.audio_transition.len().saturating_sub(1).max(1) as f64;
                self.mix_left[index] += f64::from(self.audio_transition[cursor][0]) * gain;
                self.mix_right[index] += f64::from(self.audio_transition[cursor][1]) * gain;
            }
            self.audio_transition_cursor += transition_frames;
        }
        // Convert once after every source/bus contribution is accumulated.
        // Plug-in buffers and graph edges retain their f32 representation.
        let input_gain = self.input_gain.constant();
        let polarity = self.polarity.constant();
        for index in 0..count {
            let gain = input_gain.unwrap_or_else(|| {
                let position = if context.timeline {
                    start as i64 + index as i64 - i64::from(self.source_arrival)
                } else {
                    start as i64
                };
                self.input_gain
                    .next_value(position as f64 * beats_per_sample)
            }) * polarity.unwrap_or_else(|| self.polarity.next_value(0.0));
            self.left[index] = self.mix_left[index] as f32 * gain;
            self.right[index] = self.mix_right[index] as f32 * gain;
        }
        for insert in &mut self.inserts {
            let Insert {
                effect,
                parameters,
                inputs,
                main_delay,
                arrival_latency,
                ..
            } = insert;
            let position = if context.timeline {
                start as i64 - i64::from(*arrival_latency)
            } else {
                start as i64
            };
            for index in 0..count {
                let frame = main_delay.process([self.left[index], self.right[index]]);
                self.left[index] = frame[0];
                self.right[index] = frame[1];
            }
            match effect {
                InsertProcessor::Builtin(device) => {
                    apply_plugin_automation(
                        parameters,
                        position as f64 * beats_per_sample,
                        |id, value| {
                            device.set_parameter(id, value);
                            Ok(())
                        },
                    )?;
                    let transport = daw_devices::ProcessContext {
                        tempo: context.tempo,
                        beat: position as f64 * beats_per_sample,
                        playing: context.timeline,
                    };
                    device.process_effect(
                        &transport,
                        &mut self.left[..count],
                        &mut self.right[..count],
                    )?;
                }
                InsertProcessor::Vst(effect) => {
                    for input in inputs.iter_mut() {
                        input.prepare(count);
                    }
                    let mut borrowed = [StereoBusInput {
                        bus: 0,
                        first_channel: 0,
                        left: &[],
                        right: &[],
                    }; 129];
                    borrowed[0] = StereoBusInput {
                        bus: effect
                            .main_input_bus()
                            .context("Effect lost its primary input bus")?,
                        first_channel: 0,
                        left: &self.left[..count],
                        right: &self.right[..count],
                    };
                    for (destination, input) in borrowed[1..].iter_mut().zip(inputs.iter()) {
                        *destination = input.borrowed(count);
                    }
                    let process = apply_plugin_automation(
                        parameters,
                        position as f64 * beats_per_sample,
                        |id, value| effect.queue_parameter_for_block(id, value),
                    )
                    .and_then(|()| {
                        effect.queue_transport_for_block(
                            position,
                            position as f64 * beats_per_sample,
                            context.tempo,
                            context.timeline,
                        )
                    })
                    .and_then(|()| {
                        effect.process_bus_inputs(&borrowed[..inputs.len() + 1], count)?;
                        effect.copy_output_pair(
                            effect.main_output_bus(),
                            0,
                            &mut self.insert_left[..count],
                            &mut self.insert_right[..count],
                        )
                    });
                    process.with_context(|| {
                        format!(
                            "Processing effect \"{}\" ({})",
                            effect.info().name,
                            effect.info().path.display()
                        )
                    })?;
                    std::mem::swap(&mut self.left, &mut self.insert_left);
                    std::mem::swap(&mut self.right, &mut self.insert_right);
                }
            }
        }
        let fader = self
            .gain
            .constant()
            .zip(self.pan.constant())
            .map(|(gain, pan)| fader_factors(gain, pan));
        let width = self.width.constant();
        for index in 0..count {
            let delayed = self.delay.process([self.left[index], self.right[index]]);
            if let Some(tap) = &mut self.freeze_tap {
                tap.frames[index] = tap.delay.process(delayed);
            }
            let width = width.unwrap_or_else(|| self.width.next_value(0.0));
            [self.left[index], self.right[index]] = if width == 1.0 {
                delayed
            } else {
                stereo_width(delayed, width)
            };
            let factors = fader.unwrap_or_else(|| {
                let position = if context.timeline {
                    start as i64 + index as i64 - self.latency as i64
                } else {
                    start as i64
                };
                let beat = position as f64 * beats_per_sample;
                let gain = self.gain.next_value(beat);
                fader_factors(gain, self.pan.next_value(beat))
            });
            // Insert scratch buffers are free after the chain. Reuse them for
            // fader factors so multiple sends evaluate the lane only once.
            self.insert_left[index] = factors[0];
            self.insert_right[index] = factors[1];
            self.post_fader[index] = [
                self.left[index] * self.insert_left[index],
                self.right[index] * self.insert_right[index],
            ];
        }
        for route in &mut self.routes {
            for index in 0..count {
                let position = if context.timeline {
                    start as i64 + index as i64 - self.latency as i64
                } else {
                    start as i64
                };
                let beat = position as f64 * beats_per_sample;
                let gain = route.gain.value(beat);
                let factors = if route.pre_fader {
                    [gain; 2]
                } else {
                    [
                        gain * self.insert_left[index],
                        gain * self.insert_right[index],
                    ]
                };
                let frame = match route.kind {
                    RouteKind::InstrumentOutput { output } => [
                        self.raw_outputs[output].left[index],
                        self.raw_outputs[output].right[index],
                    ],
                    _ => [
                        self.left[index] * factors[0],
                        self.right[index] * factors[1],
                    ],
                };
                route.output[index] = route.delay.process(frame);
            }
        }
        if let Some(meter) = &self.meter {
            meter.publish_audio(&self.post_fader[..count], context.sample_rate);
        }
        Ok(())
    }
}

pub(crate) struct Renderer {
    signal_monitor: Option<Arc<crate::signal_monitor::SignalMonitor>>,
    signal_monitor_transport: u64,
    signal_monitor_active: bool,
    tracks: Vec<Option<Box<RenderTrack>>>,
    levels: Vec<Vec<usize>>,
    workers: Option<WorkerPool>,
    performance: RenderPerformanceSnapshot,
    track_performance: Vec<TrackRenderPerformance>,
    profile_serial: bool,
    pub(crate) sample_rate: u32,
    tempo: f64,
    logical_sample: u64,
    source_sample: u64,
    pub(crate) end_sample: u64,
    timeline_end_sample: u64,
    release_end_sample: u64,
    latency: u32,
    loop_range: Option<(u64, u64)>,
    tolerate_plugin_failure: bool,
    pub(crate) plugin_errors: u64,
    preview: bool,
    audition: Option<(u64, u8, u8)>,
    audition_wheels: Option<(u64, f32, f32)>,
    midi_input_notes: Vec<(u64, u8, u8)>,
    expressive_input: Vec<(u64, u8, u8, daw_plugins::ExpressiveMidiData)>,
    live_edit_context: Arc<LiveEditContext>,
    streamed_clips: Arc<Vec<(u64, u64)>>,
    resident_audio_bytes: u64,
    retired_audio_reads: u64,
    retired_audio_frames: u64,
}

impl Renderer {
    pub(crate) fn new_profiled_offline(
        project: &Project,
        sample_rate: u32,
        workers: Option<usize>,
    ) -> Result<Self> {
        ensure!(workers != Some(0), "Worker count must be positive");
        let mut renderer = Self::new_inner(
            project,
            sample_rate,
            false,
            workers,
            RenderOptions::default(),
            None,
            ProcessingMode::Offline,
            None,
        )?;
        renderer.profile_serial = true;
        renderer.performance = RenderPerformanceSnapshot::default();
        for track in &mut renderer.track_performance {
            track.last_process_ns = 0;
            track.last_ready_wait_ns = 0;
        }
        Ok(renderer)
    }

    pub(crate) fn performance_snapshot(&self) -> RenderPerformanceSnapshot {
        self.performance
    }

    pub(crate) fn track_performance(&self) -> &[TrackRenderPerformance] {
        &self.track_performance
    }

    pub(crate) fn set_block_transport(
        &mut self,
        policy: daw_plugins::BlockTransport,
    ) -> Result<()> {
        for track in self.tracks.iter_mut().flatten() {
            if let Processor::Vst(plugin) = &mut track.processor {
                plugin.set_block_transport(policy)?;
            }
            for insert in &mut track.inserts {
                if let InsertProcessor::Vst(effect) = &mut insert.effect {
                    effect.set_block_transport(policy)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn ipc_stats(&self) -> daw_plugins::IpcStats {
        let mut total = daw_plugins::IpcStats::default();
        let mut add = |stats: Option<daw_plugins::IpcStats>| {
            if let Some(stats) = stats {
                total.requests = total.requests.saturating_add(stats.requests);
                total.request_bytes = total.request_bytes.saturating_add(stats.request_bytes);
                total.response_bytes = total.response_bytes.saturating_add(stats.response_bytes);
            }
        };
        for track in self.tracks.iter().flatten() {
            if let Processor::Vst(plugin) = &track.processor {
                add(plugin.ipc_stats());
            }
            for insert in &track.inserts {
                if let InsertProcessor::Vst(effect) = &insert.effect {
                    add(effect.ipc_stats());
                }
            }
        }
        total
    }
    #[cfg(test)]
    pub(crate) fn new(project: &Project, sample_rate: u32, looping: bool) -> Result<Self> {
        Self::new_inner(
            project,
            sample_rate,
            looping,
            None,
            RenderOptions::default(),
            None,
            ProcessingMode::Realtime,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_options(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        options: RenderOptions,
    ) -> Result<Self> {
        Self::new_inner(
            project,
            sample_rate,
            looping,
            None,
            options,
            None,
            ProcessingMode::Realtime,
            None,
        )
    }

    pub(crate) fn new_with_options_cancelled(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        options: RenderOptions,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Self::new_inner(
            project,
            sample_rate,
            looping,
            None,
            options,
            Some(cancel),
            ProcessingMode::Realtime,
            None,
        )
    }

    /// An offline bounce has its own plugin policy, independent of transport looping.
    pub(crate) fn new_offline_with_options(
        project: &Project,
        sample_rate: u32,
        options: RenderOptions,
    ) -> Result<Self> {
        Self::new_inner(
            project,
            sample_rate,
            false,
            None,
            options,
            None,
            ProcessingMode::Offline,
            None,
        )
    }

    pub(crate) fn new_offline_with_options_cancelled(
        project: &Project,
        sample_rate: u32,
        options: RenderOptions,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Self::new_inner(
            project,
            sample_rate,
            false,
            None,
            options,
            Some(cancel),
            ProcessingMode::Offline,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_workers(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        workers: usize,
    ) -> Result<Self> {
        ensure!(workers > 0, "Worker count must be positive");
        Self::new_inner(
            project,
            sample_rate,
            looping,
            Some(workers),
            RenderOptions::default(),
            None,
            ProcessingMode::Realtime,
            None,
        )
    }

    fn new_offline_with_workers(
        project: &Project,
        sample_rate: u32,
        workers: usize,
    ) -> Result<Self> {
        ensure!(workers > 0, "Worker count must be positive");
        Self::new_inner(
            project,
            sample_rate,
            false,
            Some(workers),
            RenderOptions::default(),
            None,
            ProcessingMode::Offline,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)] // Optional freeze taps share the same graph preparation and PDC path.
    fn new_inner(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        requested_workers: Option<usize>,
        options: RenderOptions,
        cancel: Option<&AtomicBool>,
        processing_mode: ProcessingMode,
        freeze_capture: Option<&[u64]>,
    ) -> Result<Self> {
        let check_cancelled = || {
            ensure!(
                !cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)),
                "Audio preparation cancelled"
            );
            Ok::<(), anyhow::Error>(())
        };
        check_cancelled()?;
        project.validate()?;
        ensure!(
            (8_000..=384_000).contains(&sample_rate),
            "Sample rate must be between 8000 and 384000 Hz"
        );
        ensure!(
            project.tempo.is_finite() && project.tempo > 0.0,
            "Tempo must be positive and finite"
        );
        let samples_per_beat = sample_rate as f64 * 60.0 / project.tempo;
        let to_sample = |beat: f64| (beat.max(0.0) * samples_per_beat).round() as u64;
        let mut order = project.routing_order()?;
        let audible = audible_tracks(project, &order);
        order.push(0);
        let admission = processing_tracks(project, &audible);
        let needed = &admission.processors;
        let indices: HashMap<u64, usize> = order
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect();
        let mut tracks = Vec::new();
        let mut end_sample = to_sample(project.end_beat());
        let mut audio_cache =
            HashMap::<live_edit::AudioAssetKey, Arc<crate::audio_asset::AudioAsset>>::new();
        let mut cache_bytes = 0usize;
        let mut tail_samples = sample_rate as u64 / 5;
        for id in &order {
            check_cancelled()?;
            let track = project
                .track(*id)
                .context("Routing refers to a missing track")?;
            let enabled = needed.contains(id);
            let channel_active = admission.channels.contains(id);
            let mut local_tail = 0_u64;
            ensure!(
                track.latency_samples <= sample_rate * 10,
                "Track processing latency exceeds ten seconds"
            );
            for clip in &track.clips {
                if clip.is_take_source() || clip.muted {
                    continue;
                }
                end_sample = end_sample.max(to_sample(clip.start + clip.length));
            }
            ensure!(
                track.gain.is_finite() && track.pan.is_finite(),
                "Track gain and pan must be finite"
            );
            let content = live_edit::compile_content(
                track,
                enabled,
                live_edit::CompileSettings {
                    tempo: project.tempo,
                    sample_rate,
                    options,
                    cancel,
                    preview: false,
                },
                &mut audio_cache,
                &mut cache_bytes,
            )?;
            let mut instrument_inputs = Vec::new();
            let mut raw_outputs = Vec::new();
            let (processor, plugin_latency) = match &track.instrument {
                Instrument::Vst3 { path, class_id } if enabled && track.kind == TrackKind::Midi => {
                    check_cancelled()?;
                    let mut plugin = VstInstrument::load_class_with_mode(
                        Path::new(path),
                        class_id.as_deref(),
                        sample_rate as f64,
                        BLOCK,
                        processing_mode,
                    )
                    .with_context(|| format!("Loading VST3 instrument on {}", track.name))?;
                    check_cancelled()?;
                    if !track.instrument_state.is_empty() {
                        plugin.load_state(&track.instrument_state)?;
                    }
                    for parameter in &track.instrument_parameters {
                        check_cancelled()?;
                        plugin.set_parameter(parameter.id, parameter.value)?;
                    }
                    let input_buses: Vec<_> = track
                        .instrument_inputs
                        .iter()
                        .map(|route| route.bus as usize)
                        .collect();
                    let selected_outputs: Vec<_> = project
                        .tracks
                        .iter()
                        .filter(|child| needed.contains(&child.id))
                        .filter_map(|child| {
                            child
                                .instrument_output
                                .filter(|output| output.track == track.id)
                        })
                        .collect();
                    ensure!(
                        selected_outputs.len() <= 128,
                        "Instrument has too many selected output pairs"
                    );
                    let output_buses: Vec<_> = selected_outputs
                        .iter()
                        .map(|output| output.bus as usize)
                        .collect();
                    plugin.configure_audio_buses(&input_buses, &output_buses)?;
                    let layout = plugin.audio_bus_layout();
                    instrument_inputs = aux_inputs(&track.instrument_inputs, &layout, None)?;
                    for output in selected_outputs {
                        let bus = output.bus as usize;
                        let first_channel = output.first_channel as usize;
                        ensure!(
                            bus != plugin.main_output_bus() || first_channel != 0,
                            "Instrument output channel duplicates its primary output"
                        );
                        let native = layout
                            .outputs
                            .get(bus)
                            .context("Instrument output bus is unavailable")?;
                        ensure!(
                            native.active && first_channel < native.channel_count,
                            "Instrument output channel pair is unavailable"
                        );
                        if !raw_outputs.iter().any(|output: &RawOutput| {
                            output.bus == bus && output.first_channel == first_channel
                        }) {
                            raw_outputs.push(RawOutput {
                                bus,
                                first_channel,
                                left: vec![0.0; BLOCK],
                                right: vec![0.0; BLOCK],
                            });
                        }
                    }
                    check_cancelled()?;
                    let latency = plugin.latency_samples();
                    // VST3's infinite-tail sentinel is bounded to keep automatic stop useful.
                    local_tail += (plugin.tail_samples() as u64).min(sample_rate as u64 * 30);
                    (Processor::Vst(Box::new(plugin)), latency)
                }
                Instrument::Builtin { device } if enabled && track.kind == TrackKind::Midi => {
                    // An ID this version does not know stays silent.
                    match crate::builtin::BuiltinDevice::create(
                        device,
                        sample_rate,
                        BLOCK,
                        &track.instrument_parameters,
                    ) {
                        Some(device) => {
                            local_tail +=
                                (device.tail_samples() as u64).min(sample_rate as u64 * 30);
                            let latency = device.latency_samples();
                            (Processor::Builtin(Box::new(device)), latency)
                        }
                        None => (Processor::None, 0),
                    }
                }
                _ => (Processor::None, 0),
            };
            let mut instrument_automation = plugin_automation(project, track.id, None);
            match &processor {
                Processor::Vst(plugin) => {
                    initialize_automation_defaults(&mut instrument_automation, || {
                        plugin.parameters()
                    })?
                }
                Processor::Builtin(_) => initialize_builtin_defaults(
                    &mut instrument_automation,
                    track.instrument.builtin_device(),
                ),
                Processor::None => {}
            }
            let control =
                |parameter, manual| AutomatedValue::new(project, track.id, parameter, manual);
            let event_capacity = content.events.len() + content.notes.len() + 4;
            let note_capacity = content.notes.len();
            let mut inserts = Vec::new();
            let mut latency = track.latency_samples.saturating_add(plugin_latency);
            for slot in &track.inserts {
                check_cancelled()?;
                if !enabled || !channel_active || !slot.enabled {
                    continue;
                }
                inserts.push(match &slot.effect {
                    Effect::Vst3 { path, class_id } => {
                        let mut effect = VstEffect::load_class_with_mode(
                            Path::new(path),
                            class_id.as_deref(),
                            sample_rate as f64,
                            BLOCK,
                            processing_mode,
                        )
                        .with_context(|| {
                            format!("Loading insert {} on {}", slot.name, track.name)
                        })?;
                        check_cancelled()?;
                        if !slot.state.is_empty() {
                            effect.load_state(&slot.state)?;
                        }
                        for parameter in &slot.parameters {
                            check_cancelled()?;
                            effect.set_parameter(parameter.id, parameter.value)?;
                        }
                        let input_buses: Vec<_> =
                            slot.inputs.iter().map(|route| route.bus as usize).collect();
                        effect.configure_audio_buses(&input_buses, &[])?;
                        let inputs = aux_inputs(
                            &slot.inputs,
                            &effect.audio_bus_layout(),
                            effect.main_input_bus(),
                        )?;
                        check_cancelled()?;
                        latency = latency.saturating_add(effect.latency_samples());
                        local_tail += (effect.tail_samples() as u64).min(sample_rate as u64 * 30);
                        let mut parameters = plugin_automation(project, track.id, Some(slot.id));
                        initialize_automation_defaults(&mut parameters, || effect.parameters())?;
                        Insert {
                            id: slot.id,
                            effect: InsertProcessor::Vst(Box::new(effect)),
                            parameters,
                            inputs,
                            arrival_latency: 0,
                            main_delay: CompensationDelay::new(0),
                        }
                    }
                    Effect::Builtin { device } => {
                        ensure!(
                            slot.inputs.is_empty(),
                            "Built-in insert {} on {} has no auxiliary inputs",
                            slot.name,
                            track.name
                        );
                        // An ID this version does not know passes audio
                        // through; the app shows it as an unknown device.
                        let Some(device) = crate::builtin::BuiltinDevice::create(
                            device,
                            sample_rate,
                            BLOCK,
                            &slot.parameters,
                        ) else {
                            continue;
                        };
                        latency = latency.saturating_add(device.latency_samples());
                        local_tail += (device.tail_samples() as u64).min(sample_rate as u64 * 30);
                        let mut parameters = plugin_automation(project, track.id, Some(slot.id));
                        initialize_builtin_defaults(&mut parameters, slot.effect.builtin_device());
                        Insert {
                            id: slot.id,
                            effect: InsertProcessor::Builtin(Box::new(device)),
                            parameters,
                            inputs: Vec::new(),
                            arrival_latency: 0,
                            main_delay: CompensationDelay::new(0),
                        }
                    }
                });
            }
            let mut routes = Vec::new();
            if enabled && unmuted_channel(project, track) {
                let target = if track.id == 0 {
                    None
                } else {
                    Some(
                        track
                            .output_bus
                            .map(|target| indices[&target])
                            .unwrap_or(indices[&0]),
                    )
                };
                let target_needed = target.is_none_or(|index| needed.contains(&order[index]));
                let main_audible = track.id == 0
                    || audible.contains(id)
                    || track
                        .output_bus
                        .is_some_and(|id| needed.contains(&id) && !audible.contains(&id));
                if target_needed && main_audible {
                    routes.push(Route {
                        target,
                        gain: AutomatedValue {
                            manual: 1.0,
                            smooth_value: 1.0,
                            smooth_remaining: 0,
                            lane: None,
                        },
                        pre_fader: false,
                        delay: CompensationDelay::new(0),
                        output: vec![[0.0; 2]; BLOCK],
                        kind: RouteKind::Audio,
                        signal_latency: 0,
                        send_bus: None,
                    });
                }
                for send in &track.sends {
                    let gain = control(
                        AutomationParameter::SendGain {
                            bus: send.target_bus,
                        },
                        send.gain,
                    );
                    if (send.gain > 0.0 || gain.lane.is_some())
                        && needed.contains(&send.target_bus)
                        && (audible.contains(id) || !audible.contains(&send.target_bus))
                    {
                        routes.push(Route {
                            target: Some(indices[&send.target_bus]),
                            gain,
                            pre_fader: send.pre_fader,
                            delay: CompensationDelay::new(0),
                            output: vec![[0.0; 2]; BLOCK],
                            kind: RouteKind::Audio,
                            signal_latency: 0,
                            send_bus: Some(send.target_bus),
                        });
                    }
                }
            }
            let expression_support = match &processor {
                Processor::Vst(plugin) => midi_expression::Support::query(plugin)?,
                Processor::Builtin(_) => midi_expression::Support::builtin(),
                Processor::None => midi_expression::Support::default(),
            };
            let expression =
                ExpressionRuntime::new(Arc::clone(&content.expression), expression_support);
            let stream_windows = content
                .audio
                .iter()
                .any(|clip| clip.data.is_streamed())
                .then(crate::audio_stream::windows)
                .transpose()?;
            tracks.push(RenderTrack {
                id: track.id,
                name: track.name.clone(),
                pending_error: None,
                audio: content.audio,
                audio_segments: content.audio_segments,
                audio_transition: vec![[0.0; 2]; (sample_rate as usize / 200).max(2)],
                audio_transition_cursor: (sample_rate as usize / 200).max(2),
                stream_windows,
                audio_incoming: vec![[0.0; 2]; BLOCK],
                audio_outgoing: vec![[0.0; 2]; BLOCK],
                notes: content.notes,
                events: content.events,
                note_order: content.note_order,
                event_cursor: 0,
                processor,
                expression,
                midi: Vec::with_capacity(event_capacity),
                live_midi: Vec::with_capacity(4),
                chase: Vec::with_capacity(event_capacity),
                chase_delay: 0,
                active_notes: Vec::with_capacity(note_capacity),
                active_indices: HashMap::with_capacity(note_capacity),
                left: vec![0.0; BLOCK],
                right: vec![0.0; BLOCK],
                mix_left: vec![0.0; BLOCK],
                mix_right: vec![0.0; BLOCK],
                input_gain: control(AutomationParameter::TrackInputGain, track.input_gain),
                polarity: AutomatedValue::manual(polarity_sign(track)),
                width: AutomatedValue::manual(track.stereo_width),
                gain: control(AutomationParameter::TrackGain, track.gain),
                pan: control(AutomationParameter::TrackPan, track.pan),
                instrument_automation,
                delay: CompensationDelay::new(track.latency_samples as usize),
                latency,
                input_latency: 0,
                source_latency: plugin_latency,
                source_arrival: 0,
                instrument_input_latency: 0,
                instrument_inputs,
                raw_outputs,
                post_fader: vec![[0.0; 2]; BLOCK],
                freeze_tap: None,
                tail_samples: local_tail.min(sample_rate as u64 * 30),
                plugin_failed: false,
                channel_failed: false,
                processing_channel: false,
                processing_frames: 0,
                enabled,
                channel_active,
                inserts,
                insert_left: vec![0.0; BLOCK],
                insert_right: vec![0.0; BLOCK],
                routes,
                meter: None,
                #[cfg(test)]
                observer: None,
            });
        }
        // Append dependency edges after every native processor has negotiated
        // its bus layout. These edges never sum into the main input by accident.
        for target in 0..tracks.len() {
            if !tracks[target].enabled {
                continue;
            }
            let model = project
                .track(order[target])
                .expect("compiled channel exists");
            let mut input_routes = Vec::new();
            for input in &model.instrument_inputs {
                let slot = tracks[target]
                    .instrument_inputs
                    .iter()
                    .position(|buffer| {
                        buffer.bus == input.bus as usize
                            && buffer.first_channel == input.first_channel as usize
                    })
                    .context("Instrument input pair was not prepared")?;
                input_routes.push((input, None, slot));
            }
            for (effect_index, insert) in tracks[target].inserts.iter().enumerate() {
                let Insert { id, inputs, .. } = insert;
                let model_slot = model
                    .inserts
                    .iter()
                    .find(|slot| slot.id == *id)
                    .expect("prepared insert exists");
                for input in &model_slot.inputs {
                    let slot = inputs
                        .iter()
                        .position(|buffer| {
                            buffer.bus == input.bus as usize
                                && buffer.first_channel == input.first_channel as usize
                        })
                        .context("Effect input pair was not prepared")?;
                    input_routes.push((input, Some(effect_index), slot));
                }
            }
            for (input, effect, slot) in input_routes {
                let source = indices[&input.source_track];
                ensure!(source < target, "Plug-in input dependency is not acyclic");
                if !tracks[source].enabled
                    || !unmuted_channel(project, project.track(input.source_track).unwrap())
                {
                    continue;
                }
                tracks[source].routes.push(Route {
                    target: Some(target),
                    gain: AutomatedValue {
                        manual: input.gain,
                        smooth_value: input.gain,
                        smooth_remaining: 0,
                        lane: None,
                    },
                    pre_fader: input.pre_fader,
                    delay: CompensationDelay::new(0),
                    output: vec![[0.0; 2]; BLOCK],
                    kind: RouteKind::PluginInput {
                        effect,
                        input: slot,
                    },
                    signal_latency: 0,
                    send_bus: None,
                });
            }
            if let Some(output) = model.instrument_output {
                let source = indices[&output.track];
                ensure!(
                    source < target,
                    "Instrument output dependency is not acyclic"
                );
                let raw = tracks[source]
                    .raw_outputs
                    .iter()
                    .position(|buffer| {
                        buffer.bus == output.bus as usize
                            && buffer.first_channel == output.first_channel as usize
                    })
                    .context("Instrument output pair was not prepared")?;
                tracks[source].routes.push(Route {
                    target: Some(target),
                    gain: AutomatedValue {
                        manual: 1.0,
                        smooth_value: 1.0,
                        smooth_remaining: 0,
                        lane: None,
                    },
                    pre_fader: true,
                    delay: CompensationDelay::new(0),
                    output: vec![[0.0; 2]; BLOCK],
                    kind: RouteKind::InstrumentOutput { output: raw },
                    signal_latency: 0,
                    send_bus: None,
                });
            }
        }
        // Align at the receiving processor stage: a late sidechain delays the
        // main signal before that insert, rather than misaligning its detector.
        let mut input_latency = vec![0_u32; tracks.len()];
        let mut auxiliary_latency = vec![HashMap::<Option<usize>, u32>::new(); tracks.len()];
        let mut input_tail = vec![0_u64; tracks.len()];
        let mut latency = 0;
        for index in 0..tracks.len() {
            if !tracks[index].enabled {
                continue;
            }
            let track = &mut tracks[index];
            track.input_latency = input_latency[index];
            track.instrument_input_latency =
                auxiliary_latency[index].get(&None).copied().unwrap_or(0);
            track.source_arrival = input_latency[index].max(
                track
                    .instrument_input_latency
                    .saturating_add(track.source_latency),
            );
            let mut arrival = track.source_arrival;
            for (effect_index, insert) in track.inserts.iter_mut().enumerate() {
                let Insert {
                    effect,
                    arrival_latency,
                    main_delay,
                    ..
                } = insert;
                *arrival_latency = arrival.max(
                    auxiliary_latency[index]
                        .get(&Some(effect_index))
                        .copied()
                        .unwrap_or(0),
                );
                *main_delay = CompensationDelay::new((*arrival_latency - arrival) as usize);
                arrival = arrival_latency.saturating_add(effect.latency_samples());
            }
            track.latency = arrival.saturating_add(track.delay.latency_samples() as u32);
            ensure!(
                track.latency <= sample_rate * 10,
                "Processing path latency exceeds ten seconds"
            );
            track.tail_samples =
                (track.tail_samples + input_tail[index]).min(sample_rate as u64 * 30);
            let raw_latency = track
                .instrument_input_latency
                .saturating_add(track.source_latency);
            for route in &mut track.routes {
                route.signal_latency = if matches!(route.kind, RouteKind::InstrumentOutput { .. }) {
                    raw_latency
                } else {
                    track.latency
                };
                if let Some(target) = route.target {
                    match route.kind {
                        RouteKind::PluginInput { effect, .. } => {
                            let stage = auxiliary_latency[target].entry(effect).or_default();
                            *stage = (*stage).max(route.signal_latency);
                        }
                        _ => {
                            input_latency[target] = input_latency[target].max(route.signal_latency)
                        }
                    }
                    input_tail[target] = input_tail[target].max(track.tail_samples);
                } else {
                    latency = latency.max(track.latency);
                    tail_samples = tail_samples.max(track.tail_samples);
                }
            }
        }
        if let Some(capture) = freeze_capture {
            for track in &tracks {
                if capture.contains(&track.id) {
                    latency = latency.max(track.latency);
                }
            }
        }
        ensure!(
            latency <= sample_rate * 10,
            "Processing latency exceeds ten seconds"
        );
        for index in 0..tracks.len() {
            for route_index in 0..tracks[index].routes.len() {
                let route = &tracks[index].routes[route_index];
                let target_latency = match (route.target, route.kind) {
                    (
                        Some(target),
                        RouteKind::PluginInput {
                            effect: Some(effect),
                            ..
                        },
                    ) => {
                        let Insert {
                            arrival_latency, ..
                        } = &tracks[target].inserts[effect];
                        *arrival_latency
                    }
                    (Some(target), RouteKind::PluginInput { effect: None, .. }) => {
                        tracks[target].instrument_input_latency
                    }
                    (Some(target), _) => input_latency[target],
                    (None, _) => latency,
                };
                ensure!(
                    target_latency >= route.signal_latency,
                    "Invalid processing latency alignment"
                );
                tracks[index].routes[route_index].delay =
                    CompensationDelay::new((target_latency - route.signal_latency) as usize);
            }
            if freeze_capture.is_some_and(|ids| ids.contains(&tracks[index].id)) {
                ensure!(
                    latency >= tracks[index].latency,
                    "Freeze channel is not connected to the master timing reference"
                );
                tracks[index].freeze_tap = Some(FreezeTap {
                    delay: CompensationDelay::new((latency - tracks[index].latency) as usize),
                    frames: vec![[0.0; 2]; BLOCK],
                });
            }
        }
        let loop_range = if looping && project.loop_enabled {
            let start = to_sample(project.loop_start);
            let end = to_sample(project.loop_end);
            ensure!(end > start, "Loop end must be after loop start");
            Some((start, end))
        } else {
            None
        };
        // Compile source-before-destination levels once. Within each level routing
        // stays in stable topological order, independent of worker completion order.
        let mut depths = vec![0; tracks.len()];
        for (index, track) in tracks.iter().enumerate() {
            for target in track.routes.iter().filter_map(|route| route.target) {
                depths[target] = depths[target].max(depths[index] + 1);
            }
        }
        let mut levels = vec![Vec::new(); depths.iter().copied().max().unwrap_or(0) + 1];
        for (index, &depth) in depths.iter().enumerate() {
            if tracks[index].enabled {
                levels[depth].push(index);
            }
        }
        let level_width = levels.iter().map(Vec::len).max().unwrap_or(0);
        let (useful_parallelism, width) = schedule::parallelism(&tracks, level_width);
        let available = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let worker_count = requested_workers
            .unwrap_or(if useful_parallelism { available } else { 1 })
            .min(available.max(2))
            .min(width);
        check_cancelled()?;
        let mut track_performance: Vec<_> = tracks
            .iter()
            .map(|track| TrackRenderPerformance {
                track_id: track.id,
                ..Default::default()
            })
            .collect();
        let workers = if worker_count > 1 {
            Some(WorkerPool::new(
                worker_count,
                &tracks,
                &levels,
                &mut track_performance,
            )?)
        } else {
            None
        };
        let mut streamed_clips: Vec<_> = tracks
            .iter()
            .flat_map(|track| {
                track
                    .audio
                    .iter()
                    .filter(|clip| clip.data.is_streamed())
                    .map(|clip| (track.id, clip.id))
            })
            .collect();
        streamed_clips.sort_unstable();
        let streamed_clips = Arc::new(streamed_clips);
        let mut renderer = Self {
            streamed_clips,
            resident_audio_bytes: cache_bytes as u64,
            retired_audio_reads: 0,
            retired_audio_frames: 0,
            signal_monitor: None,
            signal_monitor_transport: 0,
            signal_monitor_active: false,
            tracks: tracks
                .into_iter()
                .map(|track| Some(Box::new(track)))
                .collect(),
            levels,
            workers,
            performance: RenderPerformanceSnapshot::default(),
            track_performance,
            profile_serial: false,
            sample_rate,
            tempo: project.tempo,
            logical_sample: 0,
            source_sample: 0,
            end_sample: end_sample + tail_samples,
            timeline_end_sample: end_sample + tail_samples,
            release_end_sample: 0,
            latency,
            loop_range,
            tolerate_plugin_failure: looping,
            plugin_errors: 0,
            preview: false,
            audition: None,
            audition_wheels: None,
            midi_input_notes: Vec::with_capacity(4096),
            expressive_input: Vec::with_capacity(16384),
            live_edit_context: Arc::new(LiveEditContext::new(
                project,
                sample_rate,
                options,
                tail_samples,
            )),
        };
        renderer.seek_sample(0, false)?;
        check_cancelled()?;
        Ok(renderer)
    }

    /// Realtime-only telemetry; offline renders do not calculate or publish meters.
    pub(crate) fn attach_channel_meters(&mut self, meters: &ChannelMeters) {
        for track in self.tracks.iter_mut().flatten() {
            track.meter = meters.channel(track.id).cloned();
        }
    }

    /// Lists the telemetry of every built-in device of this graph, for
    /// [`crate::builtin::DeviceTelemetry`].
    pub(crate) fn device_telemetry(&self, out: &mut crate::builtin::TelemetryList) {
        out.clear();
        for track in self.tracks.iter().flatten() {
            if let Processor::Builtin(device) = &track.processor {
                out.push(((track.id, None), device.telemetry()));
            }
            for insert in &track.inserts {
                if let InsertProcessor::Builtin(device) = &insert.effect {
                    out.push(((track.id, Some(insert.id)), device.telemetry()));
                }
            }
        }
    }

    pub(crate) fn attach_signal_monitor(
        &mut self,
        monitor: Arc<crate::signal_monitor::SignalMonitor>,
    ) {
        self.signal_monitor = Some(monitor);
    }

    pub(crate) fn signal_monitor_transport(&mut self, generation: u64, active: bool) {
        self.signal_monitor_transport = generation;
        self.signal_monitor_active = active;
    }

    pub(crate) fn set_plugin_parameter(
        &mut self,
        track_id: u64,
        effect_id: Option<u64>,
        parameter_id: u32,
        value: f64,
    ) -> Result<()> {
        self.set_plugin_parameter_mode(track_id, effect_id, parameter_id, value, None)
    }

    pub(crate) fn set_plugin_parameter_override(
        &mut self,
        track_id: u64,
        effect_id: Option<u64>,
        parameter_id: u32,
        value: f64,
        sequence: u64,
    ) -> Result<()> {
        self.set_plugin_parameter_mode(track_id, effect_id, parameter_id, value, Some(sequence))
    }

    fn set_plugin_parameter_mode(
        &mut self,
        track_id: u64,
        effect_id: Option<u64>,
        parameter_id: u32,
        value: f64,
        override_sequence: Option<u64>,
    ) -> Result<()> {
        ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "Invalid normalized parameter value"
        );
        let track = self
            .tracks
            .iter_mut()
            .flatten()
            .find(|track| track.id == track_id)
            .context("Track is not in the render graph")?;
        if let Some(effect_id) = effect_id {
            let Insert {
                effect, parameters, ..
            } = track
                .inserts
                .iter_mut()
                .find(|insert| matches!(insert, Insert { id, .. } if *id == effect_id))
                .context("Insert is not active in the render graph")?;
            apply_plugin_parameter_change(
                parameters,
                parameter_id,
                value,
                override_sequence,
                |id, value| {
                    match effect {
                    InsertProcessor::Vst(effect) => effect.set_parameter(id, value).with_context(|| {
                        format!(
                            "Setting parameter {parameter_id} of effect \"{}\" ({}) on channel \"{}\" (ID {})",
                            effect.info().name, effect.info().path.display(), track.name, track.id
                        )
                    }),
                    InsertProcessor::Builtin(device) => {
                        device.set_parameter(id, value);
                        Ok(())
                    }
                }
                },
            )?;
        } else if let Processor::Vst(plugin) = &mut track.processor {
            apply_plugin_parameter_change(
                &mut track.instrument_automation,
                parameter_id,
                value,
                override_sequence,
                |id, value| {
                    plugin.set_parameter(id, value).with_context(|| {
                        format!(
                            "Setting parameter {parameter_id} of instrument \"{}\" ({}) on channel \"{}\" (ID {})",
                            plugin.info().name, plugin.info().path.display(), track.name, track.id
                        )
                    })
                },
            )?;
        } else if let Processor::Builtin(device) = &mut track.processor {
            apply_plugin_parameter_change(
                &mut track.instrument_automation,
                parameter_id,
                value,
                override_sequence,
                |id, value| {
                    device.set_parameter(id, value);
                    Ok(())
                },
            )?;
        } else {
            bail!("Track has no assigned instrument")
        }
        Ok(())
    }

    /// Drain one contained plugin failure after the graph workers return their nodes.
    /// Failed paths are latched, so subsequent blocks do not recreate the diagnostic.
    pub(crate) fn take_plugin_error(&mut self) -> Option<String> {
        self.tracks
            .iter_mut()
            .flatten()
            .find_map(|track| track.pending_error.take())
    }

    pub(crate) fn finished(&self) -> bool {
        self.loop_range.is_none() && self.logical_sample >= self.end_sample
    }

    pub(crate) fn seek(&mut self, beat: f64) -> Result<()> {
        ensure!(
            beat.is_finite() && (0.0..=1_000_000.0).contains(&beat),
            "Seek position is outside the supported timeline"
        );
        // Match clip/event/loop compilation exactly, including half-sample
        // boundaries. Reordering these operations can move a seek one sample.
        let samples_per_beat = self.sample_rate as f64 * 60.0 / self.tempo;
        let sample = (beat * samples_per_beat).round() as u64;
        self.seek_sample(sample, false)
    }

    fn seek_sample(&mut self, sample: u64, loop_wrap: bool) -> Result<()> {
        self.release_end_sample = 0;
        self.end_sample = self.timeline_end_sample;
        if !loop_wrap {
            self.audition = None;
            // Instruments reset on a seek; send the wheels again.
            self.audition_wheels = None;
            self.midi_input_notes.clear();
            self.expressive_input.clear();
        }
        self.source_sample = sample;
        self.logical_sample = sample;
        let wheels = self.audition_wheels;
        for track in self.tracks.iter_mut().flatten() {
            track.audio_transition_cursor = track.audio_transition.len();
            for parameter in &mut track.instrument_automation {
                parameter.previous = None;
            }
            track.delay.reset();
            if let Some(tap) = &mut track.freeze_tap {
                tap.delay.reset();
            }
            for route in &mut track.routes {
                route.delay.reset();
            }
            for insert in &mut track.inserts {
                match insert {
                    Insert {
                        effect,
                        parameters,
                        main_delay,
                        ..
                    } if !track.plugin_failed && !track.channel_failed => {
                        main_delay.reset();
                        for parameter in parameters {
                            parameter.previous = None;
                        }
                        let effect = match effect {
                            InsertProcessor::Vst(effect) => effect,
                            InsertProcessor::Builtin(device) => {
                                device.reset();
                                continue;
                            }
                        };
                        if let Err(error) = effect.reset() {
                            if !self.tolerate_plugin_failure {
                                return Err(error);
                            }
                            track.channel_failed = true;
                            track.channel_active = false;
                            track.pending_error = Some(format!(
                                "Plugin reset failed on \"{}\"\n\nResetting effect \"{}\" ({}) on channel ID {}: {error:#}",
                                track.name,
                                effect.info().name,
                                effect.info().path.display(),
                                track.id
                            ));
                            self.plugin_errors += 1;
                        }
                    }
                    Insert { .. } => {}
                }
            }
            track.event_cursor = track.events.partition_point(|event| event.sample < sample);
            track.chase_delay = track.instrument_input_latency;
            track
                .expression
                .seek(sample, track.instrument_input_latency, !loop_wrap);
            // A loop wrap resets touched controllers to their defaults; the
            // panel's wheels are still where the user holds them.
            if let Some((_, pitch, modulation)) = wheels.filter(|(id, _, _)| *id == track.id) {
                use daw_plugins::ExpressiveController as Controller;
                track.expression.chase_after_seek(
                    0,
                    Controller::PitchBend,
                    (f64::from(pitch) + 1.0) / 2.0,
                );
                track.expression.chase_after_seek(
                    0,
                    Controller::ControlChange(1),
                    f64::from(modulation),
                );
            }
            track.chase.clear();
            if loop_wrap {
                // Looping restarts sequencer notes, not the independent live
                // input/audition voices already sounding in this instrument.
                // A whole-instrument reset would kill and retrigger those keys.
                track
                    .chase
                    .extend(track.active_notes.iter().map(|&note| Event {
                        sample,
                        note,
                        on: false,
                    }));
            }
            track.active_notes.clear();
            track.active_indices.clear();
            if !loop_wrap {
                track.live_midi.clear();
            }
            match &mut track.processor {
                Processor::None => {}
                Processor::Vst(plugin) if !track.plugin_failed && !loop_wrap => {
                    if let Err(error) = plugin.reset() {
                        if !self.tolerate_plugin_failure {
                            return Err(error);
                        }
                        track.plugin_failed = true;
                        track.pending_error = Some(format!(
                            "Plugin reset failed on \"{}\"\n\nResetting instrument \"{}\" ({}) on channel ID {}: {error:#}",
                            track.name,
                            plugin.info().name,
                            plugin.info().path.display(),
                            track.id
                        ));
                        self.plugin_errors += 1;
                    }
                }
                Processor::Builtin(device) if !track.plugin_failed && !loop_wrap => device.reset(),
                Processor::Vst(_) | Processor::Builtin(_) => {}
            }
            for &note in track.notes.iter() {
                if !self.preview && note.start < sample && note.end > sample {
                    match &mut track.processor {
                        Processor::None => {}
                        Processor::Vst(_) | Processor::Builtin(_) => track.chase.push(Event {
                            sample,
                            note,
                            on: true,
                        }),
                    }
                }
            }
        }
        // Prime every path equally, removing common latency from transport/export timing.
        let mut discard = [[0.0; 2]; BLOCK];
        let mut remaining = self.latency as usize;
        while remaining > 0 {
            let count = remaining.min(BLOCK);
            self.segment(&mut discard[..count], false)?;
            remaining -= count;
        }
        // Priming feeds latency compensation, not audible playback. Do not let
        // discarded audio or a previous seek leave the visible meter ringing.
        for track in self.tracks.iter().flatten() {
            if let Some(meter) = &track.meter {
                meter.clear();
            }
        }
        Ok(())
    }

    pub(crate) fn set_preview(&mut self, preview: bool, beat: f64) -> Result<()> {
        self.preview = preview;
        self.seek(beat)
    }

    /// Only the render worker touches plugin state. A held note is replaced as a
    /// single command, so lost UI frames cannot leave a stale note sounding.
    pub(crate) fn set_audition(&mut self, note: Option<(u64, u8, u8)>) -> Result<()> {
        if self.audition == note {
            return Ok(());
        }
        if let Some((id, pitch, _)) = self.audition.take()
            && let Some(track) = self
                .tracks
                .iter_mut()
                .flatten()
                .find(|track| track.id == id)
        {
            track.live_midi.push(MidiMessage {
                offset: 0,
                pitch,
                velocity: 0,
                on: false,
            });
        }
        if let Some((id, pitch, velocity)) = note {
            ensure!(
                pitch < 128 && velocity > 0 && velocity < 128,
                "Invalid audition note"
            );
            let track = self
                .tracks
                .iter_mut()
                .flatten()
                .find(|track| track.id == id)
                .context("Audition track is not in the render graph")?;
            ensure!(
                !matches!(track.processor, Processor::None),
                "Assign a plugin instrument to audition notes"
            );
            track.live_midi.push(MidiMessage {
                offset: 0,
                pitch,
                velocity,
                on: true,
            });
            self.audition = note;
        }
        Ok(())
    }

    /// Sends changed panel wheels to their track's instrument as pitch bend
    /// and mod wheel, through the live input queue. Detaching the wheels
    /// centres the bend they left behind, as a wheel springs back; the mod
    /// wheel stays where it was, as a hardware one does.
    pub(crate) fn set_audition_wheels(&mut self, wheels: Option<(u64, f32, f32)>) {
        use daw_plugins::ExpressiveController as Controller;
        let previous = std::mem::replace(&mut self.audition_wheels, wheels);
        if previous == wheels {
            return;
        }
        if let Some((id, pitch, _)) = previous
            && pitch != 0.0
            && wheels.is_none_or(|(next, _, _)| next != id)
        {
            self.send_live_controller(id, Controller::PitchBend, 0.5);
        }
        if let Some((id, pitch, modulation)) = wheels {
            self.send_live_controller(id, Controller::PitchBend, (f64::from(pitch) + 1.0) / 2.0);
            self.send_live_controller(id, Controller::ControlChange(1), f64::from(modulation));
        }
    }

    fn send_live_controller(
        &mut self,
        id: u64,
        controller: daw_plugins::ExpressiveController,
        value: f64,
    ) {
        use daw_plugins::{ExpressiveMidiData as Data, ExpressiveMidiMessage as Message};
        let Some(track) = self
            .tracks
            .iter_mut()
            .flatten()
            .find(|track| track.id == id)
        else {
            return;
        };
        if track.expression.live.len() < 4096 {
            track.expression.live.push((
                0,
                Message {
                    offset: 0,
                    data: Data::Controller {
                        channel: 0,
                        controller,
                        value,
                    },
                },
            ));
        }
    }

    /// Normalize the complete held MIDI input chord on the render worker. A
    /// route or device disappearing releases every voice, and seek/reset clears
    /// the remembered set so currently held keys are restored on the next block.
    pub(crate) fn streamed_clips(&self) -> Arc<Vec<(u64, u64)>> {
        Arc::clone(&self.streamed_clips)
    }
    pub(crate) fn audio_streaming_snapshot(&self) -> crate::audio_stream::AudioStreamingSnapshot {
        let (mut reads, mut frames) = (self.retired_audio_reads, self.retired_audio_frames);
        for track in self.tracks.iter().flatten() {
            if let Some(windows) = &track.stream_windows {
                for window in windows.iter() {
                    reads += window.metrics.operations.load(Ordering::Relaxed);
                    frames += window.metrics.frames.load(Ordering::Relaxed);
                }
            }
        }
        crate::audio_stream::AudioStreamingSnapshot {
            streamed_clips: self.streamed_clips.len() as u32,
            resident_bytes: self.resident_audio_bytes,
            window_bytes: crate::audio_stream::window_bytes(),
            disk_read_operations: reads,
            disk_frames_read: frames,
        }
    }
    pub(crate) fn unsupported_midi(&self) -> (u64, u64) {
        self.tracks
            .iter()
            .flatten()
            .fold((0, 0), |(events, groups), track| {
                (
                    events + track.expression.unsupported_count(),
                    groups + track.expression.group_count(),
                )
            })
    }
    pub(crate) fn set_expressive_input(
        &mut self,
        state: &[(u64, u8, u8, daw_plugins::ExpressiveMidiData)],
        releases: &[(u64, daw_plugins::ExpressiveMidiData)],
        offset: u32,
    ) -> Result<()> {
        use daw_plugins::{
            ExpressiveMidiData as Data, ExpressiveMidiMessage as Message,
            ExpressiveVelocity as Velocity,
        };
        ensure!(state.len() <= 16384, "Too many live MIDI states");
        let mut controllers_reset = false;
        for &(id, group, channel, data) in &self.expressive_input {
            let Some(track) = self
                .tracks
                .iter_mut()
                .flatten()
                .find(|track| track.id == id)
            else {
                continue;
            };
            let admitted = track
                .expression
                .content
                .group
                .is_none_or(|stored| stored == group);
            match data {
                Data::Controller { controller, .. } => {
                    let present=admitted&&state.iter().any(|(target,g,_,new)|*target==id&&*g==group&&matches!(new,Data::Controller{channel:c,controller:address,..}if *c==channel&&*address==controller));
                    if !present
                        && let Some(value) = track
                            .expression
                            .support
                            .controller_default(channel, controller)
                        && track.expression.live.len() < 4096
                    {
                        controllers_reset = true;
                        track.expression.live.push((
                            channel,
                            Message {
                                offset,
                                data: Data::Controller {
                                    channel,
                                    controller,
                                    value,
                                },
                            },
                        ));
                    }
                }
                Data::NoteOn {
                    pitch,
                    note_id,
                    tuning,
                    ..
                } => {
                    let present = admitted
                        && state.iter().any(|(target, _, _, new)| {
                            *target == id
                                && matches!(new,Data::NoteOn{note_id:other,..}if *other==note_id)
                        });
                    if !present {
                        let off=releases.iter().find_map(|(target,data)|if *target==id&&matches!(data,Data::NoteOff{note_id:other,..}if *other==note_id){Some(*data)}else{None}).unwrap_or(Data::NoteOff{channel,pitch,velocity:Velocity::Midi2(0),note_id,tuning});
                        if track.expression.live.len() < 4096 {
                            track
                                .expression
                                .live
                                .push((channel, Message { offset, data: off }));
                        }
                    }
                }
                _ => {}
            }
        }
        for &(id, group, channel, data) in state {
            let Some(track) = self
                .tracks
                .iter_mut()
                .flatten()
                .find(|track| track.id == id)
            else {
                continue;
            };
            if track
                .expression
                .content
                .group
                .is_some_and(|stored| stored != group)
            {
                track.expression.reject_group(group);
                continue;
            }
            if self.expressive_input.contains(&(id, group, channel, data))
                && !(controllers_reset && matches!(data, Data::Controller { .. }))
            {
                continue;
            }
            ensure!(
                track.expression.live.len() < 4096,
                "Live MIDI event limit reached"
            );
            track
                .expression
                .live
                .push((channel, Message { offset, data }));
        }
        self.expressive_input.clear();
        self.expressive_input
            .extend(state.iter().copied().filter(|(id, group, _, _)| {
                self.tracks
                    .iter()
                    .flatten()
                    .find(|track| track.id == *id)
                    .is_some_and(|track| {
                        track
                            .expression
                            .content
                            .group
                            .is_none_or(|stored| stored == *group)
                    })
            }));
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_midi_input_notes(
        &mut self,
        notes: &[(u64, u8, u8)],
        offset: u32,
    ) -> Result<()> {
        ensure!(notes.len() <= 4096, "MIDI monitor voice limit exceeded");
        if self.midi_input_notes == notes {
            return Ok(());
        }
        for &(id, pitch, velocity) in &self.midi_input_notes {
            if notes.binary_search(&(id, pitch, velocity)).is_err()
                && let Some(track) = self
                    .tracks
                    .iter_mut()
                    .flatten()
                    .find(|track| track.id == id)
            {
                track.live_midi.push(MidiMessage {
                    offset,
                    pitch,
                    velocity: 0,
                    on: false,
                });
            }
        }
        for &(id, pitch, velocity) in notes {
            if self
                .midi_input_notes
                .binary_search(&(id, pitch, velocity))
                .is_err()
                && let Some(track) = self
                    .tracks
                    .iter_mut()
                    .flatten()
                    .find(|track| track.id == id)
                && !matches!(track.processor, Processor::None)
            {
                track.live_midi.push(MidiMessage {
                    offset,
                    pitch,
                    velocity,
                    on: true,
                });
            }
        }
        self.midi_input_notes.clear();
        self.midi_input_notes.extend_from_slice(notes);
        Ok(())
    }

    pub(crate) fn render_preview(
        &mut self,
        output: &mut [[f32; 2]],
        positions: &mut [f64],
    ) -> Result<()> {
        ensure!(
            self.preview && output.len() == positions.len(),
            "Invalid audition render"
        );
        self.segment(output, true)?;
        positions.fill(self.logical_sample as f64 / self.sample_rate as f64 * self.tempo / 60.0);
        Ok(())
    }

    pub(crate) fn render(&mut self, output: &mut [[f32; 2]], positions: &mut [f64]) -> Result<()> {
        ensure!(
            output.len() == positions.len(),
            "Output and position buffers must match"
        );
        let mut offset = 0;
        while offset < output.len() {
            if let Some((start, end)) = self.loop_range
                && self.logical_sample >= end
            {
                self.seek_sample(start, true)?;
            }
            let remaining = self
                .loop_range
                .map(|(_, end)| end.saturating_sub(self.logical_sample) as usize)
                .unwrap_or(usize::MAX);
            let count = BLOCK.min(output.len() - offset).min(remaining);
            let first_sample = self.logical_sample;
            self.segment(&mut output[offset..offset + count], true)?;
            for (index, position) in positions[offset..offset + count].iter_mut().enumerate() {
                *position = (first_sample + index as u64 + 1) as f64 / self.sample_rate as f64
                    * self.tempo
                    / 60.0;
            }
            self.logical_sample += count as u64;
            offset += count;
        }
        Ok(())
    }

    #[cfg(test)]
    fn set_processing_observer(&mut self, observer: Arc<dyn Fn(u64) + Send + Sync>) {
        for track in self.tracks.iter_mut().flatten() {
            track.observer = Some(Arc::clone(&observer));
        }
    }

    fn segment(&mut self, output: &mut [[f32; 2]], live_input: bool) -> Result<()> {
        let started = Instant::now();
        let sequence = self.performance.sequence.wrapping_add(1);
        for timing in &mut self.track_performance {
            timing.last_process_ns = 0;
            timing.last_ready_wait_ns = 0;
        }
        output.fill([0.0; 2]);
        let mut master = [[0.0_f64; 2]; BLOCK];
        let context = BlockContext {
            start: self.source_sample,
            count: output.len(),
            sample_rate: self.sample_rate,
            tempo: self.tempo,
            timeline: !self.preview,
            live_input,
        };
        for track in self.tracks.iter_mut().flatten() {
            track.left[..context.count].fill(0.0);
            track.right[..context.count].fill(0.0);
            track.mix_left[..context.count].fill(0.0);
            track.mix_right[..context.count].fill(0.0);
            track.post_fader[..context.count].fill([0.0; 2]);
            for input in &mut track.instrument_inputs {
                input.clear(context.count);
            }
            for insert in &mut track.inserts {
                let Insert { inputs, .. } = insert;
                for input in inputs {
                    input.clear(context.count);
                }
            }
            for raw in &mut track.raw_outputs {
                raw.left[..context.count].fill(0.0);
                raw.right[..context.count].fill(0.0);
            }
            if let Some(tap) = &mut track.freeze_tap {
                tap.frames[..context.count].fill([0.0; 2]);
            }
        }
        let mut stats = if let Some(workers) = &mut self.workers {
            let (failures, stats) = workers.process(
                &mut self.tracks,
                context,
                self.tolerate_plugin_failure,
                &mut master[..context.count],
                &mut self.track_performance,
            )?;
            self.plugin_errors += failures;
            stats
        } else {
            let mut stats = RenderPerformanceSnapshot {
                block_frames: context.count,
                track_timings_measured: self.profile_serial,
                ..Default::default()
            };
            for level_index in 0..self.levels.len() {
                let level = &self.levels[level_index];
                for &index in level {
                    let track = self.tracks[index].as_mut().expect("owned graph node");
                    let timed = self.profile_serial.then(Instant::now);
                    let result = track.process(context);
                    if let Some(timed) = timed {
                        let elapsed = nanoseconds(timed.elapsed());
                        self.track_performance[index].record(elapsed, 0);
                        stats.processing_ns = stats.processing_ns.saturating_add(elapsed);
                        stats.max_track_ns = stats.max_track_ns.max(elapsed);
                    }
                    stats.nodes += 1;
                    self.plugin_errors +=
                        finish_processing(track, result, self.tolerate_plugin_failure)?;
                }
                for position in 0..self.levels[level_index].len() {
                    self.route_track(
                        self.levels[level_index][position],
                        &mut master[..context.count],
                    );
                }
            }
            stats
        };
        for (output, accumulated) in output.iter_mut().zip(master) {
            *output = [accumulated[0] as f32, accumulated[1] as f32];
        }
        // Discarded delay-compensation priming must never advance analysis.
        // Disabled/pruned/muted paths contribute actual silence, not stale DSP buffers.
        if live_input
            && self.signal_monitor_active
            && let Some(monitor) = &self.signal_monitor
            && let Some((target, epoch)) = monitor.target()
        {
            let mut frames = [[0.0; 2]; BLOCK];
            match target {
                crate::SignalMonitorTarget::Master => {
                    frames[..context.count].copy_from_slice(output)
                }
                crate::SignalMonitorTarget::Channel(id) => {
                    if let Some(track) = self.tracks.iter().flatten().find(|track| track.id == id)
                        && track.enabled
                        && !track.plugin_failed
                    {
                        for (index, frame) in frames[..context.count].iter_mut().enumerate() {
                            *frame = track.post_fader[index];
                        }
                    }
                }
            }
            monitor.capture(
                target,
                epoch,
                self.signal_monitor_transport,
                &frames[..context.count],
            );
        }
        if !self.preview {
            self.source_sample += context.count as u64;
        }
        stats.sequence = sequence;
        stats.wall_ns = nanoseconds(started.elapsed());
        self.performance = stats;
        Ok(())
    }

    fn route_track(&mut self, track_index: usize, output: &mut [[f64; 2]]) {
        let count = output.len();
        let (processed, remaining) = self.tracks.split_at_mut(track_index + 1);
        let track = processed[track_index]
            .as_mut()
            .expect("completed graph node");
        if !track.enabled || track.plugin_failed {
            return;
        }
        for route in &track.routes {
            if let Some(target) = route.target {
                let destination = remaining[target - track_index - 1]
                    .as_mut()
                    .expect("downstream graph node");
                schedule::mix_route(route, destination, count);
            } else {
                schedule::mix_terminal(route, output);
            }
        }
    }
}

/// Render an arrangement to interleaved stereo floating-point samples.
/// Looping is ignored for export. Plugin delay compensation is primed and trimmed.
pub fn render(project: &Project, sample_rate: u32, length_beats: f64) -> Result<Vec<f32>> {
    render_with_options(project, sample_rate, length_beats, RenderOptions::default())
}

/// Render with explicit source-conversion quality; output is interleaved f32 stereo.
pub fn render_with_options(
    project: &Project,
    sample_rate: u32,
    length_beats: f64,
    options: RenderOptions,
) -> Result<Vec<f32>> {
    render_prepared(
        Renderer::new_offline_with_options(project, sample_rate, options)?,
        length_beats,
    )
}

/// Render with a bounded number of persistent graph workers (one forces serial).
/// Independent tracks run concurrently; insert chains and routing dependencies stay ordered.
pub fn render_with_workers(
    project: &Project,
    sample_rate: u32,
    length_beats: f64,
    workers: usize,
) -> Result<Vec<f32>> {
    render_prepared(
        Renderer::new_offline_with_workers(project, sample_rate, workers)?,
        length_beats,
    )
}

/// Render an end-exclusive range without writing a file or moving live transport.
/// Processing starts at beat zero so instruments, automation and effect tails have
/// their preceding history. Only the requested stereo samples are retained.
/// Cancellation is checked during preparation and between processing blocks;
/// a currently executing isolated plug-in command retains the bounded offline
/// deadline rather than the shorter live-playback deadline.
pub fn render_range_cancellable(
    project: &Project,
    sample_rate: u32,
    start_beat: f64,
    end_beat: f64,
    options: RenderOptions,
    cancel: &AtomicBool,
) -> Result<Vec<f32>> {
    ensure!(
        start_beat.is_finite()
            && end_beat.is_finite()
            && start_beat >= 0.0
            && end_beat > start_beat,
        "Render range must have finite, increasing, nonnegative bounds"
    );
    ensure!(
        project.tempo.is_finite() && project.tempo > 0.0,
        "Tempo must be positive and finite"
    );
    ensure!(
        (8_000..=384_000).contains(&sample_rate),
        "Sample rate must be between 8000 and 384000 Hz"
    );
    let samples_per_beat = sample_rate as f64 * 60.0 / project.tempo;
    let start = (start_beat * samples_per_beat).round();
    let end = (end_beat * samples_per_beat).round();
    ensure!(
        end <= sample_rate as f64 * 60.0 * 60.0 * 4.0,
        "Render range exceeds the four-hour processing limit"
    );
    ensure!(end > start, "Render range contains no samples");
    let check_cancelled = || -> Result<()> {
        ensure!(!cancel.load(Ordering::Acquire), "Audio analysis cancelled");
        Ok(())
    };
    check_cancelled()?;
    let renderer =
        Renderer::new_offline_with_options_cancelled(project, sample_rate, options, cancel)?;
    render_range_prepared(renderer, start as u64, end as u64, check_cancelled)
}

fn render_range_prepared(
    mut renderer: Renderer,
    start: u64,
    end: u64,
    mut check_cancelled: impl FnMut() -> Result<()>,
) -> Result<Vec<f32>> {
    let samples = usize::try_from(end - start)?
        .checked_mul(2)
        .context("Render range is too large")?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(samples)
        .context("Allocating range render output")?;
    let mut block = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    let mut rendered = 0;
    while rendered < end {
        check_cancelled()?;
        let count = (end - rendered).min(BLOCK as u64) as usize;
        renderer.render(&mut block[..count], &mut positions[..count])?;
        let first = start.saturating_sub(rendered).min(count as u64) as usize;
        for frame in &block[first..count] {
            ensure!(
                frame[0].is_finite() && frame[1].is_finite(),
                "Rendered audio contains nonfinite samples"
            );
            output.extend_from_slice(frame);
        }
        rendered += count as u64;
    }
    check_cancelled()?;
    Ok(output)
}

fn render_prepared(mut renderer: Renderer, length_beats: f64) -> Result<Vec<f32>> {
    ensure!(
        length_beats.is_finite() && length_beats > 0.0,
        "Render length must be positive and finite"
    );
    let sample_rate = renderer.sample_rate;
    let frame_count = (length_beats * 60.0 / renderer.tempo * sample_rate as f64).ceil();
    ensure!(
        frame_count <= sample_rate as f64 * 60.0 * 60.0 * 4.0,
        "Render exceeds the four-hour in-memory limit; export shorter sections"
    );
    let mut output = Vec::new();
    output
        .try_reserve_exact(frame_count as usize * 2)
        .context("Allocating render output")?;
    let mut block = [[0.0; 2]; BLOCK];
    let mut positions = [0.0; BLOCK];
    let mut remaining = frame_count as usize;
    while remaining > 0 {
        let count = remaining.min(BLOCK);
        renderer.render(&mut block[..count], &mut positions[..count])?;
        for frame in &block[..count] {
            output.extend_from_slice(frame);
        }
        remaining -= count;
    }
    Ok(output)
}

/// Stream a 32-bit float WAV to disk, preserving headroom without allocating the full mix.
pub fn export_wav(
    project: &Project,
    path: impl AsRef<Path>,
    sample_rate: u32,
    length_beats: f64,
) -> Result<()> {
    export_wav_with_options(
        project,
        path,
        sample_rate,
        length_beats,
        RenderOptions::default(),
    )
}

/// Stream a bounded-length float WAV with explicit source-conversion quality.
pub fn export_wav_with_options(
    project: &Project,
    path: impl AsRef<Path>,
    sample_rate: u32,
    length_beats: f64,
    options: RenderOptions,
) -> Result<()> {
    project.validate_audio_destination(path.as_ref())?;
    ensure!(
        length_beats.is_finite() && length_beats > 0.0,
        "Render length must be positive and finite"
    );
    let renderer = Renderer::new_offline_with_options(project, sample_rate, options)?;
    let frames = (length_beats * 60.0 / project.tempo * sample_rate as f64).ceil();
    if frames * 8.0 >= u32::MAX as f64 - 128.0 {
        bail!("Export exceeds the WAV 4 GiB size limit");
    }
    write_wav(renderer, path.as_ref(), frames as u64)
}

/// Export the complete arrangement and its finite release/effect tails (capped at 30 seconds).
/// The destination is replaced only after rendering and WAV finalization both succeed.
pub fn export_project_wav(
    project: &Project,
    path: impl AsRef<Path>,
    sample_rate: u32,
) -> Result<()> {
    export_project_wav_with_options(project, path, sample_rate, RenderOptions::default())
}

/// WAV delivery encoding. Floating point retains mix headroom; PCM is bounded
/// to nominal full scale and reports samples outside that range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WavEncoding {
    Pcm16,
    Pcm24,
    #[default]
    Float32,
}

impl WavEncoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pcm16 => "16-bit PCM",
            Self::Pcm24 => "24-bit PCM",
            Self::Float32 => "32-bit float",
        }
    }

    pub fn bits_per_sample(self) -> u16 {
        match self {
            Self::Pcm16 => 16,
            Self::Pcm24 => 24,
            Self::Float32 => 32,
        }
    }

    pub fn is_pcm(self) -> bool {
        self != Self::Float32
    }
}

/// An exact, end-exclusive musical range. Rendering still starts at song zero
/// so the selected section retains preceding instrument and effect history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportRange {
    pub start_beat: f64,
    pub end_beat: f64,
}

/// Project mixdown choices, independent of realtime playback preparation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportOptions {
    pub render: RenderOptions,
    /// Include instrument releases and effect tails after the last clip.
    /// When false, stop at the arrangement end rounded to the nearest sample.
    /// An explicit range always has exact length, irrespective of this choice.
    pub include_effect_tails: bool,
    /// Sample depth for WAV and FLAC; lossy formats ignore it.
    pub encoding: WavEncoding,
    pub format: ExportFormat,
    /// Add decorrelated triangular dither when reducing to integer samples
    /// (PCM WAV and FLAC). Float and lossy exports never apply dither.
    pub dither: bool,
    pub range: Option<ExportRange>,
}

impl ExportOptions {
    fn delivery(&self) -> Delivery {
        Delivery {
            format: self.format,
            encoding: self.encoding,
            dither: self.dither,
        }
    }
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            render: RenderOptions::default(),
            include_effect_tails: true,
            encoding: WavEncoding::Float32,
            format: ExportFormat::Wav,
            dither: true,
            range: None,
        }
    }
}

impl From<RenderOptions> for ExportOptions {
    fn from(render: RenderOptions) -> Self {
        Self {
            render,
            ..Self::default()
        }
    }
}

/// Export the arrangement with explicit conversion quality and optional effect tails.
/// Passing `RenderOptions` alone preserves the default of including finite tails.
/// Delay compensation is primed and removed in both modes; looping is ignored.
pub fn export_project_wav_with_options(
    project: &Project,
    path: impl AsRef<Path>,
    sample_rate: u32,
    options: impl Into<ExportOptions>,
) -> Result<()> {
    export_project_wav_with_report(project, path, sample_rate, options).map(|_| ())
}

/// Measurements describe rendered source samples in the selected output range,
/// before encoding/dither, not preroll or a true-peak/loudness assessment.
/// Nominal +/-1 endpoints are not overloads; `clipped_samples` counts source
/// samples outside [-1, 1], before dither, for every format that limits them
/// (PCM WAV, FLAC, Ogg Vorbis, MP3). Float WAV reports none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportReport {
    pub sample_peak: f32,
    pub clipped_samples: u64,
    pub frames: u64,
    pub sample_rate: u32,
    pub encoding: WavEncoding,
    pub format: ExportFormat,
    pub dither: bool,
    pub unsupported_midi_events: u64,
    pub unsupported_midi_groups: u64,
}

/// Stream a delivery WAV and report actual source peak and integer overloads.
/// Explicit ranges are sample-rounded, end-exclusive and have no appended tail.
pub fn export_project_wav_with_report(
    project: &Project,
    path: impl AsRef<Path>,
    sample_rate: u32,
    options: impl Into<ExportOptions>,
) -> Result<ExportReport> {
    project.validate_audio_destination(path.as_ref())?;
    let options = options.into();
    options.format.validate(options.encoding, sample_rate)?;
    let (start, end, renderer) = prepare_export(project, sample_rate, &options, None)?;
    write_wav_encoded(
        renderer,
        path.as_ref(),
        start,
        end,
        options.delivery(),
        None,
    )
}

/// As [`export_project_wav_with_report`], cancellable between processing blocks.
/// A cancelled or failed export leaves any previous file untouched.
pub fn export_project_cancellable(
    project: &Project,
    path: impl AsRef<Path>,
    sample_rate: u32,
    options: impl Into<ExportOptions>,
    cancel: &AtomicBool,
) -> Result<ExportReport> {
    project.validate_audio_destination(path.as_ref())?;
    let options = options.into();
    options.format.validate(options.encoding, sample_rate)?;
    let (start, end, renderer) = prepare_export(project, sample_rate, &options, Some(cancel))?;
    write_wav_encoded(
        renderer,
        path.as_ref(),
        start,
        end,
        options.delivery(),
        Some(cancel),
    )
}

/// The renderer and the sample range every export of `options` covers.
fn prepare_export(
    project: &Project,
    sample_rate: u32,
    options: &ExportOptions,
    cancel: Option<&AtomicBool>,
) -> Result<(u64, u64, Renderer)> {
    let range = options
        .range
        .map(|range| export_range_samples(project, sample_rate, range))
        .transpose()?;
    let renderer = match cancel {
        Some(cancel) => Renderer::new_offline_with_options_cancelled(
            project,
            sample_rate,
            options.render,
            cancel,
        )?,
        None => Renderer::new_offline_with_options(project, sample_rate, options.render)?,
    };
    let (start, end) =
        range.unwrap_or_else(|| export_extent(project, sample_rate, options, &renderer));
    Ok((start, end, renderer))
}

fn export_extent(
    project: &Project,
    sample_rate: u32,
    options: &ExportOptions,
    renderer: &Renderer,
) -> (u64, u64) {
    if options.include_effect_tails {
        (0, renderer.end_sample)
    } else {
        // Use the same beat-to-sample conversion as prepared clip boundaries.
        let samples_per_beat = sample_rate as f64 * 60.0 / project.tempo;
        (0, (project.end_beat() * samples_per_beat).round() as u64)
    }
}

fn export_range_samples(
    project: &Project,
    sample_rate: u32,
    range: ExportRange,
) -> Result<(u64, u64)> {
    ensure!(
        range.start_beat.is_finite()
            && range.end_beat.is_finite()
            && range.start_beat >= 0.0
            && range.end_beat > range.start_beat,
        "Export range must have finite, increasing, nonnegative bounds"
    );
    ensure!(
        project.tempo.is_finite() && project.tempo > 0.0,
        "Tempo must be positive and finite"
    );
    ensure!(
        (8_000..=384_000).contains(&sample_rate),
        "Sample rate must be between 8000 and 384000 Hz"
    );
    let samples_per_beat = sample_rate as f64 * 60.0 / project.tempo;
    let start = (range.start_beat * samples_per_beat).round();
    let end = (range.end_beat * samples_per_beat).round();
    ensure!(
        end <= sample_rate as f64 * 60.0 * 60.0 * 4.0,
        "Export range exceeds the four-hour processing limit"
    );
    ensure!(end > start, "Export range contains no samples");
    Ok((start as u64, end as u64))
}

pub(crate) fn write_wav(renderer: Renderer, path: &Path, frames: u64) -> Result<()> {
    write_wav_inner(renderer, path, frames, None)
}

pub(crate) fn write_wav_cancelled(
    renderer: Renderer,
    path: &Path,
    frames: u64,
    cancel: &AtomicBool,
) -> Result<()> {
    write_wav_inner(renderer, path, frames, Some(cancel))
}

fn write_wav_inner(
    renderer: Renderer,
    path: &Path,
    frames: u64,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    let delivery = Delivery {
        format: ExportFormat::Wav,
        encoding: WavEncoding::Float32,
        dither: false,
    };
    write_wav_encoded(renderer, path, 0, frames, delivery, cancel).map(|_| ())
}

/// Each sample's noise is keyed to its absolute stereo position. Channels are
/// decorrelated, repeat exports are reproducible, and range dither is the same
/// as the corresponding section of a full-file export.
fn triangular_dither(sample: u64) -> f64 {
    let uniform = |index: u64| {
        let mut value = index.wrapping_add(0x9e3779b97f4a7c15);
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((value ^ (value >> 31)) >> 11) as f64 / ((1_u64 << 53) as f64)
    };
    uniform(sample.wrapping_mul(2)) - uniform(sample.wrapping_mul(2).wrapping_add(1))
}

fn encode_pcm(sample: f32, bits: u16, noise: f64) -> i32 {
    let scale = (1_i64 << (bits - 1)) as f64;
    let value = f64::from(sample).clamp(-1.0, 1.0) * scale + noise;
    value.round().clamp(-scale, scale - 1.0) as i32
}

/// Render `start..end` into a hidden temporary file beside `path` and replace
/// `path` only after the file is finalized and synced.
fn write_wav_encoded(
    renderer: Renderer,
    path: &Path,
    start: u64,
    end: u64,
    delivery: Delivery,
    cancel: Option<&AtomicBool>,
) -> Result<ExportReport> {
    let (temp, report) = render_to_temp(renderer, path, start, end, delivery, cancel)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(report)
}

/// Stage a finished, synced delivery file next to `path`; the caller renames
/// it into place or removes it. Any failure removes the staged file.
fn render_to_temp(
    mut renderer: Renderer,
    path: &Path,
    start: u64,
    end: u64,
    delivery: Delivery,
    cancel: Option<&AtomicBool>,
) -> Result<(PathBuf, ExportReport)> {
    ensure!(end >= start, "Export range ends before its start");
    let frames = end - start;
    if delivery.format == ExportFormat::Wav {
        let bytes_per_frame = u64::from(delivery.encoding.bits_per_sample() / 8) * 2;
        ensure!(
            frames
                .checked_mul(bytes_per_frame)
                .is_some_and(|bytes| bytes < u32::MAX as u64 - 128),
            "Export exceeds the WAV 4 GiB size limit"
        );
    }
    let check_cancelled = || -> Result<()> {
        ensure!(
            !cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)),
            "Audio export cancelled"
        );
        Ok(())
    };
    check_cancelled()?;
    let filename = path
        .file_name()
        .context("Export path needs a file name")?
        .to_string_lossy();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = path.with_file_name(format!(".{filename}.{}.{nonce}.tmp", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let integer = delivery.format.is_integer(delivery.encoding);
    let clips = delivery.format.clips(delivery.encoding);
    let result = (|| -> Result<ExportReport> {
        let sync_file = file.try_clone()?;
        let mut encoder = Encoder::create(file, delivery, renderer.sample_rate)?;
        let mut report = ExportReport {
            sample_peak: 0.0,
            clipped_samples: 0,
            frames,
            sample_rate: renderer.sample_rate,
            encoding: delivery.encoding,
            format: delivery.format,
            dither: delivery.dither && integer,
            unsupported_midi_events: 0,
            unsupported_midi_groups: 0,
        };
        let mut block = [[0.0; 2]; BLOCK];
        let mut positions = [0.0; BLOCK];
        let mut rendered = 0;
        while rendered < end {
            check_cancelled()?;
            let count = (end - rendered).min(BLOCK as u64) as usize;
            renderer.render(&mut block[..count], &mut positions[..count])?;
            let first = start.saturating_sub(rendered).min(count as u64) as usize;
            for frame in &block[first..count] {
                ensure!(
                    frame[0].is_finite() && frame[1].is_finite(),
                    "Mix produced non-finite samples; reduce track or insert gain"
                );
                for sample in frame {
                    report.sample_peak = report.sample_peak.max(sample.abs());
                    report.clipped_samples += u64::from(clips && !(-1.0..=1.0).contains(sample));
                }
            }
            encoder.write(&block[first..count], rendered + first as u64, delivery)?;
            rendered += count as u64;
        }
        check_cancelled()?;
        encoder.finish()?;
        sync_file.sync_all()?;
        check_cancelled()?;
        (
            report.unsupported_midi_events,
            report.unsupported_midi_groups,
        ) = renderer.unsupported_midi();
        Ok(report)
    })();
    match result {
        Ok(report) => Ok((temp, report)),
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_delivery_does_not_replace_a_previous_file_or_leave_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("previous.wav");
        let original = b"previous successful delivery";
        std::fs::write(&output, original).unwrap();
        for encoding in [WavEncoding::Float32, WavEncoding::Pcm16, WavEncoding::Pcm24] {
            let renderer = Renderer::new_offline_with_options(
                &Project::new(),
                8_000,
                RenderOptions::default(),
            )
            .unwrap();
            let cancel = AtomicBool::new(true);
            let delivery = Delivery {
                format: ExportFormat::Wav,
                encoding,
                dither: true,
            };
            let error =
                write_wav_encoded(renderer, &output, 8_000, 16_000, delivery, Some(&cancel))
                    .unwrap_err();
            assert!(error.to_string().contains("cancelled"));
            assert_eq!(std::fs::read(&output).unwrap(), original);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn live_midi_merge_preserves_equal_offset_edges_without_allocating() {
        for left_len in 0..32 {
            for right_len in 0..32 {
                let mut messages: Vec<_> = (0..left_len + right_len)
                    .map(|index| MidiMessage {
                        offset: ((index * 17 + index / 3) % 11) as u32,
                        pitch: index as u8,
                        velocity: 100,
                        on: index % 2 == 0,
                    })
                    .collect();
                messages[..left_len].sort_by_key(|message| message.offset);
                messages[left_len..].sort_by_key(|message| message.offset);
                let mut expected = messages.clone();
                expected.sort_by_key(|message| message.offset);
                let allocations = crate::allocation_test_support::count_allocations(|| {
                    merge_midi_events(&mut messages, left_len)
                });
                assert_eq!(allocations, 0);
                let actual: Vec<_> = messages
                    .iter()
                    .map(|event| (event.offset, event.pitch, event.on))
                    .collect();
                let expected: Vec<_> = expected
                    .iter()
                    .map(|event| (event.offset, event.pitch, event.on))
                    .collect();
                assert_eq!(actual, expected);
            }
        }
    }

    use daw_core::{
        AutomationPoint, AutomationTarget, Clip, Interpolation, Note, Track, TrackKind,
    };

    fn loop_project() -> (Project, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tone.wav");
        AudioData {
            samples: vec![0.25; 4_000],
            sample_rate: 8_000,
            channels: 1,
        }
        .write_wav(&path)
        .unwrap();
        let mut project = Project::new();
        project.loop_enabled = true;
        project.loop_start = 0.0;
        project.loop_end = 0.125;
        let mut track = Track::new("Test", TrackKind::Audio);
        track
            .clips
            .push(Clip::audio("Tone", 0.0, 1.0, path.to_string_lossy()));
        project.tracks.push(track);
        (project, directory)
    }

    #[test]
    fn range_render_matches_full_render_and_retains_only_selected_samples() {
        let (project, _directory) = loop_project();
        let full = render(&project, 8_000, 1.0).unwrap();
        let range = render_range_cancellable(
            &project,
            8_000,
            0.125,
            0.625,
            RenderOptions::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        // At 120 BPM and 8 kHz, one beat is exactly 4,000 frames.
        assert_eq!(range, full[1_000..5_000]);
    }

    #[test]
    fn range_render_checks_cancellation_between_processing_blocks() {
        let (project, _directory) = loop_project();
        let renderer = Renderer::new(&project, 8_000, false).unwrap();
        let mut checks = 0;
        let result = render_range_prepared(renderer, 0, 4_000, || {
            checks += 1;
            ensure!(checks < 3, "cancelled by test after two blocks");
            Ok(())
        });
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cancelled by test")
        );
        assert_eq!(checks, 3);
        assert!(
            render_range_cancellable(
                &project,
                8_000,
                0.0,
                1.0,
                RenderOptions::default(),
                &AtomicBool::new(true),
            )
            .is_err()
        );
    }

    #[test]
    fn prepared_audio_render_including_loop_wrap_allocates_nothing() {
        let (project, _directory) = loop_project();
        let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
        let mut output = [[0.0; 2]; 512];
        let mut positions = [0.0; 512];
        let allocations = crate::allocation_test_support::count_allocations(|| {
            renderer.render(&mut output, &mut positions).unwrap()
        });
        assert_eq!(allocations, 0);
        assert!(positions[499] >= 0.125);
        assert!(positions[500] < 0.001);
        assert!(!renderer.finished());
    }

    #[test]
    fn seeking_audio_uses_selected_sample_rate() {
        let (mut project, _directory) = loop_project();
        project.loop_enabled = false;
        let mut renderer = Renderer::new(&project, 44_100, true).unwrap();
        renderer.seek(0.5).unwrap();
        let mut output = [[0.0; 2]; BLOCK];
        let mut positions = [0.0; BLOCK];
        renderer.render(&mut output, &mut positions).unwrap();
        assert!(output.iter().any(|frame| frame[0].abs() > 0.01));
        assert!((positions[0] - (0.5 + 2.0 / 44_100.0)).abs() < 1e-12);
    }

    #[test]
    fn prepared_bus_graph_allocates_nothing() {
        let (mut project, _directory) = loop_project();
        let mut bus = Track::new("Return", TrackKind::Bus);
        bus.latency_samples = 37;
        project.tracks[0].sends.push(daw_core::Send {
            target_bus: bus.id,
            gain: 0.5,
            pre_fader: true,
        });
        project.tracks.push(bus);
        let mut renderer = Renderer::new(&project, 8_000, true).unwrap();
        let mut output = [[0.0; 2]; 512];
        let mut positions = [0.0; 512];
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| renderer
                .render(&mut output, &mut positions)
                .unwrap()),
            0
        );
    }

    #[test]
    fn automation_follows_seek_and_loop_positions_without_allocating() {
        let (mut project, _directory) = loop_project();
        project.tracks[0].gain = 1.0;
        let baseline = project.clone();
        let target = project.tracks[0].id;
        let parameter = AutomationParameter::TrackGain;
        let lane = project
            .add_automation_track(
                "Level",
                AutomationTarget {
                    track: target,
                    parameter,
                },
            )
            .unwrap();
        let lane = project.automation_lane_mut(lane).unwrap();
        lane.points = vec![
            AutomationPoint::new(0.0, parameter.normalize(0.25)),
            AutomationPoint::new(0.125, parameter.normalize(1.0)),
        ];
        lane.interpolation = Interpolation::Linear;
        let mut automated = Renderer::new(&project, 8_000, true).unwrap();
        let mut reference = Renderer::new(&baseline, 8_000, true).unwrap();
        for seek in [0.0, 0.05] {
            automated.seek(seek).unwrap();
            reference.seek(seek).unwrap();
            let mut output = [[0.0; 2]; 1024];
            let mut dry = [[0.0; 2]; 1024];
            let mut positions = [0.0; 1024];
            let mut dry_positions = [0.0; 1024];
            assert_eq!(
                crate::allocation_test_support::count_allocations(|| {
                    automated.render(&mut output, &mut positions).unwrap();
                }),
                0
            );
            reference.render(&mut dry, &mut dry_positions).unwrap();
            assert_eq!(positions, dry_positions);
            for index in 0..output.len() {
                let beat = positions[index] - 1.0 / 4_000.0;
                let gain = 0.25 + (beat / 0.125).clamp(0.0, 1.0) as f32 * 0.75;
                assert!(
                    (output[index][0] - dry[index][0] * gain).abs() < 1e-6,
                    "Frame {index} after seek {seek}"
                );
            }
        }
    }

    #[test]
    fn midi_source_offset_crops_notes_before_plugin_scheduling() {
        let mut project = Project::new();
        let mut track = Track::new("MIDI", TrackKind::Midi);
        let mut clip = Clip::midi(
            "Notes",
            0.0,
            1.0,
            vec![Note::new(60, 0.0, 0.5, 100), Note::new(64, 1.25, 0.25, 90)],
        );
        clip.offset = 1.0;
        track.clips.push(clip);
        project.tracks.push(track);
        let renderer = Renderer::new(&project, 8_000, false).unwrap();
        assert_eq!(renderer.tracks[0].as_ref().unwrap().notes.len(), 1);
        let note = renderer.tracks[0].as_ref().unwrap().notes[0];
        assert_eq!((note.pitch, note.start, note.end), (64, 1_000, 2_000));
    }
}

#[cfg(test)]
#[path = "parallel_tests.rs"]
mod parallel_tests;

#[cfg(test)]
#[path = "audition_tests.rs"]
mod audition_tests;

#[cfg(test)]
#[path = "meter_tests.rs"]
mod meter_tests;
