use crate::meter::{ChannelMeter, ChannelMeters};
use crate::quality::{self, RenderOptions};
use anyhow::{Context, Result, bail, ensure};
use daw_core::{
    AudioData, AutomationLane, AutomationParameter, ClipContent, Effect, FadeCurve, Instrument,
    Project, TrackKind,
};
use daw_plugins::{MidiMessage, VstEffect, VstInstrument};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

#[path = "worker_pool.rs"]
mod worker_pool;
use worker_pool::WorkerPool;

#[path = "audio_schedule.rs"]
mod audio_schedule;

#[path = "live_edit.rs"]
pub(crate) mod live_edit;
use live_edit::LiveEditContext;

#[cfg(test)]
#[path = "live_edit_tests.rs"]
mod live_edit_tests;

#[cfg(test)]
#[path = "stretch_render_tests.rs"]
mod stretch_render_tests;

#[cfg(test)]
#[path = "effect_state_tests.rs"]
mod effect_state_tests;

#[cfg(test)]
#[path = "quality_tests.rs"]
mod quality_tests;

#[cfg(test)]
#[path = "audio_overlap_tests.rs"]
mod audio_overlap_tests;

pub(crate) const BLOCK: usize = 256;

#[derive(Clone, Copy)]
struct ScheduledNote {
    clip: u64,
    id: u64,
    start: u64,
    end: u64,
    pitch: u8,
    velocity: u8,
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
    original: Arc<AudioData>,
    start: u64,
    source_offset: f64,
    gain: f32,
    fade_in: f64,
    fade_out: f64,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
    envelope_offset: f64,
    envelope_length: f64,
    source_length: f64,
    reverse: bool,
    data: Arc<AudioData>,
}

impl AudioClip {
    #[inline]
    fn sample(&self, position: u64, sample_rate: u32) -> [f32; 2] {
        let elapsed = position - self.start;
        let source_delta = elapsed as f64 * self.data.sample_rate as f64 / sample_rate as f64;
        let source = self.source_offset
            + if self.reverse {
                self.source_length - 1.0 - source_delta
            } else {
                source_delta
            };
        if source < -1e-8 {
            return [0.0; 2];
        }
        let source = source.max(0.0);
        let frame = source.floor() as usize;
        let frames = self.data.samples.len() / self.data.channels;
        if frame >= frames {
            return [0.0; 2];
        }
        let fraction = (source - frame as f64) as f32;
        let next = (frame + 1).min(frames - 1);
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
        let gain = self.gain * fade_in * fade_out;
        let channel = |channel: usize| {
            let channel = channel.min(self.data.channels - 1);
            let a = self.data.samples[frame * self.data.channels + channel];
            let b = self.data.samples[next * self.data.channels + channel];
            (a + (b - a) * fraction) * gain
        };
        [channel(0), channel(1)]
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
}

/// Compiled lane ownership keeps graph lookup and allocation out of rendering.
struct AutomatedValue {
    manual: f32,
    smooth_value: f32,
    smooth_remaining: u32,
    lane: Option<AutomationLane>,
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
        let lane = project.tracks.iter().find_map(|candidate| {
            let lane = candidate.automation.as_ref()?;
            (lane.target.track == track
                && lane.target.parameter == parameter
                && !lane.points.is_empty()
                && lane_enabled(project, candidate))
            .then(|| lane.clone())
        });
        Self {
            manual,
            lane,
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

    #[inline]
    fn next_value(&mut self, beat: f64) -> f32 {
        if self.smooth_remaining > 0 {
            self.smooth_value += (self.manual - self.smooth_value) / self.smooth_remaining as f32;
            self.smooth_remaining -= 1;
        }
        self.lane
            .as_ref()
            .and_then(|lane| {
                lane.value_at(beat)
                    .map(|value| lane.target.parameter.denormalize(value))
            })
            .unwrap_or(self.smooth_value)
    }

    #[inline]
    fn value(&self, beat: f64) -> f32 {
        self.lane
            .as_ref()
            .and_then(|lane| {
                lane.value_at(beat)
                    .map(|value| lane.target.parameter.denormalize(value))
            })
            .unwrap_or(self.manual)
    }
}

struct PluginAutomation {
    id: u32,
    lane: AutomationLane,
    previous: Option<f32>,
}

fn plugin_automation(project: &Project, track: u64, effect: Option<u64>) -> Vec<PluginAutomation> {
    project
        .tracks
        .iter()
        .filter(|candidate| lane_enabled(project, candidate))
        .filter_map(|candidate| {
            let lane = candidate.automation.as_ref()?;
            let AutomationParameter::VstParameter { effect: target, id } = lane.target.parameter
            else {
                return None;
            };
            (lane.target.track == track && target == effect && !lane.points.is_empty()).then(|| {
                PluginAutomation {
                    id,
                    lane: lane.clone(),
                    previous: None,
                }
            })
        })
        .collect()
}

fn apply_plugin_automation(
    parameters: &mut [PluginAutomation],
    beat: f64,
    mut set: impl FnMut(u32, f64) -> Result<()>,
) -> Result<()> {
    for parameter in parameters {
        if let Some(value) = parameter.lane.value_at(beat)
            && parameter.previous != Some(value)
        {
            set(parameter.id, f64::from(value))?;
            parameter.previous = Some(value);
        }
    }
    Ok(())
}

enum Insert {
    Vst {
        id: u64,
        effect: Box<VstEffect>,
        parameters: Vec<PluginAutomation>,
    },
}

struct Route {
    target: Option<usize>,
    gain: AutomatedValue,
    pre_fader: bool,
    delay: CompensationDelay,
    output: Vec<[f32; 2]>,
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
    notes: Arc<Vec<ScheduledNote>>,
    events: Arc<Vec<Event>>,
    note_order: Arc<Vec<usize>>,
    event_cursor: usize,
    processor: Processor,
    midi: Vec<MidiMessage>,
    chase: Vec<Event>,
    active_notes: Vec<ScheduledNote>,
    active_indices: HashMap<NoteKey, usize>,
    live_midi: Vec<MidiMessage>,
    left: Vec<f32>,
    right: Vec<f32>,
    mix_left: Vec<f64>,
    mix_right: Vec<f64>,
    gain: AutomatedValue,
    pan: AutomatedValue,
    instrument_automation: Vec<PluginAutomation>,
    delay: CompensationDelay,
    latency: u32,
    input_latency: u32,
    source_latency: u32,
    tail_samples: u64,
    plugin_failed: bool,
    enabled: bool,
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
    meter_decay: f32,
}

fn finish_processing(track: &mut RenderTrack, result: Result<()>, tolerate: bool) -> Result<u64> {
    if let Err(error) = result {
        if !tolerate {
            return Err(error);
        }
        track.plugin_failed = true;
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
        let live_count = self.live_midi.len();
        self.midi.append(&mut self.live_midi);
        if context.timeline {
            for event in self.chase.drain(..) {
                self.midi.push(event.message(0));
                record_active_note(&mut self.active_notes, &mut self.active_indices, event);
            }
        }
        while context.timeline
            && let Some(event) = self.events.get(self.event_cursor)
        {
            if event.sample >= end {
                break;
            }
            self.midi
                .push(event.message(event.sample.saturating_sub(start) as u32));
            record_active_note(&mut self.active_notes, &mut self.active_indices, *event);
            self.event_cursor += 1;
        }
        // Input events can land between sequencer events within this block.
        // Stable ordering preserves a retrigger's note-off/note-on edge order.
        merge_midi_events(&mut self.midi, live_count);
    }

    fn process(&mut self, context: BlockContext) -> Result<()> {
        if !self.enabled || self.plugin_failed {
            return Ok(());
        }
        #[cfg(test)]
        if let Some(observer) = &self.observer {
            observer(self.id);
        }
        let BlockContext { start, count, .. } = context;
        let end = start + count as u64;
        let beats_per_sample = context.tempo / (60.0 * context.sample_rate as f64);
        if matches!(self.processor, Processor::Vst(_)) {
            self.collect_midi(context);
        }
        match &mut self.processor {
            Processor::None => {}
            Processor::Vst(plugin) => {
                let process = apply_plugin_automation(
                    &mut self.instrument_automation,
                    start as f64 * beats_per_sample,
                    |id, value| plugin.set_parameter(id, value),
                )
                .and_then(|()| {
                    plugin.set_transport(
                        start as i64,
                        start as f64 * beats_per_sample,
                        context.tempo,
                        context.timeline,
                    )
                })
                .and_then(|()| {
                    plugin.process(
                        &self.midi,
                        &mut self.left[..count],
                        &mut self.right[..count],
                    )
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
                if let (Some(outgoing), Some(curve)) = (segment.outgoing, &segment.fade) {
                    let outgoing = &self.audio[outgoing];
                    let duration = (segment.end - segment.start) as f64;
                    let mut curve_cursor =
                        curve.cursor((start + first as u64 - segment.start) as f64 / duration);
                    for index in first..last {
                        let position = start + index as u64;
                        let gain = curve.value(
                            (position - segment.start) as f64 / duration,
                            &mut curve_cursor,
                        );
                        let a = outgoing.sample(position, context.sample_rate);
                        let b = incoming.sample(position, context.sample_rate);
                        self.mix_left[index] += f64::from(a[0]) * f64::from(1.0 - gain)
                            + f64::from(b[0]) * f64::from(gain);
                        self.mix_right[index] += f64::from(a[1]) * f64::from(1.0 - gain)
                            + f64::from(b[1]) * f64::from(gain);
                    }
                } else {
                    for index in first..last {
                        let sample = incoming.sample(start + index as u64, context.sample_rate);
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
        for index in 0..count {
            self.left[index] = self.mix_left[index] as f32;
            self.right[index] = self.mix_right[index] as f32;
        }
        let mut effect_latency = self.input_latency + self.source_latency;
        for insert in &mut self.inserts {
            match insert {
                Insert::Vst {
                    effect, parameters, ..
                } => {
                    let position = if context.timeline {
                        start as i64 - effect_latency as i64
                    } else {
                        start as i64
                    };
                    let process = apply_plugin_automation(
                        parameters,
                        position as f64 * beats_per_sample,
                        |id, value| effect.set_parameter(id, value),
                    )
                    .and_then(|()| {
                        effect.set_transport(
                            position,
                            position as f64 * beats_per_sample,
                            context.tempo,
                            context.timeline,
                        )
                    })
                    .and_then(|()| {
                        effect.process(
                            &self.left[..count],
                            &self.right[..count],
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
                    effect_latency += effect.latency_samples();
                }
            }
        }
        let mut channel_peaks = [0.0_f32; 2];
        for index in 0..count {
            let delayed = self.delay.process([self.left[index], self.right[index]]);
            self.left[index] = delayed[0];
            self.right[index] = delayed[1];
            let position = if context.timeline {
                start as i64 + index as i64 - self.latency as i64
            } else {
                start as i64
            };
            let beat = position as f64 * beats_per_sample;
            let gain = self.gain.next_value(beat);
            let pan = self.pan.next_value(beat).clamp(-1.0, 1.0);
            // Insert scratch buffers are free after the chain. Reuse them for
            // fader factors so multiple sends evaluate the lane only once.
            self.insert_left[index] = gain * (1.0 - pan.max(0.0));
            self.insert_right[index] = gain * (1.0 + pan.min(0.0));
            if self.meter.is_some() {
                channel_peaks[0] =
                    channel_peaks[0].max((self.left[index] * self.insert_left[index]).abs());
                channel_peaks[1] =
                    channel_peaks[1].max((self.right[index] * self.insert_right[index]).abs());
            }
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
                route.output[index] = route.delay.process([
                    self.left[index] * factors[0],
                    self.right[index] * factors[1],
                ]);
            }
        }
        if let Some(meter) = &self.meter {
            meter.publish(channel_peaks, context.meter_decay);
        }
        Ok(())
    }
}

pub(crate) struct Renderer {
    tracks: Vec<Option<Box<RenderTrack>>>,
    levels: Vec<Vec<usize>>,
    workers: Option<WorkerPool>,
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
    midi_input_notes: Vec<(u64, u8, u8)>,
    metering: bool,
    live_edit_context: Arc<LiveEditContext>,
}

impl Renderer {
    #[cfg(test)]
    pub(crate) fn new(project: &Project, sample_rate: u32, looping: bool) -> Result<Self> {
        Self::new_inner(
            project,
            sample_rate,
            looping,
            None,
            RenderOptions::default(),
            None,
        )
    }

    pub(crate) fn new_with_options(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        options: RenderOptions,
    ) -> Result<Self> {
        Self::new_inner(project, sample_rate, looping, None, options, None)
    }

    pub(crate) fn new_with_options_cancelled(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        options: RenderOptions,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        Self::new_inner(project, sample_rate, looping, None, options, Some(cancel))
    }

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
        )
    }

    fn new_inner(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        requested_workers: Option<usize>,
        options: RenderOptions,
        cancel: Option<&AtomicBool>,
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
        let order = project.routing_order()?;
        let audible = audible_tracks(project, &order);
        let indices: HashMap<u64, usize> = order
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect();
        let mut tracks = Vec::new();
        let mut end_sample = to_sample(project.end_beat());
        let mut audio_cache = HashMap::<live_edit::AudioAssetKey, Arc<AudioData>>::new();
        let mut cache_bytes = 0usize;
        let mut tail_samples = sample_rate as u64 / 5;
        for id in &order {
            check_cancelled()?;
            let track = project
                .track(*id)
                .context("Routing refers to a missing track")?;
            let enabled = audible.contains(id);
            let mut local_tail = 0_u64;
            ensure!(
                track.latency_samples <= sample_rate * 10,
                "Track processing latency exceeds ten seconds"
            );
            for clip in &track.clips {
                if clip.is_take_source() {
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
            let (processor, plugin_latency) = match &track.instrument {
                Instrument::Vst3 { path, class_id } if enabled && track.kind == TrackKind::Midi => {
                    check_cancelled()?;
                    let mut plugin = VstInstrument::load_class(
                        Path::new(path),
                        class_id.as_deref(),
                        sample_rate as f64,
                        BLOCK,
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
                    check_cancelled()?;
                    let latency = plugin.latency_samples();
                    // VST3's infinite-tail sentinel is bounded to keep automatic stop useful.
                    local_tail += (plugin.tail_samples() as u64).min(sample_rate as u64 * 30);
                    (Processor::Vst(Box::new(plugin)), latency)
                }
                _ => (Processor::None, 0),
            };
            let control =
                |parameter, manual| AutomatedValue::new(project, track.id, parameter, manual);
            let event_capacity = content.events.len() + content.notes.len() + 4;
            let note_capacity = content.notes.len();
            let mut inserts = Vec::new();
            let mut latency = track.latency_samples.saturating_add(plugin_latency);
            for slot in &track.inserts {
                check_cancelled()?;
                if !enabled || !slot.enabled {
                    continue;
                }
                inserts.push(match &slot.effect {
                    Effect::Vst3 { path, class_id } => {
                        let mut effect = VstEffect::load_class(
                            Path::new(path),
                            class_id.as_deref(),
                            sample_rate as f64,
                            BLOCK,
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
                        check_cancelled()?;
                        latency = latency.saturating_add(effect.latency_samples());
                        local_tail += (effect.tail_samples() as u64).min(sample_rate as u64 * 30);
                        Insert::Vst {
                            id: slot.id,
                            effect: Box::new(effect),
                            parameters: plugin_automation(project, track.id, Some(slot.id)),
                        }
                    }
                });
            }
            let mut routes = Vec::new();
            if enabled {
                routes.push(Route {
                    target: track.output_bus.map(|target| indices[&target]),
                    gain: AutomatedValue {
                        manual: 1.0,
                        smooth_value: 1.0,
                        smooth_remaining: 0,
                        lane: None,
                    },
                    pre_fader: false,
                    delay: CompensationDelay::new(0),
                    output: vec![[0.0; 2]; BLOCK],
                });
                for send in &track.sends {
                    let gain = control(
                        AutomationParameter::SendGain {
                            bus: send.target_bus,
                        },
                        send.gain,
                    );
                    if send.gain > 0.0 || gain.lane.is_some() {
                        routes.push(Route {
                            target: Some(indices[&send.target_bus]),
                            gain,
                            pre_fader: send.pre_fader,
                            delay: CompensationDelay::new(0),
                            output: vec![[0.0; 2]; BLOCK],
                        });
                    }
                }
            }
            tracks.push(RenderTrack {
                id: track.id,
                name: track.name.clone(),
                pending_error: None,
                audio: content.audio,
                audio_segments: content.audio_segments,
                audio_transition: vec![[0.0; 2]; (sample_rate as usize / 200).max(2)],
                audio_transition_cursor: (sample_rate as usize / 200).max(2),
                notes: content.notes,
                events: content.events,
                note_order: content.note_order,
                event_cursor: 0,
                processor,
                midi: Vec::with_capacity(event_capacity),
                live_midi: Vec::with_capacity(4),
                chase: Vec::with_capacity(event_capacity),
                active_notes: Vec::with_capacity(note_capacity),
                active_indices: HashMap::with_capacity(note_capacity),
                left: vec![0.0; BLOCK],
                right: vec![0.0; BLOCK],
                mix_left: vec![0.0; BLOCK],
                mix_right: vec![0.0; BLOCK],
                gain: control(AutomationParameter::TrackGain, track.gain),
                pan: control(AutomationParameter::TrackPan, track.pan),
                instrument_automation: plugin_automation(project, track.id, None),
                delay: CompensationDelay::new(track.latency_samples as usize),
                latency,
                input_latency: 0,
                source_latency: plugin_latency,
                tail_samples: local_tail.min(sample_rate as u64 * 30),
                plugin_failed: false,
                enabled,
                inserts,
                insert_left: vec![0.0; BLOCK],
                insert_right: vec![0.0; BLOCK],
                routes,
                meter: None,
                #[cfg(test)]
                observer: None,
            });
        }
        let mut input_latency = vec![0_u32; tracks.len()];
        let mut input_tail = vec![0_u64; tracks.len()];
        let mut latency = 0;
        for index in 0..tracks.len() {
            if !tracks[index].enabled {
                continue;
            }
            tracks[index].latency = tracks[index].latency.saturating_add(input_latency[index]);
            tracks[index].input_latency = input_latency[index];
            ensure!(
                tracks[index].latency <= sample_rate * 10,
                "Processing path latency exceeds ten seconds"
            );
            tracks[index].tail_samples =
                (tracks[index].tail_samples + input_tail[index]).min(sample_rate as u64 * 30);
            for route in &tracks[index].routes {
                if let Some(target) = route.target {
                    input_latency[target] = input_latency[target].max(tracks[index].latency);
                    input_tail[target] = input_tail[target].max(tracks[index].tail_samples);
                } else {
                    latency = latency.max(tracks[index].latency);
                    tail_samples = tail_samples.max(tracks[index].tail_samples);
                }
            }
        }
        ensure!(
            latency <= sample_rate * 10,
            "Processing latency exceeds ten seconds"
        );
        for track in &mut tracks {
            for route in &mut track.routes {
                let target_latency = route
                    .target
                    .map(|target| input_latency[target])
                    .unwrap_or(latency);
                route.delay = CompensationDelay::new((target_latency - track.latency) as usize);
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
        let width = levels.iter().map(Vec::len).max().unwrap_or(0);
        let useful_parallelism = levels.iter().any(|level| {
            level.len() >= 32
                || level
                    .iter()
                    .filter(|&&index| {
                        matches!(tracks[index].processor, Processor::Vst(_))
                            || !tracks[index].inserts.is_empty()
                    })
                    .count()
                    >= 2
        });
        let available = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let worker_count = requested_workers
            .unwrap_or(if useful_parallelism { available } else { 1 })
            .min(available.max(2))
            .min(width);
        check_cancelled()?;
        let workers = if worker_count > 1 {
            Some(WorkerPool::new(worker_count, width)?)
        } else {
            None
        };
        let mut renderer = Self {
            tracks: tracks
                .into_iter()
                .map(|track| Some(Box::new(track)))
                .collect(),
            levels,
            workers,
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
            midi_input_notes: Vec::with_capacity(4096),
            metering: false,
            live_edit_context: Arc::new(LiveEditContext::new(
                project,
                sample_rate,
                options,
                tail_samples,
            )),
        };
        renderer.seek_sample(0)?;
        check_cancelled()?;
        Ok(renderer)
    }

    /// Realtime-only telemetry; offline renders do not calculate or publish meters.
    pub(crate) fn attach_channel_meters(&mut self, meters: &ChannelMeters) {
        for track in self.tracks.iter_mut().flatten() {
            track.meter = meters.channel(track.id).cloned();
        }
        self.metering = true;
    }

    pub(crate) fn set_plugin_parameter(
        &mut self,
        track_id: u64,
        effect_id: Option<u64>,
        parameter_id: u32,
        value: f64,
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
        let parameters = if let Some(effect_id) = effect_id {
            let Insert::Vst {
                effect, parameters, ..
            } = track
                .inserts
                .iter_mut()
                .find(|insert| matches!(insert, Insert::Vst { id, .. } if *id == effect_id))
                .context("Insert is not active in the render graph")?;
            effect.set_parameter(parameter_id, value).with_context(|| {
                format!(
                    "Setting parameter {parameter_id} of effect \"{}\" ({}) on channel \"{}\" (ID {})",
                    effect.info().name, effect.info().path.display(), track.name, track.id
                )
            })?;
            parameters
        } else if let Processor::Vst(plugin) = &mut track.processor {
            plugin.set_parameter(parameter_id, value).with_context(|| {
                format!(
                    "Setting parameter {parameter_id} of instrument \"{}\" ({}) on channel \"{}\" (ID {})",
                    plugin.info().name, plugin.info().path.display(), track.name, track.id
                )
            })?;
            &mut track.instrument_automation
        } else {
            bail!("Track has no assigned VST instrument")
        };
        for automation in parameters {
            if automation.id == parameter_id {
                automation.previous = None;
            }
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
        self.seek_sample(sample)
    }

    fn seek_sample(&mut self, sample: u64) -> Result<()> {
        self.release_end_sample = 0;
        self.end_sample = self.timeline_end_sample;
        self.audition = None;
        self.midi_input_notes.clear();
        self.source_sample = sample;
        self.logical_sample = sample;
        for track in self.tracks.iter_mut().flatten() {
            track.audio_transition_cursor = track.audio_transition.len();
            for parameter in &mut track.instrument_automation {
                parameter.previous = None;
            }
            track.delay.reset();
            for route in &mut track.routes {
                route.delay.reset();
            }
            for insert in &mut track.inserts {
                match insert {
                    Insert::Vst {
                        effect, parameters, ..
                    } if !track.plugin_failed => {
                        for parameter in parameters {
                            parameter.previous = None;
                        }
                        if let Err(error) = effect.reset() {
                            if !self.tolerate_plugin_failure {
                                return Err(error);
                            }
                            track.plugin_failed = true;
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
                    Insert::Vst { .. } => {}
                }
            }
            track.event_cursor = track.events.partition_point(|event| event.sample < sample);
            track.chase.clear();
            track.active_notes.clear();
            track.active_indices.clear();
            track.live_midi.clear();
            match &mut track.processor {
                Processor::None => {}
                Processor::Vst(plugin) if !track.plugin_failed => {
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
                Processor::Vst(_) => {}
            }
            for &note in track.notes.iter() {
                if !self.preview && note.start < sample && note.end > sample {
                    match &mut track.processor {
                        Processor::None => {}
                        Processor::Vst(_) => track.chase.push(Event {
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
            self.segment(&mut discard[..count])?;
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
                matches!(track.processor, Processor::Vst(_)),
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

    /// Normalize the complete held MIDI input chord on the render worker. A
    /// route or device disappearing releases every voice, and seek/reset clears
    /// the remembered set so currently held keys are restored on the next block.
    pub(crate) fn release_midi_input_note(&mut self, id: u64, pitch: u8, offset: u32) {
        if let Some(index) = self
            .midi_input_notes
            .iter()
            .position(|note| note.0 == id && note.1 == pitch)
        {
            self.midi_input_notes.remove(index);
            if let Some(track) = self
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
    }

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
                && matches!(track.processor, Processor::Vst(_))
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
        self.segment(output)?;
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
                self.seek_sample(start)?;
            }
            let remaining = self
                .loop_range
                .map(|(_, end)| end.saturating_sub(self.logical_sample) as usize)
                .unwrap_or(usize::MAX);
            let count = BLOCK.min(output.len() - offset).min(remaining);
            let first_sample = self.logical_sample;
            self.segment(&mut output[offset..offset + count])?;
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

    fn segment(&mut self, output: &mut [[f32; 2]]) -> Result<()> {
        output.fill([0.0; 2]);
        let mut master = [[0.0_f64; 2]; BLOCK];
        let context = BlockContext {
            start: self.source_sample,
            count: output.len(),
            sample_rate: self.sample_rate,
            tempo: self.tempo,
            timeline: !self.preview,
            meter_decay: if self.metering {
                // Instant attack, 24 dB/second release, independent of block size.
                (-std::f32::consts::LN_10 * 1.2 * output.len() as f32 / self.sample_rate as f32)
                    .exp()
            } else {
                0.0
            },
        };
        for track in self.tracks.iter_mut().flatten() {
            track.left[..context.count].fill(0.0);
            track.right[..context.count].fill(0.0);
            track.mix_left[..context.count].fill(0.0);
            track.mix_right[..context.count].fill(0.0);
        }
        for level_index in 0..self.levels.len() {
            let level = &self.levels[level_index];
            if let Some(workers) = &mut self.workers
                && level.len() > 1
            {
                self.plugin_errors += workers.process(
                    &mut self.tracks,
                    level,
                    context,
                    self.tolerate_plugin_failure,
                )?;
            } else {
                for &index in level {
                    let track = self.tracks[index].as_mut().expect("owned graph node");
                    let result = track.process(context);
                    self.plugin_errors +=
                        finish_processing(track, result, self.tolerate_plugin_failure)?;
                }
            }
            for position in 0..self.levels[level_index].len() {
                self.route_track(
                    self.levels[level_index][position],
                    &mut master[..context.count],
                );
            }
        }
        for (output, accumulated) in output.iter_mut().zip(master) {
            *output = [accumulated[0] as f32, accumulated[1] as f32];
        }
        if !self.preview {
            self.source_sample += context.count as u64;
        }
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
                for (index, frame) in route.output[..count].iter().enumerate() {
                    destination.mix_left[index] += f64::from(frame[0]);
                    destination.mix_right[index] += f64::from(frame[1]);
                }
            } else {
                for (destination, frame) in output.iter_mut().zip(&route.output) {
                    destination[0] += f64::from(frame[0]);
                    destination[1] += f64::from(frame[1]);
                }
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
        Renderer::new_with_options(project, sample_rate, false, options)?,
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
        Renderer::new_with_workers(project, sample_rate, false, workers)?,
        length_beats,
    )
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
    ensure!(
        length_beats.is_finite() && length_beats > 0.0,
        "Render length must be positive and finite"
    );
    let renderer = Renderer::new_with_options(project, sample_rate, false, options)?;
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

/// Project mixdown choices, independent of realtime playback preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportOptions {
    pub render: RenderOptions,
    /// Include instrument releases and effect tails after the last clip.
    /// When false, stop at the arrangement end rounded to the nearest sample.
    pub include_effect_tails: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            render: RenderOptions::default(),
            include_effect_tails: true,
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
    let options = options.into();
    let renderer = Renderer::new_with_options(project, sample_rate, false, options.render)?;
    let frames = if options.include_effect_tails {
        renderer.end_sample
    } else {
        // Use the same beat-to-sample conversion as prepared clip boundaries.
        let samples_per_beat = sample_rate as f64 * 60.0 / project.tempo;
        (project.end_beat() * samples_per_beat).round() as u64
    };
    ensure!(
        frames * 8 < u32::MAX as u64 - 128,
        "Export exceeds the WAV 4 GiB size limit"
    );
    write_wav(renderer, path.as_ref(), frames)
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
    mut renderer: Renderer,
    path: &Path,
    frames: u64,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    let check_cancelled = || -> Result<()> {
        ensure!(
            !cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)),
            "Audio export cancelled"
        );
        Ok(())
    };
    check_cancelled()?;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: renderer.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
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
    let result = (|| -> Result<()> {
        let mut writer = hound::WavWriter::new(std::io::BufWriter::new(file), spec)?;
        let mut block = [[0.0; 2]; BLOCK];
        let mut positions = [0.0; BLOCK];
        let mut remaining = frames as usize;
        while remaining > 0 {
            check_cancelled()?;
            let count = remaining.min(BLOCK);
            renderer.render(&mut block[..count], &mut positions[..count])?;
            for frame in &block[..count] {
                ensure!(
                    frame[0].is_finite() && frame[1].is_finite(),
                    "Mix produced non-finite samples; reduce track or insert gain"
                );
                writer.write_sample(frame[0])?;
                writer.write_sample(frame[1])?;
            }
            remaining -= count;
        }
        check_cancelled()?;
        writer.finalize()?;
        check_cancelled()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
