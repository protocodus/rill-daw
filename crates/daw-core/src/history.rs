//! Session-local, bounded edit history. Audio data is referenced by path, never
//! copied here. Opaque plugin state counts toward the memory budget.
use crate::{ClipContent, Effect, Instrument, Project};
use std::{
    collections::VecDeque,
    mem::size_of,
    time::{Duration, Instant},
};

pub const HISTORY_STEPS: usize = 128;
pub const HISTORY_BYTES: usize = 128 * 1024 * 1024;

struct Entry {
    project: Project,
    label: String,
    bytes: usize,
    state: u64,
}

pub struct History {
    undo: VecDeque<Entry>,
    redo: VecDeque<Entry>,
    bytes: usize,
    state: u64,
    next_state: u64,
    saved: Option<u64>,
    group: Option<(String, Instant)>,
    max_steps: usize,
    max_bytes: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::with_limits(HISTORY_STEPS, HISTORY_BYTES)
    }
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(max_steps: usize, max_bytes: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            bytes: 0,
            state: 0,
            next_state: 1,
            saved: Some(0),
            group: None,
            max_steps,
            max_bytes,
        }
    }

    /// Record the state immediately before one complete edit gesture.
    pub fn commit(&mut self, project: &Project) {
        self.commit_named(project, "Edit");
    }

    pub fn commit_named(&mut self, project: &Project, label: impl Into<String>) {
        self.commit_inner(project, label.into(), None);
    }

    /// Coalesce a burst from the same text field or native plugin control. A
    /// save, undo, redo, different control, or 600 ms pause ends the burst.
    pub fn commit_grouped(&mut self, project: &Project, label: impl Into<String>, key: String) {
        self.commit_inner(project, label.into(), Some(key));
    }

    fn commit_inner(&mut self, project: &Project, label: String, key: Option<String>) {
        let now = Instant::now();
        let merge = key.as_ref().is_some_and(|key| {
            self.group.as_ref().is_some_and(|(last, at)| {
                last == key && now.duration_since(*at) < Duration::from_millis(600)
            })
        }) && !self.undo.is_empty()
            && self.redo.is_empty();
        for entry in self.redo.drain(..) {
            self.bytes -= entry.bytes;
        }
        if !merge {
            let bytes = snapshot_bytes(project) + label.len() + size_of::<Entry>();
            if bytes <= self.max_bytes && self.max_steps > 0 {
                self.undo.push_back(Entry {
                    project: project.clone(),
                    label,
                    bytes,
                    state: self.state,
                });
                self.bytes += bytes;
            } else {
                // An unavailable step is a boundary: never jump across it and
                // restore an unrelated earlier document on the next Undo.
                self.undo.clear();
                self.bytes = 0;
            }
        }
        self.state = self.next_state;
        self.next_state += 1;
        self.group = key.map(|key| (key, now));
        self.trim(true);
    }

    pub fn undo(&mut self, project: &mut Project) -> bool {
        self.group = None;
        let Some(entry) = self.undo.pop_back() else {
            return false;
        };
        self.bytes -= entry.bytes;
        let bytes = snapshot_bytes(project) + entry.label.len() + size_of::<Entry>();
        let previous = std::mem::replace(project, entry.project);
        if bytes <= self.max_bytes && self.max_steps > 0 {
            self.redo.push_back(Entry {
                project: previous,
                label: entry.label,
                bytes,
                state: self.state,
            });
            self.bytes += bytes;
        } else {
            for entry in self.redo.drain(..) {
                self.bytes -= entry.bytes;
            }
        }
        self.state = entry.state;
        self.trim(false);
        true
    }

    pub fn redo(&mut self, project: &mut Project) -> bool {
        self.group = None;
        let Some(entry) = self.redo.pop_back() else {
            return false;
        };
        self.bytes -= entry.bytes;
        let bytes = snapshot_bytes(project) + entry.label.len() + size_of::<Entry>();
        let previous = std::mem::replace(project, entry.project);
        if bytes <= self.max_bytes && self.max_steps > 0 {
            self.undo.push_back(Entry {
                project: previous,
                label: entry.label,
                bytes,
                state: self.state,
            });
            self.bytes += bytes;
        } else {
            for entry in self.undo.drain(..) {
                self.bytes -= entry.bytes;
            }
        }
        self.state = entry.state;
        self.trim(true);
        true
    }

    fn trim(&mut self, retain_undo: bool) {
        while self.bytes > self.max_bytes || self.undo.len() + self.redo.len() > self.max_steps {
            let entry = if retain_undo {
                self.redo.pop_front().or_else(|| self.undo.pop_front())
            } else {
                self.undo.pop_front().or_else(|| self.redo.pop_front())
            };
            if let Some(entry) = entry {
                self.bytes -= entry.bytes;
            } else {
                break;
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_labels(&self) -> impl Iterator<Item = &str> {
        self.undo.iter().rev().map(|entry| entry.label.as_str())
    }
    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.redo.iter().rev().map(|entry| entry.label.as_str())
    }
    pub fn memory_bytes(&self) -> usize {
        self.bytes
    }
    pub fn state_id(&self) -> u64 {
        self.state
    }

    /// UI-only edit context can follow exactly the same bounded set of states.
    pub fn retained_state_ids(&self) -> impl Iterator<Item = u64> + '_ {
        std::iter::once(self.state)
            .chain(self.undo.iter().map(|entry| entry.state))
            .chain(self.redo.iter().map(|entry| entry.state))
    }
    pub fn seal_group(&mut self) {
        self.group = None;
    }
    pub fn is_saved(&self) -> bool {
        self.saved == Some(self.state)
    }
    pub fn mark_saved_at(&mut self, state: u64) {
        self.saved = Some(state);
        self.group = None;
    }
    pub fn mark_unsaved(&mut self) {
        self.saved = None;
        self.group = None;
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
        self.group = None;
        self.state = self.next_state;
        self.next_state += 1;
        self.saved = Some(self.state);
    }
}

// Heap sizes of cloned data plus inline storage. No JSON serialization or
// sample buffers are involved. Allocator bookkeeping is outside this estimate.
fn snapshot_bytes(project: &Project) -> usize {
    let mut bytes = size_of::<Project>() + project.name.len();
    for track in &project.tracks {
        bytes += size_of_val(track)
            + track.name.len()
            + track.instrument_name.len()
            + track.instrument_state.len()
            + track.instrument_parameters.len() * size_of::<crate::PluginParameterValue>()
            + track.sends.len() * size_of::<crate::Send>();
        if let Instrument::Vst3 { path, class_id } = &track.instrument {
            bytes += path.len() + class_id.as_ref().map_or(0, String::len);
        }
        for effect in &track.inserts {
            bytes += size_of_val(effect)
                + effect.name.len()
                + effect.state.len()
                + effect.parameters.len() * size_of::<crate::PluginParameterValue>();
            let Effect::Vst3 { path, class_id } = &effect.effect;
            bytes += path.len() + class_id.as_ref().map_or(0, String::len);
        }
        if let Some(lane) = &track.automation {
            bytes += lane.points.len() * size_of::<crate::AutomationPoint>();
            bytes += lane.clips.len() * size_of::<crate::AutomationClip>();
            for clip in &lane.clips {
                bytes += clip.name.len() + clip.points.len() * size_of::<crate::AutomationPoint>();
            }
        }
        for clip in &track.clips {
            bytes += size_of_val(clip) + clip.name.len();
            bytes += match &clip.content {
                ClipContent::Midi { notes } => notes.len() * size_of::<crate::Note>(),
                ClipContent::Audio { path, .. } => path.len(),
            };
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branching_and_saved_checkpoints_survive_undo_redo() {
        let mut history = History::new();
        let mut project = Project::new();
        history.commit_named(&project, "Tempo");
        project.tempo = 100.0;
        history.mark_saved_at(history.state_id());
        history.commit_named(&project, "Name");
        project.name = "Song".into();
        assert!(!history.is_saved());
        assert!(history.undo(&mut project));
        assert!(history.is_saved());
        assert_eq!(history.redo_labels().collect::<Vec<_>>(), vec!["Name"]);
        assert!(history.redo(&mut project));
        assert!(!history.is_saved());
        history.undo(&mut project);
        history.commit_named(&project, "Loop");
        project.loop_enabled = true;
        assert!(!history.can_redo());
        assert!(!history.is_saved());
    }

    #[test]
    fn grouped_changes_end_at_save_and_do_not_merge_unrelated_controls() {
        let mut history = History::new();
        let mut project = Project::new();
        for value in [100.0, 101.0, 102.0] {
            history.commit_grouped(&project, "Tempo", "tempo".into());
            project.tempo = value;
        }
        assert_eq!(history.undo_labels().count(), 1);
        let saved = history.state_id();
        history.mark_saved_at(saved);
        history.commit_grouped(&project, "Tempo", "tempo".into());
        project.tempo = 110.0;
        assert_eq!(history.undo_labels().count(), 2);
        history.undo(&mut project);
        assert_eq!(project.tempo, 102.0);
        assert!(history.is_saved());
        history.undo(&mut project);
        assert_eq!(project.tempo, Project::new().tempo);
    }

    #[test]
    fn step_and_memory_limits_include_plugin_states_in_both_directions() {
        let mut project = Project::new();
        let track = project.add_midi_track("Plugin");
        project.track_mut(track).unwrap().instrument_state = vec![1; 4096];
        let budget = snapshot_bytes(&project) * 3;
        let mut history = History::with_limits(2, budget);
        for value in 0..10 {
            history.commit(&project);
            project.tempo = 100.0 + value as f64;
        }
        assert_eq!(history.undo_labels().count(), 2);
        assert!(history.memory_bytes() <= budget);
        while history.undo(&mut project) {
            assert!(history.memory_bytes() <= budget);
        }
        while history.redo(&mut project) {
            assert!(history.memory_bytes() <= budget);
        }
        project.track_mut(track).unwrap().instrument_state = vec![0; budget + 1];
        history.commit(&project);
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert_eq!(history.memory_bytes(), 0);
    }
}
