//! Streaming inter-sample peak estimation on the optional analysis worker.
//!
//! The 48-tap, four-phase FIR coefficients are the numerical example in
//! ITU-R BS.1770 Annex 2 (BS.1770-3, p.18). Floating point needs no initial
//! integer-headroom attenuation. Each stage is 2x or 4x; stages are cascaded
//! until the measured sample rate is at least 192 kHz. This is an estimate,
//! not a claim of certified meter conformance or reconstructed analogue output.
//! <https://www.itu.int/dms_pubrec/itu-r/rec/bs/R-REC-BS.1770-3-201208-S!!PDF-E.pdf>

const PHASES: [[f64; 12]; 4] = [
    [
        0.0017089843750,
        0.0109863281250,
        -0.0196533203125,
        0.0332031250000,
        -0.0594482421875,
        0.1373291015625,
        0.9721679687500,
        -0.1022949218750,
        0.0476074218750,
        -0.0266113281250,
        0.0148925781250,
        -0.0083007812500,
    ],
    [
        -0.0291748046875,
        0.0292968750000,
        -0.0517578125000,
        0.0891113281250,
        -0.1665039062500,
        0.4650878906250,
        0.7797851562500,
        -0.2003173828125,
        0.1015625000000,
        -0.0582275390625,
        0.0330810546875,
        -0.0189208984375,
    ],
    [
        -0.0189208984375,
        0.0330810546875,
        -0.0582275390625,
        0.1015625000000,
        -0.2003173828125,
        0.7797851562500,
        0.4650878906250,
        -0.1665039062500,
        0.0891113281250,
        -0.0517578125000,
        0.0292968750000,
        -0.0291748046875,
    ],
    [
        -0.0083007812500,
        0.0148925781250,
        -0.0266113281250,
        0.0476074218750,
        -0.1022949218750,
        0.9721679687500,
        0.1373291015625,
        -0.0594482421875,
        0.0332031250000,
        -0.0196533203125,
        0.0109863281250,
        0.0017089843750,
    ],
];

#[derive(Clone, Copy)]
struct Stage {
    // Duplicated ring permits contiguous dot products without per-tap modulo.
    history: [[f64; 2]; 98],
    coefficients: [[f64; 49]; 4],
    cursor: usize,
    factor: usize,
    taps: usize,
}

impl Stage {
    fn new(factor: usize, reference: bool) -> Self {
        let mut coefficients = [[0.0; 49]; 4];
        let taps = if reference { 12 } else { 49 };
        for (index, coefficients) in coefficients[..factor].iter_mut().enumerate() {
            if reference {
                let phase = if factor == 2 { index * 2 + 1 } else { index };
                coefficients[..12].copy_from_slice(&PHASES[phase]);
            } else {
                // Higher oversampling uses longer Blackman-windowed sinc
                // interpolators, avoiding accumulation of the short example
                // filter's passband/phase ripple at low input rates.
                for (tap, coefficient) in coefficients.iter_mut().enumerate() {
                    let delta = tap as f64 - 24.0 + index as f64 / factor as f64;
                    let sinc = if delta.abs() < 1e-12 {
                        1.0
                    } else {
                        (std::f64::consts::PI * delta).sin() / (std::f64::consts::PI * delta)
                    };
                    let angle = std::f64::consts::PI * delta / 24.5;
                    let window = if delta.abs() <= 24.5 {
                        0.42 + 0.5 * angle.cos() + 0.08 * (2.0 * angle).cos()
                    } else {
                        0.0
                    };
                    *coefficient = sinc * window;
                }
                let sum: f64 = coefficients.iter().sum();
                for coefficient in coefficients {
                    *coefficient /= sum;
                }
            }
        }
        Self {
            history: [[0.0; 2]; 98],
            coefficients,
            cursor: 0,
            factor,
            taps,
        }
    }

    fn process(&mut self, frame: [f64; 2]) -> [[f64; 2]; 4] {
        self.history[self.cursor] = frame;
        self.history[self.cursor + self.taps] = frame;
        let mut output = [[0.0; 2]; 4];
        for (index, output) in output[..self.factor].iter_mut().enumerate() {
            // The fixed fractional latency belongs only to the meter.
            for (input, coefficient) in self.history[self.cursor + 1..self.cursor + 1 + self.taps]
                .iter()
                .rev()
                .zip(&self.coefficients[index][..self.taps])
            {
                output[0] += input[0] * coefficient;
                output[1] += input[1] * coefficient;
            }
        }
        self.cursor = (self.cursor + 1) % self.taps;
        output
    }
}

pub(super) struct TruePeak {
    stages: Vec<Stage>,
    factor: u8,
}

impl TruePeak {
    pub(super) fn new(rate: u32) -> Self {
        let mut stages = Vec::with_capacity(3);
        let mut oversampled = rate.max(8_000);
        let mut factor = 1;
        while oversampled < 192_000 {
            let step = if oversampled.saturating_mul(4) <= 192_000 {
                4
            } else {
                2
            };
            stages.push(Stage::new(step, stages.is_empty()));
            oversampled *= step as u32;
            factor *= step as u8;
        }
        Self { stages, factor }
    }

    pub(super) fn factor(&self) -> u8 {
        self.factor
    }

    pub(super) fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.history.fill([0.0; 2]);
            stage.cursor = 0;
        }
    }

    pub(super) fn process(&mut self, frame: [f32; 2]) -> [f64; 2] {
        let frame = frame.map(|value| {
            if value.is_finite() {
                f64::from(value)
            } else {
                0.0
            }
        });
        let mut peaks = frame.map(f64::abs);
        Self::cascade(&mut self.stages, frame, &mut peaks);
        peaks
    }

    fn cascade(stages: &mut [Stage], frame: [f64; 2], peaks: &mut [f64; 2]) {
        if let Some((stage, rest)) = stages.split_first_mut() {
            let output = stage.process(frame);
            for sample in output.into_iter().take(stage.factor) {
                Self::cascade(rest, sample, peaks);
            }
        } else {
            peaks[0] = peaks[0].max(frame[0].abs());
            peaks[1] = peaks[1].max(frame[1].abs());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstruction_exposes_quarter_rate_inter_sample_peaks_and_keeps_stereo_independent() {
        for rate in [8_000, 44_100, 48_000, 96_000] {
            let mut detector = TruePeak::new(rate);
            assert!(rate * u32::from(detector.factor()) >= 192_000);
            let mut sample = 0.0_f64;
            let mut measured = [0.0_f64; 2];
            for index in 0..1024 {
                let left = (std::f64::consts::FRAC_PI_2 * index as f64
                    + std::f64::consts::FRAC_PI_4)
                    .sin() as f32;
                sample = sample.max(f64::from(left.abs()));
                let peak = detector.process([left, left * 0.25]);
                if index > 64 {
                    measured[0] = measured[0].max(peak[0]);
                    measured[1] = measured[1].max(peak[1]);
                }
            }
            assert!((20.0 * sample.log10() + 3.0103).abs() < 0.001);
            assert!(
                (20.0 * measured[0].log10()).abs() < 0.35,
                "rate {rate}: {measured:?}"
            );
            assert!((measured[1] / measured[0] - 0.25).abs() < 1e-9);
        }
    }

    #[test]
    fn finite_extremes_reset_and_silence_do_not_create_invalid_levels() {
        let mut detector = TruePeak::new(48_000);
        assert_eq!(detector.process([f32::NAN, f32::INFINITY]), [0.0; 2]);
        assert!(
            detector
                .process([f32::MAX, -f32::MAX])
                .iter()
                .all(|value| value.is_finite())
        );
        detector.reset();
        for _ in 0..32 {
            assert_eq!(detector.process([0.0; 2]), [0.0; 2]);
        }
    }
}
