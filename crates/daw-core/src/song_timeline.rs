//! Song-level meter changes and markers. Clips stay at their beat positions
//! when meters change; only the bar layout around them moves.
use crate::{MAX_BEAT, MAX_MARKERS, Marker, MeterChange, Project, new_id, validate_marker_name};
use anyhow::{Context, Result, ensure};

impl Project {
    /// Add a meter change at `bar`, or replace the meter of an existing one.
    pub fn set_meter_change(&mut self, bar: u32, beats_per_bar: u8) -> Result<()> {
        ensure!(
            bar >= 2,
            "Change the meter of bar 1 with the time signature"
        );
        ensure!(
            (1..=32).contains(&beats_per_bar),
            "Meter changes need 1–32 beats per bar"
        );
        let previous = self.meter_changes.clone();
        match self
            .meter_changes
            .binary_search_by_key(&bar, |change| change.bar)
        {
            Ok(index) => self.meter_changes[index].beats_per_bar = beats_per_bar,
            Err(index) => self
                .meter_changes
                .insert(index, MeterChange { bar, beats_per_bar }),
        }
        let result = self.validate_timeline();
        if result.is_err() {
            self.meter_changes = previous;
        }
        result
    }

    pub fn move_meter_change(&mut self, from_bar: u32, to_bar: u32) -> Result<()> {
        let index = self.meter_change_index(from_bar)?;
        if from_bar == to_bar {
            return Ok(());
        }
        ensure!(to_bar >= 2, "Meter changes start after bar 1");
        ensure!(
            self.meter_changes.iter().all(|change| change.bar != to_bar),
            "Bar {to_bar} already has a meter change"
        );
        let previous = self.meter_changes.clone();
        let beats_per_bar = self.meter_changes.remove(index).beats_per_bar;
        let result = self.set_meter_change(to_bar, beats_per_bar);
        if result.is_err() {
            self.meter_changes = previous;
        }
        result
    }

    pub fn remove_meter_change(&mut self, bar: u32) -> Result<()> {
        let index = self.meter_change_index(bar)?;
        self.meter_changes.remove(index);
        Ok(())
    }

    fn meter_change_index(&self, bar: u32) -> Result<usize> {
        self.meter_changes
            .iter()
            .position(|change| change.bar == bar)
            .with_context(|| format!("No meter change starts at bar {bar}"))
    }

    pub(crate) fn validate_timeline(&self) -> Result<()> {
        daw_timeline::validate_meter_changes(&self.meter_changes).map_err(anyhow::Error::msg)?;
        ensure!(
            self.meter_changes
                .last()
                .is_none_or(|change| self.meter_map().bar_start(change.bar) <= MAX_BEAT),
            "Meter change is outside the supported timeline"
        );
        Ok(())
    }

    pub fn add_marker(&mut self, beat: f64, name: &str, color: Option<[u8; 3]>) -> Result<u64> {
        ensure!(
            self.markers.len() < MAX_MARKERS,
            "Too many markers (at most 1024)"
        );
        check_marker_beat(beat)?;
        validate_marker_name(name)?;
        let id = new_id();
        self.markers.push(Marker {
            id,
            beat,
            name: name.to_owned(),
            color,
        });
        self.sort_markers();
        Ok(id)
    }

    pub fn rename_marker(&mut self, id: u64, name: &str) -> Result<()> {
        validate_marker_name(name)?;
        self.marker_mut(id)?.name = name.to_owned();
        Ok(())
    }

    pub fn move_marker(&mut self, id: u64, beat: f64) -> Result<()> {
        check_marker_beat(beat)?;
        self.marker_mut(id)?.beat = beat;
        self.sort_markers();
        Ok(())
    }

    pub fn remove_marker(&mut self, id: u64) -> Result<()> {
        let before = self.markers.len();
        self.markers.retain(|marker| marker.id != id);
        ensure!(self.markers.len() < before, "Marker not found");
        Ok(())
    }

    /// The nearest marker strictly after (or before) `beat`, for navigation.
    pub fn adjacent_marker(&self, beat: f64, forward: bool) -> Option<&Marker> {
        const EPSILON: f64 = 1e-6;
        if forward {
            self.markers
                .iter()
                .find(|marker| marker.beat > beat + EPSILON)
        } else {
            self.markers
                .iter()
                .rev()
                .find(|marker| marker.beat < beat - EPSILON)
        }
    }

    fn marker_mut(&mut self, id: u64) -> Result<&mut Marker> {
        self.markers
            .iter_mut()
            .find(|marker| marker.id == id)
            .context("Marker not found")
    }

    fn sort_markers(&mut self) {
        self.markers
            .sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
    }
}

fn check_marker_beat(beat: f64) -> Result<()> {
    ensure!(
        beat.is_finite() && (0.0..=MAX_BEAT).contains(&beat),
        "Marker position is outside the timeline"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_changes_stay_sorted_and_validated() {
        let mut project = Project::new();
        project.set_meter_change(5, 3).unwrap();
        project.set_meter_change(3, 7).unwrap();
        project.set_meter_change(5, 6).unwrap();
        assert_eq!(
            project.meter_changes,
            [
                MeterChange {
                    bar: 3,
                    beats_per_bar: 7
                },
                MeterChange {
                    bar: 5,
                    beats_per_bar: 6
                }
            ]
        );
        assert!(project.set_meter_change(1, 3).is_err());
        assert!(project.set_meter_change(4, 0).is_err());
        assert!(project.set_meter_change(4, 33).is_err());
        let before = project.meter_changes.clone();
        assert!(project.set_meter_change(u32::MAX, 3).is_err());
        assert!(project.move_meter_change(3, u32::MAX).is_err());
        assert_eq!(project.meter_changes, before);
        assert!(project.move_meter_change(3, 5).is_err());
        assert!(project.move_meter_change(3, 1).is_err());
        assert!(project.move_meter_change(9, 10).is_err());
        project.move_meter_change(3, 8).unwrap();
        assert_eq!(
            project.meter_changes[1],
            MeterChange {
                bar: 8,
                beats_per_bar: 7
            }
        );
        // 4/4 for bars 1–4, 6/4 for bars 5–7, then 7/4.
        assert_eq!(project.meter_map().bar_start(8), 34.0);
        project.remove_meter_change(5).unwrap();
        assert!(project.remove_meter_change(5).is_err());
        assert_eq!(project.meter_changes.len(), 1);
        project.validate().unwrap();
    }

    #[test]
    fn markers_keep_beat_order_and_reject_invalid_values() {
        let mut project = Project::new();
        let chorus = project.add_marker(16.0, "Chorus", Some([1, 2, 3])).unwrap();
        let verse = project.add_marker(4.0, "Verse", None).unwrap();
        assert_eq!(
            project.markers.iter().map(|m| m.id).collect::<Vec<_>>(),
            [verse, chorus]
        );
        project.move_marker(verse, 32.0).unwrap();
        assert_eq!(project.markers[1].id, verse);
        project.rename_marker(verse, "Bridge").unwrap();
        assert_eq!(project.marker(verse).unwrap().name, "Bridge");
        assert!(project.rename_marker(verse, " ").is_err());
        assert!(project.rename_marker(verse, &"x".repeat(257)).is_err());
        assert!(project.move_marker(verse, -1.0).is_err());
        assert!(project.move_marker(verse, f64::NAN).is_err());
        assert!(project.add_marker(MAX_BEAT + 1.0, "Late", None).is_err());
        assert_eq!(project.adjacent_marker(16.0, true).unwrap().id, verse);
        assert_eq!(project.adjacent_marker(16.0, false), None);
        assert_eq!(project.adjacent_marker(20.0, false).unwrap().id, chorus);
        project.validate().unwrap();
        project.remove_marker(chorus).unwrap();
        assert!(project.remove_marker(chorus).is_err());
        assert_eq!(project.markers.len(), 1);
    }

    #[test]
    fn old_projects_load_without_meter_changes_or_markers() {
        let mut value = serde_json::to_value(Project::new()).unwrap();
        assert!(value.get("meter_changes").is_none() && value.get("markers").is_none());
        value["beats_per_bar"] = 3.into();
        let project: Project = serde_json::from_value(value).unwrap();
        assert!(project.meter_changes.is_empty() && project.markers.is_empty());
        assert_eq!(project.meter_map().position(7.0), (3, 1.0));
    }

    #[test]
    fn duplicate_marker_ids_are_rejected() {
        let mut project = Project::new();
        project.add_marker(0.0, "Intro", None).unwrap();
        project.validate().unwrap();
        project.markers.push(project.markers[0].clone());
        assert!(project.validate().is_err());
        project.markers.pop();
        project.markers[0].id = project.add_midi_track("Keys");
        assert!(project.validate().is_err());
    }
}
