//! Lossless UMP containers and editable MIDI 2.0 musical data.
//!
//! Packet sizing follows M2-104-UM v1.1.2. Reserved payload bits are retained:
//! receivers must not assume they are zero. Stored packets do not imply that a
//! particular plug-in or device supports every UMP message family.
use crate::{MAX_BEAT, Note, Project, new_id};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_MIDI_EVENTS: usize = 262_144;
pub const MAX_NOTE_EXPRESSIONS: usize = 32_768;
fn midi2_protocol() -> u8 {
    2
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmpPacket {
    pub words: [u32; 4],
    pub word_count: u8,
}

impl UmpPacket {
    pub fn from_words(words: &[u32]) -> Result<Self> {
        ensure!(
            !words.is_empty() && words.len() <= 4,
            "Invalid UMP packet length"
        );
        let mut packet = Self {
            words: [0; 4],
            word_count: words.len() as u8,
        };
        packet.words[..words.len()].copy_from_slice(words);
        packet.validate()?;
        Ok(packet)
    }

    pub const fn words_for_message_type(message_type: u8) -> usize {
        [1, 1, 1, 2, 2, 4, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4][(message_type & 15) as usize]
    }

    pub fn validate(self) -> Result<()> {
        let count = usize::from(self.word_count);
        ensure!(
            (1..=4).contains(&count)
                && count == Self::words_for_message_type(self.message_type())
                && self.words[count..].iter().all(|word| *word == 0),
            "Malformed UMP packet"
        );
        Ok(())
    }

    pub const fn message_type(self) -> u8 {
        (self.words[0] >> 28) as u8
    }

    pub const fn group(self) -> Option<u8> {
        match self.message_type() {
            1..=5 | 13 => Some(((self.words[0] >> 24) & 15) as u8),
            _ => None,
        }
    }

    pub fn midi2_channel_voice(self) -> Option<Midi2ChannelVoice> {
        (self.message_type() == 4 && self.word_count == 2).then_some(Midi2ChannelVoice {
            group: ((self.words[0] >> 24) & 15) as u8,
            channel: ((self.words[0] >> 16) & 15) as u8,
            status: ((self.words[0] >> 20) & 15) as u8,
            index: (self.words[0] >> 8) as u8,
            detail: self.words[0] as u8,
            data: self.words[1],
        })
    }

    pub fn is_note_message(self) -> bool {
        matches!(self.message_type(), 2 | 4) && matches!((self.words[0] >> 20) & 15, 8 | 9)
    }

    pub fn is_per_note(self) -> bool {
        self.midi2_channel_voice()
            .is_some_and(|message| matches!(message.status, 0 | 1 | 6 | 10 | 15))
    }

    pub fn with_note_address(mut self, group: u8, channel: u8, index: u8) -> Self {
        if self.is_per_note() {
            self.words[0] = (self.words[0] & 0xf0f0_80ff)
                | (u32::from(group & 15) << 24)
                | (u32::from(channel & 15) << 16)
                | (u32::from(index & 127) << 8);
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Midi2ChannelVoice {
    pub group: u8,
    pub channel: u8,
    pub status: u8,
    pub index: u8,
    pub detail: u8,
    pub data: u32,
}

impl Midi2ChannelVoice {
    pub fn velocity16(self) -> Option<u16> {
        matches!(self.status, 8 | 9).then_some((self.data >> 16) as u16)
    }
    pub fn attribute_data(self) -> Option<u16> {
        matches!(self.status, 8 | 9).then_some(self.data as u16)
    }
    pub fn packet(self) -> Result<UmpPacket> {
        ensure!(
            self.group < 16 && self.channel < 16 && self.status < 16,
            "Invalid MIDI 2.0 channel voice address"
        );
        UmpPacket::from_words(&[
            0x4000_0000
                | (u32::from(self.group) << 24)
                | (u32::from(self.status) << 20)
                | (u32::from(self.channel) << 16)
                | (u32::from(self.index) << 8)
                | u32::from(self.detail),
            self.data,
        ])
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MidiClipEvent {
    pub id: u64,
    /// Absolute source beat within the clip, like Note.start.
    pub beat: f64,
    pub packet: UmpPacket,
}

impl MidiClipEvent {
    pub fn new(beat: f64, packet: UmpPacket) -> Self {
        Self {
            id: new_id(),
            beat,
            packet,
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_event(self.id, self.beat, self.packet)?;
        ensure!(
            !self.packet.is_note_message() && !self.packet.is_per_note(),
            "Notes and per-note expressions belong to the editable note model"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MidiNoteExpression {
    pub id: u64,
    /// Relative source beats from the owning note's start.
    pub beat: f64,
    pub packet: UmpPacket,
}

impl MidiNoteExpression {
    pub fn new(beat: f64, packet: UmpPacket) -> Self {
        Self {
            id: new_id(),
            beat,
            packet,
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_event(self.id, self.beat, self.packet)?;
        ensure!(
            self.packet.is_per_note(),
            "Expected a MIDI 2.0 per-note expression packet"
        );
        Ok(())
    }
}

fn validate_event(id: u64, beat: f64, packet: UmpPacket) -> Result<()> {
    ensure!(id > 0 && id < u64::MAX, "Invalid MIDI event identity");
    ensure!(
        beat.is_finite() && (0.0..=MAX_BEAT).contains(&beat),
        "Invalid MIDI source beat"
    );
    packet.validate()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Midi2NoteData {
    /// Attack velocity encoding: 1 retains exact legacy u7/127 playback, 2 uses
    /// the full u16 range. Editing pitch/release/expression does not alter it.
    #[serde(default = "midi2_protocol")]
    pub protocol: u8,
    pub group: u8,
    pub channel: u8,
    pub note_index: u8,
    pub velocity: u16,
    pub release_velocity: u16,
    pub attribute_type: u8,
    pub attribute_data: u16,
    pub release_attribute_type: u8,
    pub release_attribute_data: u16,
    #[serde(default)]
    pub expressions: Vec<MidiNoteExpression>,
}

impl Midi2NoteData {
    pub fn from_note_on(message: Midi2ChannelVoice) -> Result<Self> {
        ensure!(message.status == 9, "Expected MIDI 2.0 Note On");
        let data = Self {
            protocol: 2,
            group: message.group,
            channel: message.channel,
            note_index: message.index & 127,
            velocity: (message.data >> 16) as u16,
            release_velocity: 0,
            attribute_type: message.detail,
            attribute_data: message.data as u16,
            release_attribute_type: 0,
            release_attribute_data: 0,
            expressions: Vec::new(),
        };
        data.validate()?;
        Ok(data)
    }

    pub fn from_legacy(note: &Note) -> Self {
        Self {
            protocol: 1,
            group: 0,
            channel: 0,
            note_index: note.pitch,
            velocity: midi1_velocity_to_u16(note.velocity),
            release_velocity: 0,
            attribute_type: 0,
            attribute_data: 0,
            release_attribute_type: 0,
            release_attribute_data: 0,
            expressions: Vec::new(),
        }
    }

    pub fn preview_pitch(&self) -> u8 {
        if self.attribute_type == 3 {
            (self.attribute_data >> 9) as u8
        } else {
            self.note_index
        }
    }
    pub fn preview_velocity(&self) -> u8 {
        midi2_velocity_to_u7(self.velocity)
    }
    pub fn pitch_semitones(&self) -> f64 {
        if self.attribute_type == 3 {
            f64::from(self.attribute_data) / 512.0
        } else {
            f64::from(self.note_index)
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.protocol, 1 | 2)
                && self.group < 16
                && self.channel < 16
                && self.note_index < 128,
            "Invalid MIDI 2.0 note address"
        );
        ensure!(
            self.protocol != 1 || self.velocity == midi1_velocity_to_u16(self.preview_velocity()),
            "Legacy attack velocity contains unrepresentable high-resolution data"
        );
        ensure!(
            self.expressions.len() <= MAX_NOTE_EXPRESSIONS,
            "Too many per-note expressions"
        );
        let mut ids = std::collections::HashSet::new();
        for point in &self.expressions {
            point.validate()?;
            ensure!(
                ids.insert(point.id),
                "Duplicate per-note expression identity"
            );
            let event = point
                .packet
                .midi2_channel_voice()
                .expect("validated per-note packet");
            ensure!(
                event.group == self.group
                    && event.channel == self.channel
                    && event.index & 127 == self.note_index,
                "Per-note expression address differs from its note"
            );
        }
        Ok(())
    }

    /// Right-hand split: carry each controller's last state, then its later data.
    pub fn after_split(&self, offset: f64) -> Result<Self> {
        ensure!(
            offset.is_finite() && (0.0..=MAX_BEAT).contains(&offset),
            "Invalid expression split offset"
        );
        self.validate()?;
        let mut result = self.clone();
        let mut latest: HashMap<(u8, u8), &MidiNoteExpression> = HashMap::new();
        let mut future = Vec::new();
        for point in &self.expressions {
            let event = point
                .packet
                .midi2_channel_voice()
                .expect("validated expression");
            if point.beat <= offset {
                let key = (event.status, event.detail);
                if latest
                    .get(&key)
                    .is_none_or(|previous| previous.beat <= point.beat)
                {
                    latest.insert(key, point);
                }
            } else {
                let mut point = point.clone();
                point.id = new_id();
                point.beat -= offset;
                future.push(point);
            }
        }
        let mut initial: Vec<_> = latest.into_values().collect();
        initial.sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
        let mut points: Vec<_> = initial
            .into_iter()
            .map(|point| MidiNoteExpression::new(0.0, point.packet))
            .collect();
        points.extend(future);
        points.sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
        result.expressions = points;
        Ok(result)
    }
}

/// MIDI-defined centered scaling: 0→0, 64→32768 and 127→65535.
pub fn midi1_velocity_to_u16(value: u8) -> u16 {
    let value = u16::from(value.min(127));
    if value < 64 {
        value << 9
    } else {
        (value << 9) | ((value & 63) << 3) | ((value & 63) >> 3)
    }
}

/// Preserve true MIDI 2.0 velocity zero while its legacy display stays playable.
pub fn midi2_velocity_to_u7(value: u16) -> u8 {
    ((value >> 9) as u8).max(1)
}

impl Note {
    pub fn set_midi2(&mut self, data: Midi2NoteData) -> Result<()> {
        data.validate()?;
        self.pitch = data.preview_pitch();
        self.velocity = data.preview_velocity();
        self.midi2 = Some(data);
        Ok(())
    }
    pub fn set_velocity(&mut self, value: u8) {
        if value == self.velocity
            && self.midi2.as_ref().is_none_or(|data| {
                data.preview_velocity() == value
                    && (data.protocol != 1 || data.velocity == midi1_velocity_to_u16(value))
            })
        {
            return;
        }
        self.velocity = value;
        if let Some(data) = &mut self.midi2 {
            data.velocity = midi1_velocity_to_u16(value);
        }
    }
    pub fn set_velocity16(&mut self, value: u16) {
        let mut data = self
            .midi2
            .take()
            .unwrap_or_else(|| Midi2NoteData::from_legacy(self));
        data.velocity = value;
        data.protocol = 2;
        self.velocity = data.preview_velocity();
        self.midi2 = Some(data);
    }
    pub fn set_release_velocity16(&mut self, value: u16) {
        let mut data = self
            .midi2
            .take()
            .unwrap_or_else(|| Midi2NoteData::from_legacy(self));
        data.release_velocity = value;
        self.midi2 = Some(data);
    }
    pub fn set_pitch(&mut self, value: u8) {
        if value == self.pitch
            && self
                .midi2
                .as_ref()
                .is_none_or(|data| data.preview_pitch() == value)
        {
            return;
        }
        self.pitch = value;
        if let Some(data) = &mut self.midi2 {
            if data.attribute_type == 3 {
                data.attribute_data = (u16::from(value) << 9) | (data.attribute_data & 511);
            } else {
                data.note_index = value;
            }
            for point in &mut data.expressions {
                point.packet =
                    point
                        .packet
                        .with_note_address(data.group, data.channel, data.note_index);
            }
        }
    }
    /// Set absolute pitch using the nearest representable MIDI 2.0 Pitch 7.9
    /// value. The wire note index remains an independent expression address.
    pub fn set_pitch_semitones(&mut self, value: f64) -> Result<()> {
        ensure!(
            value.is_finite() && (0.0..128.0).contains(&value),
            "Pitch must be between 0 and 128 semitones"
        );
        if self.pitch_semitones() == value {
            return Ok(());
        }
        if self
            .midi2
            .as_ref()
            .is_none_or(|data| data.attribute_type != 3)
            && value.fract() == 0.0
        {
            self.set_pitch(value as u8);
            return Ok(());
        }
        let mut data = self
            .midi2
            .take()
            .unwrap_or_else(|| Midi2NoteData::from_legacy(self));
        data.attribute_type = 3;
        data.attribute_data = (value * 512.0).round().min(f64::from(u16::MAX)) as u16;
        self.pitch = data.preview_pitch();
        self.midi2 = Some(data);
        Ok(())
    }
    /// Change the start edge while retaining the note's end and the absolute
    /// timing of its expressive performance. Use a gesture's original snapshot.
    pub fn trim_start_preserving_end(&mut self, start: f64) -> Result<()> {
        let end = self.start + self.length;
        ensure!(
            start.is_finite() && start >= 0.0 && start < end && end <= MAX_BEAT,
            "Invalid MIDI note start edge"
        );
        let delta = start - self.start;
        if delta == 0.0 {
            return Ok(());
        }
        let data = if let Some(original) = &self.midi2 {
            if delta > 0.0 {
                Some(original.after_split(delta)?)
            } else {
                let mut data = original.clone();
                for point in &mut data.expressions {
                    point.beat -= delta;
                }
                data.validate()?;
                Some(data)
            }
        } else {
            None
        };
        self.start = start;
        self.length = end - start;
        self.midi2 = data;
        Ok(())
    }
    pub fn velocity16(&self) -> u16 {
        self.midi2.as_ref().map_or_else(
            || midi1_velocity_to_u16(self.velocity),
            |data| data.velocity,
        )
    }
    pub fn release_velocity16(&self) -> u16 {
        self.midi2.as_ref().map_or(0, |data| data.release_velocity)
    }
    pub fn velocity_normalized(&self) -> f64 {
        match &self.midi2 {
            Some(data) if data.protocol == 2 => f64::from(data.velocity) / f64::from(u16::MAX),
            _ => f64::from(self.velocity) / 127.0,
        }
    }
    pub fn pitch_semitones(&self) -> f64 {
        self.midi2
            .as_ref()
            .map_or(f64::from(self.pitch), Midi2NoteData::pitch_semitones)
    }
    /// Give a copied note and every expression point new object identities.
    pub fn refresh_identity(&mut self) {
        self.id = new_id();
        if let Some(data) = &mut self.midi2 {
            for point in &mut data.expressions {
                point.id = new_id();
            }
        }
    }
}

impl Project {
    pub fn contains_midi2(&self) -> bool {
        self.tracks.iter().any(|track| track.clips.iter().any(|clip| !clip.midi_events.is_empty()
            || matches!(&clip.content, crate::ClipContent::Midi { notes } if notes.iter().any(|note|
                note.midi2.as_ref().is_some_and(|data| data.protocol == 2 || data.group != 0
                    || data.attribute_type != 0 || data.release_attribute_type != 0 || !data.expressions.is_empty()
                    || data.release_velocity != midi1_velocity_to_u16((data.release_velocity >> 9) as u8))))))
    }

    pub fn set_note_velocity16(
        &mut self,
        track: u64,
        clip: u64,
        note: u64,
        value: u16,
    ) -> Result<()> {
        self.midi_notes_mut(track, clip)
            .context("MIDI clip not found")?
            .iter_mut()
            .find(|candidate| candidate.id == note)
            .context("Note not found")?
            .set_velocity16(value);
        self.synchronize_linked_clip(track, clip)
    }

    pub fn set_note_pitch_semitones(
        &mut self,
        track: u64,
        clip: u64,
        note: u64,
        value: f64,
    ) -> Result<()> {
        self.midi_notes_mut(track, clip)
            .context("MIDI clip not found")?
            .iter_mut()
            .find(|candidate| candidate.id == note)
            .context("Note not found")?
            .set_pitch_semitones(value)?;
        self.synchronize_linked_clip(track, clip)
    }

    pub fn upsert_note_expression(
        &mut self,
        track: u64,
        clip: u64,
        note: u64,
        beat: f64,
        packet: UmpPacket,
    ) -> Result<u64> {
        let point = MidiNoteExpression::new(beat, packet);
        point.validate()?;
        let region = self.clip(track, clip).context("Clip not found")?;
        let notes = match &region.content {
            crate::ClipContent::Midi { notes } => notes,
            _ => anyhow::bail!("Choose a MIDI clip"),
        };
        let original = notes
            .iter()
            .find(|candidate| candidate.id == note)
            .context("Note not found")?;
        validate_expression_address(original, packet)?;
        let message = packet
            .midi2_channel_voice()
            .expect("validated per-note packet");
        let existing = original
            .midi2
            .as_ref()
            .and_then(|data| {
                data.expressions.iter().find(|previous| {
                    let old = previous
                        .packet
                        .midi2_channel_voice()
                        .expect("validated per-note packet");
                    previous.beat == beat
                        && old.status == message.status
                        && old.detail == message.detail
                })
            })
            .map(|point| point.id);
        if existing.is_none() {
            ensure!(
                original
                    .midi2
                    .as_ref()
                    .map_or(0, |data| data.expressions.len())
                    < MAX_NOTE_EXPRESSIONS
                    && midi_event_count(region) < MAX_MIDI_EVENTS,
                "Too many MIDI expression events"
            );
        }
        let original = self
            .midi_notes_mut(track, clip)
            .expect("validated MIDI clip")
            .iter_mut()
            .find(|candidate| candidate.id == note)
            .expect("validated note");
        if original.midi2.is_none() {
            original.midi2 = Some(Midi2NoteData::from_legacy(original));
        }
        let data = original.midi2.as_mut().expect("initialized MIDI 2.0 data");
        let id = if let Some(existing) = existing {
            data.expressions
                .iter_mut()
                .find(|point| point.id == existing)
                .expect("validated point")
                .packet = packet;
            existing
        } else {
            let id = point.id;
            data.expressions.push(point);
            id
        };
        data.expressions
            .sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
        self.synchronize_linked_clip(track, clip)?;
        Ok(id)
    }

    pub fn remove_note_expression(
        &mut self,
        track: u64,
        clip: u64,
        note: u64,
        point: u64,
    ) -> Result<()> {
        let note = self
            .midi_notes_mut(track, clip)
            .context("MIDI clip not found")?
            .iter_mut()
            .find(|candidate| candidate.id == note)
            .context("Note not found")?;
        let points = &mut note
            .midi2
            .as_mut()
            .context("Note has no MIDI 2.0 expressions")?
            .expressions;
        let index = points
            .iter()
            .position(|candidate| candidate.id == point)
            .context("Expression not found")?;
        points.remove(index);
        self.synchronize_linked_clip(track, clip)
    }

    pub fn update_note_expression(
        &mut self,
        track: u64,
        clip: u64,
        note: u64,
        point: u64,
        beat: f64,
        packet: UmpPacket,
    ) -> Result<()> {
        let replacement = MidiNoteExpression {
            id: point,
            beat,
            packet,
        };
        replacement.validate()?;
        let original = self
            .midi_notes(track, clip)
            .context("MIDI clip not found")?
            .iter()
            .find(|candidate| candidate.id == note)
            .context("Note not found")?;
        validate_expression_address(original, packet)?;
        ensure!(
            original.midi2.as_ref().is_some_and(|data| data
                .expressions
                .iter()
                .any(|candidate| candidate.id == point)),
            "Expression not found"
        );
        let data = self
            .midi_notes_mut(track, clip)
            .expect("validated MIDI clip")
            .iter_mut()
            .find(|candidate| candidate.id == note)
            .expect("validated note")
            .midi2
            .as_mut()
            .expect("validated MIDI 2.0 data");
        *data
            .expressions
            .iter_mut()
            .find(|candidate| candidate.id == point)
            .expect("validated point") = replacement;
        data.expressions
            .sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
        self.synchronize_linked_clip(track, clip)
    }

    pub fn set_midi_event(
        &mut self,
        track: u64,
        clip: u64,
        event: Option<u64>,
        beat: f64,
        packet: UmpPacket,
    ) -> Result<u64> {
        let id = event.unwrap_or_else(new_id);
        let replacement = MidiClipEvent { id, beat, packet };
        replacement.validate()?;
        let region = self.clip(track, clip).context("Clip not found")?;
        ensure!(
            matches!(region.content, crate::ClipContent::Midi { .. }),
            "Choose a MIDI clip"
        );
        if let Some(event) = event {
            ensure!(
                region
                    .midi_events
                    .iter()
                    .any(|candidate| candidate.id == event),
                "MIDI event not found"
            );
        } else {
            ensure!(
                midi_event_count(region) < MAX_MIDI_EVENTS,
                "Too many MIDI performance events"
            );
        }
        let region = self.clip_mut(track, clip).expect("validated MIDI clip");
        if let Some(event) = event {
            *region
                .midi_events
                .iter_mut()
                .find(|candidate| candidate.id == event)
                .expect("validated event") = replacement;
        } else {
            region.midi_events.push(replacement);
        }
        region
            .midi_events
            .sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
        self.synchronize_linked_clip(track, clip)?;
        Ok(id)
    }

    pub fn remove_midi_event(&mut self, track: u64, clip: u64, event: u64) -> Result<()> {
        let region = self.clip_mut(track, clip).context("Clip not found")?;
        let index = region
            .midi_events
            .iter()
            .position(|candidate| candidate.id == event)
            .context("MIDI event not found")?;
        region.midi_events.remove(index);
        self.synchronize_linked_clip(track, clip)
    }
}

fn validate_expression_address(note: &Note, packet: UmpPacket) -> Result<()> {
    let address = note.midi2.as_ref().map_or((0, 0, note.pitch), |data| {
        (data.group, data.channel, data.note_index)
    });
    let message = packet
        .midi2_channel_voice()
        .context("Expected MIDI 2.0 expression")?;
    ensure!(
        (message.group, message.channel, message.index & 127) == address,
        "Expression targets another note"
    );
    Ok(())
}

pub(crate) fn midi_event_count(clip: &crate::Clip) -> usize {
    let expressions = match &clip.content {
        crate::ClipContent::Midi { notes } => notes.iter().fold(0usize, |count, note| {
            count.saturating_add(note.midi2.as_ref().map_or(0, |data| data.expressions.len()))
        }),
        _ => 0,
    };
    clip.midi_events.len().saturating_add(expressions)
}
