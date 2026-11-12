use daw_devices::devices::equalizer::{self, *};
use daw_devices::{Device, Preset, ProcessContext, StereoIo, descriptor};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex64;

/// Plain values by parameter ID; everything else stays at its default.
type Values = Vec<(u32, f32)>;

const BELL: f32 = 0.0;
const LOW_SHELF: f32 = 1.0;
const HIGH_SHELF: f32 = 2.0;
const NOTCH: f32 = 3.0;
const DB12: f32 = 0.0;
const DB24: f32 = 1.0;
const DB48: f32 = 2.0;
const ANALOG: (u32, f32) = (MODE, 1.0);

/// One parametric band (0–3), switched on.
fn band(index: usize, kind: f32, frequency: f32, gain: f32, q: f32) -> Values {
    let base = BANDS[index];
    vec![
        (base + BAND_ON, 1.0),
        (base + BAND_TYPE, kind),
        (base + BAND_FREQUENCY, frequency),
        (base + BAND_GAIN, gain),
        (base + BAND_Q, q),
    ]
}

fn high_pass(frequency: f32, slope: f32) -> Values {
    vec![(HP_ON, 1.0), (HP_FREQUENCY, frequency), (HP_SLOPE, slope)]
}

fn low_pass(frequency: f32, slope: f32) -> Values {
    vec![(LP_ON, 1.0), (LP_FREQUENCY, frequency), (LP_SLOPE, slope)]
}

fn normalized(id: u32, plain: f32) -> f32 {
    equalizer::DESCRIPTOR
        .parameter(id)
        .unwrap()
        .to_normalized(plain)
}

fn set(device: &mut dyn Device, values: &[(u32, f32)]) {
    for &(id, plain) in values {
        device.set_parameter(id, normalized(id, plain));
    }
}

fn settings(values: &[(u32, f32)]) -> Settings {
    Settings::from_normalized(|id| {
        let plain = values.iter().rev().find(|(key, _)| *key == id);
        plain.map_or(
            equalizer::DESCRIPTOR
                .parameter(id)
                .unwrap()
                .default_normalized(),
            |(_, plain)| normalized(id, *plain),
        )
    })
}

/// A device settled at `values`: nothing glides or fades when audio starts.
fn device(rate: f32, values: &[(u32, f32)]) -> Box<dyn Device> {
    let mut device = (descriptor("r1-equalizer").unwrap().create)();
    device.prepare(rate, 512);
    set(&mut *device, values);
    device.reset();
    device
}

/// Processes in place, in blocks that do not line up with the control tick.
fn run(device: &mut dyn Device, left: &mut [f32], right: &mut [f32]) {
    for (left, right) in left.chunks_mut(500).zip(right.chunks_mut(500)) {
        device.process(
            &ProcessContext::default(),
            &[],
            StereoIo {
                left,
                right,
                sidechain: None,
            },
        );
    }
}

/// The same signal on both channels; returns the left output.
fn run_mono(device: &mut dyn Device, input: &[f32]) -> Vec<f32> {
    let mut left = input.to_vec();
    let mut right = input.to_vec();
    run(device, &mut left, &mut right);
    assert_eq!(left, right, "channels are linked");
    left
}

fn sine(frequency: f64, amplitude: f64, rate: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| (amplitude * (std::f64::consts::TAU * frequency * n as f64 / rate).sin()) as f32)
        .collect()
}

struct Noise(u32);

impl Noise {
    /// Uniform in −1–1.
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1 << 23) as f32 - 1.0
    }

    fn block(&mut self, amplitude: f32, frames: usize) -> Vec<f32> {
        (0..frames).map(|_| amplitude * self.next()).collect()
    }
}

/// Amplitude of the sinusoid at `frequency` in `signal`, by least squares:
/// exact for any length, also a fraction of a period.
fn fit(signal: &[f32], frequency: f64, rate: f64) -> f64 {
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

fn db(ratio: f64) -> f32 {
    (20.0 * ratio.log10()) as f32
}

fn rms(signal: &[f32]) -> f64 {
    (signal.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / signal.len() as f64).sqrt()
}

fn peak(signal: &[f32]) -> f32 {
    signal.iter().fold(0.0, |peak, x| peak.max(x.abs()))
}

fn log_spaced(low: f32, high: f32, count: usize) -> Vec<f32> {
    (0..count)
        .map(|i| low * (high / low).powf(i as f32 / (count - 1) as f32))
        .collect()
}

/// The device's gain in dB at each frequency for a settled sine of
/// `amplitude`, after `settle` seconds. Two frequencies per run: the
/// channels share coefficients but not signal.
fn measured_db(
    values: &[(u32, f32)],
    rate: f32,
    frequencies: &[f32],
    amplitude: f64,
    settle: f64,
) -> Vec<f32> {
    let rate64 = f64::from(rate);
    let mut gains = Vec::new();
    for pair in frequencies.chunks(2) {
        let (first, second) = (f64::from(pair[0]), f64::from(pair[pair.len() - 1]));
        // At least three periods of the lower tone.
        let measure = (0.1_f64).max(3.0 / first.min(second));
        let (skip, frames) = (
            (settle * rate64) as usize,
            ((settle + measure) * rate64) as usize,
        );
        let inputs = [
            sine(first, amplitude, rate64, frames),
            sine(second, amplitude, rate64, frames),
        ];
        let mut device = device(rate, values);
        let (mut left, mut right) = (inputs[0].clone(), inputs[1].clone());
        run(&mut *device, &mut left, &mut right);
        for (output, (input, frequency)) in [left, right]
            .iter()
            .zip(inputs.iter().zip([first, second]))
            .take(pair.len())
        {
            assert!(output.iter().all(|x| x.is_finite()));
            let gain =
                fit(&output[skip..], frequency, rate64) / fit(&input[skip..], frequency, rate64);
            gains.push(db(gain));
        }
    }
    gains
}

/// Seconds until a filter around `frequency` has settled to a thousandth of
/// a dB, for the most resonant setting these tests use.
fn settle_seconds(frequency: f32) -> f64 {
    (40.0 / f64::from(frequency)).clamp(0.03, 2.5)
}

/// Asserts that what the device does to sines is what `response_db` draws.
fn assert_audio_equals_drawn(values: &[(u32, f32)], rate: f32, lowest: f32, amplitude: f64) {
    let top = 20_000.0_f32.min(0.45 * rate);
    let frequencies = log_spaced(20.0, top, 31);
    let mut drawn = vec![0.0; frequencies.len()];
    response_db(&settings(values), rate, &frequencies, &mut drawn);
    let measured = measured_db(
        values,
        rate,
        &frequencies,
        amplitude,
        settle_seconds(lowest),
    );
    for ((frequency, drawn), measured) in frequencies.iter().zip(drawn).zip(measured) {
        // Below −60 dB the measurement is rounding, not the filter.
        if drawn < -60.0 {
            continue;
        }
        assert!(
            (measured - drawn).abs() <= 0.1,
            "{values:?} at {frequency} Hz, {rate} Hz: measured {measured}, drawn {drawn}"
        );
    }
}

const BAND_FREQUENCIES: [f32; 4] = [100.0, 1000.0, 12_000.0, 18_000.0];

fn band_cases(kinds: &[f32], frequencies: &[f32]) -> Vec<(f32, Values)> {
    let mut cases = Vec::new();
    for &kind in kinds {
        for &frequency in frequencies {
            for gain in [6.0, -6.0, 18.0, -18.0] {
                // A notch has no gain.
                if kind == NOTCH && gain != 6.0 {
                    continue;
                }
                for q in [0.5, 4.0] {
                    cases.push((frequency, band(1, kind, frequency, gain, q)));
                }
            }
        }
    }
    cases
}

#[test]
fn bells_sound_as_drawn() {
    for rate in [44_100.0, 48_000.0] {
        for (frequency, values) in band_cases(&[BELL], &BAND_FREQUENCIES) {
            assert_audio_equals_drawn(&values, rate, frequency, 0.05);
        }
    }
}

#[test]
fn shelves_sound_as_drawn() {
    for rate in [44_100.0, 48_000.0] {
        for (frequency, values) in band_cases(&[LOW_SHELF, HIGH_SHELF], &BAND_FREQUENCIES) {
            assert_audio_equals_drawn(&values, rate, frequency, 0.05);
        }
    }
}

#[test]
fn notches_cuts_and_all_bands_together_sound_as_drawn() {
    let mut cases = band_cases(&[NOTCH], &BAND_FREQUENCIES);
    for slope in [DB12, DB24, DB48] {
        cases.push((100.0, high_pass(100.0, slope)));
        cases.push((1000.0, high_pass(1000.0, slope)));
        cases.push((2000.0, low_pass(2000.0, slope)));
        cases.push((16_000.0, low_pass(16_000.0, slope)));
    }
    cases.push((
        60.0,
        [
            high_pass(60.0, DB24),
            band(0, LOW_SHELF, 120.0, 6.0, 0.71),
            band(1, BELL, 500.0, -9.0, 2.0),
            band(2, BELL, 3000.0, 12.0, 3.0),
            band(3, HIGH_SHELF, 9000.0, -6.0, 0.71),
            low_pass(15_000.0, DB48),
        ]
        .concat(),
    ));
    for rate in [44_100.0, 48_000.0] {
        for (frequency, values) in &cases {
            assert_audio_equals_drawn(values, rate, *frequency, 0.05);
        }
    }
}

#[test]
fn analog_mode_sounds_as_drawn_at_low_level() {
    for rate in [44_100.0, 48_000.0] {
        for (frequency, mut values) in
            band_cases(&[BELL, LOW_SHELF, HIGH_SHELF], &[100.0, 12_000.0])
        {
            values.push(ANALOG);
            // −40 dBFS: the output stage adds nothing measurable.
            assert_audio_equals_drawn(&values, rate, frequency, 0.01);
        }
    }
}

#[test]
fn other_sample_rates_sound_as_drawn() {
    let values = [
        band(0, BELL, 20.0, 12.0, 2.0),
        band(3, BELL, 20_000.0, 12.0, 2.0),
        band(1, HIGH_SHELF, 20_000.0, -9.0, 0.71),
        high_pass(20.0, DB24),
    ]
    .concat();
    for rate in [32_000.0, 88_200.0, 96_000.0, 192_000.0] {
        assert_audio_equals_drawn(&values, rate, 20.0, 0.05);
    }
}

/// The analog response a band is named after (RBJ's prototypes), written
/// out here independently of the device's designer.
fn analog_db(kind: f32, frequency: f64, gain_db: f64, q: f64, at: f64) -> f64 {
    let a = 10.0_f64.powf(gain_db / 40.0);
    let x = at / frequency;
    let magnitude = |[p0, p1, p2]: [f64; 3]| (p2 - p0 * x * x).hypot(p1 * x);
    let root = a.sqrt();
    let (numerator, denominator) = if kind == BELL {
        ([1.0, a / q, 1.0], [1.0, 1.0 / (a * q), 1.0])
    } else if kind == LOW_SHELF {
        ([a, a * root / q, a * a], [a, root / q, 1.0])
    } else {
        ([a * a, a * root / q, a], [1.0, root / q, a])
    };
    20.0 * (magnitude(numerator) / magnitude(denominator)).log10()
}

#[test]
fn digital_bands_match_their_analog_curves_to_20_khz() {
    for (rate, tolerance) in [(44_100.0, 0.5), (48_000.0, 0.35), (96_000.0, 0.03)] {
        let frequencies = log_spaced(20.0, 20_000.0, 16);
        for (kind, qs) in [
            (BELL, [0.5, 4.0]),
            (LOW_SHELF, [0.5, 2.0]),
            (HIGH_SHELF, [0.5, 2.0]),
        ] {
            for centre in [20.0, 1000.0, 8000.0, 16_000.0, 20_000.0] {
                for gain in [-12.0, 6.0, 12.0] {
                    for q in qs {
                        let values = band(1, kind, centre, gain, q);
                        let measured =
                            measured_db(&values, rate, &frequencies, 0.05, settle_seconds(centre));
                        for (frequency, measured) in frequencies.iter().zip(measured) {
                            let analog = analog_db(
                                kind,
                                f64::from(centre),
                                f64::from(gain),
                                f64::from(q),
                                f64::from(*frequency),
                            ) as f32;
                            let error = (measured - analog).abs();
                            assert!(
                                error <= tolerance,
                                "{values:?} at {frequency} Hz, {rate} Hz: measured {measured}, analog {analog}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn flat_settings_are_transparent() {
    let input = Noise(7).block(0.5, 20_000);
    // Digital: bit for bit.
    let mut digital = device(48_000.0, &[]);
    assert_eq!(run_mono(&mut *digital, &input), input);
    // Bands that are on but flat, cuts that are off, any frequencies.
    let flat = [
        band(0, BELL, 60.0, 0.0, 18.0),
        band(1, LOW_SHELF, 5000.0, 0.0, 0.1),
        vec![
            (HP_FREQUENCY, 500.0),
            (LP_FREQUENCY, 900.0),
            (AUTO_GAIN, 1.0),
        ],
    ]
    .concat();
    let mut digital = device(48_000.0, &flat);
    assert_eq!(run_mono(&mut *digital, &input), input);

    // Analog at −60 dBFS, once the switch to Analog has faded in.
    let quiet = Noise(9).block(0.001, 20_000);
    let mut analog = device(48_000.0, &[]);
    set(&mut *analog, &[ANALOG]);
    let output = run_mono(&mut *analog, &quiet);
    let error = quiet[2000..]
        .iter()
        .zip(&output[2000..])
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max);
    assert!(error <= 1e-6, "{error}");
    // And it is in fact on: a loud signal is coloured.
    let loud = sine(1000.0, 1.0, 48_000.0, 4000);
    let coloured = run_mono(&mut *analog, &loud);
    assert!(
        loud.iter()
            .zip(&coloured)
            .any(|(x, y)| (x - y).abs() > 1e-3)
    );
}

#[test]
fn a_cut_undoes_the_same_boost() {
    let cases = [
        band(1, BELL, 1000.0, 12.0, 2.0),
        band(1, BELL, 60.0, 12.0, 4.0),
        band(1, LOW_SHELF, 100.0, 12.0, 0.71),
        band(1, HIGH_SHELF, 8000.0, 12.0, 0.71),
        [
            band(0, LOW_SHELF, 150.0, -12.0, 1.0),
            band(1, BELL, 400.0, 12.0, 3.0),
            band(2, BELL, 2500.0, -12.0, 0.7),
            band(3, HIGH_SHELF, 6000.0, 12.0, 0.71),
        ]
        .concat(),
    ];
    for rate in [44_100.0, 48_000.0] {
        for boost in &cases {
            let cut: Values = boost
                .iter()
                .map(|&(id, value)| {
                    let gain = BANDS.iter().any(|base| id == base + BAND_GAIN);
                    (id, if gain { -value } else { value })
                })
                .collect();
            let input = Noise(3).block(0.1, 48_000);
            let between = run_mono(&mut *device(rate, boost), &input);
            let output = run_mono(&mut *device(rate, &cut), &between);
            let difference: Vec<f32> = input.iter().zip(&output).map(|(x, y)| x - y).collect();
            let null = db(rms(&difference) / rms(&input));
            assert!(null <= -120.0, "{boost:?} at {rate}: {null} dB");
        }
    }
}

fn drawn(values: &[(u32, f32)], frequencies: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; frequencies.len()];
    response_db(&settings(values), 48_000.0, frequencies, &mut out);
    out
}

/// Octaves between the two points where a bell has half its peak gain in dB.
fn width_octaves(values: &[(u32, f32)], gain: f32) -> f32 {
    let frequencies = log_spaced(20.0, 20_000.0, 4001);
    let curve = drawn(values, &frequencies);
    let inside: Vec<f32> = frequencies
        .iter()
        .zip(&curve)
        .filter(|(_, level)| **level >= 0.5 * gain)
        .map(|(frequency, _)| *frequency)
        .collect();
    (inside[inside.len() - 1] / inside[0]).log2()
}

#[test]
fn analog_bells_narrow_as_gain_rises() {
    let bell = |gain: f32, analog: bool| {
        let mut values = band(1, BELL, 1000.0, gain, 1.0);
        if analog {
            values.push(ANALOG);
        }
        values
    };
    let widths = [3.0, 12.0, 24.0].map(|gain| width_octaves(&bell(gain, true), gain));
    assert!(widths[0] > widths[1] && widths[1] > widths[2], "{widths:?}");
    let ratio = widths[1] / widths[2];
    assert!((ratio - 2.0).abs() <= 0.2, "{widths:?}: {ratio}");
    // Digital keeps its width in octaves much better.
    let digital = [12.0, 24.0].map(|gain| width_octaves(&bell(gain, false), gain));
    assert!(digital[0] / digital[1] < 1.3, "{digital:?}");

    // At ±12 dB the two modes are the same bell: in the drawing and in sound.
    let frequencies = log_spaced(20.0, 20_000.0, 400);
    for gain in [12.0, -12.0] {
        let (analog, digital) = (
            drawn(&bell(gain, true), &frequencies),
            drawn(&bell(gain, false), &frequencies),
        );
        for (analog, digital) in analog.iter().zip(&digital) {
            assert!((analog - digital).abs() <= 0.05, "{gain} dB");
        }
        let tones = [500.0, 1000.0, 1600.0, 4000.0];
        let analog = measured_db(&bell(gain, true), 48_000.0, &tones, 0.01, 0.05);
        let digital = measured_db(&bell(gain, false), 48_000.0, &tones, 0.01, 0.05);
        for (analog, digital) in analog.iter().zip(&digital) {
            assert!(
                (analog - digital).abs() <= 0.05,
                "{gain} dB: {analog} {digital}"
            );
        }
    }
}

#[test]
fn analog_shelves_dip_before_they_rise() {
    let frequencies = log_spaced(20.0, 20_000.0, 2000);
    let shelf = band(1, LOW_SHELF, 300.0, 12.0, 0.71);
    let lowest =
        |values: &[(u32, f32)]| drawn(values, &frequencies).into_iter().fold(0.0, f32::min);
    let analog = lowest(&[shelf.clone(), vec![ANALOG]].concat());
    assert!((analog + 1.3).abs() <= 0.2, "{analog} dB");
    let digital = lowest(&shelf);
    assert!(digital >= -0.05, "{digital} dB");
    // The dip is in the sound, on the far side of the transition.
    let mut values = shelf.clone();
    values.push(ANALOG);
    let (index, _) = drawn(&values, &frequencies)
        .into_iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    let dip = frequencies[index];
    assert!(dip > 300.0, "{dip} Hz");
    let measured = measured_db(&values, 48_000.0, &[dip], 0.01, 0.2)[0];
    assert!(
        (measured - analog).abs() <= 0.05,
        "{measured} dB at {dip} Hz"
    );
    // A high shelf mirrors it, and the plateau is the set gain.
    let high = [band(1, HIGH_SHELF, 3000.0, 12.0, 0.71), vec![ANALOG]].concat();
    assert!((lowest(&high) + 1.3).abs() <= 0.2);
    assert!((drawn(&values, &[20.0])[0] - 12.0).abs() <= 0.2);
}

const FFT: usize = 1 << 16;

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

/// Magnitude spectrum of the last [`FFT`] frames under a Kaiser window
/// whose side lobes are below −250 dB.
fn spectrum(signal: &[f32]) -> Vec<f64> {
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
fn on_bin(frequency: f64, rate: f64) -> (f64, usize) {
    let bin = (frequency * FFT as f64 / rate).round();
    (bin * rate / FFT as f64, bin as usize)
}

/// Spectra of two tones through the Analog stage of a flat equalizer.
fn analog_spectra(tones: [f64; 2], level_db: f64) -> [Vec<f64>; 2] {
    let amplitude = 10.0_f64.powf(level_db / 20.0);
    let mut device = device(48_000.0, &[ANALOG]);
    let [mut left, mut right] = tones.map(|tone| {
        (0..2 * FFT)
            .map(|n| {
                let phase = std::f64::consts::TAU * tone * n as f64 / 48_000.0 + 0.3;
                (amplitude * phase.sin()) as f32
            })
            .collect::<Vec<f32>>()
    });
    run(&mut *device, &mut left, &mut right);
    [spectrum(&left), spectrum(&right)]
}

#[test]
fn analog_stage_does_not_alias() {
    let tones = [
        1003.0, 2503.0, 4999.0, 7001.0, 10_007.0, 13_001.0, 16_001.0, 18_503.0,
    ];
    for (level, limit) in [
        (0.0, -80.0),
        (6.0, -67.0),
        (9.0, -67.0),
        (12.0, -67.0),
        (18.0, -67.0),
    ] {
        for pair in tones.chunks(2) {
            let bins = [on_bin(pair[0], 48_000.0), on_bin(pair[1], 48_000.0)];
            let spectra = analog_spectra([bins[0].0, bins[1].0], level);
            for (spectrum, (tone, bin)) in spectra.iter().zip(bins) {
                // Everything that is not the tone or one of its harmonics
                // below Nyquist: folded harmonics.
                let harmonics: Vec<usize> = (1..60)
                    .map(|harmonic| harmonic * bin)
                    .filter(|bin| *bin < FFT / 2)
                    .collect();
                let worst = spectrum
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| harmonics.iter().all(|bin| index.abs_diff(*bin) > 40))
                    .map(|(_, magnitude)| *magnitude)
                    .fold(0.0, f64::max);
                let dbc = db(worst / spectrum[bin]);
                assert!(dbc <= limit, "{tone} Hz at {level} dBFS: {dbc} dBc");
            }
        }
    }
}

#[test]
fn analog_stage_adds_a_subtle_third_harmonic() {
    let (tone, bin) = on_bin(1000.0, 48_000.0);
    let third = |level: f64| {
        let [spectrum, _] = analog_spectra([tone, tone], level);
        db(spectrum[3 * bin] / spectrum[bin])
    };
    let quiet = third(-12.0);
    assert!(quiet <= -69.0 && quiet > -80.0, "{quiet} dBc at -12 dBFS");
    let nominal = third(0.0);
    assert!((nominal + 51.0).abs() <= 1.0, "{nominal} dBc at 0 dBFS");
    let hot = third(12.0);
    assert!((hot + 54.0).abs() <= 1.5, "{hot} dBc at +12 dBFS");
    // Digital adds nothing.
    let clean = sine(tone, 1.0, 48_000.0, 2 * FFT);
    let spectrum = spectrum(&run_mono(&mut *device(48_000.0, &[]), &clean));
    assert!(db(spectrum[3 * bin] / spectrum[bin]) < -140.0);
}

#[test]
fn analog_stage_never_combs_the_response() {
    let frequencies = [
        100.0, 300.0, 1000.0, 3000.0, 6000.0, 9000.0, 12_000.0, 4700.0,
    ];
    for level in [-12.0, 0.0, 6.0, 12.0, 18.0, 24.0] {
        let amplitude = 10.0_f64.powf(level / 20.0);
        let gains = measured_db(&[ANALOG], 48_000.0, &frequencies, amplitude, 0.05);
        for (frequency, gain) in frequencies.iter().zip(gains) {
            assert!(
                gain.abs() <= 0.2,
                "{frequency} Hz at {level} dBFS: {gain} dB"
            );
        }
    }
    // Driven by a boost: +18 dBFS inside from a full-scale tone.
    let values = [band(1, BELL, 1000.0, 18.0, 1.0), vec![ANALOG]].concat();
    let curve = drawn(&values, &frequencies);
    let gains = measured_db(&values, 48_000.0, &frequencies, 1.0, 0.1);
    for ((frequency, drawn), gain) in frequencies.iter().zip(curve).zip(gains) {
        assert!(
            (gain - drawn).abs() <= 0.2,
            "{frequency} Hz: measured {gain}, drawn {drawn}"
        );
    }
}

#[test]
fn output_guard_bounds_absurd_settings_and_nothing_else() {
    let mut values: Values = (0..4)
        .flat_map(|index| band(index, LOW_SHELF, 20_000.0, 24.0, 0.71))
        .collect();
    values.push((OUTPUT, 24.0));
    let hot = Noise(11).block(4.0, 48_000);
    for mode in [0.0, 1.0] {
        let mut device = device(48_000.0, &[values.clone(), vec![(MODE, mode)]].concat());
        let output = run_mono(&mut *device, &hot);
        assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 64.0));
        // It was needed: the settings ask for +120 dB.
        assert_eq!(peak(&output), 64.0);
    }
    // Nor does a finite input at the edge of the number format get through
    // the single-precision Analog stage as anything else.
    let mut analog = device(
        48_000.0,
        &[band(1, BELL, 1000.0, 24.0, 1.0), vec![ANALOG]].concat(),
    );
    let huge = Noise(3).block(3e38, 4800);
    let output = run_mono(&mut *analog, &huge);
    assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 64.0));
    // At ordinary settings even an overloaded input passes untouched.
    assert_eq!(run_mono(&mut *device(48_000.0, &[]), &hot), hot);
    // One bad sample does not stay in the filters.
    let mut device = device(48_000.0, &band(1, BELL, 100.0, 12.0, 8.0));
    let mut poisoned = Noise(5).block(0.1, 4000);
    poisoned[10] = f32::NAN;
    poisoned[11] = f32::INFINITY;
    assert!(
        run_mono(&mut *device, &poisoned)
            .iter()
            .all(|x| x.is_finite())
    );
}

/// Largest absolute sample and largest step between neighbouring samples.
fn peak_and_step(signal: &[f32]) -> (f32, f32) {
    assert!(signal.iter().all(|x| x.is_finite()));
    let step = signal
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max);
    (peak(signal), step)
}

const MODULATION_RATE: f32 = 48_000.0;
/// Frequency of the test sine: the ninth of sixteen log steps from 20 Hz to
/// 20 kHz, so a band swept over the range has a static setting right on it.
const MODULATION_TONE: f64 = 796.214_3;

fn modulation_inputs(seconds: f32) -> [Vec<f32>; 2] {
    let frames = (seconds * MODULATION_RATE) as usize;
    [
        Noise(21).block(0.25, frames),
        sine(MODULATION_TONE, 0.25, f64::from(MODULATION_RATE), frames),
    ]
}

/// Steady state at fixed settings: peak and largest step, after the start.
fn steady(values: &[(u32, f32)], input: &[f32]) -> (f32, f32) {
    let output = run_mono(&mut *device(MODULATION_RATE, values), input);
    peak_and_step(&output[output.len() / 10..])
}

/// Output while the settings jump between `from` and `to`: first two slow
/// jumps, then back and forth 20 times a second.
fn modulated(
    base: &[(u32, f32)],
    from: &[(u32, f32)],
    to: &[(u32, f32)],
    input: &[f32],
) -> (f32, f32) {
    let mut device = device(MODULATION_RATE, &[base, from].concat());
    let block = (MODULATION_RATE * 0.025) as usize;
    let mut output = Vec::with_capacity(input.len());
    for (index, chunk) in input.chunks(block).enumerate() {
        let seconds = index as f32 * 0.025;
        // Settled, a jump, a jump back, then a jump every block.
        let at_target = if seconds < 0.5 {
            false
        } else if seconds < 1.0 {
            true
        } else if seconds < 1.5 {
            false
        } else {
            index % 2 == 0
        };
        set(&mut *device, if at_target { to } else { from });
        output.extend(run_mono(&mut *device, chunk));
    }
    // Like `steady`, without the filter's answer to the input starting.
    peak_and_step(&output[output.len() / 10..])
}

/// Sixteen settings along the way from `from` to `to`, evenly spaced on
/// each parameter's own scale.
fn along(from: &[(u32, f32)], to: &[(u32, f32)]) -> Vec<Values> {
    (0..16)
        .map(|step| {
            from.iter()
                .zip(to)
                .map(|(&(id, from), &(_, to))| {
                    let spec = equalizer::DESCRIPTOR.parameter(id).unwrap();
                    let (from, to) = (spec.to_normalized(from), spec.to_normalized(to));
                    (id, spec.to_plain(from + (to - from) * step as f32 / 15.0))
                })
                .collect()
        })
        .collect()
}

#[test]
fn sweeping_a_band_stays_bounded() {
    let b = BANDS[1];
    let resonant = band(1, BELL, MODULATION_TONE as f32, 24.0, 18.0);
    let shelf = band(1, LOW_SHELF, 1000.0, 24.0, 2.0);
    let cuts = [high_pass(500.0, DB48), low_pass(2000.0, DB48)].concat();
    let notch = band(1, NOTCH, 300.0, 0.0, 18.0);
    let paths: [(&str, &Values, Values, Values); 10] = [
        (
            "bell frequency",
            &resonant,
            vec![(b + BAND_FREQUENCY, 20.0)],
            vec![(b + BAND_FREQUENCY, 20_000.0)],
        ),
        (
            "bell gain",
            &resonant,
            vec![(b + BAND_GAIN, -24.0)],
            vec![(b + BAND_GAIN, 24.0)],
        ),
        (
            "bell Q",
            &resonant,
            vec![(b + BAND_Q, 0.1)],
            vec![(b + BAND_Q, 18.0)],
        ),
        (
            "shelf frequency",
            &shelf,
            vec![(b + BAND_FREQUENCY, 20.0)],
            vec![(b + BAND_FREQUENCY, 20_000.0)],
        ),
        (
            "shelf Q",
            &shelf,
            vec![(b + BAND_Q, 0.1)],
            vec![(b + BAND_Q, 2.0)],
        ),
        (
            "notch frequency",
            &notch,
            vec![(b + BAND_FREQUENCY, 20.0)],
            vec![(b + BAND_FREQUENCY, 20_000.0)],
        ),
        (
            "notch Q",
            &notch,
            vec![(b + BAND_Q, 0.1)],
            vec![(b + BAND_Q, 18.0)],
        ),
        (
            "shelf gain",
            &shelf,
            vec![(b + BAND_GAIN, 24.0)],
            vec![(b + BAND_GAIN, -24.0)],
        ),
        (
            "high-pass frequency",
            &cuts,
            vec![(HP_FREQUENCY, 20.0)],
            vec![(HP_FREQUENCY, 2000.0)],
        ),
        (
            "low-pass frequency",
            &cuts,
            vec![(LP_FREQUENCY, 200.0)],
            vec![(LP_FREQUENCY, 20_000.0)],
        ),
    ];
    let inputs = modulation_inputs(2.5);
    let mut failures = Vec::new();
    for (name, base, from, to) in paths {
        for (input, source) in inputs.iter().zip(["noise", "sine"]) {
            // The two ends alone are no bound: a +24 dB bell passing through
            // the middle of the band lets through far more noise than at
            // 20 Hz or 20 kHz.
            let bound = along(&from, &to)
                .iter()
                .map(|step| steady(&[base.clone(), step.clone()].concat(), input).0)
                .fold(0.0, f32::max);
            for (from, to) in [(&from, &to), (&to, &from)] {
                let (peak, _) = modulated(base, from, to, input);
                if peak > 2.0 * bound {
                    failures.push(format!(
                        "{name} {from:?} to {to:?} on {source}: peak {peak}, steady {bound}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn switching_is_click_free() {
    let b = BANDS[1];
    let tone = MODULATION_TONE as f32;
    let resonant = band(1, BELL, tone, 24.0, 18.0);
    let shelf = band(1, LOW_SHELF, tone, 24.0, 2.0);
    let steps: [(&str, Values, Values, Values); 11] = [
        (
            "bell to notch",
            resonant.clone(),
            vec![(b + BAND_TYPE, BELL)],
            vec![(b + BAND_TYPE, NOTCH)],
        ),
        (
            "bell to low shelf",
            resonant.clone(),
            vec![(b + BAND_TYPE, BELL)],
            vec![(b + BAND_TYPE, LOW_SHELF)],
        ),
        (
            "low to high shelf",
            shelf.clone(),
            vec![(b + BAND_TYPE, LOW_SHELF)],
            vec![(b + BAND_TYPE, HIGH_SHELF)],
        ),
        (
            "shelf to notch",
            shelf.clone(),
            vec![(b + BAND_TYPE, HIGH_SHELF)],
            vec![(b + BAND_TYPE, NOTCH)],
        ),
        (
            "band on",
            resonant.clone(),
            vec![(b + BAND_ON, 0.0)],
            vec![(b + BAND_ON, 1.0)],
        ),
        (
            "mode",
            resonant.clone(),
            vec![(MODE, 0.0)],
            vec![(MODE, 1.0)],
        ),
        (
            "mode with a shelf",
            shelf.clone(),
            vec![(MODE, 0.0)],
            vec![(MODE, 1.0)],
        ),
        (
            "high-pass slope",
            high_pass(tone, DB12),
            vec![(HP_SLOPE, DB12)],
            vec![(HP_SLOPE, DB48)],
        ),
        (
            "low-pass slope",
            low_pass(tone, DB24),
            vec![(LP_SLOPE, DB24)],
            vec![(LP_SLOPE, DB48)],
        ),
        (
            "high-pass on",
            high_pass(2000.0, DB48),
            vec![(HP_ON, 0.0)],
            vec![(HP_ON, 1.0)],
        ),
        (
            "low-pass on",
            low_pass(200.0, DB48),
            vec![(LP_ON, 0.0)],
            vec![(LP_ON, 1.0)],
        ),
    ];
    let inputs = modulation_inputs(2.5);
    let mut failures = Vec::new();
    for (name, base, from, to) in steps {
        for (input, source) in inputs.iter().zip(["noise", "sine"]) {
            let ends = [&from, &to].map(|end| steady(&[base.clone(), end.clone()].concat(), input));
            let peak_bound = ends[0].0.max(ends[1].0);
            let step_bound = ends[0].1.max(ends[1].1);
            for (from, to) in [(&from, &to), (&to, &from)] {
                let (peak, step) = modulated(&base, from, to, input);
                if peak > 2.0 * peak_bound {
                    failures.push(format!(
                        "{name} on {source}: peak {peak}, steady {peak_bound}"
                    ));
                }
                if step > 1.5 * step_bound + 0.02 {
                    failures.push(format!(
                        "{name} on {source}: step {step}, steady {step_bound}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The gain in dB a running default device ends up with at `frequency`
/// after each of `edits` in turn, with audio flowing in between.
fn edited_live_db(edits: &[Values], frequency: f32) -> f32 {
    let rate = 48_000.0;
    let mut device = device(rate as f32, &[]);
    let input = sine(f64::from(frequency), 0.25, rate, 48_000);
    for edit in edits {
        run_mono(&mut *device, &input);
        set(&mut *device, edit);
    }
    let output = run_mono(&mut *device, &input);
    let skip = 24_000;
    db(fit(&output[skip..], f64::from(frequency), rate)
        / fit(&input[skip..], f64::from(frequency), rate))
}

#[test]
fn a_flat_band_changes_type_while_running() {
    let b = BANDS[1];
    let cases: [(Vec<Values>, f32); 4] = [
        // Type first, the gain afterwards.
        (
            vec![
                vec![(b + BAND_TYPE, LOW_SHELF)],
                vec![(b + BAND_GAIN, 12.0), (b + BAND_FREQUENCY, 2000.0)],
            ],
            400.0,
        ),
        // All in one change.
        (vec![band(1, HIGH_SHELF, 200.0, 12.0, 0.71)], 2000.0),
        (vec![vec![(b + BAND_TYPE, NOTCH)]], 300.0),
        // And back to a bell that then does something.
        (
            vec![
                vec![(b + BAND_TYPE, NOTCH), (b + BAND_ON, 0.0)],
                vec![(b + BAND_TYPE, BELL), (b + BAND_ON, 1.0)],
                vec![(b + BAND_GAIN, -9.0)],
            ],
            400.0,
        ),
    ];
    for (edits, frequency) in cases {
        let measured = edited_live_db(&edits, frequency);
        let drawn = drawn(&edits.concat(), &[frequency])[0];
        assert!(drawn.abs() > 3.0, "{edits:?}: the case shows nothing");
        assert!(
            (measured - drawn).abs() <= 0.1,
            "{edits:?}: measured {measured}, drawn {drawn}"
        );
    }
}

#[test]
fn factory_presets_load_into_a_running_device() {
    for (text, frequency) in [
        (
            include_str!("../presets/r1-equalizer/hum-notch-50-hz.rillpreset"),
            100.0,
        ),
        (
            include_str!("../presets/r1-equalizer/telephone.rillpreset"),
            100.0,
        ),
    ] {
        let values = preset(text);
        let measured = edited_live_db(std::slice::from_ref(&values), frequency);
        let drawn = drawn(&values, &[frequency])[0].max(-60.0);
        assert!(drawn.abs() > 3.0, "the case shows nothing");
        assert!(
            (measured.max(-60.0) - drawn).abs() <= 0.1,
            "at {frequency} Hz: measured {measured}, drawn {drawn}"
        );
    }
}

#[test]
fn a_band_placed_while_off_stays_silent() {
    let b = BANDS[1];
    let mut device = device(48_000.0, &[]);
    let input = sine(400.0, 0.25, 48_000.0, 24_000);
    for edit in [
        vec![(b + BAND_ON, 0.0)],
        vec![(b + BAND_GAIN, 24.0), (b + BAND_Q, 18.0)],
        vec![(b + BAND_TYPE, NOTCH)],
    ] {
        set(&mut *device, &edit);
        assert_eq!(run_mono(&mut *device, &input), input, "{edit:?}");
    }
}

/// Pink noise from 40 Hz to 16 kHz at −20 dBFS RMS: equal power per octave,
/// built from sines so that it is the same on every run.
fn pink(rate: f64, frames: usize) -> Vec<f32> {
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
    let scale = 0.1 / (TONES as f64 / 2.0).sqrt();
    signal.iter().map(|x| (x * scale) as f32).collect()
}

fn preset(text: &str) -> Values {
    let resolved = Preset::parse(text)
        .unwrap()
        .resolve(&equalizer::DESCRIPTOR)
        .unwrap();
    resolved
        .values
        .iter()
        .map(|&(id, normalized)| {
            (
                id,
                equalizer::DESCRIPTOR
                    .parameter(id)
                    .unwrap()
                    .to_plain(normalized),
            )
        })
        .collect()
}

#[test]
fn auto_gain_keeps_the_loudness_of_pink_noise() {
    let rate = 48_000.0;
    let input = pink(f64::from(rate), 96_000);
    assert!((db(rms(&input)) + 20.0).abs() < 0.1);
    // Cuts are not compensated, so the presets are measured without theirs.
    let without_cuts = vec![(HP_ON, 0.0), (LP_ON, 0.0)];
    let cases: [(&str, Values, Option<f32>); 4] = [
        ("bell", band(1, BELL, 1000.0, 6.0, 1.0), Some(-1.6)),
        (
            "low shelf",
            band(0, LOW_SHELF, 100.0, 6.0, 0.71),
            Some(-1.4),
        ),
        (
            "Vocal Presence",
            [
                preset(include_str!(
                    "../presets/r1-equalizer/vocal-presence.rillpreset"
                )),
                without_cuts.clone(),
            ]
            .concat(),
            None,
        ),
        (
            "Analog Smile",
            [
                preset(include_str!(
                    "../presets/r1-equalizer/analog-smile.rillpreset"
                )),
                without_cuts,
            ]
            .concat(),
            None,
        ),
    ];
    for (name, values, expected) in cases {
        let level = |auto: f32| {
            let values = [values.clone(), vec![(AUTO_GAIN, auto)]].concat();
            let output = run_mono(&mut *device(rate, &values), &input);
            rms(&output[24_000..])
        };
        let (off, on) = (level(0.0), level(1.0));
        let matched = db(on / rms(&input[24_000..]));
        assert!(
            matched.abs() <= 1.0,
            "{name}: {matched} dB louder than the input"
        );
        // What it applied is what the panel shows.
        let applied = db(on / off);
        let shown = auto_gain_db(
            &settings(&[values.clone(), vec![(AUTO_GAIN, 1.0)]].concat()),
            rate,
        );
        assert!(
            (applied - shown).abs() <= 0.02,
            "{name}: applied {applied}, shown {shown}"
        );
        let off = settings(&[values.clone(), vec![(AUTO_GAIN, 0.0)]].concat());
        assert_eq!(auto_gain_db(&off, rate), 0.0, "{name}: off");
        if let Some(expected) = expected {
            assert!((shown - expected).abs() <= 0.1, "{name}: {shown} dB");
        }
    }
}

#[test]
fn auto_gain_leaves_the_cut_filters_alone() {
    let values = [
        high_pass(200.0, DB48),
        low_pass(3000.0, DB24),
        vec![(AUTO_GAIN, 1.0)],
    ]
    .concat();
    assert_eq!(auto_gain_db(&settings(&values), 48_000.0), 0.0);
    let gain = measured_db(&values, 48_000.0, &[900.0], 0.1, 0.1)[0];
    assert!(gain.abs() <= 0.02, "{gain} dB");
    // A switched-off band does not count either.
    let mut off = band(1, BELL, 1000.0, 12.0, 1.0);
    off.extend([(BANDS[1] + BAND_ON, 0.0), (AUTO_GAIN, 1.0)]);
    assert_eq!(auto_gain_db(&settings(&off), 48_000.0), 0.0);
    // Extreme curves are compensated by at most 12 dB.
    let extreme: Values = (0..4)
        .flat_map(|index| band(index, BELL, 1000.0, 24.0, 0.1))
        .chain([(AUTO_GAIN, 1.0)])
        .collect();
    assert_eq!(auto_gain_db(&settings(&extreme), 48_000.0), -12.0);
}

#[test]
fn latency_is_zero_and_the_tail_covers_the_ringing() {
    let rate = 48_000.0;
    for mode in [0.0, 1.0] {
        let device = device(
            rate,
            &[band(1, BELL, 1000.0, 6.0, 1.0), vec![(MODE, mode)]].concat(),
        );
        assert_eq!(device.latency_samples(), 0);
    }
    assert_eq!(device(rate, &[]).tail_samples(), 0);
    // Flat bands and an idle cut filter ring no longer than nothing.
    let mut flat = device(rate, &[]);
    set(
        &mut *flat,
        &[(BANDS[0] + BAND_Q, 18.0), (HP_FREQUENCY, 20.0)],
    );
    assert_eq!(flat.tail_samples(), 0);
    // The oversampler of the Analog stage rings a little on its own.
    assert_eq!(device(rate, &[ANALOG]).tail_samples(), 64);

    let cases = [
        (band(1, BELL, 20.0, 24.0, 18.0), Some(16.75)),
        (high_pass(100.0, DB48), None),
        (low_pass(200.0, DB48), None),
        (band(1, BELL, 20_000.0, -24.0, 18.0), None),
        (band(1, BELL, 60.0, 4.0, 1.2), Some(0.117)),
        (band(1, BELL, 60.0, -4.0, 1.2), None),
        (band(1, NOTCH, 50.0, 0.0, 12.0), None),
    ];
    for (values, seconds) in cases {
        let mut device = device(rate, &values);
        let tail = device.tail_samples() as usize;
        if let Some(seconds) = seconds {
            let reported = tail as f32 / rate;
            assert!(
                (reported / seconds - 1.0).abs() < 0.05,
                "{values:?}: {reported} s"
            );
        }
        let mut impulse = vec![0.0; tail + 24_000];
        impulse[0] = 1.0;
        let output = run_mono(&mut *device, &impulse);
        let late = db(f64::from(peak(&output[tail..])) / f64::from(peak(&output)));
        assert!(late <= -120.0, "{values:?}: {late} dB after {tail} samples");
        // Not a blanket figure either: a third of the way it still rings.
        let early = db(f64::from(peak(&output[tail / 3..])) / f64::from(peak(&output)));
        assert!(
            early > -120.0,
            "{values:?}: {early} dB after a third of the tail"
        );
    }
    // Never more than the engine's 30 seconds, also where a pole barely decays.
    let slow = device(192_000.0, &band(1, BELL, 20.0, 24.0, 18.0));
    assert!(slow.tail_samples() <= 30 * 192_000);
    // The tail follows the parameters at once, without processing.
    let mut device = device(rate, &[]);
    set(&mut *device, &high_pass(20.0, DB48));
    let seconds = device.tail_samples() as f32 / rate;
    assert!((seconds - 0.59).abs() < 0.03, "{seconds} s");
}

#[test]
fn telemetry_reports_the_sample_rate_and_both_sides() {
    for rate in [44_100.0, 96_000.0] {
        let mut device = (descriptor("r1-equalizer").unwrap().create)();
        device.prepare(rate, 512);
        let telemetry = device.telemetry();
        assert_eq!(telemetry.value(TELEMETRY_SAMPLE_RATE), rate);
        set(&mut *device, &[(OUTPUT, -6.0)]);
        device.reset();
        let input = Noise(31).block(0.5, 2000);
        let output = run_mono(&mut *device, &input);
        let mut seen = vec![0.0; 300];
        telemetry.read_input(&mut seen);
        assert_eq!(seen, input[1700..]);
        telemetry.read_output(&mut seen);
        assert_eq!(seen, output[1700..]);
        assert!(telemetry.output_peak() < telemetry.input_peak() && telemetry.output_peak() > 0.0);
    }
}

#[test]
fn settings_and_curves_follow_the_parameters() {
    let defaults = settings(&[]);
    assert!(!defaults.hp.on && !defaults.lp.on);
    assert!((defaults.hp.frequency - 30.0).abs() < 1e-3 && defaults.hp.slope == Slope::Db12);
    assert!((defaults.lp.frequency - 18_000.0).abs() < 0.1 && defaults.lp.slope == Slope::Db12);
    let kinds = defaults.bands.map(|band| band.kind);
    assert_eq!(
        kinds,
        [
            BandType::LowShelf,
            BandType::Bell,
            BandType::Bell,
            BandType::HighShelf
        ]
    );
    for (band, (frequency, q)) in
        defaults
            .bands
            .iter()
            .zip([(100.0, 0.71), (400.0, 1.0), (2500.0, 1.0), (8000.0, 0.71)])
    {
        assert!(band.on && band.gain_db == 0.0);
        assert!((band.frequency / frequency - 1.0).abs() < 1e-5 && (band.q / q - 1.0).abs() < 1e-5);
    }
    assert_eq!(
        (
            defaults.mode,
            defaults.auto_gain,
            defaults.analyzer,
            defaults.output_db
        ),
        (Mode::Digital, false, Analyzer::Output, 0.0)
    );
    assert_eq!(
        [Slope::Db12, Slope::Db24, Slope::Db48].map(Slope::sections),
        [1, 2, 4]
    );

    // Gain means nothing to a notch; everything else always applies.
    let notch = settings(&band(2, NOTCH, 1000.0, 0.0, 4.0));
    for spec in equalizer::DESCRIPTOR.parameters {
        assert_eq!(
            applies(spec.id, &notch),
            spec.id != BANDS[2] + BAND_GAIN,
            "{}",
            spec.key
        );
        assert!(applies(spec.id, &defaults));
    }

    // The bands' own curves add up to the whole, and a flat or disabled
    // band draws a flat line.
    let values = [
        high_pass(80.0, DB24),
        band(0, LOW_SHELF, 120.0, 6.0, 0.71),
        band(1, BELL, 500.0, -9.0, 2.0),
        band(2, NOTCH, 3000.0, 0.0, 3.0),
        vec![(BANDS[3] + BAND_GAIN, 9.0), (BANDS[3] + BAND_ON, 0.0)],
        low_pass(9000.0, DB48),
    ]
    .concat();
    let settings = settings(&values);
    let frequencies = log_spaced(20.0, 20_000.0, 60);
    let mut whole = vec![0.0; 60];
    response_db(&settings, 48_000.0, &frequencies, &mut whole);
    let mut sum = vec![0.0; 60];
    for band in 0..6 {
        let mut curve = vec![0.0; 60];
        band_response_db(&settings, band, 48_000.0, &frequencies, &mut curve);
        if band == 4 {
            assert!(curve.iter().all(|level| *level == 0.0));
        } else {
            assert!(curve.iter().any(|level| level.abs() > 3.0), "band {band}");
        }
        for (sum, level) in sum.iter_mut().zip(curve) {
            *sum += level;
        }
    }
    for (whole, sum) in whole.iter().zip(sum) {
        // Deep in the stop bands the floor of the drawing clips the product.
        assert!(
            (whole - sum).abs() < 1e-3 || *whole < -150.0,
            "{whole} {sum}"
        );
    }
    assert!(band_prototype(BandType::Bell, 0.0, 1.0, Mode::Analog).is_none());
    assert!(band_prototype(BandType::Notch, 0.0, 1.0, Mode::Digital).is_some());
    let designer = daw_devices::dsp::MatchedDesigner::new(48_000.0);
    assert!(
        band_mix(
            &designer,
            BandType::HighShelf,
            5000.0,
            0.0,
            0.71,
            Mode::Digital
        )
        .is_identity()
    );
    let sections = design(&settings, &designer);
    let active: Vec<bool> = sections.iter().map(Option::is_some).collect();
    assert_eq!(
        active,
        [
            true, true, false, false, true, true, true, false, true, true, true, true
        ]
    );
    assert_eq!(
        sections[8],
        Some(cut_mix(
            &designer,
            false,
            f64::from(settings.lp.frequency),
            Slope::Db48,
            0
        ))
    );
}
