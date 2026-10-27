//! Bounded presentation context accompanying the project's own undo states.
use crate::{app::DawApp, arranger_selection::ArrangerSelectionSnapshot};
use std::collections::{BTreeMap, HashSet};

const SELECTION_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct SelectionSnapshot {
    track: Option<u64>,
    clip: Option<u64>,
    notes: Vec<u64>,
    arranger: ArrangerSelectionSnapshot,
}

impl SelectionSnapshot {
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.notes.capacity() * std::mem::size_of::<u64>()
            + self.arranger.clips.capacity() * std::mem::size_of::<(u64, u64)>()
    }
}

#[derive(Default)]
pub(crate) struct SelectionHistory {
    states: BTreeMap<u64, SelectionSnapshot>,
}

impl SelectionHistory {
    pub(crate) fn record(
        &mut self,
        before: (u64, SelectionSnapshot),
        after: (u64, SelectionSnapshot),
        retained: impl Iterator<Item = u64>,
    ) {
        self.states.insert(before.0, before.1);
        self.states.insert(after.0, after.1);
        let retained: HashSet<_> = retained.collect();
        self.states.retain(|id, _| retained.contains(id));
        let mut bytes: usize = self.states.values().map(SelectionSnapshot::bytes).sum();
        while bytes > SELECTION_BYTES {
            let Some(id) = self
                .states
                .keys()
                .find(|id| **id != after.0)
                .copied()
                .or_else(|| self.states.keys().next().copied())
            else {
                break;
            };
            bytes -= self.states.remove(&id).unwrap().bytes();
        }
    }

    pub(crate) fn get(&self, state: u64) -> Option<SelectionSnapshot> {
        self.states.get(&state).cloned()
    }
}

impl DawApp {
    pub(crate) fn capture_selection(&self) -> SelectionSnapshot {
        SelectionSnapshot {
            track: self.selected_track,
            clip: self.selected_clip,
            notes: self.selected_notes.iter().copied().collect(),
            arranger: self.arranger_selection_snapshot(),
        }
    }

    pub(crate) fn restore_history_selection(&mut self) {
        let Some(snapshot) = self.selection_history.get(self.history.state_id()) else {
            self.sync_arranger_selection();
            return;
        };
        self.selected_track = snapshot
            .track
            .filter(|id| self.project.track(*id).is_some());
        self.selected_clip = snapshot.clip.filter(|id| {
            self.selected_track
                .and_then(|track| self.project.clip(track, *id))
                .is_some_and(|clip| !clip.is_take_source())
        });
        self.restore_arranger_selection_snapshot(snapshot.arranger);
        self.selected_notes = snapshot.notes.into_iter().collect();
        self.drag = None;
        self.midi_editor.clear_hidden_selection();
    }
}
