//! Spectrum of a telemetry ring for device panels. Runs on the UI side: the
//! audio thread only writes samples.
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// Level a bin falls to when nothing feeds it.
pub const SPECTRUM_FLOOR_DB: f32 = -120.0;
/// How fast a bin falls once its level drops; it rises at once.
const RELEASE_DB_PER_SECOND: f32 = 20.0;

/// Windowed FFT with peak-hold ballistics per bin. A full-scale sine reads
/// 0 dB at its bin.
pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    levels: Vec<f32>,
    settled: bool,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: f32) -> Self {
        // About 6 Hz per bin at any rate: enough bins below 100 Hz for a
        // smooth trace there.
        let size = if sample_rate <= 50_000.0 {
            8192
        } else {
            16_384
        };
        let fft = FftPlanner::new().plan_fft_forward(size);
        Self {
            window: (0..size)
                .map(|n| 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / size as f32).cos())
                .collect(),
            buffer: vec![Complex32::default(); size],
            scratch: vec![Complex32::default(); fft.get_inplace_scratch_len()],
            levels: vec![SPECTRUM_FLOOR_DB; size / 2 + 1],
            settled: true,
            fft,
        }
    }

    /// Frames one analysis takes; read this many from the telemetry ring.
    pub fn size(&self) -> usize {
        self.window.len()
    }

    pub fn is_settled(&self) -> bool {
        self.settled
    }

    /// Decays the spectrum smoothly when no new audio telemetry arrived,
    /// skipping FFT work once all bins have settled at the floor. Returns the
    /// levels slice and whether any bin is still above the floor.
    pub fn decay_silence(&mut self, seconds: f32) -> (&[f32], bool) {
        if self.settled {
            return (&self.levels, false);
        }
        let fall = RELEASE_DB_PER_SECOND * seconds.max(0.0);
        let mut active = false;
        for level in &mut self.levels {
            let next = (*level - fall).max(SPECTRUM_FLOOR_DB);
            *level = next;
            if next > SPECTRUM_FLOOR_DB {
                active = true;
            }
        }
        self.settled = !active;
        (&self.levels, active)
    }

    /// Analyzes the latest frames (oldest first; fewer than [`Self::size`]
    /// are padded with silence) `seconds` after the previous call, and
    /// returns the level in dB of each bin from 0 Hz to Nyquist.
    pub fn update(&mut self, frames: &[f32], seconds: f32) -> &[f32] {
        let size = self.size();
        let frames = &frames[frames.len().saturating_sub(size)..];
        let padding = size - frames.len();
        self.buffer[..padding].fill(Complex32::default());
        for ((slot, frame), window) in self.buffer[padding..]
            .iter_mut()
            .zip(frames)
            .zip(&self.window[padding..])
        {
            *slot = Complex32::new(
                if frame.is_finite() {
                    frame * window
                } else {
                    0.0
                },
                0.0,
            );
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        let fall = RELEASE_DB_PER_SECOND * seconds.max(0.0);
        // The Hann window halves a sine's peak and the one-sided spectrum
        // halves it again.
        let scale = 4.0 / size as f32;
        let scale2 = scale * scale;
        const MIN_SQR: f32 = 1e-12;
        let mut active = false;
        for (level, bin) in self.levels.iter_mut().zip(&self.buffer) {
            let db = 10.0 * (bin.norm_sqr() * scale2).max(MIN_SQR).log10();
            let next = db.max(*level - fall).max(SPECTRUM_FLOOR_DB);
            *level = next;
            if next > SPECTRUM_FLOOR_DB {
                active = true;
            }
        }
        self.settled = !active;
        &self.levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(bin: f32, size: usize) -> Vec<f32> {
        (0..size)
            .map(|n| (std::f32::consts::TAU * bin * n as f32 / size as f32).sin())
            .collect()
    }

    #[test]
    fn a_full_scale_sine_reads_zero_db_at_its_bin() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            let mut analyzer = SpectrumAnalyzer::new(rate);
            let size = analyzer.size();
            assert_eq!(size, if rate > 50_000.0 { 16_384 } else { 8192 });
            // On a bin and halfway between two, where the window loses most.
            for bin in [200.0, 200.5] {
                let levels = analyzer.update(&sine(bin, size), 10.0);
                assert_eq!(levels.len(), size / 2 + 1);
                let (peak, level) = levels
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .unwrap();
                assert!(peak == 200 || peak == 201, "{peak}");
                assert!(level.abs() <= 1.5, "bin {bin} at {rate}: {level} dB");
                assert!(levels[size / 4] < -100.0, "leakage {}", levels[size / 4]);
            }
        }
    }

    #[test]
    fn bins_rise_at_once_and_fall_twenty_db_per_second() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        let size = analyzer.size();
        let tone = sine(300.0, size);
        let silence = vec![0.0; size];
        let start = analyzer.update(&tone, 0.0)[300];
        let mut level = start;
        for _ in 0..30 {
            level = analyzer.update(&silence, 1.0 / 30.0)[300];
        }
        let fallen = start - level;
        assert!((18.0..=22.0).contains(&fallen), "{fallen} dB in one second");
        assert_eq!(analyzer.update(&tone, 1.0 / 30.0)[300], start);
        for _ in 0..10 {
            level = analyzer.update(&silence, 1.0)[300];
        }
        assert_eq!(level, SPECTRUM_FLOOR_DB);
    }

    #[test]
    fn short_and_long_inputs_use_the_latest_frames() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        let size = analyzer.size();
        // Half a window of a tone still shows the tone.
        let short = analyzer.update(&sine(300.0, size)[..size / 2], 0.0)[300];
        assert!((-12.0..0.0).contains(&short), "{short}");
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        let mut long = vec![0.0; size];
        long.extend(sine(300.0, size));
        assert!(analyzer.update(&long, 0.0)[300].abs() < 0.1);
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        assert!(
            analyzer
                .update(&[f32::NAN; 64], 0.0)
                .iter()
                .all(|level| *level == SPECTRUM_FLOOR_DB)
        );
    }
}
