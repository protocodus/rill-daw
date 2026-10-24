use anyhow::{Context, Result, bail, ensure};
use daw_core::{AudioData, ClipContent, Effect, Instrument, Project, TrackKind};
use daw_plugins::{MidiMessage, VstEffect, VstInstrument};
use std::{
    collections::{HashMap, HashSet},
    f32::consts::TAU,
    path::Path,
    sync::Arc,
};

pub(crate) const BLOCK: usize = 256;
const VOICES: usize = 128;

#[derive(Clone, Copy)]
struct ScheduledNote {
    start: u64,
    end: u64,
    pitch: u8,
    velocity: u8,
    id: usize,
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

#[derive(Clone, Copy, Default)]
struct Voice {
    released: bool,
    id: usize,
    phase: f32,
    phase_step: f32,
    gain: f32,
    envelope: f32,
}

struct Synth {
    voices: [Voice; VOICES],
    voice_count: usize,
    waveform: u8,
    sample_rate: f32,
}

impl Synth {
    fn event(&mut self, event: Event) {
        if event.on {
            let index = if self.voice_count < VOICES {
                let index = self.voice_count;
                self.voice_count += 1;
                index
            } else {
                event.note.id % VOICES
            };
            self.voices[index] = Voice {
                id: event.note.id,
                phase_step: 440.0 * 2.0_f32.powf((event.note.pitch as f32 - 69.0) / 12.0)
                    / self.sample_rate,
                gain: event.note.velocity as f32 / 127.0 * 0.18,
                ..Voice::default()
            };
        } else {
            for voice in &mut self.voices[..self.voice_count] {
                if voice.id == event.note.id {
                    voice.released = true;
                }
            }
        }
    }

    fn sample(&mut self) -> f32 {
        let mut result = 0.0;
        let mut index = 0;
        while index < self.voice_count {
            let voice = &mut self.voices[index];
            if voice.released {
                voice.envelope = (voice.envelope - 1.0 / (self.sample_rate * 0.005)).max(0.0);
                if voice.envelope == 0.0 {
                    self.voice_count -= 1;
                    self.voices[index] = self.voices[self.voice_count];
                    continue;
                }
            } else {
                voice.envelope = (voice.envelope + 1.0 / (self.sample_rate * 0.002)).min(1.0);
            }
            let wave = match self.waveform {
                1 => 1.0 - 4.0 * (voice.phase - 0.5).abs(),
                2 => 2.0 * voice.phase - 1.0 - poly_blep(voice.phase, voice.phase_step),
                _ => (voice.phase * TAU).sin(),
            };
            result += wave * voice.gain * voice.envelope;
            voice.phase = (voice.phase + voice.phase_step).fract();
            index += 1;
        }
        result
    }
}

fn poly_blep(t: f32, step: f32) -> f32 {
    if t < step {
        let x = t / step;
        x + x - x * x - 1.0
    } else if t > 1.0 - step {
        let x = (t - 1.0) / step;
        x * x + x + x + 1.0
    } else {
        0.0
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
    Builtin(Box<Synth>),
    Vst(Box<VstInstrument>),
}

enum Insert {
    Gain(f32),
    Delay {
        frames: Vec<[f32; 2]>,
        cursor: usize,
        feedback: f32,
        mix: f32,
    },
    Vst(Box<VstEffect>),
}

struct Route {
    target: Option<usize>,
    gain: f32,
    pre_fader: bool,
    delay: CompensationDelay,
}

fn audible_tracks(project: &Project, order: &[u64]) -> HashSet<u64> {
    let mut unmuted = HashSet::new();
    let mut selected = HashSet::new();
    let any_solo = project.tracks.iter().any(|track| track.solo);
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
    audio: Vec<AudioClip>,
    notes: Vec<ScheduledNote>,
    events: Vec<Event>,
    event_cursor: usize,
    processor: Processor,
    midi: Vec<MidiMessage>,
    chase: Vec<MidiMessage>,
    left: Vec<f32>,
    right: Vec<f32>,
    left_gain: f32,
    right_gain: f32,
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
}

pub(crate) struct Renderer {
    tracks: Vec<RenderTrack>,
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
        let mut end_sample = 0;
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
                                id: notes.len(),
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
                Instrument::Vst3 { path } if enabled && track.kind == TrackKind::Midi => {
                    let plugin = VstInstrument::load(Path::new(path), sample_rate as f64, BLOCK)
                        .with_context(|| format!("Loading VST3 instrument on {}", track.name))?;
                    let latency = plugin.latency_samples();
                    // VST3's infinite-tail sentinel is bounded to keep automatic stop useful.
                    local_tail += (plugin.tail_samples() as u64).min(sample_rate as u64 * 30);
                    (Processor::Vst(Box::new(plugin)), latency)
                }
                instrument => (
                    Processor::Builtin(Box::new(Synth {
                        voices: [Voice::default(); VOICES],
                        voice_count: 0,
                        sample_rate: sample_rate as f32,
                        waveform: match instrument {
                            Instrument::Triangle => 1,
                            Instrument::Saw => 2,
                            _ => 0,
                        },
                    })),
                    0,
                ),
            };
            let pan = track.pan.clamp(-1.0, 1.0);
            let left_gain = track.gain * (1.0 - pan.max(0.0));
            let right_gain = track.gain * (1.0 + pan.min(0.0));
            let event_capacity = events.len() + notes.len();
            let mut inserts = Vec::new();
            let mut latency = track.latency_samples.saturating_add(plugin_latency);
            for slot in &track.inserts {
                if !enabled || !slot.enabled {
                    continue;
                }
                inserts.push(match &slot.effect {
                    Effect::Gain { gain } => Insert::Gain(*gain),
                    Effect::Delay {
                        time_ms,
                        feedback,
                        mix,
                    } => {
                        let length = (*time_ms as f64 * sample_rate as f64 / 1000.0)
                            .round()
                            .max(1.0) as usize;
                        let echoes = if feedback.abs() > 0.001 {
                            (0.001_f32.ln() / feedback.abs().ln()).ceil().max(1.0) as u64
                        } else {
                            1
                        };
                        local_tail += (length as u64 * echoes).min(sample_rate as u64 * 30);
                        Insert::Delay {
                            frames: vec![[0.0; 2]; length],
                            cursor: 0,
                            feedback: *feedback,
                            mix: *mix,
                        }
                    }
                    Effect::Vst3 { path } => {
                        let effect = VstEffect::load(Path::new(path), sample_rate as f64, BLOCK)
                            .with_context(|| {
                                format!("Loading insert {} on {}", slot.name, track.name)
                            })?;
                        latency = latency.saturating_add(effect.latency_samples());
                        local_tail += (effect.tail_samples() as u64).min(sample_rate as u64 * 30);
                        Insert::Vst(Box::new(effect))
                    }
                });
            }
            let mut routes = Vec::new();
            if enabled {
                routes.push(Route {
                    target: track.output_bus.map(|target| indices[&target]),
                    gain: 1.0,
                    pre_fader: false,
                    delay: CompensationDelay::new(0),
                });
                for send in &track.sends {
                    if send.gain > 0.0 {
                        routes.push(Route {
                            target: Some(indices[&send.target_bus]),
                            gain: send.gain,
                            pre_fader: send.pre_fader,
                            delay: CompensationDelay::new(0),
                        });
                    }
                }
            }
            tracks.push(RenderTrack {
                audio,
                notes,
                events,
                event_cursor: 0,
                processor,
                midi: Vec::with_capacity(event_capacity),
                chase: Vec::with_capacity(event_capacity),
                left: vec![0.0; BLOCK],
                right: vec![0.0; BLOCK],
                left_gain,
                right_gain,
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
        let mut renderer = Self {
            tracks,
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

    pub(crate) fn position_beats(&self) -> f64 {
        self.logical_sample as f64 / self.sample_rate as f64 * self.tempo / 60.0
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
        for track in &mut self.tracks {
            track.delay.reset();
            for route in &mut track.routes {
                route.delay.reset();
            }
            for insert in &mut track.inserts {
                match insert {
                    Insert::Gain(_) => {}
                    Insert::Delay { frames, cursor, .. } => {
                        frames.fill([0.0; 2]);
                        *cursor = 0;
                    }
                    Insert::Vst(effect) if !track.plugin_failed => {
                        if let Err(error) = effect.reset() {
                            if !self.tolerate_plugin_failure {
                                return Err(error);
                            }
                            track.plugin_failed = true;
                            self.plugin_errors += 1;
                        }
                    }
                    Insert::Vst(_) => {}
                }
            }
            track.event_cursor = track.events.partition_point(|event| event.sample < sample);
            track.chase.clear();
            match &mut track.processor {
                Processor::Builtin(synth) => {
                    synth.voices.fill(Voice::default());
                    synth.voice_count = 0;
                }
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
                        Processor::Builtin(synth) => synth.event(Event {
                            sample,
                            note,
                            on: true,
                        }),
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

    fn segment(&mut self, output: &mut [[f32; 2]]) -> Result<()> {
        output.fill([0.0; 2]);
        let count = output.len();
        let start = self.source_sample;
        let end = start + count as u64;
        for track in &mut self.tracks {
            track.left[..count].fill(0.0);
            track.right[..count].fill(0.0);
        }
        for track_index in 0..self.tracks.len() {
            let (processed, remaining) = self.tracks.split_at_mut(track_index + 1);
            let track = &mut processed[track_index];
            if !track.enabled || track.plugin_failed {
                continue;
            }
            match &mut track.processor {
                Processor::Builtin(synth) => {
                    for index in 0..count {
                        while let Some(event) = track.events.get(track.event_cursor) {
                            if event.sample > start + index as u64 {
                                break;
                            }
                            synth.event(*event);
                            track.event_cursor += 1;
                        }
                        let sample = synth.sample();
                        track.left[index] += sample;
                        track.right[index] += sample;
                    }
                }
                Processor::Vst(plugin) => {
                    if track.plugin_failed {
                        continue;
                    }
                    track.midi.clear();
                    track.midi.append(&mut track.chase);
                    while let Some(event) = track.events.get(track.event_cursor) {
                        if event.sample >= end {
                            break;
                        }
                        track.midi.push(MidiMessage {
                            offset: event.sample.saturating_sub(start) as u32,
                            pitch: event.note.pitch,
                            velocity: event.note.velocity,
                            on: event.on,
                        });
                        track.event_cursor += 1;
                    }
                    let process = plugin
                        .set_transport(
                            start as i64,
                            start as f64 / self.sample_rate as f64 * self.tempo / 60.0,
                            self.tempo,
                            true,
                        )
                        .and_then(|()| {
                            plugin.process(
                                &track.midi,
                                &mut track.left[..count],
                                &mut track.right[..count],
                            )
                        });
                    if let Err(error) = process {
                        if !self.tolerate_plugin_failure {
                            return Err(error);
                        }
                        track.plugin_failed = true;
                        track.delay.reset();
                        self.plugin_errors += 1;
                        continue;
                    }
                }
            }
            for clip in &track.audio {
                if clip.start >= end || clip.end <= start {
                    continue;
                }
                let first = clip.start.saturating_sub(start) as usize;
                let last = (clip.end - start).min(count as u64) as usize;
                let source_frames = clip.data.samples.len() / clip.data.channels;
                for index in first..last {
                    let elapsed = start + index as u64 - clip.start;
                    let source = clip.source_offset
                        + elapsed as f64 * clip.data.sample_rate as f64 / self.sample_rate as f64;
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
                    track.left[index] += sample(0);
                    track.right[index] += sample(1);
                }
            }
            let mut effect_latency = track.input_latency + track.source_latency;
            for insert in &mut track.inserts {
                match insert {
                    Insert::Gain(gain) => {
                        for index in 0..count {
                            track.left[index] *= *gain;
                            track.right[index] *= *gain;
                        }
                    }
                    Insert::Delay {
                        frames,
                        cursor,
                        feedback,
                        mix,
                    } => {
                        for index in 0..count {
                            let dry = [track.left[index], track.right[index]];
                            let wet = frames[*cursor];
                            frames[*cursor] =
                                [dry[0] + wet[0] * *feedback, dry[1] + wet[1] * *feedback];
                            *cursor = (*cursor + 1) % frames.len();
                            track.left[index] = dry[0] * (1.0 - *mix) + wet[0] * *mix;
                            track.right[index] = dry[1] * (1.0 - *mix) + wet[1] * *mix;
                        }
                    }
                    Insert::Vst(effect) => {
                        let position = start as i64 - effect_latency as i64;
                        let process = effect
                            .set_transport(
                                position,
                                position as f64 / self.sample_rate as f64 * self.tempo / 60.0,
                                self.tempo,
                                true,
                            )
                            .and_then(|()| {
                                effect.process(
                                    &track.left[..count],
                                    &track.right[..count],
                                    &mut track.insert_left[..count],
                                    &mut track.insert_right[..count],
                                )
                            });
                        if let Err(error) = process {
                            if !self.tolerate_plugin_failure {
                                return Err(error);
                            }
                            track.plugin_failed = true;
                            self.plugin_errors += 1;
                            break;
                        }
                        std::mem::swap(&mut track.left, &mut track.insert_left);
                        std::mem::swap(&mut track.right, &mut track.insert_right);
                        effect_latency += effect.latency_samples();
                    }
                }
            }
            if track.plugin_failed {
                continue;
            }
            for index in 0..count {
                let delayed = track.delay.process([track.left[index], track.right[index]]);
                track.left[index] = delayed[0];
                track.right[index] = delayed[1];
            }
            for route in &mut track.routes {
                let gain_left = route.gain
                    * if route.pre_fader {
                        1.0
                    } else {
                        track.left_gain
                    };
                let gain_right = route.gain
                    * if route.pre_fader {
                        1.0
                    } else {
                        track.right_gain
                    };
                if let Some(target) = route.target {
                    let destination = &mut remaining[target - track_index - 1];
                    for index in 0..count {
                        let frame = route.delay.process([
                            track.left[index] * gain_left,
                            track.right[index] * gain_right,
                        ]);
                        destination.left[index] += frame[0];
                        destination.right[index] += frame[1];
                    }
                } else {
                    for (index, destination) in output.iter_mut().enumerate() {
                        let frame = route.delay.process([
                            track.left[index] * gain_left,
                            track.right[index] * gain_right,
                        ]);
                        destination[0] += frame[0];
                        destination[1] += frame[1];
                    }
                }
            }
        }
        self.source_sample += count as u64;
        Ok(())
    }
}

/// Render an arrangement to interleaved stereo floating-point samples.
/// Looping is ignored for export. Plugin delay compensation is primed and trimmed.
pub fn render(project: &Project, sample_rate: u32, length_beats: f64) -> Result<Vec<f32>> {
    ensure!(
        length_beats.is_finite() && length_beats > 0.0,
        "Render length must be positive and finite"
    );
    let mut renderer = Renderer::new(project, sample_rate, false)?;
    let frame_count = (length_beats * 60.0 / project.tempo * sample_rate as f64).ceil();
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
    use daw_core::{Clip, Note, Track, TrackKind};

    fn loop_project() -> Project {
        let mut project = Project::new();
        project.loop_enabled = true;
        project.loop_start = 0.0;
        project.loop_end = 0.125;
        let mut track = Track::new("Test", TrackKind::Midi);
        track.clips.push(Clip::midi(
            "Note",
            0.0,
            1.0,
            vec![Note::new(60, 0.0, 1.0, 100)],
        ));
        project.tracks.push(track);
        project
    }

    #[test]
    fn prepared_builtin_render_including_loop_wrap_allocates_nothing() {
        let mut renderer = Renderer::new(&loop_project(), 8_000, true).unwrap();
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
    fn seeking_chases_a_note_and_uses_selected_sample_rate() {
        let mut project = loop_project();
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
    fn prepared_insert_and_bus_graph_allocates_nothing() {
        let mut project = loop_project();
        let mut bus = Track::new("Return", TrackKind::Bus);
        bus.latency_samples = 37;
        bus.inserts.push(daw_core::EffectSlot::new(
            "Echo",
            Effect::Delay {
                time_ms: 10.0,
                feedback: 0.25,
                mix: 0.5,
            },
        ));
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
}
