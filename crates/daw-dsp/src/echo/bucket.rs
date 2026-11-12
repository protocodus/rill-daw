//! A bucket-brigade delay line after Holters and Parker: a true sampled
//! line at its own clock rate, with its analog input and output filters
//! integrated analytically at the clock's tick times. Bandwidth, hold
//! droop, aliasing and the repitch of a clock change come from the clock
//! itself; there is no interpolator.
use crate::Complex;

const TABLE: usize = 256;
/// Angles of the upper-half poles of a fourth-order Butterworth filter.
const POLE_ANGLES: [f64; 2] = [
    5.0 * std::f64::consts::FRAC_PI_8,
    7.0 * std::f64::consts::FRAC_PI_8,
];
const ZERO: Complex = Complex::new(0.0, 0.0);

#[inline]
fn flushed(value: Complex) -> Complex {
    if value.re.abs() + value.im.abs() < 1e-20 {
        ZERO
    } else {
        value
    }
}

/// The filter on either side of a [`BucketLine`]: a fourth-order
/// Butterworth low-pass as a sum of first-order sections, `Σ r/(s − p)`,
/// with what a section does over a fraction of a sample tabulated. Only the
/// two poles of the upper half-plane are kept; the others mirror them.
#[derive(Clone, Debug)]
pub struct BucketFilter {
    corner_hz: f64,
    /// `r/fs`: an input sample's weight in a tick's value.
    weights: [Complex; 2],
    /// `r/p`: a step's weight in the output's state.
    steps: [Complex; 2],
    /// `e^(p/fs)`: one sample.
    advance: [Complex; 2],
    /// `e^(p·t/fs)` for `t` from 0 to 1.
    table: Box<[[Complex; 2]; TABLE + 1]>,
}

impl Default for BucketFilter {
    fn default() -> Self {
        let mut filter = Self {
            corner_hz: 0.0,
            weights: [ZERO; 2],
            steps: [ZERO; 2],
            advance: [ZERO; 2],
            table: Box::new([[ZERO; 2]; TABLE + 1]),
        };
        filter.set(8000.0, 48_000.0);
        filter
    }
}

impl BucketFilter {
    pub fn corner_hz(&self) -> f64 {
        self.corner_hz
    }

    /// Moves the corner. The states of a line carry over.
    pub fn set(&mut self, corner_hz: f64, sample_rate: f64) {
        self.corner_hz = corner_hz;
        let radians = std::f64::consts::TAU * corner_hz;
        let unit: [Complex; 4] = [
            Complex::unit(POLE_ANGLES[0]),
            Complex::unit(POLE_ANGLES[1]),
            Complex::unit(-POLE_ANGLES[0]),
            Complex::unit(-POLE_ANGLES[1]),
        ];
        for pole in 0..2 {
            let apart = (0..4)
                .filter(|other| *other != pole)
                .fold(Complex::ONE, |product, other| {
                    product * (unit[pole] - unit[other])
                });
            // r = ω⁴/Π(p − p′), with every pole on the circle of radius ω.
            let residue = Complex::real(radians) / apart;
            self.weights[pole] = residue.scale(1.0 / sample_rate);
            self.steps[pole] = residue / unit[pole].scale(radians);
            let exponent = unit[pole].scale(radians / sample_rate);
            let power = |t: f64| Complex::unit(exponent.im * t).scale((exponent.re * t).exp());
            self.advance[pole] = power(1.0);
            // A geometric run of the table's step.
            let step = power(1.0 / TABLE as f64);
            let mut value = Complex::ONE;
            for row in self.table.iter_mut() {
                row[pole] = value;
                value = value * step;
            }
        }
    }

    /// `e^(p·t/fs)` of both poles, `t` in samples from 0 to 1.
    #[inline]
    fn power(&self, t: f64) -> [Complex; 2] {
        let place = t.clamp(0.0, 1.0) * TABLE as f64;
        let index = (place as usize).min(TABLE - 1);
        let share = place - index as f64;
        let (near, far) = (&self.table[index], &self.table[index + 1]);
        [0, 1].map(|pole| near[pole] + (far[pole] - near[pole]).scale(share))
    }

    /// Magnitude of the analog filter with a corner at `corner_hz`.
    pub fn magnitude(corner_hz: f64, hz: f64) -> f64 {
        1.0 / (1.0 + (hz / corner_hz).powi(8)).sqrt()
    }

    /// Seconds by which the filter delays low frequencies.
    pub fn group_delay(corner_hz: f64) -> f64 {
        2.613_126 / (std::f64::consts::TAU * corner_hz)
    }
}

/// What a bucket does to the charge it passes on: a little even and odd
/// distortion, between rails at ±2. Monotonic on that range.
#[inline]
fn bucket(x: f32) -> f32 {
    let x = x.clamp(-2.0, 2.0);
    x - 0.05 * x * x - x * x * x / 18.0
}

/// The line itself: a queue of buckets filled at the clock's input ticks
/// and read at its output ticks, which fall between them. Up to four
/// stages are read at once: the last (the main head, which may feed back)
/// through one output filter, the sum of the others through a second.
#[derive(Clone, Debug)]
pub struct BucketLine {
    queue: Vec<f32>,
    written: usize,
    input: [Complex; 2],
    held: [f32; 2],
    output: [[Complex; 2]; 2],
    /// Samples until the next tick.
    next: f64,
    input_next: bool,
}

impl Default for BucketLine {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            written: 0,
            input: [ZERO; 2],
            held: [0.0; 2],
            output: [[ZERO; 2]; 2],
            next: 0.0,
            input_next: true,
        }
    }
}

impl BucketLine {
    /// Input ticks the queue holds: twice as many stages.
    pub const QUEUE: usize = 32_768;
    /// Index of the main head among the four.
    pub const MAIN: usize = 3;
    /// The shortest clock half-period in samples: a clock of 2.5 times the
    /// sample rate.
    pub const SHORTEST_HALF_PERIOD: f64 = 0.2;

    /// Call from `prepare` only.
    pub fn allocate(&mut self) {
        self.queue = vec![0.0; Self::QUEUE];
        self.reset();
    }

    pub fn reset(&mut self) {
        self.queue.fill(0.0);
        self.written = 0;
        self.input = [ZERO; 2];
        self.held = [0.0; 2];
        self.output = [[ZERO; 2]; 2];
        self.next = 0.0;
        self.input_next = true;
    }

    /// One sample; the outputs are for the next one. `half_period` is half
    /// a clock period in samples (`fs/(2·clock)`). `offsets` are the input
    /// ticks each head reads behind the newest bucket (a line of `N`
    /// stages ends `N/2 − 1` behind, for a delay of `N/(2·clock)`), `gains`
    /// those of the heads before the main one. Returns the main head and
    /// the sum of the others.
    #[inline]
    pub fn process(
        &mut self,
        input: f32,
        filter: &BucketFilter,
        half_period: f64,
        offsets: &[usize; 4],
        gains: &[f32; 3],
    ) -> [f32; 2] {
        if self.queue.is_empty() {
            return [0.0; 2];
        }
        let mask = Self::QUEUE - 1;
        for pole in 0..2 {
            self.input[pole] =
                flushed(self.input[pole] * filter.advance[pole] + Complex::real(f64::from(input)));
            for output in &mut self.output {
                output[pole] = flushed(output[pole] * filter.advance[pole]);
            }
        }
        let half_period = half_period.max(Self::SHORTEST_HALF_PERIOD);
        while self.next < 1.0 {
            if self.input_next {
                let power = filter.power(self.next);
                let value: f64 = (0..2)
                    .map(|pole| (filter.weights[pole] * power[pole] * self.input[pole]).re)
                    .sum();
                self.written = (self.written + 1) & mask;
                self.queue[self.written] = bucket(2.0 * value as f32);
            } else {
                let stage = |offset: usize| self.queue[self.written.wrapping_sub(offset) & mask];
                let mut others = 0.0;
                for (offset, gain) in offsets.iter().zip(gains) {
                    if *gain != 0.0 {
                        others += gain * stage(*offset);
                    }
                }
                let read = [stage(offsets[Self::MAIN]), others];
                let power = filter.power(1.0 - self.next);
                for (head, read) in read.into_iter().enumerate() {
                    let step = read - self.held[head];
                    if step != 0.0 {
                        self.held[head] = read;
                        for (pole, power) in power.into_iter().enumerate() {
                            self.output[head][pole] = self.output[head][pole]
                                + (filter.steps[pole] * power).scale(f64::from(step));
                        }
                    }
                }
            }
            self.input_next = !self.input_next;
            self.next += half_period;
        }
        self.next -= 1.0;
        [0, 1].map(|head| {
            self.held[head] + 2.0 * (self.output[head][0].re + self.output[head][1].re) as f32
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    struct Line {
        line: BucketLine,
        filter: BucketFilter,
        half_period: f64,
        offsets: [usize; 4],
    }

    fn line(stages: usize, clock: f64, corner: f64) -> Line {
        let mut line = BucketLine::default();
        line.allocate();
        let mut filter = BucketFilter::default();
        filter.set(corner, RATE);
        Line {
            line,
            filter,
            half_period: RATE / (2.0 * clock),
            offsets: [0, 0, 0, stages / 2 - 1],
        }
    }

    impl Line {
        fn process(&mut self, input: f32) -> f32 {
            self.line.process(
                input,
                &self.filter,
                self.half_period,
                &self.offsets,
                &[0.0; 3],
            )[0]
        }

        /// Amplitude of a settled sine of `amplitude` at `hz`, and of its
        /// second and third harmonic.
        fn harmonics(&mut self, hz: f64, amplitude: f64) -> [f64; 3] {
            const WINDOW: usize = 9600;
            let mut sums = [(0.0, 0.0); 3];
            for n in 0..4 * WINDOW {
                let phase = std::f64::consts::TAU * hz * n as f64 / RATE;
                let y = f64::from(self.process((amplitude * phase.sin()) as f32));
                if n >= 3 * WINDOW {
                    for (harmonic, (sine, cosine)) in sums.iter_mut().enumerate() {
                        let phase = (harmonic + 1) as f64 * phase;
                        *sine += y * phase.sin();
                        *cosine += y * phase.cos();
                    }
                }
            }
            sums.map(|(sine, cosine)| 2.0 * sine.hypot(cosine) / WINDOW as f64)
        }
    }

    #[test]
    fn the_delay_is_the_stages_over_twice_the_clock() {
        for (stages, clock, corner) in [
            (1024, 20_000.0, 7000.0),
            (4096, 11_317.0, 3960.0),
            (512, 96_000.0, 9600.0),
            (8, 2000.0, 700.0),
        ] {
            let mut line = line(stages, clock, corner);
            // The first moment of a low-pass's impulse response is its
            // delay at low frequencies.
            let (mut sum, mut moment) = (0.0, 0.0);
            let frames = (stages as f64 / (2.0 * clock) * RATE) as usize + 4000;
            for n in 0..frames {
                let y = f64::from(line.process(if n == 0 { 1e-3 } else { 0.0 }));
                sum += y;
                // The output is the next sample's.
                moment += y * (n + 1) as f64;
            }
            let expected =
                (stages as f64 / (2.0 * clock) + 2.0 * BucketFilter::group_delay(corner)) * RATE;
            let delay = moment / sum;
            assert!(
                (delay - expected).abs() <= 1.0,
                "{stages} stages at {clock} Hz: {delay} samples, not {expected}"
            );
        }
    }

    #[test]
    fn the_response_is_the_hold_and_the_two_filters() {
        for (clock, corner) in [(20_000.0, 7000.0), (6000.0, 2100.0), (60_000.0, 9600.0)] {
            for share in [0.05, 0.2, 0.5, 0.8] {
                let hz = (share * corner / 5.0_f64).round() * 5.0;
                let measured = line(256, clock, corner).harmonics(hz, 0.01)[0] / 0.01;
                let x = std::f64::consts::PI * hz / clock;
                let expected = x.sin() / x * BucketFilter::magnitude(corner, hz).powi(2);
                let error = 20.0 * (measured / expected).log10();
                assert!(
                    error.abs() <= 0.5,
                    "clock {clock}, {hz} Hz: {error} dB off {expected}"
                );
            }
        }
    }

    #[test]
    fn a_constant_passes_at_unit_gain_whatever_the_clock() {
        let settled = |clock: f64, corner: f64| -> Vec<f64> {
            let mut line = line(64, clock, corner);
            (0..6000)
                .map(|_| f64::from(line.process(1e-3)) / 1e-3)
                .skip(4000)
                .collect()
        };
        // Clocks that divide the sample rate tick at one phase for ever.
        for (clock, corner) in [(48_000.0, 9600.0), (24_000.0, 8400.0), (12_000.0, 4200.0)] {
            for value in settled(clock, corner) {
                assert!((value - 1.0).abs() < 0.005, "clock {clock}: {value}");
            }
        }
        // Others pass every phase: the mean is exact.
        for (clock, corner) in [(17_777.0, 6222.0), (5123.0, 1793.0), (71_003.0, 9600.0)] {
            let values = settled(clock, corner);
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            assert!((mean - 1.0).abs() < 5e-4, "clock {clock}: mean {mean}");
            assert!(values.iter().all(|value| (value - 1.0).abs() < 0.005));
        }
    }

    #[test]
    fn buckets_distort_a_little_and_more_when_driven() {
        // Peaks of 0.5 and of 1 in the line: 1.3 % and 3.0 %.
        for (amplitude, low, high) in [(0.5, 0.010, 0.016), (1.0, 0.025, 0.035)] {
            let [first, second, third] = line(64, 20_000.0, 7000.0).harmonics(200.0, amplitude);
            let distortion = second.hypot(third) / first;
            assert!(
                (low..=high).contains(&distortion),
                "peak {amplitude}: {distortion}"
            );
            assert!(second > third, "even above odd: {second} {third}");
        }
        assert_eq!(bucket(0.0), 0.0);
        assert_eq!(bucket(9.0), bucket(2.0));
        let mut previous = bucket(-2.0);
        for step in 1..=400 {
            let value = bucket(-2.0 + step as f32 / 100.0);
            assert!(value > previous);
            previous = value;
        }
    }

    #[test]
    fn heads_read_earlier_stages_and_silence_ends_in_exact_zeros() {
        let mut line = line(2048, 20_000.0, 7000.0);
        line.offsets = [255, 511, 0, 1023];
        let mut main = Vec::new();
        let mut others = Vec::new();
        for n in 0..6000 {
            let x = if n < 48 { 0.1 } else { 0.0 };
            let [a, b] = line.line.process(
                x,
                &line.filter,
                line.half_period,
                &line.offsets,
                &[1.0, 0.5, 0.0],
            );
            main.push(a);
            others.push(b);
        }
        let peak = |signal: &[f32]| {
            signal
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(index, value)| (index as f64, *value))
                .unwrap()
        };
        // A quarter and a half of the line: 12.8 and 25.6 ms.
        let (main_at, main_level) = peak(&main);
        let (first_at, first_level) = peak(&others[..900]);
        let (second_at, second_level) = peak(&others[900..]);
        assert!((main_at / 48.0 - 51.7).abs() < 0.5, "{main_at}");
        assert!((first_at / 48.0 - 13.3).abs() < 0.5, "{first_at}");
        assert!(
            ((second_at + 900.0) / 48.0 - 26.1).abs() < 0.5,
            "{second_at}"
        );
        assert!((first_level / main_level - 1.0).abs() < 0.02);
        assert!((second_level / main_level - 0.5).abs() < 0.02);
        assert!(
            main[5000..]
                .iter()
                .chain(&others[5000..])
                .all(|x| *x == 0.0)
        );
    }
}
