mod support;

use daw_devices::devices::modulator::{self, *};
use daw_devices::{Device, ProcessContext, StereoIo};
use support::*;

const CHORUS: f32 = 0.0;
const FLANGER: f32 = 1.0;
const PHASER: f32 = 2.0;
const MODE_NAMES: [(f32, &str); 3] = [(CHORUS, "Chorus"), (FLANGER, "Flanger"), (PHASER, "Phaser")];
const CLEAN: f32 = 0.0;
const ANALOG: f32 = 1.0;
const LATENCY: usize = modulator::LATENCY as usize;

fn device(rate: f32, values: &[(u32, f32)]) -> Box<dyn Device> {
    support::device(&modulator::DESCRIPTOR, rate, values)
}

fn settings(values: &[(u32, f32)]) -> Settings {
    Settings::from_normalized(value_of(&modulator::DESCRIPTOR, values))
}

fn impulse_response(values: &[(u32, f32)], rate: f32, amplitude: f32, frames: usize) -> Vec<f32> {
    let mut input = vec![0.0; frames];
    input[0] = amplitude;
    run_mono(&mut *device(rate, values), &input)
}

/// The frequency between `low` and `high` where an impulse response is
/// weakest, to a hundredth of a percent.
fn notch(response: &[f32], rate: f64, low: f64, high: f64) -> (f64, f64) {
    let (mut low, mut high) = (low, high);
    let mut best = (low, f64::MAX);
    for _ in 0..4 {
        let step = (high - low) / 40.0;
        for point in 0..=40 {
            let hz = low + step * f64::from(point);
            let level = response_at(response, 1.0, hz, rate);
            if level < best.1 {
                best = (hz, level);
            }
        }
        (low, high) = (best.0 - step, best.0 + step);
    }
    best
}

#[test]
fn still_response_matches_the_drawn_one() {
    for rate in [44_100.0_f32, 48_000.0, 96_000.0] {
        let top = (0.36 * rate).min(20_000.0);
        for (mode, name) in MODE_NAMES {
            for (character, amplitude) in [(CLEAN, 1.0), (ANALOG, 0.001)] {
                for feedback in [0.0, 70.0, -70.0] {
                    let values = [
                        (MODE, mode),
                        (CHARACTER, character),
                        (DEPTH, 0.0),
                        (VOICES, 1.0),
                        (FEEDBACK, feedback),
                    ];
                    let settings = settings(&values);
                    // A chorus at 70 % takes 33 passes of 12 ms to fall by
                    // 100 dB.
                    let frames = (0.45 * rate) as usize;
                    let response = impulse_response(&values, rate, amplitude, frames);
                    for point in 0..36 {
                        let hz = 30.0 * (top / 30.0).powf(point as f32 / 35.0);
                        let (mut low, mut high) = ([0.0], [0.0]);
                        response_band_db(&settings, [0.0; 4], rate, &[hz], &mut low, &mut high);
                        assert_eq!(low, high);
                        let measured = response_at(
                            &response,
                            f64::from(amplitude),
                            f64::from(hz),
                            f64::from(rate),
                        );
                        // In a notch a decibel is no measure: both are held
                        // against a floor 40 dB down.
                        let floor = 0.01;
                        let expected = f64::from(10.0_f32.powf(low[0] / 20.0));
                        let error = db((measured + floor) / (expected + floor));
                        let tolerance = if hz <= 0.3 * rate { 0.1 } else { 0.3 };
                        assert!(
                            error.abs() <= tolerance,
                            "{name} character {character} feedback {feedback} at {rate} Hz, \
                             {hz} Hz: {error} dB off {}",
                            low[0]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn flanger_notches_sit_at_odd_multiples_of_half_the_delay_rate() {
    let rate = 48_000.0;
    let values = [
        (MODE, FLANGER),
        (DEPTH, 0.0),
        (FEEDBACK, 0.0),
        (CENTER_FLANGER, 1.0),
    ];
    let response = impulse_response(&values, 48_000.0, 1.0, 2000);
    let tooth = response_at(&response, 1.0, 1000.0, rate);
    for k in 0..5 {
        let expected = 500.0 * f64::from(2 * k + 1);
        let (hz, level) = notch(&response, rate, 0.97 * expected, 1.03 * expected);
        assert!(
            (hz / expected - 1.0).abs() <= 0.005,
            "notch at {hz} Hz, not {expected}"
        );
        let depth = db(level / tooth);
        assert!(depth <= -40.0, "notch at {expected} Hz: {depth} dB");
    }
}

#[test]
fn phaser_notches_sit_where_the_chain_turns_half_circles() {
    let rate = 48_000.0;
    for (index, stages) in STAGE_COUNTS.iter().enumerate() {
        let values = [
            (MODE, PHASER),
            (DEPTH, 0.0),
            (FEEDBACK, 0.0),
            (STAGES, index as f32),
        ];
        let response = impulse_response(&values, 48_000.0, 1.0, 9600);
        let g = (std::f64::consts::PI * 800.0 / (2.0 * rate)).tan();
        for k in 0..stages / 2 {
            let turn = f64::from(2 * k as u32 + 1) * std::f64::consts::PI / (2 * stages) as f64;
            let expected = (g * turn.tan()).atan() * 2.0 * rate / std::f64::consts::PI;
            let (hz, level) = notch(&response, rate, 0.97 * expected, 1.03 * expected);
            assert!(
                (hz / expected - 1.0).abs() <= 0.005,
                "{stages} stages: notch at {hz} Hz, not {expected}"
            );
            assert!(db(level) < -40.0, "{stages} stages, {expected} Hz");
        }
    }
}

#[test]
fn a_vibrato_adds_nothing_away_from_the_tone() {
    let rate = 48_000.0;
    let (frequency, bin) = on_bin(1000.0, rate);
    let input = sine(frequency, 0.5, rate, FFT + 9600);
    let values = [(VOICES, 1.0), (MIX, 100.0), (RATE, 2.0), (DEPTH, 50.0)];
    let output = run_mono_free(&mut *device(rate as f32, &values), &input).0;
    let reference = spectrum(&input)[bin];
    let spectrum = spectrum(&output);
    let away = (200.0 * FFT as f64 / rate) as usize;
    let worst = spectrum
        .iter()
        .enumerate()
        .filter(|(index, _)| index.abs_diff(bin) > away)
        .map(|(_, level)| *level)
        .fold(0.0, f64::max);
    assert!(
        db(worst / reference) < -90.0,
        "{} dB",
        db(worst / reference)
    );
    // The sidebands themselves are there: the tone is modulated.
    assert!(db(spectrum[bin + 8] / reference) > -40.0);
}

#[test]
fn vibrato_bends_the_pitch_by_the_slope_of_the_delay() {
    let rate = 48_000.0;
    let values = [(VOICES, 1.0), (MIX, 100.0), (RATE, 5.0), (DEPTH, 50.0)];
    let input = sine(1000.0, 0.5, rate, 48_000);
    let output = run_mono_free(&mut *device(rate as f32, &values), &input).0;
    let pitch = instantaneous_hz(&output, 1000.0, rate, 48);
    let deviation = pitch[9600..]
        .iter()
        .map(|hz| (hz - 1000.0).abs())
        .fold(0.0, f64::max);
    let expected = std::f64::consts::TAU * 5.0 * 0.5 * 0.004 * 1000.0;
    assert!(
        (deviation / expected - 1.0).abs() <= 0.05,
        "{deviation} Hz against {expected}"
    );
}

#[test]
fn analog_colour_does_not_alias() {
    let rate = 48_000.0;
    for (mode, name) in [(CHORUS, "Chorus"), (PHASER, "Phaser")] {
        let values = [
            (MODE, mode),
            (CHARACTER, ANALOG),
            (DEPTH, 0.0),
            (MIX, 100.0),
        ];
        for tone in [100.0, 1000.0, 2217.0, 5000.0] {
            let (frequency, bin) = on_bin(tone, rate);
            let input = sine(frequency, 0.5, rate, FFT + FFT / 4);
            let output = run_mono_free(&mut *device(rate as f32, &values), &input).0;
            let worst = worst_inharmonic_db(
                &spectrum(&output),
                bin,
                on_bin(20.0, rate).1 + SKIRT,
                on_bin(20_000.0, rate).1,
            );
            assert!(worst <= -70.0, "{name} at {tone} Hz: {worst} dBc");
        }
    }
}

#[test]
fn full_feedback_stays_bounded() {
    let rate = 48_000.0;
    let mut noise = Noise(5);
    let input: Vec<f32> = (0..48_000).map(|_| noise.next()).collect();
    for (mode, name) in MODE_NAMES {
        for character in [CLEAN, ANALOG] {
            for feedback in [95.0, -95.0] {
                let values = [
                    (MODE, mode),
                    (CHARACTER, character),
                    (FEEDBACK, feedback),
                    (DEPTH, 100.0),
                    (RATE, 2.0),
                ];
                let output = run_mono_free(&mut *device(rate, &values), &input).0;
                assert!(output.iter().all(|x| x.is_finite()));
                let label = format!("{name} character {character} feedback {feedback}");
                if mode == PHASER {
                    // With the corner at the bottom of its sweep nearly the
                    // whole band is in phase with the fed-back signal and
                    // comes out up to 26 dB up, as the drawn response says:
                    // no fixed ceiling holds, the largest gain of the
                    // response does.
                    let settings = settings(&values);
                    let largest = (1.0 - settings.mix
                        + settings.mix / (1.0 - settings.feedback.abs()))
                        * settings.loudness();
                    let gain = rms(&output) / rms(&input);
                    assert!(gain < f64::from(largest), "{label}: {gain} of {largest}");
                } else {
                    let peak = db(f64::from(peak(&output)));
                    assert!(peak < 12.0, "{label}: {peak} dBFS");
                }
            }
        }
    }
}

/// Noise that repeats every `PERIOD` frames, at `rms`: every FFT bin from
/// 20 Hz to 20 kHz at a random phase, of equal level (`pink` false) or
/// falling by 3 dB an octave.
fn band_noise(rate: f64, pink: bool, rms: f64) -> Vec<f32> {
    use rustfft::num_complex::Complex64;
    let mut random = Noise(41);
    let mut bins = vec![Complex64::new(0.0, 0.0); PERIOD];
    let bin_of = |hz: f64| (hz * PERIOD as f64 / rate) as usize;
    for bin in bin_of(20.0).max(1)..=bin_of(20_000.0) {
        let level = if pink { 1.0 / (bin as f64).sqrt() } else { 1.0 };
        bins[bin] = Complex64::from_polar(level, std::f64::consts::PI * f64::from(random.next()));
        bins[PERIOD - bin] = bins[bin].conj();
    }
    rustfft::FftPlanner::new()
        .plan_fft_inverse(PERIOD)
        .process(&mut bins);
    let signal: Vec<f32> = bins.iter().map(|x| x.re as f32).collect();
    let scale = (rms / support::rms(&signal)) as f32;
    signal.iter().map(|x| x * scale).collect()
}

const PERIOD: usize = 1 << 15;

#[test]
fn loudness_holds_across_mix_and_feedback() {
    let rate = 48_000.0;
    for (mode, name) in MODE_NAMES {
        // Broadband material inside the audio band. A comb has teeth all
        // over it and is held against white noise. A phaser turns the phase
        // around one place and is level elsewhere: it is normalised over
        // the pitch axis, as music fills it, and white noise would weigh
        // the top two octaves as much as everything below them.
        let period = band_noise(f64::from(rate), mode == PHASER, 0.25);
        let input: Vec<f32> = period.iter().chain(&period).copied().collect();
        let level = rms(&period);
        for feedback in [-95.0, 0.0, 95.0] {
            for mix in [0.0, 25.0, 50.0, 75.0, 100.0] {
                let values = [(MODE, mode), (FEEDBACK, feedback), (MIX, mix), (RATE, 2.0)];
                let output = run_mono_free(&mut *device(rate, &values), &input).0;
                let change = db(rms(&output[PERIOD..]) / level);
                assert!(
                    change.abs() <= 1.0,
                    "{name} feedback {feedback} mix {mix}: {change} dB"
                );
            }
        }
    }
}

#[test]
fn dry_and_wet_are_aligned_to_the_sample() {
    for rate in [44_100.0_f32, 48_000.0, 96_000.0] {
        let values = [
            (MODE, FLANGER),
            (DEPTH, 0.0),
            (FEEDBACK, 0.0),
            (CENTER_FLANGER, 2.0),
        ];
        let response = impulse_response(&values, rate, 1.0, 600);
        let loudest = |range: std::ops::Range<usize>| {
            range
                .clone()
                .max_by(|a, b| response[*a].abs().total_cmp(&response[*b].abs()))
                .unwrap()
        };
        assert_eq!(loudest(0..LATENCY + 20), LATENCY, "dry at {rate} Hz");
        let wet = LATENCY + (0.002 * rate).round() as usize;
        assert_eq!(loudest(LATENCY + 20..600), wet, "wet at {rate} Hz");
        // Nothing before the dry impulse but the wet path's silence.
        assert_eq!(peak(&response[..LATENCY]), 0.0);
    }
}

#[test]
fn stereo_spreads_the_sweep() {
    let rate = 48_000.0;
    let mut noise = Noise(9);
    let input: Vec<f32> = (0..24_000).map(|_| 0.5 * noise.next()).collect();
    for (mode, _) in MODE_NAMES {
        // run_mono holds the channels against each other, bit for bit.
        run_mono(
            &mut *device(rate, &[(MODE, mode), (STEREO, 0.0), (DEPTH, 100.0)]),
            &input,
        );
    }

    let mut opposite = device(rate, &[(VOICES, 1.0), (STEREO, 180.0)]);
    run_mono_free(&mut *opposite, &input[..14_400]);
    let telemetry = opposite.telemetry();
    let (left, right) = (
        telemetry.value(TELEMETRY_SWEEP_LEFT),
        telemetry.value(TELEMETRY_SWEEP_RIGHT),
    );
    assert!(left > 0.5, "0.18 of a cycle in: {left}");
    assert!((left + right).abs() < 1e-3, "{left} and {right}");

    for voices in [2.0, 3.0, 4.0] {
        for stereo in [90.0, 180.0] {
            let values = [(VOICES, voices), (STEREO, stereo), (MIX, 100.0)];
            let (left, right) = run_mono_free(&mut *device(rate, &values), &input);
            let side: Vec<f32> = left.iter().zip(&right).map(|(l, r)| l - r).collect();
            let mid: Vec<f32> = left.iter().zip(&right).map(|(l, r)| l + r).collect();
            let share = (rms(&side) / rms(&mid)).powi(2);
            assert!(share >= 0.1, "{voices} voices at {stereo}°: {share}");
        }
    }
}

/// The same signal on both channels, which may come out different.
fn run_mono_free(device: &mut dyn Device, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (input.to_vec(), input.to_vec());
    run(device, &mut left, &mut right);
    (left, right)
}

/// Processes with the song running from `beat` at 120 BPM; returns the left
/// channel and moves `beat` on.
fn run_playing(device: &mut dyn Device, input: &[f32], rate: f32, beat: &mut f64) -> Vec<f32> {
    let (mut left, mut right) = (input.to_vec(), input.to_vec());
    for (left, right) in left.chunks_mut(500).zip(right.chunks_mut(500)) {
        let context = ProcessContext {
            tempo: 120.0,
            beat: *beat,
            playing: true,
        };
        *beat += left.len() as f64 * 2.0 / f64::from(rate);
        device.process(
            &context,
            &[],
            StereoIo {
                left,
                right,
                sidechain: None,
            },
        );
    }
    left
}

#[test]
fn a_synced_lfo_is_a_function_of_the_song_position() {
    let rate = 48_000.0;
    // One bar.
    let values = [(SYNC, 1.0), (DIVISION, 3.0)];
    let phase_at = |beat: f64| {
        let mut device = device(rate, &values);
        run_playing(&mut *device, &[0.0], rate, &mut { beat });
        device.telemetry().value(TELEMETRY_PHASE)
    };
    assert_eq!(phase_at(0.0), 0.0);
    assert_eq!(phase_at(4.0), phase_at(0.0));
    assert_eq!(phase_at(1.0), 0.25);
    assert_eq!(phase_at(7.0), 0.75);

    let mut noise = Noise(3);
    let input: Vec<f32> = (0..24_000).map(|_| 0.5 * noise.next()).collect();
    let render = || run_playing(&mut *device(rate, &values), &input, rate, &mut 2.5);
    assert_eq!(render(), render());
}

#[test]
fn switches_do_not_click() {
    let rate = 48_000.0;
    let mut device = device(rate, &[]);
    let mut position = 0;
    let mut step = |device: &mut dyn Device, frames: usize| {
        let input: Vec<f32> = (position..position + frames)
            .map(|n| 0.5 * (std::f32::consts::TAU * 220.0 * n as f32 / rate).sin())
            .collect();
        position += frames;
        largest_step(&run_mono_free(device, &input).0)
    };
    step(&mut *device, 4800);
    let steady = step(&mut *device, 48_000);
    let changes = [
        ("Flanger", (MODE, FLANGER)),
        ("Phaser", (MODE, PHASER)),
        ("12 stages", (STAGES, 4.0)),
        ("Analog", (CHARACTER, ANALOG)),
        ("2 stages", (STAGES, 0.0)),
        ("Chorus", (MODE, CHORUS)),
        ("4 voices", (VOICES, 4.0)),
        ("Clean", (CHARACTER, CLEAN)),
        ("1 voice", (VOICES, 1.0)),
        ("Flanger again", (MODE, FLANGER)),
        ("Analog flanger", (CHARACTER, ANALOG)),
    ];
    for (name, change) in changes {
        set(&mut *device, &[change]);
        let largest = step(&mut *device, 24_000);
        assert!(
            largest <= 1.5 * steady + 0.02,
            "{name}: {largest} against {steady}"
        );
    }
}

#[test]
fn lfo_changes_glide_the_pitch() {
    let rate = 48_000.0;
    let mut device = device(rate, &[(VOICES, 1.0), (DEPTH, 100.0), (MIX, 100.0)]);
    let input = sine(1000.0, 0.5, f64::from(rate), 7 * 24_000);
    let mut beat = 0.0;
    let mut output = Vec::new();
    let changes: [Option<(u32, f32)>; 7] = [
        None,
        Some((SHAPE, 2.0)),
        Some((SHAPE, 1.0)),
        Some((SYNC, 1.0)),
        // A quarter note: two cycles a second.
        Some((DIVISION, 5.0)),
        // The song jumps: the cycle is 0.37 further at once.
        None,
        Some((SYNC, 0.0)),
    ];
    for (index, (change, input)) in changes.iter().zip(input.chunks(24_000)).enumerate() {
        if let Some(change) = change {
            set(&mut *device, &[*change]);
        }
        if index == 5 {
            beat += 1.37;
        }
        output.extend(run_playing(&mut *device, input, rate, &mut beat));
    }
    let pitch = instantaneous_hz(&output[4800..], 1000.0, f64::from(rate), 48);
    let (lowest, highest) = pitch.iter().fold((f64::MAX, f64::MIN), |(low, high), hz| {
        (low.min(*hz), high.max(*hz))
    });
    assert!(
        lowest >= 920.0 && highest <= 1080.0,
        "{lowest} to {highest} Hz"
    );
    // It does move: the quarter-note triangle alone bends by 3.2 % either
    // way.
    assert!(highest - lowest > 50.0, "{lowest} to {highest} Hz");
}

#[test]
fn voices_keep_the_wet_level() {
    let rate = 48_000.0;
    let mut noise = Noise(21);
    let input: Vec<f32> = (0..48_000).map(|_| 0.5 * noise.next()).collect();
    let levels: Vec<f32> = [1.0, 2.0, 3.0, 4.0]
        .iter()
        .map(|voices| {
            let values = [(VOICES, *voices), (MIX, 100.0), (STEREO, 0.0)];
            db(rms(&run_mono(&mut *device(rate, &values), &input)[4800..]))
        })
        .collect();
    let (lowest, highest) = levels
        .iter()
        .fold((f32::MAX, f32::MIN), |(low, high), level| {
            (low.min(*level), high.max(*level))
        });
    assert!(highest - lowest <= 1.0, "{levels:?}");
}

#[test]
fn each_mode_brings_its_defaults_and_its_own_controls() {
    assert_eq!(
        mode_defaults(Mode::Chorus),
        [(FEEDBACK, 0.0), (CENTER_CHORUS, 12.0), (VOICES, 2.0)]
    );
    assert_eq!(
        mode_defaults(Mode::Flanger),
        [(FEEDBACK, 50.0), (CENTER_FLANGER, 1.5)]
    );
    assert_eq!(
        mode_defaults(Mode::Phaser),
        // The second choice: four stages.
        [(FEEDBACK, 50.0), (CENTER_PHASER, 800.0), (STAGES, 1.0)]
    );
    // A mode's defaults only touch what the mode uses.
    for mode in [Mode::Chorus, Mode::Flanger, Mode::Phaser] {
        let chosen = Settings {
            mode,
            ..settings(&[])
        };
        for (id, _) in mode_defaults(mode) {
            assert!(applies(id, &chosen), "{mode:?} sets {id}");
        }
    }

    let by_mode = |mode: f32| settings(&[(MODE, mode), (SYNC, 0.0)]);
    let shown = |settings: &Settings| -> Vec<u32> {
        modulator::PARAMETERS
            .iter()
            .map(|spec| spec.id)
            .filter(|id| !applies(*id, settings))
            .collect()
    };
    assert_eq!(
        shown(&by_mode(CHORUS)),
        [DIVISION, CENTER_FLANGER, CENTER_PHASER, STAGES]
    );
    assert_eq!(
        shown(&by_mode(FLANGER)),
        [DIVISION, CENTER_CHORUS, CENTER_PHASER, VOICES, STAGES]
    );
    assert_eq!(
        shown(&by_mode(PHASER)),
        [DIVISION, CENTER_CHORUS, CENTER_FLANGER, VOICES]
    );
    assert_eq!(
        shown(&settings(&[(SYNC, 1.0)])),
        [RATE, CENTER_FLANGER, CENTER_PHASER, STAGES]
    );
}

#[test]
fn latency_is_the_oversampler_at_every_rate_and_setting() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for (mode, _) in MODE_NAMES {
            for character in [CLEAN, ANALOG] {
                let device = device(rate, &[(MODE, mode), (CHARACTER, character)]);
                assert_eq!(device.latency_samples(), 31);
                assert_eq!((modulator::DESCRIPTOR.latency)(&|_| 0.0, rate), 31);
            }
        }
    }
}
