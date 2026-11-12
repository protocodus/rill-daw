//! Oversampling with linear-phase half-band FIR stages (Kaiser window): 2×
//! as [`Oversampler2x`], or a cascade for 2×, 4× and 8× as [`Oversampler`].
//! Linear phase, so an oversampled path can be summed with a delayed linear
//! one without combing.

const RING: usize = 64;
const MAX_HALF: usize = 16;

/// One half-band stage of `TAPS` taps (4k + 3, at most 63): doubles or halves
/// the rate. Up- and down-sampling together delay the signal by `TAPS − 1`
/// samples at the upper rate.
#[derive(Clone, Debug)]
pub struct HalfBand<const TAPS: usize> {
    /// The nonzero taps before the center (0.5): every even index. The other
    /// half mirrors them.
    even: [f32; MAX_HALF],
    up: [f32; RING],
    down: [f32; RING],
    up_cursor: usize,
    down_cursor: usize,
}

/// The 63-tap stage, wide enough to be the only one: flat to 18 kHz at
/// 44.1 kHz.
pub type Oversampler2x = HalfBand<63>;

impl Default for Oversampler2x {
    fn default() -> Self {
        Self::new()
    }
}

impl Oversampler2x {
    /// Round trip in base-rate samples.
    pub const LATENCY: u32 = Self::ROUND_TRIP / 2;

    pub fn new() -> Self {
        Self::with_beta(8.0)
    }
}

impl<const TAPS: usize> HalfBand<TAPS> {
    const CENTER: usize = TAPS / 2;
    const HALF: usize = {
        assert!(TAPS % 4 == 3 && TAPS < RING);
        Self::CENTER.div_ceil(2)
    };
    /// Up plus down, in samples at the upper rate.
    pub const ROUND_TRIP: u32 = TAPS as u32 - 1;

    /// `beta` is the Kaiser window's: more stopband depth for a wider
    /// transition.
    pub fn with_beta(beta: f64) -> Self {
        let mut even = [0.0; MAX_HALF];
        for (index, slot) in even[..Self::HALF].iter_mut().enumerate() {
            let n = (2 * index) as f64 - Self::CENTER as f64;
            let sinc = (std::f64::consts::PI * n / 2.0).sin() / (std::f64::consts::PI * n);
            let ratio = n / Self::CENTER as f64;
            let window = bessel_i0(beta * (1.0 - ratio * ratio).max(0.0).sqrt()) / bessel_i0(beta);
            *slot = (sinc * window) as f32;
        }
        Self {
            even,
            up: [0.0; RING],
            down: [0.0; RING],
            up_cursor: 0,
            down_cursor: 0,
        }
    }

    pub fn reset(&mut self) {
        self.up = [0.0; RING];
        self.down = [0.0; RING];
    }

    /// One sample in, two at twice the rate out, oldest first.
    #[inline]
    pub fn upsample(&mut self, input: f32) -> [f32; 2] {
        // The zero-stuffed stream is never stored: history holds only the real
        // samples (scaled by 2 for unity gain), and each output phase uses the
        // taps that meet them. The second phase sits on an input sample and
        // only the center tap (0.5) reaches it.
        self.up_cursor = (self.up_cursor + 1) % RING;
        self.up[self.up_cursor] = 2.0 * input;
        let at = |delay: usize| self.up[(self.up_cursor + RING - delay) % RING];
        // The taps are symmetric, so fold each pair into one multiply.
        let mut between = 0.0;
        for (delay, &tap) in self.even[..Self::HALF].iter().enumerate() {
            between += tap * (at(delay) + at(Self::CENTER - delay));
        }
        [between, 0.5 * at(Self::CENTER / 2)]
    }

    /// Two samples in, one at half the rate out.
    #[inline]
    pub fn downsample(&mut self, input: [f32; 2]) -> f32 {
        for sample in input {
            self.down_cursor = (self.down_cursor + 1) % RING;
            self.down[self.down_cursor] = sample;
        }
        // Filter output for the older sample of the pair, which lands on an
        // integer delay at the lower rate.
        let at = |delay: usize| self.down[(self.down_cursor + RING - 1 - delay) % RING];
        let mut sum = 0.5 * at(Self::CENTER);
        for (index, &tap) in self.even[..Self::HALF].iter().enumerate() {
            sum += tap * (at(2 * index) + at(2 * (Self::CENTER - index)));
        }
        sum
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OversampleFactor {
    X2,
    X4,
    X8,
}

impl OversampleFactor {
    pub const fn factor(self) -> usize {
        match self {
            Self::X2 => 2,
            Self::X4 => 4,
            Self::X8 => 8,
        }
    }
}

/// 2×, 4× or 8× as a cascade of half-band stages. The later stages only
/// have to stop what would fold onto the audio band, so they are short.
///
/// The round trip is a whole number of base-rate samples only together with
/// an inner process that delays by an even number of top-rate samples that
/// is not a multiple of four (see [`Self::latency`]); a centred ADAA shaper
/// with its droop corrector delays by two.
#[derive(Clone, Debug)]
pub struct Oversampler {
    factor: OversampleFactor,
    first: Oversampler2x,
    second: HalfBand<31>,
    third: HalfBand<19>,
}

impl Oversampler {
    pub const MAX: usize = 8;

    pub fn new(factor: OversampleFactor) -> Self {
        Self {
            factor,
            first: Oversampler2x::new(),
            second: HalfBand::with_beta(10.0),
            third: HalfBand::with_beta(10.0),
        }
    }

    pub fn factor(&self) -> OversampleFactor {
        self.factor
    }

    /// Also clears the histories.
    pub fn set_factor(&mut self, factor: OversampleFactor) {
        self.factor = factor;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.first.reset();
        self.second.reset();
        self.third.reset();
    }

    /// Up plus down, in top-rate samples.
    pub const fn round_trip(factor: OversampleFactor) -> u32 {
        let first = Oversampler2x::ROUND_TRIP;
        let second = HalfBand::<31>::ROUND_TRIP;
        let third = HalfBand::<19>::ROUND_TRIP;
        match factor {
            OversampleFactor::X2 => first,
            OversampleFactor::X4 => 2 * first + second,
            OversampleFactor::X8 => 4 * first + 2 * second + third,
        }
    }

    /// Base-rate delay of up, an inner process of two top-rate samples, and
    /// down: 32, 39 or 41.
    pub const fn latency(factor: OversampleFactor) -> u32 {
        (Self::round_trip(factor) + 2) / factor.factor() as u32
    }

    /// One base-rate sample in; `factor` samples out, oldest first. Returns
    /// the count.
    #[inline]
    pub fn upsample(&mut self, input: f32, out: &mut [f32; Self::MAX]) -> usize {
        let first = self.first.upsample(input);
        match self.factor {
            OversampleFactor::X2 => out[..2].copy_from_slice(&first),
            OversampleFactor::X4 => {
                for (pair, sample) in out.chunks_exact_mut(2).zip(first) {
                    pair.copy_from_slice(&self.second.upsample(sample));
                }
            }
            OversampleFactor::X8 => {
                for (quad, sample) in out.chunks_exact_mut(4).zip(first) {
                    for (pair, sample) in quad.chunks_exact_mut(2).zip(self.second.upsample(sample))
                    {
                        pair.copy_from_slice(&self.third.upsample(sample));
                    }
                }
            }
        }
        self.factor.factor()
    }

    /// `factor` samples in, one base-rate sample out.
    #[inline]
    pub fn downsample(&mut self, input: &[f32; Self::MAX]) -> f32 {
        let pair = match self.factor {
            OversampleFactor::X2 => [input[0], input[1]],
            OversampleFactor::X4 => [
                self.second.downsample([input[0], input[1]]),
                self.second.downsample([input[2], input[3]]),
            ],
            OversampleFactor::X8 => {
                let mut quad = [0.0; 4];
                for (slot, pair) in quad.iter_mut().zip(input.chunks_exact(2)) {
                    *slot = self.third.downsample([pair[0], pair[1]]);
                }
                [
                    self.second.downsample([quad[0], quad[1]]),
                    self.second.downsample([quad[2], quad[3]]),
                ]
            }
        };
        self.first.downsample(pair)
    }
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..50 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(frequency: f32, sample_rate: f32) -> (Vec<f32>, Vec<f32>) {
        let mut oversampler = Oversampler2x::new();
        let input: Vec<f32> = (0..4096)
            .map(|n| (std::f32::consts::TAU * frequency * n as f32 / sample_rate).sin())
            .collect();
        let output = input
            .iter()
            .map(|&x| {
                let pair = oversampler.upsample(x);
                oversampler.downsample(pair)
            })
            .collect();
        (input, output)
    }

    #[test]
    fn round_trip_is_a_pure_delay_in_the_passband() {
        let (input, output) = run(1000.0, 48_000.0);
        let latency = Oversampler2x::LATENCY as usize;
        let error = input[..4000]
            .iter()
            .zip(&output[latency..4000 + latency])
            .skip(100)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(error < 1e-3, "{error}");
    }

    #[test]
    fn passband_is_flat_to_eighteen_kilohertz() {
        let (_, output) = run(18_000.0, 48_000.0);
        let peak = output[200..]
            .iter()
            .fold(0.0_f32, |peak, x| peak.max(x.abs()));
        assert!((0.97..1.03).contains(&peak), "{peak}");
    }

    const FACTORS: [OversampleFactor; 3] = [
        OversampleFactor::X2,
        OversampleFactor::X4,
        OversampleFactor::X8,
    ];

    /// Up, a two-sample delay at the top rate, down.
    fn cascade(factor: OversampleFactor, input: &[f32]) -> Vec<f32> {
        let mut oversampler = Oversampler::new(factor);
        let mut delay = [0.0_f32; 2];
        let mut buffer = [0.0; Oversampler::MAX];
        input
            .iter()
            .map(|&x| {
                let count = oversampler.upsample(x, &mut buffer);
                for sample in &mut buffer[..count] {
                    let newer = std::mem::replace(&mut delay[1], *sample);
                    *sample = std::mem::replace(&mut delay[0], newer);
                }
                oversampler.downsample(&buffer)
            })
            .collect()
    }

    fn tone(frequency: f64, rate: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (std::f64::consts::TAU * frequency * n as f64 / rate).sin() as f32)
            .collect()
    }

    #[test]
    fn cascade_latency_is_a_whole_number_of_samples() {
        assert_eq!(FACTORS.map(Oversampler::round_trip), [62, 154, 326]);
        assert_eq!(FACTORS.map(Oversampler::latency), [32, 39, 41]);
        for factor in FACTORS {
            let mut impulse = vec![0.0; 256];
            impulse[10] = 1.0;
            let output = cascade(factor, &impulse);
            let peak = (0..output.len())
                .max_by(|a, b| output[*a].abs().total_cmp(&output[*b].abs()))
                .unwrap();
            assert_eq!(
                peak,
                10 + Oversampler::latency(factor) as usize,
                "{factor:?}"
            );
            assert!(
                (output[peak] - 1.0).abs() < 0.05,
                "{factor:?} {}",
                output[peak]
            );
        }
    }

    #[test]
    fn cascade_is_a_pure_delay_in_the_passband() {
        for factor in FACTORS {
            let latency = Oversampler::latency(factor) as usize;
            let input = tone(1000.0, 48_000.0, 4096);
            let output = cascade(factor, &input);
            let error = input[..4000]
                .iter()
                .zip(&output[latency..])
                .skip(200)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(error < 1e-3, "{factor:?} {error}");
        }
    }

    #[test]
    fn cascade_passband_is_flat() {
        for factor in FACTORS {
            for (frequency, rate) in [(18_000.0, 44_100.0), (20_000.0, 48_000.0), (40.0, 48_000.0)]
            {
                let input = tone(frequency, rate, 8192);
                let output = cascade(factor, &input);
                let level = |signal: &[f32]| {
                    let (mut s, mut c) = (0.0, 0.0);
                    for (n, y) in signal.iter().enumerate() {
                        let phase = std::f64::consts::TAU * frequency * n as f64 / rate;
                        s += f64::from(*y) * phase.sin();
                        c += f64::from(*y) * phase.cos();
                    }
                    s.hypot(c)
                };
                // A whole number of periods would need a length per tone;
                // both signals share the window, so its leakage cancels.
                let gain = 20.0 * (level(&output[1000..7000]) / level(&input[1000..7000])).log10();
                assert!(gain.abs() < 0.1, "{factor:?} {frequency} Hz: {gain} dB");
            }
        }
    }

    /// Top-rate tones that fold into the audio band at the base rate.
    #[test]
    fn cascade_stops_what_would_fold_into_the_audio_band() {
        let rate = 48_000.0;
        for factor in FACTORS {
            let m = factor.factor();
            let top = rate * m as f64;
            let mut worst = 0.0_f64;
            // From where the first stage's stopband begins to the top
            // Nyquist; below it lies the transition band.
            let mut frequency = 28_000.0;
            while frequency < 0.5 * top {
                let folded = {
                    let wrapped = frequency % rate;
                    wrapped.min(rate - wrapped)
                };
                if folded <= 20_000.0 {
                    let mut oversampler = Oversampler::new(factor);
                    let mut peak = 0.0_f64;
                    for n in 0..600 {
                        let mut buffer = [0.0; Oversampler::MAX];
                        for (k, slot) in buffer[..m].iter_mut().enumerate() {
                            let t = (n * m + k) as f64 / top;
                            *slot = (std::f64::consts::TAU * frequency * t).sin() as f32;
                        }
                        let out = oversampler.downsample(&buffer);
                        if n >= 200 {
                            peak = peak.max(f64::from(out.abs()));
                        }
                    }
                    worst = worst.max(peak);
                }
                frequency += 487.0;
            }
            let db = 20.0 * worst.log10();
            assert!(db < -75.0, "{factor:?} {db} dB");
        }
    }
}
