use crate::{Clip, ClipContent, MAX_BEAT, Note, Project, Track, TrackKind};
use anyhow::{Context, Result, bail, ensure};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
};

/// The project currently uses one global tempo. Files with a tempo map or SMPTE
/// timing are rejected explicitly instead of silently changing musical timing.
pub struct MidiImport {
    pub tracks: Vec<Track>,
    pub tempo: Option<f64>,
    pub beats_per_bar: Option<u8>,
}

pub fn import_midi(path: impl AsRef<Path>) -> Result<MidiImport> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).with_context(|| format!("Read MIDI {}", path.display()))?;
    let smf = Smf::parse(&bytes).context("Parse MIDI file")?;
    ensure!(
        smf.header.format != Format::Sequential,
        "Sequential MIDI patterns are not supported; use a type 0 or type 1 file"
    );
    let ticks_per_beat = match smf.header.timing {
        Timing::Metrical(ticks) if ticks.as_int() > 0 => f64::from(ticks.as_int()),
        _ => bail!("MIDI import requires nonzero quarter-note tick timing; SMPTE is unsupported"),
    };
    let mut tempo_micros = None;
    let mut tempo_at_zero = false;
    let mut signature = None;
    let mut signature_at_zero = false;
    let mut tracks = Vec::new();
    for (index, events) in smf.tracks.iter().enumerate() {
        let mut tick = 0u64;
        let mut notes = Vec::new();
        let mut active: HashMap<(u8, u8), VecDeque<(u64, u8)>> = HashMap::new();
        let mut name = format!("MIDI {}", index + 1);
        for event in events {
            tick = tick
                .checked_add(u64::from(event.delta.as_int()))
                .context("MIDI duration overflow")?;
            match event.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(text)) => {
                    name = String::from_utf8_lossy(text).into_owned()
                }
                TrackEventKind::Meta(MetaMessage::Tempo(value)) => {
                    let value = value.as_int();
                    ensure!(value > 0, "MIDI has zero tempo");
                    ensure!(
                        tempo_micros.is_none_or(|previous| previous == value),
                        "Variable MIDI tempo is not supported by this project format"
                    );
                    tempo_micros = Some(value);
                    tempo_at_zero |= tick == 0;
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(numerator, denominator, _, _)) => {
                    ensure!(
                        denominator == 2 && (1..=32).contains(&numerator),
                        "Only quarter-note time signatures with 1–32 beats per bar are supported"
                    );
                    ensure!(
                        signature.is_none_or(|previous| previous == numerator),
                        "Changing MIDI time signatures are not supported"
                    );
                    signature = Some(numerator);
                    signature_at_zero |= tick == 0;
                }
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn { key, vel },
                } if vel.as_int() != 0 => {
                    active
                        .entry((channel.as_int(), key.as_int()))
                        .or_default()
                        .push_back((tick, vel.as_int()));
                }
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. },
                } => {
                    if let Some((start, velocity)) = active
                        .get_mut(&(channel.as_int(), key.as_int()))
                        .and_then(VecDeque::pop_front)
                    {
                        let length = tick.saturating_sub(start).max(1) as f64 / ticks_per_beat;
                        notes.push(Note::new(
                            key.as_int(),
                            start as f64 / ticks_per_beat,
                            length,
                            velocity,
                        ));
                    }
                }
                _ => {}
            }
        }
        for ((_, pitch), voices) in active {
            for (start, velocity) in voices {
                notes.push(Note::new(
                    pitch,
                    start as f64 / ticks_per_beat,
                    tick.saturating_sub(start).max(1) as f64 / ticks_per_beat,
                    velocity,
                ));
            }
        }
        if notes.is_empty() {
            continue;
        }
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        let length = notes
            .iter()
            .map(|note| note.start + note.length)
            .fold(0.0, f64::max)
            .max(tick as f64 / ticks_per_beat);
        ensure!(
            length <= MAX_BEAT,
            "MIDI file exceeds the supported timeline"
        );
        let mut track = Track::new(&name, TrackKind::Midi);
        track.clips.push(Clip::midi(
            name,
            0.0,
            length.max(1.0 / ticks_per_beat),
            notes,
        ));
        tracks.push(track);
    }
    ensure!(!tracks.is_empty(), "MIDI file contains no notes");
    ensure!(
        tempo_at_zero || tempo_micros.is_none_or(|value| value == 500_000),
        "MIDI tempo changes after the initial default tempo; tempo maps are unsupported"
    );
    ensure!(
        signature_at_zero || signature.is_none_or(|value| value == 4),
        "MIDI time signature changes after the initial 4/4 meter"
    );
    let tempo = tempo_micros.map(|micros| 60_000_000.0 / f64::from(micros));
    ensure!(
        tempo.is_none_or(|value| (20.0..=400.0).contains(&value)),
        "MIDI tempo is outside 20–400 BPM"
    );
    Ok(MidiImport {
        tracks,
        tempo,
        beats_per_bar: signature,
    })
}

pub fn export_midi(project: &Project, path: impl AsRef<Path>) -> Result<()> {
    project.validate()?;
    const PPQ: u16 = 960;
    let mut tracks = vec![vec![
        TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::Tempo(
                ((60_000_000.0 / project.tempo).round() as u32).into(),
            )),
        },
        TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::TimeSignature(project.beats_per_bar, 2, 24, 8)),
        },
        TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        },
    ]];
    for (index, track) in project
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Midi)
        .enumerate()
    {
        let channel = (index % 16) as u8;
        let mut events = Vec::new();
        for clip in &track.clips {
            let ClipContent::Midi { notes } = &clip.content else {
                continue;
            };
            for note in notes {
                let start = note.start.max(clip.offset);
                let end = (note.start + note.length).min(clip.source_end());
                if end <= start {
                    continue;
                }
                let on_tick = to_tick(clip.start + clip.source_to_local(start), PPQ)?;
                let off_tick =
                    to_tick(clip.start + clip.source_to_local(end), PPQ)?.max(on_tick + 1);
                events.push((on_tick, 1u8, note.pitch, note.velocity));
                events.push((off_tick, 0u8, note.pitch, 0u8));
            }
        }
        // Note-off precedes note-on at equal ticks for retriggered notes.
        events.sort_unstable_by_key(|event| (event.0, event.1, event.2));
        let mut output = vec![TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::TrackName(track.name.as_bytes())),
        }];
        let mut last_tick = 0;
        for (tick, on, pitch, velocity) in events {
            let delta = tick - last_tick;
            ensure!(
                delta <= 0x0fff_ffff,
                "MIDI event delta exceeds format limit"
            );
            output.push(TrackEvent {
                delta: delta.into(),
                kind: TrackEventKind::Midi {
                    channel: channel.into(),
                    message: if on == 1 {
                        MidiMessage::NoteOn {
                            key: pitch.into(),
                            vel: velocity.into(),
                        }
                    } else {
                        MidiMessage::NoteOff {
                            key: pitch.into(),
                            vel: 0.into(),
                        }
                    },
                },
            });
            last_tick = tick;
        }
        output.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        tracks.push(output);
    }
    let smf = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(PPQ.into()),
        },
        tracks,
    };
    smf.save(path).context("Write MIDI file")
}

fn to_tick(beat: f64, ppq: u16) -> Result<u32> {
    let ticks = (beat * f64::from(ppq)).round();
    ensure!(
        ticks.is_finite() && ticks >= 0.0 && ticks < f64::from(u32::MAX),
        "MIDI timeline exceeds supported duration"
    );
    Ok(ticks as u32)
}

impl Project {
    /// Import notes as new tracks, adopting file tempo/meter only for an empty project.
    pub fn import_midi(&mut self, path: impl AsRef<Path>, at_beat: f64) -> Result<Vec<u64>> {
        ensure!(
            at_beat.is_finite() && (0.0..=MAX_BEAT).contains(&at_beat),
            "Invalid MIDI import position"
        );
        let mut imported = import_midi(path)?;
        for track in &mut imported.tracks {
            for clip in &mut track.clips {
                clip.start += at_beat;
                ensure!(
                    clip.end() <= MAX_BEAT && clip.end() > clip.start,
                    "MIDI import position is outside the supported timeline"
                );
            }
        }
        if self.tracks.is_empty() {
            if let Some(tempo) = imported.tempo {
                self.tempo = tempo;
            }
            if let Some(meter) = imported.beats_per_bar {
                self.beats_per_bar = meter;
            }
        }
        let ids = imported
            .tracks
            .into_iter()
            .map(|track| self.insert_new_track(track))
            .collect();
        Ok(ids)
    }

    pub fn export_midi(&self, path: impl AsRef<Path>) -> Result<()> {
        export_midi(self, path)
    }
}
