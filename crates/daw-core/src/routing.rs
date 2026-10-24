use crate::{Effect, Project, Send, Track, TrackKind};
use anyhow::{Context, Result, ensure};
use std::collections::{HashMap, HashSet, VecDeque};

impl Effect {
    pub(crate) fn validate(&self) -> Result<()> {
        let Self::Vst3 { path, class_id } = self;
        ensure!(!path.is_empty(), "Missing VST3 insert path");
        crate::model::validate_vst_class_id(class_id.as_deref())?;
        Ok(())
    }
}

impl Project {
    pub fn add_bus(&mut self, name: impl Into<String>) -> u64 {
        let mut track = Track::new(name, TrackKind::Bus);
        track.gain = 1.0;
        let id = track.id;
        self.tracks.push(track);
        id
    }

    pub fn add_folder(&mut self, name: impl Into<String>) -> u64 {
        let mut track = Track::new(name, TrackKind::Folder);
        track.gain = 1.0;
        let id = track.id;
        self.tracks.push(track);
        id
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
        for track in &mut candidate.tracks {
            if track.output_bus == Some(track_id) {
                track.output_bus = None;
            }
            track.sends.retain(|send| send.target_bus != track_id);
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
        let source = self.track(track_id).context("Source track not found")?;
        ensure!(
            matches!(
                source.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            ),
            "Only audio, MIDI, and bus tracks have an audio output"
        );
        let mut incoming: HashMap<u64, Vec<u64>> = HashMap::new();
        for track in &self.tracks {
            for destination in track
                .output_bus
                .into_iter()
                .chain(track.sends.iter().map(|send| send.target_bus))
            {
                incoming.entry(destination).or_default().push(track.id);
            }
        }
        let mut excluded = HashSet::from([track_id]);
        let mut pending = vec![track_id];
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
            .filter(|track| track.kind == TrackKind::Bus && !excluded.contains(&track.id))
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
        let track = self.track_mut(track_id).context("Track not found")?;
        let previous = std::mem::replace(&mut track.parent_folder, parent_folder);
        if let Err(error) = self.routing_order() {
            self.track_mut(track_id)
                .expect("track exists")
                .parent_folder = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Stable source-before-destination audio processing order. Includes empty
    /// buses, excludes folders/automation, and validates both signal and folder graphs.
    /// This is a control-thread operation, not an audio callback operation.
    pub fn routing_order(&self) -> Result<Vec<u64>> {
        let mut indices = HashMap::with_capacity(self.tracks.len());
        for (index, track) in self.tracks.iter().enumerate() {
            ensure!(
                indices.insert(track.id, index).is_none(),
                "Duplicate track ID"
            );
        }
        let mut incoming = vec![0usize; self.tracks.len()];
        let mut targets = vec![Vec::new(); self.tracks.len()];
        for (index, track) in self.tracks.iter().enumerate() {
            if matches!(track.kind, TrackKind::Folder | TrackKind::Automation) {
                ensure!(
                    track.output_bus.is_none()
                        && track.sends.is_empty()
                        && track.inserts.is_empty()
                        && track.clips.is_empty(),
                    "Folders and automation tracks cannot contain audio, inserts, or routing"
                );
            }
            if track.kind == TrackKind::Bus {
                ensure!(track.clips.is_empty(), "Bus tracks cannot contain clips");
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
                    self.tracks[destination].kind == TrackKind::Bus,
                    "Audio routing destinations must be buses"
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
}
