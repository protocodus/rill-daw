//! Ordered, bounded operations accompanying one native processing block.
use crate::{
    error::{Error, Result},
    midi::{MidiChannel, MidiController, MidiEvent, PluginEvent},
    plugin::{Plugin, TransportPosition},
};
use serde::{Deserialize, Serialize};

/// Select an isolated processing transport per instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BlockTransport {
    /// Negotiate batching and shared PCM, with lossless fallback for older helpers.
    #[default]
    Auto,
    /// Preserve one immediate exchange per operation and a legacy audio request.
    Legacy,
    /// Batch operations and inline PCM, without shared memory.
    Batched,
}

/// Counters for actual host/helper exchanges, excluding local queued operations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IpcStats {
    /// Requests serialized for dispatch, including capability/configuration requests.
    pub requests: u64,
    /// Serialized request bytes, excluding the framing newline.
    pub request_bytes: u64,
    /// Received reply bytes, including framing newlines.
    pub response_bytes: u64,
}

/// Maximum operations in one block, including panic releases and automation.
pub const MAX_BLOCK_COMMANDS: usize = 16_384;

/// One operation applied in order immediately before processing its audio block.
/// Lifecycle, state, metadata and editor operations deliberately remain separate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockCommand {
    /// Set a normalized parameter using the ordinary controller/processor path.
    SetParameter {
        /// Native parameter identifier.
        id: u32,
        /// Normalized value.
        #[serde(with = "super::lossless_f64")]
        value: f64,
    },
    /// Schedule a normalized parameter at an exact block-relative offset.
    SetParameterAt {
        /// Native parameter identifier.
        id: u32,
        /// Normalized value.
        #[serde(with = "super::lossless_f64")]
        value: f64,
        /// Block-relative sample offset.
        offset: i32,
    },
    /// Schedule ordinary MIDI without changing same-offset ordering.
    SendMidiAt {
        /// MIDI payload.
        event: MidiEvent,
        /// Block-relative sample offset.
        sample_offset: i32,
    },
    /// Schedule a high-resolution or owned native event.
    SendPluginEvent {
        /// Native event including bus and sample offset.
        event: PluginEvent,
    },
    /// Queue a mapped musical controller.
    SendMidiControllerAt {
        /// Event bus index.
        bus: i32,
        /// MIDI channel.
        channel: MidiChannel,
        /// Controller address.
        controller: MidiController,
        /// Normalized value.
        value: f64,
        /// Block-relative sample offset.
        sample_offset: i32,
    },
    /// Set the position at which this block begins.
    SetTransportPosition {
        /// Absolute sample and quarter-note position.
        position: TransportPosition,
    },
    /// Set the block's tempo.
    SetTempo {
        /// Positive finite beats per minute.
        #[serde(with = "super::lossless_f64")]
        bpm: f64,
    },
    /// Set the block's transport state.
    SetPlaying {
        /// Whether timeline playback is active.
        playing: bool,
    },
}

impl BlockCommand {
    /// Validate every value before any operation in a block is applied.
    pub fn validate(&self, frames: usize) -> Result<()> {
        let invalid = |message: &str| Error::InvalidParameter(message.into());
        let normalized = |value: f64| {
            if value.is_finite() && (0.0..=1.0).contains(&value) {
                Ok(())
            } else {
                Err(invalid("Block parameter must be finite and normalized"))
            }
        };
        let offset = |offset: i32| {
            if offset >= 0 && (offset as usize) < frames {
                Ok(())
            } else {
                Err(invalid("Block event offset is outside the audio block"))
            }
        };
        match self {
            Self::SetParameter { value, .. } => normalized(*value),
            Self::SetParameterAt {
                value,
                offset: position,
                ..
            } => {
                normalized(*value)?;
                offset(*position)
            }
            Self::SendMidiAt {
                event,
                sample_offset,
            } => {
                crate::plugin::validate_midi_event(event)?;
                offset(*sample_offset)
            }
            Self::SendPluginEvent { event } => {
                crate::plugin::validate_plugin_event(event)?;
                offset(event.sample_offset)
            }
            Self::SendMidiControllerAt {
                bus,
                controller,
                value,
                sample_offset,
                ..
            } => {
                if *bus < 0 || !controller.is_valid() {
                    return Err(invalid("Invalid block controller address"));
                }
                normalized(*value)?;
                offset(*sample_offset)
            }
            Self::SetTransportPosition { position } => {
                if position.quarter_notes.is_finite() {
                    Ok(())
                } else {
                    Err(invalid("Invalid block transport position"))
                }
            }
            Self::SetTempo { bpm } => {
                if bpm.is_finite() && *bpm > 0.0 {
                    Ok(())
                } else {
                    Err(invalid("Invalid block tempo"))
                }
            }
            Self::SetPlaying { .. } => Ok(()),
        }
    }

    /// Apply an admitted operation on the helper's owning thread.
    pub fn apply(&self, plugin: &mut Plugin) -> Result<()> {
        match self {
            Self::SetParameter { id, value } => plugin.set_parameter(*id, *value),
            Self::SetParameterAt { id, value, offset } => {
                plugin.set_parameter_at(*id, *value, *offset)
            }
            Self::SendMidiAt {
                event,
                sample_offset,
            } => plugin.send_midi_event_at(*event, *sample_offset),
            Self::SendPluginEvent { event } => plugin.send_plugin_event(event.clone()),
            Self::SendMidiControllerAt {
                bus,
                channel,
                controller,
                value,
                sample_offset,
            } => {
                plugin.send_midi_controller_at(*bus, *channel, *controller, *value, *sample_offset)
            }
            Self::SetTransportPosition { position } => plugin.set_transport_position(*position),
            Self::SetTempo { bpm } => plugin.set_tempo(*bpm),
            Self::SetPlaying { playing } => plugin.set_playing(*playing),
        }
    }

    pub(crate) fn into_command(self) -> super::HostCommand {
        match self {
            Self::SetParameter { id, value } => super::HostCommand::SetParameter { id, value },
            Self::SetParameterAt { id, value, offset } => {
                super::HostCommand::SetParameterAt { id, value, offset }
            }
            Self::SendMidiAt {
                event,
                sample_offset,
            } => super::HostCommand::SendMidiAt {
                event,
                sample_offset,
            },
            Self::SendPluginEvent { event } => super::HostCommand::SendPluginEvent { event },
            Self::SendMidiControllerAt {
                bus,
                channel,
                controller,
                value,
                sample_offset,
            } => super::HostCommand::SendMidiControllerAt {
                bus,
                channel,
                controller,
                value,
                sample_offset,
            },
            Self::SetTransportPosition { position } => {
                super::HostCommand::SetTransportPosition { position }
            }
            Self::SetTempo { bpm } => super::HostCommand::SetTempo { bpm },
            Self::SetPlaying { playing } => super::HostCommand::SetPlaying { playing },
        }
    }
}

/// Validate the complete batch before dispatching any side effects.
pub fn validate_block_commands(commands: &[BlockCommand], frames: usize) -> Result<()> {
    validate_command_limits(commands)?;
    for command in commands {
        command.validate(frames)?;
    }
    Ok(())
}

pub(super) fn validate_command_limits(commands: &[BlockCommand]) -> Result<()> {
    if commands.len() > MAX_BLOCK_COMMANDS {
        return Err(Error::InvalidParameter(
            "Too many processing block operations".into(),
        ));
    }
    let mut payload_bytes = 0usize;
    let mut parameter_points = 0usize;
    for command in commands {
        if matches!(
            command,
            BlockCommand::SetParameter { .. } | BlockCommand::SetParameterAt { .. }
        ) {
            parameter_points += 1;
            if parameter_points > crate::parameters::MAX_PENDING_PARAM_CHANGES {
                return Err(Error::InvalidParameter(
                    "Block exceeds native parameter queue capacity".into(),
                ));
            }
        }
        if let BlockCommand::SendPluginEvent { event } = command {
            payload_bytes = payload_bytes.saturating_add(event.payload_bytes());
            if payload_bytes > crate::midi::MAX_QUEUED_EVENT_PAYLOAD_BYTES {
                return Err(Error::InvalidParameter(
                    "Block event payload exceeds native queue limit".into(),
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn deserialize_commands<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<BlockCommand>, D::Error> {
    struct Commands;
    impl<'de> serde::de::Visitor<'de> for Commands {
        type Value = Vec<BlockCommand>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                formatter,
                "at most {MAX_BLOCK_COMMANDS} processing operations"
            )
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut commands =
                Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(MAX_BLOCK_COMMANDS));
            let mut payload_bytes = 0usize;
            while let Some(command) = sequence.next_element::<BlockCommand>()? {
                if commands.len() == MAX_BLOCK_COMMANDS {
                    return Err(serde::de::Error::custom(
                        "Too many processing block operations",
                    ));
                }
                if let BlockCommand::SendPluginEvent { event } = &command {
                    payload_bytes = payload_bytes.saturating_add(event.payload_bytes());
                    if payload_bytes > crate::midi::MAX_QUEUED_EVENT_PAYLOAD_BYTES {
                        return Err(serde::de::Error::custom(
                            "Block event payload exceeds native queue limit",
                        ));
                    }
                }
                commands.push(command);
            }
            Ok(commands)
        }
    }
    deserializer.deserialize_seq(Commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_isolation::{is_slow_command, HostCommand};

    #[test]
    fn wire_preserves_parameter_bits_and_operation_order() {
        for value in [
            0.0,
            -0.0,
            f64::MIN_POSITIVE,
            f64::from_bits(0x3fd0000000000001),
            1.0,
        ] {
            let request = HostCommand::ProcessBlock {
                commands: vec![
                    BlockCommand::SetParameter { id: 7, value },
                    BlockCommand::SetParameterAt {
                        id: 8,
                        value,
                        offset: 3,
                    },
                    BlockCommand::SetTempo { bpm: value },
                ],
                inputs: vec![],
                outputs: vec![],
                frames: 8,
                shared_audio: None,
            };
            assert!(!is_slow_command(&request));
            let decoded: HostCommand =
                serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
            let HostCommand::ProcessBlock { commands, .. } = decoded else {
                panic!("wrong request");
            };
            assert!(
                matches!(commands[0], BlockCommand::SetParameter { id: 7, value: actual } if actual.to_bits() == value.to_bits())
            );
            assert!(
                matches!(commands[1], BlockCommand::SetParameterAt { id: 8, value: actual, offset: 3 } if actual.to_bits() == value.to_bits())
            );
            assert!(
                matches!(commands[2], BlockCommand::SetTempo { bpm: actual } if actual.to_bits() == value.to_bits())
            );
        }
        assert!(is_slow_command(&HostCommand::ReleaseSharedAudio));
    }

    #[test]
    fn explicit_parameter_points_cannot_overflow_the_native_queue() {
        let mut commands = vec![
            BlockCommand::SetParameter { id: 7, value: 0.5 };
            crate::parameters::MAX_PENDING_PARAM_CHANGES
        ];
        assert!(validate_block_commands(&commands, 8).is_ok());
        commands.push(BlockCommand::SetParameterAt {
            id: 7,
            value: 0.75,
            offset: 1,
        });
        assert!(validate_block_commands(&commands, 8).is_err());
    }

    #[test]
    fn aggregate_owned_event_payload_is_bounded_before_dispatch_and_while_decoding() {
        let event = PluginEvent::sysex(vec![0x7f; crate::midi::MAX_EVENT_PAYLOAD_BYTES]);
        let mut commands = vec![BlockCommand::SendPluginEvent { event }; 8];
        assert!(validate_block_commands(&commands, 8).is_ok());
        commands.push(BlockCommand::SendPluginEvent {
            event: PluginEvent::sysex(vec![1]),
        });
        assert!(validate_block_commands(&commands, 8).is_err());
        let request = HostCommand::ProcessBlock {
            commands,
            inputs: vec![],
            outputs: vec![],
            frames: 8,
            shared_audio: None,
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(serde_json::from_str::<HostCommand>(&encoded)
            .unwrap_err()
            .to_string()
            .contains("payload exceeds"));
    }

    #[test]
    fn complete_batch_admission_rejects_invalid_offsets_values_and_oversize() {
        let note = |sample_offset| BlockCommand::SendMidiAt {
            event: MidiEvent::NoteOn {
                channel: MidiChannel::Ch1,
                note: 60,
                velocity: 100,
            },
            sample_offset,
        };
        assert!(validate_block_commands(&[note(0), note(7)], 8).is_ok());
        assert!(validate_block_commands(&[note(0), note(8)], 8).is_err());
        assert!(validate_block_commands(&[note(-1)], 8).is_err());
        assert!(validate_block_commands(&[BlockCommand::SetTempo { bpm: f64::NAN }], 8).is_err());
        assert!(validate_block_commands(
            &[BlockCommand::SetParameter {
                id: 7,
                value: f64::INFINITY
            }],
            8
        )
        .is_err());
        assert!(validate_block_commands(
            &[BlockCommand::SetTransportPosition {
                position: TransportPosition {
                    samples: -512,
                    quarter_notes: -0.25
                }
            }],
            8
        )
        .is_ok());
        let operations = vec![BlockCommand::SetPlaying { playing: true }; MAX_BLOCK_COMMANDS];
        assert!(validate_block_commands(&operations, 8).is_ok());
        let mut too_many = operations;
        too_many.push(note(0));
        assert!(validate_block_commands(&too_many, 8).is_err());
        let request = HostCommand::ProcessBlock {
            commands: too_many,
            inputs: vec![],
            outputs: vec![],
            frames: 8,
            shared_audio: None,
        };
        assert!(
            serde_json::from_str::<HostCommand>(&serde_json::to_string(&request).unwrap()).is_err()
        );
    }
}
