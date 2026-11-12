mod support;

use daw_devices::devices::sampler::{self, *};
use daw_devices::{Device, Event, EventKind, ProcessContext, SampleData, StereoIo};
use std::sync::Arc;
use support::{db, largest_step, rms};

const RATE: f32 = 48_000.0;
const BLOCK: usize = 256;

fn recording(rate: f32, seconds: f32, mut sample: impl FnMut(f32) -> f32) -> Arc<SampleData> {
    let samples: Vec<f32> = (0..(seconds * rate) as usize)
        .map(|n| sample(n as f32 / rate))
        .collect();
    Arc::new(SampleData::new(rate, &[&samples]))
}

fn sine(rate: f32, seconds: f32, hz: f32, amplitude: f32) -> Arc<SampleData> {
    // The phase in f64: in f32 it is off by -60 dB after a second at 9 kHz.
    let mut n = 0.0_f64;
    recording(rate, seconds, |_| {
        let phase = std::f64::consts::TAU * f64::from(hz) * n / f64::from(rate);
        n += 1.0;
        amplitude * phase.sin() as f32
    })
}

/// A sampler at unit gain, velocity-independent, with `values` set.
fn sampler(sample: Arc<SampleData>, values: &[(u32, f32)]) -> Box<dyn Device> {
    let mut device = (sampler::DESCRIPTOR.create)();
    device.prepare(RATE, BLOCK);
    device.set_sample(0, Some(sample));
    let defaults = [(VOLUME, 0.0), (VELOCITY_LEVEL, 0.0)];
    for &(id, plain) in defaults.iter().chain(values) {
        let spec = sampler::DESCRIPTOR.parameter(id).unwrap();
        device.set_parameter(id, spec.to_normalized(plain));
    }
    device.reset();
    device
}

fn on(note: u32) -> EventKind {
    EventKind::NoteOn {
        note,
        pitch: note as f32,
        velocity: 1.0,
    }
}

fn off(note: u32) -> EventKind {
    EventKind::NoteOff { note }
}

fn at(seconds: f32) -> usize {
    (seconds * RATE) as usize
}

/// Renders `seconds` of both channels, delivering each event at its frame.
fn render(
    device: &mut dyn Device,
    seconds: f32,
    events: &[(f32, EventKind)],
) -> (Vec<f32>, Vec<f32>) {
    let total = at(seconds);
    let (mut out_left, mut out_right) = (Vec::new(), Vec::new());
    let mut left = vec![0.0; BLOCK];
    let mut right = vec![0.0; BLOCK];
    for start in (0..total).step_by(BLOCK) {
        let block: Vec<Event> = events
            .iter()
            .filter(|(time, _)| (start..start + BLOCK).contains(&at(*time)))
            .map(|(time, kind)| Event {
                offset: (at(*time) - start) as u32,
                kind: *kind,
            })
            .collect();
        device.process(
            &ProcessContext::default(),
            &block,
            StereoIo {
                left: &mut left,
                right: &mut right,
                sidechain: None,
            },
        );
        out_left.extend_from_slice(&left);
        out_right.extend_from_slice(&right);
    }
    (out_left, out_right)
}

fn span(signal: &[f32], from: f32, to: f32) -> &[f32] {
    &signal[at(from)..at(to)]
}

/// How much of `signal` is a sine at `hz`: its amplitude, and the level
/// of everything else against it in dB.
fn tone(signal: &[f32], hz: f32) -> (f64, f32) {
    let omega = std::f64::consts::TAU * f64::from(hz) / f64::from(RATE);
    let (mut ss, mut sc, mut cc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (n, y) in signal.iter().enumerate() {
        let (s, c) = (omega * n as f64).sin_cos();
        let y = f64::from(*y);
        (ss, sc, cc, ys, yc) = (ss + s * s, sc + s * c, cc + c * c, ys + y * s, yc + y * c);
    }
    let determinant = ss * cc - sc * sc;
    let a = (ys * cc - yc * sc) / determinant;
    let b = (yc * ss - ys * sc) / determinant;
    // The residual itself: a sine's power over a window that is not a
    // whole number of cycles is not exactly half its amplitude squared.
    let rest: Vec<f32> = signal
        .iter()
        .enumerate()
        .map(|(n, y)| {
            let (s, c) = (omega * n as f64).sin_cos();
            (f64::from(*y) - a * s - b * c) as f32
        })
        .collect();
    let amplitude = a.hypot(b);
    (amplitude, db(rms(&rest) / (amplitude / 2.0_f64.sqrt())))
}

#[test]
fn keys_play_the_recording_at_their_interval_from_the_root() {
    // A4 recorded at 44.1 kHz, played back at 48 kHz: the rate is part of
    // the step.
    let sample = sine(44_100.0, 2.0, 440.0, 0.5);
    for (values, note, hz) in [
        (vec![(ROOT, 69.0)], 69, 440.0),
        (vec![(ROOT, 69.0)], 81, 880.0),
        (vec![(ROOT, 69.0)], 57, 220.0),
        (vec![(ROOT, 69.0)], 76, 440.0 * (7.0_f32 / 12.0).exp2()),
        (vec![(ROOT, 57.0)], 69, 880.0),
        // Without key tracking every key plays the recording as it is.
        (vec![(ROOT, 69.0), (KEY_TRACKING, 0.0)], 81, 440.0),
        (
            vec![(ROOT, 69.0), (TRANSPOSE, 7.0), (FINE, 50.0)],
            69,
            440.0 * (7.5_f32 / 12.0).exp2(),
        ),
    ] {
        let mut device = sampler(Arc::clone(&sample), &values);
        let (left, right) = render(&mut *device, 0.6, &[(0.0, on(note))]);
        assert_eq!(left, right, "a mono recording plays on both sides");
        let (amplitude, rest) = tone(span(&left, 0.1, 0.5), hz);
        assert!(
            (db(amplitude / 0.5)).abs() < 0.05,
            "note {note} {values:?}: {amplitude}"
        );
        assert!(
            rest < -70.0,
            "note {note} {values:?}: {rest} dB besides the tone"
        );
    }
}

#[test]
fn transposing_up_leaves_no_alias() {
    // A 9 kHz tone twelve semitones up is 18 kHz; nineteen up it would be
    // 27 kHz, above Nyquist, and must vanish instead of folding back.
    let sample = sine(RATE, 1.0, 9000.0, 0.5);
    let mut device = sampler(Arc::clone(&sample), &[(ROOT, 60.0)]);
    let (left, _) = render(&mut *device, 0.3, &[(0.0, on(72))]);
    let (amplitude, rest) = tone(span(&left, 0.05, 0.25), 18_000.0);
    assert!(db(amplitude / 0.5).abs() < 0.1, "{amplitude}");
    assert!(rest < -70.0, "{rest}");
    let mut device = sampler(sample, &[(ROOT, 60.0)]);
    let (left, _) = render(&mut *device, 0.3, &[(0.0, on(79))]);
    let level = db(rms(span(&left, 0.05, 0.25)) / (0.5 / 2.0_f64.sqrt()));
    assert!(level < -80.0, "aliasing at {level} dB");
}

#[test]
fn loops_sustain_a_held_note_without_clicks_and_release_it() {
    let hz = 440.0;
    let sample = sine(44_100.0, 0.25, hz, 0.5);
    // A sine moves at most this much from one sample to the next.
    let smooth = 0.5 * std::f32::consts::TAU * hz / RATE;
    let events = [(0.0, on(60)), (1.0, off(60))];
    let mut device = sampler(Arc::clone(&sample), &[]);
    let (left, _) = render(&mut *device, 1.5, &events);
    assert!(
        rms(span(&left, 0.3, 1.0)) == 0.0,
        "without a loop the note ends"
    );
    for mode in [1.0, 2.0] {
        let mut device = sampler(
            Arc::clone(&sample),
            &[
                (LOOP_MODE, mode),
                (LOOP_START, 20.0),
                (LOOP_END, 80.0),
                (CROSSFADE, 20.0),
            ],
        );
        let (left, _) = render(&mut *device, 1.8, &events);
        let held = rms(span(&left, 0.3, 1.0));
        assert!(
            db(held / (0.5 / 2.0_f64.sqrt())).abs() < 1.5,
            "{mode}: {held}"
        );
        let step = largest_step(span(&left, 0.01, 1.0));
        assert!(step < smooth * 1.5, "{mode}: step {step} against {smooth}");
        assert!(peak(span(&left, 1.5, 1.8)) < 1e-3, "{mode}: released");
    }
}

fn peak(signal: &[f32]) -> f32 {
    support::peak(signal)
}

#[test]
fn a_one_shot_plays_out_after_a_short_note() {
    let sample = sine(RATE, 0.25, 300.0, 0.5);
    let events = [(0.0, on(60)), (0.01, off(60))];
    let mut device = sampler(Arc::clone(&sample), &[(AENV_RELEASE, 5.0)]);
    let (left, _) = render(&mut *device, 0.4, &events);
    assert!(peak(span(&left, 0.1, 0.2)) < 1e-3);
    let mut device = sampler(sample, &[(AENV_RELEASE, 5.0), (LOOP_MODE, 3.0)]);
    let (left, _) = render(&mut *device, 0.4, &events);
    assert!(peak(span(&left, 0.1, 0.2)) > 0.45);
    // It ends at the recording's end, fading over the last moment.
    assert!(peak(span(&left, 0.26, 0.4)) == 0.0);
    assert!(largest_step(span(&left, 0.2, 0.3)) < 0.5 * std::f32::consts::TAU * 300.0 / RATE * 1.5);
}

#[test]
fn reverse_and_markers_choose_what_plays() {
    // A rising ramp of level over one second.
    let sample = recording(RATE, 1.0, |t| t * (std::f32::consts::TAU * 200.0 * t).sin());
    let level = |values: &[(u32, f32)], from: f32, to: f32| {
        let mut device = sampler(Arc::clone(&sample), values);
        let (left, _) = render(&mut *device, 1.0, &[(0.0, on(60))]);
        rms(span(&left, from, to))
    };
    let early = level(&[], 0.0, 0.1);
    let late = level(&[], 0.85, 0.95);
    assert!(late > 10.0 * early, "{early} {late}");
    let reversed_early = level(&[(REVERSE, 1.0)], 0.0, 0.1);
    assert!(
        (reversed_early / late - 1.0).abs() < 0.15,
        "{reversed_early} {late}"
    );
    // Start at 80 %: the loud end at once, and nothing after its 0.2 s.
    let started = level(&[(START, 80.0)], 0.0, 0.1);
    assert!(started > 0.5 * late, "{started}");
    assert_eq!(level(&[(START, 80.0)], 0.25, 0.5), 0.0);
}

#[test]
fn the_amp_envelope_has_its_attack_hold_and_decay() {
    let sample = recording(RATE, 2.0, |_| 0.5);
    let mut device = sampler(
        sample,
        &[
            (AENV_ATTACK, 50.0),
            (AENV_HOLD, 100.0),
            (AENV_DECAY, 100.0),
            (AENV_SUSTAIN, 25.0),
        ],
    );
    let (left, _) = render(&mut *device, 0.6, &[(0.0, on(60))]);
    let level = |seconds: f32| left[at(seconds)] / 0.5;
    let full = left.iter().position(|x| *x >= 0.4999).unwrap();
    assert!(full.abs_diff(at(0.05)) < 100, "{full}");
    assert!(level(0.1) > 0.999 && level(0.149) > 0.999);
    assert!(level(0.16) < 0.99);
    assert!((level(0.25) - 0.25).abs() < 0.02, "{}", level(0.25));
}

#[test]
fn the_filter_takes_the_top_off_when_on() {
    let mut noise = 1_u32;
    let sample = recording(RATE, 1.0, move |_| {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        noise as f32 / u32::MAX as f32 - 0.5
    });
    let top = |values: &[(u32, f32)]| {
        let mut device = sampler(Arc::clone(&sample), values);
        let (left, _) = render(&mut *device, 0.5, &[(0.0, on(60))]);
        // What changes from sample to sample is mostly the top octaves.
        let difference: Vec<f32> = left.windows(2).map(|pair| pair[1] - pair[0]).collect();
        rms(span(&difference, 0.05, 0.45))
    };
    let open = top(&[(CUTOFF, 500.0)]);
    let closed = top(&[(FILTER_ON, 1.0), (CUTOFF, 500.0), (FILTER_SLOPE, 1.0)]);
    assert!(db(closed / open) < -25.0, "{}", db(closed / open));
}

#[test]
fn voices_report_their_playheads() {
    let sample = sine(RATE, 1.0, 100.0, 0.5);
    let mut device = sampler(sample, &[]);
    render(&mut *device, 0.25, &[(0.0, on(60)), (0.0, on(72))]);
    let telemetry = device.telemetry();
    let mut heads: Vec<f32> = (0..MAX_VOICES)
        .map(|voice| telemetry.value(TELEMETRY_PLAYHEADS + voice))
        .filter(|head| *head >= 0.0)
        .collect();
    heads.sort_by(f32::total_cmp);
    assert_eq!(telemetry.value(TELEMETRY_VOICES), 2.0);
    // A quarter second in, and an octave up twice as far.
    assert!(
        (heads[0] - 0.25).abs() < 0.01 && (heads[1] - 0.5).abs() < 0.02,
        "{heads:?}"
    );
}

#[test]
fn without_a_sample_it_is_silent() {
    let mut device = (sampler::DESCRIPTOR.create)();
    device.prepare(RATE, BLOCK);
    let (left, right) = render(&mut *device, 0.1, &[(0.0, on(60))]);
    assert!(left.iter().chain(&right).all(|x| *x == 0.0));
}
