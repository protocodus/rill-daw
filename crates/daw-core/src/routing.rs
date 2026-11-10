use crate::{
    Effect, Instrument, InstrumentOutput, MASTER_CHANNEL_ID, MAX_PLUGIN_AUDIO_BUSES,
    MAX_PLUGIN_AUDIO_CHANNELS, PluginInputRoute, Project, Send, Track, TrackKind,
};
use anyhow::{Context, Result, ensure};
use std::collections::{HashMap, HashSet, VecDeque};

impl Effect {
    pub(crate) fn validate(&self) -> Result<()> {
        let Self::Vst3 { path, class_id } = self;
        ensure!(!path.is_empty(), "Missing plug-in insert path");
        crate::model::validate_plugin_class_id(path, class_id.as_deref())?;
        Ok(())
    }
}

impl Project {
    /// Replace a slot's plug-in while keeping its identity and routing position.
    /// A different processor invalidates its saved state and automation IDs.
    pub fn set_effect(&mut self, track_id: u64, effect_id: u64, effect: Effect) -> Result<()> {
        let mut candidate = self.clone();
        let slot = candidate
            .track_mut(track_id)
            .context("Track not found")?
            .inserts
            .iter_mut()
            .find(|slot| slot.id == effect_id)
            .context("Insert not found")?;
        if slot.effect != effect {
            slot.effect = effect;
            slot.state.clear();
            slot.parameters.clear();
            slot.inputs.clear();
            candidate.tracks.retain(|track| {
                !track.automation.as_ref().is_some_and(|lane| {
                    lane.target.track == track_id
                        && matches!(
                            lane.target.parameter,
                            crate::AutomationParameter::VstParameter { effect: Some(id), .. }
                                if id == effect_id
                        )
                })
            });
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn add_bus(&mut self, name: impl Into<String>) -> u64 {
        let mut track = Track::new(name, TrackKind::Bus);
        track.gain = 1.0;
        self.insert_new_track(track)
    }

    pub fn add_folder(&mut self, name: impl Into<String>) -> u64 {
        let mut track = Track::new(name, TrackKind::Folder);
        track.gain = 1.0;
        self.insert_new_track(track)
    }

    /// Remove a channel without leaving dangling signal or hierarchy references.
    /// Main outputs targeting it return to master, its incoming sends disappear,
    /// and direct folder children are promoted to the removed folder's parent.
    /// The complete candidate is validated before replacing the current project.
    pub fn delete_track(&mut self, track_id: u64) -> Result<Track> {
        let index = self
            .tracks
            .iter()
            .position(|track| track.id == track_id)
            .context("Track not found")?;
        let mut candidate = self.clone();
        let removed = candidate.tracks.remove(index);
        let mut removed_ids = HashSet::from([track_id]);
        candidate.tracks.retain(|track| {
            let remove = track
                .instrument_output
                .is_some_and(|output| output.track == track_id);
            if remove {
                removed_ids.insert(track.id);
            }
            !remove
        });
        for track in candidate.channels_mut() {
            if track.output_bus.is_some_and(|id| removed_ids.contains(&id)) {
                track.output_bus = None;
            }
            track
                .sends
                .retain(|send| !removed_ids.contains(&send.target_bus));
            track
                .instrument_inputs
                .retain(|route| !removed_ids.contains(&route.source_track));
            for effect in &mut track.inserts {
                effect
                    .inputs
                    .retain(|route| !removed_ids.contains(&route.source_track));
            }
            if track.parent_folder == Some(track_id) {
                track.parent_folder = removed.parent_folder;
            }
        }
        candidate.prune_automation();
        candidate.validate()?;
        *self = candidate;
        Ok(removed)
    }

    /// Route a track's main signal to a bus, or `None` for the master. Invalid
    /// changes leave the project unchanged, including attempted feedback loops.
    pub fn set_output(&mut self, track_id: u64, output_bus: Option<u64>) -> Result<()> {
        ensure!(
            track_id != MASTER_CHANNEL_ID,
            "Master has no outgoing routing"
        );
        let track = self.track_mut(track_id).context("Source track not found")?;
        ensure!(
            matches!(
                track.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            ),
            "Only audio, MIDI, and bus tracks have an audio output"
        );
        let previous = std::mem::replace(&mut track.output_bus, output_bus);
        if let Err(error) = self.routing_order() {
            self.track_mut(track_id).expect("source exists").output_bus = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Bus destinations for a channel's main output, in project order.
    /// Excludes the source itself and every bus whose output/send paths reach it,
    /// since adding the reverse route would create feedback. Muting a channel or
    /// reducing a send to zero does not make a feedback route valid.
    ///
    /// This control-thread query traverses the graph once without cloning the
    /// project. It assumes the existing routing is valid; apply the choice with
    /// `set_output` for final validation. Master is represented separately by `None`.
    pub fn eligible_output_buses(&self, track_id: u64) -> Result<Vec<&Track>> {
        self.eligible_output_buses_for(&[track_id])
    }

    /// Destinations valid for every source in a channel selection. A single
    /// reverse-graph traversal computes the intersection, rather than rebuilding
    /// and walking the graph once per selected channel on each menu frame.
    pub fn eligible_output_buses_for(&self, track_ids: &[u64]) -> Result<Vec<&Track>> {
        ensure!(!track_ids.is_empty(), "Select at least one source channel");
        let selected: HashSet<_> = track_ids.iter().copied().collect();
        let mut found = 0;
        let mut incoming: HashMap<u64, Vec<u64>> = HashMap::new();
        for track in &self.tracks {
            if selected.contains(&track.id) {
                ensure!(
                    matches!(
                        track.kind,
                        TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                    ),
                    "Only audio, MIDI, and bus tracks have an audio output"
                );
                found += 1;
            }
            for destination in track
                .output_bus
                .into_iter()
                .chain(track.sends.iter().map(|send| send.target_bus))
            {
                incoming.entry(destination).or_default().push(track.id);
            }
            for route in track
                .instrument_inputs
                .iter()
                .chain(track.inserts.iter().flat_map(|effect| &effect.inputs))
            {
                incoming
                    .entry(track.id)
                    .or_default()
                    .push(route.source_track);
            }
            if let Some(output) = track.instrument_output {
                incoming.entry(track.id).or_default().push(output.track);
            }
        }
        ensure!(found == selected.len(), "Source track not found");
        let mut excluded = selected;
        let mut pending: Vec<_> = excluded.iter().copied().collect();
        while let Some(destination) = pending.pop() {
            if let Some(sources) = incoming.get(&destination) {
                for &source in sources {
                    if excluded.insert(source) {
                        pending.push(source);
                    }
                }
            }
        }
        Ok(self
            .tracks
            .iter()
            .filter(|track| {
                track.kind == TrackKind::Bus
                    && track.instrument_output.is_none()
                    && !excluded.contains(&track.id)
            })
            .collect())
    }

    /// Add one auxiliary send per destination bus. Pre-fader sends are taken
    /// after inserts and before gain/pan; both kinds respect track/folder mute.
    pub fn add_send(
        &mut self,
        track_id: u64,
        bus_id: u64,
        gain: f32,
        pre_fader: bool,
    ) -> Result<()> {
        ensure!(
            track_id != MASTER_CHANNEL_ID,
            "Master has no outgoing routing"
        );
        ensure!(
            gain.is_finite() && (0.0..=16.0).contains(&gain),
            "Send gain must be between 0 and 16"
        );
        let track = self.track_mut(track_id).context("Source track not found")?;
        ensure!(
            !track.sends.iter().any(|send| send.target_bus == bus_id),
            "A send to this bus already exists"
        );
        track.sends.push(Send {
            target_bus: bus_id,
            gain,
            pre_fader,
        });
        if let Err(error) = self.routing_order() {
            self.track_mut(track_id).expect("source exists").sends.pop();
            return Err(error);
        }
        Ok(())
    }

    pub fn set_parent_folder(&mut self, track_id: u64, parent_folder: Option<u64>) -> Result<()> {
        ensure!(
            track_id != MASTER_CHANNEL_ID,
            "Master cannot be placed in a folder"
        );
        ensure!(
            self.track(track_id)
                .context("Track not found")?
                .instrument_output
                .is_none(),
            "Move the owning instrument to change an output channel's folder"
        );
        let mut candidate = self.clone();
        candidate
            .track_mut(track_id)
            .expect("validated track")
            .parent_folder = parent_folder;
        for child in &mut candidate.tracks {
            if child
                .instrument_output
                .is_some_and(|output| output.track == track_id)
            {
                child.parent_folder = parent_folder;
            }
        }
        candidate.routing_order()?;
        *self = candidate;
        Ok(())
    }

    /// Stable source-before-destination audio processing order. Includes empty
    /// buses, excludes folders/automation, and validates both signal and folder graphs.
    /// This is a control-thread operation, not an audio callback operation.
    pub fn routing_order(&self) -> Result<Vec<u64>> {
        ensure!(
            self.master.id == MASTER_CHANNEL_ID
                && self.master.kind == TrackKind::Bus
                && self.master.output_bus.is_none()
                && self.master.sends.is_empty()
                && self.master.parent_folder.is_none()
                && self.master.instrument_output.is_none()
                && self.master.instrument_inputs.is_empty(),
            "Invalid Master routing"
        );
        for effect in &self.master.inserts {
            self.validate_plugin_inputs(&effect.inputs)?;
        }
        let mut indices = HashMap::with_capacity(self.tracks.len());
        for (index, track) in self.tracks.iter().enumerate() {
            ensure!(
                track.id != MASTER_CHANNEL_ID,
                "Track uses the reserved Master identity"
            );
            ensure!(
                indices.insert(track.id, index).is_none(),
                "Duplicate track ID"
            );
        }
        let mut incoming = vec![0usize; self.tracks.len()];
        let mut targets = vec![Vec::new(); self.tracks.len()];
        let mut instrument_outputs = HashSet::new();
        for (index, track) in self.tracks.iter().enumerate() {
            if matches!(track.kind, TrackKind::Folder | TrackKind::Automation) {
                ensure!(
                    track.output_bus.is_none()
                        && track.sends.is_empty()
                        && track.inserts.is_empty()
                        && track.clips.is_empty()
                        && track.instrument_inputs.is_empty()
                        && track.instrument_output.is_none()
                        && track.instrument == Instrument::None,
                    "Folders and automation tracks cannot contain audio, inserts, or routing"
                );
            }
            if track.kind == TrackKind::Bus {
                ensure!(track.clips.is_empty(), "Bus tracks cannot contain clips");
            }
            if !track.instrument_inputs.is_empty() {
                ensure!(
                    track.kind == TrackKind::Midi && track.instrument != Instrument::None,
                    "Only an assigned MIDI instrument has audio input routes"
                );
            }
            self.validate_plugin_inputs(&track.instrument_inputs)?;
            for effect in &track.inserts {
                self.validate_plugin_inputs(&effect.inputs)?;
            }
            if let Some(output) = track.instrument_output {
                ensure!(
                    track.kind == TrackKind::Bus && track.instrument == Instrument::None,
                    "Instrument output channels must be buses without an instrument"
                );
                validate_plugin_pair(output.bus, output.first_channel)?;
                ensure!(
                    instrument_outputs.insert(output),
                    "Duplicate instrument output channel"
                );
                let owner = *indices
                    .get(&output.track)
                    .context("Instrument output owner is missing")?;
                ensure!(
                    self.tracks[owner].kind == TrackKind::Midi
                        && self.tracks[owner].instrument != Instrument::None,
                    "Instrument output owner must have a MIDI instrument"
                );
                ensure!(
                    track.parent_folder == self.tracks[owner].parent_folder,
                    "Instrument output channels must share their owner's folder"
                );
                targets[owner].push(index);
                incoming[index] += 1;
            }
            for route in track
                .instrument_inputs
                .iter()
                .chain(track.inserts.iter().flat_map(|effect| &effect.inputs))
            {
                let source = *indices
                    .get(&route.source_track)
                    .context("Plug-in input source is missing")?;
                targets[source].push(index);
                incoming[index] += 1;
            }
            let mut send_targets = HashSet::new();
            for send in &track.sends {
                ensure!(
                    send.gain.is_finite() && (0.0..=16.0).contains(&send.gain),
                    "Invalid send gain"
                );
                ensure!(
                    send_targets.insert(send.target_bus),
                    "Duplicate send destination"
                );
            }
            for target in track
                .output_bus
                .into_iter()
                .chain(track.sends.iter().map(|send| send.target_bus))
            {
                let destination = *indices
                    .get(&target)
                    .context("Routing destination is missing")?;
                ensure!(
                    self.tracks[destination].kind == TrackKind::Bus
                        && self.tracks[destination].instrument_output.is_none(),
                    "Audio routing destinations must be regular buses, not instrument outputs"
                );
                targets[index].push(destination);
                incoming[destination] += 1;
            }
            let mut ancestor = track.parent_folder;
            let mut visited = HashSet::from([track.id]);
            while let Some(id) = ancestor {
                ensure!(visited.insert(id), "Folder hierarchy contains a cycle");
                let parent = &self.tracks[*indices.get(&id).context("Parent folder is missing")?];
                ensure!(
                    parent.kind == TrackKind::Folder,
                    "Track parent must be a folder"
                );
                ancestor = parent.parent_folder;
            }
        }
        let signal_tracks = self
            .tracks
            .iter()
            .filter(|track| !matches!(track.kind, TrackKind::Folder | TrackKind::Automation))
            .count();
        let mut ready: VecDeque<_> = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(index, track)| {
                (!matches!(track.kind, TrackKind::Folder | TrackKind::Automation)
                    && incoming[index] == 0)
                    .then_some(index)
            })
            .collect();
        let mut ordered = Vec::with_capacity(signal_tracks);
        while let Some(index) = ready.pop_front() {
            ordered.push(self.tracks[index].id);
            for &target in &targets[index] {
                incoming[target] -= 1;
                if incoming[target] == 0 {
                    ready.push_back(target);
                }
            }
        }
        ensure!(
            ordered.len() == signal_tracks,
            "Audio routing contains a feedback cycle"
        );
        Ok(ordered)
    }

    fn validate_plugin_inputs(&self, inputs: &[PluginInputRoute]) -> Result<()> {
        ensure!(inputs.len() <= 128, "Too many plug-in input routes");
        let mut seen = HashSet::new();
        for route in inputs {
            validate_plugin_pair(route.bus, route.first_channel)?;
            ensure!(
                route.gain.is_finite() && (0.0..=16.0).contains(&route.gain),
                "Invalid plug-in input gain"
            );
            ensure!(
                seen.insert((route.source_track, route.bus, route.first_channel)),
                "Duplicate plug-in input route"
            );
            ensure!(
                route.source_track != MASTER_CHANNEL_ID
                    && self
                        .track(route.source_track)
                        .is_some_and(|source| matches!(
                            source.kind,
                            TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                        )),
                "Plug-in input source must be an existing audio channel other than Master"
            );
        }
        Ok(())
    }

    /// Replace all external inputs of one processor as a single validated edit.
    pub fn set_plugin_inputs(
        &mut self,
        track: u64,
        effect: Option<u64>,
        inputs: Vec<PluginInputRoute>,
    ) -> Result<()> {
        let mut candidate = self.clone();
        let channel = candidate.track_mut(track).context("Track not found")?;
        if let Some(effect) = effect {
            channel
                .inserts
                .iter_mut()
                .find(|slot| slot.id == effect)
                .context("Insert not found")?
                .inputs = inputs;
        } else {
            ensure!(
                channel.kind == TrackKind::Midi && channel.instrument != Instrument::None,
                "Choose an assigned MIDI instrument"
            );
            channel.instrument_inputs = inputs;
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn add_instrument_output(
        &mut self,
        owner: u64,
        name: impl Into<String>,
        bus: u32,
        first_channel: u32,
    ) -> Result<u64> {
        Ok(self.add_instrument_outputs(owner, vec![(name.into(), bus, first_channel)])?[0])
    }

    /// Enable a native output selection with one document clone and validation.
    pub fn add_instrument_outputs(
        &mut self,
        owner: u64,
        outputs: Vec<(String, u32, u32)>,
    ) -> Result<Vec<u64>> {
        let source = self.track(owner).context("Instrument track not found")?;
        ensure!(
            source.kind == TrackKind::Midi && source.instrument != Instrument::None,
            "Choose an assigned MIDI instrument"
        );
        ensure!(
            outputs.len() <= 128,
            "Too many instrument outputs in one edit"
        );
        let (color, parent_folder) = (source.color, source.parent_folder);
        let mut candidate = self.clone();
        let mut ids = Vec::with_capacity(outputs.len());
        for (name, bus, first_channel) in outputs {
            let mut channel = Track::new(name, TrackKind::Bus);
            channel.color = color;
            channel.parent_folder = parent_folder;
            channel.instrument_output = Some(InstrumentOutput {
                track: owner,
                bus,
                first_channel,
            });
            ids.push(channel.id);
            candidate.tracks.push(channel);
        }
        candidate.validate()?;
        *self = candidate;
        Ok(ids)
    }

    /// Sources which can feed this channel without adding a feedback cycle.
    pub fn eligible_plugin_input_sources(&self, target: u64) -> Result<Vec<&Track>> {
        let target_track = self.track(target).context("Target channel not found")?;
        ensure!(
            matches!(
                target_track.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            ),
            "Choose an audio channel"
        );
        self.routing_order()?;
        let mut outgoing: HashMap<u64, Vec<u64>> = HashMap::new();
        for track in &self.tracks {
            outgoing.entry(track.id).or_default().extend(
                track
                    .output_bus
                    .into_iter()
                    .chain(track.sends.iter().map(|send| send.target_bus)),
            );
            for route in track
                .instrument_inputs
                .iter()
                .chain(track.inserts.iter().flat_map(|effect| &effect.inputs))
            {
                outgoing
                    .entry(route.source_track)
                    .or_default()
                    .push(track.id);
            }
            if let Some(output) = track.instrument_output {
                outgoing.entry(output.track).or_default().push(track.id);
            }
        }
        let mut excluded = HashSet::from([target]);
        let mut pending = vec![target];
        while let Some(source) = pending.pop() {
            if let Some(targets) = outgoing.get(&source) {
                for &destination in targets {
                    if excluded.insert(destination) {
                        pending.push(destination);
                    }
                }
            }
        }
        Ok(self
            .tracks
            .iter()
            .filter(|track| {
                !excluded.contains(&track.id)
                    && matches!(
                        track.kind,
                        TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                    )
            })
            .collect())
    }
}

fn validate_plugin_pair(bus: u32, first_channel: u32) -> Result<()> {
    ensure!(
        bus < MAX_PLUGIN_AUDIO_BUSES
            && first_channel < MAX_PLUGIN_AUDIO_CHANNELS
            && first_channel.is_multiple_of(2),
        "Invalid plug-in audio bus or stereo pair"
    );
    Ok(())
}
