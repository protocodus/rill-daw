//! Editing reusable automation patterns independently of their placements.
use crate::{
    AutomationClip, AutomationLane, AutomationPoint, Interpolation, MAX_BEAT, Project, new_id,
};
use anyhow::{Context, Result, ensure};

impl Project {
    pub fn create_automation_clip(&mut self, track: u64, start: f64, length: f64) -> Result<u64> {
        ensure!(
            start.is_finite()
                && start >= 0.0
                && length.is_finite()
                && length > 0.0
                && start + length <= MAX_BEAT,
            "Invalid automation clip bounds"
        );
        let lane = self
            .automation_lane(track)
            .context("Automation track not found")?;
        let fallback = lane.target.current_value(self).unwrap_or(0.5);
        let mut points = vec![AutomationPoint::new(
            0.0,
            lane.arrangement_value_at(start).unwrap_or(fallback),
        )];
        points.extend(
            lane.points
                .iter()
                .filter(|p| p.beat > start && p.beat < start + length)
                .map(|p| {
                    let mut point = AutomationPoint::new(p.beat - start, p.value);
                    point.interpolation = p.interpolation;
                    point.curve = p.curve;
                    point
                }),
        );
        points[0].interpolation = lane
            .points
            .iter()
            .rev()
            .find(|p| p.beat <= start)
            .and_then(|p| p.interpolation);
        points.push(AutomationPoint::new(
            length,
            lane.arrangement_value_at(start + length)
                .unwrap_or(fallback),
        ));
        let clip = AutomationClip {
            id: new_id(),
            name: "Automation pattern".into(),
            start,
            length,
            offset: 0.0,
            link_group: None,
            points,
            interpolation: lane.interpolation,
        };
        let id = clip.id;
        self.automation_lane_mut(track).unwrap().clips.push(clip);
        Ok(id)
    }

    pub fn copy_automation_clip(
        &mut self,
        track: u64,
        clip: u64,
        start: f64,
        linked: bool,
    ) -> Result<u64> {
        let lane = self
            .automation_lane_mut(track)
            .context("Automation track not found")?;
        let original = lane
            .clips
            .iter_mut()
            .find(|pattern| pattern.id == clip)
            .context("Automation clip not found")?;
        ensure!(
            start.is_finite() && start >= 0.0 && start + original.length <= MAX_BEAT,
            "Invalid automation clip position"
        );
        if linked && original.link_group.is_none() {
            original.link_group = Some(new_id());
        }
        let mut copy = original.clone();
        copy.id = new_id();
        copy.start = start;
        if !linked {
            copy.link_group = None;
        }
        for point in &mut copy.points {
            point.id = new_id();
        }
        let id = copy.id;
        lane.clips.push(copy);
        Ok(id)
    }

    pub fn paste_automation_clip(
        &mut self,
        track: u64,
        original: &AutomationClip,
        start: f64,
    ) -> Result<u64> {
        ensure!(
            start.is_finite() && start >= 0.0 && start + original.length <= MAX_BEAT,
            "Invalid automation clip position"
        );
        let mut copy = original.clone();
        copy.id = new_id();
        copy.start = start;
        copy.link_group = None;
        for point in &mut copy.points {
            point.id = new_id();
        }
        let id = copy.id;
        let lane = self
            .automation_lane_mut(track)
            .context("Automation track not found")?;
        lane.clips.push(copy);
        if let Err(error) = lane.validate() {
            lane.clips.pop();
            return Err(error);
        }
        Ok(id)
    }

    pub fn set_automation_clip_bounds(
        &mut self,
        track: u64,
        id: u64,
        start: f64,
        length: f64,
        offset: f64,
    ) -> Result<()> {
        ensure!(
            start.is_finite()
                && start >= 0.0
                && length.is_finite()
                && length > 0.0
                && start + length <= MAX_BEAT
                && offset.is_finite()
                && offset >= 0.0
                && offset + length <= MAX_BEAT,
            "Invalid automation clip bounds"
        );
        let clip = self
            .automation_lane_mut(track)
            .context("Automation track not found")?
            .clips
            .iter_mut()
            .find(|clip| clip.id == id)
            .context("Automation clip not found")?;
        clip.start = start;
        clip.length = length;
        clip.offset = offset;
        Ok(())
    }

    pub fn remove_automation_clip(&mut self, track: u64, id: u64) -> Result<()> {
        let clips = &mut self
            .automation_lane_mut(track)
            .context("Automation track not found")?
            .clips;
        let index = clips
            .iter()
            .position(|clip| clip.id == id)
            .context("Automation clip not found")?;
        clips.remove(index);
        Ok(())
    }

    pub fn make_automation_clip_independent(&mut self, track: u64, id: u64) -> Result<()> {
        self.automation_lane_mut(track)
            .context("Automation track not found")?
            .clips
            .iter_mut()
            .find(|clip| clip.id == id)
            .context("Automation clip not found")?
            .link_group = None;
        Ok(())
    }

    /// Ghost edits propagate source data while preserving every placement.
    pub fn edit_automation_pattern<T>(
        &mut self,
        track: u64,
        clip: Option<u64>,
        edit: impl FnOnce(&mut AutomationLane) -> Result<T>,
    ) -> Result<T> {
        let lane = self
            .automation_lane_mut(track)
            .context("Automation track not found")?;
        let Some(id) = clip else {
            return edit(lane);
        };
        let original = lane
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .context("Automation clip not found")?
            .clone();
        let mut source = AutomationLane {
            override_value: None,
            target: lane.target,
            points: original.points.clone(),
            interpolation: original.interpolation,
            clips: Vec::new(),
        };
        let result = edit(&mut source)?;
        source.validate()?;
        if source.points == original.points && source.interpolation == original.interpolation {
            return Ok(result);
        }
        let original_indices: std::collections::HashMap<_, _> = original
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| (point.id, index))
            .collect();
        for pattern in &mut lane.clips {
            if pattern.id == id
                || original.link_group.is_some() && pattern.link_group == original.link_group
            {
                if pattern.id != id {
                    let previous_ids: Vec<_> =
                        pattern.points.iter().map(|point| point.id).collect();
                    pattern.points.clone_from(&source.points);
                    for point in &mut pattern.points {
                        point.id = original_indices
                            .get(&point.id)
                            .and_then(|index| previous_ids.get(*index))
                            .copied()
                            .unwrap_or_else(new_id);
                    }
                } else {
                    pattern.points.clone_from(&source.points);
                }
                pattern.interpolation = source.interpolation;
            }
        }
        Ok(result)
    }

    pub fn set_automation_point_interpolation(
        &mut self,
        track: u64,
        clip: Option<u64>,
        id: u64,
        interpolation: Interpolation,
    ) -> Result<()> {
        self.edit_automation_pattern(track, clip, |lane| {
            lane.points
                .iter_mut()
                .find(|point| point.id == id)
                .context("Automation point not found")?
                .interpolation = Some(interpolation);
            Ok(())
        })
    }
}
