mod support;

use daw_devices::devices::drums::{self, *};
use daw_devices::{Device, Event, EventKind, ProcessContext, SampleData, StereoIo};
use std::sync::Arc;
use support::{db, largest_step, rms};

const RATE: f32 = 48_000.0;
const BLOCK: usize = 256;

fn sine(rate: f32, seconds: f32, hz: f32, amplitude: f32) -> Arc<SampleData> {
    let samples: Vec<f32> = (0..(seconds * rate) as usize)
        .map(|n| {
            let phase = std::f64::consts::TAU * f64::from(hz) * n as f64 / f64::from(rate);
            amplitude * phase.sin() as f32
        })
        .collect();
    Arc::new(SampleData::new(rate, &[&samples]))
}

/// A ramp from 0 to 1 over `seconds`: where the playhead is, read off.
fn ramp(seconds: f32) -> Arc<SampleData> {
    let frames = (seconds * RATE) as usize;
    let samples: Vec<f32> = (0..frames).map(|n| n as f32 / frames as f32).collect();
    Arc::new(SampleData::new(RATE, &[&samples]))
}

/// A kit at unit gain whose hits do not follow velocity, with `samples` by
/// pad and `values` (plain, by ID) set.
fn kit(samples: &[(usize, Arc<SampleData>)], values: &[(u32, f32)]) -> Box<dyn Device> {
    let mut device = (drums::DESCRIPTOR.create)();
    device.prepare(RATE, BLOCK);
    for (pad, sample) in samples {
        device.set_sample(*pad as u32, Some(Arc::clone(sample)));
    }
    let mut all = vec![(VOLUME, 0.0)];
    all.extend((0..PADS).map(|pad| (id(pad, VELOCITY), 0.0)));
    all.extend_from_slice(values);
    for (id, plain) in all {
        let spec = drums::DESCRIPTOR.parameter(id).unwrap();
        device.set_parameter(id, spec.to_normalized(plain));
    }
    device.reset();
    device
}

fn hit(pad: usize) -> EventKind {
    hit_with(pad, 1.0)
}

fn hit_with(pad: usize, velocity: f32) -> EventKind {
    let note = u32::from(FIRST_NOTE) + pad as u32;
    EventKind::NoteOn {
        note,
        pitch: note as f32,
        velocity,
    }
}

fn release(pad: usize) -> EventKind {
    EventKind::NoteOff {
        note: u32::from(FIRST_NOTE) + pad as u32,
    }
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

/// Upward zero crossings per second.
fn frequency(signal: &[f32]) -> f32 {
    let crossings = signal
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    crossings as f32 * RATE / signal.len() as f32
}

#[test]
fn pads_play_on_notes_36_to_51_bottom_left_first() {
    let samples: Vec<_> = (0..PADS)
        .map(|pad| (pad, sine(RATE, 0.5, 200.0 + 100.0 * pad as f32, 0.5)))
        .collect();
    for pad in [0, 5, 15] {
        let mut device = kit(&samples, &[]);
        let (left, _) = render(&mut *device, 0.4, &[(0.0, hit(pad))]);
        let hz = frequency(span(&left, 0.05, 0.35));
        assert!(
            (hz - (200.0 + 100.0 * pad as f32)).abs() < 5.0,
            "pad {pad}: {hz}"
        );
    }
    // Notes outside the pads are silent.
    let mut device = kit(&samples, &[]);
    let below = EventKind::NoteOn {
        note: 1,
        pitch: 35.0,
        velocity: 1.0,
    };
    let above = EventKind::NoteOn {
        note: 2,
        pitch: 52.0,
        velocity: 1.0,
    };
    let (left, right) = render(&mut *device, 0.2, &[(0.0, below), (0.0, above)]);
    assert!(left.iter().chain(&right).all(|x| *x == 0.0));
    assert_eq!(pad_of_note(36.0), Some(0));
    assert_eq!(pad_of_note(51.0), Some(15));
    assert_eq!(id(0, START), 32);
    assert_eq!(id(15, MUTE), 32 * 16 + 18);
    assert_eq!(pad_of(id(7, CHOKE)), Some((7, CHOKE)));
    assert_eq!(pad_of(VOLUME), None);
}

#[test]
fn a_default_pad_plays_its_recording_as_it_is_and_whole() {
    let sample = sine(RATE, 0.3, 440.0, 0.5);
    let mut device = kit(&[(2, Arc::clone(&sample))], &[]);
    let (left, right) = render(&mut *device, 0.5, &[(0.01, hit(2))]);
    // Same rate, no pitch: the recording itself, a few frames in.
    let start = at(0.01);
    let reference = sample.channel(0).samples();
    let error = (1000..10_000)
        .map(|n| (left[start + n] - reference[n]).abs())
        .fold(0.0_f32, f32::max);
    assert!(error < 1e-3, "{error}");
    assert_eq!(left, right, "centred");
    // It ends with the recording, without a click.
    assert!(rms(span(&left, 0.32, 0.5)) < 1e-6);
    assert!(largest_step(span(&left, 0.25, 0.33)) < 0.05);
}

#[test]
fn pitch_fine_and_the_pitch_envelope_transpose_a_hit() {
    let sample = sine(44_100.0, 1.0, 300.0, 0.5);
    let pitched = |values: &[(u32, f32)]| {
        let mut device = kit(&[(0, Arc::clone(&sample))], values);
        render(&mut *device, 0.5, &[(0.0, hit(0))]).0
    };
    // The recording's own rate is played back at the engine's.
    let hz = frequency(span(&pitched(&[]), 0.05, 0.45));
    assert!((hz - 300.0).abs() < 4.0, "{hz}");
    let hz = frequency(span(&pitched(&[(id(0, PITCH), 12.0)]), 0.05, 0.45));
    assert!((hz - 600.0).abs() < 6.0, "{hz}");
    let hz = frequency(span(
        &pitched(&[(id(0, PITCH), -12.0), (id(0, FINE), 50.0)]),
        0.05,
        0.45,
    ));
    let expected = 300.0 * 2f32.powf(-11.5 / 12.0);
    assert!((hz - expected).abs() < 4.0, "{hz} {expected}");
    // An envelope of +24 semitones over 50 ms starts two octaves up and
    // settles on the pitch.
    let swept = pitched(&[(id(0, PITCH_ENV), 24.0), (id(0, ENV_DECAY), 50.0)]);
    let early = frequency(span(&swept, 0.0, 0.006));
    assert!(early > 750.0, "{early}");
    let late = frequency(span(&swept, 0.15, 0.45));
    assert!((late - 300.0).abs() < 5.0, "{late}");
}

#[test]
fn a_choke_group_cuts_the_other_pads_of_the_group_in_five_ms() {
    let long = sine(RATE, 2.0, 500.0, 0.5);
    let samples = [(6, Arc::clone(&long)), (10, Arc::clone(&long))];
    let choked = [(id(6, CHOKE), 1.0), (id(10, CHOKE), 1.0)];
    // The open hat rings until the closed one cuts it.
    let mut device = kit(&samples, &choked);
    let (left, _) = render(&mut *device, 1.0, &[(0.0, hit(10)), (0.5, hit(6))]);
    let mut alone = kit(&samples[..1], &[]);
    let (closed, _) = render(&mut *alone, 1.0, &[(0.5, hit(6))]);
    let rest: Vec<f32> = left.iter().zip(&closed).map(|(a, b)| a - b).collect();
    assert!(rms(span(&rest, 0.4, 0.49)) > 0.3, "it rang");
    assert!(rms(span(&rest, 0.506, 1.0)) < 1e-6, "and stopped");
    assert!(
        largest_step(span(&left, 0.49, 0.52)) < 0.08,
        "without a click"
    );
    // In different groups, or none, both sound on.
    for values in [vec![], vec![(id(6, CHOKE), 1.0), (id(10, CHOKE), 2.0)]] {
        let mut device = kit(&samples, &values);
        let (left, _) = render(&mut *device, 1.0, &[(0.0, hit(10)), (0.5, hit(6))]);
        let rest: Vec<f32> = left.iter().zip(&closed).map(|(a, b)| a - b).collect();
        assert!(rms(span(&rest, 0.6, 1.0)) > 0.3, "{values:?}");
    }
}

#[test]
fn hits_of_one_pad_overlap_unless_it_gates_or_chokes() {
    let long = sine(RATE, 2.0, 500.0, 0.25);
    let hits = [(0.0, hit(0)), (0.1, hit(0))];
    // The second hit is in phase with the first only by chance; compare
    // levels: two overlapping hits are louder than one.
    let level = |values: &[(u32, f32)]| {
        let mut device = kit(&[(0, Arc::clone(&long))], values);
        let (left, _) = render(&mut *device, 0.5, &hits);
        rms(span(&left, 0.2, 0.45))
    };
    let mut single = kit(&[(0, Arc::clone(&long))], &[]);
    let one = rms(span(&render(&mut *single, 0.5, &hits[..1]).0, 0.2, 0.45));
    assert!(level(&[]) > one * 1.2, "a roll rings on");
    for cut in [vec![(id(0, TRIGGER), 1.0)], vec![(id(0, CHOKE), 3.0)]] {
        assert!((level(&cut) - one).abs() < one * 0.05, "{cut:?}");
    }
    // A fifth hit fades the oldest: never more than four at once.
    let mut device = kit(&[(0, Arc::clone(&long))], &[]);
    let roll: Vec<_> = (0..8).map(|n| (n as f32 * 0.02, hit(0))).collect();
    let (left, _) = render(&mut *device, 0.4, &roll);
    let peak = span(&left, 0.2, 0.4)
        .iter()
        .fold(0.0_f32, |p, x| p.max(x.abs()));
    assert!(peak <= 4.0 * 0.25 + 1e-3, "{peak}");
}

#[test]
fn gate_releases_over_decay_and_a_one_shot_ignores_the_release() {
    let long = sine(RATE, 2.0, 400.0, 0.5);
    let events = [(0.0, hit(0)), (0.2, release(0))];
    let mut one_shot = kit(&[(0, Arc::clone(&long))], &[]);
    let (left, _) = render(&mut *one_shot, 1.0, &events);
    assert!(rms(span(&left, 0.6, 0.9)) > 0.3);
    let mut gate = kit(
        &[(0, Arc::clone(&long))],
        &[(id(0, TRIGGER), 1.0), (id(0, DECAY), 100.0)],
    );
    let (left, _) = render(&mut *gate, 1.0, &events);
    assert!(rms(span(&left, 0.1, 0.19)) > 0.3, "held");
    assert!(db(rms(span(&left, 0.31, 0.4))) < -40.0, "released");
}

#[test]
fn hold_and_decay_shape_a_hit_as_its_drawing_says() {
    let long = sine(RATE, 2.0, 400.0, 0.5);
    let values = [
        (id(0, ATTACK), 10.0),
        (id(0, HOLD), 100.0),
        (id(0, DECAY), 200.0),
    ];
    let mut device = kit(&[(0, long)], &values);
    let (left, _) = render(&mut *device, 1.0, &[(0.0, hit(0))]);
    let settings = drums::Settings::from_normalized(support::value_of(&drums::DESCRIPTOR, &values));
    let pad = settings.pads[0];
    assert_eq!(pad.hold_ms, Some(100.0));
    for seconds in [0.005, 0.05, 0.16, 0.25] {
        let level =
            rms(span(&left, seconds - 0.0025, seconds + 0.0025)) as f32 / (0.5 / 2f32.sqrt());
        let drawn = pad.envelope(seconds);
        assert!(
            (level - drawn).abs() < 0.08,
            "{seconds}: {level} played, {drawn} drawn"
        );
    }
    assert!(db(rms(span(&left, 0.4, 1.0))) < -50.0);
    // Hold at its top holds as long as the recording.
    let full = drums::Settings::from_normalized(support::value_of(&drums::DESCRIPTOR, &[]));
    assert_eq!(full.pads[0].hold_ms, None);
    assert_eq!(full.pads[0].envelope(30.0), 1.0);
}

#[test]
fn the_filter_cuts_lows_or_highs_and_nothing_at_its_centre() {
    let level = |filter: f32, hz: f32| {
        let sample = sine(RATE, 0.5, hz, 0.5);
        let mut device = kit(&[(0, sample)], &[(id(0, FILTER), filter)]);
        let (left, _) = render(&mut *device, 0.4, &[(0.0, hit(0))]);
        db(rms(span(&left, 0.1, 0.35)) / (0.5 / 2f64.sqrt()))
    };
    for hz in [80.0, 8000.0] {
        assert!(level(0.0, hz).abs() < 0.05, "{hz}");
    }
    assert!(level(-60.0, 8000.0) < -24.0 && level(-60.0, 80.0) > -1.0);
    assert!(level(60.0, 80.0) < -24.0 && level(60.0, 8000.0) > -1.0);
}

#[test]
fn drive_adds_harmonics_and_keeps_the_level_in_check() {
    let sample = sine(RATE, 0.5, 200.0, 0.9);
    let play = |drive: f32| {
        let mut device = kit(&[(0, Arc::clone(&sample))], &[(id(0, DRIVE), drive)]);
        render(&mut *device, 0.4, &[(0.0, hit(0))]).0
    };
    let clean = play(0.0);
    let driven = play(24.0);
    let peak = |signal: &[f32]| signal.iter().fold(0.0_f32, |p, x| p.max(x.abs()));
    let (clean, driven) = (span(&clean, 0.1, 0.35), span(&driven, 0.1, 0.35));
    // Denser: a lower crest factor at a peak no higher than before.
    let crest = |signal: &[f32]| f64::from(peak(signal)) / rms(signal);
    assert!(
        crest(driven) < crest(clean) * 0.85,
        "{} {}",
        crest(driven),
        crest(clean)
    );
    assert!(peak(driven) <= peak(clean));
    assert!(driven.iter().all(|x| x.is_finite()));
}

#[test]
fn velocity_pan_volume_and_mute_set_the_level_of_a_hit() {
    let sample = sine(RATE, 0.5, 300.0, 0.5);
    let play = |values: &[(u32, f32)], velocity: f32| {
        let mut device = kit(&[(0, Arc::clone(&sample))], values);
        let (left, right) = render(&mut *device, 0.4, &[(0.0, hit_with(0, velocity))]);
        (
            rms(span(&left, 0.1, 0.35)) / (0.5 / 2f64.sqrt()),
            rms(span(&right, 0.1, 0.35)) / (0.5 / 2f64.sqrt()),
        )
    };
    // 80 %: a half-strength hit plays at 1 − 0.8 + 0.8 · 0.25.
    let (soft, _) = play(&[(id(0, VELOCITY), 80.0)], 0.5);
    assert!((soft - 0.4).abs() < 0.01, "{soft}");
    let (hard, _) = play(&[(id(0, VELOCITY), 80.0)], 1.0);
    assert!((hard - 1.0).abs() < 0.01, "{hard}");
    let (left, right) = play(&[(id(0, PAN), -100.0)], 1.0);
    assert!((left - 1.0).abs() < 0.01 && right < 1e-6);
    let (left, _) = play(&[(id(0, LEVEL), -6.0)], 1.0);
    assert!((db(left) + 6.0).abs() < 0.1);
    let (left, _) = play(&[(id(0, LEVEL), LEVEL_OFF_DB)], 1.0);
    assert_eq!(left, 0.0);
    let (left, right) = play(&[(id(0, MUTE), 1.0)], 1.0);
    assert_eq!((left, right), (0.0, 0.0));
}

#[test]
fn start_end_and_reverse_choose_what_of_the_recording_plays() {
    let sample = ramp(1.0);
    let play = |values: &[(u32, f32)]| {
        let mut device = kit(&[(0, Arc::clone(&sample))], values);
        render(&mut *device, 1.2, &[(0.0, hit(0))]).0
    };
    let played = play(&[(id(0, START), 25.0), (id(0, END), 75.0)]);
    assert!(
        (played[at(0.01)] - 0.26).abs() < 0.01,
        "{}",
        played[at(0.01)]
    );
    assert!(rms(span(&played, 0.51, 1.2)) < 1e-6, "ends at 75 %");
    let reversed = play(&[(id(0, REVERSE), 1.0)]);
    assert!(
        (reversed[at(0.1)] - 0.9).abs() < 0.01,
        "{}",
        reversed[at(0.1)]
    );
}

#[test]
fn the_panel_sees_each_pads_activity_and_playhead() {
    let mut device = kit(&[(3, ramp(1.0))], &[]);
    let telemetry = device.telemetry();
    render(&mut *device, 0.25, &[(0.0, hit(3))]);
    assert!(telemetry.value(TELEMETRY_ACTIVITY + 3) > 0.9);
    assert_eq!(telemetry.value(TELEMETRY_ACTIVITY + 2), 0.0);
    let at = telemetry.value(TELEMETRY_PLAYHEADS + 3);
    assert!((at - 0.25).abs() < 0.01, "{at}");
    assert_eq!(telemetry.value(TELEMETRY_PLAYHEADS + 4), -1.0);
    assert_eq!(telemetry.value(TELEMETRY_VOICES), 1.0);
}

#[test]
fn a_roll_moves_the_sound_of_every_pad_but_not_the_kit() {
    let defaults = |id: u32| {
        drums::DESCRIPTOR
            .parameter(id)
            .map_or(0.0, |spec| spec.default_normalized())
    };
    let rolled = daw_devices::randomized(&drums::DESCRIPTOR, defaults, 0.1, 11);
    let moved = |offset: u32| {
        rolled
            .iter()
            .filter(|(id, _)| pad_of(*id).is_some_and(|(_, o)| o == offset))
            .count()
    };
    for offset in [START, END, REVERSE, MUTE, CHOKE, LEVEL] {
        assert_eq!(moved(offset), 0, "offset {offset}");
    }
    assert!(rolled.iter().all(|(id, _)| *id != VOLUME));
    // Tune, filter and the envelope move on most pads.
    for offset in [FINE, FILTER, DECAY, PAN] {
        assert!(moved(offset) >= 12, "offset {offset}: {}", moved(offset));
    }
}

#[test]
fn a_ringing_filter_drains_instead_of_cutting_off() {
    let sample = sine(RATE, 0.02, 60.0, 0.8);
    let mut device = kit(
        &[(0, sample)],
        &[(id(0, FILTER), -100.0), (id(0, RESONANCE), 100.0)],
    );
    let (left, _) = render(&mut *device, 2.0, &[(0.0, hit(0))]);
    let rings = left[at(0.15)..].iter().any(|x| x.abs() > 1e-4);
    assert!(
        rings,
        "the filter still rings after the tail's first 100 ms"
    );
    let cut = left
        .windows(2)
        .position(|pair| pair[0].abs() > 1e-4 && pair[1] == 0.0);
    assert_eq!(cut, None, "the ring goes silent, never stops dead");
}
