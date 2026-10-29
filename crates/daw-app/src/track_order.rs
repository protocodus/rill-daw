//! One history boundary for track placement from Arrangement and Mixer.
use crate::app::DawApp;
use daw_core::{Project, TrackKind, TrackPlacement};
use daw_ui::{TrackDropIntent, TrackDropPlacement};

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
