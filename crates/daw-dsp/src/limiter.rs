//! The pieces of a lookahead limiter: a sliding maximum that holds each peak
//! for the lookahead, a moving average that turns the held steps into
//! ramps, and a detector for peaks between the samples.
use super::{OversampleFactor, Oversampler};

/// Maximum of the last `window` values: a monotonic wedge (Lemire) on a ring
/// allocated once. Amortised constant time; one call never does more work
/// than the window is long.
#[derive(Clone, Debug, Default)]
pub struct SlidingMax {
    /// Candidates, newest last, values falling: `(value, time)`.
    wedge: Vec<(f32, u32)>,
    head: usize,
    len: usize,
    window: u32,
    time: u32,
}

impl SlidingMax {
    /// Room for windows up to `max_window`. Call from `prepare` only.
    pub fn allocate(&mut self, max_window: usize) {
        self.wedge = vec![(0.0, 0); max_window.max(1).next_power_of_two()];
        self.set_window(1, 0.0);
    }

    /// Sets the window (at least 1, at most the allocation) and starts over
    /// as if every value in it had been `fill`.
    pub fn set_window(&mut self, window: usize, fill: f32) {
        self.window = window.clamp(1, self.wedge.len().max(1)) as u32;
        self.head = 0;
        self.len = 0;
        self.time = 0;
        if let Some(first) = self.wedge.first_mut() {
            *first = (fill, 0);
            self.len = 1;
        }
    }

    #[inline]
    pub fn process(&mut self, value: f32) -> f32 {
        if self.wedge.is_empty() {
            return value;
        }
        let mask = self.wedge.len() - 1;
        self.time = self.time.wrapping_add(1);
        while self.len > 0 && self.wedge[(self.head + self.len - 1) & mask].0 <= value {
            self.len -= 1;
        }
        // The oldest candidate left the window. Checked before the push, so
        // the wedge never holds more than `window` entries.
        if self.len > 0 && self.time.wrapping_sub(self.wedge[self.head].1) >= self.window {
            self.head = (self.head + 1) & mask;
            self.len -= 1;
        }
        self.wedge[(self.head + self.len) & mask] = (value, self.time);
        self.len += 1;
        self.wedge[self.head].0
    }
}

/// Moving average over the last `length` values. The running sum is `f64`
/// and rebuilt from the ring every few thousand samples, so rounding cannot
/// drift.
#[derive(Clone, Debug, Default)]
pub struct BoxSmoother {
    ring: Vec<f32>,
    length: usize,
    cursor: usize,
    sum: f64,
    until_rebuild: u32,
}

const REBUILD: u32 = 4096;

impl BoxSmoother {
    /// Room for lengths up to `max`. Call from `prepare` only.
    pub fn allocate(&mut self, max: usize) {
        self.ring = vec![0.0; max.max(1)];
        self.set_length(1, 0.0);
    }

    /// Sets the length (at least 1, at most the allocation) and starts over
    /// as if every value in it had been `fill`.
    pub fn set_length(&mut self, length: usize, fill: f32) {
        self.length = length.clamp(1, self.ring.len().max(1));
        self.ring.fill(fill);
        self.cursor = 0;
        self.sum = self.length as f64 * f64::from(fill);
        self.until_rebuild = REBUILD;
    }

    #[inline]
    pub fn process(&mut self, value: f32) -> f32 {
        if self.ring.is_empty() {
            return value;
        }
        self.sum += f64::from(value) - f64::from(self.ring[self.cursor]);
        self.ring[self.cursor] = value;
        self.cursor += 1;
        if self.cursor == self.length {
            self.cursor = 0;
        }
        self.until_rebuild -= 1;
        if self.until_rebuild == 0 {
            self.until_rebuild = REBUILD;
            self.sum = self.ring[..self.length]
                .iter()
                .map(|value| f64::from(*value))
                .sum();
        }
        (self.sum / self.length as f64) as f32
    }
}

/// Peak detector that also sees between the samples: the largest magnitude
/// of the signal interpolated to four times the rate. The interpolator
/// passes the original samples unchanged, so sample peaks are always
/// included. A tone at `f` is under-read by at most
/// `20·log10(cos(π·f/(4·fs)))`, 0.47 dB at 20 kHz and 48 kHz.
#[derive(Clone, Debug)]
pub struct TruePeak4x {
    oversampler: Oversampler,
}

impl Default for TruePeak4x {
    fn default() -> Self {
        Self {
            oversampler: Oversampler::new(OversampleFactor::X4),
        }
    }
}

impl TruePeak4x {
    /// A peak at sample `m` and the half samples around it is read in the
    /// calls for samples `m + 18` to `m + 20`.
    pub const DELAY: u32 = 20;

    pub fn reset(&mut self) {
        self.oversampler.reset();
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let mut buffer = [0.0; Oversampler::MAX];
        let count = self.oversampler.upsample(input, &mut buffer);
        buffer[..count]
            .iter()
            .fold(0.0, |peak, sample| peak.max(sample.abs()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    #[test]
    fn sliding_max_matches_brute_force() {
        let mut random = Lcg(3);
        let mut sliding = SlidingMax::default();
        assert_eq!(sliding.process(0.3), 0.3);
        sliding.allocate(300);
        for window in [1, 2, 7, 243, 300, 5] {
            let fill = random.next();
            sliding.set_window(window, fill);
            let mut history = vec![fill; window];
            for n in 0..5000 {
                // Runs of equal and falling values exercise the wedge.
                let value = match (n / 97) % 3 {
                    0 => random.next(),
                    1 => 1.0 - n as f32 * 1e-4,
                    _ => 0.25,
                };
                history.push(value);
                let expected = history[history.len() - window..]
                    .iter()
                    .fold(f32::MIN, |max, x| max.max(*x));
                assert_eq!(sliding.process(value), expected, "window {window} at {n}");
            }
        }
        // A window beyond the allocation is held to it.
        sliding.set_window(100_000, 0.0);
        assert_eq!(sliding.process(1.0), 1.0);
        for _ in 0..511 {
            assert_eq!(sliding.process(0.0), 1.0);
        }
        assert_eq!(sliding.process(0.0), 0.0);
    }

    #[test]
    fn box_smoother_is_a_mean_that_does_not_drift() {
        let mut random = Lcg(9);
        let mut smoother = BoxSmoother::default();
        assert_eq!(smoother.process(0.3), 0.3);
        smoother.allocate(500);
        smoother.set_length(481, 2.0);
        assert_eq!(smoother.process(2.0), 2.0);
        let mut history = vec![2.0_f32; 482];
        let mut last = 0.0;
        for _ in 0..1_000_000 {
            // Large and small values together are where a running sum
            // loses bits.
            let value = if random.next() < 0.01 {
                1e4 * random.next()
            } else {
                1e-3 * random.next()
            };
            history.push(value);
            last = smoother.process(value);
        }
        let mean = history[history.len() - 481..]
            .iter()
            .map(|x| f64::from(*x))
            .sum::<f64>()
            / 481.0;
        assert!(
            (f64::from(last) - mean).abs() < 1e-5 * mean.max(1.0),
            "{last} {mean}"
        );
        // Length 1 passes values through; a step becomes a ramp of the
        // length.
        smoother.set_length(1, 0.0);
        assert_eq!(smoother.process(0.7), 0.7);
        smoother.set_length(4, 0.0);
        let ramp = [1.0; 5].map(|x| smoother.process(x));
        assert_eq!(ramp, [0.25, 0.5, 0.75, 1.0, 1.0]);
    }

    #[test]
    fn true_peak_reads_between_the_samples() {
        // A sine at a quarter of the rate, sampled 45° off its crests:
        // every sample is 3.01 dB below the true peak.
        let mut detector = TruePeak4x::default();
        let mut peak = 0.0_f32;
        let mut sample_peak = 0.0_f32;
        for n in 0..400 {
            let x = (std::f32::consts::FRAC_PI_2 * n as f32 + std::f32::consts::FRAC_PI_4).sin();
            sample_peak = sample_peak.max(x.abs());
            let read = detector.process(x);
            if n > 100 {
                peak = peak.max(read);
            }
        }
        assert!((20.0 * sample_peak.log10() + 3.01).abs() < 0.01);
        assert!((20.0 * peak.log10()).abs() < 0.05, "{peak}");
    }

    #[test]
    fn true_peak_includes_every_sample_within_its_delay() {
        let mut detector = TruePeak4x::default();
        let mut reads = Vec::new();
        for n in 0..80 {
            reads.push(detector.process(if n == 10 { 1.0 } else { 0.0 }));
        }
        let at = |n: usize| reads[n];
        // The impulse itself passes unchanged, inside the stated span.
        let exact = (0..80).find(|n| at(*n) == 1.0).unwrap();
        assert!(
            (10 + 18..=10 + TruePeak4x::DELAY as usize).contains(&exact),
            "{exact}"
        );
        detector.reset();
        assert_eq!(detector.process(0.0), 0.0);
    }
}
