//! Bounded live UMP voice ownership and controller chase. This runs on the render
//! worker; the OS callback only publishes fixed-size packets into MidiShared.
use super::*;
use crate::render::midi_expression;
use daw_plugins::{ExpressiveMidiData as Data, ExpressiveVelocity as Velocity};
#[derive(Clone, Copy)]
struct LiveVoice {
    group: u8,
    channel: u8,
    index: u8,
    pitch: u8,
    id: i32,
    on: Data,
    down: bool,
    attached: bool,
}
#[derive(Clone, Copy)]
struct State {
    group: u8,
    channel: u8,
    data: Data,
}
#[derive(Clone, Copy)]
struct Pending {
    group: u8,
    channel: u8,
    index: u8,
    status: u8,
    detail: u8,
    packet: UmpPacket,
}
#[derive(Clone, Copy)]
struct Release {
    group: u8,
    channel: u8,
    data: Data,
}
pub(crate) struct MidiMonitor {
    voices: Vec<LiveVoice>,
    states: Vec<State>,
    pending: Vec<Pending>,
    releases: Vec<Release>,
    reset: u64,
    next_id: i32,
    snapshot: Vec<(u64, u8, u8, Data)>,
    routed_releases: Vec<(u64, Data)>,
    unsupported: std::collections::HashSet<u64>,
}
fn id(data: Data) -> Option<i32> {
    match data {
        Data::NoteOn { note_id, .. }
        | Data::NoteOff { note_id, .. }
        | Data::PolyPressure { note_id, .. }
        | Data::NoteExpressionValue { note_id, .. }
        | Data::NoteExpressionInt { note_id, .. } => Some(note_id),
        _ => None,
    }
}
fn same_key(left: Data, right: Data) -> bool {
    match (left, right) {
        (
            Data::Controller {
                channel: a,
                controller: b,
                ..
            },
            Data::Controller {
                channel: c,
                controller: d,
                ..
            },
        ) => a == c && b == d,
        (Data::PolyPressure { note_id: a, .. }, Data::PolyPressure { note_id: b, .. }) => a == b,
        (
            Data::NoteExpressionValue {
                type_id: a,
                note_id: b,
                ..
            },
            Data::NoteExpressionValue {
                type_id: c,
                note_id: d,
                ..
            },
        )
        | (
            Data::NoteExpressionInt {
                type_id: a,
                note_id: b,
                ..
            },
            Data::NoteExpressionInt {
                type_id: c,
                note_id: d,
                ..
            },
        ) => a == c && b == d,
        _ => false,
    }
}
impl MidiMonitor {
    pub(crate) fn new() -> Self {
        Self {
            voices: Vec::with_capacity(2048),
            states: Vec::with_capacity(8192),
            pending: Vec::with_capacity(4096),
            releases: Vec::with_capacity(2048),
            reset: 0,
            next_id: 0x40000000,
            snapshot: Vec::with_capacity(16384),
            routed_releases: Vec::with_capacity(4096),
            unsupported: std::collections::HashSet::with_capacity(4096),
        }
    }
    fn reset(&mut self, shared: &MidiShared) {
        let reset = shared.reset.load(Ordering::Acquire);
        if self.reset != reset {
            self.voices.clear();
            self.states.clear();
            self.pending.clear();
            self.releases.clear();
            self.reset = reset;
            self.unsupported.clear();
        }
    }
    fn unsupported(&mut self, packet: UmpPacket) {
        if self.unsupported.len() < 4096 {
            self.unsupported
                .insert(midi_expression::unsupported_key(packet));
        }
    }
    fn state(&mut self, group: u8, channel: u8, data: Data) {
        if let Some(existing) = self.states.iter_mut().find(|state| {
            state.group == group && state.channel == channel && same_key(state.data, data)
        }) {
            existing.data = data;
        } else if self.states.len() < 8192 {
            self.states.push(State {
                group,
                channel,
                data,
            });
        }
    }
    fn release(&mut self, index: usize, velocity: Velocity) {
        let voice = self.voices.remove(index);
        self.states.retain(|state| id(state.data) != Some(voice.id));
        if self.releases.len() < 2048 {
            self.releases.push(Release {
                group: voice.group,
                channel: voice.channel,
                data: Data::NoteOff {
                    channel: voice.channel,
                    pitch: voice.pitch,
                    velocity,
                    note_id: voice.id,
                    tuning: match voice.on {
                        Data::NoteOn { tuning, .. } => tuning,
                        _ => 0.0,
                    },
                },
            });
        }
    }
    fn apply(&mut self, event: Event) {
        let packet = event.packet();
        let group = packet.group().unwrap_or(0);
        let m = packet.midi2_channel_voice();
        let legacy = packet.message_type() == 2;
        let (channel, status, index, detail, data) = if let Some(m) = m {
            (m.channel, m.status, m.index, m.detail, m.data)
        } else if legacy {
            let word = packet.words[0];
            (
                ((word >> 16) & 15) as u8,
                ((word >> 20) & 15) as u8,
                ((word >> 8) & 127) as u8,
                (word & 127) as u8,
                0,
            )
        } else {
            self.unsupported(packet);
            return;
        };
        if status == 9 && (!legacy || detail > 0) {
            if legacy
                && let Some(at) = self
                    .voices
                    .iter()
                    .position(|v| v.group == group && v.channel == channel && v.index == index)
            {
                self.release(at, Velocity::Midi1(0));
            }
            if self.voices.len() >= 2048 {
                self.unsupported(packet);
                return;
            }
            let meta = if let Some(m) = m {
                daw_core::Midi2NoteData::from_note_on(m).ok()
            } else {
                None
            };
            let pitch = meta
                .as_ref()
                .map_or(index, daw_core::Midi2NoteData::preview_pitch);
            let tuning = meta.as_ref().map_or(0.0, |meta| {
                ((meta.pitch_semitones() - f64::from(pitch)) * 100.0) as f32
            });
            let velocity = if legacy {
                Velocity::Midi1(detail)
            } else {
                Velocity::Midi2((data >> 16) as u16)
            };
            let note_id = self.next_id;
            self.next_id = if self.next_id == i32::MAX {
                0x40000000
            } else {
                self.next_id + 1
            };
            let on = Data::NoteOn {
                channel,
                pitch,
                velocity,
                note_id,
                tuning,
            };
            self.voices.push(LiveVoice {
                group,
                channel,
                index,
                pitch,
                id: note_id,
                on,
                down: true,
                attached: true,
            });
            for at in 0..self.pending.len() {
                let pending = self.pending[at];
                if pending.group == group
                    && pending.channel == channel
                    && pending.index == index
                    && let Some((_, data)) =
                        midi_expression::packet_data(pending.packet, Some((note_id, pitch)))
                {
                    self.state(group, channel, data);
                }
            }
            if let Some(meta) = meta
                && meta.attribute_type != 0
                && meta.attribute_type != 3
            {
                self.state(
                    group,
                    channel,
                    Data::NoteExpressionInt {
                        type_id: 203000 + u32::from(meta.attribute_type),
                        note_id,
                        value: u64::from(meta.attribute_data),
                    },
                );
            }
        } else if status == 8 || (legacy && status == 9 && detail == 0) {
            if let Some(at) = self.voices.iter().position(|v| {
                v.group == group && v.channel == channel && v.index == index && v.down
            }) {
                // Rich notes preserve the actual key-up edge; mapped pedal controls belong
                // to the instrument. Legacy input keeps its previous host sustain behavior.
                let sustain=legacy&&self.states.iter().any(|state|state.group==group&&matches!(state.data,Data::Controller{channel:c,controller:daw_plugins::ExpressiveController::ControlChange(64),value}if c==channel&&value>=0.5));
                if sustain {
                    self.voices[at].down = false;
                } else {
                    self.release(
                        at,
                        if legacy {
                            Velocity::Midi1(detail)
                        } else {
                            Velocity::Midi2((data >> 16) as u16)
                        },
                    );
                }
            }
        } else if !legacy && matches!(status, 0 | 1 | 6 | 10 | 15) {
            if status == 15 && detail & 1 != 0 {
                self.pending
                    .retain(|p| p.group != group || p.channel != channel || p.index != index);
            } else if status != 15 {
                let pending = Pending {
                    group,
                    channel,
                    index,
                    status,
                    detail,
                    packet,
                };
                if let Some(old) = self.pending.iter_mut().find(|p| {
                    p.group == group
                        && p.channel == channel
                        && p.index == index
                        && p.status == status
                        && p.detail == detail
                }) {
                    *old = pending;
                } else if self.pending.len() < 4096 {
                    self.pending.push(pending);
                } else {
                    self.unsupported(packet);
                }
            }
            for at in 0..self.voices.len() {
                let voice = self.voices[at];
                if voice.group == group
                    && voice.channel == channel
                    && voice.index == index
                    && voice.attached
                {
                    if let Some((_, data)) =
                        midi_expression::packet_data(packet, Some((voice.id, voice.pitch)))
                    {
                        if status == 15 && detail & 1 != 0 {
                            self.states.retain(|state| id(state.data) != Some(voice.id));
                        }
                        self.state(group, channel, data);
                    } else if status != 15 || detail & 1 != 0 {
                        self.unsupported(packet);
                    }
                    if status == 15 && detail & 2 != 0 {
                        self.voices[at].attached = false;
                    }
                }
            }
        } else if legacy && status == 10 {
            for at in 0..self.voices.len() {
                let voice = self.voices[at];
                if voice.group == group
                    && voice.channel == channel
                    && voice.index == index
                    && let Some((_, data)) =
                        midi_expression::packet_data(packet, Some((voice.id, voice.pitch)))
                {
                    self.state(group, channel, data);
                }
            }
        } else {
            if let Some((_, data)) = midi_expression::packet_data(packet, None) {
                self.state(group, channel, data);
            } else {
                self.unsupported(packet);
            }
            if status == 11
                && ((index == 64
                    && if legacy {
                        detail < 64
                    } else {
                        data < 0x80000000
                    })
                    || matches!(index, 120 | 123))
            {
                let mut at = 0;
                while at < self.voices.len() {
                    let voice = self.voices[at];
                    if voice.group == group
                        && voice.channel == channel
                        && (!voice.down || index != 64)
                    {
                        self.release(at, Velocity::Midi1(0));
                    } else {
                        at += 1;
                    }
                }
            }
        }
    }
    fn refresh(&mut self, shared: &MidiShared, renderer: &mut Renderer, offset: u32) -> Result<()> {
        self.snapshot.clear();
        self.routed_releases.clear();
        if shared.monitoring_requested() {
            for route in 0..shared.route_count.load(Ordering::Acquire).min(32) {
                if !shared.route_monitor[route].load(Ordering::Relaxed) {
                    continue;
                }
                let track = shared.route_tracks[route].load(Ordering::Relaxed);
                let group = shared.route_groups[route].load(Ordering::Relaxed) as u8;
                let channel = shared.route_channels[route].load(Ordering::Relaxed);
                for voice in &self.voices {
                    if voice.group == group
                        && (channel == 16 || channel == u32::from(voice.channel))
                        && self.snapshot.len() < 16384
                    {
                        self.snapshot.push((track, group, voice.channel, voice.on));
                    }
                }
                for state in &self.states {
                    if state.group == group
                        && (channel == 16 || channel == u32::from(state.channel))
                        && self.snapshot.len() < 16384
                    {
                        self.snapshot
                            .push((track, group, state.channel, state.data));
                    }
                }
                for release in &self.releases {
                    if release.group == group
                        && (channel == 16 || channel == u32::from(release.channel))
                        && self.routed_releases.len() < 4096
                    {
                        self.routed_releases.push((track, release.data));
                    }
                }
            }
        }
        shared.held.store(
            self.snapshot
                .iter()
                .filter(|(_, _, _, data)| matches!(data, Data::NoteOn { .. }))
                .count() as u32,
            Ordering::Relaxed,
        );
        shared
            .unsupported_input
            .store(self.unsupported.len() as u64, Ordering::Relaxed);
        renderer.set_expressive_input(&self.snapshot, &self.routed_releases, offset)?;
        self.releases.clear();
        Ok(())
    }
    pub(crate) fn update_suspended(&mut self, shared: &MidiShared) {
        self.reset(shared);
        let epoch = shared.epoch.load(Ordering::Acquire);
        for _ in 0..256 {
            let Some(event) = shared.monitoring.pop() else {
                break;
            };
            if event.epoch == epoch && shared.ready.load(Ordering::Acquire) {
                self.apply(event);
            }
        }
        self.releases.clear();
        shared.held.store(0, Ordering::Relaxed);
    }
    pub(crate) fn update(&mut self, shared: &MidiShared, renderer: &mut Renderer) -> Result<()> {
        self.reset(shared);
        self.refresh(shared, renderer, 0)?;
        let epoch = shared.epoch.load(Ordering::Acquire);
        let mut first = None;
        for _ in 0..256 {
            let Some(event) = shared.monitoring.pop() else {
                break;
            };
            if event.epoch != epoch || !shared.ready.load(Ordering::Acquire) {
                continue;
            }
            let anchor = *first.get_or_insert(event.time);
            let frames = u128::from(event.time.saturating_sub(anchor))
                * u128::from(shared.sample_rate.load(Ordering::Relaxed))
                / 1_000_000_000;
            let offset = frames.min((crate::render::BLOCK - 1) as u128) as u32;
            self.apply(event);
            self.refresh(shared, renderer, offset)?;
        }
        Ok(())
    }
}
