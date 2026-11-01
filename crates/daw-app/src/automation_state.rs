//! Automation ownership and manual takeover, shared by every parameter surface.
use crate::app::DawApp;
use daw_core::{AutomationLane, AutomationParameter, AutomationTarget, Project};

pub(crate) fn lane_enabled(project: &Project, lane: u64) -> bool {
    let Some(track) = project.track(lane) else {
        return false;
    };
    if track.mute {
        return false;
    }
    let mut parent = track.parent_folder;
    // Valid projects have acyclic folders; retain a bound for incomplete edits.
    for _ in 0..project.tracks.len() {
        let Some(folder) = parent.and_then(|id| project.track(id)) else {
            return true;
        };
        if folder.mute {
            return false;
        }
        parent = folder.parent_folder;
    }
    parent.is_none()
}

fn manual_value(project: &Project, target: AutomationTarget) -> Option<f32> {
    let track = project.track(target.track)?;
    let value = match target.parameter {
        AutomationParameter::TrackGain => track.gain,
        AutomationParameter::TrackPan => track.pan,
        AutomationParameter::SendGain { bus } => {
            track.sends.iter().find(|send| send.target_bus == bus)?.gain
        }
        AutomationParameter::VstParameter { effect, id } => {
            let parameters = if let Some(effect) = effect {
                &track
                    .inserts
                    .iter()
                    .find(|slot| slot.id == effect)?
                    .parameters
            } else {
                &track.instrument_parameters
            };
            parameters
                .iter()
                .find(|parameter| parameter.id == id)?
                .value as f32
        }
    };
    value.is_finite().then(|| target.parameter.normalize(value))
}

/// Take over active lanes whose stored parameter value changed. Same-value
/// commands have no inferred gesture; callers can set an override explicitly.
/// Explicit override/resume commands retain their atomic intent even when the
/// requested override equals the before-state (notably Resume from None).
pub(crate) fn capture_manual_parameter_changes(
    before: &Project,
    candidate: &mut Project,
    explicit_lanes: &[u64],
) {
    let changes: Vec<_> = candidate
        .tracks
        .iter()
        .filter_map(|track| {
            let lane = track.automation.as_ref()?;
            let old_lane = before.automation_lane(track.id)?;
            if explicit_lanes.contains(&track.id)
                || lane.target != old_lane.target
                || lane.override_value != old_lane.override_value
                || (lane.points.is_empty()
                    && lane.clips.iter().all(|clip| clip.points.is_empty())
                    && lane.override_value.is_none())
                || !lane_enabled(candidate, track.id)
            {
                return None;
            }
            let value = manual_value(candidate, lane.target)?;
            (manual_value(before, lane.target) != Some(value)).then_some((track.id, value))
        })
        .collect();
    for (lane, value) in changes {
        let _ = candidate.set_automation_override(lane, Some(value));
    }
}

impl DawApp {
    /// Status and sounding value at the current musical position. Reading this
    /// never writes fader positions back into the saved manual parameter.
    pub(crate) fn automation_control_state(
        &self,
        target: AutomationTarget,
    ) -> Option<(u64, daw_ui::AutomationState, f32)> {
        let (id, lane) = self.project.automation_for_target(target)?;
        Some(self.automation_lane_control_state(id, lane))
    }

    /// Evaluate an already located lane without searching the project again.
    /// Bulk parameter views should walk the lanes once, not search all tracks
    /// for every parameter exposed by a plug-in.
    pub(crate) fn automation_lane_control_state(
        &self,
        id: u64,
        lane: &AutomationLane,
    ) -> (u64, daw_ui::AutomationState, f32) {
        let enabled = lane_enabled(&self.project, id)
            && (!lane.points.is_empty()
                || lane.clips.iter().any(|clip| !clip.points.is_empty())
                || lane.override_value.is_some());
        let state = if !enabled {
            daw_ui::AutomationState::Bypassed
        } else if lane.override_value.is_some() {
            daw_ui::AutomationState::Overridden
        } else {
            daw_ui::AutomationState::Read
        };
        let value = if enabled {
            lane.effective_value_at(self.cursor)
        } else {
            manual_value(&self.project, lane.target)
        }
        .or_else(|| manual_value(&self.project, lane.target))
        .unwrap_or(0.5);
        (id, state, value)
    }

    /// Record a real manual gesture in the same transaction as its parameter.
    /// Explicit event handling also catches a drag back to the stored manual
    /// value, which a project-diff fallback alone cannot distinguish.
    pub(crate) fn override_automated_parameter(
        &mut self,
        target: AutomationTarget,
        value: f32,
    ) -> bool {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return false;
        }
        let Some((id, lane)) = self.project.automation_for_target(target) else {
            return false;
        };
        if (lane.points.is_empty()
            && lane.clips.iter().all(|clip| clip.points.is_empty())
            && lane.override_value.is_none())
            || !lane_enabled(&self.project, id)
            || lane.override_value == Some(value)
        {
            return false;
        }
        self.project
            .set_automation_override(id, Some(value))
            .is_ok()
    }

    pub(crate) fn resume_automation(&mut self, lane: u64) {
        if self
            .project
            .automation_lane(lane)
            .is_none_or(|lane| lane.override_value.is_none())
        {
            return;
        }
        self.begin_edit();
        if self.project.set_automation_override(lane, None).is_ok() {
            self.edit_label = Some("Resume automation".into());
            self.changed();
        }
    }

    /// Fallback for stored parameter changes within a pending UI edit. Direct
    /// controls report their gestures explicitly, including same-value drags.
    pub(crate) fn capture_manual_automation_overrides(&mut self) {
        if let Some(before) = &self.before_edit {
            capture_manual_parameter_changes(before, &mut self.project, &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::AutomationPoint;
    use eframe::egui;

    fn fixture() -> (DawApp, AutomationTarget, u64) {
        let mut project = Project::new();
        let track = project.add_audio_track("Automated channel");
        project.track_mut(track).unwrap().gain = 0.9;
        let target = AutomationTarget {
            track,
            parameter: AutomationParameter::TrackGain,
        };
        let lane = project.add_automation_track("Volume", target).unwrap();
        project.automation_lane_mut(lane).unwrap().points = vec![
            AutomationPoint::new(8.0, 0.2),
            AutomationPoint::new(16.0, 0.8),
        ];
        (
            DawApp::with_context(&egui::Context::default(), project, None),
            target,
            lane,
        )
    }

    #[test]
    fn stopped_controls_read_held_endpoints_without_editing_the_manual_value() {
        let (mut app, target, _) = fixture();
        let before = app.project.clone();
        for (beat, expected) in [(0.0, 0.2), (4.0, 0.2), (16.0, 0.8), (10000.0, 0.8)] {
            app.cursor = beat;
            let (_, state, value) = app.automation_control_state(target).unwrap();
            assert_eq!(state, daw_ui::AutomationState::Read);
            assert_eq!(value, expected);
        }
        assert_eq!(app.project, before);
    }

    #[test]
    fn explicit_takeover_tracks_a_drag_back_to_the_stored_value_and_resume_keeps_points() {
        let (mut app, target, lane) = fixture();
        let points = app.project.automation_lane(lane).unwrap().points.clone();
        app.begin_edit();
        for value in [0.4, 0.7, 0.6] {
            app.project.track_mut(target.track).unwrap().gain = target.parameter.denormalize(value);
            app.override_automated_parameter(target, value);
            app.changed();
            assert_eq!(app.automation_control_state(target).unwrap().2, value);
        }
        app.flush_pending_edit();
        app.resume_automation(lane);
        assert_eq!(
            app.project.automation_lane(lane).unwrap().override_value,
            None
        );
        assert_eq!(app.project.automation_lane(lane).unwrap().points, points);
        assert_eq!(app.automation_control_state(target).unwrap().2, 0.2);
    }

    #[test]
    fn pending_parameter_changes_override_but_curve_and_unrelated_edits_do_not() {
        let (mut app, target, lane) = fixture();
        app.begin_edit();
        app.project.track_mut(target.track).unwrap().pan = 0.5;
        app.changed();
        assert_eq!(
            app.project.automation_lane(lane).unwrap().override_value,
            None
        );
        app.project.track_mut(target.track).unwrap().gain = 0.3;
        app.changed();
        assert_eq!(
            app.project.automation_lane(lane).unwrap().override_value,
            Some(0.2)
        );
        app.flush_pending_edit();
        app.resume_automation(lane);
        app.flush_pending_edit();
        app.begin_edit();
        app.project.automation_lane_mut(lane).unwrap().points[0].value = 0.7;
        app.changed();
        assert_eq!(
            app.project.automation_lane(lane).unwrap().override_value,
            None
        );
    }

    #[test]
    fn bypass_and_empty_overrides_report_the_value_the_renderer_uses() {
        let (mut app, target, lane) = fixture();
        app.project
            .set_automation_override(lane, Some(0.4))
            .unwrap();
        app.project
            .automation_lane_mut(lane)
            .unwrap()
            .points
            .clear();
        assert_eq!(
            app.automation_control_state(target).unwrap().1,
            daw_ui::AutomationState::Overridden
        );
        assert_eq!(app.automation_control_state(target).unwrap().2, 0.4);
        assert!(app.override_automated_parameter(target, 0.7));
        assert_eq!(app.automation_control_state(target).unwrap().2, 0.7);
        app.begin_edit();
        app.project.track_mut(target.track).unwrap().gain = 0.3;
        app.changed();
        assert_eq!(app.automation_control_state(target).unwrap().2, 0.2);
        app.flush_pending_edit();
        app.project.track_mut(lane).unwrap().mute = true;
        assert_eq!(
            app.automation_control_state(target).unwrap().1,
            daw_ui::AutomationState::Bypassed
        );
        assert!((app.automation_control_state(target).unwrap().2 - 0.2).abs() < 1e-6);
        assert!(!app.override_automated_parameter(target, 0.1));
        assert_eq!(
            app.project.automation_lane(lane).unwrap().override_value,
            Some(0.2)
        );
    }
}
