use daw_devices::devices::mini::{self, *};
use daw_devices::{Device, Event, EventKind, ProcessContext, StereoIo};

const RATE: f32 = 48_000.0;
const BLOCK: usize = 256;

fn synth(values: &[(u32, f32)]) -> Box<dyn Device> {
    let mut device = (mini::DESCRIPTOR.create)();
    device.prepare(RATE, BLOCK);
    for &(id, plain) in values {
        let spec = mini::DESCRIPTOR.parameter(id).unwrap();
        device.set_parameter(id, spec.to_normalized(plain));
    }
    device.reset();
    device
}

fn note_on(offset: u32, note: u32, pitch: f32, velocity: f32) -> Event {
    Event {
        offset,
        kind: EventKind::NoteOn {
            note,
            pitch,
            velocity,
        },
    }
}

fn note_off(offset: u32, note: u32) -> Event {
    Event {
        offset,
        kind: EventKind::NoteOff { note },
    }
}

/// Renders `seconds`, delivering each `(frame, event)` at its frame. Without
/// unison spread both sides are the same.
fn render(device: &mut dyn Device, seconds: f32, events: &[(usize, EventKind)]) -> Vec<f32> {
    let (left, right) = render_stereo(device, seconds, events);
    assert_eq!(left, right, "Mini without spread is mono");
    left
}

fn render_stereo(
    device: &mut dyn Device,
    seconds: f32,
    events: &[(usize, EventKind)],
) -> (Vec<f32>, Vec<f32>) {
    let total = (seconds * RATE) as usize;
    let mut output = (Vec::with_capacity(total), Vec::with_capacity(total));
    let mut left = vec![0.0; BLOCK];
    let mut right = vec![0.0; BLOCK];
    for start in (0..total).step_by(BLOCK) {
        let block: Vec<Event> = events
            .iter()
            .filter(|(frame, _)| (start..start + BLOCK).contains(frame))
            .map(|(frame, kind)| Event {
                offset: (frame - start) as u32,
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
        output.0.extend_from_slice(&left);
        output.1.extend_from_slice(&right);
    }
    output.0.truncate(total);
    output.1.truncate(total);
    output
}

fn hold(pitch: f32) -> Vec<(usize, EventKind)> {
    vec![(0, note_on(0, 1, pitch, 1.0).kind)]
}

/// Frequency from rising zero crossings, with linear interpolation.
fn frequency(samples: &[f32]) -> f32 {
    let crossings: Vec<f32> = samples
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] < 0.0 && pair[1] >= 0.0)
        .map(|(index, pair)| index as f32 + pair[0] / (pair[0] - pair[1]))
        .collect();
    let (first, last) = (crossings[0], *crossings.last().unwrap());
    (crossings.len() - 1) as f32 * RATE / (last - first)
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
}

/// High-band share of the energy, from first differences.
fn brightness(samples: &[f32]) -> f32 {
    let high: f32 = samples.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    high / samples.iter().map(|x| x * x).sum::<f32>().max(1e-12)
}

/// A pure tone: one DCO sine, filter open, no drift.
fn sine_values() -> Vec<(u32, f32)> {
    vec![
        (OSC1_MODEL, 1.0),
        (OSC1_WAVE, 3.0),
        (MIX_OSC2, 0.0),
        (CUTOFF, 20_000.0),
        (FILTER_ENV, 0.0),
        (KEY_TRACKING, 0.0),
        (VELOCITY_CUTOFF, 0.0),
        (DRIFT, 0.0),
    ]
}

#[test]
fn a_held_note_sounds_at_its_pitch_and_the_release_ends_in_silence() {
    let mut device = synth(&sine_values());
    let output = render(
        &mut *device,
        2.0,
        &[
            (0, note_on(0, 1, 69.0, 1.0).kind),
            (RATE as usize, note_off(0, 1).kind),
        ],
    );
    let held = &output[RATE as usize / 4..RATE as usize];
    let measured = frequency(held);
    assert!((measured - 440.0).abs() < 0.2, "{measured}");
    assert!(rms(held) > 0.05);
    // Release 250 ms: silent well before the end, exactly.
    assert!(output[(1.8 * RATE) as usize..].iter().all(|x| *x == 0.0));
}

#[test]
fn fractional_pitch_and_pitch_bend_tune_the_voice() {
    let mut device = synth(&sine_values());
    let output = render(&mut *device, 0.5, &hold(69.5));
    let quarter_tone = 440.0 * 2.0_f32.powf(0.5 / 12.0);
    assert!((frequency(&output[4800..]) - quarter_tone).abs() < 0.3);

    let mut device = synth(&[sine_values(), vec![(BEND_RANGE, 7.0)]].concat());
    let mut events = hold(69.0);
    events.push((0, EventKind::PitchBend(1.0)));
    let output = render(&mut *device, 0.5, &events);
    let fifth = 440.0 * 2.0_f32.powf(7.0 / 12.0);
    assert!((frequency(&output[9600..]) - fifth).abs() < 0.5);
}

#[test]
fn oscillator_two_transposes_and_hard_syncs_to_oscillator_one() {
    let values = [
        sine_values(),
        vec![
            (MIX_OSC1, 0.0),
            (MIX_OSC2, 100.0),
            (OSC2_MODEL, 1.0),
            (OSC2_WAVE, 3.0),
            (OSC2_OCTAVE, 1.0),
            (OSC2_SEMITONES, -5.0),
            (OSC2_DETUNE, 0.0),
        ],
    ]
    .concat();
    let output = render(&mut *synth(&values), 0.5, &hold(57.0));
    // A3 an octave up and a fourth down: E4.
    assert!((frequency(&output[4800..]) - 329.63).abs() < 0.3);

    // Synced at 2.67 times the master's pitch, the slave repeats at the master's period.
    let synced = [
        values.clone(),
        vec![
            (OSC2_OCTAVE, 1.0),
            (OSC2_SEMITONES, 5.0),
            (OSC2_WAVE, 0.0),
            (OSC2_SYNC, 1.0),
        ],
    ]
    .concat();
    let output = render(&mut *synth(&synced), 0.5, &hold(57.0));
    let period = RATE / 220.0;
    let shift = period.round() as usize;
    let settled = &output[9600..20_000];
    let correlation: f32 = settled
        .iter()
        .zip(&output[9600 + shift..])
        .map(|(a, b)| a * b)
        .sum::<f32>()
        / settled.iter().map(|x| x * x).sum::<f32>();
    assert!(correlation > 0.8, "{correlation}");
}

#[test]
fn cutoff_envelope_and_velocity_shape_the_tone() {
    let saw = |cutoff: f32, velocity: f32| {
        let mut device = synth(&[
            (CUTOFF, cutoff),
            (FILTER_ENV, 0.0),
            (MIX_OSC2, 0.0),
            (VELOCITY_CUTOFF, 2.0),
        ]);
        let output = render(
            &mut *device,
            0.5,
            &[(0, note_on(0, 1, 48.0, velocity).kind)],
        );
        output[12_000..].to_vec()
    };
    let bright = saw(8000.0, 1.0);
    let dark = saw(400.0, 1.0);
    assert!(brightness(&dark) < brightness(&bright) * 0.2);
    // Softer notes are darker and quieter.
    let soft = saw(8000.0, 0.3);
    assert!(brightness(&soft) < brightness(&bright) * 0.7);
    assert!(rms(&soft) < rms(&bright) * 0.8);
}

#[test]
fn the_mod_wheel_scales_the_vibrato() {
    let values = [sine_values(), vec![(LFO_PITCH, 1.0), (LFO_RATE, 6.0)]].concat();
    let spread = |wheel: f32| {
        let mut device = synth(&values);
        let mut events = hold(69.0);
        events.push((0, EventKind::ModWheel(wheel)));
        let output = render(&mut *device, 1.0, &events);
        // Pitch over short windows.
        let pitches: Vec<f32> = output[4800..].chunks(1200).map(frequency).collect();
        pitches.iter().fold(0.0_f32, |m, p| m.max(*p))
            - pitches.iter().fold(1e9_f32, |m, p| m.min(*p))
    };
    assert!(spread(0.0) < 0.5, "{}", spread(0.0));
    assert!(spread(1.0) > 20.0, "{}", spread(1.0));
}

#[test]
fn legato_glides_between_overlapping_notes_and_mono_holds_one_voice() {
    let values = [sine_values(), vec![(VOICE_MODE, 2.0), (GLIDE, 200.0)]].concat();
    let mut device = synth(&values);
    let half = RATE as usize / 2;
    let output = render(
        &mut *device,
        1.5,
        &[
            (0, note_on(0, 1, 57.0, 1.0).kind),
            (half, note_on(0, 2, 69.0, 1.0).kind),
        ],
    );
    let before = frequency(&output[half - 4800..half]);
    let during = frequency(&output[half + 960..half + 3360]);
    let after = frequency(&output[half + 24_000..half + 33_600]);
    assert!((before - 220.0).abs() < 0.5, "{before}");
    assert!(during > 240.0 && during < 400.0, "{during}");
    assert!((after - 440.0).abs() < 1.0, "{after}");
}

#[test]
fn voices_are_limited_and_stealing_does_not_click() {
    let values = [sine_values(), vec![(VOICES, 4.0), (AENV_RELEASE, 2000.0)]].concat();
    let mut device = synth(&values);
    // Six short notes, each released before the next: the fifth and sixth
    // steal releasing voices.
    let mut events = Vec::new();
    for index in 0..6 {
        let frame = index * 4800;
        events.push((
            frame,
            note_on(0, index as u32, 60.0 + 2.0 * index as f32, 0.8).kind,
        ));
        events.push((frame + 2400, note_off(0, index as u32).kind));
    }
    let output = render(&mut *device, 1.0, &events);
    let step = output
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0_f32, f32::max);
    // A middle-C sine at this level moves at most about 0.02 per sample.
    assert!(step < 0.05, "{step}");
}

#[test]
fn vco_drift_detunes_stacked_voices_and_dco_stays_exact() {
    let unison = |model: f32| {
        let mut device = synth(&[
            (OSC1_MODEL, model),
            (OSC1_WAVE, 3.0),
            (MIX_OSC2, 0.0),
            (CUTOFF, 20_000.0),
            (FILTER_ENV, 0.0),
            (DRIFT, 100.0),
        ]);
        // The same pitch on two keys: two voices.
        let output = render(
            &mut *device,
            2.0,
            &[
                (0, note_on(0, 1, 69.0, 1.0).kind),
                (0, note_on(0, 2, 69.0, 1.0).kind),
            ],
        );
        // Detuned voices beat, so the level of their sum swings.
        let levels: Vec<f32> = output[9600..].chunks(2400).map(rms).collect();
        levels.iter().fold(0.0_f32, |m, x| m.max(*x))
            / levels.iter().fold(1e9_f32, |m, x| m.min(*x))
    };
    assert!(unison(0.0) > 1.2, "VCO: {}", unison(0.0));
    assert!(unison(1.0) < 1.02, "DCO: {}", unison(1.0));
}

#[test]
fn sustain_pedal_holds_released_notes() {
    let mut device = synth(&sine_values());
    let output = render(
        &mut *device,
        2.0,
        &[
            (0, EventKind::Sustain(true)),
            (0, note_on(0, 1, 60.0, 1.0).kind),
            (4800, note_off(0, 1).kind),
            (RATE as usize, EventKind::Sustain(false)),
        ],
    );
    assert!(rms(&output[30_000..40_000]) > 0.05);
    assert!(output[(1.8 * RATE) as usize..].iter().all(|x| *x == 0.0));
}

#[test]
fn sustain_pedal_holds_a_note_released_while_it_steals_a_voice() {
    let values = [sine_values(), vec![(VOICES, 1.0)]].concat();
    let mut device = synth(&values);
    let output = render(
        &mut *device,
        1.0,
        &[
            (0, EventKind::Sustain(true)),
            (0, note_on(0, 1, 60.0, 1.0).kind),
            (4800, note_off(0, 1).kind),
            // Released within the 5 ms fade of the stolen voice.
            (9600, note_on(0, 2, 64.0, 1.0).kind),
            (9700, note_off(0, 2).kind),
        ],
    );
    assert!(rms(&output[30_000..40_000]) > 0.05);
}

#[test]
fn lowering_the_voice_count_releases_the_voices_beyond_it() {
    let values = [sine_values(), vec![(AENV_RELEASE, 50.0)]].concat();
    let mut device = synth(&values);
    let chord: Vec<_> = (0..8)
        .map(|index| (0, note_on(0, index, 60.0 + index as f32, 1.0).kind))
        .collect();
    render(&mut *device, 0.2, &chord);
    let telemetry = device.telemetry();
    assert_eq!(telemetry.value(TELEMETRY_VOICES), 8.0);
    let spec = mini::DESCRIPTOR.parameter(VOICES).unwrap();
    device.set_parameter(VOICES, spec.to_normalized(1.0));
    render(&mut *device, 1.0, &[]);
    assert_eq!(telemetry.value(TELEMETRY_VOICES), 1.0);
}

#[test]
fn a_mode_change_during_a_steal_fade_keeps_the_next_mono_note() {
    let values = [sine_values(), vec![(VOICES, 1.0)]].concat();
    let mut device = synth(&values);
    // The second note steals the only voice and is released at once; the
    // render stops inside its 5 ms fade.
    render(
        &mut *device,
        19.0 * BLOCK as f32 / RATE,
        &[
            (0, note_on(0, 1, 60.0, 1.0).kind),
            (4700, note_on(0, 2, 64.0, 1.0).kind),
            (4750, note_off(0, 2).kind),
        ],
    );
    let spec = mini::DESCRIPTOR.parameter(VOICE_MODE).unwrap();
    device.set_parameter(VOICE_MODE, spec.to_normalized(1.0));
    let output = render(&mut *device, 1.0, &[(0, note_on(0, 3, 67.0, 1.0).kind)]);
    assert!(rms(&output[30_000..40_000]) > 0.05);
}

#[test]
fn mono_falls_back_to_a_held_key_while_a_steal_fade_runs() {
    let values = [sine_values(), vec![(VOICES, 1.0)]].concat();
    let mut device = synth(&values);
    render(
        &mut *device,
        19.0 * BLOCK as f32 / RATE,
        &[
            (0, note_on(0, 1, 60.0, 1.0).kind),
            (4700, note_on(0, 2, 64.0, 1.0).kind),
        ],
    );
    let spec = mini::DESCRIPTOR.parameter(VOICE_MODE).unwrap();
    device.set_parameter(VOICE_MODE, spec.to_normalized(1.0));
    // Two keys during the fade; releasing the newer leaves the older held.
    let output = render(
        &mut *device,
        1.0,
        &[
            (0, note_on(0, 3, 67.0, 1.0).kind),
            (10, note_on(0, 4, 69.0, 1.0).kind),
            (20, note_off(0, 4).kind),
        ],
    );
    let held = &output[30_000..40_000];
    assert!(rms(held) > 0.05);
    let pitch = 440.0 * ((67.0 - 69.0) / 12.0_f32).exp2();
    assert!((frequency(held) - pitch).abs() < 1.0, "{}", frequency(held));
}

#[test]
fn drawn_response_follows_cutoff_and_resonance() {
    let settings = |cutoff: f32, resonance: f32| {
        Settings::from_normalized(|id| {
            let spec = mini::DESCRIPTOR.parameter(id).unwrap();
            match id {
                CUTOFF => spec.to_normalized(cutoff),
                RESONANCE => spec.to_normalized(resonance),
                _ => spec.default_normalized(),
            }
        })
    };
    let frequencies = [100.0, 1000.0, 10_000.0];
    let mut db = [0.0; 3];
    response_db(&settings(1000.0, 0.0), RATE, &frequencies, &mut db);
    assert!(db[0] > -3.0 && db[2] < -30.0, "{db:?}");
    let mut resonant = [0.0; 3];
    response_db(&settings(1000.0, 90.0), RATE, &frequencies, &mut resonant);
    assert!(resonant[1] > db[1] + 6.0, "{resonant:?}");
}

#[test]
fn unison_stacks_voices_on_one_note_and_spread_detunes_them_across_the_stereo_field() {
    let values = [sine_values(), vec![(UNISON, 2.0), (SPREAD, 0.0)]].concat();
    let mut device = synth(&values);
    let output = render(&mut *device, 0.5, &hold(69.0));
    assert!((frequency(&output[4800..]) - 440.0).abs() < 0.3);
    assert_eq!(device.telemetry().value(TELEMETRY_VOICES), 2.0);

    // At full spread the two voices stand at either side, a quarter
    // semitone below and above.
    let values = [sine_values(), vec![(UNISON, 2.0), (SPREAD, 100.0)]].concat();
    let (left, right) = render_stereo(&mut *synth(&values), 0.5, &hold(69.0));
    let below = 440.0 * (-0.25_f32 / 12.0).exp2();
    let above = 440.0 * (0.25_f32 / 12.0).exp2();
    assert!(
        (frequency(&left[4800..]) - below).abs() < 0.3,
        "{}",
        frequency(&left[4800..])
    );
    assert!(
        (frequency(&right[4800..]) - above).abs() < 0.3,
        "{}",
        frequency(&right[4800..])
    );
    // A stack is about as loud as one voice.
    let single = rms(&render(&mut *synth(&sine_values()), 0.5, &hold(69.0))[4800..]);
    let stacked = (rms(&left[4800..]).powi(2) + rms(&right[4800..]).powi(2)).sqrt();
    assert!((stacked / single - 1.0).abs() < 0.1, "{stacked} {single}");
}

#[test]
fn unison_shares_the_voices_between_notes() {
    let values = [
        sine_values(),
        vec![(UNISON, 4.0), (VOICES, 8.0), (SPREAD, 0.0)],
    ]
    .concat();
    let mut device = synth(&values);
    let events: Vec<_> = (0..5)
        .map(|index| {
            (
                index * 480,
                note_on(0, index as u32, 60.0 + index as f32, 1.0).kind,
            )
        })
        .collect();
    render(&mut *device, 0.2, &events);
    // Sixteen voices hold four notes of four; the fifth note stole the first.
    assert_eq!(device.telemetry().value(TELEMETRY_VOICES), 16.0);

    let values = [
        sine_values(),
        vec![(UNISON, 3.0), (VOICE_MODE, 1.0), (SPREAD, 0.0)],
    ]
    .concat();
    let mut device = synth(&values);
    let output = render(
        &mut *device,
        0.6,
        &[
            (0, note_on(0, 1, 57.0, 1.0).kind),
            (9600, note_on(0, 2, 69.0, 1.0).kind),
        ],
    );
    assert_eq!(device.telemetry().value(TELEMETRY_VOICES), 3.0);
    assert!((frequency(&output[19_200..]) - 440.0).abs() < 0.5);
}

#[test]
fn the_filter_type_switches_to_high_pass_where_the_model_has_one() {
    let saw = |model: f32, filter_type: f32| {
        let values = vec![
            (OSC1_MODEL, 1.0),
            (MIX_OSC2, 0.0),
            (FILTER_MODEL, model),
            (FILTER_TYPE, filter_type),
            (CUTOFF, 1000.0),
            (FILTER_ENV, 0.0),
            (KEY_TRACKING, 0.0),
            (DRIFT, 0.0),
        ];
        let output = render(&mut *synth(&values), 0.5, &hold(45.0));
        brightness(&output[4800..])
    };
    // OTA Cascade: high-pass keeps the top and loses the fundamental.
    assert!(
        saw(1.0, 1.0) > 5.0 * saw(1.0, 0.0),
        "{} {}",
        saw(1.0, 1.0),
        saw(1.0, 0.0)
    );
    // The ladder is low-pass only; the type is ignored.
    assert_eq!(saw(0.0, 1.0), saw(0.0, 0.0));
}

#[test]
fn the_lfo_fades_in_on_each_note_and_pressure_opens_it() {
    let swing = |output: &[f32]| {
        let pitches: Vec<f32> = output.chunks(1200).map(frequency).collect();
        pitches.iter().fold(0.0_f32, |m, p| m.max(*p))
            - pitches.iter().fold(1e9_f32, |m, p| m.min(*p))
    };
    let values = [
        sine_values(),
        vec![
            (LFO_PITCH, 1.0),
            (LFO_RATE, 6.0),
            (LFO_WHEEL, 0.0),
            (LFO_FADE, 1500.0),
        ],
    ]
    .concat();
    let output = render(&mut *synth(&values), 2.5, &hold(69.0));
    let early = swing(&output[2400..14_400]);
    let late = swing(&output[96_000..120_000]);
    assert!(early < 0.3 * late, "{early} {late}");

    // With the mod wheel in charge, pressure on the note opens the vibrato.
    let values = [sine_values(), vec![(LFO_PITCH, 1.0), (LFO_RATE, 6.0)]].concat();
    let mut events = hold(69.0);
    events.push((
        100,
        EventKind::Pressure {
            note: 1,
            value: 1.0,
        },
    ));
    let output = render(&mut *synth(&values), 1.0, &events);
    assert!(swing(&output[4800..]) > 20.0, "{}", swing(&output[4800..]));
}

#[test]
fn the_filter_envelope_sweeps_oscillator_two() {
    let values = [
        sine_values(),
        vec![
            (MIX_OSC1, 0.0),
            (MIX_OSC2, 100.0),
            (OSC2_MODEL, 1.0),
            (OSC2_WAVE, 3.0),
            (OSC2_DETUNE, 0.0),
            (FENV_SUSTAIN, 100.0),
            (FENV_PITCH, 12.0),
        ],
    ]
    .concat();
    let output = render(&mut *synth(&values), 0.5, &hold(57.0));
    // Envelope held at full: an octave up.
    assert!((frequency(&output[4800..]) - 440.0).abs() < 0.5);
}
