//! Shared, non-destructive sample analysis for waveform previews and player edits.
use daw_core::AudioData;

/// First and last audible frames across every channel, normalized to the source.
/// A −60 dBFS floor and 1 ms padding retain attacks and quiet release boundaries.
/// All-silent recordings have no audible region; their markers stay unchanged.
pub(crate) fn audible_region(audio: &AudioData) -> Option<(f32, f32)> {
    let channels = audio.channels.max(1);
    let frames = audio.samples.len() / channels;
    if frames == 0 {
        return None;
    }
    let audible = |frame: &[f32]| {
        frame
            .iter()
            .any(|value| value.is_finite() && value.abs() > 0.001)
    };
    let first = audio.samples.chunks_exact(channels).position(audible)?;
    let last = audio.samples.chunks_exact(channels).rposition(audible)?;
    let padding = (audio.sample_rate as usize / 1000).max(1);
    Some((
        first.saturating_sub(padding) as f32 / frames as f32,
        (last + 1).saturating_add(padding).min(frames) as f32 / frames as f32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stereo_boundaries_retain_both_channels_without_cancellation() {
        let mut audio = AudioData {
            sample_rate: 1000,
            channels: 2,
            samples: vec![0.0; 200],
        };
        audio.samples[21] = 0.5;
        audio.samples[160] = 0.4;
        audio.samples[161] = -0.4;
        assert_eq!(audible_region(&audio), Some((0.09, 0.82)));
        audio.samples.fill(0.0);
        assert_eq!(audible_region(&audio), None);
        audio.samples[0] = f32::NAN;
        assert_eq!(audible_region(&audio), None);
    }
}
