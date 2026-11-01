//! Prepare editable timeline data away from rendering; retain the live DSP graph.
use super::*;
use daw_core::{AudioProcessing, AudioProcessingAlgorithm, PluginParameterValue, Track};
use std::{
    path::PathBuf,
    sync::{Mutex, Weak},
};

pub(super) type AudioAssetKey = (PathBuf, u64, u64, AudioProcessingAlgorithm, bool);

fn original_key(path: &Path) -> AudioAssetKey {
    (
        path.to_path_buf(),
        1.0_f64.to_bits(),
        0,
        AudioProcessingAlgorithm::Signalsmith,
        false,
    )
}

fn asset_key(path: &Path, rate: f64, processing: AudioProcessing, preview: bool) -> AudioAssetKey {
    if rate == 1.0 && processing.transpose_semitones == 0.0 {
        return original_key(path);
    }
    let pitch = if processing.transpose_semitones == 0.0 {
        0
    } else {
        processing.transpose_semitones.to_bits()
    };
    (
        path.to_path_buf(),
        rate.to_bits(),
        pitch,
        processing.algorithm,
        preview,
    )
}

#[derive(Clone)]
pub(super) struct CompiledContent {
    pub audio: Arc<Vec<AudioClip>>,
    pub audio_segments: Arc<Vec<audio_schedule::Segment>>,
    pub notes: Arc<Vec<ScheduledNote>>,
    pub events: Arc<Vec<Event>>,
    pub note_order: Arc<Vec<usize>>,
}

type ParameterState = HashMap<(u64, Option<u64>), Vec<PluginParameterValue>>;

pub(super) struct LiveEditContext {
    baseline: Arc<Project>,
    sample_rate: u32,
    options: RenderOptions,
    tail_samples: u64,
    parameters: Mutex<ParameterState>,
}

impl LiveEditContext {
    pub fn new(
        project: &Project,
        sample_rate: u32,
        options: RenderOptions,
        tail_samples: u64,
    ) -> Self {
        let mut parameters = HashMap::new();
        for track in &project.tracks {
            parameters.insert((track.id, None), track.instrument_parameters.clone());
            for slot in &track.inserts {
                parameters.insert((track.id, Some(slot.id)), slot.parameters.clone());
            }
        }
        Self {
            baseline: Arc::new(project.clone()),
            sample_rate,
            options,
            tail_samples,
            parameters: Mutex::new(parameters),
        }
    }
}

/// A single preparation owner tracks the maximum MIDI scratch storage needed by
/// every plan issued to this renderer. The UI gets a separate compatibility handle.
pub(crate) struct LiveEditPreparer {
    context: Arc<LiveEditContext>,
    /// The compiled graph keeps its topology and buffers when workspace rows
    /// move. Every later plan must retain this exact processor order.
    track_order: Vec<u64>,
    previous: Arc<Project>,
    content: HashMap<u64, CompiledContent>,
    maximum_notes: HashMap<u64, usize>,
    assets: HashMap<AudioAssetKey, Weak<AudioData>>,
}

struct TrackEdit {
    id: u64,
    name: String,
    gain: f32,
    pan: f32,
    gain_lane: Option<AutomationLane>,
    pan_lane: Option<AutomationLane>,
    send_gains: Vec<AutomatedValue>,
    instrument_automation: Vec<PluginAutomation>,
    insert_automation: Vec<(u64, Vec<PluginAutomation>)>,
    content: CompiledContent,
    midi: Vec<MidiMessage>,
    chase: Vec<Event>,
    active: Vec<ScheduledNote>,
    active_indices: HashMap<NoteKey, usize>,
    unmatched: Vec<ScheduledNote>,
    active_flags: Vec<bool>,
    pitch_next: Vec<usize>,
}

pub(crate) struct PreparedLiveEdit {
    context: Arc<LiveEditContext>,
    tracks: Vec<TrackEdit>,
    end_sample: u64,
    automation_sequence: u64,
}

impl PreparedLiveEdit {
    pub(crate) fn set_automation_sequence(&mut self, sequence: u64) {
        self.automation_sequence = sequence;
    }

    pub(crate) fn needs_refinement(&self) -> bool {
        self.tracks
            .iter()
            .any(|track| track.content.audio.iter().any(|audio| audio.is_preview))
    }
}

/// Cheap UI-side graph compatibility handle; never retains decoded audio.
#[derive(Clone)]
pub(crate) struct LiveEditCompatibility {
    context: Arc<LiveEditContext>,
}

impl LiveEditCompatibility {
    pub(crate) fn compatible(&self, project: &Project) -> bool {
        self.check(project, true)
    }

    fn check(&self, project: &Project, compare_parameters: bool) -> bool {
        let baseline = &self.context.baseline;
        let tracks: HashMap<_, _> = project
            .tracks
            .iter()
            .map(|track| (track.id, track))
            .collect();
        let parameters = self
            .context
            .parameters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        baseline.tempo == project.tempo
            && baseline.beats_per_bar == project.beats_per_bar
            && baseline.loop_enabled == project.loop_enabled
            && baseline.loop_start == project.loop_start
            && baseline.loop_end == project.loop_end
            && baseline.tracks.len() == project.tracks.len()
            && tracks.len() == project.tracks.len()
            && baseline.tracks.iter().all(|a| {
                tracks.get(&a.id).is_some_and(|b| {
                    a.kind == b.kind
                        && (a.mute == b.mute || a.kind == daw_core::TrackKind::Automation)
                        && a.solo == b.solo
                        && a.instrument == b.instrument
                        && a.instrument_state == b.instrument_state
                        && (!compare_parameters
                            || parameters.get(&(a.id, None)) == Some(&b.instrument_parameters))
                        && a.latency_samples == b.latency_samples
                        && automation_edit_compatible(baseline, project, a, b)
                        && a.sends.len() == b.sends.len()
                        && a.sends.iter().zip(&b.sends).all(|(old, new)| {
                            old.target_bus == new.target_bus
                            && old.pre_fader == new.pre_fader
                            // A silent, unautomated send is omitted from the
                            // graph. Preserve graph/PDC membership until rebuild.
                            && send_active(baseline, a.id, old) == send_active(project, b.id, new)
                        })
                        && a.output_bus == b.output_bus
                        && a.parent_folder == b.parent_folder
                        && a.inserts.len() == b.inserts.len()
                        && a.inserts.iter().zip(&b.inserts).all(|(a_slot, b_slot)| {
                            a_slot.id == b_slot.id
                                && a_slot.enabled == b_slot.enabled
                                && a_slot.effect == b_slot.effect
                                && a_slot.state == b_slot.state
                                && (!compare_parameters
                                    || parameters.get(&(a.id, Some(a_slot.id)))
                                        == Some(&b_slot.parameters))
                        })
                })
            })
    }

    /// A parameter accepted by the render worker's live queue is already part of
    /// this instance's expected state. Later ordinary edits must not restore it.
    pub(crate) fn acknowledge_parameter(
        &self,
        track: u64,
        effect: Option<u64>,
        id: u32,
        value: f64,
    ) {
        let mut all = self
            .context
            .parameters
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(parameters) = all.get_mut(&(track, effect)) {
            if let Some(parameter) = parameters.iter_mut().find(|parameter| parameter.id == id) {
                parameter.value = value;
            } else {
                parameters.push(PluginParameterValue { id, value });
            }
        }
    }
}

impl LiveEditPreparer {
    pub(crate) fn compatibility(&self) -> LiveEditCompatibility {
        LiveEditCompatibility {
            context: Arc::clone(&self.context),
        }
    }

    #[cfg(test)]
    pub(crate) fn compatible(&self, project: &Project) -> bool {
        self.compatibility().compatible(project)
    }

    #[cfg(test)]
    pub(crate) fn prepare(&mut self, project: &Project) -> Result<PreparedLiveEdit> {
        ensure!(
            self.compatible(project),
            "Edit changes the running plugin graph"
        );
        self.prepare_cancelled(project, &AtomicBool::new(false), false)
    }

    pub(crate) fn prepare_cancelled(
        &mut self,
        project: &Project,
        cancel: &AtomicBool,
        preview: bool,
    ) -> Result<PreparedLiveEdit> {
        ensure!(
            self.compatibility().check(project, false),
            "Edit changes the running plugin graph"
        );
        // The request was checked against the live parameter ledger before
        // enqueue. A newer accepted knob gesture may advance that ledger while
        // this preparation waits; this plan never restores plugin parameters.
        project.validate()?;
        ensure!(
            !cancel.load(Ordering::Acquire),
            "Audio edit preparation cancelled"
        );
        // Weak entries do not keep deleted audio alive. A prepared plan and the
        // renderer own the live sources; stale plans can be retired independently.
        self.assets.retain(|_, asset| asset.strong_count() > 0);
        let mut cache: HashMap<AudioAssetKey, Arc<AudioData>> = self
            .assets
            .iter()
            .filter_map(|(path, asset)| asset.upgrade().map(|asset| (path.clone(), asset)))
            .collect();
        let mut cache_bytes = 0;
        let mut used_sources = HashSet::new();
        let mut next_content = HashMap::new();
        let mut tracks = Vec::with_capacity(self.content.len());
        let audible = audible_tracks(project, &self.track_order);
        let project_tracks: HashMap<_, _> = project
            .tracks
            .iter()
            .map(|track| (track.id, track))
            .collect();
        let previous_tracks: HashMap<_, _> = self
            .previous
            .tracks
            .iter()
            .map(|track| (track.id, track))
            .collect();
        for &id in &self.track_order {
            ensure!(
                !cancel.load(Ordering::Acquire),
                "Audio edit preparation cancelled"
            );
            let track = *project_tracks.get(&id).context("Edited track is missing")?;
            let content = if previous_tracks
                .get(&id)
                .is_some_and(|old| old.clips == track.clips)
                && (preview
                    || !self
                        .content
                        .get(&id)
                        .is_some_and(|content| content.audio.iter().any(|audio| audio.is_preview)))
            {
                self.content
                    .get(&id)
                    .context("Prepared track is missing")?
                    .clone()
            } else {
                // The total cache limit is checked over the resulting project,
                // excluding sources held only by an older in-flight plan.
                let mut compilation_bytes = 0;
                compile_content(
                    track,
                    audible.contains(&id),
                    CompileSettings {
                        tempo: project.tempo,
                        sample_rate: self.context.sample_rate,
                        options: self.context.options,
                        cancel: Some(cancel),
                        preview,
                    },
                    &mut cache,
                    &mut compilation_bytes,
                )?
            };
            for audio in content.audio.iter() {
                cache.insert(
                    asset_key(
                        &audio.source_alias,
                        audio.source_rate,
                        audio.processing,
                        audio.is_preview,
                    ),
                    Arc::clone(&audio.data),
                );
                cache.insert(
                    original_key(&audio.source_alias),
                    Arc::clone(&audio.original),
                );
                for data in [&audio.data, &audio.original] {
                    if used_sources.insert(Arc::as_ptr(data)) {
                        cache_bytes += data.samples.len() * size_of::<f32>();
                    }
                }
            }
            ensure!(
                cache_bytes <= quality::MAX_CACHE_BYTES,
                "Prepared audio exceeds the 1 GiB session cache limit"
            );
            let maximum = self.maximum_notes.entry(id).or_default();
            *maximum = (*maximum).max(content.notes.len());
            let midi_capacity = content.events.len() + 2 * *maximum + 4;
            tracks.push(TrackEdit {
                id,
                name: track.name.clone(),
                gain: track.gain,
                pan: track.pan,
                gain_lane: active_automation_lane(project, id, AutomationParameter::TrackGain)
                    .cloned(),
                pan_lane: active_automation_lane(project, id, AutomationParameter::TrackPan)
                    .cloned(),
                send_gains: track
                    .sends
                    .iter()
                    .filter(|send| audible.contains(&id) && send_active(project, id, send))
                    .map(|send| {
                        AutomatedValue::new(
                            project,
                            id,
                            AutomationParameter::SendGain {
                                bus: send.target_bus,
                            },
                            send.gain,
                        )
                    })
                    .collect(),
                instrument_automation: plugin_automation(project, id, None),
                insert_automation: track
                    .inserts
                    .iter()
                    .filter(|slot| audible.contains(&id) && slot.enabled)
                    .map(|slot| (slot.id, plugin_automation(project, id, Some(slot.id))))
                    .collect(),
                content: content.clone(),
                midi: Vec::with_capacity(midi_capacity),
                chase: Vec::with_capacity(2 * *maximum),
                active: Vec::with_capacity(*maximum),
                // Reserve the union of retiring and new note identities. Even
                // after mass note-offs, hash-table tombstones must not cause a
                // growth allocation when the new note-ons arrive in that block.
                active_indices: HashMap::with_capacity(*maximum + content.notes.len()),
                unmatched: Vec::with_capacity(*maximum),
                active_flags: vec![false; content.notes.len()],
                pitch_next: vec![usize::MAX; content.notes.len()],
            });
            next_content.insert(id, content);
        }
        self.assets = cache
            .iter()
            .filter(|(_, value)| Arc::strong_count(value) > 1)
            .map(|(path, value)| (path.clone(), Arc::downgrade(value)))
            .collect();
        self.content = next_content;
        self.previous = Arc::new(project.clone());
        let samples_per_beat = self.context.sample_rate as f64 * 60.0 / project.tempo;
        Ok(PreparedLiveEdit {
            context: Arc::clone(&self.context),
            tracks,
            end_sample: (project.end_beat() * samples_per_beat).round() as u64
                + self.context.tail_samples,
            automation_sequence: 0,
        })
    }
}

impl Renderer {
    pub(crate) fn live_edit_preparer(&self) -> LiveEditPreparer {
        let mut assets = HashMap::new();
        let mut content = HashMap::new();
        let mut maximum_notes = HashMap::new();
        for track in self.tracks.iter().flatten() {
            for audio in track.audio.iter() {
                for path in [&audio.source_path, &audio.source_alias] {
                    assets.insert(
                        asset_key(path, audio.source_rate, audio.processing, audio.is_preview),
                        Arc::downgrade(&audio.data),
                    );
                    assets.insert(original_key(path), Arc::downgrade(&audio.original));
                }
            }
            maximum_notes.insert(track.id, track.notes.len());
            content.insert(
                track.id,
                CompiledContent {
                    audio: Arc::clone(&track.audio),
                    audio_segments: Arc::clone(&track.audio_segments),
                    notes: Arc::clone(&track.notes),
                    events: Arc::clone(&track.events),
                    note_order: Arc::clone(&track.note_order),
                },
            );
        }
        LiveEditPreparer {
            context: Arc::clone(&self.live_edit_context),
            track_order: self.tracks.iter().flatten().map(|track| track.id).collect(),
            previous: Arc::clone(&self.live_edit_context.baseline),
            content,
            maximum_notes,
            assets,
        }
    }

    /// Validate before swapping anything. The caller retires this same plan on
    /// the preparation thread, including on error, so heavy drops stay off audio.
    pub(crate) fn apply_live_edit(&mut self, edit: &mut PreparedLiveEdit) -> Result<()> {
        ensure!(
            Arc::ptr_eq(&self.live_edit_context, &edit.context)
                && edit.tracks.len() == self.tracks.len()
                && edit
                    .tracks
                    .iter()
                    .zip(&self.tracks)
                    .all(|(next, old)| old.as_ref().is_some_and(|old| {
                        old.id == next.id
                            && old.routes.len() == next.send_gains.len() + usize::from(old.enabled)
                            && old.inserts.len() == next.insert_automation.len()
                            && old.inserts.iter().zip(&next.insert_automation).all(|(old, (id, _))| {
                                matches!(old, Insert::Vst { id: old_id, .. } if old_id == id)
                            })
                    })),
            "Prepared edit belongs to a different audio graph"
        );
        let mut content_changed = false;
        for (next, old) in edit.tracks.iter_mut().zip(&mut self.tracks) {
            let track = old.as_mut().unwrap();
            std::mem::swap(&mut track.name, &mut next.name);
            track.gain.set_manual(next.gain, self.sample_rate);
            track.pan.set_manual(next.pan, self.sample_rate);
            std::mem::swap(&mut track.gain.lane, &mut next.gain_lane);
            std::mem::swap(&mut track.pan.lane, &mut next.pan_lane);
            for (route, gain) in track.routes.iter_mut().skip(1).zip(&mut next.send_gains) {
                route.gain.set_manual(gain.manual, self.sample_rate);
                std::mem::swap(&mut route.gain.lane, &mut gain.lane);
            }
            swap_plugin_automation(
                &mut track.instrument_automation,
                &mut next.instrument_automation,
                edit.automation_sequence,
            );
            for (insert, (_, next_parameters)) in
                track.inserts.iter_mut().zip(&mut next.insert_automation)
            {
                let Insert::Vst { parameters, .. } = insert;
                swap_plugin_automation(parameters, next_parameters, edit.automation_sequence);
            }
            if Arc::ptr_eq(&next.content.events, &track.events)
                && Arc::ptr_eq(&next.content.audio, &track.audio)
            {
                continue;
            }
            content_changed = true;
            next.chase.clear();
            next.active.clear();
            next.active_indices.clear();
            next.unmatched.clear();
            next.active_flags.fill(false);
            if !self.preview {
                let mut heads = [usize::MAX; 128];
                for (index, note) in next.content.notes.iter().enumerate() {
                    if note.start < self.source_sample && note.end > self.source_sample {
                        next.active_flags[index] = true;
                        next.pitch_next[index] = heads[note.pitch as usize];
                        heads[note.pitch as usize] = index;
                    }
                }
                // The identity index is sorted during preparation. Match every
                // surviving identity first, then reconcile remaining pitch counts
                // for split/join edits. No nested scan of the active voices.
                for &old_note in &track.active_notes {
                    let key = (old_note.clip, old_note.id, old_note.pitch);
                    let exact = next
                        .content
                        .note_order
                        .binary_search_by_key(&key, |&index| {
                            let note = next.content.notes[index];
                            (note.clip, note.id, note.pitch)
                        })
                        .ok()
                        .map(|index| next.content.note_order[index])
                        .filter(|&index| next.active_flags[index]);
                    if let Some(index) = exact {
                        next.active_flags[index] = false;
                        next.active.push(next.content.notes[index]);
                    } else {
                        next.unmatched.push(old_note);
                    }
                }
                for &old_note in &next.unmatched {
                    let head = &mut heads[old_note.pitch as usize];
                    while *head != usize::MAX && !next.active_flags[*head] {
                        *head = next.pitch_next[*head];
                    }
                    if *head != usize::MAX {
                        let index = *head;
                        *head = next.pitch_next[index];
                        next.active_flags[index] = false;
                        next.active.push(next.content.notes[index]);
                    } else {
                        next.active.push(old_note);
                        next.chase.push(Event {
                            sample: self.source_sample,
                            note: old_note,
                            on: false,
                        });
                    }
                }
                for (index, &note) in next.content.notes.iter().enumerate() {
                    if next.active_flags[index] {
                        next.chase.push(Event {
                            sample: self.source_sample,
                            note,
                            on: true,
                        });
                    }
                }
            }
            // Capacity and hash state were created off the render thread.
            // Populate only retained voices; per-event updates then remain O(1).
            for (index, note) in next.active.iter().enumerate() {
                next.active_indices.insert(note.key(), index);
            }
            std::mem::swap(&mut track.active_indices, &mut next.active_indices);
            if !self.preview
                && !Arc::ptr_eq(&track.audio, &next.content.audio)
                && (!track.audio.is_empty() || !next.content.audio.is_empty())
            {
                let old_cursor = track.audio_transition_cursor;
                let length = track.audio_transition.len();
                for index in 0..length {
                    let position = self.source_sample + index as u64;
                    let old = timeline_sample(
                        &track.audio,
                        &track.audio_segments,
                        position,
                        self.sample_rate,
                    );
                    let new = timeline_sample(
                        &next.content.audio,
                        &next.content.audio_segments,
                        position,
                        self.sample_rate,
                    );
                    let retiring = if old_cursor + index < length {
                        let gain = 1.0 - (old_cursor + index) as f32 / (length - 1).max(1) as f32;
                        track.audio_transition[old_cursor + index].map(|sample| sample * gain)
                    } else {
                        [0.0; 2]
                    };
                    track.audio_transition[index] =
                        [old[0] + retiring[0] - new[0], old[1] + retiring[1] - new[1]];
                }
                track.audio_transition_cursor = 0;
            }
            std::mem::swap(&mut track.audio, &mut next.content.audio);
            std::mem::swap(&mut track.audio_segments, &mut next.content.audio_segments);
            std::mem::swap(&mut track.notes, &mut next.content.notes);
            std::mem::swap(&mut track.events, &mut next.content.events);
            std::mem::swap(&mut track.note_order, &mut next.content.note_order);
            std::mem::swap(&mut track.midi, &mut next.midi);
            std::mem::swap(&mut track.chase, &mut next.chase);
            std::mem::swap(&mut track.active_notes, &mut next.active);
            // Reconciliation already handles notes ending exactly at the new
            // frontier. Notes starting there remain normal sample-offset events.
            track.event_cursor = track.events.partition_point(|event| {
                event.sample < self.source_sample
                    || (event.sample == self.source_sample && !event.on)
            });
        }
        self.timeline_end_sample = edit.end_sample;
        if content_changed && !self.preview {
            // Removing the final clip behind the playhead must still deliver
            // pending note-offs and drain existing plugin/PDC tails. This window
            // is bounded by the graph's known tail and resets on an explicit seek.
            let bridge = self
                .tracks
                .iter()
                .flatten()
                .map(|track| {
                    track
                        .audio_transition
                        .len()
                        .saturating_sub(track.audio_transition_cursor) as u64
                })
                .max()
                .unwrap_or(0);
            self.release_end_sample = self.release_end_sample.max(
                self.source_sample
                    .saturating_add(self.live_edit_context.tail_samples.max(bridge)),
            );
        }
        self.end_sample = self.timeline_end_sample.max(self.release_end_sample);
        Ok(())
    }
}

fn automation_edit_compatible(before: &Project, after: &Project, a: &Track, b: &Track) -> bool {
    match (&a.automation, &b.automation) {
        (None, None) => true,
        (Some(old), Some(new)) if old.target == new.target => {
            if matches!(
                old.target.parameter,
                AutomationParameter::VstParameter { .. }
            ) {
                // Dropping VST automation must restore its opaque preset/manual
                // parameter state. Keep that transition on the rebuild path;
                // curve and override changes retain the same loaded instance.
                let enabled = |project: &Project, track: &Track, lane: &AutomationLane| {
                    lane_enabled(project, track)
                        && (!lane.points.is_empty()
                            || lane.clips.iter().any(|clip| !clip.points.is_empty())
                            || lane.override_value.is_some())
                };
                enabled(before, a, old) == enabled(after, b, new)
                    // A clip-only lane needs the preset's original value in its
                    // gaps. Reload once when removing the arranger fallback so
                    // that value can be read before automation touches the plug-in.
                    && (old.points.is_empty() == new.points.is_empty() || new.clips.is_empty())
            } else {
                true
            }
        }
        _ => false,
    }
}

/// Prepared lane buffers replace the old ones without allocating or destroying
/// them on the render worker. Keep the last submitted values when identities
/// are unchanged, avoiding redundant plug-in IPC during unrelated live edits.
fn swap_plugin_automation(
    current: &mut Vec<PluginAutomation>,
    next: &mut Vec<PluginAutomation>,
    sequence: u64,
) {
    for (index, new) in next.iter_mut().enumerate() {
        new.action_sequence = sequence;
        // Preparation sorts parameter identities, making the render-worker swap
        // linear even when automation rows have moved. Keep a fallback for a
        // future preparer that changes their order.
        if let Some(old) = current
            .get(index)
            .filter(|old| old.id == new.id)
            .or_else(|| current.iter().find(|old| old.id == new.id))
        {
            new.previous = old.previous;
            if new.manual.is_none() {
                new.manual = old.manual;
            }
            if old.action_sequence > sequence {
                // An immediate override made after this project snapshot must
                // survive both its prepared preview and its final refinement.
                new.lane.override_value = old.lane.override_value;
                new.manual = old.manual;
                new.action_sequence = old.action_sequence;
            }
        }
    }
    std::mem::swap(current, next);
}

/// Bounded lookup used only for the 5 ms edit bridge, never a whole asset render.
fn timeline_sample(
    audio: &[AudioClip],
    segments: &[audio_schedule::Segment],
    position: u64,
    sample_rate: u32,
) -> [f32; 2] {
    let index = segments.partition_point(|segment| segment.end <= position);
    let Some(segment) = segments
        .get(index)
        .filter(|segment| segment.start <= position)
    else {
        return [0.0; 2];
    };
    let b = audio[segment.incoming].sample(position, sample_rate);
    if let (Some(outgoing), Some(curve)) = (segment.outgoing, &segment.fade) {
        let progress = (position - segment.start) as f64 / (segment.end - segment.start) as f64;
        let gain = curve.value(progress, &mut curve.cursor(progress));
        let a = audio[outgoing].sample(position, sample_rate);
        [
            a[0] * (1.0 - gain) + b[0] * gain,
            a[1] * (1.0 - gain) + b[1] * gain,
        ]
    } else {
        b
    }
}

pub(super) struct CompileSettings<'a> {
    pub tempo: f64,
    pub sample_rate: u32,
    pub options: RenderOptions,
    pub cancel: Option<&'a AtomicBool>,
    pub preview: bool,
}

pub(super) fn compile_content(
    track: &Track,
    enabled: bool,
    settings: CompileSettings<'_>,
    audio_cache: &mut HashMap<AudioAssetKey, Arc<AudioData>>,
    cache_bytes: &mut usize,
) -> Result<CompiledContent> {
    let CompileSettings {
        tempo,
        sample_rate,
        options,
        cancel,
        preview,
    } = settings;
    let check_cancelled = || {
        ensure!(
            !cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)),
            "Audio edit preparation cancelled"
        );
        Ok::<(), anyhow::Error>(())
    };
    let samples_per_beat = sample_rate as f64 * 60.0 / tempo;
    let to_sample = |beat: f64| (beat.max(0.0) * samples_per_beat).round() as u64;
    let mut notes = Vec::new();
    let mut audio = Vec::new();
    for clip in &track.clips {
        check_cancelled()?;
        if !enabled || clip.is_take_source() {
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
                    let relative_start = clip.source_to_local(note.start);
                    let relative_end = relative_start + note.length / clip.source_rate;
                    if relative_end <= 0.0 || relative_start >= clip.length || note.velocity == 0 {
                        continue;
                    }
                    notes.push(ScheduledNote {
                        clip: clip.id,
                        id: note.id,
                        start: to_sample(clip.start + relative_start.max(0.0)),
                        end: to_sample(clip.start + relative_end.min(clip.length)),
                        pitch: note.pitch.min(127),
                        velocity: note.velocity.min(127),
                    });
                }
            }
            ClipContent::Audio {
                path,
                processing,
                gain,
                fade_in,
                fade_out,
                fade_in_curve,
                fade_out_curve,
                reverse,
                envelope,
                boundary_fades,
                ..
            } => {
                let requested = PathBuf::from(path);
                let source_path = if audio_cache.contains_key(&original_key(&requested)) {
                    requested
                } else {
                    std::fs::canonicalize(path)
                        .with_context(|| format!("Loading audio clip {} ({path})", clip.name))?
                };
                let original = if let Some(data) = audio_cache.get(&original_key(&source_path)) {
                    Arc::clone(data)
                } else {
                    let source = quality::load_source(&source_path)
                        .with_context(|| format!("Loading audio clip {} ({path})", clip.name))?;
                    check_cancelled()?;
                    let data = Arc::new(quality::prepare_audio_cancelled(
                        source,
                        sample_rate,
                        options.resample_quality,
                        cancel,
                    )?);
                    check_cancelled()?;
                    *cache_bytes = cache_bytes
                        .checked_add(data.samples.len() * size_of::<f32>())
                        .context("Audio cache size overflow")?;
                    ensure!(
                        *cache_bytes <= quality::MAX_CACHE_BYTES,
                        "Prepared audio exceeds the 1 GiB session cache limit"
                    );
                    audio_cache.insert(original_key(&source_path), Arc::clone(&data));
                    data
                };
                let full_key = asset_key(&source_path, clip.source_rate, *processing, false);
                let is_preview = preview
                    && crate::stretch::preview_worthwhile(&original)
                    && !(clip.source_rate == 1.0 && processing.transpose_semitones == 0.0)
                    && !audio_cache.contains_key(&full_key);
                let key = if is_preview {
                    asset_key(&source_path, clip.source_rate, *processing, true)
                } else {
                    full_key
                };
                let data = if clip.source_rate == 1.0 && processing.transpose_semitones == 0.0 {
                    Arc::clone(&original)
                } else if let Some(data) = audio_cache.get(&key) {
                    Arc::clone(data)
                } else {
                    check_cancelled()?;
                    let prepared_bytes = (original.frames() as f64 / clip.source_rate).ceil()
                        as usize
                        * original.channels
                        * size_of::<f32>();
                    ensure!(
                        cache_bytes.saturating_add(prepared_bytes) <= quality::MAX_CACHE_BYTES,
                        "Prepared audio exceeds the 1 GiB session cache limit"
                    );
                    let prepare = if is_preview {
                        crate::stretch::prepare_preview
                    } else {
                        crate::stretch::prepare_processed
                    };
                    let data = Arc::new(prepare(
                        &original,
                        clip.source_rate,
                        processing.transpose_semitones,
                        processing.algorithm,
                        cancel,
                    )?);
                    *cache_bytes += data.samples.len() * size_of::<f32>();
                    audio_cache.insert(key, Arc::clone(&data));
                    data
                };
                ensure!(
                    data.channels > 0 && data.sample_rate > 0,
                    "Invalid WAV format"
                );
                audio.push(AudioClip {
                    source_path,
                    source_alias: PathBuf::from(path),
                    source_rate: clip.source_rate,
                    processing: *processing,
                    is_preview,
                    original,
                    start: to_sample(clip.start),
                    source_offset: clip.offset / clip.source_rate * 60.0 / tempo
                        * data.sample_rate as f64,
                    gain: *gain,
                    fade_in: fade_in.max(0.0) * samples_per_beat / clip.source_rate,
                    fade_out: fade_out.max(0.0) * samples_per_beat / clip.source_rate,
                    fade_in_curve: *fade_in_curve,
                    fade_out_curve: *fade_out_curve,
                    envelope_offset: envelope.map_or(0.0, |envelope| envelope.offset)
                        * samples_per_beat
                        / clip.source_rate,
                    envelope_length: envelope
                        .map_or(clip.source_length(), |envelope| envelope.length)
                        * samples_per_beat
                        / clip.source_rate,
                    boundary_fades: boundary_fades.map(|boundary| {
                        let scale = samples_per_beat / clip.source_rate;
                        daw_core::AudioBoundaryFades {
                            offset: boundary.offset * scale,
                            length: boundary.length * scale,
                            fade_in: boundary.fade_in * scale,
                            fade_out: boundary.fade_out * scale,
                        }
                    }),
                    source_length: clip.length * 60.0 / tempo * data.sample_rate as f64,
                    reverse: *reverse,
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
    let mut note_order: Vec<usize> = (0..notes.len()).collect();
    note_order.sort_unstable_by_key(|&index| {
        let note = notes[index];
        (note.clip, note.id, note.pitch)
    });
    Ok(CompiledContent {
        audio: Arc::new(audio),
        audio_segments: Arc::new(audio_schedule::compile(&track.clips, samples_per_beat)?),
        notes: Arc::new(notes),
        events: Arc::new(events),
        note_order: Arc::new(note_order),
    })
}
