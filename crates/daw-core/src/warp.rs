//! Editable warp maps for audio clips and grid quantization of detected hits.
//!
//! A warp marker pins a source position to a forward clip-local beat. Between
//! neighbouring markers time stretches linearly; before the first and after the
//! last marker the clip's `source_rate` applies. Beats are forward positions:
//! a reversed clip plays the same map mirrored inside its bounds, exactly as it
//! mirrors an unwarped source window. Markers outside the clip bounds are kept
//! so trimming back out restores the same timing.
use crate::{Clip, ClipContent, MAX_BEAT, MAX_SOURCE_RATE, MIN_SOURCE_RATE, MeterMap, Project};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_WARP_MARKERS: usize = 2048;
/// Neighbouring markers must be at least this far apart in both domains.
const MIN_WARP_SPACING: f64 = 1e-6;
// Segment rates come from user drags and floating-point division; accept the
// same range the engine admits with a relative tolerance far below audibility.
const RATE_TOLERANCE: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpMarker {
    /// Source beat at project tempo, the same unit as `Clip::offset`.
    pub source: f64,
    /// Forward clip-local beat where that source position plays.
    pub beat: f64,
}

pub fn validate_warp_markers(markers: &[WarpMarker]) -> Result<()> {
    ensure!(
        markers.len() <= MAX_WARP_MARKERS,
        "A clip can have at most {MAX_WARP_MARKERS} warp markers"
    );
    for marker in markers {
        ensure!(
            marker.source.is_finite()
                && marker.beat.is_finite()
                && (0.0..=MAX_BEAT).contains(&marker.source)
                && marker.beat.abs() <= MAX_BEAT,
            "Warp markers need finite source and beat positions"
        );
    }
    for pair in markers.windows(2) {
        let source = pair[1].source - pair[0].source;
        let beats = pair[1].beat - pair[0].beat;
        ensure!(
            source >= MIN_WARP_SPACING && beats >= MIN_WARP_SPACING,
            "Warp markers must be in increasing source and beat order"
        );
        ensure!(
            segment_rate_allowed(source / beats),
            "Warp markers must keep each segment between 25% and 400% of source speed"
        );
    }
    Ok(())
}

fn segment_rate_allowed(rate: f64) -> bool {
    (MIN_SOURCE_RATE * (1.0 - RATE_TOLERANCE)..=MAX_SOURCE_RATE * (1.0 + RATE_TOLERANCE))
        .contains(&rate)
}

/// Source beat at a forward clip-local beat through a non-empty marker list.
fn source_at(markers: &[WarpMarker], rate: f64, beat: f64) -> f64 {
    let index = markers.partition_point(|marker| marker.beat <= beat);
    if index == 0 {
        let first = markers[0];
        return first.source + (beat - first.beat) * rate;
    }
    let left = markers[index - 1];
    let Some(right) = markers.get(index) else {
        return left.source + (beat - left.beat) * rate;
    };
    left.source + (beat - left.beat) * (right.source - left.source) / (right.beat - left.beat)
}

fn beat_at(markers: &[WarpMarker], rate: f64, source: f64) -> f64 {
    let index = markers.partition_point(|marker| marker.source <= source);
    if index == 0 {
        let first = markers[0];
        return first.beat + (source - first.source) / rate;
    }
    let left = markers[index - 1];
    let Some(right) = markers.get(index) else {
        return left.beat + (source - left.source) / rate;
    };
    left.beat + (source - left.source) * (right.beat - left.beat) / (right.source - left.source)
}

impl Clip {
    pub fn is_warped(&self) -> bool {
        !self.warp_markers.is_empty()
    }

    /// Forward clip-local beat to source beat, honouring warp markers.
    pub fn local_to_source(&self, local_beat: f64) -> f64 {
        if self.warp_markers.is_empty() {
            self.offset + local_beat * self.source_rate
        } else {
            source_at(&self.warp_markers, self.source_rate, local_beat)
        }
    }

    /// Source beat to forward clip-local beat, honouring warp markers.
    pub fn source_to_local(&self, source_beat: f64) -> f64 {
        if self.warp_markers.is_empty() {
            (source_beat - self.offset) / self.source_rate
        } else {
            beat_at(&self.warp_markers, self.source_rate, source_beat)
        }
    }

    /// Exclusive end of the source window the clip plays.
    pub fn source_end(&self) -> f64 {
        if self.warp_markers.is_empty() {
            self.offset + self.source_length()
        } else {
            self.local_to_source(self.length)
        }
    }

    /// Source beats consumed per local beat at a forward local position.
    pub fn local_rate(&self, local_beat: f64) -> f64 {
        let markers = &self.warp_markers;
        let index = markers.partition_point(|marker| marker.beat <= local_beat);
        if index == 0 || index == markers.len() {
            return self.source_rate;
        }
        let (left, right) = (markers[index - 1], markers[index]);
        (right.source - left.source) / (right.beat - left.beat)
    }

    /// `offset` caches the source at local zero so code that only needs the
    /// window start keeps working; markers stay authoritative when present.
    pub(crate) fn sync_warp_offset(&mut self) {
        if !self.warp_markers.is_empty() {
            self.offset = source_at(&self.warp_markers, self.source_rate, 0.0);
        }
    }

    /// Move the forward local origin by `local` beats, keeping every source
    /// position at the same arrangement time. Splits, trims and regions use this.
    pub(crate) fn shift_source_origin(&mut self, local: f64) {
        if self.warp_markers.is_empty() {
            self.offset += local * self.source_rate;
        } else {
            for marker in &mut self.warp_markers {
                marker.beat -= local;
            }
            self.sync_warp_offset();
        }
    }

    /// Stretch keeps relative marker timing, scaled with the clip length.
    pub(crate) fn scale_warp(&mut self, factor: f64) {
        for marker in &mut self.warp_markers {
            marker.beat *= factor;
        }
    }

    /// Slip moves warped content as a unit by `local` forward beats, bounded
    /// by the source file. Returns the applied movement.
    pub(crate) fn slip_warped(&mut self, local: f64, source_beats: f64) -> f64 {
        let earliest = self.length - self.source_to_local(source_beats);
        let latest = -self.source_to_local(0.0);
        let movement = local.clamp(earliest.min(latest), latest);
        for marker in &mut self.warp_markers {
            marker.beat += movement;
        }
        self.sync_warp_offset();
        movement
    }

    /// Playback-ordered source position at an unwarped distance `u` (local
    /// beats at `source_rate`) from where the clip begins playing.
    pub(crate) fn playback_source(&self, u: f64) -> f64 {
        if matches!(self.content, ClipContent::Audio { reverse: true, .. }) {
            self.source_end() - u * self.source_rate
        } else {
            self.offset + u * self.source_rate
        }
    }

    /// Playback clip-local beat where a source position sounds.
    pub fn playback_local(&self, source: f64) -> f64 {
        let local = self.source_to_local(source);
        if matches!(self.content, ClipContent::Audio { reverse: true, .. }) {
            self.length - local
        } else {
            local
        }
    }

    /// Forward local beat for a playback-local beat.
    pub fn forward_local(&self, playback: f64) -> f64 {
        if matches!(self.content, ClipContent::Audio { reverse: true, .. }) {
            self.length - playback
        } else {
            playback
        }
    }
}

/// Snap a beat to the grid. "1 bar" follows real bar lines once the meter
/// changes along the song; other steps are uniform from song zero.
pub fn snap_to_grid(meter: &MeterMap, grid: f64, beat: f64) -> f64 {
    if grid <= 0.0 || !grid.is_finite() {
        return beat;
    }
    if !meter.is_uniform() && grid == f64::from(meter.initial()) {
        return meter.nearest_bar_line(beat);
    }
    (beat / grid).round() * grid
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioQuantizeSettings {
    /// Grid step in beats; a step equal to the first bar follows bar lines.
    pub grid: f64,
    /// 0 leaves hits in place, 1 moves them onto the grid.
    pub strength: f64,
    /// Hits further than this many beats from their grid line stay in place.
    pub max_move: f64,
}

impl AudioQuantizeSettings {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.grid.is_finite() && self.grid > 0.0 && self.grid <= 64.0,
            "Choose a grid between 0 and 64 beats"
        );
        ensure!(
            self.strength.is_finite() && (0.0..=1.0).contains(&self.strength),
            "Quantize strength must be between 0 and 100%"
        );
        ensure!(
            self.max_move.is_finite() && self.max_move >= 0.0 && self.max_move <= 64.0,
            "Maximum move must be between 0 and 64 beats"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WarpQuantizePlan {
    pub markers: Vec<WarpMarker>,
    /// Playback-local beats of each kept hit before and after quantizing.
    pub hits: Vec<(f64, f64)>,
    pub moved: usize,
    /// Largest movement in beats.
    pub largest_move: f64,
}

/// Pin each transient (playback-local beats through the current map) where it
/// sounds now, plus clip start and end. With `settings`, move each hit toward
/// its nearest grid line. Hits that would fold time or leave the supported
/// stretch range are dropped rather than failing the whole plan.
pub fn plan_warp_markers(
    clip: &Clip,
    transients: &[f64],
    meter: &MeterMap,
    settings: Option<AudioQuantizeSettings>,
) -> Result<WarpQuantizePlan> {
    ensure!(
        matches!(clip.content, ClipContent::Audio { .. }),
        "Select an audio clip"
    );
    ensure!(!clip.is_take_source(), "Choose a take before warping it");
    if let Some(settings) = settings {
        settings.validate()?;
    }
    ensure!(
        transients.len() <= MAX_WARP_MARKERS - 2,
        "Too many transients; increase the minimum gap"
    );
    let reverse = matches!(clip.content, ClipContent::Audio { reverse: true, .. });
    // Work in playback order, so a reversed clip quantizes what is heard.
    let mut hits: Vec<(f64, f64, f64)> = Vec::with_capacity(transients.len());
    for &beat in transients {
        ensure!(beat.is_finite(), "Invalid transient position");
        if beat <= MIN_WARP_SPACING || beat >= clip.length - MIN_WARP_SPACING {
            continue;
        }
        let source = clip.local_to_source(clip.forward_local(beat));
        let target = settings.map_or(beat, |settings| {
            let grid = snap_to_grid(meter, settings.grid, clip.start + beat) - clip.start;
            if (grid - beat).abs() <= settings.max_move + 1e-12 {
                beat + (grid - beat) * settings.strength
            } else {
                beat
            }
        });
        hits.push((beat, target, source));
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let start = (0.0, 0.0, clip.local_to_source(clip.forward_local(0.0)));
    let end = (
        clip.length,
        clip.length,
        clip.local_to_source(clip.forward_local(clip.length)),
    );
    // Greedy pass in playback order; source decreases when reversed.
    let valid = |a: (f64, f64, f64), b: (f64, f64, f64)| {
        let beats = b.1 - a.1;
        let source = (b.2 - a.2).abs();
        beats >= MIN_WARP_SPACING
            && source >= MIN_WARP_SPACING
            && segment_rate_allowed(source / beats)
            && (b.2 > a.2) != reverse
    };
    let mut kept = vec![start];
    for hit in hits {
        if valid(*kept.last().unwrap(), hit) {
            kept.push(hit);
        }
    }
    while kept.len() > 1 && !valid(*kept.last().unwrap(), end) {
        kept.pop();
    }
    ensure!(
        valid(*kept.last().unwrap(), end),
        "The clip cannot be warped within the supported stretch range"
    );
    kept.push(end);
    let mut plan = WarpQuantizePlan::default();
    for &(beat, target, _) in &kept[1..kept.len() - 1] {
        let movement = (target - beat).abs();
        if movement > 1e-9 {
            plan.moved += 1;
            plan.largest_move = plan.largest_move.max(movement);
        }
        plan.hits.push((beat, target));
    }
    plan.markers = kept
        .iter()
        .map(|&(_, target, source)| WarpMarker {
            source,
            beat: if reverse {
                clip.length - target
            } else {
                target
            },
        })
        .collect();
    if reverse {
        plan.markers.reverse();
    }
    validate_warp_markers(&plan.markers)?;
    Ok(plan)
}

/// Forward-beat range marker `index` can move to while its neighbours stay
/// fixed, keeping both adjacent segments within the supported rates and the
/// clip start inside the source.
pub fn warp_marker_bounds(clip: &Clip, index: usize) -> (f64, f64) {
    let markers = &clip.warp_markers;
    let Some(marker) = markers.get(index) else {
        return (0.0, 0.0);
    };
    let mut low = -MAX_BEAT;
    let mut high = MAX_BEAT;
    if let Some(left) = index.checked_sub(1).map(|left| markers[left]) {
        let source = marker.source - left.source;
        low = low.max(left.beat + source / MAX_SOURCE_RATE);
        high = high.min(left.beat + source / MIN_SOURCE_RATE);
    } else {
        // Before the first marker source_rate applies down to local zero.
        high = high.min(marker.source / clip.source_rate);
    }
    if let Some(right) = markers.get(index + 1) {
        let source = right.source - marker.source;
        low = low.max(right.beat - source / MIN_SOURCE_RATE);
        high = high.min(right.beat - source / MAX_SOURCE_RATE);
    }
    (low, high.max(low))
}

impl Project {
    /// Replace one clip's warp markers in a single edit. An empty list returns
    /// the clip to its whole-clip `source_rate`, keeping the audio at its start.
    pub fn set_warp_markers(
        &mut self,
        track: u64,
        clip: u64,
        markers: Vec<WarpMarker>,
    ) -> Result<()> {
        let clip = self.clip_mut(track, clip).context("Clip not found")?;
        ensure!(
            matches!(clip.content, ClipContent::Audio { .. }),
            "Warp markers apply to audio clips"
        );
        ensure!(!clip.is_take_source(), "Choose a take before warping it");
        validate_warp_markers(&markers)?;
        clip.warp_markers = markers;
        clip.sync_warp_offset();
        ensure!(
            clip.offset >= 0.0 && clip.offset.is_finite(),
            "Warp markers would start the clip before its source"
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "warp_tests.rs"]
mod tests;
