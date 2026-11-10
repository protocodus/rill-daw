//! Headless plugin worker. Only this executable process loads third-party native code.
//!
//! All plugin lifecycle, controller, and processing calls execute on this process's main
//! thread. Native editors are deliberately unsupported. The main DAW enforces response
//! deadlines, terminates hung children, and can start a fresh instance after a crash.

use anyhow::{Context, Result, bail, ensure};
use std::io::{BufRead, BufWriter, Write};
use vst3_host::process_isolation::{BlockCommand, HostCommand, HostResponse, ProtocolChannel};
use vst3_host::{AudioBuffers, BusAudioBuffers, audio::AudioBusBuffer};

use crate::native::{LoadRequest, NativeProcessor};

use crate::audio_buffers::{prepare_bus_outputs, prepare_outputs, validate_audio_bus_layout};

/// Run only in the standalone `daw-plugin-host` executable. Claim stdout before loading
/// any plugin so a plugin's console messages cannot corrupt the response protocol on Unix.
pub fn run() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--editor") => return crate::editor::helper_main(),
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        Some("--native-context-menu") => return crate::editor_context_menu::run_worker(),
        Some("--scan") => return crate::scan::helper_main(),
        _ => {}
    }
    // Coalesce JSON punctuation and payload writes into buffered pipe writes;
    // every complete response is still flushed before accepting another command.
    let mut protocol = BufWriter::new(ProtocolChannel::claim());
    let input = std::io::stdin();
    let mut worker = Worker::default();
    let mut input = input.lock();
    let mut line = String::new();
    loop {
        // Reuse normal processing messages, but do not retain a large one-off
        // state/preset payload for the lifetime of this helper.
        if line.capacity() > 1024 * 1024 {
            line = String::new();
        } else {
            line.clear();
        }
        if input.read_line(&mut line)? == 0 {
            break;
        }
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
        write_response(&mut protocol, &response)?;
        worker.recycle(response);
    }
    Ok(())
}

fn write_response(writer: &mut impl Write, response: &HostResponse) -> Result<()> {
    serde_json::to_writer(&mut *writer, response)?;
    writeln!(writer)?;
    writer.flush()?;
    Ok(())
}

#[derive(Default)]
struct Worker {
    plugin: Option<Box<dyn NativeProcessor>>,
    sample_rate: f64,
    max_block: usize,
    block_failed: bool,
    shared_audio: Option<vst3_host::shared_audio::SharedAudioWorker>,
    shared_buffers: Option<BusAudioBuffers>,
    outputs: Vec<Vec<f32>>,
    bus_outputs: Vec<AudioBusBuffer>,
}

impl Worker {
    /// Responses own their samples during serialization; reclaim those exact
    /// allocations afterward instead of creating output storage for every block.
    fn recycle(&mut self, response: HostResponse) {
        match response {
            HostResponse::AudioOutput { outputs, .. } => self.outputs = outputs,
            HostResponse::BusAudioOutput { outputs, .. } => self.bus_outputs = outputs,
            _ => {}
        }
    }

    fn handle(&mut self, command: HostCommand) -> Result<HostResponse> {
        if matches!(command, HostCommand::ProcessingCapabilities) {
            return Ok(HostResponse::ProcessingCapabilities {
                block_processing: true,
                shared_audio: true,
            });
        }
        if matches!(command, HostCommand::ReleaseSharedAudio) {
            self.shared_audio = None;
            self.shared_buffers = None;
            return Ok(success());
        }
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
            let mut plugin = crate::native::load(LoadRequest {
                path: std::path::Path::new(&path),
                class_id: class_id.as_deref(),
                sample_rate,
                block_size: block_size as usize,
                tempo,
                time_signature: (time_sig_numerator, time_sig_denominator),
            })?;
            let info = plugin.loaded_info();
            let layout = plugin.audio_bus_layout()?;
            let (compatibility, bus_counts) = match plugin.as_vst3() {
                Some(vst3) => (
                    vst3.class_compatibility().to_vec(),
                    (
                        vst3.info().audio_inputs as i32,
                        vst3.info().audio_outputs as i32,
                        vst3.output_channel_count() as i32,
                    ),
                ),
                None => (
                    Vec::new(),
                    (
                        layout.inputs.len() as i32,
                        layout.outputs.len() as i32,
                        layout
                            .outputs
                            .iter()
                            .map(|bus| bus.channel_count)
                            .sum::<usize>()
                            .min(i32::MAX as usize) as i32,
                    ),
                ),
            };
            let response = HostResponse::PluginInfo {
                vendor: info.vendor,
                name: info.name,
                version: info.version,
                category: info.category,
                uid: info.uid,
                has_gui: false,
                audio_inputs: bus_counts.0,
                audio_outputs: bus_counts.1,
                output_channels: bus_counts.2,
                has_midi_input: info.has_midi_input,
                has_midi_output: info.has_midi_output,
                compatibility,
            };
            self.shared_audio = None;
            self.shared_buffers = None;
            self.plugin = Some(plugin);
            self.block_failed = false;
            self.outputs.clear();
            self.bus_outputs.clear();
            self.sample_rate = sample_rate;
            self.max_block = block_size as usize;
            return Ok(response);
        }
        if matches!(command, HostCommand::UnloadPlugin) {
            self.shared_audio = None;
            self.shared_buffers = None;
            self.plugin = None;
            self.outputs.clear();
            self.bus_outputs.clear();
            return Ok(success());
        }
        let plugin = self.plugin.as_mut().context("No plugin loaded")?;
        plugin.service();
        match command {
            HostCommand::ConfigureSharedAudio { descriptor } => {
                let layout = plugin.audio_bus_layout()?;
                ensure!(
                    descriptor.layout == layout,
                    "Shared audio bus layout differs from native layout"
                );
                ensure!(
                    descriptor.max_frames as usize == self.max_block,
                    "Shared audio capacity differs from native setup"
                );
                validate_audio_bus_layout(&layout, self.max_block)?;
                let shared = vst3_host::shared_audio::SharedAudioWorker::open(descriptor)?;
                let buffers = BusAudioBuffers::new(&layout, self.max_block, self.sample_rate);
                self.shared_audio = Some(shared);
                self.shared_buffers = Some(buffers);
            }
            HostCommand::ProcessBlock {
                commands,
                inputs,
                outputs,
                frames,
                shared_audio,
            } => {
                ensure!(
                    frames > 0 && frames as usize <= self.max_block,
                    "Invalid processing block size"
                );
                ensure!(
                    !self.block_failed,
                    "Native block failed; reload the processor before processing again"
                );
                ensure!(plugin.is_processing(), "Plug-in is not processing");
                let frames = frames as usize;
                let layout = plugin.audio_bus_layout()?;
                validate_audio_bus_layout(&layout, frames)?;
                ensure!(outputs == layout.outputs, "Output bus layout changed");
                vst3_host::process_isolation::validate_block_commands(&commands, frames)?;
                if let Some(block) = shared_audio {
                    ensure!(
                        inputs.is_empty(),
                        "Shared PCM block must not contain inline audio"
                    );
                    ensure!(
                        block.frames as usize == frames,
                        "Shared PCM frame count differs from block"
                    );
                    let shared = self
                        .shared_audio
                        .as_mut()
                        .context("Shared PCM is not configured")?;
                    let buffers = self
                        .shared_buffers
                        .as_mut()
                        .context("Shared PCM buffers are missing")?;
                    ensure!(
                        shared.descriptor().layout == layout,
                        "Shared PCM native layout changed"
                    );
                    for bus in buffers.inputs.iter_mut().chain(&mut buffers.outputs) {
                        crate::audio_buffers::prepare_channels(&mut bus.channels, frames);
                    }
                    buffers.block_size = frames;
                    shared.read_inputs(block, &mut buffers.inputs)?;
                    // Full shape/value admission precedes every stateful native operation.
                    let processed = (|| -> Result<()> {
                        for command in &commands {
                            apply(plugin.as_mut(), command)?;
                        }
                        plugin.process_bus_audio(buffers)?;
                        Ok(())
                    })();
                    if let Err(error) = processed {
                        self.block_failed = true;
                        return Err(error);
                    }
                    if let Err(error) = shared.write_outputs(block, &buffers.outputs) {
                        self.block_failed = true;
                        return Err(error.into());
                    }
                    return Ok(HostResponse::SharedAudioOutput {
                        block,
                        output_events: plugin.take_output_events(),
                    });
                }
                validate_input_buses(&inputs, &layout.inputs, frames)?;
                prepare_bus_outputs(&mut self.bus_outputs, &outputs, frames);
                let mut buffers = BusAudioBuffers {
                    inputs,
                    outputs: std::mem::take(&mut self.bus_outputs),
                    block_size: frames,
                    sample_rate: self.sample_rate,
                };
                let processed = (|| -> Result<()> {
                    for command in &commands {
                        apply(plugin.as_mut(), command)?;
                    }
                    plugin.process_bus_audio(&mut buffers)?;
                    Ok(())
                })();
                if let Err(error) = processed {
                    self.block_failed = true;
                    self.bus_outputs = buffers.outputs;
                    return Err(error);
                }
                return Ok(HostResponse::BusAudioOutput {
                    outputs: buffers.outputs,
                    output_events: plugin.take_output_events(),
                });
            }
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
            HostCommand::ServiceHostRequests => {
                return Ok(HostResponse::RestartFlags {
                    bits: plugin.service_host_requests()?,
                });
            }
            HostCommand::NoteExpressions { bus, channel } => {
                ensure!(
                    (0..16).contains(&channel),
                    "Invalid note-expression channel"
                );
                return Ok(HostResponse::NoteExpressions {
                    expressions: plugin.note_expressions_on(bus, channel as u8)?,
                });
            }
            HostCommand::MidiControllerAssignments { direction } => {
                return Ok(HostResponse::MidiControllerAssignments {
                    assignments: plugin.midi_controller_assignments(direction)?,
                });
            }
            HostCommand::SendMidiControllerAt {
                bus,
                channel,
                controller,
                value,
                sample_offset,
            } => plugin.send_midi_controller_at(bus, channel, controller, value, sample_offset)?,
            HostCommand::SetBusActive {
                media_type,
                direction,
                bus_index,
                active,
            } => plugin.set_bus_active(media_type, direction, bus_index, active)?,
            HostCommand::BusArrangements => {
                return Ok(HostResponse::BusArrangements {
                    arrangements: vst3(plugin.as_mut())?.bus_arrangements()?,
                });
            }
            HostCommand::SetBusArrangements { inputs, outputs } => {
                vst3(plugin.as_mut())?.set_bus_arrangements(&inputs, &outputs)?
            }
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
            HostCommand::SendMidi { event } => plugin.send_midi_event_at(event, 0)?,
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
            HostCommand::AudioBusDetails => {
                return Ok(HostResponse::AudioBusDetails {
                    details: plugin.audio_bus_details()?,
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
            HostCommand::LoadState { data, context } => match plugin.as_vst3() {
                Some(vst3) => vst3.load_state_with_context(&data, &context)?,
                None => plugin.load_state(&data)?,
            },
            HostCommand::ProcessBuses {
                inputs,
                outputs,
                frames,
            } => {
                ensure!(
                    frames > 0 && frames as usize <= self.max_block,
                    "Invalid processing block size"
                );
                ensure!(
                    !self.block_failed,
                    "Native block failed; reload the processor before processing again"
                );
                ensure!(plugin.is_processing(), "Plug-in is not processing");
                let expected = plugin.audio_bus_layout()?;
                validate_audio_bus_layout(&expected, frames as usize)?;
                ensure!(outputs == expected.outputs, "Output bus layout changed");
                validate_input_buses(&inputs, &expected.inputs, frames as usize)?;
                prepare_bus_outputs(&mut self.bus_outputs, &outputs, frames as usize);
                let mut buffers = BusAudioBuffers {
                    inputs,
                    outputs: std::mem::take(&mut self.bus_outputs),
                    block_size: frames as usize,
                    sample_rate: self.sample_rate,
                };
                if let Err(error) = plugin.process_bus_audio(&mut buffers) {
                    self.bus_outputs = buffers.outputs;
                    return Err(error.into());
                }
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
                ensure!(
                    !self.block_failed,
                    "Native block failed; reload the processor before processing again"
                );
                ensure!(plugin.is_processing(), "Plug-in is not processing");
                let plugin = vst3(plugin.as_mut())?;
                let output_channels = plugin.output_channel_count();
                ensure!(
                    inputs.len().saturating_add(output_channels)
                        <= crate::audio_buffers::MAX_AUDIO_CHANNELS,
                    "Plug-in exceeds the 256 audio channels limit"
                );
                prepare_outputs(&mut self.outputs, output_channels, frames as usize);
                let mut buffers = AudioBuffers {
                    inputs,
                    outputs: std::mem::take(&mut self.outputs),
                    block_size: frames as usize,
                    sample_rate: self.sample_rate,
                };
                if let Err(error) = plugin.process_audio(&mut buffers) {
                    self.outputs = buffers.outputs;
                    return Err(error.into());
                }
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

fn vst3(plugin: &mut dyn NativeProcessor) -> Result<&mut vst3_host::Plugin> {
    plugin
        .as_vst3()
        .context("Command is supported only for VST3 plug-ins")
}

/// `BlockCommand::apply` targets VST3 directly; route it through the format seam.
fn apply(plugin: &mut dyn NativeProcessor, command: &BlockCommand) -> Result<()> {
    match command {
        BlockCommand::SetParameter { id, value } => plugin.set_parameter(*id, *value),
        BlockCommand::SetParameterAt { id, value, offset } => {
            plugin.set_parameter_at(*id, *value, *offset)
        }
        BlockCommand::SendMidiAt {
            event,
            sample_offset,
        } => plugin.send_midi_event_at(*event, *sample_offset),
        BlockCommand::SendPluginEvent { event } => plugin.send_plugin_event(event.clone()),
        BlockCommand::SendMidiControllerAt {
            bus,
            channel,
            controller,
            value,
            sample_offset,
        } => plugin.send_midi_controller_at(*bus, *channel, *controller, *value, *sample_offset),
        BlockCommand::SetTransportPosition { position } => plugin.set_transport_position(*position),
        BlockCommand::SetTempo { bpm } => plugin.set_tempo(*bpm),
        BlockCommand::SetPlaying { playing } => plugin.set_playing(*playing),
    }
}

fn validate_input_buses(
    inputs: &[AudioBusBuffer],
    layout: &[vst3_host::AudioBusConfig],
    frames: usize,
) -> Result<()> {
    ensure!(inputs.len() == layout.len(), "Input bus layout changed");
    for (bus, config) in inputs.iter().zip(layout) {
        ensure!(
            bus.active == config.active && bus.channels.len() == config.channel_count,
            "Input bus layout changed"
        );
        ensure!(
            bus.channels.iter().all(|channel| channel.len() == frames),
            "Input bus block lengths differ"
        );
    }
    Ok(())
}

fn success() -> HostResponse {
    HostResponse::Success {
        message: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct CountingWriter {
        bytes: Vec<u8>,
        writes: usize,
        flushes: usize,
    }

    impl Write for CountingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn responses_are_buffered_flushed_and_preserve_audio_exactly() {
        let samples = vec![0.0, -0.0, 0.125, -1.0, f32::MIN_POSITIVE, f32::MAX];
        let response = HostResponse::BusAudioOutput {
            outputs: vec![AudioBusBuffer {
                active: true,
                channels: vec![samples.clone()],
            }],
            output_events: Vec::new(),
        };
        let mut direct = CountingWriter::default();
        write_response(&mut direct, &response).unwrap();
        let mut buffered = BufWriter::new(CountingWriter::default());
        write_response(&mut buffered, &response).unwrap();
        let sink = buffered.get_ref();
        assert_eq!(sink.bytes, direct.bytes);
        assert!(direct.writes > 1);
        assert_eq!(sink.writes, 1, "small replies should use one pipe write");
        assert_eq!(
            sink.flushes, 1,
            "the parent must receive each reply promptly"
        );
        assert_eq!(sink.bytes.last(), Some(&b'\n'));
        let decoded: HostResponse = serde_json::from_slice(&sink.bytes).unwrap();
        let HostResponse::BusAudioOutput { outputs, .. } = decoded else {
            panic!("wrong response");
        };
        for (actual, expected) in outputs[0].channels[0].iter().zip(samples) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn serialized_outputs_are_recycled_and_unload_releases_them() {
        let mut worker = Worker::default();
        let samples = vec![0.5; 64];
        let pointer = samples.as_ptr();
        worker.recycle(HostResponse::AudioOutput {
            outputs: vec![samples],
            output_events: Vec::new(),
        });
        assert_eq!(worker.outputs[0].as_ptr(), pointer);
        let bus = AudioBusBuffer::new(2, 64, true);
        let pointer = bus.channels[0].as_ptr();
        worker.recycle(HostResponse::BusAudioOutput {
            outputs: vec![bus],
            output_events: Vec::new(),
        });
        assert_eq!(worker.bus_outputs[0].channels[0].as_ptr(), pointer);
        worker.handle(HostCommand::UnloadPlugin).unwrap();
        assert!(worker.outputs.is_empty());
        assert!(worker.bus_outputs.is_empty());
    }

    #[test]
    fn commands_before_loading_return_error() {
        let mut worker = Worker::default();
        assert!(worker.handle(HostCommand::StartProcessing).is_err());
        assert!(worker.handle(HostCommand::UnloadPlugin).is_ok());
    }
}
