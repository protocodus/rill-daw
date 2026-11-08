//! High-resolution, format-independent musical events. UMP storage/decoding belongs
//! to the song/input layers; this adapter maps explicit musical values to VST3.
use crate::{MAX_MIDI_EVENTS_PER_BLOCK, VstInstrument};
use anyhow::{Result, ensure};
use vst3_host::{
    MidiChannel,
    midi::{MidiController, PluginEvent, PluginEventData},
};

/// Preserve the source's normalization exactly: a MIDI-1 attack is not rescaled
/// through 16 bits merely because its channel or expression data is now stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpressiveVelocity {
    Midi1(u8),
    Midi2(u16),
}
impl ExpressiveVelocity {
    fn normalized(self) -> f32 {
        match self {
            Self::Midi1(value) => f32::from(value) / 127.0,
            Self::Midi2(value) => f32::from(value) / 65535.0,
        }
    }
    fn valid(self) -> bool {
        match self {
            Self::Midi1(value) => value < 128,
            Self::Midi2(_) => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExpressiveController {
    ControlChange(u8),
    ChannelPressure,
    PitchBend,
    Registered { bank: u8, index: u8 },
    Assignable { bank: u8, index: u8 },
}
impl ExpressiveController {
    fn native(self) -> MidiController {
        match self {
            Self::ControlChange(index) => MidiController::Midi1(u16::from(index)),
            Self::ChannelPressure => MidiController::Midi1(128),
            Self::PitchBend => MidiController::Midi1(129),
            Self::Registered { bank, index } => MidiController::Registered { bank, index },
            Self::Assignable { bank, index } => MidiController::Assignable { bank, index },
        }
    }
    fn valid(self) -> bool {
        match self {
            Self::ControlChange(index) => index < 128,
            Self::Registered { bank, index } | Self::Assignable { bank, index } => {
                bank < 128 && index < 128
            }
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExpressiveMidiData {
    NoteOn {
        channel: u8,
        pitch: u8,
        velocity: ExpressiveVelocity,
        note_id: i32,
        tuning: f32,
    },
    NoteOff {
        channel: u8,
        pitch: u8,
        velocity: ExpressiveVelocity,
        note_id: i32,
        tuning: f32,
    },
    /// VST3's native poly-pressure field is f32. The song keeps its original u32
    /// precision, but this target field has the plug-in API's native precision.
    PolyPressure {
        channel: u8,
        pitch: u8,
        value: f64,
        note_id: i32,
    },
    NoteExpressionValue {
        type_id: u32,
        note_id: i32,
        value: f64,
    },
    NoteExpressionInt {
        type_id: u32,
        note_id: i32,
        value: u64,
    },
    Controller {
        channel: u8,
        controller: ExpressiveController,
        value: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpressiveMidiMessage {
    pub offset: u32,
    pub data: ExpressiveMidiData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Voice {
    pub channel: u8,
    pub pitch: u8,
    pub note_id: i32,
}

impl ExpressiveMidiMessage {
    fn validate(self, frames: usize) -> Result<()> {
        ensure!(
            (self.offset as usize) < frames,
            "Expressive MIDI sample offset is outside the block"
        );
        let channel_pitch = |channel: u8, pitch: u8| -> Result<()> {
            ensure!(
                channel < 16 && pitch < 128,
                "Invalid expressive MIDI channel or pitch"
            );
            Ok(())
        };
        let note = |id: i32| -> Result<()> {
            ensure!(
                id >= 0,
                "Expressive MIDI requires a nonnegative host note id"
            );
            Ok(())
        };
        let normalized = |value: f64| -> Result<()> {
            ensure!(
                value.is_finite() && (0.0..=1.0).contains(&value),
                "Expressive MIDI value must be finite and normalized"
            );
            Ok(())
        };
        match self.data {
            ExpressiveMidiData::NoteOn {
                channel,
                pitch,
                note_id,
                tuning,
                velocity,
                ..
            }
            | ExpressiveMidiData::NoteOff {
                channel,
                pitch,
                note_id,
                tuning,
                velocity,
                ..
            } => {
                channel_pitch(channel, pitch)?;
                note(note_id)?;
                ensure!(tuning.is_finite(), "Invalid expressive MIDI tuning");
                ensure!(velocity.valid(), "Invalid expressive MIDI velocity");
            }
            ExpressiveMidiData::PolyPressure {
                channel,
                pitch,
                note_id,
                value,
            } => {
                channel_pitch(channel, pitch)?;
                note(note_id)?;
                normalized(value)?;
            }
            ExpressiveMidiData::NoteExpressionValue {
                type_id,
                note_id,
                value,
            } => {
                note(note_id)?;
                normalized(value)?;
                ensure!(type_id != u32::MAX, "Invalid note-expression type");
            }
            ExpressiveMidiData::NoteExpressionInt {
                type_id, note_id, ..
            } => {
                note(note_id)?;
                ensure!(type_id != u32::MAX, "Invalid note-expression type");
            }
            ExpressiveMidiData::Controller {
                channel,
                controller,
                value,
            } => {
                ensure!(
                    channel < 16 && controller.valid(),
                    "Invalid expressive MIDI controller"
                );
                normalized(value)?;
            }
        }
        Ok(())
    }

    pub(crate) fn block_command(self) -> vst3_host::process_isolation::BlockCommand {
        use vst3_host::process_isolation::BlockCommand;
        let data = match self.data {
            ExpressiveMidiData::NoteOn {
                channel,
                pitch,
                velocity,
                note_id,
                tuning,
            } => PluginEventData::NoteOn {
                channel: i16::from(channel),
                pitch: i16::from(pitch),
                velocity: velocity.normalized(),
                note_id,
                tuning,
                length: 0,
            },
            ExpressiveMidiData::NoteOff {
                channel,
                pitch,
                velocity,
                note_id,
                tuning,
            } => PluginEventData::NoteOff {
                channel: i16::from(channel),
                pitch: i16::from(pitch),
                velocity: velocity.normalized(),
                note_id,
                tuning,
            },
            ExpressiveMidiData::PolyPressure {
                channel,
                pitch,
                value,
                note_id,
            } => PluginEventData::PolyPressure {
                channel: i16::from(channel),
                pitch: i16::from(pitch),
                pressure: value as f32,
                note_id,
            },
            ExpressiveMidiData::NoteExpressionValue {
                type_id,
                note_id,
                value,
            } => PluginEventData::NoteExpressionValue {
                type_id,
                note_id,
                value,
            },
            ExpressiveMidiData::NoteExpressionInt {
                type_id,
                note_id,
                value,
            } => PluginEventData::NoteExpressionIntValue {
                type_id,
                note_id,
                value,
            },
            ExpressiveMidiData::Controller {
                channel,
                controller,
                value,
            } => {
                return BlockCommand::SendMidiControllerAt {
                    bus: 0,
                    channel: MidiChannel::from_index(channel).expect("validated channel"),
                    controller: controller.native(),
                    value,
                    sample_offset: self.offset as i32,
                };
            }
        };
        BlockCommand::SendPluginEvent {
            event: PluginEvent {
                bus_index: 0,
                sample_offset: self.offset as i32,
                ppq_position: 0.0,
                flags: 0,
                data,
            },
        }
    }
}

impl VstInstrument {
    pub(crate) fn prepare_expressive(
        &mut self,
        events: &[ExpressiveMidiMessage],
        frames: usize,
    ) -> Result<()> {
        ensure!(
            events.len() <= MAX_MIDI_EVENTS_PER_BLOCK,
            "Too many expressive MIDI events in one block"
        );
        self.next_expressive_notes.clear();
        if !self.panic_pending {
            self.next_expressive_notes
                .extend_from_slice(&self.expressive_notes);
        }
        let mut previous = 0;
        for &event in events {
            event.validate(frames)?;
            ensure!(
                event.offset >= previous,
                "Expressive MIDI events must be ordered by sample offset"
            );
            previous = event.offset;
            match event.data {
                ExpressiveMidiData::NoteOn {
                    channel,
                    pitch,
                    note_id,
                    ..
                } => {
                    ensure!(
                        !self
                            .next_expressive_notes
                            .iter()
                            .any(|voice| voice.note_id == note_id),
                        "Expressive MIDI note id is already active"
                    );
                    ensure!(
                        self.next_expressive_notes.len() < MAX_MIDI_EVENTS_PER_BLOCK,
                        "Too many held expressive MIDI notes"
                    );
                    self.next_expressive_notes.push(Voice {
                        channel,
                        pitch,
                        note_id,
                    });
                }
                ExpressiveMidiData::NoteOff { note_id, .. } => {
                    if let Some(index) = self
                        .next_expressive_notes
                        .iter()
                        .position(|voice| voice.note_id == note_id)
                    {
                        self.next_expressive_notes.swap_remove(index);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Advertised dimensions on native event bus zero. An empty list means this plug-in
    /// has not advertised Note Expression; stored data still remains in the song.
    pub fn supported_note_expression_types(&self, channel: u8) -> Result<Vec<u32>> {
        Ok(self
            .plugin
            .note_expressions_on(0, channel)?
            .into_iter()
            .map(|info| info.kind.type_id())
            .collect())
    }

    /// Advertised expression defaults, queried only while preparing a graph.
    pub fn note_expression_defaults(&self, channel: u8) -> Result<Vec<(u32, f64)>> {
        Ok(self
            .plugin
            .note_expressions_on(0, channel)?
            .into_iter()
            .filter(|info| {
                info.default_value.is_finite() && (0.0..=1.0).contains(&info.default_value)
            })
            .map(|info| (info.kind.type_id(), info.default_value))
            .collect())
    }

    /// Native parameter mappings on event bus zero. No host query is needed in DSP.
    pub fn expressive_controller_assignments(&self) -> Result<Vec<(u8, ExpressiveController)>> {
        Ok(self
            .expressive_controller_defaults()?
            .into_iter()
            .map(|(channel, address, _)| (channel, address))
            .collect())
    }
    /// Current restored parameter values, captured during graph setup for seek reset.
    pub fn expressive_controller_defaults(&self) -> Result<Vec<(u8, ExpressiveController, f64)>> {
        let mut defaults = std::collections::HashMap::new();
        let mut result = Vec::new();
        for entry in self
            .plugin
            .midi_controller_assignments(vst3_host::audio::BusDirection::Input)?
        {
            if entry.bus != 0 {
                continue;
            }
            let address = match entry.controller {
                MidiController::Midi1(value) if value < 128 => {
                    ExpressiveController::ControlChange(value as u8)
                }
                MidiController::Midi1(128) => ExpressiveController::ChannelPressure,
                MidiController::Midi1(129) => ExpressiveController::PitchBend,
                MidiController::Registered { bank, index } => {
                    ExpressiveController::Registered { bank, index }
                }
                MidiController::Assignable { bank, index } => {
                    ExpressiveController::Assignable { bank, index }
                }
                _ => continue,
            };
            let value = if let Some(value) = defaults.get(&entry.parameter_id) {
                *value
            } else {
                let value = self.plugin.get_parameter(entry.parameter_id)?;
                ensure!(
                    value.is_finite() && (0.0..=1.0).contains(&value),
                    "Invalid mapped MIDI parameter value"
                );
                defaults.insert(entry.parameter_id, value);
                value
            };
            result.push((entry.channel.as_index(), address, value));
        }
        Ok(result)
    }

    /// Whether a controller has an actual high-resolution parameter mapping in this plug-in.
    pub fn supports_expressive_controller(
        &self,
        channel: u8,
        controller: ExpressiveController,
    ) -> Result<bool> {
        ensure!(
            channel < 16 && controller.valid(),
            "Invalid expressive MIDI controller"
        );
        Ok(self
            .plugin
            .midi_controller_assignments(vst3_host::audio::BusDirection::Input)?
            .iter()
            .any(|assignment| {
                assignment.bus == 0
                    && assignment.channel.as_index() == channel
                    && assignment.controller == controller.native()
            }))
    }
}
