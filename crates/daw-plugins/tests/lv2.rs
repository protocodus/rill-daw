//! Real native LV2 instruments and effects across the isolation helper (Linux).
#![cfg(target_os = "linux")]
#[path = "common/lv2_probe_fixture.rs"]
mod lv2_probe_fixture;

use daw_plugins::{
    BlockTransport, MidiMessage, PluginFormat, PluginInfo, StereoBusInput, VstEffect, VstInstrument,
};
use lv2_probe_fixture::{GAIN_LATENCY, GAIN_URI, Lv2ProbeFixture, SYNTH_URI};
use std::time::{Duration, Instant};

fn synth(fixture: &Lv2ProbeFixture) -> VstInstrument {
    VstInstrument::load_class(&fixture.path, Some(SYNTH_URI), 48_000.0, 64).unwrap()
}

fn note(offset: u32, pitch: u8, on: bool) -> MidiMessage {
    MidiMessage {
        offset,
        pitch,
        velocity: if on { 127 } else { 0 },
        on,
    }
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
}

#[test]
fn scan_reads_bundle_metadata_in_the_helper() {
    let fixture = Lv2ProbeFixture::build();
    let found = daw_plugins::scan_paths(&[fixture.path.parent().unwrap().to_owned()]).unwrap();
    let ids: Vec<_> = found.iter().map(|p| p.class_id.as_deref()).collect();
    assert_eq!(ids, [Some(GAIN_URI), Some(SYNTH_URI)], "{found:?}");
    assert!(found.iter().all(|p| p.format == PluginFormat::Lv2));
    assert!(found.iter().all(|p| p.vendor == "RILL tests"));
    assert_eq!(found[1].name, "Rill LV2 Synth");
    assert_eq!(found[1].is_instrument(), Some(true));
    assert_eq!(found[0].is_effect(), Some(true));
    assert_eq!(found[0].category, "Fx|Amplifier");
}

#[test]
fn instrument_renders_midi_parameters_worker_responses_and_state() {
    let fixture = Lv2ProbeFixture::build();
    let mut plugin = synth(&fixture);
    assert_ne!(plugin.isolation_pid(), Some(std::process::id()));
    assert_eq!(plugin.info().format, PluginFormat::Lv2);
    assert_eq!(plugin.latency_samples(), 0);
    let (mut left, mut right) = ([0.0; 64], [0.0; 64]);
    plugin
        .process(&[note(10, 69, true)], &mut left, &mut right)
        .unwrap();
    assert_eq!(left[..10], [0.0; 10]);
    let loud = peak(&left);
    assert!(loud > 0.1, "note was not rendered: {loud}");
    assert_eq!(left, right);

    let parameters = plugin.parameters().unwrap();
    let level = parameters.iter().find(|p| p.name == "Level").unwrap();
    assert_eq!((level.value, level.default, level.steps), (0.5, 0.5, 0));
    let mode = parameters.iter().find(|p| p.name == "Mode").unwrap();
    assert_eq!(mode.steps, 3);
    plugin.set_parameter(level.id, 0.25).unwrap();
    plugin.process(&[], &mut left, &mut right).unwrap();
    let quiet = peak(&left);
    assert!((quiet / loud - 0.5).abs() < 0.05, "{quiet} vs {loud}");
    // The waveform switch is applied by the worker's response, after this block.
    plugin.set_parameter(mode.id, 1.0 / 3.0).unwrap();
    plugin.process(&[], &mut left, &mut right).unwrap();
    assert!(left.iter().any(|s| s.abs() < quiet * 0.9), "still a sine");
    plugin.process(&[], &mut left, &mut right).unwrap();
    assert!(
        left.iter().all(|s| (s.abs() - quiet).abs() < 1e-3),
        "now square"
    );
    let state = plugin.save_state().unwrap();
    assert!(
        String::from_utf8_lossy(&state).contains("level"),
        "Turtle state"
    );
    plugin.set_parameter(level.id, 1.0).unwrap();
    plugin.set_parameter(mode.id, 0.0).unwrap();
    plugin.load_state(&state).unwrap();
    let restored = plugin.parameters().unwrap();
    let value = |id| restored.iter().find(|p| p.id == id).unwrap().value;
    assert_eq!((value(level.id), value(mode.id)), (0.25, 1.0 / 3.0));

    // Automation splits the block at its offset.
    plugin.queue_parameter_for_block(level.id, 0.0).unwrap();
    plugin.process(&[], &mut left, &mut right).unwrap();
    assert_eq!(peak(&left), 0.0);
}

#[test]
fn block_transports_render_identical_audio() {
    let fixture = Lv2ProbeFixture::build();
    let mut rendered = Vec::new();
    for transport in [
        BlockTransport::Legacy,
        BlockTransport::Batched,
        BlockTransport::Auto,
    ] {
        let mut plugin = synth(&fixture);
        plugin.set_block_transport(transport).unwrap();
        let mut output = Vec::new();
        for block in 0..4 {
            let events = match block {
                0 => vec![note(3, 60, true)],
                2 => vec![note(5, 60, false)],
                _ => vec![],
            };
            let (mut left, mut right) = ([0.0; 64], [0.0; 64]);
            plugin.process(&events, &mut left, &mut right).unwrap();
            output.extend(left);
        }
        assert!(peak(&output) > 0.1);
        rendered.push(output);
    }
    assert_eq!(rendered[0], rendered[1]);
    assert_eq!(rendered[1], rendered[2]);
}

#[test]
fn effect_reports_latency_mixes_its_sidechain_and_recalls_state() {
    let fixture = Lv2ProbeFixture::build();
    let mut effect = VstEffect::load_class(&fixture.path, Some(GAIN_URI), 48_000.0, 64).unwrap();
    assert_eq!(effect.latency_samples(), GAIN_LATENCY);
    let details = effect.audio_bus_details().unwrap();
    let names: Vec<_> = details.inputs.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Input", "Sidechain"]);
    assert!(details.inputs[0].main && !details.inputs[1].main);
    let (mut left, mut right) = ([0.0; 64], [0.0; 64]);
    effect
        .process(&[0.5; 64], &[0.25; 64], &mut left, &mut right)
        .unwrap();
    assert_eq!(left[..32], [0.0; 32], "latency delays the signal");
    assert_eq!(left[32..], [0.5; 32]);
    assert_eq!(right[32..], [0.25; 32]);
    effect.configure_audio_buses(&[1], &[]).unwrap();
    let main = StereoBusInput {
        bus: 0,
        first_channel: 0,
        left: &[0.5; 64],
        right: &[0.5; 64],
    };
    let sidechain = StereoBusInput {
        bus: 1,
        first_channel: 0,
        left: &[0.25; 64],
        right: &[0.0; 64],
    };
    for _ in 0..2 {
        effect.process_bus_inputs(&[main, sidechain], 64).unwrap();
    }
    effect
        .copy_output_pair(0, 0, &mut left, &mut right)
        .unwrap();
    assert_eq!(left, [0.75; 64]);
    assert_eq!(right, [0.5; 64]);
    let gain = effect
        .parameters()
        .unwrap()
        .into_iter()
        .find(|p| p.name == "Level")
        .unwrap();
    assert!(
        !effect
            .parameters()
            .unwrap()
            .iter()
            .any(|p| p.name == "Latency"),
        "output controls are not parameters"
    );
    effect.set_parameter(gain.id, 0.25).unwrap();
    let state = effect.save_state().unwrap();
    effect.set_parameter(gain.id, 1.0).unwrap();
    effect.load_state(&state).unwrap();
    for _ in 0..2 {
        effect.process_bus_inputs(&[main, sidechain], 64).unwrap();
    }
    effect
        .copy_output_pair(0, 0, &mut left, &mut right)
        .unwrap();
    assert_eq!(left, [0.375; 64]);
}

#[test]
fn plugin_crash_and_hang_are_contained_and_recoverable() {
    let fixture = Lv2ProbeFixture::build();
    let mut plugin = synth(&fixture);
    let state = plugin.save_state().unwrap();
    let pid = plugin.isolation_pid().unwrap();
    let (mut left, mut right) = ([0.0; 64], [0.0; 64]);
    assert!(
        plugin
            .process(&[note(0, 127, true)], &mut left, &mut right)
            .is_err()
    );
    plugin.recover().unwrap();
    assert_ne!(plugin.isolation_pid(), Some(pid));
    plugin.load_state(&state).unwrap();
    plugin
        .process(&[note(0, 60, true)], &mut left, &mut right)
        .unwrap();
    assert!(peak(&left) > 0.1);
    let start = Instant::now();
    assert!(
        plugin
            .process(&[note(0, 126, true)], &mut left, &mut right)
            .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn unknown_uris_effects_as_instruments_and_missing_bundles_fail_cleanly() {
    let fixture = Lv2ProbeFixture::build();
    let error = VstInstrument::load_class(&fixture.path, Some("urn:rill:missing"), 48_000.0, 64)
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("urn:rill:missing"),
        "{error:#}"
    );
    assert!(VstInstrument::load_class(&fixture.path, Some(GAIN_URI), 48_000.0, 64).is_err());
    let missing = PluginInfo {
        path: fixture.path.with_file_name("Missing.lv2"),
        format: PluginFormat::Lv2,
        ..PluginInfo::default()
    };
    assert!(VstInstrument::load_info(&missing, 48_000.0, 64).is_err());
}

#[test]
fn inspection_reports_ports_and_parameters_from_the_helper() {
    let fixture = Lv2ProbeFixture::build();
    let info = PluginInfo {
        path: fixture.path.clone(),
        class_id: Some(SYNTH_URI.into()),
        format: PluginFormat::Lv2,
        ..PluginInfo::default()
    };
    let inspection = daw_plugins::inspect_plugin(&info).unwrap();
    assert!(inspection.midi_input);
    assert!(inspection.inputs.is_empty());
    let outputs: Vec<_> = inspection
        .outputs
        .iter()
        .map(|b| (b.name.as_str(), b.channels, b.main))
        .collect();
    assert_eq!(outputs, [("Output", 2, true)]);
    let names: Vec<_> = inspection
        .parameters
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["Level", "Mode"]);
}

#[test]
fn editor_requests_fall_back_to_the_generic_parameter_browser() {
    let fixture = Lv2ProbeFixture::build();
    let error = daw_plugins::EditorSession::open_class(&fixture.path, Some(SYNTH_URI), &[], &[])
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").starts_with("This plug-in has no native editor"),
        "{error:#}"
    );
}
