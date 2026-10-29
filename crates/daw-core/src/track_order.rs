//! Stable track placement shared by Arrangement, Mixer and song commands.
use crate::{Project, TrackKind};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackPlacement {
    Before,
    After,
    IntoFolder,
}

impl Project {
    /// The complete visual order, with each folder followed by its descendants.
    /// Siblings retain their saved relative order. Collapsing a folder affects
    /// visibility only and never removes its children from this ordering.
    /// Iterative traversal also keeps incomplete editor states bounded: orphaned
    /// or cyclic rows are appended once in saved order until validation fixes them.
    pub fn ordered_track_ids(&self) -> Vec<u64> {
        let mut children: HashMap<Option<u64>, Vec<usize>> = HashMap::new();
        for (index, track) in self.tracks.iter().enumerate() {
            children.entry(track.parent_folder).or_default().push(index);
        }
        let mut seen = vec![false; self.tracks.len()];
        let mut ordered = Vec::with_capacity(self.tracks.len());
        let mut pending = Vec::new();
        if let Some(roots) = children.get(&None) {
            pending.extend(roots.iter().rev().copied());
        }
        let mut orphan = 0;
        loop {
            while let Some(index) = pending.pop() {
                if std::mem::replace(&mut seen[index], true) {
                    continue;
                }
                let id = self.tracks[index].id;
                ordered.push(id);
                if let Some(descendants) = children.get(&Some(id)) {
                    pending.extend(descendants.iter().rev().copied());
                }
            }
            while orphan < seen.len() && seen[orphan] {
                orphan += 1;
            }
            if orphan == seen.len() {
                break;
            }
            pending.push(orphan);
        }
        ordered
    }

    /// Move one track and its complete folder subtree without replacing any
    /// musical, routing, plug-in or automation identity. Before/After makes the
    /// source a sibling of the destination; IntoFolder appends it to that folder.
    /// Invalid placements and visual no-ops leave the entire project untouched.
    pub fn reorder_track(
        &mut self,
        source: u64,
        target: u64,
        placement: TrackPlacement,
    ) -> Result<bool> {
        let source_parent = self
            .track(source)
            .context("Source track not found")?
            .parent_folder;
        let target_track = self.track(target).context("Destination track not found")?;
        if source == target && placement != TrackPlacement::IntoFolder {
            return Ok(false);
        }
        let parent = match placement {
            TrackPlacement::Before | TrackPlacement::After => target_track.parent_folder,
            TrackPlacement::IntoFolder => {
                ensure!(
                    target_track.kind == TrackKind::Folder,
                    "Choose a destination folder"
                );
                Some(target)
            }
        };
        // Validate the current graphs before deriving complete subtree ranges.
        // Their only proposed change is the root's parent, checked below.
        self.routing_order()?;
        let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
        for track in &self.tracks {
            if let Some(parent) = track.parent_folder {
                children.entry(parent).or_default().push(track.id);
            }
        }
        let subtree = |root| {
            let mut ids = HashSet::new();
            let mut pending = vec![root];
            while let Some(id) = pending.pop() {
                if ids.insert(id)
                    && let Some(descendants) = children.get(&id)
                {
                    pending.extend(descendants);
                }
            }
            ids
        };
        let moving = subtree(source);
        ensure!(
            !moving.contains(&target),
            "A folder cannot be placed inside its own subtree"
        );
        let before = self.ordered_track_ids();
        let block: Vec<_> = before
            .iter()
            .copied()
            .filter(|id| moving.contains(id))
            .collect();
        let mut after: Vec<_> = before
            .iter()
            .copied()
            .filter(|id| !moving.contains(id))
            .collect();
        let target_index = after
            .iter()
            .position(|id| *id == target)
            .expect("destination retained");
        let insert = if placement == TrackPlacement::Before {
            target_index
        } else {
            let target_subtree = subtree(target);
            after
                .iter()
                .enumerate()
                .skip(target_index)
                .take_while(|(_, id)| target_subtree.contains(id))
                .last()
                .map_or(target_index + 1, |(index, _)| index + 1)
        };
        after.splice(insert..insert, block);
        if source_parent == parent && before == after {
            return Ok(false);
        }
        let rank: HashMap<_, _> = after
            .into_iter()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        self.tracks.sort_by_cached_key(|track| rank[&track.id]);
        self.track_mut(source)
            .expect("source retained")
            .parent_folder = parent;
        Ok(true)
    }
}
