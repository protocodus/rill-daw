//! Headless plugin worker. Only this executable process loads third-party native code.
//!
//! All plugin lifecycle, controller, and processing calls execute on this process's main
//! thread. Native editors are deliberately unsupported. The main DAW enforces response
//! deadlines, terminates hung children, and can start a fresh instance after a crash.

use anyhow::{Context, Result, bail, ensure};
use std::io::{BufRead, Write};
use vst3_host::process_isolation::{HostCommand, HostResponse, ProtocolChannel};
use vst3_host::{AudioBuffers, AudioBusLayout, BusAudioBuffers, Plugin, Vst3Host};

/// Run only in the standalone `daw-plugin-host` executable. Claim stdout before loading
/// any plugin so a plugin's console messages cannot corrupt the response protocol on Unix.
pub fn run() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--editor") => return crate::editor::helper_main(),
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        Some("--native-context-menu") => return crate::editor_context_menu::run_worker(),
        _ => {}
    }
    let mut protocol = ProtocolChannel::claim();
    let input = std::io::stdin();
    let mut worker = Worker::default();
    for line in input.lock().lines() {
        let line = line?;
        let response = match serde_json::from_str::<HostCommand>(&line) {
            Ok(HostCommand::Shutdown) => break,
            Ok(command) => match worker.handle(command) {
                Ok(response) => response,
                Err(error) => HostResponse::Error {
                    message: format!("{error:#}"),
                },
            },
            Err(error) => HostResponse::Error {
                message: format!("Invalid helper command: {error}"),
            },
        };
        serde_json::to_writer(&mut protocol, &response)?;
        writeln!(protocol)?;
        protocol.flush()?;
    }
    Ok(())
}

#[derive(Default)]
struct Worker {
    plugin: Option<Plugin>,
    sample_rate: f64,
    max_block: usize,
}

impl Worker {
    fn handle(&mut self, command: HostCommand) -> Result<HostResponse> {
        if let HostCommand::LoadPlugin {
            path,
            sample_rate,
            block_size,
            tempo,
            time_sig_numerator,
            time_sig_denominator,
            class_id,
        } = command
        {
            ensure!(
                (1..=65_536).contains(&block_size),
                "Invalid maximum block size"
            );
            let mut host = Vst3Host::builder()
                .with_process_isolation(false)
                .sample_rate(sample_rate)
                .block_size(block_size as usize)
                .tempo(tempo)
                .time_signature(time_sig_numerator, time_sig_denominator)
                .build()?;
            let plugin = match class_id {
                Some(id) => host.load_plugin_class(path, &id)?,
                None => host.load_plugin(path)?,
            };
            let info = plugin.info();
            let response = HostResponse::PluginInfo {
                vendor: info.vendor.clone(),
                name: info.name.clone(),
                version: info.version.clone(),
                category: info.category.clone(),
                uid: info.uid.clone(),
                has_gui: false,
                audio_inputs: info.audio_inputs as i32,
                audio_outputs: info.audio_outputs as i32,
                output_channels: plugin.output_channel_count() as i32,
                has_midi_input: info.has_midi_input,
                has_midi_output: info.has_midi_output,
                compatibility: plugin.class_compatibility().to_vec(),
            };
            self.plugin = Some(plugin);
            self.sample_rate = sample_rate;
            self.max_block = block_size as usize;
            return Ok(response);
        }
        if matches!(command, HostCommand::UnloadPlugin) {
            self.plugin = None;
            return Ok(success());
        }
        let plugin = self.plugin.as_mut().context("No plugin loaded")?;
        match command {
            HostCommand::StartProcessing => plugin.start_processing()?,
            HostCommand::StopProcessing => plugin.stop_processing()?,
            HostCommand::Reconfigure {
                sample_rate,
                block_size,
            } => {
                ensure!(
                    (1..=65_536).contains(&block_size),
                    "Invalid maximum block size"
                );
                plugin.reconfigure(sample_rate, block_size as usize)?;
                self.sample_rate = sample_rate;
                self.max_block = block_size as usize;
            }
            HostCommand::SetProcessMode { mode } => plugin.set_process_mode(mode)?,
            HostCommand::SetTransportPosition { position } => {
                plugin.set_transport_position(position)?
            }
            HostCommand::TransportPosition => {
                return Ok(HostResponse::TransportPosition {
                    position: plugin.transport_position()?,
                });
            }
            HostCommand::SetTempo { bpm } => plugin.set_tempo(bpm)?,
            HostCommand::SetPlaying { playing } => plugin.set_playing(playing)?,
            HostCommand::SetTimeSignature {
                numerator,
                denominator,
            } => plugin.set_time_signature(numerator, denominator)?,
            HostCommand::SetParameter { id, value } => plugin.set_parameter(id, value)?,
            HostCommand::SetParameterAt { id, value, offset } => {
                plugin.set_parameter_at(id, value, offset)?
            }
            HostCommand::GetParameter { id } => {
                return Ok(HostResponse::ParameterValue {
                    value: plugin.get_parameter(id)?,
                });
            }
            HostCommand::GetAllParameters => {
                return Ok(HostResponse::Parameters {
                    params: plugin.get_parameters()?,
                });
            }
            HostCommand::FormatParameter { id, normalized } => {
                return Ok(HostResponse::ParameterString {
                    value: plugin.format_parameter(id, normalized)?,
                });
            }
            HostCommand::SendMidi { event } => plugin.send_midi_event(event)?,
            HostCommand::SendMidiAt {
                event,
                sample_offset,
            } => plugin.send_midi_event_at(event, sample_offset)?,
            HostCommand::SendPluginEvent { event } => plugin.send_plugin_event(event)?,
            HostCommand::MidiPanic => plugin.midi_panic()?,
            HostCommand::AudioBusLayout => {
                return Ok(HostResponse::AudioBusLayout {
                    layout: plugin.audio_bus_layout()?,
                });
            }
            HostCommand::LatencySamples => {
                return Ok(HostResponse::LatencySamples {
                    samples: plugin.latency_samples(),
                });
            }
            HostCommand::TailSamples => {
                return Ok(HostResponse::TailSamples {
                    samples: plugin.tail_samples(),
                });
            }
            HostCommand::SaveState => {
                return Ok(HostResponse::State {
                    data: plugin.save_state()?,
                });
            }
            HostCommand::LoadState { data, context } => {
                plugin.load_state_with_context(&data, &context)?
            }
            HostCommand::ProcessBuses {
                inputs,
                outputs,
                frames,
            } => {
                ensure!(
                    frames > 0 && frames as usize <= self.max_block,
                    "Invalid processing block size"
                );
                let expected = plugin.audio_bus_layout()?;
                ensure!(outputs == expected.outputs, "Output bus layout changed");
                ensure!(
                    inputs.len() == expected.inputs.len(),
                    "Input bus layout changed"
                );
                let mut buffers = BusAudioBuffers::new(
                    &AudioBusLayout {
                        inputs: expected.inputs,
                        outputs,
                    },
                    frames as usize,
                    self.sample_rate,
                );
                buffers.inputs = inputs;
                plugin.process_bus_audio(&mut buffers)?;
                return Ok(HostResponse::BusAudioOutput {
                    outputs: buffers.outputs,
                    output_events: plugin.take_output_events(),
                });
            }
            HostCommand::Process { inputs, frames } => {
                ensure!(
                    frames > 0 && frames as usize <= self.max_block,
                    "Invalid processing block size"
                );
                let mut buffers = AudioBuffers::new(
                    inputs.len(),
                    plugin.output_channel_count(),
                    frames as usize,
                    self.sample_rate,
                );
                buffers.inputs = inputs;
                plugin.process_audio(&mut buffers)?;
                return Ok(HostResponse::AudioOutput {
                    outputs: buffers.outputs,
                    output_events: plugin.take_output_events(),
                });
            }
            HostCommand::CreateGui | HostCommand::CloseGui => {
                bail!("Native plugin editors are not supported by the headless helper")
            }
            _ => bail!("Command is not supported by this version of the DAW plugin helper"),
        }
        Ok(success())
    }
}

fn success() -> HostResponse {
    HostResponse::Success {
        message: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_before_loading_return_error() {
        let mut worker = Worker::default();
        assert!(worker.handle(HostCommand::StartProcessing).is_err());
        assert!(worker.handle(HostCommand::UnloadPlugin).is_ok());
    }
}
