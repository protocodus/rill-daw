use crate::{AudioCrossfade, AudioEnvelope, AudioProcessing, AutomationLane, FadeCurve};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A generous finite timeline limit keeps beat-to-sample conversion exact and
/// prevents malformed project files from saturating integer sample positions.
pub const MAX_BEAT: f64 = 1_000_000.0;
pub const MIN_SOURCE_RATE: f64 = 0.25;
pub const MAX_SOURCE_RATE: f64 = 4.0;

fn default_source_rate() -> f64 {
    1.0
}

pub const MAX_PLUGIN_STATE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_INSTRUMENT_STATE_BYTES: usize = MAX_PLUGIN_STATE_BYTES;

pub(crate) fn validate_vst_class_id(class_id: Option<&str>) -> Result<()> {
    ensure!(
        class_id.is_none_or(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())),
        "VST3 processor class ID must contain exactly 32 hexadecimal characters"
    );
    Ok(())
}

/// IDs are unique within a process; loading a project reserves its existing IDs.
pub fn new_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub tempo: f64,
    pub beats_per_bar: u8,
    pub tracks: Vec<Track>,
    pub loop_enabled: bool,
    pub loop_start: f64,
    pub loop_end: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub name: String,
    pub kind: TrackKind,
    pub color: [u8; 3],
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub instrument: Instrument,
    /// Opaque VST3 state captured by the isolated native editor.
    #[serde(default)]
    pub instrument_state: Vec<u8>,
    /// Normalized values applied after restoring opaque state.
    #[serde(default)]
    pub instrument_parameters: Vec<PluginParameterValue>,
    pub clips: Vec<Clip>,
    /// Additional path latency, added to latency reported by the instrument.
    pub latency_samples: u32,
    #[serde(default)]
    pub inserts: Vec<EffectSlot>,
    #[serde(default)]
    pub sends: Vec<Send>,
    /// Main audio output; `None` routes to the master output.
    #[serde(default)]
    pub output_bus: Option<u64>,
    /// Organizational parent. Folders do not sum or process audio.
    #[serde(default)]
    pub parent_folder: Option<u64>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub automation: Option<AutomationLane>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Audio,
    Midi,
    Bus,
    Folder,
    Automation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectSlot {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub effect: Effect,
    /// Opaque VST3 component/controller state, restored before overrides.
    #[serde(default)]
    pub state: Vec<u8>,
    #[serde(default)]
    pub parameters: Vec<PluginParameterValue>,
}

impl EffectSlot {
    pub fn new(name: impl Into<String>, effect: Effect) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            enabled: true,
            effect,
            state: Vec::new(),
            parameters: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Vst3 {
        path: String,
        #[serde(default)]
        class_id: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Send {
    pub target_bus: u64,
    pub gain: f32,
    pub pre_fader: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Instrument {
    #[default]
    None,
    Vst3 {
        path: String,
        #[serde(default)]
        class_id: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginParameterValue {
    pub id: u32,
    pub value: f64,
}

fn validate_plugin_state(state: &[u8], parameters: &[PluginParameterValue]) -> Result<()> {
    ensure!(
        state.len() <= MAX_PLUGIN_STATE_BYTES,
        "Plug-in state exceeds 64 MiB"
    );
    let mut parameter_ids = HashSet::new();
    for parameter in parameters {
        ensure!(
            parameter_ids.insert(parameter.id),
            "Duplicate plug-in parameter ID"
        );
        ensure!(
            parameter.value.is_finite() && (0.0..=1.0).contains(&parameter.value),
            "Plug-in parameter must be normalized between 0 and 1"
        );
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: u64,
    pub name: String,
    /// Shared source identity. Arrangement bounds remain local to this instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_group: Option<u64>,
    pub start: f64,
    pub length: f64,
    /// Source beat at the clip's left edge. Trimming never destroys source notes.
    pub offset: f64,
    /// Source beats consumed per arrangement beat. Local to this occurrence,
    /// including when its content belongs to a shared ghost group.
    #[serde(default = "default_source_rate")]
    pub source_rate: f64,
    pub content: ClipContent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipContent {
    Midi {
        notes: Vec<Note>,
    },
    Audio {
        path: String,
        #[serde(default)]
        processing: AudioProcessing,
        gain: f32,
        fade_in: f64,
        fade_out: f64,
        #[serde(default)]
        fade_in_curve: FadeCurve,
        #[serde(default)]
        fade_out_curve: FadeCurve,
        #[serde(default)]
        reverse: bool,
        #[serde(default)]
        envelope: Option<AudioEnvelope>,
        #[serde(default)]
        crossfade: Option<AudioCrossfade>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub pitch: u8,
    /// Source position, not arrangement position; see `Clip::offset`.
    pub start: f64,
    pub length: f64,
    pub velocity: u8,
}

impl Note {
    pub fn new(pitch: u8, start: f64, length: f64, velocity: u8) -> Self {
        Self {
            id: new_id(),
            pitch,
            start,
            length,
            velocity,
        }
    }
}

impl Clip {
    pub fn midi(name: impl Into<String>, start: f64, length: f64, notes: Vec<Note>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            link_group: None,
            start,
            length,
            offset: 0.0,
            source_rate: 1.0,
            content: ClipContent::Midi { notes },
        }
    }

    pub fn audio(
        name: impl Into<String>,
        start: f64,
        length: f64,
        path: impl Into<String>,
    ) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            link_group: None,
            start,
            length,
            offset: 0.0,
            source_rate: 1.0,
            content: ClipContent::Audio {
                path: path.into(),
                processing: AudioProcessing::default(),
                gain: 1.0,
                fade_in: 0.0,
                fade_out: 0.0,
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                reverse: false,
                envelope: None,
                crossfade: None,
            },
        }
    }

    pub fn end(&self) -> f64 {
        self.start + self.length
    }

    pub fn source_length(&self) -> f64 {
        self.length * self.source_rate
    }
    pub fn source_end(&self) -> f64 {
        self.offset + self.source_length()
    }
    pub fn source_to_local(&self, source_beat: f64) -> f64 {
        (source_beat - self.offset) / self.source_rate
    }
    pub fn local_to_source(&self, local_beat: f64) -> f64 {
        self.offset + local_beat * self.source_rate
    }

    fn refresh_ids(&mut self) {
        self.id = new_id();
        self.link_group = None;
        if let ClipContent::Midi { notes } = &mut self.content {
            for note in notes {
                note.id = new_id();
            }
        }
    }

    fn constrain_fades(&mut self) {
        if let ClipContent::Audio {
            fade_in,
            fade_out,
            envelope,
            ..
        } = &mut self.content
        {
            let length =
                envelope.map_or(self.length * self.source_rate, |envelope| envelope.length);
            *fade_in = fade_in.min(length);
            *fade_out = fade_out.min(length);
        }
    }
}

impl Track {
    pub fn new(name: impl Into<String>, kind: TrackKind) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            kind,
            color: match kind {
                TrackKind::Audio => [80, 184, 195],
                TrackKind::Midi => [169, 132, 255],
                TrackKind::Bus => [227, 164, 95],
                TrackKind::Folder => [131, 143, 166],
                TrackKind::Automation => [234, 185, 96],
            },
            gain: if matches!(kind, TrackKind::Bus | TrackKind::Folder) {
                1.0
            } else {
                0.75
            },
            pan: 0.0,
            mute: false,
            solo: false,
            instrument: Instrument::None,
            instrument_state: Vec::new(),
            instrument_parameters: Vec::new(),
            clips: Vec::new(),
            latency_samples: 0,
            inserts: Vec::new(),
            sends: Vec::new(),
            output_bus: None,
            parent_folder: None,
            collapsed: false,
            automation: None,
        }
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    pub fn new() -> Self {
        Self {
            name: "Untitled session".into(),
            tempo: 120.0,
            beats_per_bar: 4,
            tracks: Vec::new(),
            loop_enabled: false,
            loop_start: 0.0,
            loop_end: 16.0,
        }
    }

    pub fn add_audio_track(&mut self, name: impl Into<String>) -> u64 {
        self.insert_new_track(Track::new(name, TrackKind::Audio))
    }

    pub fn add_midi_track(&mut self, name: impl Into<String>) -> u64 {
        self.insert_new_track(Track::new(name, TrackKind::Midi))
    }

    pub fn track(&self, id: u64) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }
    pub fn track_mut(&mut self, id: u64) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|track| track.id == id)
    }

    pub fn clip(&self, track_id: u64, clip_id: u64) -> Option<&Clip> {
        self.track(track_id)?
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
    }

    pub fn clip_mut(&mut self, track_id: u64, clip_id: u64) -> Option<&mut Clip> {
        self.track_mut(track_id)?
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
    }

    pub fn midi_notes(&self, track_id: u64, clip_id: u64) -> Option<&[Note]> {
        match &self.clip(track_id, clip_id)?.content {
            ClipContent::Midi { notes } => Some(notes),
            _ => None,
        }
    }

    pub fn midi_notes_mut(&mut self, track_id: u64, clip_id: u64) -> Option<&mut Vec<Note>> {
        match &mut self.clip_mut(track_id, clip_id)?.content {
            ClipContent::Midi { notes } => Some(notes),
            _ => None,
        }
    }

    pub fn split_note(
        &mut self,
        track_id: u64,
        clip_id: u64,
        note_id: u64,
        at_source_beat: f64,
    ) -> Result<u64> {
        let notes = self
            .midi_notes_mut(track_id, clip_id)
            .context("MIDI clip not found")?;
        let index = notes
            .iter()
            .position(|note| note.id == note_id)
            .context("Note not found")?;
        let note = &mut notes[index];
        ensure!(
            at_source_beat.is_finite()
                && at_source_beat > note.start
                && at_source_beat < note.start + note.length,
            "Split must be inside the note"
        );
        let right = Note::new(
            note.pitch,
            at_source_beat,
            note.start + note.length - at_source_beat,
            note.velocity,
        );
        note.length = at_source_beat - note.start;
        let id = right.id;
        notes.insert(index + 1, right);
        self.synchronize_linked_clip(track_id, clip_id)?;
        Ok(id)
    }

    pub fn duplicate_note(&mut self, track_id: u64, clip_id: u64, note_id: u64) -> Result<u64> {
        let notes = self
            .midi_notes_mut(track_id, clip_id)
            .context("MIDI clip not found")?;
        let mut duplicate = notes
            .iter()
            .find(|note| note.id == note_id)
            .context("Note not found")?
            .clone();
        duplicate.start += duplicate.length;
        validate_note(&duplicate)?;
        duplicate.id = new_id();
        let id = duplicate.id;
        notes.push(duplicate);
        self.synchronize_linked_clip(track_id, clip_id)?;
        Ok(id)
    }

    pub fn delete_note(&mut self, track_id: u64, clip_id: u64, note_id: u64) -> Result<Note> {
        let notes = self
            .midi_notes_mut(track_id, clip_id)
            .context("MIDI clip not found")?;
        let index = notes
            .iter()
            .position(|note| note.id == note_id)
            .context("Note not found")?;
        let note = notes.remove(index);
        self.synchronize_linked_clip(track_id, clip_id)?;
        Ok(note)
    }

    /// Glue same-pitch notes into one sustained note, including gaps between them.
    /// The earliest selected note supplies pitch and velocity.
    pub fn glue_notes(&mut self, track_id: u64, clip_id: u64, note_ids: &[u64]) -> Result<u64> {
        let selected: HashSet<_> = note_ids.iter().copied().collect();
        ensure!(selected.len() >= 2, "Select at least two notes to glue");
        let notes = self
            .midi_notes_mut(track_id, clip_id)
            .context("MIDI clip not found")?;
        let selected_notes: Vec<_> = notes
            .iter()
            .filter(|note| selected.contains(&note.id))
            .collect();
        ensure!(
            selected_notes.len() == selected.len(),
            "A selected note was not found"
        );
        let first = selected_notes
            .iter()
            .min_by(|a, b| a.start.total_cmp(&b.start))
            .expect("selection has at least two notes");
        ensure!(
            selected_notes.iter().all(|note| note.pitch == first.pitch),
            "Glue requires notes with the same pitch"
        );
        let end = selected_notes
            .iter()
            .map(|note| note.start + note.length)
            .fold(first.start, f64::max);
        let glued = Note::new(first.pitch, first.start, end - first.start, first.velocity);
        validate_note(&glued)?;
        let id = glued.id;
        notes.retain(|note| !selected.contains(&note.id));
        notes.push(glued);
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        self.synchronize_linked_clip(track_id, clip_id)?;
        Ok(id)
    }

    pub fn end_beat(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(Clip::end)
            .fold(0.0, f64::max)
    }

    pub fn move_clip(&mut self, track_id: u64, clip_id: u64, new_start: f64) -> Result<()> {
        ensure!(
            nonnegative(new_start),
            "Clip position must be finite and nonnegative"
        );
        let clip = self.clip_mut(track_id, clip_id).context("Clip not found")?;
        ensure!(
            valid_range(new_start, clip.length),
            "Clip exceeds the supported timeline"
        );
        clip.start = new_start;
        Ok(())
    }

    /// Split at an absolute arrangement beat, keeping the source intact on both sides.
    pub fn split_clip(&mut self, track_id: u64, clip_id: u64, at_beat: f64) -> Result<u64> {
        let track = self.track_mut(track_id).context("Track not found")?;
        let index = track
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .context("Clip not found")?;
        let clip = &mut track.clips[index];
        ensure!(
            at_beat.is_finite() && at_beat > clip.start && at_beat < clip.end(),
            "Split must be inside the clip"
        );
        let delta = at_beat - clip.start;
        if let ClipContent::Audio {
            envelope: Some(envelope),
            ..
        } = clip.content
        {
            ensure!(
                (envelope.offset + delta * clip.source_rate).abs() <= MAX_BEAT,
                "Split exceeds the envelope range"
            );
        }
        let mut right = clip.clone();
        if right.link_group.is_some() {
            right.id = new_id();
        } else {
            right.refresh_ids();
        }
        right.start = at_beat;
        if let ClipContent::Audio { crossfade, .. } = &mut right.content {
            *crossfade = None;
        }
        if matches!(clip.content, ClipContent::Audio { reverse: true, .. }) {
            clip.offset += (clip.length - delta) * clip.source_rate;
        } else {
            right.offset += delta * clip.source_rate;
        }
        let envelope = match &mut clip.content {
            ClipContent::Audio { envelope, .. } => {
                let original = envelope.unwrap_or(AudioEnvelope {
                    offset: 0.0,
                    length: clip.length * clip.source_rate,
                });
                *envelope = Some(original);
                Some(original)
            }
            _ => None,
        };
        if let (Some(original), ClipContent::Audio { envelope, .. }) =
            (envelope, &mut right.content)
        {
            *envelope = Some(AudioEnvelope {
                offset: original.offset + delta * clip.source_rate,
                ..original
            });
        }
        right.length -= delta;
        right.constrain_fades();
        clip.length = delta;
        clip.constrain_fades();
        let id = right.id;
        track.clips.insert(index + 1, right);
        Ok(id)
    }

    pub fn duplicate_clip(&mut self, track_id: u64, clip_id: u64) -> Result<u64> {
        let start = self
            .clip(track_id, clip_id)
            .context("Clip not found")?
            .end();
        self.duplicate_clip_at(track_id, clip_id, start)
    }

    /// Duplicate directly at the requested position. Validate before allocating
    /// IDs or changing the document, including near the timeline boundary where
    /// an adjacent temporary copy would be invalid even when this target is valid.
    pub fn duplicate_clip_at(&mut self, track_id: u64, clip_id: u64, start: f64) -> Result<u64> {
        let track = self.track_mut(track_id).context("Track not found")?;
        let source = track
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("Clip not found")?;
        ensure!(
            valid_range(start, source.length),
            "Duplicate clip exceeds the supported timeline"
        );
        let mut clip = source.clone();
        clip.start = start;
        clip.refresh_ids();
        let id = clip.id;
        track.clips.push(clip);
        Ok(id)
    }

    pub fn delete_clip(&mut self, track_id: u64, clip_id: u64) -> Result<Clip> {
        let track = self.track_mut(track_id).context("Track not found")?;
        let index = track
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .context("Clip not found")?;
        Ok(track.clips.remove(index))
    }

    /// Change clip bounds without changing source events. Extending left cannot
    /// expose time before the source begins; extending right may reveal silence.
    pub fn trim_clip(
        &mut self,
        track_id: u64,
        clip_id: u64,
        new_start: f64,
        new_length: f64,
    ) -> Result<()> {
        ensure!(valid_range(new_start, new_length), "Invalid clip bounds");
        let clip = self.clip_mut(track_id, clip_id).context("Clip not found")?;
        let delta = new_start - clip.start;
        let new_offset = if matches!(clip.content, ClipContent::Audio { reverse: true, .. }) {
            clip.offset + (clip.length - delta - new_length) * clip.source_rate
        } else {
            clip.offset + delta * clip.source_rate
        };
        ensure!(
            valid_range(new_offset, new_length * clip.source_rate),
            "Trim extends before the source or overflows its range"
        );
        if let ClipContent::Audio { envelope, .. } = &mut clip.content {
            let original = envelope.unwrap_or(AudioEnvelope {
                offset: 0.0,
                length: clip.length * clip.source_rate,
            });
            ensure!(
                (original.offset + delta * clip.source_rate).abs() <= MAX_BEAT,
                "Trim exceeds the envelope range"
            );
            *envelope = Some(AudioEnvelope {
                offset: original.offset + delta * clip.source_rate,
                ..original
            });
        }
        clip.offset = new_offset;
        clip.start = new_start;
        clip.length = new_length;
        clip.constrain_fades();
        Ok(())
    }

    /// Change timing without changing source data, placement, or ghost membership.
    pub fn stretch_clip(&mut self, track_id: u64, clip_id: u64, new_length: f64) -> Result<()> {
        let clip = self.clip_mut(track_id, clip_id).context("Clip not found")?;
        ensure!(
            valid_range(clip.start, new_length),
            "Invalid stretch bounds"
        );
        let rate = clip.source_rate * clip.length / new_length;
        ensure!(
            rate.is_finite() && (MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&rate),
            "Stretch must stay between 25% and 400% of source speed"
        );
        // Division/multiplication can move an extreme source boundary by one
        // floating-point ULP. Reject before mutation rather than making a
        // previously valid document impossible to save.
        ensure!(
            valid_range(clip.offset, new_length * rate),
            "Stretch exceeds the supported source range"
        );
        if new_length == clip.length {
            return Ok(());
        }
        let source_length = clip.source_length();
        if let ClipContent::Audio { envelope, .. } = &mut clip.content {
            // Fades remain anchored to the exact original source interval,
            // including a full-length fade at a floating-point boundary.
            envelope.get_or_insert(AudioEnvelope {
                offset: 0.0,
                length: source_length,
            });
        }
        clip.length = new_length;
        clip.source_rate = rate;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.tempo.is_finite() && (20.0..=400.0).contains(&self.tempo),
            "Tempo must be between 20 and 400 BPM"
        );
        ensure!(
            (1..=32).contains(&self.beats_per_bar),
            "Invalid time signature numerator"
        );
        ensure!(
            nonnegative(self.loop_start)
                && nonnegative(self.loop_end)
                && self.loop_end > self.loop_start,
            "Invalid loop range"
        );
        self.validate_linked_clips()?;
        let mut ids = HashSet::new();
        let mut link_groups = HashSet::new();
        let mut midi_groups = HashSet::new();
        let mut automation_targets = HashSet::new();
        let mut check_id = |id| -> Result<()> {
            ensure!(
                id > 0 && id < u64::MAX && ids.insert(id),
                "Duplicate or invalid object ID: {id}"
            );
            Ok(())
        };
        for track in &self.tracks {
            check_id(track.id)?;
            if let Some(lane) = &track.automation {
                ensure!(
                    track.kind == TrackKind::Automation,
                    "Only automation tracks may contain automation lanes"
                );
                lane.validate()?;
                lane.target.validate(self)?;
                ensure!(
                    automation_targets.insert(lane.target),
                    "More than one track automates the same parameter"
                );
                for point in &lane.points {
                    check_id(point.id)?;
                }
            } else {
                ensure!(
                    track.kind != TrackKind::Automation,
                    "Automation track has no lane"
                );
            }
            ensure!(
                track.gain.is_finite() && track.gain >= 0.0,
                "Invalid track gain"
            );
            ensure!(
                track.pan.is_finite() && (-1.0..=1.0).contains(&track.pan),
                "Invalid track pan"
            );
            if let Instrument::Vst3 { path, class_id } = &track.instrument {
                ensure!(!path.is_empty(), "Missing VST3 path");
                validate_vst_class_id(class_id.as_deref())?;
            }
            validate_plugin_state(&track.instrument_state, &track.instrument_parameters)
                .context("Invalid instrument state")?;
            ensure!(
                (track.instrument_state.is_empty() && track.instrument_parameters.is_empty())
                    || (track.kind == TrackKind::Midi
                        && matches!(track.instrument, Instrument::Vst3 { .. })),
                "Only VST3 MIDI instruments may contain saved state or parameter overrides"
            );
            for slot in &track.inserts {
                check_id(slot.id)?;
                slot.effect.validate()?;
                validate_plugin_state(&slot.state, &slot.parameters)
                    .with_context(|| format!("Invalid insert state: {}", slot.name))?;
            }
            for clip in &track.clips {
                check_id(clip.id)?;
                if let Some(group) = clip.link_group
                    && link_groups.insert(group)
                {
                    check_id(group)?;
                }
                ensure!(valid_range(clip.start, clip.length), "Invalid clip bounds");
                ensure!(
                    valid_range(clip.offset, clip.source_length()),
                    "Invalid source offset"
                );
                ensure!(
                    clip.source_rate.is_finite()
                        && (MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&clip.source_rate),
                    "Invalid clip source rate"
                );
                match &clip.content {
                    ClipContent::Midi { notes } => {
                        ensure!(
                            track.kind == TrackKind::Midi,
                            "MIDI clip is on an audio track"
                        );
                        let shared_notes = clip
                            .link_group
                            .is_some_and(|group| !midi_groups.insert(group));
                        for note in notes {
                            if !shared_notes {
                                check_id(note.id)?;
                            }
                            ensure!(
                                note.pitch <= 127 && (1..=127).contains(&note.velocity),
                                "Invalid MIDI pitch or velocity"
                            );
                            ensure!(
                                valid_range(note.start, note.length),
                                "Invalid MIDI note bounds"
                            );
                        }
                    }
                    ClipContent::Audio {
                        path,
                        processing,
                        gain,
                        fade_in,
                        fade_out,
                        envelope,
                        crossfade,
                        ..
                    } => {
                        ensure!(
                            track.kind == TrackKind::Audio,
                            "Audio clip is on a MIDI track"
                        );
                        ensure!(!path.is_empty(), "Missing audio source path");
                        processing.validate()?;
                        if let Some(crossfade) = crossfade {
                            crossfade.validate()?;
                        }
                        ensure!(gain.is_finite() && *gain >= 0.0, "Invalid audio gain");
                        let envelope_length =
                            envelope.map_or(clip.source_length(), |envelope| envelope.length);
                        ensure!(
                            envelope.is_none_or(|envelope| envelope.offset.is_finite()
                                && envelope.offset.abs() <= MAX_BEAT
                                && envelope.length.is_finite()
                                && envelope.length > 0.0
                                && envelope.length <= MAX_BEAT),
                            "Invalid audio envelope region"
                        );
                        ensure!(
                            nonnegative(*fade_in)
                                && nonnegative(*fade_out)
                                && *fade_in <= envelope_length
                                && *fade_out <= envelope_length,
                            "Invalid audio fades"
                        );
                    }
                }
            }
        }
        self.routing_order()?;
        Ok(())
    }

    /// Atomically save a portable ZIP bundle for `.rill`, or legacy JSON for other
    /// extensions. Bundles include referenced WAV assets and saved plug-in state.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        if crate::archive::is_bundle(path) {
            return crate::archive::save(self, path);
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let filename = path
            .file_name()
            .context("Project destination needs a filename")?
            .to_string_lossy();
        let temporary = parent.join(format!(
            ".{filename}.{}.{}.tmp",
            std::process::id(),
            new_id()
        ));
        let result = (|| -> Result<()> {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .context("Create temporary project")?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer_pretty(
                &mut writer,
                &ProjectFile {
                    version: 3,
                    project: self.clone(),
                },
            )?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            fs::rename(&temporary, path).context("Replace project file")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_assets(path, std::env::temp_dir().join("rill-project-assets"))
    }

    /// Load a project, extracting portable bundle assets into a private persistent
    /// directory under `scratch_root`. Failed loads remove incomplete extraction.
    pub fn load_with_assets(
        path: impl AsRef<Path>,
        scratch_root: impl AsRef<Path>,
    ) -> Result<Self> {
        let path = path.as_ref();
        if crate::archive::is_bundle(path) {
            return crate::archive::load(path, scratch_root.as_ref());
        }
        let file =
            fs::File::open(path).with_context(|| format!("Open project {}", path.display()))?;
        let envelope: ProjectFile =
            serde_json::from_reader(BufReader::new(file)).context("Read project JSON")?;
        ensure!(
            (1..=3).contains(&envelope.version),
            "Unsupported project version {}",
            envelope.version
        );
        let base = fs::canonicalize(path)?
            .parent()
            .context("Project has no parent directory")?
            .to_path_buf();
        envelope.project.finish_loading(&base, true)
    }

    pub(crate) fn finish_loading(
        mut self,
        base: &Path,
        resolve_plugin_paths: bool,
    ) -> Result<Self> {
        self.validate()?;
        let mut max_id = 0;
        for track in &mut self.tracks {
            max_id = max_id.max(track.id);
            if let Some(lane) = &track.automation {
                for point in &lane.points {
                    max_id = max_id.max(point.id);
                }
            }
            if resolve_plugin_paths && let Instrument::Vst3 { path, .. } = &mut track.instrument {
                absolutize(path, base);
            }
            for slot in &mut track.inserts {
                max_id = max_id.max(slot.id);
                let Effect::Vst3 { path, .. } = &mut slot.effect;
                if resolve_plugin_paths {
                    absolutize(path, base);
                }
            }
            for clip in &mut track.clips {
                max_id = max_id.max(clip.id).max(clip.link_group.unwrap_or(0));
                match &mut clip.content {
                    ClipContent::Midi { notes } => {
                        for note in notes {
                            max_id = max_id.max(note.id);
                        }
                    }
                    ClipContent::Audio { path, .. } => absolutize(path, base),
                }
            }
        }
        NEXT_ID.fetch_max(max_id + 1, Ordering::Relaxed);
        Ok(self)
    }

    pub fn demo() -> Self {
        let mut project = Self {
            name: "Midnight circuit".into(),
            tempo: 112.0,
            loop_enabled: true,
            ..Self::new()
        };
        let mut chords = Track::new("Glass chords", TrackKind::Midi);
        chords.color = [160, 137, 246];
        chords.gain = 0.34;
        let mut chord_notes = Vec::new();
        for (bar, pitches) in [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]]
            .iter()
            .enumerate()
        {
            for pitch in pitches {
                chord_notes.push(Note::new(*pitch, bar as f64 * 4.0, 3.5, 84));
            }
        }
        chords
            .clips
            .push(Clip::midi("Am · F · C · G", 0.0, 16.0, chord_notes));
        let mut bass = Track::new("Sub motion", TrackKind::Midi);
        bass.color = [86, 190, 170];
        bass.gain = 0.48;
        let mut bass_notes = Vec::new();
        for (bar, pitch) in [33, 29, 24, 31].iter().enumerate() {
            for beat in [0.0, 1.5, 2.5, 3.0] {
                bass_notes.push(Note::new(*pitch, bar as f64 * 4.0 + beat, 0.45, 103));
            }
        }
        bass.clips
            .push(Clip::midi("Pulse bass", 0.0, 16.0, bass_notes));
        let mut melody = Track::new("Signal lead", TrackKind::Midi);
        melody.color = [234, 176, 100];
        melody.gain = 0.24;
        let pitches = [
            69, 72, 76, 72, 69, 67, 64, 67, 69, 72, 79, 76, 74, 71, 67, 64,
        ];
        let notes = pitches
            .iter()
            .enumerate()
            .map(|(i, p)| Note::new(*p, i as f64 * 0.5, 0.3, 90))
            .collect();
        melody
            .clips
            .push(Clip::midi("Night sequence", 4.0, 8.0, notes));
        project.tracks = vec![
            chords,
            bass,
            melody,
            Track::new("Audio textures", TrackKind::Audio),
        ];
        project
    }
}

fn absolutize(path: &mut String, base: &Path) {
    if Path::new(path).is_relative() {
        *path = base.join(&*path).to_string_lossy().into_owned();
    }
}

fn nonnegative(value: f64) -> bool {
    value.is_finite() && (0.0..=MAX_BEAT).contains(&value)
}
fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_BEAT
}
fn valid_range(start: f64, length: f64) -> bool {
    nonnegative(start) && positive(length) && start + length <= MAX_BEAT && start + length > start
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ProjectFile {
    pub(crate) version: u32,
    pub(crate) project: Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scale {
    Chromatic,
    Major,
    Minor,
    Dorian,
    Pentatonic,
}

impl Scale {
    pub const ALL: [Self; 5] = [
        Self::Chromatic,
        Self::Major,
        Self::Minor,
        Self::Dorian,
        Self::Pentatonic,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Chromatic => "Chromatic",
            Self::Major => "Major",
            Self::Minor => "Natural minor",
            Self::Dorian => "Dorian",
            Self::Pentatonic => "Major pentatonic",
        }
    }
    pub fn contains(self, root: u8, pitch: u8) -> bool {
        let interval = ((u16::from(pitch) + 12 - u16::from(root % 12)) % 12) as u8;
        match self {
            Self::Chromatic => true,
            Self::Major => [0, 2, 4, 5, 7, 9, 11].contains(&interval),
            Self::Minor => [0, 2, 3, 5, 7, 8, 10].contains(&interval),
            Self::Dorian => [0, 2, 3, 5, 7, 9, 10].contains(&interval),
            Self::Pentatonic => [0, 2, 4, 7, 9].contains(&interval),
        }
    }
}

pub fn note_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(pitch % 12)],
        i16::from(pitch) / 12 - 1
    )
}

/// Validate note edits before accepting them into a project.
pub fn validate_note(note: &Note) -> Result<()> {
    if note.pitch > 127
        || note.velocity == 0
        || note.velocity > 127
        || !valid_range(note.start, note.length)
    {
        bail!("Invalid MIDI note");
    }
    Ok(())
}
