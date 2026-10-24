use crate::{Effect, Project, Send, Track, TrackKind};
use anyhow::{Context, Result, ensure};
use std::collections::{HashMap, HashSet, VecDeque};

impl Effect {
    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::Gain { gain } => ensure!(
                gain.is_finite() && (0.0..=16.0).contains(gain),
                "Insert gain must be between 0 and 16"
            ),
            Self::Delay {
                time_ms,
                feedback,
                mix,
            } => {
                ensure!(
                    time_ms.is_finite() && (1.0..=10_000.0).contains(time_ms),
                    "Delay time must be between 1 and 10000 ms"
                );
                ensure!(
                    feedback.is_finite() && (-0.99..=0.99).contains(feedback),
                    "Delay feedback must be between -0.99 and 0.99"
                );
                ensure!(
                    mix.is_finite() && (0.0..=1.0).contains(mix),
                    "Delay mix must be between 0 and 1"
                );
            }
            Self::Vst3 { path } => ensure!(!path.is_empty(), "Missing VST3 insert path"),
        }
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
        candidate.validate()?;
        *self = candidate;
        Ok(removed)
    }

    /// Route a track's main signal to a bus, or `None` for the master. Invalid
    /// changes leave the project unchanged, including attempted feedback loops.
    pub fn set_output(&mut self, track_id: u64, output_bus: Option<u64>) -> Result<()> {
        let track = self.track_mut(track_id).context("Source track not found")?;
        let previous = std::mem::replace(&mut track.output_bus, output_bus);
        if let Err(error) = self.routing_order() {
            self.track_mut(track_id).expect("source exists").output_bus = previous;
            return Err(error);
        }
        Ok(())
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
    /// buses, excludes folders, and validates both signal and folder graphs.
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
            if track.kind == TrackKind::Folder {
                ensure!(
                    track.output_bus.is_none()
                        && track.sends.is_empty()
                        && track.inserts.is_empty()
                        && track.clips.is_empty(),
                    "Folders organize tracks and cannot contain audio, inserts, or routing"
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
            .filter(|track| track.kind != TrackKind::Folder)
            .count();
        let mut ready: VecDeque<_> = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(index, track)| {
                (track.kind != TrackKind::Folder && incoming[index] == 0).then_some(index)
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
