//! Actual native VST3 loading through the shipped sandbox worker, without a GUI.
#[path = "common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vst3_host::midi::{MidiChannel, MidiEvent};
use vst3_host::plugin::{ProcessMode, StateContext};
use vst3_host::process_isolation::{HostCommand, HostResponse, PluginHostProcess};

fn success(worker: &mut PluginHostProcess, command: HostCommand) {
    let response = worker.send_command(command).unwrap();
    assert!(
        matches!(response, HostResponse::Success { .. }),
        "{response:?}"
    );
}

fn render(worker: &mut PluginHostProcess) -> Vec<Vec<f32>> {
    match worker
        .send_command(HostCommand::Process {
            inputs: vec![],
            frames: 64,
        })
        .unwrap()
    {
        HostResponse::AudioOutput { outputs, .. } => {
            assert_eq!(outputs.len(), 2);
            assert!(outputs.iter().all(|channel| channel.len() == 64));
            outputs
        }
        response => panic!("Expected audio: {response:?}"),
    }
}

fn load_fixture(fixture: &EditorProbeFixture, timeout: Duration) -> PluginHostProcess {
    let mut worker = PluginHostProcess::new(
        Some(PathBuf::from(env!("CARGO_BIN_EXE_daw-plugin-host"))),
        timeout,
    )
    .unwrap();
    // Cold loading several freshly compiled native bundles can contend on OS
    // verification in parallel CI. Bootstrap is outside the deadline assertions.
    worker.set_slow_command_timeout(Duration::from_secs(15));
    let response = worker
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
        matches!(response, HostResponse::PluginInfo { .. }),
        "{response:?}"
    );
    worker.set_slow_command_timeout(Duration::from_secs(3));
    worker
}

#[test]
fn slow_native_activation_and_reconfiguration_preserve_the_worker() {
    let fixture = EditorProbeFixture::build_with_lifecycle_delay(Duration::from_millis(200));
    let process_timeout = Duration::from_millis(100);
    let mut worker = load_fixture(&fixture, process_timeout);
    let pid = worker.helper_pid();
    let started = Instant::now();
    let response = worker.send_command(HostCommand::GetAllParameters).unwrap();
    assert!(
        matches!(response, HostResponse::Parameters { ref params }
        if params.len() == 1 && params[0].id == 7),
        "{response:?}"
    );
    assert!(
        started.elapsed() > process_timeout,
        "Full metadata enumeration may exceed the audio deadline"
    );
    for command in [
        HostCommand::StartProcessing,
        HostCommand::StopProcessing,
        HostCommand::Reconfigure {
            sample_rate: 44_100.0,
            block_size: 128,
        },
        HostCommand::SetProcessMode {
            mode: ProcessMode::Offline,
        },
    ] {
        let started = Instant::now();
        success(&mut worker, command);
        assert!(
            started.elapsed() > process_timeout,
            "Fixture must exceed the audio deadline"
        );
        assert!(worker.is_alive());
        assert_eq!(worker.helper_pid(), pid);
    }
    success(&mut worker, HostCommand::StartProcessing);
    assert!(
        render(&mut worker)
            .iter()
            .flatten()
            .all(|sample| *sample == 0.0)
    );
    success(&mut worker, HostCommand::UnloadPlugin);
    assert!(worker.is_alive());
}

#[test]
fn native_lifecycle_operations_still_have_a_bounded_deadline() {
    let fixture = EditorProbeFixture::build_with_lifecycle_delay(Duration::from_millis(200));
    let mut worker = load_fixture(&fixture, Duration::from_millis(50));
    worker.set_slow_command_timeout(Duration::from_millis(100));
    let started = Instant::now();
    let error = worker
        .send_command(HostCommand::StartProcessing)
        .unwrap_err();
    assert!(error.to_lowercase().contains("timed out"), "{error}");
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        !worker.is_alive(),
        "A lifecycle operation exceeding its deadline must be killed"
    );
    assert!(worker.send_command(HostCommand::StartProcessing).is_err());
}

#[test]
fn native_audio_processing_keeps_the_short_deadline() {
    let fixture = EditorProbeFixture::build_with_delays(Duration::ZERO, Duration::from_millis(500));
    for command in [
        HostCommand::Process {
            inputs: vec![],
            frames: 64,
        },
        HostCommand::ProcessBuses {
            inputs: vec![],
            outputs: vec![vst3_host::audio::AudioBusConfig {
                channel_count: 2,
                active: true,
            }],
            frames: 64,
        },
    ] {
        let mut worker = load_fixture(&fixture, Duration::from_millis(100));
        success(&mut worker, HostCommand::StartProcessing);
        let started = Instant::now();
        let error = worker.send_command(command).unwrap_err();
        assert!(error.to_lowercase().contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            !worker.is_alive(),
            "Audio must not use the slower lifecycle deadline"
        );
    }
}

#[test]
fn native_plugin_load_process_parameters_and_recall_never_construct_an_editor() {
    let fixture = EditorProbeFixture::build();
    let mut worker = PluginHostProcess::new(
        Some(PathBuf::from(env!("CARGO_BIN_EXE_daw-plugin-host"))),
        Duration::from_secs(5),
    )
    .unwrap();
    let pid = worker.helper_pid().unwrap();
    for _ in 0..2 {
        let response = worker
            .send_command(HostCommand::LoadPlugin {
                path: fixture.path.to_string_lossy().into_owned(),
                sample_rate: 48_000.0,
                block_size: 64,
                tempo: 120.0,
                time_sig_numerator: 4,
                time_sig_denominator: 4,
                class_id: Some(CLASS_ID.into()),
            })
            .expect("DSP loading must not construct a plugin editor (fixture exits86 if probed)");
        assert!(
            matches!(response, HostResponse::PluginInfo {
            ref name, audio_inputs: 0, audio_outputs: 1, output_channels: 2,
            has_midi_input: true, ..
        } if name == "No Editor Probe"),
            "{response:?}"
        );
        let response = worker.send_command(HostCommand::GetAllParameters).unwrap();
        assert!(
            matches!(response, HostResponse::Parameters { ref params }
            if params.len() == 1 && params[0].id == 7 && params[0].can_automate),
            "{response:?}"
        );
        let response = worker
            .send_command(HostCommand::FormatParameter {
                id: 7,
                normalized: 0.5,
            })
            .unwrap();
        assert!(
            matches!(response, HostResponse::ParameterString { ref value } if value == "50"),
            "{response:?}"
        );
        success(
            &mut worker,
            HostCommand::SetParameter { id: 7, value: 0.75 },
        );
        let state = match worker.send_command(HostCommand::SaveState).unwrap() {
            HostResponse::State { data } => {
                assert!(!data.is_empty());
                data
            }
            response => panic!("Expected plugin state: {response:?}"),
        };
        success(
            &mut worker,
            HostCommand::SetParameter { id: 7, value: 0.25 },
        );
        success(
            &mut worker,
            HostCommand::LoadState {
                data: state,
                context: StateContext::Project,
            },
        );
        let response = worker
            .send_command(HostCommand::GetParameter { id: 7 })
            .unwrap();
        assert!(
            matches!(response, HostResponse::ParameterValue { value } if value == 0.75),
            "{response:?}"
        );
        success(&mut worker, HostCommand::StartProcessing);
        assert!(
            render(&mut worker)
                .iter()
                .flatten()
                .all(|sample| *sample == 0.0)
        );
        success(
            &mut worker,
            HostCommand::SendMidiAt {
                event: MidiEvent::NoteOn {
                    channel: MidiChannel::Ch1,
                    note: 60,
                    velocity: 100,
                },
                sample_offset: 16,
            },
        );
        let output = render(&mut worker);
        for channel in output {
            assert!(channel[..16].iter().all(|sample| *sample == 0.0));
            assert!(channel[16..].iter().all(|sample| *sample == 0.1875));
        }
        success(
            &mut worker,
            HostCommand::SendMidiAt {
                event: MidiEvent::NoteOff {
                    channel: MidiChannel::Ch1,
                    note: 60,
                    velocity: 0,
                },
                sample_offset: 32,
            },
        );
        let output = render(&mut worker);
        for channel in output {
            assert!(channel[..32].iter().all(|sample| *sample == 0.1875));
            assert!(channel[32..].iter().all(|sample| *sample == 0.0));
        }
        success(&mut worker, HostCommand::StopProcessing);
        success(&mut worker, HostCommand::UnloadPlugin);
        assert!(worker.is_alive());
        assert_eq!(
            worker.helper_pid(),
            Some(pid),
            "DSP operations must preserve the helper"
        );
    }
}

#[test]
fn native_editor_capability_queries_do_not_construct_a_view() {
    // Re-exec this exact test so an accidental createView kills only a bounded child,
    // with a useful exit86 failure, rather than terminating the entire test suite.
    if let Some(path) = std::env::var_os("RILL_TEST_EDITOR_PROBE_PLUGIN") {
        let mut host = vst3_host::Vst3Host::builder().build().unwrap();
        let plugin = host.load_plugin_class(path, CLASS_ID).unwrap();
        for _ in 0..8 {
            assert!(plugin.has_editor());
        }
        return;
    }
    let fixture = EditorProbeFixture::build();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_editor_capability_queries_do_not_construct_a_view",
            "--nocapture",
        ])
        .env("RILL_TEST_EDITOR_PROBE_PLUGIN", &fixture.path)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "In-process load/capability query called createView: {status}"
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Native capability query child timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
