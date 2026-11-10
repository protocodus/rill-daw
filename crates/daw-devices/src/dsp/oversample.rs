//! 2× oversampling with a linear-phase half-band FIR (Kaiser window). Up- and
//! down-sampling together delay the signal by exactly [`Oversampler2x::LATENCY`]
//! base-rate samples.

const TAPS: usize = 63;
const CENTER: usize = TAPS / 2;

#[derive(Clone, Debug)]
pub struct Oversampler2x {
    /// The nonzero taps besides the center (0.5): every even index.
    even: [f32; CENTER + 1],
    up: [f32; HISTORY],
    down: [f32; HISTORY],
    up_cursor: usize,
    down_cursor: usize,
}

const HISTORY: usize = TAPS + 1;

impl Default for Oversampler2x {
    fn default() -> Self {
        Self::new()
    }
}

impl Oversampler2x {
    pub const LATENCY: u32 = CENTER as u32;

    pub fn new() -> Self {
        let taps = half_band();
        let mut even = [0.0; CENTER + 1];
        for (slot, index) in even.iter_mut().zip((0..TAPS).step_by(2)) {
            *slot = taps[index] as f32;
        }
        Self {
            even,
            up: [0.0; HISTORY],
            down: [0.0; HISTORY],
            up_cursor: 0,
            down_cursor: 0,
        }
    }

    pub fn reset(&mut self) {
        self.up = [0.0; HISTORY];
        self.down = [0.0; HISTORY];
    }

    /// One base-rate sample in, two oversampled samples out, oldest first.
    #[inline]
    pub fn upsample(&mut self, input: f32) -> [f32; 2] {
        // The zero-stuffed stream is never stored: history holds only the real
        // samples (scaled by 2 for unity gain), and each output phase uses the
        // taps that meet them. The second phase sits on an input sample and
        // only the center tap (0.5) reaches it.
        self.up_cursor = (self.up_cursor + 1) % HISTORY;
        self.up[self.up_cursor] = 2.0 * input;
        let at = |delay: usize| self.up[(self.up_cursor + HISTORY - delay) % HISTORY];
        let mut between = 0.0;
        for (delay, &tap) in self.even.iter().enumerate() {
            between += tap * at(delay);
        }
        [between, 0.5 * at(CENTER / 2)]
    }

    /// Two oversampled samples in, one base-rate sample out.
    #[inline]
    pub fn downsample(&mut self, input: [f32; 2]) -> f32 {
        for sample in input {
            self.down_cursor = (self.down_cursor + 1) % HISTORY;
            self.down[self.down_cursor] = sample;
        }
        // Filter output for the older sample of the pair, which lands on an
        // integer base-rate delay.
        let at = |delay: usize| self.down[(self.down_cursor + HISTORY - 1 - delay) % HISTORY];
        let mut sum = 0.5 * at(CENTER);
        for (index, &tap) in self.even.iter().enumerate() {
            sum += tap * at(2 * index);
        }
        sum
    }
}

fn half_band() -> [f64; TAPS] {
    const BETA: f64 = 8.0;
    let mut taps = [0.0; TAPS];
    for (index, tap) in taps.iter_mut().enumerate() {
        let n = index as f64 - CENTER as f64;
        let sinc = if n == 0.0 {
            0.5
        } else {
            (std::f64::consts::PI * n / 2.0).sin() / (std::f64::consts::PI * n)
        };
        let ratio = n / CENTER as f64;
        let window = bessel_i0(BETA * (1.0 - ratio * ratio).max(0.0).sqrt()) / bessel_i0(BETA);
        *tap = sinc * window;
    }
    taps
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
}
