//! Source-preserving audio regions, envelopes and analysis.

use crate::{
    AudioData, AudioTakeMembership, AudioTakeRole, Clip, ClipContent, MAX_BEAT, Project, TrackKind,
    new_id,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FadeCurve {
    #[default]
    Linear,
    EqualPower,
    SCurve,
}

impl FadeCurve {
    pub const ALL: [Self; 3] = [Self::Linear, Self::EqualPower, Self::SCurve];

    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::EqualPower => "Equal power",
            Self::SCurve => "S curve",
        }
    }

    pub fn value(self, progress: f64) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        (match self {
            Self::Linear => t,
            Self::EqualPower => (t * std::f64::consts::FRAC_PI_2).sin(),
            Self::SCurve => t * t * (3.0 - 2.0 * t),
        }) as f32
    }
}

/// Splits and trims retain the original envelope's timing. Editing a fade
/// explicitly clears this reference and anchors it to the current clip again.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioEnvelope {
    pub offset: f64,
    pub length: f64,
}

impl Clip {
    /// A source-preserving subregion in arrangement coordinates. The source,
    /// fades and processing are shared by value; placement and bounds are local.
    /// Callers decide whether the resulting region keeps ghost/take membership.
    pub fn audio_region(&self, start: f64, end: f64) -> Result<Self> {
        ensure!(
            matches!(self.content, ClipContent::Audio { .. }),
            "Select an audio clip"
        );
        ensure!(
            start.is_finite()
                && end.is_finite()
                && start >= self.start
                && end <= self.end()
                && end > start,
            "Selection must be inside the clip"
        );
        let mut region = self.clone();
        let delta = start - self.start;
        region.offset += if matches!(self.content, ClipContent::Audio { reverse: true, .. }) {
            (self.end() - end) * self.source_rate
        } else {
            delta * self.source_rate
        };
        region.start = start;
        region.length = end - start;
        if let ClipContent::Audio {
            envelope,
            boundary_fades,
            crossfade,
            ..
        } = &mut region.content
        {
            let original = envelope.unwrap_or(AudioEnvelope {
                offset: 0.0,
                length: self.source_length(),
            });
            ensure!(
                (original.offset + delta * self.source_rate).abs() <= MAX_BEAT,
                "Selection exceeds the envelope range"
            );
            *envelope = Some(AudioEnvelope {
                offset: original.offset + delta * self.source_rate,
                ..original
            });
            if let Some(boundary) = boundary_fades {
                ensure!(
                    (boundary.offset + delta * self.source_rate).abs() <= MAX_BEAT,
                    "Selection exceeds the boundary fade range"
                );
                boundary.offset += delta * self.source_rate;
            }
            // A transition belongs to the existing left edge, not an arbitrary
            // new boundary created inside the region.
            if start != self.start {
                *crossfade = None;
            }
        }
        Ok(region)
    }

    pub fn audio_envelope_gain(&self, beat: f64) -> f32 {
        let ClipContent::Audio {
            fade_in,
            fade_out,
            fade_in_curve,
            fade_out_curve,
            envelope,
            boundary_fades,
            ..
        } = &self.content
        else {
            return 1.0;
        };
        let region = envelope.unwrap_or(AudioEnvelope {
            offset: 0.0,
            length: self.source_length(),
        });
        let position = beat * self.source_rate + region.offset;
        let attack = if *fade_in > 0.0 {
            fade_in_curve.value(position / fade_in)
        } else {
            1.0
        };
        let release = if *fade_out > 0.0 {
            fade_out_curve.value((region.length - position) / fade_out)
        } else {
            1.0
        };
        attack
            * release
            * boundary_fades.map_or(1.0, |boundary| boundary.gain(beat * self.source_rate))
    }

    pub fn audio_source_frame(&self, beat: f64, tempo: f64, sample_rate: u32) -> f64 {
        let frames_per_beat = f64::from(sample_rate) * 60.0 / tempo;
        let reverse = matches!(self.content, ClipContent::Audio { reverse: true, .. });
        self.offset * frames_per_beat
            + if reverse {
                (self.length - beat) * self.source_rate * frames_per_beat - 1.0
            } else {
                beat * self.source_rate * frames_per_beat
            }
    }
}

/// Linear interpolation shared by waveform previews and analysis. Out-of-source
/// portions are silent; channel frames remain interleaved when reversed.
pub fn audio_sample(data: &AudioData, frame: f64, channel: usize) -> f32 {
    if !frame.is_finite() || frame < -1e-8 || data.channels == 0 || data.frames() == 0 {
        return 0.0;
    }
    let frame = frame.max(0.0);
    let index = frame.floor() as usize;
    if index >= data.frames() {
        return 0.0;
    }
    let next = (index + 1).min(data.frames() - 1);
    let channel = channel.min(data.channels - 1);
    let a = data.samples[index * data.channels + channel];
    let b = data.samples[next * data.channels + channel];
    a + (b - a) * (frame - index as f64) as f32
}

/// Find a source zero crossing near an arrangement beat (at most 5 ms each
/// side). A single frame is chosen for every channel. The quietest multichannel
/// crossing wins, then the nearest; anti-phase stereo cannot cancel to silence.
/// Pitch processing can change the resulting waveform: this snaps the source.
pub fn nearest_audio_zero_crossing(
    clip: &Clip,
    data: &AudioData,
    tempo: f64,
    beat: f64,
) -> Option<f64> {
    if !matches!(clip.content, ClipContent::Audio { .. })
        || data.frames() < 2
        || data.channels == 0
        || data.sample_rate == 0
        || !(crate::MIN_SOURCE_RATE..=crate::MAX_SOURCE_RATE).contains(&clip.source_rate)
        || !tempo.is_finite()
        || tempo <= 0.0
        || !beat.is_finite()
    {
        return None;
    }
    let fpb = f64::from(data.sample_rate) * 60.0 / tempo;
    let source = clip.audio_source_frame(beat - clip.start, tempo, data.sample_rate);
    let radius = (f64::from(data.sample_rate) * 0.005 * clip.source_rate)
        .ceil()
        .max(1.0);
    let lo = (source - radius).ceil().max(1.0) as usize;
    let hi = (source + radius).floor().max(0.0) as usize;
    let reverse = matches!(clip.content, ClipContent::Audio { reverse: true, .. });
    (lo..=hi.min(data.frames() - 1))
        .filter_map(|frame| {
            let mut crossing = false;
            let mut peak = 0.0_f32;
            for channel in 0..data.channels {
                let previous = data.samples[(frame - 1) * data.channels + channel];
                let value = data.samples[frame * data.channels + channel];
                crossing |= value == 0.0
                    || (previous != 0.0 && previous.is_sign_negative() != value.is_sign_negative());
                peak = peak.max(value.abs());
            }
            if !crossing {
                return None;
            }
            let local = if reverse {
                clip.length - (frame as f64 - clip.offset * fpb + 1.0) / fpb / clip.source_rate
            } else {
                (frame as f64 - clip.offset * fpb) / fpb / clip.source_rate
            };
            let position = clip.start + local;
            (0.0..=MAX_BEAT).contains(&position).then_some((
                position,
                peak,
                (frame as f64 - source).abs(),
            ))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.2.total_cmp(&b.2)))
        .map(|value| value.0)
}

/// Re-anchor alternate takes to an edited comp occurrence, including movement,
/// slip, reverse and stretch. Each occurrence gets its own group because its
/// source-to-timeline mapping may differ from the other comps in the old group.
fn ripple_take_windows(original: &[Clip], right: &mut Clip) -> Result<Vec<Clip>> {
    let Some(take) = right.audio_take else {
        return Ok(Vec::new());
    };
    let AudioTakeRole::Comp { source } = take.role else {
        return Ok(Vec::new());
    };
    let reference = original
        .iter()
        .find(|clip| clip.id == source)
        .context("Retained take not found")?;
    let reference_reverse = matches!(reference.content, ClipContent::Audio { reverse: true, .. });
    let reverse =
        reference_reverse != matches!(right.content, ClipContent::Audio { reverse: true, .. });
    let start = reference.start
        + if reference_reverse {
            (reference.source_end() - right.source_end()) / reference.source_rate
        } else {
            (right.offset - reference.offset) / reference.source_rate
        };
    let end = start + right.source_length() / reference.source_rate;
    let ratio = reference.source_rate / right.source_rate;
    let group = new_id();
    let mut windows = Vec::new();
    let mut reference_id = None;
    for candidate in original
        .iter()
        .filter(|clip| clip.is_take_source() && clip.take_group() == Some(take.group))
    {
        // Keep the current comp's exact material even if it exposes source
        // handles beyond the original take's trimmed metadata.
        let mut window = if candidate.id == source {
            let mut window = right.clone();
            window.name.clone_from(&candidate.name);
            window
        } else {
            let a = start.max(candidate.start);
            let b = end.min(candidate.end());
            if b <= a {
                continue;
            }
            let mut window = candidate.audio_region(a, b)?;
            let source_rate = window.source_rate / ratio;
            // An alternate may already use the opposite stretch extreme.
            // Keep that original retained rather than rejecting an otherwise
            // valid ripple or creating an unsupported transformed candidate.
            if !(crate::MIN_SOURCE_RATE..=crate::MAX_SOURCE_RATE).contains(&source_rate) {
                continue;
            }
            window.start = right.start
                + if reverse {
                    (end - b) * ratio
                } else {
                    (a - start) * ratio
                };
            window.length *= ratio;
            window.source_rate = source_rate;
            if reverse && let ClipContent::Audio { reverse, .. } = &mut window.content {
                *reverse = !*reverse;
            }
            window
        };
        window.id = new_id();
        window.link_group = None;
        window.audio_take = Some(AudioTakeMembership {
            group,
            role: AudioTakeRole::Source,
        });
        if let ClipContent::Audio { crossfade, .. } = &mut window.content {
            *crossfade = None;
        }
        if candidate.id == source {
            reference_id = Some(window.id);
        }
        windows.push(window);
    }
    right.audio_take = Some(AudioTakeMembership {
        group,
        role: AudioTakeRole::Comp {
            source: reference_id.context("Retained take not found")?,
        },
    });
    Ok(windows)
}

/// Normalize the source window's sample peak, before fades, to an absolute dBFS
/// target. Existing gain does not accumulate; silent clips are left unchanged.
pub fn normalization_gain(
    clip: &Clip,
    data: &AudioData,
    tempo: f64,
    target_db: f32,
) -> Result<f32> {
    ensure!(
        matches!(clip.content, ClipContent::Audio { .. }),
        "Select an audio clip"
    );
    data.validate()?;
    ensure!(
        tempo.is_finite()
            && tempo > 0.0
            && target_db.is_finite()
            && (-60.0..=0.0).contains(&target_db),
        "Invalid normalization target"
    );
    let frames_per_beat = f64::from(data.sample_rate) * 60.0 / tempo;
    let start = (clip.offset * frames_per_beat).floor().max(0.0) as usize;
    let end = (clip.source_end() * frames_per_beat).ceil().max(0.0) as usize;
    let peak = if start < data.frames() {
        data.samples[start * data.channels..end.min(data.frames()) * data.channels]
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max)
    } else {
        0.0
    };
    ensure!(peak > 1e-12, "Cannot normalize a silent clip");
    Ok(10.0_f32.powf(target_db / 20.0) / peak)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransientAlgorithm {
    EnergyRise,
    AmplitudeGate,
}

impl TransientAlgorithm {
    pub const ALL: [Self; 2] = [Self::EnergyRise, Self::AmplitudeGate];
    pub fn name(self) -> &'static str {
        match self {
            Self::EnergyRise => "Energy rise",
            Self::AmplitudeGate => "Amplitude gate",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransientSettings {
    pub algorithm: TransientAlgorithm,
    pub sensitivity: f32,
    pub minimum_gap_ms: f32,
}

impl Default for TransientSettings {
    fn default() -> Self {
        Self {
            algorithm: TransientAlgorithm::EnergyRise,
            sensitivity: 0.5,
            minimum_gap_ms: 80.0,
        }
    }
}

/// Return unsnapped, clip-relative onset beats. A 5 ms multichannel RMS/peak
/// envelope rejects zero crossings; hysteresis and a minimum gap reject chatter.
pub fn detect_transients(
    clip: &Clip,
    data: &AudioData,
    tempo: f64,
    settings: TransientSettings,
) -> Result<Vec<f64>> {
    ensure!(
        matches!(clip.content, ClipContent::Audio { .. }),
        "Select an audio clip"
    );
    data.validate()?;
    ensure!(
        tempo.is_finite()
            && tempo > 0.0
            && settings.sensitivity.is_finite()
            && (0.0..=1.0).contains(&settings.sensitivity)
            && settings.minimum_gap_ms.is_finite()
            && (20.0..=2000.0).contains(&settings.minimum_gap_ms),
        "Invalid transient settings"
    );
    let frames_per_beat = f64::from(data.sample_rate) * 60.0 / tempo;
    // Inspect every source frame, even for a compressed clip. Sampling at the
    // output rate would skip narrow source transients when source_rate > 1.
    let frames = (clip.source_length() * frames_per_beat).ceil() as usize;
    let window = (f64::from(data.sample_rate) * 0.005 * clip.source_rate)
        .round()
        .max(1.0) as usize;
    let count = frames.div_ceil(window);
    ensure!(
        count <= 1_000_000,
        "Clip is too long for transient analysis"
    );
    let mut levels = Vec::with_capacity(count);
    for block in 0..count {
        let mut sum = 0.0_f64;
        let mut peak = 0.0_f32;
        let length = window.min(frames - block * window);
        for frame in 0..length {
            let beat = (block * window + frame) as f64 / frames_per_beat / clip.source_rate;
            let source = clip.audio_source_frame(beat, tempo, data.sample_rate);
            let mut level = 0.0_f32;
            for channel in 0..data.channels {
                level = level.max(audio_sample(data, source, channel).abs());
            }
            peak = peak.max(level);
            sum += f64::from(level) * f64::from(level);
        }
        levels.push(match settings.algorithm {
            TransientAlgorithm::EnergyRise => (sum / length.max(1) as f64).sqrt() as f32,
            TransientAlgorithm::AmplitudeGate => peak,
        });
    }
    let peak = levels.iter().copied().fold(0.0_f32, f32::max);
    if peak <= 1e-8 {
        return Ok(Vec::new());
    }
    let floor = peak * (0.04 + 0.46 * (1.0 - settings.sensitivity));
    let minimum_gap = settings.minimum_gap_ms as f64 * 0.001 * tempo / 60.0;
    let mut result = Vec::new();
    let mut previous = 0.0_f32;
    let mut baseline = 0.0_f32;
    let mut armed = true;
    for (index, level) in levels.iter().copied().enumerate() {
        let attack = match settings.algorithm {
            TransientAlgorithm::AmplitudeGate => armed && level >= floor,
            TransientAlgorithm::EnergyRise => {
                level >= floor
                    && level > previous * (1.15 + (1.0 - settings.sensitivity) * 1.2)
                    && level > baseline * (1.1 + (1.0 - settings.sensitivity))
            }
        };
        let beat = (index * window) as f64 / frames_per_beat / clip.source_rate;
        if attack
            && beat > 1e-9
            && beat < clip.length - 1e-9
            && result
                .last()
                .is_none_or(|last: &f64| beat - *last >= minimum_gap)
        {
            ensure!(
                result.len() < 2048,
                "Too many transients; increase the minimum gap"
            );
            result.push(beat);
        }
        if level >= floor {
            armed = false;
        }
        if level < floor * 0.4 {
            armed = true;
        }
        baseline = baseline * 0.9 + level * 0.1;
        previous = level;
    }
    Ok(result)
}

impl Project {
    /// Close a time selection on one audio track. All audible regions intersecting
    /// the selection are cut; subsequent regions move left by its duration.
    /// Original takes remain retained. Shifted comps receive aligned source
    /// windows in a new group so switching takes cannot reintroduce old timing.
    pub fn ripple_delete_audio_range(
        &mut self,
        track_id: u64,
        start: f64,
        end: f64,
    ) -> Result<Vec<u64>> {
        ensure!(
            start.is_finite() && end.is_finite() && start >= 0.0 && end > start && end <= MAX_BEAT,
            "Invalid ripple selection"
        );
        let track = self.track(track_id).context("Track not found")?;
        ensure!(
            track.kind == TrackKind::Audio,
            "Ripple editing requires an audio track"
        );
        let original = track.clips.clone();
        let duration = end - start;
        let mut clips = Vec::with_capacity(original.len());
        let mut source_windows = Vec::new();
        let mut boundary_pieces = Vec::new();
        let mut selected = Vec::new();
        for clip in &original {
            if clip.is_take_source() || clip.end() <= start {
                clips.push(clip.clone());
                continue;
            }
            if clip.start < start {
                let left = clip.audio_region(clip.start, start)?;
                selected.push(left.id);
                clips.push(left);
            }
            if clip.end() <= end {
                continue;
            }
            let mut right = clip.audio_region(clip.start.max(end), clip.end())?;
            if clip.start < start {
                right.id = new_id();
            }
            right.start -= duration;
            if clip.start <= end
                && let ClipContent::Audio { crossfade, .. } = &mut right.content
            {
                *crossfade = None;
            }
            source_windows.extend(ripple_take_windows(&original, &mut right)?);
            selected.push(right.id);
            if clip.start <= end {
                boundary_pieces.push((clip.start, right));
            } else {
                clips.push(right);
            }
        }
        // At the collapsed boundary several formerly overlapping clips now
        // start together. Preserve their original latest-start priority; stable
        // sorting also preserves the existing vector order for equal starts.
        boundary_pieces.sort_by(|a, b| a.0.total_cmp(&b.0));
        clips.extend(boundary_pieces.into_iter().map(|(_, clip)| clip));
        clips.extend(source_windows);
        self.track_mut(track_id).unwrap().clips = clips;
        if let Err(error) = self.validate() {
            self.track_mut(track_id).unwrap().clips = original;
            return Err(error);
        }
        Ok(selected)
    }

    /// Copy selected time as independent source-preserving audio.
    pub fn copy_audio_range(&self, track: u64, clip: u64, start: f64, end: f64) -> Result<Clip> {
        let mut region = self
            .clip(track, clip)
            .context("Clip not found")?
            .audio_region(start, end)?;
        region.link_group = None;
        region.audio_take = None;
        if let ClipContent::Audio { crossfade, .. } = &mut region.content {
            *crossfade = None;
        }
        Ok(region)
    }

    /// Remove selected time without closing the gap. Retained takes and other
    /// ghost occurrences remain untouched. Validation commits all pieces at once.
    pub fn delete_audio_range(
        &mut self,
        track_id: u64,
        clip_id: u64,
        start: f64,
        end: f64,
    ) -> Result<Vec<u64>> {
        let original = self.clip(track_id, clip_id).context("Clip not found")?;
        ensure!(
            !original.is_take_source(),
            "Choose a take before editing it"
        );
        original.audio_region(start, end)?;
        let mut pieces = Vec::new();
        if start > original.start {
            pieces.push(original.audio_region(original.start, start)?);
        }
        if end < original.end() {
            let mut right = original.audio_region(end, original.end())?;
            if !pieces.is_empty() {
                right.id = new_id();
            }
            pieces.push(right);
        }
        let ids = pieces.iter().map(|clip| clip.id).collect();
        let track = self.track_mut(track_id).context("Track not found")?;
        let index = track
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .context("Clip not found")?;
        let before = track.clips.clone();
        track.clips.splice(index..=index, pieces);
        if let Err(error) = self.validate() {
            self.track_mut(track_id).unwrap().clips = before;
            return Err(error);
        }
        Ok(ids)
    }

    /// Slip content in timeline beats while keeping its edges and envelopes
    /// fixed. Positive movement moves the waveform right, including reverse
    /// clips. Clamp against the actual file bounds; ghosts keep local offsets.
    pub fn slip_audio_clip(
        &mut self,
        track: u64,
        clip: u64,
        movement: f64,
        source_beats: f64,
    ) -> Result<()> {
        ensure!(
            movement.is_finite()
                && source_beats.is_finite()
                && (0.0..=MAX_BEAT).contains(&source_beats),
            "Invalid source bounds"
        );
        let clip = self.clip_mut(track, clip).context("Clip not found")?;
        ensure!(!clip.is_take_source(), "Choose a take before editing it");
        let ClipContent::Audio { reverse, .. } = clip.content else {
            anyhow::bail!("Select an audio clip");
        };
        ensure!(
            source_beats + 1e-9 >= clip.source_length(),
            "Trim the clip to fit its source before slipping"
        );
        let direction = if reverse { 1.0 } else { -1.0 };
        clip.offset = (clip.offset + movement * clip.source_rate * direction)
            .clamp(0.0, (source_beats - clip.source_length()).max(0.0));
        Ok(())
    }

    pub fn set_audio_gain(&mut self, track: u64, clip: u64, gain: f32) -> Result<()> {
        ensure!(gain.is_finite() && gain >= 0.0, "Invalid audio gain");
        ensure!(
            !self
                .clip(track, clip)
                .context("Clip not found")?
                .is_take_source(),
            "Choose a take before editing it"
        );
        let ClipContent::Audio { gain: value, .. } = &mut self
            .clip_mut(track, clip)
            .context("Clip not found")?
            .content
        else {
            anyhow::bail!("Select an audio clip");
        };
        *value = gain;
        self.synchronize_linked_clip(track, clip)
    }

    pub fn set_audio_fades(
        &mut self,
        track: u64,
        clip_id: u64,
        fade_in: f64,
        fade_out: f64,
        in_curve: FadeCurve,
        out_curve: FadeCurve,
    ) -> Result<()> {
        let clip = self.clip_mut(track, clip_id).context("Clip not found")?;
        ensure!(!clip.is_take_source(), "Choose a take before editing it");
        let linked = clip.link_group.is_some();
        let fade_limit = if linked {
            match &clip.content {
                ClipContent::Audio {
                    envelope: Some(region),
                    ..
                } => region.length,
                _ => clip.source_length(),
            }
        } else {
            clip.source_length()
        };
        ensure!(
            fade_in.is_finite()
                && fade_out.is_finite()
                && fade_in >= 0.0
                && fade_out >= 0.0
                && fade_in <= fade_limit
                && fade_out <= fade_limit,
            "Fades must fit inside the clip"
        );
        let ClipContent::Audio {
            fade_in: attack,
            fade_out: release,
            fade_in_curve,
            fade_out_curve,
            envelope,
            ..
        } = &mut clip.content
        else {
            anyhow::bail!("Select an audio clip");
        };
        *attack = fade_in;
        *release = fade_out;
        *fade_in_curve = in_curve;
        *fade_out_curve = out_curve;
        if !linked {
            *envelope = None;
        }
        self.synchronize_linked_clip(track, clip_id)
    }

    pub fn reverse_audio_clip(&mut self, track: u64, clip: u64) -> Result<()> {
        ensure!(
            !self
                .clip(track, clip)
                .context("Clip not found")?
                .is_take_source(),
            "Choose a take before editing it"
        );
        let ClipContent::Audio { reverse, .. } = &mut self
            .clip_mut(track, clip)
            .context("Clip not found")?
            .content
        else {
            anyhow::bail!("Select an audio clip");
        };
        *reverse = !*reverse;
        self.synchronize_linked_clip(track, clip)
    }

    pub fn paste_audio_clip(&mut self, track: u64, original: &Clip, start: f64) -> Result<u64> {
        ensure!(
            matches!(original.content, ClipContent::Audio { .. }),
            "Clipboard does not contain audio"
        );
        ensure!(
            start.is_finite() && start >= 0.0 && start + original.length <= MAX_BEAT,
            "Invalid paste position"
        );
        let track = self.track_mut(track).context("Track not found")?;
        ensure!(
            track.kind == TrackKind::Audio,
            "Paste audio onto an audio track"
        );
        let mut clip = original.clone();
        clip.id = new_id();
        clip.link_group = None;
        clip.audio_take = None;
        clip.start = start;
        let id = clip.id;
        let track_id = track.id;
        track.clips.push(clip);
        if let Err(error) = self.validate() {
            self.track_mut(track_id).unwrap().clips.pop();
            return Err(error);
        }
        Ok(id)
    }

    pub fn split_audio_transients(
        &mut self,
        track: u64,
        clip: u64,
        beats: &[f64],
    ) -> Result<Vec<u64>> {
        let original = self.clip(track, clip).context("Clip not found")?;
        ensure!(
            matches!(original.content, ClipContent::Audio { .. }),
            "Select an audio clip"
        );
        ensure!(
            beats.len() <= 2048
                && beats
                    .iter()
                    .all(|beat| beat.is_finite() && *beat > 0.0 && *beat < original.length)
                && beats.windows(2).all(|pair| pair[0] < pair[1]),
            "Transient positions must be sorted and strictly inside the clip"
        );
        if let ClipContent::Audio {
            envelope: Some(envelope),
            ..
        } = original.content
        {
            ensure!(
                beats
                    .last()
                    .is_none_or(
                        |beat| (envelope.offset + beat * original.source_rate).abs() <= MAX_BEAT
                    ),
                "Split exceeds the envelope range"
            );
        }
        let start = original.start;
        if let ClipContent::Audio {
            boundary_fades: Some(boundary),
            ..
        } = original.content
        {
            ensure!(
                beats
                    .last()
                    .is_none_or(
                        |beat| (boundary.offset + beat * original.source_rate).abs() <= MAX_BEAT
                    ),
                "Split exceeds the boundary fade range"
            );
        }
        let mut current = clip;
        let mut ids = vec![clip];
        for beat in beats {
            current = self.split_clip(track, current, start + beat)?;
            ids.push(current);
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curves_are_bounded_monotone_and_have_distinct_shapes() {
        for curve in FadeCurve::ALL {
            assert_eq!(curve.value(-1.0), 0.0);
            assert_eq!(curve.value(2.0), 1.0);
            let values: Vec<_> = (0..=100).map(|i| curve.value(i as f64 / 100.0)).collect();
            assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        }
        assert!(FadeCurve::EqualPower.value(0.5) > FadeCurve::Linear.value(0.5));
        assert!(FadeCurve::SCurve.value(0.25) < FadeCurve::Linear.value(0.25));
        assert!(FadeCurve::SCurve.value(0.75) > FadeCurve::Linear.value(0.75));
    }
    #[test]
    fn normalization_uses_selected_multichannel_source_window_and_absolute_target() {
        let data = AudioData {
            sample_rate: 8000,
            channels: 2,
            samples: (0..8000)
                .flat_map(|i| if i < 4000 { [1.0, 1.0] } else { [0.2, -0.4] })
                .collect(),
        };
        let mut clip = Clip::audio("region", 0.0, 1.0, "sample.wav");
        clip.offset = 1.0;
        let gain = normalization_gain(&clip, &data, 120.0, -1.0).unwrap();
        assert!((gain * 0.4 - 10.0_f32.powf(-0.05)).abs() < 1e-6);
        if let ClipContent::Audio { gain: value, .. } = &mut clip.content {
            *value = gain;
        }
        assert_eq!(normalization_gain(&clip, &data, 120.0, -1.0).unwrap(), gain);
        assert!(
            normalization_gain(
                &clip,
                &AudioData {
                    samples: vec![0.0; 16000],
                    ..data
                },
                120.0,
                -1.0
            )
            .is_err()
        );
    }
    #[test]
    fn onset_algorithms_sensitivity_and_reverse_find_source_attacks() {
        let mut samples = vec![0.0; 16000];
        for (start, level) in [(2000, 0.8), (6000, 0.1), (10000, 0.8)] {
            samples[start..start + 400].fill(level);
        }
        let data = AudioData {
            sample_rate: 8000,
            channels: 1,
            samples,
        };
        let mut clip = Clip::audio("drums", 0.0, 4.0, "drums.wav");
        for algorithm in TransientAlgorithm::ALL {
            let settings = TransientSettings {
                algorithm,
                sensitivity: 1.0,
                ..Default::default()
            };
            let high = detect_transients(&clip, &data, 120.0, settings).unwrap();
            assert_eq!(high, vec![0.5, 1.5, 2.5]);
            let low = detect_transients(
                &clip,
                &data,
                120.0,
                TransientSettings {
                    sensitivity: 0.0,
                    ..settings
                },
            )
            .unwrap();
            assert_eq!(low, vec![0.5, 2.5]);
        }
        if let ClipContent::Audio { reverse, .. } = &mut clip.content {
            *reverse = true;
        }
        let reversed = detect_transients(
            &clip,
            &data,
            120.0,
            TransientSettings {
                sensitivity: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(reversed, vec![1.4, 2.4, 3.4]);
    }
    #[test]
    fn audio_operations_roundtrip_undo_and_reject_invalid_splits_without_edits() {
        let mut project = Project::new();
        let track = project.add_audio_track("Audio");
        let clip = Clip::audio("clip", 0.0, 4.0, "source.wav");
        let id = clip.id;
        project.track_mut(track).unwrap().clips.push(clip);
        let original = project.clone();
        let mut history = crate::History::default();
        history.commit(&project);
        project
            .set_audio_fades(
                track,
                id,
                2.0,
                1.0,
                FadeCurve::SCurve,
                FadeCurve::EqualPower,
            )
            .unwrap();
        project.reverse_audio_clip(track, id).unwrap();
        project.set_audio_gain(track, id, 1.5).unwrap();
        let edited = project.clone();
        assert!(
            project
                .split_audio_transients(track, id, &[2.0, 1.0])
                .is_err()
        );
        assert_eq!(project, edited);
        project
            .split_audio_transients(track, id, &[1.0, 2.0])
            .unwrap();
        let copy = project.clip(track, id).unwrap().clone();
        let pasted = project.paste_audio_clip(track, &copy, 8.0).unwrap();
        assert_ne!(pasted, id);
        project.validate().unwrap();
        assert_eq!(
            serde_json::from_str::<Project>(&serde_json::to_string(&project).unwrap()).unwrap(),
            project
        );
        assert!(history.undo(&mut project));
        assert_eq!(project, original);
    }
}
