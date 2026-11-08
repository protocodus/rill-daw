//! Native block transport equivalence, admission, and bounded failure behavior.
#[path = "common/editor_probe_fixture.rs"]
#[allow(dead_code)]
mod editor_probe_fixture;
use daw_plugins::{
    BlockTransport, ExpressiveMidiData as Data, ExpressiveMidiMessage as Event,
    ExpressiveVelocity as Velocity, MidiMessage, VstInstrument,
};
use editor_probe_fixture::{CLASS_ID, DspProbeOptions, EditorProbeFixture};
use std::{path::PathBuf, time::Duration};
use vst3_host::{
    audio::{AudioBusBuffer, AudioBusConfig},
    midi::{MidiChannel, MidiEvent},
    process_isolation::{BlockCommand, HostCommand, HostResponse, PluginHostProcess},
};

fn plugin(fixture: &EditorProbeFixture, transport: BlockTransport) -> VstInstrument {
    let mut plugin =
        VstInstrument::load_class(&fixture.path, Some(CLASS_ID), 48_000.0, 64).unwrap();
    plugin.set_block_transport(transport).unwrap();
    plugin
}
fn outputs(plugin: &VstInstrument, frames: usize) -> Vec<u32> {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    plugin
        .copy_output_pair(plugin.main_output_bus(), 0, &mut left, &mut right)
        .unwrap();
    left.into_iter().chain(right).map(f32::to_bits).collect()
}
fn on(offset: u32, id: i32) -> Event {
    Event {
        offset,
        data: Data::NoteOn {
            channel: 0,
            pitch: 60,
            velocity: Velocity::Midi2(32769),
            note_id: id,
            tuning: 0.125,
        },
    }
}

#[test]
fn all_transports_preserve_expression_order_parameter_bits_and_exact_audio() {
    let fixture = EditorProbeFixture::build_expressive();
    let mut results = Vec::new();
    let mut traffic = Vec::new();
    for transport in [
        BlockTransport::Legacy,
        BlockTransport::Batched,
        BlockTransport::Auto,
    ] {
        let mut plugin = plugin(&fixture, transport);
        let gain = f64::from_bits(0x3fd0000000000001);
        plugin.queue_parameter_for_block(7, gain).unwrap();
        plugin
            .queue_transport_for_block(-32, -0.125, 137.5, true)
            .unwrap();
        let before = plugin.ipc_stats().unwrap();
        plugin
            .process_bus_inputs_expressive(
                &[],
                &[
                    on(2, 10),
                    Event {
                        offset: 4,
                        data: Data::NoteOff {
                            channel: 0,
                            pitch: 60,
                            velocity: Velocity::Midi2(401),
                            note_id: 10,
                            tuning: 0.0,
                        },
                    },
                    on(4, 11),
                    Event {
                        offset: 5,
                        data: Data::NoteExpressionValue {
                            type_id: 201074,
                            note_id: 11,
                            value: f64::from(0x80000001u32) / f64::from(u32::MAX),
                        },
                    },
                ],
                &[],
                8,
            )
            .unwrap();
        let after = plugin.ipc_stats().unwrap();
        assert_eq!(
            after.requests - before.requests,
            if transport == BlockTransport::Legacy {
                9
            } else {
                1
            }
        );
        traffic.push(
            after.request_bytes + after.response_bytes
                - before.request_bytes
                - before.response_bytes,
        );
        let rendered = outputs(&plugin, 8);
        assert_eq!(rendered[0], 0.0_f32.to_bits());
        assert_ne!(
            rendered[4],
            0.0_f32.to_bits(),
            "same-offset off/on must leave the new voice sounding"
        );
        results.push(rendered);
        let actual = plugin
            .parameters()
            .unwrap()
            .into_iter()
            .find(|parameter| parameter.id == 7)
            .unwrap()
            .value;
        assert_eq!(actual.to_bits(), gain.to_bits());
        // Panic releases precede same-offset chased note-ons, and variable frames keep state.
        plugin.reset().unwrap();
        plugin
            .process_bus_inputs_expressive(&[], &[on(0, 12)], &[], 17)
            .unwrap();
        results.push(outputs(&plugin, 17));
    }
    assert_eq!(results[0], results[2]);
    assert_eq!(results[0], results[4]);
    assert_eq!(results[1], results[3]);
    assert_eq!(results[1], results[5]);
    assert!(
        traffic[2] < traffic[1],
        "shared PCM must remove audio bytes: {traffic:?}"
    );
}

#[test]
fn local_rejection_and_zero_frames_preserve_queued_automation_without_dispatch() {
    let fixture = EditorProbeFixture::build();
    for transport in [
        BlockTransport::Legacy,
        BlockTransport::Batched,
        BlockTransport::Auto,
    ] {
        let mut plugin = plugin(&fixture, transport);
        plugin.queue_parameter_for_block(7, 0.75).unwrap();
        plugin
            .queue_transport_for_block(-8, -0.01, 140.0, false)
            .unwrap();
        let before = plugin.ipc_stats().unwrap();
        plugin.process_bus_inputs(&[], &[], 0).unwrap();
        assert!(
            plugin
                .process_bus_inputs(
                    &[MidiMessage {
                        offset: 8,
                        pitch: 60,
                        velocity: 100,
                        on: true
                    }],
                    &[],
                    8
                )
                .is_err()
        );
        assert_eq!(plugin.ipc_stats().unwrap(), before);
        plugin
            .process_bus_inputs(
                &[MidiMessage {
                    offset: 0,
                    pitch: 60,
                    velocity: 100,
                    on: true,
                }],
                &[],
                8,
            )
            .unwrap();
        assert!(
            outputs(&plugin, 8)
                .iter()
                .all(|sample| *sample == 0.1875_f32.to_bits())
        );
        let after = plugin.ipc_stats().unwrap();
        assert_eq!(
            after.requests - before.requests,
            if transport == BlockTransport::Legacy {
                6
            } else {
                1
            }
        );
    }
}

fn worker(fixture: &EditorProbeFixture, timeout: Duration) -> PluginHostProcess {
    let mut worker = PluginHostProcess::new(
        Some(PathBuf::from(env!("CARGO_BIN_EXE_daw-plugin-host"))),
        timeout,
    )
    .unwrap();
    let reply = worker
        .send_command(HostCommand::LoadPlugin {
            path: fixture.path.to_string_lossy().into_owned(),
            sample_rate: 48_000.0,
            block_size: 64,
            tempo: 120.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            class_id: Some(CLASS_ID.into()),
        })
        .unwrap();
    assert!(
        matches!(reply, HostResponse::PluginInfo { .. }),
        "{reply:?}"
    );
    assert!(matches!(
        worker.send_command(HostCommand::StartProcessing).unwrap(),
        HostResponse::Success { .. }
    ));
    worker
}
fn block(commands: Vec<BlockCommand>, frames: u32) -> HostCommand {
    HostCommand::ProcessBlock {
        commands,
        inputs: vec![],
        outputs: vec![AudioBusConfig {
            active: true,
            channel_count: 2,
        }],
        frames,
        shared_audio: None,
    }
}
fn note(offset: i32) -> BlockCommand {
    BlockCommand::SendMidiAt {
        event: MidiEvent::NoteOn {
            channel: MidiChannel::Ch1,
            note: 60,
            velocity: 100,
        },
        sample_offset: offset,
    }
}

#[test]
fn malformed_batch_is_rejected_before_any_parameter_or_note_side_effect() {
    let fixture = EditorProbeFixture::build();
    let mut worker = worker(&fixture, Duration::from_secs(2));
    let before = worker.ipc_stats();
    assert!(
        worker
            .send_command(block(
                vec![
                    BlockCommand::SetParameter { id: 7, value: 0.75 };
                    vst3_host::parameters::MAX_PENDING_PARAM_CHANGES + 1
                ],
                8
            ))
            .is_err()
    );
    assert_eq!(
        worker.ipc_stats(),
        before,
        "over-capacity raw requests must fail before serialization or dispatch"
    );
    for request in [
        block(
            vec![
                BlockCommand::SetParameter { id: 7, value: 0.75 },
                note(0),
                note(8),
            ],
            8,
        ),
        HostCommand::ProcessBlock {
            commands: vec![note(0)],
            inputs: vec![AudioBusBuffer::new(1, 8, true)],
            outputs: vec![AudioBusConfig {
                active: true,
                channel_count: 2,
            }],
            frames: 8,
            shared_audio: None,
        },
    ] {
        assert!(matches!(
            worker.send_command(request).unwrap(),
            HostResponse::Error { .. }
        ));
    }
    assert!(
        matches!(worker.send_command(HostCommand::GetParameter {id:7}).unwrap(), HostResponse::ParameterValue{value} if value == 0.5)
    );
    let response = worker.send_command(block(vec![], 8)).unwrap();
    let HostResponse::BusAudioOutput { outputs, .. } = response else {
        panic!("{response:?}")
    };
    assert!(
        outputs
            .iter()
            .flat_map(|bus| &bus.channels)
            .flatten()
            .all(|sample| *sample == 0.0)
    );
}

#[test]
fn failed_native_batch_is_not_processed_or_replayed_again() {
    let trace = tempfile::tempdir().unwrap();
    let fixture = EditorProbeFixture::build_cpu_load(DspProbeOptions {
        stages: 1,
        trace_directory: Some(trace.path()),
        fail_after_blocks: Some(0),
        ..Default::default()
    });
    for transport in [
        BlockTransport::Legacy,
        BlockTransport::Batched,
        BlockTransport::Auto,
    ] {
        let mut plugin = plugin(&fixture, transport);
        plugin.queue_parameter_for_block(7, 0.75).unwrap();
        assert!(
            plugin
                .process_bus_inputs(
                    &[MidiMessage {
                        offset: 0,
                        pitch: 60,
                        velocity: 100,
                        on: true
                    }],
                    &[],
                    8
                )
                .is_err()
        );
        assert!(outputs(&plugin, 8).iter().all(|sample| *sample == 0));
        let failed = plugin.ipc_stats().unwrap();
        assert!(plugin.process_bus_inputs(&[], &[], 8).is_err());
        assert_eq!(
            plugin.ipc_stats().unwrap(),
            failed,
            "failed instances must not dispatch another block"
        );
    }
    let callbacks: usize = std::fs::read_dir(trace.path())
        .unwrap()
        .map(|entry| {
            std::fs::read_to_string(entry.unwrap().path())
                .unwrap()
                .lines()
                .count()
        })
        .sum();
    assert_eq!(
        callbacks, 3,
        "one native processing call per failed instance"
    );
}

#[test]
fn processing_batch_keeps_audio_deadline_and_never_retries() {
    let fixture = EditorProbeFixture::build_with_delays(Duration::ZERO, Duration::from_millis(500));
    let mut worker = worker(&fixture, Duration::from_millis(100));
    let before = worker.ipc_stats();
    let error = worker.send_command(block(vec![note(0)], 8)).unwrap_err();
    assert!(error.to_lowercase().contains("timed out"), "{error}");
    assert!(!worker.is_alive());
    assert_eq!(worker.ipc_stats().requests - before.requests, 1);
    assert!(worker.send_command(block(vec![], 8)).is_err());
    assert_eq!(worker.ipc_stats().requests - before.requests, 1);
}

#[test]
fn rejected_legacy_shape_and_partial_batch_failure_cannot_leak_events_across_apis() {
    let fixture = EditorProbeFixture::build_cpu_load(DspProbeOptions {
        fail_after_blocks: Some(1),
        ..Default::default()
    });
    for transport in [
        BlockTransport::Legacy,
        BlockTransport::Batched,
        BlockTransport::Auto,
    ] {
        let mut host = vst3_host::Vst3Host::builder()
            .sample_rate(48_000.0)
            .block_size(64)
            .with_process_isolation(true)
            .auto_recover_plugins(false)
            .helper_path(env!("CARGO_BIN_EXE_daw-plugin-host"))
            .build()
            .unwrap();
        let mut plugin = host.load_plugin_class(&fixture.path, CLASS_ID).unwrap();
        plugin.start_processing().unwrap();
        plugin.set_block_transport(transport).unwrap();
        let mut buffers = plugin.create_bus_audio_buffers(8).unwrap();
        buffers.outputs[0].active = false;
        let mut commands = vec![note(0)];
        let before = plugin.ipc_stats().unwrap();
        assert!(
            plugin
                .process_bus_audio_block(&mut buffers, &mut commands)
                .is_err()
        );
        assert_eq!(
            plugin.ipc_stats().unwrap(),
            before,
            "malformed layout must precede legacy event dispatch"
        );
        buffers.outputs[0].active = true;
        plugin.process_bus_audio(&mut buffers).unwrap();
        assert!(
            buffers
                .outputs
                .iter()
                .flat_map(|bus| &bus.channels)
                .flatten()
                .all(|sample| *sample == 0.0)
        );
        commands.push(BlockCommand::SetParameter { id: 7, value: 0.5 });
        let capacity = commands.capacity();
        assert!(
            plugin
                .process_bus_audio_block(&mut buffers, &mut commands)
                .is_err()
        );
        assert!(
            commands.is_empty(),
            "possibly applied operations must never be replayed"
        );
        assert!(
            commands.capacity() >= capacity,
            "command allocation is reusable"
        );
        let failed = plugin.ipc_stats().unwrap();
        assert!(plugin.process_bus_audio(&mut buffers).is_err());
        let mut flat = vst3_host::AudioBuffers::new(0, 2, 8, 48_000.0);
        assert!(plugin.process_audio(&mut flat).is_err());
        assert_eq!(plugin.ipc_stats().unwrap(), failed);
        plugin.recover().unwrap();
        plugin.process_bus_audio(&mut buffers).unwrap();
        assert!(
            buffers
                .outputs
                .iter()
                .flat_map(|bus| &bus.channels)
                .flatten()
                .all(|sample| *sample == 0.0)
        );
    }
}

#[test]
fn overflowing_native_parameter_queue_rejects_before_controller_mirroring() {
    let fixture = EditorProbeFixture::build();
    let mut host = vst3_host::Vst3Host::builder()
        .sample_rate(48_000.0)
        .block_size(64)
        .with_process_isolation(false)
        .build()
        .unwrap();
    let mut plugin = host.load_plugin_class(&fixture.path, CLASS_ID).unwrap();
    for _ in 0..vst3_host::parameters::MAX_PENDING_PARAM_CHANGES {
        plugin.set_parameter(7, 0.25).unwrap();
    }
    assert_eq!(plugin.get_parameter(7).unwrap(), 0.25);
    assert!(plugin.set_parameter(7, 0.75).is_err());
    assert_eq!(
        plugin.get_parameter(7).unwrap(),
        0.25,
        "rejected DSP point must not change the controller"
    );
    plugin.start_processing().unwrap();
    let mut buffers = plugin.create_bus_audio_buffers(8).unwrap();
    plugin.process_bus_audio(&mut buffers).unwrap();
    plugin.set_parameter(7, 0.75).unwrap();
    assert_eq!(
        plugin.get_parameter(7).unwrap(),
        0.75,
        "drained queue admits the next point"
    );
}
