//! Precise musical scheduling alongside the unchanged seven-bit note path.
//! Song UMPs remain portable; only explicitly mapped VST3 constructs are emitted.
use super::*;
use daw_core::{Midi2NoteData, Track, UmpPacket};
use daw_plugins::{
    ExpressiveController as Controller, ExpressiveMidiData as Data,
    ExpressiveMidiMessage as Message, ExpressiveVelocity as Velocity,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreciseNote {
    pub channel: u8,
    pub note_id: i32,
    pub velocity: Velocity,
    pub release: u16,
    pub tuning: f32,
}
impl PreciseNote {
    pub fn data(self, pitch: u8, on: bool) -> Data {
        if on {
            Data::NoteOn {
                channel: self.channel,
                pitch,
                velocity: self.velocity,
                note_id: self.note_id,
                tuning: self.tuning,
            }
        } else {
            Data::NoteOff {
                channel: self.channel,
                pitch,
                velocity: Velocity::Midi2(self.release),
                note_id: self.note_id,
                tuning: self.tuning,
            }
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct Timed {
    pub sample: u64,
    pub data: Data,
    pub channel: u8,
}
#[derive(Clone)]
struct State {
    events: Vec<Timed>,
    range: Option<(u64, u64)>,
}
#[derive(Clone, Default)]
pub(super) struct Content {
    pub events: Vec<Timed>,
    states: Vec<State>,
    pub group: Option<u8>,
    pub unsupported: usize,
    pub unsupported_groups: usize,
}
#[derive(Default)]
pub(super) struct Builder {
    pub events: Vec<(Timed, Option<(u64, u64)>)>,
    pub unsupported: HashSet<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key(u8, u8, u32, i32);
fn key(data: Data) -> Key {
    match data {
        Data::Controller {
            channel,
            controller,
            ..
        } => Key(0, channel, controller_key(controller), 0),
        Data::PolyPressure {
            channel, note_id, ..
        } => Key(1, channel, 0, note_id),
        Data::NoteExpressionValue {
            type_id, note_id, ..
        }
        | Data::NoteExpressionInt {
            type_id, note_id, ..
        } => Key(2, 0, type_id, note_id),
        Data::NoteOn {
            channel, note_id, ..
        }
        | Data::NoteOff {
            channel, note_id, ..
        } => Key(3, channel, 0, note_id),
    }
}
fn controller_key(c: Controller) -> u32 {
    match c {
        Controller::ControlChange(v) => u32::from(v),
        Controller::ChannelPressure => 128,
        Controller::PitchBend => 129,
        Controller::Registered { bank, index } => 0x10000 | u32::from(bank) << 8 | u32::from(index),
        Controller::Assignable { bank, index } => 0x20000 | u32::from(bank) << 8 | u32::from(index),
    }
}
pub(super) fn controller_state_key(packet: UmpPacket) -> Option<(u8, u32)> {
    match packet_data(packet, None)?.1 {
        Data::Controller {
            channel,
            controller,
            ..
        } => Some((channel, controller_key(controller))),
        _ => None,
    }
}
pub(crate) fn unsupported_key(packet: UmpPacket) -> u64 {
    let word = packet.words[0];
    u64::from(word & 0xffff_ff00)
}
pub(super) fn stored_group(track: &Track) -> Option<u8> {
    let mut earliest: Option<(u64, u8)> = None;
    for clip in &track.clips {
        if clip.muted || clip.is_take_source() {
            continue;
        }
        if let ClipContent::Midi { notes } = &clip.content {
            for note in notes {
                if let Some(data) = &note.midi2 {
                    let start = clip.source_to_local(note.start);
                    let end = start + note.length / clip.source_rate;
                    if start < clip.length && end > 0.0 {
                        let candidate = ((clip.start + start.max(0.0)).to_bits(), data.group);
                        if earliest.is_none_or(|old| candidate < old) {
                            earliest = Some(candidate);
                        }
                    }
                }
            }
        }
        for event in &clip.midi_events {
            let local = clip.source_to_local(event.beat);
            if (0.0..clip.length).contains(&local)
                && let Some(group) = event.packet.group()
            {
                let candidate = ((clip.start + local).to_bits(), group);
                if earliest.is_none_or(|old| candidate < old) {
                    earliest = Some(candidate);
                }
            }
        }
    }
    earliest.map(|(_, group)| group)
}
/// Absolute note pitch is carried by the VST note and its cent tuning. The wire
/// note index remains the address used to attach UMP per-note controls in the song.
pub(crate) fn precise(data: &Midi2NoteData, velocity: u8, note_id: i32) -> PreciseNote {
    PreciseNote {
        channel: data.channel,
        note_id,
        velocity: if data.protocol == 1 {
            Velocity::Midi1(velocity)
        } else {
            Velocity::Midi2(data.velocity)
        },
        release: data.release_velocity,
        tuning: ((data.pitch_semitones() - f64::from(data.preview_pitch())) * 100.0) as f32,
    }
}
/// Decode supported non-note channel voice messages. None is an explicit capability gap.
pub(crate) fn packet_data(packet: UmpPacket, note: Option<(i32, u8)>) -> Option<(u8, Data)> {
    if let Some(m) = packet.midi2_channel_voice() {
        let value = f64::from(m.data) / f64::from(u32::MAX);
        let controller = |controller| {
            Some((
                m.channel,
                Data::Controller {
                    channel: m.channel,
                    controller,
                    value,
                },
            ))
        };
        return match m.status {
            0 | 1 => {
                let (note_id, _) = note?;
                Some((
                    m.channel,
                    Data::NoteExpressionValue {
                        type_id: if m.status == 0 { 202000 } else { 201000 } + u32::from(m.detail),
                        note_id,
                        value,
                    },
                ))
            }
            15 if m.detail & 1 != 0 => {
                let (note_id, pitch) = note?;
                Some((
                    m.channel,
                    Data::NoteExpressionInt {
                        type_id: u32::MAX,
                        note_id,
                        value: u64::from(pitch),
                    },
                ))
            }
            10 => {
                let (note_id, pitch) = note?;
                Some((
                    m.channel,
                    Data::PolyPressure {
                        channel: m.channel,
                        pitch,
                        note_id,
                        value,
                    },
                ))
            }
            2 => controller(Controller::Registered {
                bank: m.index,
                index: m.detail,
            }),
            3 => controller(Controller::Assignable {
                bank: m.index,
                index: m.detail,
            }),
            11 => controller(Controller::ControlChange(m.index)),
            13 => controller(Controller::ChannelPressure),
            14 => controller(Controller::PitchBend),
            _ => None,
        };
    }
    if packet.message_type() == 2 {
        let word = packet.words[0];
        let channel = ((word >> 16) & 15) as u8;
        let status = (word >> 20) & 15;
        let index = ((word >> 8) & 127) as u8;
        let value = (word & 127) as u8;
        let (controller, value) = match status {
            11 => (Controller::ControlChange(index), f64::from(value) / 127.0),
            13 => (Controller::ChannelPressure, f64::from(index) / 127.0),
            14 => (
                Controller::PitchBend,
                f64::from(u16::from(index) | (u16::from(value) << 7)) / 16383.0,
            ),
            10 => {
                let (note_id, pitch) = note?;
                return Some((
                    channel,
                    Data::PolyPressure {
                        channel,
                        pitch,
                        note_id,
                        value: f64::from(value) / 127.0,
                    },
                ));
            }
            _ => return None,
        };
        return Some((
            channel,
            Data::Controller {
                channel,
                controller,
                value,
            },
        ));
    }
    None
}
impl Builder {
    pub fn add(
        &mut self,
        sample: u64,
        packet: UmpPacket,
        note: Option<(i32, u8)>,
        range: Option<(u64, u64)>,
    ) {
        if packet
            .midi2_channel_voice()
            .is_some_and(|message| message.status == 15 && message.detail & 1 == 0)
        {
            return;
        }
        if let Some((channel, data)) = packet_data(packet, note) {
            self.events.push((
                Timed {
                    sample,
                    data,
                    channel,
                },
                range,
            ));
        } else {
            self.unsupported.insert(unsupported_key(packet));
        }
    }
    pub fn finish(mut self, group: Option<u8>) -> Content {
        self.events.sort_by_key(|(event, _)| event.sample);
        let mut indices = HashMap::new();
        let mut states: Vec<State> = Vec::new();
        for &(event, range) in &self.events {
            let next = states.len();
            let index = *indices.entry(key(event.data)).or_insert_with(|| {
                states.push(State {
                    events: Vec::new(),
                    range,
                });
                next
            });
            states[index].events.push(event);
        }
        Content {
            events: self.events.into_iter().map(|(event, _)| event).collect(),
            states,
            group,
            unsupported: self
                .unsupported
                .iter()
                .filter(|key| **key < 0x100000000)
                .count(),
            unsupported_groups: self
                .unsupported
                .iter()
                .filter(|key| **key >= 0x100000000)
                .count(),
        }
    }
}
#[derive(Default)]
pub(super) struct Support {
    expressions: [Vec<(u32, f64)>; 16],
    controllers: HashSet<(u8, Controller)>,
    defaults: Vec<(u8, Controller, f64)>,
}
impl Support {
    pub fn query(plugin: &VstInstrument) -> Result<Self> {
        let mut support = Self::default();
        for channel in 0..16 {
            support.expressions[channel] = plugin.note_expression_defaults(channel as u8)?;
        }
        support.defaults = plugin.expressive_controller_defaults()?;
        support.controllers = support
            .defaults
            .iter()
            .map(|(channel, address, _)| (*channel, *address))
            .collect();
        Ok(support)
    }
    /// Built-in instruments take pitch bend, mod wheel and sustain on every
    /// channel.
    pub fn builtin() -> Self {
        let mut support = Self::default();
        for channel in 0..16 {
            for (controller, value) in crate::builtin::INSTRUMENT_CONTROLLERS {
                support.defaults.push((channel, controller, value));
                support.controllers.insert((channel, controller));
            }
        }
        support
    }
    pub fn controller_default(&self, channel: u8, address: Controller) -> Option<f64> {
        self.defaults.iter().find_map(|(ch, controller, value)| {
            if *ch == channel && *controller == address {
                Some(*value)
            } else {
                None
            }
        })
    }
    pub fn supports(&self, channel: u8, data: Data) -> bool {
        match data {
            Data::Controller {
                channel,
                controller,
                ..
            } => self.controllers.contains(&(channel, controller)),
            Data::NoteExpressionValue { type_id, .. } | Data::NoteExpressionInt { type_id, .. } => {
                self.expressions[channel as usize]
                    .iter()
                    .any(|(id, _)| *id == type_id)
            }
            _ => true,
        }
    }
}
pub(super) struct Runtime {
    pub content: Arc<Content>,
    pub support: Support,
    pub messages: Vec<Message>,
    pub live: Vec<(u8, Message)>,
    cursor: usize,
    pub chase: Vec<Timed>,
    pub delay: u32,
    unsupported: HashSet<Key>,
    pub group_mismatches: u64,
    touched_controllers: HashSet<(u8, Controller)>,
}
impl Runtime {
    pub fn new(content: Arc<Content>, support: Support) -> Self {
        let capacity = content.events.len() + 8192;
        Self {
            content,
            support,
            messages: Vec::with_capacity(capacity),
            live: Vec::with_capacity(4096),
            cursor: 0,
            chase: Vec::with_capacity(capacity),
            delay: 0,
            unsupported: HashSet::with_capacity(8192),
            group_mismatches: 0,
            touched_controllers: HashSet::with_capacity(8192),
        }
    }
    pub fn unsupported_count(&self) -> u64 {
        self.content.unsupported as u64 + self.unsupported.len() as u64
    }
    pub fn reject_group(&mut self, group: u8) {
        self.group_mismatches |= 1u64 << group;
    }
    pub fn group_count(&self) -> u64 {
        self.content.unsupported_groups as u64 + u64::from(self.group_mismatches.count_ones())
    }
    pub fn push(&mut self, channel: u8, message: Message) {
        if self.messages.len() >= 2049 {
            return;
        }
        if let Data::NoteExpressionInt {
            type_id: u32::MAX,
            note_id,
            value: pitch,
        } = message.data
        {
            for index in 0..self.support.expressions[channel as usize].len() {
                let (type_id, value) = self.support.expressions[channel as usize][index];
                if type_id == 2 || (201000..=202255).contains(&type_id) {
                    if self.messages.len() >= 2049 {
                        break;
                    }
                    self.messages.push(Message {
                        offset: message.offset,
                        data: Data::NoteExpressionValue {
                            type_id,
                            note_id,
                            value,
                        },
                    });
                }
            }
            self.messages.push(Message {
                offset: message.offset,
                data: Data::PolyPressure {
                    channel,
                    pitch: pitch as u8,
                    note_id,
                    value: 0.0,
                },
            });
            return;
        }
        if self.support.supports(channel, message.data) {
            if let Data::Controller {
                channel,
                controller,
                ..
            } = message.data
                && self.touched_controllers.len() < 4096
            {
                self.touched_controllers.insert((channel, controller));
            }
            self.messages.push(message);
        } else if self.unsupported.len() < 4096 {
            let mut k = key(message.data);
            k.3 = 0;
            k.1 = channel;
            self.unsupported.insert(k);
        }
    }
    pub fn seek(&mut self, sample: u64, delay: u32, clear_live: bool) {
        self.cursor = self
            .content
            .events
            .partition_point(|event| event.sample < sample);
        self.delay = delay;
        self.chase.clear();
        for &(channel, controller, value) in &self.support.defaults {
            if self.touched_controllers.contains(&(channel, controller)) {
                self.chase.push(Timed {
                    sample: 0,
                    channel,
                    data: Data::Controller {
                        channel,
                        controller,
                        value,
                    },
                });
            }
        }
        for state in &self.content.states {
            if state
                .range
                .is_some_and(|(start, end)| sample <= start || sample >= end)
            {
                continue;
            }
            let at = state.events.partition_point(|event| event.sample < sample);
            if at > 0 {
                self.chase.push(state.events[at - 1]);
            }
        }
        for index in 1..self.chase.len() {
            let sample = self.chase[index].sample;
            if self.chase[index - 1].sample > sample {
                let at = self.chase[..index].partition_point(|event| event.sample <= sample);
                self.chase[at..=index].rotate_right(1);
            }
        }
        if clear_live {
            self.live.clear();
        }
    }
    pub fn collect(&mut self, context: BlockContext, latency: u32) {
        for index in 1..self.live.len() {
            let offset = self.live[index].1.offset;
            if self.live[index - 1].1.offset > offset {
                let at =
                    self.live[..index].partition_point(|(_, message)| message.offset <= offset);
                self.live[at..=index].rotate_right(1);
            }
        }
        if context.timeline && self.delay < context.count as u32 {
            for index in 0..self.chase.len() {
                let event = self.chase[index];
                self.push(
                    event.channel,
                    Message {
                        offset: self.delay,
                        data: event.data,
                    },
                );
            }
            self.chase.clear();
        }
        self.delay = self.delay.saturating_sub(context.count as u32);
        while context.timeline
            && let Some(event) = self.content.events.get(self.cursor).copied()
        {
            let sample = event.sample.saturating_add(u64::from(latency));
            if sample >= context.start + context.count as u64 {
                break;
            }
            self.push(
                event.channel,
                Message {
                    offset: sample.saturating_sub(context.start) as u32,
                    data: event.data,
                },
            );
            self.cursor += 1;
        }
        if context.live_input {
            let count = self
                .live
                .partition_point(|(_, event)| event.offset < context.count as u32);
            for index in 0..count {
                let (channel, message) = self.live[index];
                self.push(channel, message);
            }
            self.live.drain(..count);
            for (_, event) in &mut self.live {
                event.offset -= context.count as u32;
            }
        }
    }
}
pub(super) fn sort_messages(messages: &mut [Message]) {
    // Preserve causal arrival order when several input edges share a sample.
    // Sorting all note-offs ahead of note-ons can release an ID before its own
    // start, leaving a rapid/coalesced retrigger permanently sounding.
    for index in 1..messages.len() {
        let offset = messages[index].offset;
        if messages[index - 1].offset > offset {
            let before = messages[..index].partition_point(|message| message.offset <= offset);
            messages[before..=index].rotate_right(1);
        }
    }
}
