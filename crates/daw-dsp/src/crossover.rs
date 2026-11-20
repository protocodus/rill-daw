use super::{Svf, SvfCoeffs, prewarp};

/// Butterworth damping of each second-order stage: two in series make a
/// Linkwitz–Riley pair whose low and high outputs sum to an allpass.
const K: f64 = std::f64::consts::SQRT_2;

/// Two to four stereo bands from up to three Linkwitz–Riley (24 dB/oct)
/// crossovers. The bands sum back to the input's magnitude exactly: each
/// band below a later split passes the allpass that split leaves on the
/// bands above it, so every band shares one phase response.
#[derive(Clone, Debug, Default)]
pub struct BandSplitter {
    /// `[split][low, high][stage][channel]`
    splits: [[[[Svf<f64>; 2]; 2]; 2]; 3],
    /// `[band][later split][channel]`
    allpasses: [[[Svf<f64>; 2]; 3]; 3],
}

impl BandSplitter {
    pub const MAX_BANDS: usize = 4;

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Coefficients for the crossovers in Hz, lowest first; each is raised
    /// to its predecessor where it is set below it.
    pub fn coefficients(crossovers: [f32; 3], sample_rate: f32) -> [SvfCoeffs<f64>; 3] {
        let mut previous = 0.0_f32;
        crossovers.map(|frequency| {
            previous = previous.max(frequency);
            SvfCoeffs::new(f64::from(prewarp(previous, sample_rate)), K)
        })
    }

    /// One frame into `bands` (2–4) outputs, lowest first, using the first
    /// `bands - 1` crossovers. Outputs past them are silent.
    #[inline]
    pub fn process(
        &mut self,
        input: [f32; 2],
        bands: usize,
        coefficients: &[SvfCoeffs<f64>; 3],
    ) -> [[f32; 2]; Self::MAX_BANDS] {
        let bands = bands.clamp(2, Self::MAX_BANDS);
        let mut out = [[0.0; 2]; Self::MAX_BANDS];
        let mut rest = input.map(f64::from);
        for split in 0..bands - 1 {
            let (mut low, high) = Self::split(&mut self.splits[split], rest, coefficients[split]);
            for later in split + 1..bands - 1 {
                for (channel, sample) in low.iter_mut().enumerate() {
                    let outputs = self.allpasses[split][later][channel]
                        .process_coeffs(*sample, coefficients[later]);
                    *sample = outputs.low - K * outputs.band + outputs.high;
                }
            }
            out[split] = low.map(|sample| sample as f32);
            rest = high;
        }
        out[bands - 1] = rest.map(|sample| sample as f32);
        out
    }

    #[inline]
    fn split(
        stages: &mut [[[Svf<f64>; 2]; 2]; 2],
        input: [f64; 2],
        coefficients: SvfCoeffs<f64>,
    ) -> ([f64; 2], [f64; 2]) {
        let mut low = input;
        let mut high = input;
        for channel in 0..2 {
            for stage in 0..2 {
                low[channel] = stages[0][stage][channel]
                    .process_coeffs(low[channel], coefficients)
                    .low;
                high[channel] = stages[1][stage][channel]
                    .process_coeffs(high[channel], coefficients)
                    .high;
            }
        }
        (low, high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RMS gain of the summed bands for a sine, after the filters settle.
    fn summed_gain(frequency: f32, bands: usize) -> f32 {
        let rate = 48_000.0;
        let coefficients = BandSplitter::coefficients([120.0, 2500.0, 8000.0], rate);
        let mut splitter = BandSplitter::default();
        let (mut input_power, mut output_power) = (0.0_f64, 0.0_f64);
        for index in 0..96_000 {
            let x = (std::f32::consts::TAU * frequency * index as f32 / rate).sin();
            let bands = splitter.process([x, -x], bands, &coefficients);
            let sum: f32 = bands.iter().map(|band| band[0]).sum();
            if index > 48_000 {
                input_power += f64::from(x * x);
                output_power += f64::from(sum * sum);
            }
        }
        (output_power / input_power).sqrt() as f32
    }

    #[test]
    fn bands_sum_flat() {
        for bands in 2..=4 {
            for frequency in [40.0, 120.0, 600.0, 2500.0, 8000.0, 12_000.0] {
                let gain = summed_gain(frequency, bands);
                assert!(
                    (gain - 1.0).abs() < 0.01,
                    "{bands} bands, {frequency} Hz: {gain}"
                );
            }
        }
    }

    #[test]
    fn each_band_holds_its_range() {
        let rate = 48_000.0;
        let coefficients = BandSplitter::coefficients([120.0, 2500.0, 8000.0], rate);
        for (frequency, band) in [(40.0, 0), (600.0, 1), (4000.0, 2), (14_000.0, 3)] {
            let mut splitter = BandSplitter::default();
            let mut peaks = [0.0_f32; 4];
            for index in 0..48_000 {
                let x = (std::f32::consts::TAU * frequency * index as f32 / rate).sin();
                let bands = splitter.process([x, x], 4, &coefficients);
                if index > 24_000 {
                    for (peak, output) in peaks.iter_mut().zip(bands) {
                        *peak = peak.max(output[0].abs());
                    }
                }
            }
            let loudest = (0..4)
                .max_by(|a, b| peaks[*a].total_cmp(&peaks[*b]))
                .unwrap();
            assert_eq!(loudest, band, "{frequency} Hz: {peaks:?}");
        }
    }
}
