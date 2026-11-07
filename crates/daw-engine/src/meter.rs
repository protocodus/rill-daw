//! Immutable channel lookup and coherent stereo telemetry. Audio graph workers
//! publish once per block; UI reads never consume peaks or touch the callback.
use daw_core::{Project, TrackKind};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Instant attack and 24 dB/second release, independent of callback/block size.
pub(crate) fn release_factor(frames: usize, sample_rate: u32) -> f32 {
    (-std::f32::consts::LN_10 * 1.2 * frames as f32 / sample_rate as f32).exp()
}

#[derive(Default)]
pub(crate) struct ChannelMeter {
    stereo: AtomicU64,
    held: AtomicU64,
    rms: AtomicU64,
}

impl ChannelMeter {
    /// Post-fader level telemetry. RMS uses an exponential mean-square detector
    /// with a 300 ms time constant, not a VU calibration or a 300 ms boxcar.
    /// Single graph writer; coherent stereo atomic publication, no allocation.
    pub(crate) fn publish_audio(&self, frames: &[[f32; 2]], sample_rate: u32) {
        if frames.is_empty() {
            return;
        }
        let coefficient = (-1.0 / (f64::from(sample_rate.max(1)) * 0.3)).exp();
        let previous = self.rms();
        let mut energy = previous.map(|value| f64::from(value).powi(2));
        let mut peaks = [0.0_f32; 2];
        for frame in frames {
            for channel in 0..2 {
                let value = if frame[channel].is_finite() {
                    frame[channel]
                } else {
                    0.0
                };
                peaks[channel] = peaks[channel].max(value.abs());
                energy[channel] =
                    coefficient * energy[channel] + (1.0 - coefficient) * f64::from(value).powi(2);
            }
        }
        let rms = energy.map(|value| value.max(0.0).sqrt().min(f64::from(f32::MAX)) as f32);
        self.rms.store(
            u64::from(rms[0].to_bits()) | (u64::from(rms[1].to_bits()) << 32),
            Ordering::Relaxed,
        );
        self.publish(peaks, release_factor(frames.len(), sample_rate.max(1)));
    }

    pub(crate) fn publish(&self, peaks: [f32; 2], decay: f32) {
        let peaks = peaks.map(|peak| if peak.is_finite() { peak.abs() } else { 0.0 });
        // Capture raw block maxima before ballistics. A UI frame may arrive
        // much later, or the mixer may be closed when this transient occurs.
        let _ = self
            .held
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |packed| {
                let previous = [
                    f32::from_bits(packed as u32),
                    f32::from_bits((packed >> 32) as u32),
                ];
                let next = u64::from(peaks[0].max(previous[0]).to_bits())
                    | (u64::from(peaks[1].max(previous[1]).to_bits()) << 32);
                (next != packed).then_some(next)
            });
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
        self.rms.store(0, Ordering::Relaxed);
    }

    pub(crate) fn rms(&self) -> [f32; 2] {
        let packed = self.rms.load(Ordering::Relaxed);
        [
            f32::from_bits(packed as u32),
            f32::from_bits((packed >> 32) as u32),
        ]
    }

    pub(crate) fn held(&self) -> [f32; 2] {
        let packed = self.held.load(Ordering::Relaxed);
        [
            f32::from_bits(packed as u32),
            f32::from_bits((packed >> 32) as u32),
        ]
    }

    pub(crate) fn reset_hold(&self) {
        self.held.store(0, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub(crate) struct ChannelMeters {
    channels: Vec<(u64, Arc<ChannelMeter>)>,
}

impl ChannelMeters {
    pub(crate) fn new(project: &Project) -> Self {
        let mut channels: Vec<_> = project
            .channels()
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

    pub(crate) fn rms(&self, id: u64) -> [f32; 2] {
        self.channel(id)
            .map(|meter| meter.rms())
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

    #[test]
    fn peak_hold_captures_transients_survives_silence_and_resets_independently() {
        let meter = ChannelMeter::default();
        meter.publish([1.25, -0.75], 0.0);
        meter.publish([0.0; 2], 0.0);
        assert_eq!(meter.peaks(), [0.0; 2]);
        assert_eq!(meter.held(), [1.25, 0.75]);
        meter.clear();
        assert_eq!(meter.held(), [1.25, 0.75]);
        meter.reset_hold();
        assert_eq!(meter.held(), [0.0; 2]);
        meter.publish([0.4, 0.2], 0.0);
        meter.publish([f32::NAN, f32::INFINITY], 0.0);
        assert_eq!(meter.held(), [0.4, 0.2]);
        meter.reset_hold();
        // Resetting must not clear audible meter levels or stop measurement.
        meter.publish([1.1, 0.3], 0.0);
        meter.reset_hold();
        assert_eq!(meter.peaks(), [1.1, 0.3]);
        meter.publish([1.1, 0.3], 0.0);
        assert_eq!(meter.held(), [1.1, 0.3]);
    }

    #[test]
    fn rms_has_a_calibrated_300_ms_time_constant_without_block_size_dependence() {
        let rate = 48_000;
        let samples = vec![[0.5, -0.25]; rate as usize];
        let expected = (1.0 - (-1.0_f64 / 0.3).exp()).sqrt();
        for block in [1, 64, 256, 511] {
            let meter = ChannelMeter::default();
            for frames in samples.chunks(block) {
                meter.publish_audio(frames, rate);
            }
            for (actual, amplitude) in meter.rms().into_iter().zip([0.5, 0.25]) {
                assert!(
                    (f64::from(actual) - expected * amplitude).abs() < 2e-6,
                    "block={block}: {}",
                    f64::from(actual)
                );
            }
            assert_eq!(meter.peaks(), [0.5, 0.25]);
            assert_eq!(meter.held(), [0.5, 0.25]);
            meter.clear();
            assert_eq!(meter.rms(), [0.0; 2]);
            assert_eq!(meter.held(), [0.5, 0.25]);
        }
    }

    #[test]
    fn rms_publication_is_allocation_free_and_finite_with_floating_headroom() {
        let meter = ChannelMeter::default();
        let frames = [[1.5, 0.75]; 256];
        assert_eq!(
            crate::allocation_test_support::count_allocations(|| {
                meter.publish_audio(&frames, 48_000);
            }),
            0
        );
        assert_eq!(meter.peaks(), [1.5, 0.75]);
        assert!(meter.rms()[0] > meter.rms()[1]);
        meter.publish_audio(&[[f32::NAN, f32::INFINITY], [f32::MAX, -f32::MAX]], 48_000);
        assert!(meter.rms().iter().all(|value| value.is_finite()));
        assert!(meter.rms()[0] > 0.0);
    }
}
