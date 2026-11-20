mod support;

use daw_devices::devices::strip::{self, *};
use daw_devices::devices::{compressor, equalizer};
use daw_devices::dsp::stereo_width;
use support::*;

const RATE: f32 = 48_000.0;

/// A second of noise, different on each side, at a level the dynamics hear.
fn program() -> (Vec<f32>, Vec<f32>) {
    let mut noise = Noise(9);
    let frames = RATE as usize;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for n in 0..frames {
        // A swell, so the gate and the compressor both move.
        let level = 0.02 + 0.4 * (n as f32 / frames as f32);
        left.push(level * noise.next());
        right.push(level * noise.next());
    }
    (left, right)
}

fn strip(values: &[(u32, f32)]) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = program();
    let mut device = device(&strip::DESCRIPTOR, RATE, values);
    run(&mut *device, &mut left, &mut right);
    (left, right)
}

fn assert_close(a: &[f32], b: &[f32], what: &str) {
    let worst = a
        .iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(worst < 1e-6, "{what}: differs by {worst}");
}

#[test]
fn default_strip_is_transparent() {
    let (left, right) = strip(&[]);
    let (dry_left, dry_right) = program();
    assert_close(&left, &dry_left, "left");
    assert_close(&right, &dry_right, "right");
}

#[test]
fn every_stage_is_its_r1_device_in_the_mixer_order() {
    let values = [
        (TRIM, 6.0),
        (POLARITY, 1.0),
        (HP_ON, 1.0),
        (HP_FREQUENCY, 120.0),
        (HP_SLOPE, 1.0),
        (LP_ON, 1.0),
        (LP_FREQUENCY, 9000.0),
        (GATE_ON, 1.0),
        (GATE_THRESHOLD, -30.0),
        (LOW_GAIN, 4.0),
        (MID_FREQUENCY, 700.0),
        (MID_GAIN, -5.0),
        (MID_Q, 2.0),
        (HIGH_GAIN, 3.0),
        (COMP_ON, 1.0),
        (COMP_THRESHOLD, -24.0),
        (COMP_RATIO, 5.0),
        (COMP_MAKEUP, 3.0),
        (WIDTH, 150.0),
        (PAN, -40.0),
        (OUTPUT, -2.0),
    ];
    let (left, right) = strip(&values);

    // The same chain built from the devices themselves.
    let (mut l, mut r) = program();
    let trim = -daw_devices::dsp::db_to_gain(6.0);
    for sample in l.iter_mut().chain(r.iter_mut()) {
        *sample *= trim;
    }
    let mut cut = device(
        &equalizer::DESCRIPTOR,
        RATE,
        &[
            (equalizer::HP_ON, 1.0),
            (equalizer::HP_FREQUENCY, 120.0),
            (equalizer::HP_SLOPE, 1.0),
            (equalizer::LP_ON, 1.0),
            (equalizer::LP_FREQUENCY, 9000.0),
            (equalizer::BANDS[0] + equalizer::BAND_ON, 0.0),
            (equalizer::BANDS[1] + equalizer::BAND_ON, 0.0),
            (equalizer::BANDS[2] + equalizer::BAND_ON, 0.0),
            (equalizer::BANDS[3] + equalizer::BAND_ON, 0.0),
        ],
    );
    let mut gate = device(
        &compressor::DESCRIPTOR,
        RATE,
        &[
            (compressor::MODE, 2.0),
            (compressor::KNEE, 3.0),
            (compressor::THRESHOLD, -30.0),
            (compressor::RATIO, 10.0),
            (compressor::ATTACK, 0.5),
            (compressor::RELEASE, 150.0),
        ],
    );
    let band = |index: usize, offset: u32| equalizer::BANDS[index] + offset;
    let mut eq = device(
        &equalizer::DESCRIPTOR,
        RATE,
        &[
            (band(0, equalizer::BAND_TYPE), 1.0),
            (band(0, equalizer::BAND_FREQUENCY), 100.0),
            (band(0, equalizer::BAND_GAIN), 4.0),
            (band(0, equalizer::BAND_Q), 0.71),
            (band(1, equalizer::BAND_TYPE), 0.0),
            (band(1, equalizer::BAND_FREQUENCY), 700.0),
            (band(1, equalizer::BAND_GAIN), -5.0),
            (band(1, equalizer::BAND_Q), 2.0),
            (band(2, equalizer::BAND_TYPE), 2.0),
            (band(2, equalizer::BAND_FREQUENCY), 8000.0),
            (band(2, equalizer::BAND_GAIN), 3.0),
            (band(2, equalizer::BAND_Q), 0.71),
            (band(3, equalizer::BAND_ON), 0.0),
        ],
    );
    let mut comp = device(
        &compressor::DESCRIPTOR,
        RATE,
        &[
            (compressor::THRESHOLD, -24.0),
            (compressor::RATIO, 5.0),
            (compressor::MAKEUP, 3.0),
        ],
    );
    for stage in [&mut cut, &mut gate, &mut eq, &mut comp] {
        // The strip hands its stages 64 frames at a time.
        for (l, r) in l.chunks_mut(64).zip(r.chunks_mut(64)) {
            stage.process(
                &Default::default(),
                &[],
                daw_devices::StereoIo {
                    left: l,
                    right: r,
                    sidechain: None,
                },
            );
        }
    }
    let output = daw_devices::dsp::db_to_gain(-2.0);
    let [pan_left, pan_right] = daw_devices::dsp::balance(-0.4);
    for (l, r) in l.iter_mut().zip(r.iter_mut()) {
        let [wide_l, wide_r] = stereo_width([*l, *r], 1.5);
        *l = wide_l * output * pan_left;
        *r = wide_r * output * pan_right;
    }
    assert_close(&left, &l, "left");
    assert_close(&right, &r, "right");
}

#[test]
fn width_zero_is_mono_and_pan_left_silences_the_right() {
    let (left, right) = strip(&[(WIDTH, 0.0)]);
    assert_close(&left, &right, "mono");
    let (_, right) = strip(&[(PAN, -100.0)]);
    assert!(right.iter().all(|sample| *sample == 0.0));
}

#[test]
fn switching_a_dynamics_stage_fades_instead_of_stepping() {
    let mut device = device(
        &strip::DESCRIPTOR,
        RATE,
        &[(COMP_THRESHOLD, -60.0), (COMP_RATIO, 20.0)],
    );
    let input = sine(200.0, 0.5, f64::from(RATE), 4800);
    let mut left = input.clone();
    let mut right = input.clone();
    run(&mut *device, &mut left[..2400], &mut right[..2400]);
    device.set_parameter(COMP_ON, 1.0);
    run(&mut *device, &mut left[2400..], &mut right[2400..]);
    let step = left
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0_f32, f32::max);
    // A 200 Hz sine at 0.5 moves at most 0.013 a frame at 48 kHz.
    assert!(step < 0.02, "switching on stepped by {step}");
}

#[test]
fn drawn_response_is_both_equalizers_together() {
    let values = [
        (HP_ON, 1.0),
        (HP_FREQUENCY, 200.0),
        (MID_GAIN, 6.0),
        (MID_FREQUENCY, 1000.0),
    ];
    let settings = strip::equalizer_settings(value_of(&strip::DESCRIPTOR, &values));
    let mut db = [0.0_f32; 3];
    equalizer::response_db(&settings, RATE, &[50.0, 1000.0, 15_000.0], &mut db);
    assert!(db[0] < -10.0, "high-pass: {db:?}");
    assert!((db[1] - 6.0).abs() < 0.5, "mid band: {db:?}");
    assert!(db[2].abs() < 0.5, "flat top: {db:?}");
    let off = strip::equalizer_settings(value_of(
        &strip::DESCRIPTOR,
        &[(EQ_ON, 0.0), (MID_GAIN, 6.0)],
    ));
    assert!(off.bands.iter().all(|band| !band.on));
}
