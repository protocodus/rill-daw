//! Real native CLAP instruments and effects across the isolation helper.
#[path = "common/clap_probe_fixture.rs"]
mod clap_probe_fixture;

use clap_probe_fixture::{ClapProbeFixture, GAIN_ID, GAIN_LATENCY, SYNTH_ID};
use daw_plugins::{
    BlockTransport, ExpressiveMidiData, ExpressiveMidiMessage, ExpressiveVelocity, MidiMessage,
    PluginFormat, PluginInfo, StereoBusInput, VstEffect, VstInstrument,
};
use std::time::{Duration, Instant};

fn synth(fixture: &ClapProbeFixture) -> VstInstrument {
    VstInstrument::load_class(&fixture.path, Some(SYNTH_ID), 48_000.0, 64).unwrap()
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
fn scan_reads_every_descriptor_in_the_helper_with_format_and_category() {
    let fixture = ClapProbeFixture::build();
    let found = daw_plugins::scan_paths(&[fixture.path.parent().unwrap().to_owned()]).unwrap();
    let ids: Vec<_> = found.iter().map(|p| p.class_id.as_deref()).collect();
    assert_eq!(ids, [Some(GAIN_ID), Some(SYNTH_ID)], "{found:?}");
    assert!(found.iter().all(|p| p.format == PluginFormat::Clap));
    assert!(
        found
            .iter()
            .all(|p| p.vendor == "RILL tests" && p.version == "1.0.0")
    );
    let synth = &found[1];
    assert_eq!(synth.name, "Rill CLAP Synth");
    assert_eq!(synth.is_instrument(), Some(true));
    assert_eq!(found[0].is_effect(), Some(true));
    assert_eq!(found[0].is_instrument(), Some(false));
}

#[test]
fn instrument_renders_notes_parameters_state_and_transport_in_isolation() {
    let fixture = ClapProbeFixture::build();
    let info = daw_plugins::scan_paths(std::slice::from_ref(&fixture.path))
        .unwrap()
        .into_iter()
        .find(|p| p.class_id.as_deref() == Some(SYNTH_ID))
        .unwrap();
    let mut plugin = VstInstrument::load_info(&info, 48_000.0, 64).unwrap();
    assert_ne!(plugin.isolation_pid(), Some(std::process::id()));
    assert_eq!(plugin.info().format, PluginFormat::Clap);
    assert_eq!(plugin.info().class_id.as_deref(), Some(SYNTH_ID));
    assert_eq!(plugin.latency_samples(), 0);
    let (mut left, mut right) = ([0.0; 64], [0.0; 64]);
    plugin.process(&[], &mut left, &mut right).unwrap();
    assert_eq!(peak(&left), 0.0);
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
    let state = plugin.save_state().unwrap();
    assert!(state.starts_with(b"RILLCLAP"));
    plugin.set_parameter(level.id, 1.0).unwrap();
    plugin.load_state(&state).unwrap();
    let restored = plugin.parameters().unwrap();
    assert_eq!(
        restored.iter().find(|p| p.id == level.id).unwrap().value,
        0.25
    );

    // Automation is applied at its block offset.
    plugin.queue_parameter_for_block(level.id, 0.0).unwrap();
    plugin
        .process(&[note(0, 69, true)], &mut left, &mut right)
        .unwrap();
    assert_eq!(peak(&left), 0.0);

    plugin
        .queue_transport_for_block(4_800, 0.2, 150.0, true)
        .unwrap();
    plugin.process_bus_inputs(&[], &[], 64).unwrap();
    let (mut aux_left, mut aux_right) = ([0.0; 64], [0.0; 64]);
    plugin
        .copy_output_pair(1, 0, &mut aux_left, &mut aux_right)
        .unwrap();
    assert!((aux_left[0] - 0.15).abs() < 1e-6, "{}", aux_left[0]);
    assert!((aux_right[0] - 0.2).abs() < 1e-6, "{}", aux_right[0]);
    plugin.process_bus_inputs(&[], &[], 64).unwrap();
    plugin
        .copy_output_pair(1, 0, &mut aux_left, &mut aux_right)
        .unwrap();
    let advanced = 0.2 + 64.0 / 48_000.0 * 150.0 / 60.0;
    assert!((f64::from(aux_right[0]) - advanced).abs() < 1e-5);
}

#[test]
fn block_transports_render_identical_audio() {
    let fixture = ClapProbeFixture::build();
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
                0 => vec![note(3, 60, true), note(9, 64, true)],
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
fn effect_reports_latency_and_mixes_its_sidechain_bus() {
    let fixture = ClapProbeFixture::build();
    let mut effect = VstEffect::load_class(&fixture.path, Some(GAIN_ID), 48_000.0, 64).unwrap();
    assert_eq!(effect.latency_samples(), GAIN_LATENCY);
    let details = effect.audio_bus_details().unwrap();
    let names: Vec<_> = details.inputs.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Main", "Sidechain"]);
    assert!(details.inputs[0].main && !details.inputs[1].main);
    assert_eq!(effect.main_input_bus(), Some(0));
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
fn note_expressions_reach_the_addressed_clap_voice() {
    let fixture = ClapProbeFixture::build();
    let mut plugin = synth(&fixture);
    assert_eq!(
        plugin.supported_note_expression_types(0).unwrap(),
        [0, 1, 2, 3, 4, 5]
    );
    let on = ExpressiveMidiMessage {
        offset: 0,
        data: ExpressiveMidiData::NoteOn {
            channel: 0,
            pitch: 69,
            velocity: ExpressiveVelocity::Midi2(u16::MAX),
            note_id: 7,
            tuning: 0.0,
        },
    };
    let (mut left, mut right) = ([0.0; 64], [0.0; 64]);
    plugin
        .process_bus_inputs_expressive(&[], &[on], &[], 64)
        .unwrap();
    plugin
        .copy_output_pair(0, 0, &mut left, &mut right)
        .unwrap();
    let base = peak(&left);
    // VST3-normalized volume 0.5 is CLAP linear gain 2.0.
    let louder = ExpressiveMidiMessage {
        offset: 0,
        data: ExpressiveMidiData::NoteExpressionValue {
            type_id: 0,
            note_id: 7,
            value: 0.5,
        },
    };
    for _ in 0..4 {
        plugin
            .process_bus_inputs_expressive(&[], &[louder], &[], 64)
            .unwrap();
    }
    plugin
        .copy_output_pair(0, 0, &mut left, &mut right)
        .unwrap();
    assert!(
        (peak(&left) / base - 2.0).abs() < 0.1,
        "{} vs {base}",
        peak(&left)
    );
}

#[cfg(unix)]
#[test]
fn plugin_crash_and_hang_are_contained_and_recoverable() {
    let fixture = ClapProbeFixture::build();
    let mut plugin = synth(&fixture);
    let state = plugin.save_state().unwrap();
    let pid = plugin.isolation_pid().unwrap();
    let (mut left, mut right) = ([0.0; 64], [0.0; 64]);
    assert!(
        plugin
            .process(&[note(0, 127, true)], &mut left, &mut right)
            .is_err()
    );
    assert_eq!(peak(&left), 0.0);
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
    assert_eq!(peak(&left), 0.0);
}

#[test]
fn unknown_plugin_ids_and_missing_bundles_fail_cleanly() {
    let fixture = ClapProbeFixture::build();
    let error = VstInstrument::load_class(&fixture.path, Some("org.rill.missing"), 48_000.0, 64)
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("org.rill.missing"),
        "{error:#}"
    );
    let effect_as_instrument =
        VstInstrument::load_class(&fixture.path, Some(GAIN_ID), 48_000.0, 64);
    assert!(effect_as_instrument.is_err(), "effects accept no MIDI");
    let missing = PluginInfo {
        path: fixture.path.with_file_name("Missing.clap"),
        format: PluginFormat::Clap,
        ..PluginInfo::default()
    };
    assert!(VstInstrument::load_info(&missing, 48_000.0, 64).is_err());
}

#[test]
fn inspection_reports_ports_and_parameters_from_the_helper() {
    let fixture = ClapProbeFixture::build();
    let info = PluginInfo {
        path: fixture.path.clone(),
        class_id: Some(SYNTH_ID.into()),
        format: PluginFormat::Clap,
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
    assert_eq!(outputs, [("Main", 2, true), ("Aux", 2, false)]);
    let names: Vec<_> = inspection
        .parameters
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["Level", "Mode"]);
    assert!(inspection.parameters.iter().all(|p| p.can_automate));
}
