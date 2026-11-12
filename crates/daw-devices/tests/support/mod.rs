//! Measurement helpers shared by the device tests: settled devices, test
//! signals, tone fitting and spectra.
#![allow(dead_code)]
use daw_devices::{Device, DeviceDescriptor, ProcessContext, StereoIo};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex64;

/// Plain values by parameter ID; everything else stays at its default.
pub type Values = Vec<(u32, f32)>;

pub fn normalized(descriptor: &DeviceDescriptor, id: u32, plain: f32) -> f32 {
    descriptor.parameter(id).unwrap().to_normalized(plain)
}

pub fn set(device: &mut dyn Device, values: &[(u32, f32)]) {
    let descriptor = device.descriptor();
    for &(id, plain) in values {
        device.set_parameter(id, normalized(descriptor, id, plain));
    }
}

/// The normalized value of every parameter for `values`, as a device's
/// `Settings::from_normalized` wants it.
pub fn value_of(
    descriptor: &'static DeviceDescriptor,
    values: &[(u32, f32)],
) -> impl Fn(u32) -> f32 {
    let values = values.to_vec();
    move |id| {
        let spec = descriptor.parameter(id).unwrap();
        values
            .iter()
            .rev()
            .find(|(key, _)| *key == id)
            .map_or(spec.default_normalized(), |(_, plain)| {
                spec.to_normalized(*plain)
            })
    }
}

/// A device settled at `values`: nothing glides or fades when audio starts.
pub fn device(descriptor: &DeviceDescriptor, rate: f32, values: &[(u32, f32)]) -> Box<dyn Device> {
    let mut device = (descriptor.create)();
    device.prepare(rate, 512);
    set(&mut *device, values);
    device.reset();
    device
}

/// Processes in place, in blocks that do not line up with any control tick.
pub fn run(device: &mut dyn Device, left: &mut [f32], right: &mut [f32]) {
    run_keyed(device, left, right, None);
}

/// [`run`] with an external sidechain.
pub fn run_keyed(
    device: &mut dyn Device,
    left: &mut [f32],
    right: &mut [f32],
    sidechain: Option<(&[f32], &[f32])>,
) {
    let mut offset = 0;
    for (left, right) in left.chunks_mut(500).zip(right.chunks_mut(500)) {
        let frames = left.len();
        device.process(
            &ProcessContext::default(),
            &[],
            StereoIo {
                left,
                right,
                sidechain: sidechain
                    .map(|(l, r)| (&l[offset..offset + frames], &r[offset..offset + frames])),
            },
        );
        offset += frames;
    }
}

/// The same signal on both channels; returns the left output.
pub fn run_mono(device: &mut dyn Device, input: &[f32]) -> Vec<f32> {
    let mut left = input.to_vec();
    let mut right = input.to_vec();
    run(device, &mut left, &mut right);
    assert_eq!(left, right, "equal channels stay equal");
    left
}

pub fn sine(frequency: f64, amplitude: f64, rate: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| (amplitude * (std::f64::consts::TAU * frequency * n as f64 / rate).sin()) as f32)
        .collect()
}

pub struct Noise(pub u32);

impl Noise {
    /// Uniform in −1–1.
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1 << 23) as f32 - 1.0
    }
}

/// Pink-like noise: 240 tones of equal level and random phase, spaced evenly
/// in pitch from 40 Hz to 16 kHz, at `rms`. Near-Gaussian, deterministic.
pub fn pink(rate: f64, frames: usize, rms: f64) -> Vec<f32> {
    const TONES: usize = 240;
    let mut random = Noise(77);
    let mut signal = vec![0.0_f64; frames];
    for tone in 0..TONES {
        let frequency = 40.0 * 400.0_f64.powf((tone as f64 + 0.5) / TONES as f64);
        let phase = std::f64::consts::PI * f64::from(random.next());
        let step = std::f64::consts::TAU * frequency / rate;
        for (n, sample) in signal.iter_mut().enumerate() {
            *sample += (step * n as f64 + phase).sin();
        }
    }
    let scale = rms / (TONES as f64 / 2.0).sqrt();
    signal.iter().map(|x| (x * scale) as f32).collect()
}

/// Amplitude of the sinusoid at `frequency` in `signal`, by least squares:
/// exact for any length, also a fraction of a period.
pub fn fit(signal: &[f32], frequency: f64, rate: f64) -> f64 {
    let (mut ss, mut sc, mut cc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (n, y) in signal.iter().enumerate() {
        let (s, c) = (std::f64::consts::TAU * frequency * n as f64 / rate).sin_cos();
        ss += s * s;
        sc += s * c;
        cc += c * c;
        ys += f64::from(*y) * s;
        yc += f64::from(*y) * c;
    }
    let determinant = ss * cc - sc * sc;
    let a = (ys * cc - yc * sc) / determinant;
    let b = (yc * ss - ys * sc) / determinant;
    a.hypot(b)
}

pub fn db(ratio: f64) -> f32 {
    (20.0 * ratio.max(1e-30).log10()) as f32
}

pub fn rms(signal: &[f32]) -> f64 {
    (signal.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / signal.len() as f64).sqrt()
}

pub fn peak(signal: &[f32]) -> f32 {
    signal.iter().fold(0.0, |peak, x| peak.max(x.abs()))
}

/// Largest change from one sample to the next.
pub fn largest_step(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max)
}

pub const FFT: usize = 1 << 16;
/// Bins a tone occupies on either side of its own under [`spectrum`]'s
/// window.
pub const SKIRT: usize = 16;

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..200 {
        term *= (x / (2.0 * f64::from(k))).powi(2);
        sum += term;
        if term < 1e-17 * sum {
            break;
        }
    }
    sum
}

/// Magnitude spectrum of the last [`FFT`] frames under a Kaiser window whose
/// side lobes are below −250 dB.
pub fn spectrum(signal: &[f32]) -> Vec<f64> {
    let signal = &signal[signal.len() - FFT..];
    let mut buffer: Vec<Complex64> = signal
        .iter()
        .enumerate()
        .map(|(n, x)| {
            let ratio = 2.0 * n as f64 / (FFT - 1) as f64 - 1.0;
            let window = bessel_i0(38.0 * (1.0 - ratio * ratio).sqrt()) / bessel_i0(38.0);
            Complex64::new(f64::from(*x) * window, 0.0)
        })
        .collect();
    FftPlanner::new().plan_fft_forward(FFT).process(&mut buffer);
    buffer[..FFT / 2].iter().map(|bin| bin.norm()).collect()
}

/// The frequency of an FFT bin near `frequency`, and the bin.
pub fn on_bin(frequency: f64, rate: f64) -> (f64, usize) {
    let bin = (frequency * FFT as f64 / rate).round();
    (bin * rate / FFT as f64, bin as usize)
}

/// Samples until `signal` has covered `share` of the way from its first to
/// its last value.
pub fn time_to(signal: &[f32], share: f32) -> usize {
    let (from, to) = (signal[0], signal[signal.len() - 1]);
    let goal = from + share * (to - from);
    signal
        .iter()
        .position(|x| if to > from { *x >= goal } else { *x <= goal })
        .unwrap()
}

/// Level in dB of the largest component between the bins `low` and `high`
/// that is not a harmonic of the tone at `bin`, against the tone, in a
/// [`spectrum`].
pub fn worst_inharmonic_db(spectrum: &[f64], bin: usize, low: usize, high: usize) -> f32 {
    let near_harmonic = |candidate: usize| {
        let nearest = ((candidate as f64 / bin as f64).round() as usize).max(1) * bin;
        candidate.abs_diff(nearest) <= SKIRT
    };
    let worst = (low..=high.min(spectrum.len() - 1))
        .filter(|candidate| !near_harmonic(*candidate))
        .map(|candidate| spectrum[candidate])
        .fold(0.0, f64::max);
    db(worst / spectrum[bin])
}

/// Magnitude of the transfer function at `frequency` from an impulse
/// response that has died away within `response`, for an impulse of
/// `amplitude`.
pub fn response_at(response: &[f32], amplitude: f64, frequency: f64, rate: f64) -> f64 {
    let step = std::f64::consts::TAU * frequency / rate;
    let (mut re, mut im) = (0.0, 0.0);
    for (n, y) in response.iter().enumerate() {
        let (sin, cos) = (step * n as f64).sin_cos();
        re += f64::from(*y) * cos;
        im -= f64::from(*y) * sin;
    }
    re.hypot(im) / amplitude
}

/// Running mean over `window` values; `window` shorter than the input.
fn running_mean(values: &[f64], window: usize) -> Vec<f64> {
    let mut sum: f64 = values[..window].iter().sum();
    let mut means = vec![sum / window as f64];
    for n in window..values.len() {
        sum += values[n] - values[n - window];
        means.push(sum / window as f64);
    }
    means
}

/// Frequency of a signal near `carrier` over time, in hertz, one value per
/// sample: the tone is shifted to zero, its image at twice the carrier is
/// averaged away (three running means over `window` samples, a whole number
/// of carrier periods) and the phase differentiated.
pub fn instantaneous_hz(signal: &[f32], carrier: f64, rate: f64, window: usize) -> Vec<f64> {
    let step = std::f64::consts::TAU * carrier / rate;
    let shift = |turn: fn(f64) -> f64| {
        let mut part: Vec<f64> = signal
            .iter()
            .enumerate()
            .map(|(n, y)| f64::from(*y) * turn(step * n as f64))
            .collect();
        for _ in 0..3 {
            part = running_mean(&part, window);
        }
        part
    };
    let (re, im) = (shift(f64::cos), shift(|angle| -angle.sin()));
    let phases: Vec<f64> = re.iter().zip(&im).map(|(re, im)| im.atan2(*re)).collect();
    phases
        .windows(2)
        .map(|pair| {
            let mut turn = pair[1] - pair[0];
            if turn > std::f64::consts::PI {
                turn -= std::f64::consts::TAU;
            } else if turn < -std::f64::consts::PI {
                turn += std::f64::consts::TAU;
            }
            carrier + turn * rate / std::f64::consts::TAU
        })
        .collect()
}
