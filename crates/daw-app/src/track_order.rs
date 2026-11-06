//! One history boundary for track placement from Arrangement and Mixer.
use crate::app::DawApp;
use daw_core::{Project, TrackKind, TrackPlacement};
use daw_ui::{TrackDropIntent, TrackDropPlacement};

type TrackShiftTargets =
    std::collections::HashMap<u64, (Option<TrackDropIntent>, Option<TrackDropIntent>)>;

/// A one-step move keeps the owner's folder and moves its complete subtree.
/// Mixer channels skip non-channel siblings that have no strip in that view.
pub(crate) fn track_shift_intent(
    project: &Project,
    source: u64,
    next: bool,
    channels_only: bool,
) -> Option<TrackDropIntent> {
    let (previous, following) = track_shift_targets(project, channels_only)
        .get(&source)
        .copied()?;
    if next { following } else { previous }
}

/// Build the visible sibling adjacency once, with indexed ownership lookups.
/// The mixer queries every strip each frame; repeated project scans here would
/// otherwise make even a read-only console expensive in larger sessions.
pub(crate) fn track_shift_targets(project: &Project, channels_only: bool) -> TrackShiftTargets {
    use std::collections::HashMap;
    let tracks: HashMap<_, _> = project
        .tracks
        .iter()
        .map(|track| (track.id, track))
        .collect();
    let mut siblings: HashMap<Option<u64>, Vec<u64>> = HashMap::new();
    for id in project.ordered_track_ids() {
        let track = tracks[&id];
        if !channels_only
            || matches!(
                track.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            )
        {
            siblings.entry(track.parent_folder).or_default().push(id);
        }
    }
    siblings
        .values()
        .flat_map(|group| {
            group.iter().enumerate().map(move |(index, source)| {
                let intent = |target, placement| TrackDropIntent {
                    source: *source,
                    target,
                    placement,
                };
                (
                    *source,
                    (
                        index
                            .checked_sub(1)
                            .and_then(|previous| group.get(previous))
                            .map(|id| intent(*id, TrackDropPlacement::Before)),
                        group
                            .get(index + 1)
                            .map(|id| intent(*id, TrackDropPlacement::After)),
                    ),
                )
            })
        })
        .collect()
}

/// Cheap hover validation; the core validates the complete operation on drop.
pub(crate) fn track_drop_allowed(project: &Project, intent: TrackDropIntent) -> bool {
    if intent.source == intent.target || project.track(intent.source).is_none() {
        return false;
    }
    let Some(target) = project.track(intent.target) else {
        return false;
    };
    if intent.placement == TrackDropPlacement::IntoFolder && target.kind != TrackKind::Folder {
        return false;
    }
    let mut ancestor = Some(target.id);
    for _ in 0..=project.tracks.len() {
        let Some(id) = ancestor else {
            return true;
        };
        if id == intent.source {
            return false;
        }
        let Some(track) = project.track(id) else {
            return false;
        };
        ancestor = track.parent_folder;
    }
    false
}

impl DawApp {
    pub(crate) fn apply_track_reorder(&mut self, intent: TrackDropIntent) -> bool {
        if !track_drop_allowed(&self.project, intent) {
            return false;
        }
        let placement = match intent.placement {
            TrackDropPlacement::Before => TrackPlacement::Before,
            TrackDropPlacement::After => TrackPlacement::After,
            TrackDropPlacement::IntoFolder => TrackPlacement::IntoFolder,
        };
        self.flush_pending_edit();
        self.begin_edit();
        // The core operation is atomic on errors and preserves every track ID.
        // A drop gets one history entry, independent of frames spent dragging.
        let changed = match self
            .project
            .reorder_track(intent.source, intent.target, placement)
        {
            Ok(true) => {
                self.edit_label = Some("Move track".into());
                self.changed();
                self.sync_arranger_selection();
                true
            }
            Ok(false) => false,
            Err(error) => {
                self.message(Err(error), "");
                false
            }
        };
        self.flush_pending_edit();
        changed
    }
}
