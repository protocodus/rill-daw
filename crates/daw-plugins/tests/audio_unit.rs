//! Apple's system Audio Units across the isolation helper. Every Mac ships these
//! components, so the suite needs no fixture and runs on a headless runner.
#![cfg(target_os = "macos")]

use daw_plugins::audio_unit::AU_LOCATOR;
use daw_plugins::{MidiMessage, PluginFormat, PluginInfo, VstEffect, VstInstrument};
use std::path::Path;

const DELAY: &str = "aufx:dely:appl";
const LOWPASS: &str = "aufx:lpas:appl";
const LIMITER: &str = "aufx:lmtr:appl";
const DLS: &str = "aumu:dls :appl";
const RATE: f64 = 48_000.0;
const BLOCK: usize = 512;

fn locator() -> &'static Path {
    Path::new(AU_LOCATOR)
}

fn effect(id: &str) -> VstEffect {
    VstEffect::load_class(locator(), Some(id), RATE, BLOCK).unwrap()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
}

fn sine(frequency: f64, start: usize, frames: usize) -> Vec<f32> {
    (start..start + frames)
        .map(|n| (0.5 * (std::f64::consts::TAU * frequency * n as f64 / RATE).sin()) as f32)
        .collect()
}

/// Feed `blocks` blocks of `input(start)` and return the last block's left output.
fn run(effect: &mut VstEffect, blocks: usize, input: impl Fn(usize) -> Vec<f32>) -> Vec<f32> {
    let (mut left, mut right) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    for block in 0..blocks {
        let dry = input(block * BLOCK);
        effect.process(&dry, &dry, &mut left, &mut right).unwrap();
        assert!(left.iter().chain(&right).all(|s| s.is_finite()));
    }
    left
}

fn distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| (a - b).abs()).sum()
}

#[test]
fn scan_lists_apple_effects_and_instruments_from_the_component_registry() {
    let found = daw_plugins::scan_paths(&[locator().to_owned()]).unwrap();
    for id in [DELAY, LOWPASS, LIMITER, DLS] {
        let plugin = found
            .iter()
            .find(|p| p.class_id.as_deref() == Some(id))
            .unwrap_or_else(|| panic!("{id} missing from {found:?}"));
        assert_eq!(plugin.format, PluginFormat::Au);
        assert_eq!(plugin.path, locator());
        assert_eq!(plugin.vendor, "Apple", "{plugin:?}");
        assert!(!plugin.name.is_empty());
    }
    let dls = found.iter().find(|p| p.class_id.as_deref() == Some(DLS));
    assert_eq!(dls.unwrap().is_instrument(), Some(true));
    let delay = found.iter().find(|p| p.class_id.as_deref() == Some(DELAY));
    assert_eq!(delay.unwrap().is_effect(), Some(true));
    assert!(found.iter().all(|p| p.format == PluginFormat::Au));
    let mut ids: Vec<_> = found.iter().map(|p| p.class_id.clone()).collect();
    ids.dedup();
    assert_eq!(ids.len(), found.len(), "class IDs are unique");
}

#[test]
fn delay_processes_in_isolation_and_its_mix_parameter_is_automatable() {
    let mut delay = effect(DELAY);
    assert_ne!(delay.isolation_pid(), Some(std::process::id()));
    assert_eq!(delay.info().format, PluginFormat::Au);
    let input = |start| sine(440.0, start, BLOCK);
    let wet = run(&mut delay, 8, input);
    let dry = input(7 * BLOCK);
    assert!(peak(&wet) > 0.01, "delay produced silence");
    let wet_distance = distance(&wet, &dry);
    assert!(wet_distance > 1.0, "delay left the signal unchanged");

    let parameters = delay.parameters().unwrap();
    let mix = parameters.iter().find(|p| p.id == 0).unwrap();
    assert!(!mix.read_only && !mix.name.is_empty(), "{mix:?}");
    delay.set_parameter(0, 0.0).unwrap();
    let value = delay.parameters().unwrap();
    assert!(value.iter().find(|p| p.id == 0).unwrap().value < 1e-6);
    let dry_out = run(&mut delay, 8, input);
    assert!(
        distance(&dry_out, &dry) < wet_distance / 4.0,
        "a fully dry mix should pass the input"
    );

    delay.set_parameter(0, 1.0).unwrap();
    run(&mut delay, 4, input);
    delay.queue_parameter_for_block(0, 0.0).unwrap();
    let automated = run(&mut delay, 1, |_| dry.clone());
    assert!(distance(&automated, &dry) < wet_distance / 4.0);
}

#[test]
fn lowpass_attenuates_high_frequencies_and_passes_low_ones() {
    let mut lowpass = effect(LOWPASS);
    // Cutoff is parameter 0; a low setting leaves no doubt at 18 kHz.
    lowpass.set_parameter(0, 0.05).unwrap();
    let high = run(&mut lowpass, 8, |start| sine(18_000.0, start, BLOCK));
    let low = run(&mut lowpass, 8, |start| sine(200.0, start, BLOCK));
    assert!(peak(&low) > 0.3, "200 Hz passes: {}", peak(&low));
    assert!(peak(&high) < 0.05, "18 kHz is cut: {}", peak(&high));
}

#[test]
fn limiter_reports_latency_and_tail_and_processes() {
    let mut limiter = effect(LIMITER);
    // Values come from the component; they only need to be sane.
    assert!(limiter.latency_samples() < RATE as u32);
    assert!(limiter.tail_samples() < 60 * RATE as u32);
    let out = run(&mut limiter, 8, |start| sine(440.0, start, BLOCK));
    assert!(peak(&out) > 0.01 && peak(&out) <= 1.0);
}

#[test]
fn dls_synth_is_audible_after_a_note() {
    let mut synth = VstInstrument::load_class(locator(), Some(DLS), RATE, BLOCK).unwrap();
    assert_eq!(synth.info().format, PluginFormat::Au);
    let (mut left, mut right) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    synth.process(&[], &mut left, &mut right).unwrap();
    assert_eq!(peak(&left), 0.0);
    let on = MidiMessage {
        offset: 0,
        pitch: 60,
        velocity: 127,
        on: true,
    };
    let mut loudest = 0.0f32;
    for block in 0..16 {
        let events = if block == 0 { vec![on] } else { vec![] };
        synth.process(&events, &mut left, &mut right).unwrap();
        loudest = loudest.max(peak(&left)).max(peak(&right));
    }
    assert!(loudest > 0.001, "note was not rendered: {loudest}");
}

#[test]
fn state_round_trips_as_a_binary_property_list() {
    let mut delay = effect(DELAY);
    delay.set_parameter(0, 0.25).unwrap();
    let state = delay.save_state().unwrap();
    assert!(state.starts_with(b"bplist00"), "{:?}", &state[..8]);
    delay.set_parameter(0, 1.0).unwrap();
    delay.load_state(&state).unwrap();
    let restored = delay.parameters().unwrap();
    let mix = restored.iter().find(|p| p.id == 0).unwrap().value;
    assert!((mix - 0.25).abs() < 1e-3, "{mix}");
    assert!(delay.load_state(b"<?xml version").is_err());
}

#[test]
fn a_killed_helper_is_contained_and_recoverable() {
    let mut delay = effect(DELAY);
    let state = delay.save_state().unwrap();
    let pid = delay.isolation_pid().unwrap();
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let input = sine(440.0, 0, BLOCK);
    let (mut left, mut right) = (vec![0.0; BLOCK], vec![0.0; BLOCK]);
    let mut failed = false;
    for _ in 0..20 {
        if delay
            .process(&input, &input, &mut left, &mut right)
            .is_err()
        {
            failed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(failed, "a dead helper must surface an error");
    delay.recover().unwrap();
    assert_ne!(delay.isolation_pid(), Some(pid));
    delay.load_state(&state).unwrap();
    run(&mut delay, 4, |start| sine(440.0, start, BLOCK));
}

#[test]
fn inspection_reads_buses_and_parameters_from_the_helper() {
    let info = PluginInfo {
        path: locator().to_owned(),
        class_id: Some(DELAY.into()),
        format: PluginFormat::Au,
        ..PluginInfo::default()
    };
    let inspection = daw_plugins::inspect_plugin(&info).unwrap();
    assert!(!inspection.midi_input);
    assert_eq!(inspection.inputs.first().map(|b| b.main), Some(true));
    assert_eq!(inspection.outputs.first().map(|b| b.channels), Some(2));
    assert!(inspection.parameters.iter().any(|p| p.id == 0));
}

#[test]
fn unknown_components_and_effects_as_instruments_fail_cleanly() {
    let error = VstEffect::load_class(locator(), Some("aufx:none:rill"), RATE, BLOCK)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("aufx:none:rill"), "{error:#}");
    let as_instrument = VstInstrument::load_class(locator(), Some(DELAY), RATE, BLOCK);
    assert!(as_instrument.is_err(), "effects accept no MIDI");
}
