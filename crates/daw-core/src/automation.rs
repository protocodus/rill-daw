//! Parameter automation in arrangement beats. Curves contain normalized values;
//! stable track/effect/bus IDs keep targets independent of channel ordering.

use crate::{Effect, EffectSlot, Instrument, MAX_BEAT, Project, Send, Track, TrackKind, new_id};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, ops::RangeInclusive};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutomationParameter {
    TrackGain,
    TrackPan,
    SendGain { bus: u64 },
    InsertGain { effect: u64 },
    DelayTime { effect: u64 },
    DelayFeedback { effect: u64 },
    DelayMix { effect: u64 },
    VstParameter { effect: Option<u64>, id: u32 },
}

impl AutomationParameter {
    pub fn name(self) -> &'static str {
        match self {
            Self::TrackGain => "Track gain",
            Self::TrackPan => "Track pan",
            Self::SendGain { .. } => "Send gain",
            Self::InsertGain { .. } => "Insert gain",
            Self::DelayTime { .. } => "Delay time",
            Self::DelayFeedback { .. } => "Delay feedback",
            Self::DelayMix { .. } => "Delay mix",
            Self::VstParameter { .. } => "VST parameter",
        }
    }

    pub fn range(self) -> RangeInclusive<f32> {
        match self {
            Self::TrackGain => 0.0..=1.5,
            Self::TrackPan => -1.0..=1.0,
            Self::SendGain { .. } | Self::InsertGain { .. } => 0.0..=16.0,
            Self::DelayTime { .. } => 1.0..=10_000.0,
            Self::DelayFeedback { .. } => -0.99..=0.99,
            Self::DelayMix { .. } | Self::VstParameter { .. } => 0.0..=1.0,
        }
    }

    /// Convert a parameter value to normalized curve coordinates, clamping the
    /// finite input to this parameter's supported automation range.
    pub fn normalize(self, value: f32) -> f32 {
        let range = self.range();
        ((value - range.start()) / (range.end() - range.start())).clamp(0.0, 1.0)
    }

    pub fn denormalize(self, normalized: f32) -> f32 {
        let range = self.range();
        range.start() + normalized.clamp(0.0, 1.0) * (range.end() - range.start())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AutomationTarget {
    pub track: u64,
    pub parameter: AutomationParameter,
}

impl AutomationTarget {
    pub fn validate(self, project: &Project) -> Result<()> {
        let track = project
            .track(self.track)
            .context("Automation target track is missing")?;
        ensure!(
            matches!(
                track.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            ),
            "Automation targets must be audio, MIDI, or bus tracks"
        );
        let effect = |id| {
            track
                .inserts
                .iter()
                .find(|slot| slot.id == id)
                .map(|slot| &slot.effect)
                .context("Automation target insert is missing")
        };
        match self.parameter {
            AutomationParameter::TrackGain | AutomationParameter::TrackPan => {}
            AutomationParameter::SendGain { bus } => {
                ensure!(
                    track.sends.iter().any(|send| send.target_bus == bus),
                    "Automation target send is missing"
                );
                ensure!(
                    project
                        .track(bus)
                        .is_some_and(|track| track.kind == TrackKind::Bus),
                    "Automation send destination is not a bus"
                );
            }
            AutomationParameter::InsertGain { effect: id } => ensure!(
                matches!(effect(id)?, Effect::Gain { .. }),
                "Automation target is not a gain insert"
            ),
            AutomationParameter::DelayTime { effect: id }
            | AutomationParameter::DelayFeedback { effect: id }
            | AutomationParameter::DelayMix { effect: id } => ensure!(
                matches!(effect(id)?, Effect::Delay { .. }),
                "Automation target is not a delay insert"
            ),
            AutomationParameter::VstParameter {
                effect: Some(id), ..
            } => ensure!(
                matches!(effect(id)?, Effect::Vst3 { .. }),
                "Automation target is not a VST3 insert"
            ),
            AutomationParameter::VstParameter { effect: None, .. } => ensure!(
                track.kind == TrackKind::Midi
                    && matches!(track.instrument, Instrument::Vst3 { .. }),
                "Automation target is not a VST3 instrument"
            ),
        }
        Ok(())
    }

    /// Current normalized native value. VST state belongs to the isolated host;
    /// callers must supply its explicit initial normalized value instead.
    pub fn current_value(self, project: &Project) -> Result<f32> {
        self.validate(project)?;
        let track = project.track(self.track).expect("validated track");
        let effect = |id| {
            &track
                .inserts
                .iter()
                .find(|slot| slot.id == id)
                .expect("validated insert")
                .effect
        };
        let value = match self.parameter {
            AutomationParameter::TrackGain => track.gain,
            AutomationParameter::TrackPan => track.pan,
            AutomationParameter::SendGain { bus } => {
                track
                    .sends
                    .iter()
                    .find(|send| send.target_bus == bus)
                    .expect("validated send")
                    .gain
            }
            AutomationParameter::InsertGain { effect: id } => match effect(id) {
                Effect::Gain { gain } => *gain,
                _ => unreachable!(),
            },
            AutomationParameter::DelayTime { effect: id } => match effect(id) {
                Effect::Delay { time_ms, .. } => *time_ms,
                _ => unreachable!(),
            },
            AutomationParameter::DelayFeedback { effect: id } => match effect(id) {
                Effect::Delay { feedback, .. } => *feedback,
                _ => unreachable!(),
            },
            AutomationParameter::DelayMix { effect: id } => match effect(id) {
                Effect::Delay { mix, .. } => *mix,
                _ => unreachable!(),
            },
            AutomationParameter::VstParameter { .. } => {
                bail!("Provide an explicit initial normalized VST parameter value")
            }
        };
        ensure!(
            value.is_finite() && self.parameter.range().contains(&value),
            "Current parameter value lies outside the automation range"
        );
        Ok(self.parameter.normalize(value))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    Step,
    #[default]
    Linear,
    Smooth,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub id: u64,
    pub beat: f64,
    pub value: f32,
}

impl AutomationPoint {
    pub fn new(beat: f64, value: f32) -> Self {
        Self {
            id: new_id(),
            beat,
            value,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    pub points: Vec<AutomationPoint>,
    pub interpolation: Interpolation,
}

impl AutomationLane {
    /// Empty lanes do not override the manual parameter. Otherwise endpoints
    /// hold indefinitely. Evaluation uses binary search and never allocates.
    pub fn value_at(&self, beat: f64) -> Option<f32> {
        if beat.is_nan() {
            return None;
        }
        let first = self.points.first()?;
        if beat <= first.beat {
            return Some(first.value);
        }
        let index = self.points.partition_point(|point| point.beat <= beat);
        if index == self.points.len() {
            return self.points.last().map(|point| point.value);
        }
        let a = &self.points[index - 1];
        let b = &self.points[index];
        if self.interpolation == Interpolation::Step {
            return Some(a.value);
        }
        let width = b.beat - a.beat;
        let t = (beat - a.beat) / width;
        let linear = f64::from(a.value) + t * (f64::from(b.value) - f64::from(a.value));
        if self.interpolation == Interpolation::Linear {
            return Some(linear as f32);
        }
        let start_slope = self.slope(index - 1);
        let end_slope = self.slope(index);
        let t2 = t * t;
        let t3 = t2 * t;
        let smooth = (2.0 * t3 - 3.0 * t2 + 1.0) * f64::from(a.value)
            + (t3 - 2.0 * t2 + t) * width * start_slope
            + (-2.0 * t3 + 3.0 * t2) * f64::from(b.value)
            + (t3 - t2) * width * end_slope;
        // Extremely close floating-point beat positions can overflow a slope.
        // Linear fallback preserves bounded interpolation for those inputs.
        let value = if smooth.is_finite() { smooth } else { linear } as f32;
        Some(value.clamp(a.value.min(b.value), a.value.max(b.value)))
    }

    fn slope(&self, index: usize) -> f64 {
        let interval = |left: usize| {
            let a = &self.points[left];
            let b = &self.points[left + 1];
            let width = b.beat - a.beat;
            (width, (f64::from(b.value) - f64::from(a.value)) / width)
        };
        if self.points.len() == 2 {
            return interval(0).1;
        }
        if index == 0 {
            let (h0, d0) = interval(0);
            let (h1, d1) = interval(1);
            return endpoint_slope(h0, h1, d0, d1);
        }
        if index + 1 == self.points.len() {
            let (h0, d0) = interval(index - 1);
            let (h1, d1) = interval(index - 2);
            return endpoint_slope(h0, h1, d0, d1);
        }
        let (before, left) = interval(index - 1);
        let (after, right) = interval(index);
        if left == 0.0 || right == 0.0 || left.signum() != right.signum() {
            return 0.0;
        }
        let w1 = 2.0 * after + before;
        let w2 = after + 2.0 * before;
        (w1 + w2) / (w1 / left + w2 / right)
    }

    pub fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        let mut previous = None;
        for point in &self.points {
            validate_position(point.beat, point.value)?;
            ensure!(
                point.id > 0 && point.id < u64::MAX && ids.insert(point.id),
                "Duplicate or invalid automation point ID"
            );
            ensure!(
                previous.is_none_or(|beat| point.beat > beat),
                "Automation points must have strictly increasing beat positions"
            );
            previous = Some(point.beat);
        }
        Ok(())
    }

    /// Exact-beat insertion replaces the existing value and preserves its ID.
    pub fn upsert_point(&mut self, beat: f64, value: f32) -> Result<u64> {
        validate_position(beat, value)?;
        self.validate()?;
        match self.points.binary_search_by(|point| {
            point
                .beat
                .partial_cmp(&beat)
                .expect("validated finite beats")
        }) {
            Ok(index) => {
                self.points[index].value = value;
                Ok(self.points[index].id)
            }
            Err(index) => {
                let point = AutomationPoint::new(beat, value);
                let id = point.id;
                self.points.insert(index, point);
                Ok(id)
            }
        }
    }

    pub fn move_point(&mut self, id: u64, beat: f64, value: f32) -> Result<()> {
        validate_position(beat, value)?;
        self.validate()?;
        let index = self
            .points
            .iter()
            .position(|point| point.id == id)
            .context("Automation point not found")?;
        ensure!(
            !self
                .points
                .iter()
                .any(|point| point.id != id && point.beat == beat),
            "Another automation point already occupies this beat"
        );
        let mut point = self.points.remove(index);
        point.beat = beat;
        point.value = value;
        let insert = self.points.partition_point(|other| other.beat < beat);
        self.points.insert(insert, point);
        Ok(())
    }

    pub fn remove_point(&mut self, id: u64) -> Result<AutomationPoint> {
        let index = self
            .points
            .iter()
            .position(|point| point.id == id)
            .context("Automation point not found")?;
        Ok(self.points.remove(index))
    }
}

fn endpoint_slope(h0: f64, h1: f64, d0: f64, d1: f64) -> f64 {
    let slope = ((2.0 * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
    if slope.signum() != d0.signum() || d0 == 0.0 {
        0.0
    } else if d0.signum() != d1.signum() && slope.abs() > 3.0 * d0.abs() {
        3.0 * d0
    } else {
        slope
    }
}

fn validate_position(beat: f64, value: f32) -> Result<()> {
    ensure!(
        beat.is_finite() && (0.0..=MAX_BEAT).contains(&beat),
        "Automation beat is outside the supported timeline"
    );
    ensure!(
        value.is_finite() && (0.0..=1.0).contains(&value),
        "Automation value must be normalized between 0 and 1"
    );
    Ok(())
}

impl Project {
    /// Replacing an instrument invalidates its VST parameter IDs even when both
    /// old and new instruments are VST3. Insert and native automation survive.
    pub fn set_instrument(&mut self, track: u64, instrument: Instrument) -> Result<()> {
        let mut candidate = self.clone();
        let channel = candidate.track_mut(track).context("Track not found")?;
        ensure!(
            channel.kind == TrackKind::Midi,
            "Only MIDI tracks host instruments"
        );
        if channel.instrument != instrument {
            channel.instrument = instrument;
            candidate.tracks.retain(|lane_track| {
                !lane_track.automation.as_ref().is_some_and(|lane| {
                    lane.target.track == track
                        && matches!(
                            lane.target.parameter,
                            AutomationParameter::VstParameter { effect: None, .. }
                        )
                })
            });
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn add_automation_track(
        &mut self,
        name: impl Into<String>,
        target: AutomationTarget,
    ) -> Result<u64> {
        let value = target.current_value(self)?;
        self.add_automation_track_with_value(name, target, value)
    }

    pub fn add_automation_track_with_value(
        &mut self,
        name: impl Into<String>,
        target: AutomationTarget,
        value: f32,
    ) -> Result<u64> {
        target.validate(self)?;
        validate_position(0.0, value)?;
        ensure!(
            !self
                .tracks
                .iter()
                .filter_map(|track| track.automation.as_ref())
                .any(|lane| lane.target == target),
            "A track already automates this parameter"
        );
        let mut track = Track::new(name, TrackKind::Automation);
        track.automation = Some(AutomationLane {
            target,
            points: vec![
                AutomationPoint::new(0.0, value),
                AutomationPoint::new(4.0, value),
            ],
            interpolation: Interpolation::Linear,
        });
        let id = track.id;
        self.tracks.push(track);
        Ok(id)
    }

    pub fn automation_lane(&self, track: u64) -> Option<&AutomationLane> {
        self.track(track)
            .filter(|track| track.kind == TrackKind::Automation)?
            .automation
            .as_ref()
    }

    pub fn automation_lane_mut(&mut self, track: u64) -> Option<&mut AutomationLane> {
        self.track_mut(track)
            .filter(|track| track.kind == TrackKind::Automation)?
            .automation
            .as_mut()
    }

    pub fn upsert_automation_point(&mut self, track: u64, beat: f64, value: f32) -> Result<u64> {
        self.automation_lane_mut(track)
            .context("Automation track not found")?
            .upsert_point(beat, value)
    }

    pub fn move_automation_point(
        &mut self,
        track: u64,
        point: u64,
        beat: f64,
        value: f32,
    ) -> Result<()> {
        self.automation_lane_mut(track)
            .context("Automation track not found")?
            .move_point(point, beat, value)
    }

    pub fn remove_automation_point(&mut self, track: u64, point: u64) -> Result<AutomationPoint> {
        self.automation_lane_mut(track)
            .context("Automation track not found")?
            .remove_point(point)
    }

    /// Remove lanes whose parameter no longer exists after a channel, send,
    /// insert, or instrument change. Point errors remain validation errors.
    pub fn prune_automation(&mut self) -> usize {
        let removed: HashSet<_> = self
            .tracks
            .iter()
            .filter(|track| {
                track
                    .automation
                    .as_ref()
                    .is_some_and(|lane| lane.target.validate(self).is_err())
            })
            .map(|track| track.id)
            .collect();
        self.tracks.retain(|track| !removed.contains(&track.id));
        removed.len()
    }

    pub fn remove_insert(&mut self, track: u64, effect: u64) -> Result<EffectSlot> {
        let mut candidate = self.clone();
        let channel = candidate.track_mut(track).context("Track not found")?;
        let index = channel
            .inserts
            .iter()
            .position(|slot| slot.id == effect)
            .context("Insert not found")?;
        let removed = channel.inserts.remove(index);
        candidate.prune_automation();
        candidate.validate()?;
        *self = candidate;
        Ok(removed)
    }

    pub fn remove_send(&mut self, track: u64, bus: u64) -> Result<Send> {
        let mut candidate = self.clone();
        let channel = candidate.track_mut(track).context("Track not found")?;
        let index = channel
            .sends
            .iter()
            .position(|send| send.target_bus == bus)
            .context("Send not found")?;
        let removed = channel.sends.remove(index);
        candidate.prune_automation();
        candidate.validate()?;
        *self = candidate;
        Ok(removed)
    }
}
