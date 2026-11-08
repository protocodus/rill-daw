//! Shared track presentation order and history boundary for Arrangement and Mixer.
use crate::app::DawApp;
use daw_core::{MASTER_CHANNEL_ID, Project, TrackKind, TrackPlacement};
use daw_ui::{TrackDropIntent, TrackDropPlacement};
use std::collections::{HashMap, HashSet};

pub(crate) type TrackShiftTargets =
    HashMap<u64, (Option<TrackDropIntent>, Option<TrackDropIntent>)>;

/// Build the visible sibling adjacency once, with indexed ownership lookups.
/// The mixer queries every strip each frame; repeated project scans here would
/// otherwise make even a read-only console expensive in larger sessions.
pub(crate) fn track_shift_targets(project: &Project, channels_only: bool) -> TrackShiftTargets {
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
    sibling_shift_targets(siblings.values().map(Vec::as_slice))
}

/// Both views use the same previous/next semantics, with their own grouping:
/// Arrangement uses folders; Mixer also keeps instrument outputs together.
pub(crate) fn sibling_shift_targets<'a>(
    groups: impl Iterator<Item = &'a [u64]>,
) -> TrackShiftTargets {
    groups
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
                            .map(|previous| intent(group[previous], TrackDropPlacement::Before)),
                        group
                            .get(index + 1)
                            .map(|id| intent(*id, TrackDropPlacement::After)),
                    ),
                )
            })
        })
        .collect()
}

/// Preorder preserves sibling order and places each automation lane directly
/// below its owner. Collapsed folders hide descendants, not their own lanes.
pub(crate) fn visible_tracks(project: &Project) -> Vec<(usize, usize)> {
    visible_tracks_with_expanded(project, &HashSet::new())
}

/// Presentation-only expansion when jumping from a mixer channel. Indexing
/// ownership once avoids scanning every track for every visible row each frame.
/// An explicit stack handles deep valid hierarchies without a depth cutoff.
pub(crate) fn visible_tracks_with_expanded(
    project: &Project,
    expanded: &HashSet<u64>,
) -> Vec<(usize, usize)> {
    let mut children: HashMap<Option<u64>, Vec<usize>> = HashMap::new();
    let mut automation: HashMap<u64, Vec<usize>> = HashMap::new();
    for (index, track) in project.tracks.iter().enumerate() {
        if track.kind != TrackKind::Automation {
            children.entry(track.parent_folder).or_default().push(index);
        }
        if let Some(lane) = &track.automation {
            automation.entry(lane.target.track).or_default().push(index);
        }
    }
    let mut out = Vec::with_capacity(project.tracks.len());
    // Master lives outside the track array; keep its global lanes at the top.
    if let Some(lanes) = automation.get(&MASTER_CHANNEL_ID) {
        out.extend(lanes.iter().map(|index| (*index, 0)));
    }
    let mut pending = Vec::new();
    if let Some(roots) = children.get(&None) {
        pending.extend(roots.iter().rev().map(|index| (*index, 0)));
    }
    let mut seen = vec![false; project.tracks.len()];
    while let Some((index, depth)) = pending.pop() {
        if std::mem::replace(&mut seen[index], true) {
            continue;
        }
        let track = &project.tracks[index];
        out.push((index, depth));
        if let Some(lanes) = automation.get(&track.id) {
            out.extend(lanes.iter().map(|index| (*index, depth + 1)));
        }
        if track.kind == TrackKind::Folder
            && (!track.collapsed || expanded.contains(&track.id))
            && let Some(descendants) = children.get(&Some(track.id))
        {
            pending.extend(descendants.iter().rev().map(|index| (*index, depth + 1)));
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::{AutomationParameter, AutomationTarget, Track};

    fn rows(project: &Project, expanded: &HashSet<u64>) -> Vec<(u64, usize)> {
        visible_tracks_with_expanded(project, expanded)
            .into_iter()
            .map(|(index, depth)| (project.tracks[index].id, depth))
            .collect()
    }

    #[test]
    fn visible_order_attaches_automation_and_preserves_folder_siblings() {
        let mut project = Project::new();
        let folder = project.add_folder("Folder");
        let child = project.add_audio_track("Child");
        project.track_mut(child).unwrap().parent_folder = Some(folder);
        let root = project.add_audio_track("Root");
        let lane = project
            .add_automation_track(
                "Child gain",
                AutomationTarget {
                    track: child,
                    parameter: AutomationParameter::TrackGain,
                },
            )
            .unwrap();
        let master_lane = project
            .add_automation_track(
                "Master gain",
                AutomationTarget {
                    track: MASTER_CHANNEL_ID,
                    parameter: AutomationParameter::TrackGain,
                },
            )
            .unwrap();
        assert_eq!(
            rows(&project, &HashSet::new()),
            vec![
                (master_lane, 0),
                (folder, 0),
                (child, 1),
                (lane, 2),
                (root, 0),
            ]
        );
        project.track_mut(folder).unwrap().collapsed = true;
        assert_eq!(
            rows(&project, &HashSet::new()),
            vec![(master_lane, 0), (folder, 0), (root, 0),]
        );
        assert_eq!(
            rows(&project, &HashSet::from([folder])),
            vec![
                (master_lane, 0),
                (folder, 0),
                (child, 1),
                (lane, 2),
                (root, 0),
            ]
        );
        assert!(project.track(folder).unwrap().collapsed);
    }

    #[test]
    fn deeply_nested_valid_folders_do_not_hide_tracks_after_sixty_four_levels() {
        let mut project = Project::new();
        let mut parent = None;
        for _ in 0..256 {
            let mut track = Track::new("Folder", TrackKind::Folder);
            track.parent_folder = parent;
            parent = Some(track.id);
            project.tracks.push(track);
        }
        let mut audio = Track::new("Deep audio", TrackKind::Audio);
        audio.parent_folder = parent;
        let audio_id = audio.id;
        project.tracks.push(audio);
        let result = rows(&project, &HashSet::new());
        assert_eq!(result.len(), project.tracks.len());
        assert_eq!(result.last(), Some(&(audio_id, 256)));
    }

    #[test]
    fn invalid_cycles_cannot_repeat_or_hide_unrelated_roots() {
        let mut project = Project::new();
        let first = project.add_folder("First");
        let second = project.add_folder("Second");
        let root = project.add_audio_track("Root");
        project.track_mut(first).unwrap().parent_folder = Some(second);
        project.track_mut(second).unwrap().parent_folder = Some(first);
        assert_eq!(rows(&project, &HashSet::new()), vec![(root, 0)]);
    }

    #[test]
    fn sibling_actions_stay_inside_their_group_and_keep_direction() {
        let groups = [vec![10, 20, 30], vec![40], Vec::new()];
        let targets = sibling_shift_targets(groups.iter().map(Vec::as_slice));
        assert_eq!(targets.len(), 4);
        assert_eq!(targets[&10].0, None);
        assert_eq!(targets[&30].1, None);
        assert_eq!(targets[&40], (None, None));
        let previous = targets[&20].0.unwrap();
        assert_eq!(
            (previous.source, previous.target, previous.placement),
            (20, 10, TrackDropPlacement::Before)
        );
        let following = targets[&20].1.unwrap();
        assert_eq!(
            (following.source, following.target, following.placement),
            (20, 30, TrackDropPlacement::After)
        );
    }
}
