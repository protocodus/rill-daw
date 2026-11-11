//! Mixer presentation order. Routing and song identities remain with the project.
use crate::app::DawApp;
use daw_core::{Project, TrackKind};
use eframe::egui;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Output disclosure is presentation only. New instrument outputs start folded;
/// no channel is removed from the project or the engine while it is folded.
#[derive(Clone, Default)]
pub(crate) struct MixerOutputView {
    generation: u64,
    pub(crate) expanded: HashSet<u64>,
}

impl MixerOutputView {
    pub(crate) fn read(ctx: &egui::Context, generation: u64) -> Self {
        ctx.data(|data| data.get_temp::<Self>(egui::Id::new("mixer_output_view")))
            .filter(|state| state.generation == generation)
            .unwrap_or_else(|| Self {
                generation,
                ..Self::default()
            })
    }

    pub(crate) fn store(self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("mixer_output_view"), self));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MixerOrder {
    SignalFlow,
    #[default]
    Song,
}

impl MixerOrder {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::SignalFlow => "Signal flow",
            Self::Song => "Song order",
        }
    }

    pub(crate) fn read(ctx: &egui::Context) -> Self {
        ctx.data(|data| data.get_temp(egui::Id::new("mixer_order")))
            .unwrap_or_default()
    }
}

/// Automation retains its own arranger selection, while the mixer presents the
/// channel that owns the parameter. Pure resolution never changes the song.
pub(crate) fn channel_owner(project: &Project, track: u64) -> Option<u64> {
    let track = project.track(track)?;
    let owner = track.automation.as_ref().map_or(track, |lane| {
        project.track(lane.target.track).unwrap_or(track)
    });
    matches!(
        owner.kind,
        TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
    )
    .then_some(owner.id)
}

impl DawApp {
    pub(crate) fn visible_arranger_tracks(&self) -> Vec<(usize, usize)> {
        if self.arranger_selection.revealed_folders.is_empty() {
            crate::track_order::visible_tracks(&self.project)
        } else {
            crate::track_order::visible_tracks_with_expanded(
                &self.project,
                &self.arranger_selection.revealed_folders,
            )
        }
    }

    /// Keep one shared track identity while moving between the two workspaces.
    pub(crate) fn show_channel_in_mixer(&mut self, ctx: &egui::Context, track: u64) {
        if self.project.track(track).is_none_or(|track| {
            !matches!(
                track.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            )
        }) {
            return;
        }
        if self.selected_track != Some(track) {
            self.selected_clip = None;
            self.arranger_selection.ids.clear();
            self.arranger_selection.span = None;
        }
        self.selected_track = Some(track);
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("mixer_reveal_channel"), track));
        self.open_workspace(crate::app::WorkspaceView::Mixer);
    }

    pub(crate) fn queue_arranger_channel_reveal(&mut self, track: u64) {
        if self.project.track(track).is_none() {
            return;
        }
        let mut parent = self
            .project
            .track(track)
            .and_then(|track| track.parent_folder);
        for _ in 0..self.project.tracks.len() {
            let Some(folder) = parent.and_then(|id| self.project.track(id)) else {
                break;
            };
            self.arranger_selection.revealed_folders.insert(folder.id);
            parent = folder.parent_folder;
        }
        self.arranger_selection.reveal = Some((track, self.selected_clip.unwrap_or(0)));
    }

    pub(crate) fn show_channel_in_arranger(&mut self, track: u64) {
        if track == daw_core::MASTER_CHANNEL_ID {
            self.selected_track = Some(track);
            self.open_popup(crate::app::PopupKind::Inspector);
            return;
        }
        if self.project.track(track).is_none() {
            return;
        }
        if self.selected_track != Some(track) {
            self.selected_clip = None;
            self.arranger_selection.ids.clear();
            self.arranger_selection.span = None;
        }
        self.selected_track = Some(track);
        self.queue_arranger_channel_reveal(track);
        self.open_workspace(crate::app::WorkspaceView::Arrangement);
    }

    pub(crate) fn mixer_track_navigation(&mut self, ui: &mut egui::Ui) {
        if let Some(track) = self
            .selected_track
            .filter(|id| self.project.track(*id).is_some())
            .filter(|id| *id != daw_core::MASTER_CHANNEL_ID)
            && ui
                .add(if ui.ctx().content_rect().width() < 1200.0 {
                    daw_ui::IconButton::icon_only(daw_ui::Icon::Arranger, "Show in Arranger")
                        .quiet()
                } else {
                    daw_ui::IconButton::with_icon(daw_ui::Icon::Arranger, "Show in Arranger")
                        .quiet()
                })
                .on_hover_text("Reveal this channel's track in Arrangement")
                .clicked()
        {
            self.show_channel_in_arranger(track);
        }
    }

    pub(crate) fn mixer_order_menu(&self, ui: &mut egui::Ui) {
        let mut order = MixerOrder::read(ui.ctx());
        let previous = order;
        daw_ui::view_menu(ui, order.label(), |ui| {
            for choice in [MixerOrder::SignalFlow, MixerOrder::Song] {
                let help = match choice {
                    MixerOrder::SignalFlow => "Sources first, then downstream channels and Master. Includes main outputs, sends and plug-in inputs; instrument outputs stay grouped.",
                    MixerOrder::Song => "Follow Arrangement order, keeping instrument outputs together. Drag channel names to reorder the song.",
                };
                if daw_ui::selectable_value(ui, &mut order, choice, choice.label())
                    .on_hover_text(help)
                    .clicked()
                {
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(match order {
            MixerOrder::SignalFlow => "Channel order follows the signal graph. Choose Song order to drag channels.",
            MixerOrder::Song => "Channel order follows Arrangement. Drag channel names to reorder.",
        });
        if order != previous {
            egui::DragAndDrop::clear_payload(ui.ctx());
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("mixer_order"), order);
            });
        }
    }
}

pub(crate) fn ordered_channels(project: &Project, order: MixerOrder) -> Vec<u64> {
    let channels: HashMap<_, _> = project
        .tracks
        .iter()
        .filter(|track| {
            matches!(
                track.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            )
        })
        .map(|track| (track.id, track))
        .collect();
    let song_order: Vec<_> = project
        .ordered_track_ids()
        .into_iter()
        .filter(|id| channels.contains_key(id))
        .collect();
    if order == MixerOrder::Song {
        return grouped_instrument_outputs(&channels, song_order);
    }

    let position: HashMap<_, _> = song_order
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    let mut incoming: HashMap<_, usize> = song_order.iter().map(|id| (*id, 0)).collect();
    let mut outgoing = HashMap::<u64, BTreeSet<u64>>::new();
    for track in channels.values() {
        // A channel may have both its main output and a send aimed at one bus;
        // that dependency must be counted only once.
        let targets: BTreeSet<_> = track
            .output_bus
            .into_iter()
            .chain(track.sends.iter().map(|send| send.target_bus))
            .filter(|id| channels.contains_key(id))
            .collect();
        for target in &targets {
            *incoming.get_mut(target).unwrap() += 1;
        }
        outgoing.insert(track.id, targets);
    }
    for track in channels.values() {
        for source in track
            .instrument_inputs
            .iter()
            .chain(track.inserts.iter().flat_map(|slot| &slot.inputs))
            .map(|input| input.source_track)
            .chain(track.instrument_output.map(|output| output.track))
        {
            if let Some(targets) = outgoing.get_mut(&source)
                && targets.insert(track.id)
            {
                *incoming.get_mut(&track.id).unwrap() += 1;
            }
        }
    }
    let priority = |id: u64| (channels[&id].kind == TrackKind::Bus, position[&id], id);
    let mut ready: BTreeSet<_> = song_order
        .iter()
        .filter(|id| incoming[id] == 0)
        .map(|id| priority(*id))
        .collect();
    let mut result = Vec::with_capacity(song_order.len());
    while let Some((_, _, id)) = ready.pop_first() {
        result.push(id);
        for target in &outgoing[&id] {
            let count = incoming.get_mut(target).unwrap();
            *count -= 1;
            if *count == 0 {
                ready.insert(priority(*target));
            }
        }
    }
    // Validation normally rejects feedback. Keep every strip reachable even
    // while an incomplete or externally supplied graph is being repaired.
    if result.len() != song_order.len() {
        let seen: HashSet<_> = result.iter().copied().collect();
        let mut remaining: Vec<_> = song_order
            .into_iter()
            .filter(|id| !seen.contains(id))
            .collect();
        remaining.sort_by_key(|id| priority(*id));
        result.extend(remaining);
    }
    grouped_instrument_outputs(&channels, result)
}

/// Mixer groups define visible adjacency: auxiliary outputs move among their
/// siblings, while ordinary channels move past complete instrument groups.
pub(crate) fn mixer_shift_targets(project: &Project) -> crate::track_order::TrackShiftTargets {
    let tracks: HashMap<_, _> = project
        .tracks
        .iter()
        .map(|track| (track.id, track))
        .collect();
    let mut siblings: HashMap<(Option<u64>, Option<u64>), Vec<u64>> = HashMap::new();
    for id in project.ordered_track_ids() {
        let Some(track) = tracks.get(&id) else {
            continue;
        };
        if matches!(
            track.kind,
            TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
        ) {
            siblings
                .entry((
                    track.parent_folder,
                    track.instrument_output.map(|output| output.track),
                ))
                .or_default()
                .push(id);
        }
    }
    crate::track_order::sibling_shift_targets(siblings.values().map(Vec::as_slice))
}

fn grouped_instrument_outputs(
    tracks: &HashMap<u64, &daw_core::Track>,
    channels: Vec<u64>,
) -> Vec<u64> {
    let mut outputs: HashMap<u64, Vec<u64>> = HashMap::new();
    for id in &channels {
        if let Some(output) = tracks.get(id).and_then(|track| track.instrument_output) {
            outputs.entry(output.track).or_default().push(*id);
        }
    }
    let mut result = Vec::with_capacity(channels.len());
    for id in &channels {
        if tracks
            .get(id)
            .is_some_and(|track| track.instrument_output.is_some())
        {
            continue;
        }
        result.push(*id);
        if let Some(children) = outputs.get(id) {
            result.extend(children);
        }
    }
    // Keep an externally supplied incomplete graph repairable in the mixer.
    // Normal projects have already emitted every channel; avoid even this
    // additional membership pass unless an output's owner is missing.
    if result.len() != channels.len() {
        let seen: HashSet<_> = result.iter().copied().collect();
        result.extend(channels.into_iter().filter(|id| !seen.contains(id)));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::InstrumentOutput;

    fn output(project: &mut Project, owner: u64, name: &str) -> u64 {
        let id = project.add_bus(name);
        project.track_mut(id).unwrap().instrument_output = Some(InstrumentOutput {
            track: owner,
            bus: 1,
            first_channel: 0,
        });
        id
    }

    #[test]
    fn grouping_keeps_output_sibling_order_and_emits_incomplete_channels_once() {
        let mut project = Project::new();
        let first = project.add_midi_track("First");
        let other = project.add_audio_track("Other");
        let second = project.add_midi_track("Second");
        let first_b = output(&mut project, first, "First B");
        let second_a = output(&mut project, second, "Second A");
        let first_a = output(&mut project, first, "First A");
        let missing = output(&mut project, u64::MAX, "Missing owner");
        let nested = output(&mut project, first_b, "Invalid nested output");
        let tracks = project
            .tracks
            .iter()
            .map(|track| (track.id, track))
            .collect();
        assert_eq!(
            grouped_instrument_outputs(
                &tracks,
                vec![
                    first_b, other, second, first_a, first, second_a, missing, nested,
                ]
            ),
            vec![
                other, second, second_a, first, first_b, first_a, missing, nested
            ]
        );
    }

    #[test]
    fn mixer_shift_actions_share_folder_and_output_group_boundaries() {
        let mut project = Project::new();
        let owner = project.add_midi_track("Owner");
        let other = project.add_audio_track("Other");
        let first = output(&mut project, owner, "First output");
        let second = output(&mut project, owner, "Second output");
        let folder = project.add_folder("Folder");
        let child = project.add_audio_track("Child");
        project.track_mut(child).unwrap().parent_folder = Some(folder);
        let targets = mixer_shift_targets(&project);
        assert_eq!(targets[&owner].1.unwrap().target, other);
        assert_eq!(targets[&other].0.unwrap().target, owner);
        assert_eq!(targets[&first].1.unwrap().target, second);
        assert_eq!(targets[&second].0.unwrap().target, first);
        assert_eq!(targets[&child], (None, None));
    }

    #[test]
    fn signal_order_deduplicates_parallel_edges_and_retains_cycle_members() {
        let mut project = Project::new();
        let bus = project.add_bus("Bus");
        let source = project.add_audio_track("Source");
        project.track_mut(source).unwrap().output_bus = Some(bus);
        project.add_send(source, bus, 0.5, false).unwrap();
        assert_eq!(
            ordered_channels(&project, MixerOrder::SignalFlow),
            vec![source, bus]
        );
        // Incomplete editor graphs stay reachable instead of losing strips.
        project.track_mut(bus).unwrap().output_bus = Some(source);
        let ordered = ordered_channels(&project, MixerOrder::SignalFlow);
        assert_eq!(ordered.len(), 2);
        assert_eq!(
            ordered.into_iter().collect::<HashSet<_>>(),
            HashSet::from([source, bus])
        );
    }
}
