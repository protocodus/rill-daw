//! Immutable channel lookup and coherent stereo telemetry. Audio graph workers
//! publish once per block; UI reads never consume peaks or touch the callback.
use daw_core::{Project, TrackKind};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
pub(crate) struct ChannelMeter {
    stereo: AtomicU64,
}

impl ChannelMeter {
    pub(crate) fn publish(&self, peaks: [f32; 2], decay: f32) {
        let previous = self.peaks();
        let clean = |channel: usize| {
            let peak = peaks[channel];
            let peak = if peak.is_finite() { peak.abs() } else { 0.0 };
            peak.max(previous[channel] * decay)
        };
        let packed = u64::from(clean(0).to_bits()) | (u64::from(clean(1).to_bits()) << 32);
        self.stereo.store(packed, Ordering::Relaxed);
    }

    pub(crate) fn peaks(&self) -> [f32; 2] {
        let packed = self.stereo.load(Ordering::Relaxed);
        [
            f32::from_bits(packed as u32),
            f32::from_bits((packed >> 32) as u32),
        ]
    }

    pub(crate) fn clear(&self) {
        self.stereo.store(0, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub(crate) struct ChannelMeters {
    channels: Vec<(u64, Arc<ChannelMeter>)>,
}

impl ChannelMeters {
    pub(crate) fn new(project: &Project) -> Self {
        let mut channels: Vec<_> = project
            .tracks
            .iter()
            .filter(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            })
            .map(|track| (track.id, Arc::new(ChannelMeter::default())))
            .collect();
        channels.sort_unstable_by_key(|(id, _)| *id);
        Self { channels }
    }

    pub(crate) fn channel(&self, id: u64) -> Option<&Arc<ChannelMeter>> {
        self.channels
            .binary_search_by_key(&id, |(id, _)| *id)
            .ok()
            .map(|index| &self.channels[index].1)
    }

    pub(crate) fn peaks(&self, id: u64) -> [f32; 2] {
        self.channel(id)
            .map(|meter| meter.peaks())
            .unwrap_or([0.0; 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_peaks_are_nonconsuming_finite_and_release_at_block_rate() {
        let meter = ChannelMeter::default();
        meter.publish([0.8, 0.3], 0.5);
        assert_eq!(meter.peaks(), [0.8, 0.3]);
        assert_eq!(meter.peaks(), [0.8, 0.3]);
        meter.publish([0.1, 0.2], 0.5);
        assert_eq!(meter.peaks(), [0.4, 0.2]);
        meter.publish([f32::NAN, f32::INFINITY], 0.5);
        assert_eq!(meter.peaks(), [0.2, 0.1]);
        meter.clear();
        assert_eq!(meter.peaks(), [0.0; 2]);
    }
}
