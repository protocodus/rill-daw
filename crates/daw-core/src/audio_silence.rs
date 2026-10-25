//! Source-preserving silence removal. Analysis measures the loudest channel
//! before gain/fades, in playback order, without rendering pitch processing.
use crate::{AudioData, Clip, Project, audio_sample, new_id};
use anyhow::{Context, Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioSilenceSettings {
    pub threshold_db: f32,
    pub minimum_silence_ms: f32,
    pub padding_ms: f32,
}
impl Default for AudioSilenceSettings {
    fn default() -> Self {
        Self {
            threshold_db: -40.0,
            minimum_silence_ms: 100.0,
            padding_ms: 5.0,
        }
    }
}
impl AudioSilenceSettings {
    fn validate(self) -> Result<()> {
        ensure!(
            self.threshold_db.is_finite() && (-96.0..=0.0).contains(&self.threshold_db),
            "Threshold must be between -96 and 0 dBFS"
        );
        ensure!(
            self.minimum_silence_ms.is_finite()
                && (1.0..=10000.0).contains(&self.minimum_silence_ms),
            "Minimum silence must be between 1 and 10000 ms"
        );
        ensure!(
            self.padding_ms.is_finite() && (0.0..=1000.0).contains(&self.padding_ms),
            "Padding must be between 0 and 1000 ms"
        );
        Ok(())
    }
}

/// Retained spans in arrangement beats, restricted to `range`. Short pauses
/// remain intact; padding protects attacks/releases. Source-rate and reversal
/// affect sample lookup while all duration controls remain playback time.
pub fn detect_audio_silence(
    clip: &Clip,
    data: &AudioData,
    tempo: f64,
    range: (f64, f64),
    settings: AudioSilenceSettings,
) -> Result<Vec<(f64, f64)>> {
    clip.audio_region(range.0, range.1)?;
    data.validate()?;
    settings.validate()?;
    ensure!(
        tempo.is_finite() && tempo > 0.0 && clip.source_rate.is_finite() && clip.source_rate > 0.0,
        "Invalid audio timing"
    );
    let source_frames_per_beat = f64::from(data.sample_rate) * 60.0 / tempo * clip.source_rate;
    let frames = ((range.1 - range.0) * source_frames_per_beat).ceil();
    ensure!(
        frames <= 134_217_728.0,
        "Audio selection is too long for silence analysis"
    );
    let frames = frames as usize;
    let minimum = (f64::from(settings.minimum_silence_ms)
        * 0.001
        * f64::from(data.sample_rate)
        * clip.source_rate)
        .ceil()
        .max(1.0) as usize;
    let padding = f64::from(settings.padding_ms) * 0.001 * tempo / 60.0;
    let threshold = 10_f32.powf(settings.threshold_db / 20.0);
    let mut quiet = None;
    let mut gaps = Vec::new();
    for frame in 0..=frames {
        let below = if frame == frames {
            false
        } else {
            let beat = range.0 - clip.start + frame as f64 / source_frames_per_beat;
            let source = clip.audio_source_frame(beat, tempo, data.sample_rate);
            (0..data.channels).all(|channel| audio_sample(data, source, channel).abs() < threshold)
        };
        if below {
            quiet.get_or_insert(frame);
        } else if let Some(start) = quiet.take()
            && frame - start >= minimum
        {
            let left = if start == 0 {
                range.0
            } else {
                range.0 + start as f64 / source_frames_per_beat + padding
            };
            let right = if frame == frames {
                range.1
            } else {
                range.0 + frame as f64 / source_frames_per_beat - padding
            };
            if right > left {
                ensure!(
                    gaps.len() < 2048,
                    "Too many silent regions; increase Minimum silence"
                );
                gaps.push((left, right));
            }
        }
    }
    let mut retained = Vec::new();
    let mut start = range.0;
    for (left, right) in gaps {
        if left > start {
            retained.push((start, left));
        }
        start = right;
    }
    if start < range.1 {
        retained.push((start, range.1));
    }
    Ok(retained)
}

impl Project {
    /// Apply a reviewed analysis to this occurrence only. Outside the selected
    /// range, other ghosts and retained takes are untouched. Files, processing,
    /// original envelope timing and source identities survive every cut.
    pub fn strip_audio_silence(
        &mut self,
        track_id: u64,
        clip_id: u64,
        range: (f64, f64),
        retained: &[(f64, f64)],
    ) -> Result<Vec<u64>> {
        let original = self.clip(track_id, clip_id).context("Clip not found")?;
        ensure!(
            !original.is_take_source(),
            "Choose a take before editing it"
        );
        original.audio_region(range.0, range.1)?;
        ensure!(retained.len() <= 2049, "Too many retained audio regions");
        let mut spans = Vec::new();
        if original.start < range.0 {
            spans.push((original.start, range.0));
        }
        let mut previous = range.0;
        for &(start, end) in retained {
            ensure!(
                start.is_finite()
                    && end.is_finite()
                    && start >= previous
                    && end > start
                    && end <= range.1,
                "Invalid retained audio regions"
            );
            spans.push((start, end));
            previous = end;
        }
        if range.1 < original.end() {
            spans.push((range.1, original.end()));
        }
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (start, end) in spans {
            if let Some(last) = merged.last_mut()
                && last.1 == start
            {
                last.1 = end;
            } else {
                merged.push((start, end));
            }
        }
        let pieces = merged
            .into_iter()
            .enumerate()
            .map(|(index, (start, end))| {
                let mut piece = original.audio_region(start, end)?;
                if index > 0 {
                    piece.id = new_id();
                }
                Ok(piece)
            })
            .collect::<Result<Vec<_>>>()?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClipContent;
    fn data() -> AudioData {
        AudioData {
            sample_rate: 1000,
            channels: 2,
            samples: (0..2000)
                .flat_map(|frame| {
                    if (200..600).contains(&frame) || (1000..1400).contains(&frame) {
                        [0.5, -0.5]
                    } else {
                        [0.001, -0.001]
                    }
                })
                .collect(),
        }
    }
    fn options() -> AudioSilenceSettings {
        AudioSilenceSettings {
            padding_ms: 0.0,
            ..Default::default()
        }
    }
    #[test]
    fn finds_antiphase_stereo_islands_and_keeps_padding_in_playback_time() {
        let clip = Clip::audio("Source", 3.0, 4.0, "source.wav");
        let spans = detect_audio_silence(&clip, &data(), 120.0, (3.0, 7.0), options()).unwrap();
        assert_eq!(spans, vec![(3.4, 4.2), (5.0, 5.8)]);
        let spans = detect_audio_silence(
            &clip,
            &data(),
            120.0,
            (3.0, 7.0),
            AudioSilenceSettings {
                padding_ms: 50.0,
                ..options()
            },
        )
        .unwrap();
        for (actual, expected) in spans.into_iter().zip([(3.3, 4.3), (4.9, 5.9)]) {
            assert!((actual.0 - expected.0).abs() < 1e-10 && (actual.1 - expected.1).abs() < 1e-10);
        }
    }
    #[test]
    fn reversed_stretched_selection_maps_to_correct_source_frames() {
        let mut clip = Clip::audio("Reverse", 1.0, 8.0, "source.wav");
        clip.source_rate = 0.5;
        if let ClipContent::Audio { reverse, .. } = &mut clip.content {
            *reverse = true;
        }
        let spans = detect_audio_silence(&clip, &data(), 120.0, (1.0, 9.0), options()).unwrap();
        assert_eq!(spans, vec![(3.4, 5.0), (6.6, 8.2)]);
        let spans = detect_audio_silence(&clip, &data(), 120.0, (4.0, 7.0), options()).unwrap();
        assert_eq!(spans, vec![(4.0, 5.0), (6.6, 7.0)]);
    }
    #[test]
    fn short_silence_is_kept_and_all_silence_can_be_removed() {
        let clip = Clip::audio("Silent", 0.0, 0.1, "source.wav");
        assert_eq!(
            detect_audio_silence(&clip, &data(), 120.0, (0.0, 0.1), options()).unwrap(),
            vec![(0.0, 0.1)]
        );
        let clip = Clip::audio("Silent", 0.0, 0.2, "source.wav");
        assert!(
            detect_audio_silence(&clip, &data(), 120.0, (0.0, 0.2), options())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn apply_preserves_ghosts_source_mapping_envelopes_and_atomicity() {
        let mut p = Project::new();
        let t = p.add_audio_track("Audio");
        let mut clip = Clip::audio("Source", 2.0, 4.0, "source.wav");
        clip.offset = 1.0;
        if let ClipContent::Audio {
            reverse, fade_in, ..
        } = &mut clip.content
        {
            *reverse = true;
            *fade_in = 0.25;
        }
        let id = clip.id;
        p.track_mut(t).unwrap().clips.push(clip);
        let ghost = p.clone_clip(t, id).unwrap();
        let original = p.clip(t, id).unwrap().clone();
        let other = p.clip(t, ghost).unwrap().clone();
        let ids = p
            .strip_audio_silence(t, id, (2.5, 5.5), &[(3.0, 4.0), (5.0, 5.5)])
            .unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(p.clip(t, ghost), Some(&other));
        for id in ids {
            let piece = p.clip(t, id).unwrap();
            assert_eq!(piece.link_group, original.link_group);
            assert!(
                (piece.audio_source_frame(0.0, 120.0, 1000)
                    - original.audio_source_frame(piece.start - original.start, 120.0, 1000))
                .abs()
                    < 1e-8
            );
            assert_eq!(
                piece.audio_envelope_gain(0.01),
                original.audio_envelope_gain(piece.start - original.start + 0.01)
            );
        }
        let before = p.clone();
        assert!(
            p.strip_audio_silence(
                t,
                ghost,
                (other.start, other.end()),
                &[(other.start, other.end() + 1.0)]
            )
            .is_err()
        );
        assert_eq!(p, before);
        p.validate().unwrap();
    }
    #[test]
    fn comp_cuts_keep_retained_take_alternatives() {
        let mut p = Project::new();
        let t = p.add_audio_track("Takes");
        let a = Clip::audio("A", 0.0, 4.0, "a.wav");
        let b = Clip::audio("B", 0.0, 4.0, "b.wav");
        let ids = [a.id, b.id];
        p.track_mut(t).unwrap().clips.extend([a, b]);
        let group = p.group_audio_takes(t, &ids).unwrap();
        let comp = p.audio_take_regions(t, group)[0].id;
        let sources: Vec<_> = p
            .track(t)
            .unwrap()
            .clips
            .iter()
            .filter(|c| c.is_take_source())
            .cloned()
            .collect();
        p.strip_audio_silence(t, comp, (0.0, 4.0), &[(0.5, 1.0), (2.0, 3.0)])
            .unwrap();
        assert_eq!(
            p.track(t)
                .unwrap()
                .clips
                .iter()
                .filter(|c| c.is_take_source())
                .cloned()
                .collect::<Vec<_>>(),
            sources
        );
        assert_eq!(p.audio_take_regions(t, group).len(), 2);
        p.validate().unwrap();
        let regions: Vec<_> = p
            .audio_take_regions(t, group)
            .into_iter()
            .cloned()
            .collect();
        for region in regions {
            p.strip_audio_silence(t, region.id, (region.start, region.end()), &[])
                .unwrap();
        }
        assert!(p.audio_take_regions(t, group).is_empty());
        assert_eq!(p.track(t).unwrap().clips, sources);
        p.validate().unwrap();
    }
}
