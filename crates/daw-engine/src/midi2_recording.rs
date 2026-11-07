//! Finalize timestamped performance into editable notes and source-relative controller data.
//! Runs after capture stops; the device/input callbacks only copy fixed-size packets.
use super::{Event, MAX_EVENTS, MidiRecordingResult, RecordedMidiTrack, Recording};
use daw_core::{
    MAX_NOTE_EXPRESSIONS, Midi2NoteData, MidiClipEvent, MidiNoteExpression, Note, UmpPacket,
    midi1_velocity_to_u16,
};
use std::collections::HashMap;

struct Voice {
    start: u64,
    data: Midi2NoteData,
    down: bool,
    attached: bool,
}
struct Finished {
    voice: Voice,
    end: u64,
}

impl Event {
    pub(super) fn packet(self) -> UmpPacket {
        self.ump.unwrap_or(UmpPacket {
            word_count: 1,
            words: [
                0x2000_0000
                    | ((self.data & 255) << 16)
                    | (((self.data >> 8) & 255) << 8)
                    | ((self.data >> 16) & 255),
                0,
                0,
                0,
            ],
        })
    }
}

pub(super) fn assemble(
    mut recording: Recording,
    frames: u64,
    start: u64,
    sample_rate: u32,
) -> MidiRecordingResult {
    if frames == 0 {
        return MidiRecordingResult::default();
    }
    recording.events.sort_by_key(|event| event.time);
    let factor = recording.tempo / 60.0 / f64::from(sample_rate);
    let map = |at: u64| -> (u64, u64) {
        if let Some((left, right)) = recording.loop_range {
            let first = right.saturating_sub(start);
            if at >= first {
                let within = (at - first) % (right - left);
                return (left + within, right - left - within);
            }
            (start + at, first - at)
        } else {
            (start + at, u64::MAX)
        }
    };
    let clip_start = if recording
        .loop_range
        .is_some_and(|(_, right)| frames > right.saturating_sub(start))
    {
        recording.loop_range.unwrap().0.min(start)
    } else {
        start
    };
    let end_position = if let Some((_, right)) = recording.loop_range {
        if frames >= right.saturating_sub(start) {
            right
        } else {
            start + frames
        }
    } else {
        start + frames
    };
    let mut result = MidiRecordingResult::default();
    for route in &recording.routes {
        let mut active = Vec::<Voice>::new();
        let mut finished = Vec::<Finished>::new();
        let mut sustain = [false; 16];
        let mut controls = HashMap::<(u8, u8, u8, u8), UmpPacket>::new();
        let mut unattached = std::collections::HashSet::<(u8, u8, u8, u8)>::new();
        let mut global = Vec::new();
        let mut budget = 0;
        let mut cutoff = frames;
        'capture: for event in recording
            .events
            .iter()
            .copied()
            .filter(|event| event.time < frames)
        {
            let packet = event.packet();
            if packet.group().is_some_and(|group| group != route.group) {
                continue;
            }
            let decoded = packet.midi2_channel_voice();
            let legacy = packet.message_type() == 2;
            let (channel, status, index, detail, data) = if let Some(message) = decoded {
                (
                    message.channel,
                    message.status,
                    message.index & 127,
                    message.detail,
                    message.data,
                )
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
                if budget >= MAX_EVENTS {
                    cutoff = event.time;
                    break;
                }
                global.push(MidiClipEvent::new(
                    (map(event.time).0 - clip_start) as f64 * factor,
                    packet,
                ));
                budget += 1;
                continue;
            };
            if route.channel.is_some_and(|selected| selected != channel) {
                continue;
            }
            let note_on = status == 9 && (!legacy || detail != 0);
            let note_off = status == 8 || (legacy && status == 9 && detail == 0);
            if note_on {
                if budget >= MAX_EVENTS || active.len() >= 2048 {
                    cutoff = event.time;
                    break;
                }
                if legacy
                    && let Some(position) = active.iter().position(|voice| {
                        voice.data.channel == channel && voice.data.note_index == index
                    })
                {
                    finished.push(Finished {
                        voice: active.remove(position),
                        end: event.time,
                    });
                }
                let mut note_data = if let Some(message) = decoded {
                    Midi2NoteData::from_note_on(message).expect("validated UMP note address")
                } else {
                    let mut value = Midi2NoteData::from_legacy(&Note::new(index, 0.0, 1.0, detail));
                    value.group = route.group;
                    value.channel = channel;
                    value
                };
                let mut initial: Vec<_> = controls
                    .iter()
                    .filter(|((ch, pitch, _, _), _)| *ch == channel && *pitch == index)
                    .collect();
                initial.sort_by_key(|(key, _)| **key);
                for (_, packet) in initial {
                    if budget + 1 >= MAX_EVENTS
                        || note_data.expressions.len() >= MAX_NOTE_EXPRESSIONS
                    {
                        cutoff = event.time;
                        break 'capture;
                    }
                    note_data
                        .expressions
                        .push(MidiNoteExpression::new(0.0, *packet));
                    budget += 1;
                }
                unattached.retain(|(ch, pitch, _, _)| *ch != channel || *pitch != index);
                active.push(Voice {
                    start: event.time,
                    data: note_data,
                    down: true,
                    attached: true,
                });
                budget += 1;
            } else if note_off {
                if let Some(position) = active.iter().position(|voice| {
                    voice.down && voice.data.channel == channel && voice.data.note_index == index
                }) {
                    let voice = &mut active[position];
                    voice.data.release_velocity = if legacy {
                        midi1_velocity_to_u16(detail)
                    } else {
                        (data >> 16) as u16
                    };
                    if !legacy {
                        voice.data.release_attribute_type = detail;
                        voice.data.release_attribute_data = data as u16;
                    }
                    voice.down = false;
                    if !legacy || !sustain[usize::from(channel)] {
                        finished.push(Finished {
                            voice: active.remove(position),
                            end: event.time,
                        });
                    }
                }
            } else if !legacy && matches!(status, 0 | 1 | 6 | 10 | 15) {
                if status == 15 && detail & 1 != 0 {
                    controls.retain(|(ch, pitch, _, _), _| *ch != channel || *pitch != index);
                    unattached.retain(|(ch, pitch, _, _)| *ch != channel || *pitch != index);
                } else if status != 15 {
                    controls.insert((channel, index, status, detail), packet);
                    let key = (channel, index, status, detail);
                    if active.iter().any(|voice| {
                        voice.attached
                            && voice.data.channel == channel
                            && voice.data.note_index == index
                    }) {
                        unattached.remove(&key);
                    } else {
                        unattached.insert(key);
                    }
                }
                let mut overflow = false;
                for voice in active.iter_mut().filter(|voice| {
                    voice.attached
                        && voice.data.channel == channel
                        && voice.data.note_index == index
                }) {
                    if budget >= MAX_EVENTS || voice.data.expressions.len() >= MAX_NOTE_EXPRESSIONS
                    {
                        overflow = true;
                        break;
                    }
                    voice.data.expressions.push(MidiNoteExpression::new(
                        (event.time - voice.start) as f64 * factor,
                        packet,
                    ));
                    budget += 1;
                    if status == 15 && detail & 2 != 0 {
                        voice.attached = false;
                    }
                }
                if overflow {
                    cutoff = event.time;
                    break;
                }
            } else {
                if budget >= MAX_EVENTS {
                    cutoff = event.time;
                    break;
                }
                global.push(MidiClipEvent::new(
                    (map(event.time).0 - clip_start) as f64 * factor,
                    packet,
                ));
                budget += 1;
                if status == 11 && index == 64 {
                    let down = if legacy {
                        detail >= 64
                    } else {
                        data >= 0x8000_0000
                    };
                    sustain[usize::from(channel)] = down;
                    if !down {
                        let mut position = 0;
                        while position < active.len() {
                            if active[position].data.channel == channel && !active[position].down {
                                finished.push(Finished {
                                    voice: active.remove(position),
                                    end: event.time,
                                });
                            } else {
                                position += 1;
                            }
                        }
                    }
                } else if status == 11 && matches!(index, 120 | 123) {
                    sustain[usize::from(channel)] = false;
                    let mut position = 0;
                    while position < active.len() {
                        if active[position].data.channel == channel {
                            finished.push(Finished {
                                voice: active.remove(position),
                                end: event.time,
                            });
                        } else {
                            position += 1;
                        }
                    }
                }
            }
        }
        let orphaned = !unattached.is_empty();
        if orphaned {
            result.error=Some("Unassociated per-note controller setup could not be attached to a recorded note; other captured notes and channel data were retained".into());
        }
        if cutoff < frames {
            result.error = Some(
                "MIDI recording data limit reached; the bounded captured prefix was retained"
                    .into(),
            );
        }
        finished.extend(
            active
                .into_iter()
                .map(|voice| Finished { voice, end: cutoff }),
        );
        let mut notes = Vec::new();
        let mut published = global.len();
        'publish: for captured in finished {
            let mut at = captured.voice.start;
            while at < captured.end && notes.len() < MAX_EVENTS {
                let (position, remaining) = map(at);
                let length = (captured.end - at).min(remaining);
                if length == 0 {
                    break;
                }
                let mut note = Note::new(
                    captured.voice.data.note_index,
                    (position - clip_start) as f64 * factor,
                    length as f64 * factor,
                    captured.voice.data.preview_velocity(),
                );
                let mut data = match captured
                    .voice
                    .data
                    .after_split((at - captured.voice.start) as f64 * factor)
                {
                    Ok(data) => data,
                    Err(error) => {
                        result.error = Some(format!(
                            "Captured note metadata could not be retained: {error:#}"
                        ));
                        break;
                    }
                };
                data.expressions
                    .retain(|point| point.beat < length as f64 * factor);
                if published + data.expressions.len() + 1 > daw_core::MAX_MIDI_EVENTS {
                    result.error = Some(
                        "Loop recording data limit reached; the valid captured prefix was retained"
                            .into(),
                    );
                    break 'publish;
                }
                published += data.expressions.len() + 1;
                // Keep the old, implicit channel-one representation when no extra
                // information exists; all other channels/resolutions remain explicit.
                if (data.protocol != 1
                    || data.group != 0
                    || data.channel != 0
                    || data.release_velocity != 0
                    || !data.expressions.is_empty())
                    && let Err(error) = note.set_midi2(data)
                {
                    result.error = Some(format!("Captured note metadata is invalid: {error:#}"));
                    break;
                }
                notes.push(note);
                at += length;
            }
        }
        if !notes.is_empty() || !global.is_empty() {
            notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
            global.sort_by(|a, b| a.beat.total_cmp(&b.beat));
            result.tracks.push(RecordedMidiTrack {
                track: route.track,
                start_beat: clip_start as f64 * factor,
                length_beats: (end_position - clip_start) as f64 * factor,
                notes,
                midi_events: global,
            });
        }
    }
    result
}
