use anyhow::{Context, Result, bail, ensure};
use daw_core::{
    AudioData, AutomationLane, AutomationParameter, ClipContent, Effect, Instrument, Project,
    TrackKind,
};
use daw_plugins::{MidiMessage, VstEffect, VstInstrument};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
    thread,
};

#[path = "worker_pool.rs"]
mod worker_pool;
use worker_pool::WorkerPool;

pub(crate) const BLOCK: usize = 256;

#[derive(Clone, Copy)]
struct ScheduledNote {
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

struct AudioClip {
    start: u64,
    end: u64,
    source_offset: f64,
    gain: f32,
    fade_in: f64,
    fade_out: f64,
    data: Arc<AudioData>,
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
        Self { manual, lane }
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
    Vst(Box<VstEffect>, Vec<PluginAutomation>),
}

struct Route {
    target: Option<usize>,
    gain: AutomatedValue,
    pre_fader: bool,
    delay: CompensationDelay,
    output: Vec<[f32; 2]>,
}

fn audible_tracks(project: &Project, order: &[u64]) -> HashSet<u64> {
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
    audio: Vec<AudioClip>,
    notes: Vec<ScheduledNote>,
    events: Vec<Event>,
    event_cursor: usize,
    processor: Processor,
    midi: Vec<MidiMessage>,
    chase: Vec<MidiMessage>,
    left: Vec<f32>,
    right: Vec<f32>,
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
    #[cfg(test)]
    observer: Option<Arc<dyn Fn(u64) + Send + Sync>>,
}

#[derive(Clone, Copy)]
struct BlockContext {
    start: u64,
    count: usize,
    sample_rate: u32,
    tempo: f64,
}

fn finish_processing(track: &mut RenderTrack, result: Result<()>, tolerate: bool) -> Result<u64> {
    if let Err(error) = result {
        if !tolerate {
            return Err(error);
        }
        track.plugin_failed = true;
        track.delay.reset();
        return Ok(1);
    }
    Ok(0)
}

impl RenderTrack {
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
        match &mut self.processor {
            Processor::None => {}
            Processor::Vst(plugin) => {
                self.midi.clear();
                self.midi.append(&mut self.chase);
                while let Some(event) = self.events.get(self.event_cursor) {
                    if event.sample >= end {
                        break;
                    }
                    self.midi.push(MidiMessage {
                        offset: event.sample.saturating_sub(start) as u32,
                        pitch: event.note.pitch,
                        velocity: event.note.velocity,
                        on: event.on,
                    });
                    self.event_cursor += 1;
                }
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
                        true,
                    )
                })
                .and_then(|()| {
                    plugin.process(
                        &self.midi,
                        &mut self.left[..count],
                        &mut self.right[..count],
                    )
                });
                process?;
            }
        }
        for clip in &self.audio {
            if clip.start >= end || clip.end <= start {
                continue;
            }
            let first = clip.start.saturating_sub(start) as usize;
            let last = (clip.end - start).min(count as u64) as usize;
            let source_frames = clip.data.samples.len() / clip.data.channels;
            for index in first..last {
                let elapsed = start + index as u64 - clip.start;
                let source = clip.source_offset
                    + elapsed as f64 * clip.data.sample_rate as f64 / context.sample_rate as f64;
                let frame = source.floor() as usize;
                if frame >= source_frames {
                    continue;
                }
                let fraction = (source - frame as f64) as f32;
                let next = (frame + 1).min(source_frames - 1);
                let fade_in = if clip.fade_in > 0.0 {
                    (elapsed as f64 / clip.fade_in).min(1.0) as f32
                } else {
                    1.0
                };
                let fade_out = if clip.fade_out > 0.0 {
                    ((clip.end - start - index as u64) as f64 / clip.fade_out).min(1.0) as f32
                } else {
                    1.0
                };
                let gain = clip.gain * fade_in * fade_out;
                let sample = |channel: usize| {
                    let channel = channel.min(clip.data.channels - 1);
                    let a = clip.data.samples[frame * clip.data.channels + channel];
                    let b = clip.data.samples[next * clip.data.channels + channel];
                    (a + (b - a) * fraction) * gain
                };
                self.left[index] += sample(0);
                self.right[index] += sample(1);
            }
        }
        let mut effect_latency = self.input_latency + self.source_latency;
        for insert in &mut self.inserts {
            match insert {
                Insert::Vst(effect, parameters) => {
                    let position = start as i64 - effect_latency as i64;
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
                            true,
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
                    process?;
                    std::mem::swap(&mut self.left, &mut self.insert_left);
                    std::mem::swap(&mut self.right, &mut self.insert_right);
                    effect_latency += effect.latency_samples();
                }
            }
        }
        for index in 0..count {
            let delayed = self.delay.process([self.left[index], self.right[index]]);
            self.left[index] = delayed[0];
            self.right[index] = delayed[1];
            let beat =
                (start as i64 + index as i64 - self.latency as i64) as f64 * beats_per_sample;
            let gain = self.gain.value(beat);
            let pan = self.pan.value(beat).clamp(-1.0, 1.0);
            // Insert scratch buffers are free after the chain. Reuse them for
            // fader factors so multiple sends evaluate the lane only once.
            self.insert_left[index] = gain * (1.0 - pan.max(0.0));
            self.insert_right[index] = gain * (1.0 + pan.min(0.0));
        }
        for route in &mut self.routes {
            for index in 0..count {
                let beat =
                    (start as i64 + index as i64 - self.latency as i64) as f64 * beats_per_sample;
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
    latency: u32,
    loop_range: Option<(u64, u64)>,
    tolerate_plugin_failure: bool,
    pub(crate) plugin_errors: u64,
}

impl Renderer {
    pub(crate) fn new(project: &Project, sample_rate: u32, looping: bool) -> Result<Self> {
        Self::new_inner(project, sample_rate, looping, None)
    }

    pub(crate) fn new_with_workers(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        workers: usize,
    ) -> Result<Self> {
        ensure!(workers > 0, "Worker count must be positive");
        Self::new_inner(project, sample_rate, looping, Some(workers))
    }

    fn new_inner(
        project: &Project,
        sample_rate: u32,
        looping: bool,
        requested_workers: Option<usize>,
    ) -> Result<Self> {
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
        let mut audio_cache = HashMap::<String, Arc<AudioData>>::new();
        let mut tail_samples = sample_rate as u64 / 5;
        for id in &order {
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
                end_sample = end_sample.max(to_sample(clip.start + clip.length));
            }
            ensure!(
                track.gain.is_finite() && track.pan.is_finite(),
                "Track gain and pan must be finite"
            );
            let mut notes = Vec::new();
            let mut audio = Vec::new();
            for clip in &track.clips {
                if !enabled {
                    continue;
                }
                ensure!(
                    clip.start.is_finite()
                        && clip.length.is_finite()
                        && clip.offset.is_finite()
                        && clip.start >= 0.0
                        && clip.length > 0.0
                        && clip.offset >= 0.0,
                    "Invalid clip timing on {}",
                    track.name
                );
                match &clip.content {
                    ClipContent::Midi { notes: clip_notes } => {
                        for note in clip_notes {
                            let relative_start = note.start - clip.offset;
                            let relative_end = relative_start + note.length;
                            if relative_end <= 0.0
                                || relative_start >= clip.length
                                || note.velocity == 0
                            {
                                continue;
                            }
                            notes.push(ScheduledNote {
                                start: to_sample(clip.start + relative_start.max(0.0)),
                                end: to_sample(clip.start + relative_end.min(clip.length)),
                                pitch: note.pitch.min(127),
                                velocity: note.velocity.min(127),
                            });
                        }
                    }
                    ClipContent::Audio {
                        path,
                        gain,
                        fade_in,
                        fade_out,
                    } => {
                        let data = if let Some(data) = audio_cache.get(path) {
                            Arc::clone(data)
                        } else {
                            let data =
                                Arc::new(AudioData::load_wav(path).with_context(|| {
                                    format!("Loading audio clip {}", clip.name)
                                })?);
                            audio_cache.insert(path.clone(), Arc::clone(&data));
                            data
                        };
                        ensure!(
                            data.channels > 0 && data.sample_rate > 0,
                            "Invalid WAV format"
                        );
                        audio.push(AudioClip {
                            start: to_sample(clip.start),
                            end: to_sample(clip.start + clip.length),
                            source_offset: clip.offset * 60.0 / project.tempo
                                * data.sample_rate as f64,
                            gain: *gain,
                            fade_in: fade_in.max(0.0) * samples_per_beat,
                            fade_out: fade_out.max(0.0) * samples_per_beat,
                            data,
                        });
                    }
                }
            }
            let mut events = Vec::with_capacity(notes.len() * 2);
            for &note in &notes {
                if note.end > note.start {
                    events.push(Event {
                        sample: note.start,
                        note,
                        on: true,
                    });
                    events.push(Event {
                        sample: note.end,
                        note,
                        on: false,
                    });
                }
            }
            events.sort_by_key(|event| (event.sample, event.on));
            let (processor, plugin_latency) = match &track.instrument {
                Instrument::Vst3 { path, class_id } if enabled && track.kind == TrackKind::Midi => {
                    let mut plugin = VstInstrument::load_class(
                        Path::new(path),
                        class_id.as_deref(),
                        sample_rate as f64,
                        BLOCK,
                    )
                    .with_context(|| format!("Loading VST3 instrument on {}", track.name))?;
                    if !track.instrument_state.is_empty() {
                        plugin.load_state(&track.instrument_state)?;
                    }
                    for parameter in &track.instrument_parameters {
                        plugin.set_parameter(parameter.id, parameter.value)?;
                    }
                    let latency = plugin.latency_samples();
                    // VST3's infinite-tail sentinel is bounded to keep automatic stop useful.
                    local_tail += (plugin.tail_samples() as u64).min(sample_rate as u64 * 30);
                    (Processor::Vst(Box::new(plugin)), latency)
                }
                _ => (Processor::None, 0),
            };
            let control =
                |parameter, manual| AutomatedValue::new(project, track.id, parameter, manual);
            let event_capacity = events.len() + notes.len();
            let mut inserts = Vec::new();
            let mut latency = track.latency_samples.saturating_add(plugin_latency);
            for slot in &track.inserts {
                if !enabled || !slot.enabled {
                    continue;
                }
                inserts.push(match &slot.effect {
                    Effect::Vst3 { path, class_id } => {
                        let effect = VstEffect::load_class(
                            Path::new(path),
                            class_id.as_deref(),
                            sample_rate as f64,
                            BLOCK,
                        )
                        .with_context(|| {
                            format!("Loading insert {} on {}", slot.name, track.name)
                        })?;
                        latency = latency.saturating_add(effect.latency_samples());
                        local_tail += (effect.tail_samples() as u64).min(sample_rate as u64 * 30);
                        Insert::Vst(
                            Box::new(effect),
                            plugin_automation(project, track.id, Some(slot.id)),
                        )
                    }
                });
            }
            let mut routes = Vec::new();
            if enabled {
                routes.push(Route {
                    target: track.output_bus.map(|target| indices[&target]),
                    gain: AutomatedValue {
                        manual: 1.0,
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
                audio,
                notes,
                events,
                event_cursor: 0,
                processor,
                midi: Vec::with_capacity(event_capacity),
                chase: Vec::with_capacity(event_capacity),
                left: vec![0.0; BLOCK],
                right: vec![0.0; BLOCK],
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
            latency,
            loop_range,
            tolerate_plugin_failure: looping,
            plugin_errors: 0,
        };
        renderer.seek_sample(0)?;
        Ok(renderer)
    }

    pub(crate) fn set_instrument_parameter(
        &mut self,
        track_id: u64,
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
            .context("Instrument track is not in the render graph")?;
        if let Processor::Vst(plugin) = &mut track.processor {
            plugin.set_parameter(parameter_id, value)?;
            for automation in &mut track.instrument_automation {
                if automation.id == parameter_id {
                    automation.previous = None;
                }
            }
            Ok(())
        } else {
            bail!("Track has no assigned VST instrument")
        }
    }

    pub(crate) fn finished(&self) -> bool {
        self.loop_range.is_none() && self.logical_sample >= self.end_sample
    }

    pub(crate) fn seek(&mut self, beat: f64) -> Result<()> {
        ensure!(
            beat.is_finite() && (0.0..=1_000_000.0).contains(&beat),
            "Seek position is outside the supported timeline"
        );
        let sample = (beat.max(0.0) * 60.0 / self.tempo * self.sample_rate as f64).round() as u64;
        self.seek_sample(sample)
    }

    fn seek_sample(&mut self, sample: u64) -> Result<()> {
        self.source_sample = sample;
        self.logical_sample = sample;
        for track in self.tracks.iter_mut().flatten() {
            for parameter in &mut track.instrument_automation {
                parameter.previous = None;
            }
            track.delay.reset();
            for route in &mut track.routes {
                route.delay.reset();
            }
            for insert in &mut track.inserts {
                match insert {
                    Insert::Vst(effect, parameters) if !track.plugin_failed => {
                        for parameter in parameters {
                            parameter.previous = None;
                        }
                        if let Err(error) = effect.reset() {
                            if !self.tolerate_plugin_failure {
                                return Err(error);
                            }
                            track.plugin_failed = true;
                            self.plugin_errors += 1;
                        }
                    }
                    Insert::Vst(..) => {}
                }
            }
            track.event_cursor = track.events.partition_point(|event| event.sample < sample);
            track.chase.clear();
            match &mut track.processor {
                Processor::None => {}
                Processor::Vst(plugin) if !track.plugin_failed => {
                    if let Err(error) = plugin.reset() {
                        if !self.tolerate_plugin_failure {
                            return Err(error);
                        }
                        track.plugin_failed = true;
                        self.plugin_errors += 1;
                    }
                }
                Processor::Vst(_) => {}
            }
            for &note in &track.notes {
                if note.start < sample && note.end > sample {
                    match &mut track.processor {
                        Processor::None => {}
                        Processor::Vst(_) => track.chase.push(MidiMessage {
                            offset: 0,
                            pitch: note.pitch,
                            velocity: note.velocity,
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
        let context = BlockContext {
            start: self.source_sample,
            count: output.len(),
            sample_rate: self.sample_rate,
            tempo: self.tempo,
        };
        for track in self.tracks.iter_mut().flatten() {
            track.left[..context.count].fill(0.0);
            track.right[..context.count].fill(0.0);
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
                self.route_track(self.levels[level_index][position], output);
            }
        }
        self.source_sample += context.count as u64;
        Ok(())
    }

    fn route_track(&mut self, track_index: usize, output: &mut [[f32; 2]]) {
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
                    destination.left[index] += frame[0];
                    destination.right[index] += frame[1];
                }
            } else {
                for (destination, frame) in output.iter_mut().zip(&route.output) {
                    destination[0] += frame[0];
                    destination[1] += frame[1];
                }
            }
        }
    }
}

/// Render an arrangement to interleaved stereo floating-point samples.
/// Looping is ignored for export. Plugin delay compensation is primed and trimmed.
pub fn render(project: &Project, sample_rate: u32, length_beats: f64) -> Result<Vec<f32>> {
    render_prepared(Renderer::new(project, sample_rate, false)?, length_beats)
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
    ensure!(
        length_beats.is_finite() && length_beats > 0.0,
        "Render length must be positive and finite"
    );
    let renderer = Renderer::new(project, sample_rate, false)?;
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
    let renderer = Renderer::new(project, sample_rate, false)?;
    let frames = renderer.end_sample;
    ensure!(
        frames * 8 < u32::MAX as u64 - 128,
        "Export exceeds the WAV 4 GiB size limit"
    );
    write_wav(renderer, path.as_ref(), frames)
}

fn write_wav(mut renderer: Renderer, path: &Path, frames: u64) -> Result<()> {
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
        writer.finalize()?;
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
