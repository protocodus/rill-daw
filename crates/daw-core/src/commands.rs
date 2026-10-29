//! Validated, atomic song edits shared by interactive and programmatic clients.
//! No operation accepts a filesystem path, serialized project, or opaque plug-in state.
use crate::*;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const MAX_BATCH_EDITS: usize = 128;
pub const MAX_BATCH_NOTES: usize = 8192;
/// Trusted host lookup from an existing source path to its duration in seconds.
pub type AudioSourceDurations<'a> = dyn Fn(&str) -> Option<f64> + 'a;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SongSession {
    pub id: u64,
    pub revision: u64,
}

impl Default for SongSession {
    fn default() -> Self {
        Self {
            id: new_id(),
            revision: 0,
        }
    }
}
impl SongSession {
    /// Advance on every committed document change, including Undo and Redo.
    pub fn bump(&mut self) {
        if let Some(next) = self.revision.checked_add(1) {
            self.revision = next;
        } else {
            self.reset();
        }
    }
    /// Revoke all old handles when opening, creating, or recovering a document.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Numeric existing ID or a batch-local key returned by an earlier creation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EntityRef {
    Id(u64),
    Key(String),
}
impl From<u64> for EntityRef {
    fn from(value: u64) -> Self {
        Self::Id(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditableTrackKind {
    Audio,
    Midi,
    Bus,
    Folder,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteData {
    pub pitch: u8,
    /// Source beats within the clip, independent of arrangement start.
    pub start: f64,
    pub length: f64,
    pub velocity: u8,
}
impl NoteData {
    pub fn into_note(&self) -> Result<Note> {
        let note = Note::new(self.pitch, self.start, self.length, self.velocity);
        validate_note(&note)?;
        Ok(note)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteChange {
    pub id: u64,
    #[serde(default)]
    pub pitch: Option<u8>,
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub length: Option<f64>,
    #[serde(default)]
    pub velocity: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MixChange {
    pub track: EntityRef,
    /// Linear gain, including exact zero. Mutually exclusive with gain_db.
    #[serde(default)]
    pub gain: Option<f32>,
    #[serde(default)]
    pub gain_db: Option<f64>,
    #[serde(default)]
    pub pan: Option<f32>,
    #[serde(default)]
    pub mute: Option<bool>,
    #[serde(default)]
    pub solo: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum SongEdit {
    RenameSong {
        name: String,
    },
    SetTempo {
        bpm: f64,
    },
    SetMeter {
        beats_per_bar: u8,
    },
    SetLoop {
        enabled: bool,
        start: f64,
        end: f64,
    },
    SetMetronome {
        enabled: bool,
        gain: f32,
        precount_bars: u8,
    },
    AddTrack {
        kind: EditableTrackKind,
        name: String,
        #[serde(default)]
        key: Option<String>,
    },
    DeleteTrack {
        track: EntityRef,
    },
    ReorderTrack {
        track: EntityRef,
        index: usize,
    },
    MoveTrack {
        track: EntityRef,
        target: EntityRef,
        placement: TrackPlacement,
    },
    RenameTrack {
        track: EntityRef,
        name: String,
    },
    SetAudioInput {
        track: EntityRef,
        input: AudioInputChannels,
    },
    SetMidiInput {
        track: EntityRef,
        input: MidiInputChannel,
    },
    ClearTrackInput {
        track: EntityRef,
    },
    SetTrackColor {
        track: EntityRef,
        color: [u8; 3],
    },
    SetMix {
        channels: Vec<MixChange>,
    },
    SetOutput {
        track: EntityRef,
        bus: Option<EntityRef>,
    },
    SetFolder {
        track: EntityRef,
        folder: Option<EntityRef>,
    },
    AddSend {
        track: EntityRef,
        bus: EntityRef,
        gain: f32,
        pre_fader: bool,
    },
    UpdateSend {
        track: EntityRef,
        bus: EntityRef,
        gain: f32,
        pre_fader: bool,
    },
    RemoveSend {
        track: EntityRef,
        bus: EntityRef,
    },
    AddMidiClip {
        track: EntityRef,
        name: String,
        start: f64,
        length: f64,
        notes: Vec<NoteData>,
        #[serde(default)]
        key: Option<String>,
    },
    RenameClip {
        track: EntityRef,
        clip: EntityRef,
        name: String,
    },
    MoveClip {
        track: EntityRef,
        clip: EntityRef,
        start: f64,
    },
    MoveClipToTrack {
        track: EntityRef,
        clip: EntityRef,
        destination: EntityRef,
        start: f64,
    },
    TrimClip {
        track: EntityRef,
        clip: EntityRef,
        start: f64,
        length: f64,
    },
    StretchClip {
        track: EntityRef,
        clip: EntityRef,
        length: f64,
    },
    SplitClip {
        track: EntityRef,
        clip: EntityRef,
        beat: f64,
        #[serde(default)]
        key: Option<String>,
    },
    DuplicateClip {
        track: EntityRef,
        clip: EntityRef,
        start: f64,
        #[serde(default)]
        key: Option<String>,
    },
    GhostClip {
        track: EntityRef,
        clip: EntityRef,
        #[serde(default)]
        key: Option<String>,
    },
    MakeIndependent {
        track: EntityRef,
        clip: EntityRef,
    },
    DeleteClip {
        track: EntityRef,
        clip: EntityRef,
    },
    JoinMidiClips {
        track: EntityRef,
        clips: Vec<EntityRef>,
        #[serde(default)]
        key: Option<String>,
    },
    AddNotes {
        track: EntityRef,
        clip: EntityRef,
        notes: Vec<NoteData>,
    },
    UpdateNotes {
        track: EntityRef,
        clip: EntityRef,
        notes: Vec<NoteChange>,
    },
    DeleteNotes {
        track: EntityRef,
        clip: EntityRef,
        note_ids: Vec<u64>,
    },
    TransformNotes {
        track: EntityRef,
        clip: EntityRef,
        #[serde(default)]
        note_ids: Vec<u64>,
        beats: f64,
        semitones: i16,
    },
    QuantizeNotes {
        track: EntityRef,
        clip: EntityRef,
        #[serde(default)]
        note_ids: Vec<u64>,
        grid: f64,
        strength: f32,
        swing: f32,
        ends: bool,
    },
    RandomizeNotes {
        track: EntityRef,
        clip: EntityRef,
        #[serde(default)]
        note_ids: Vec<u64>,
        #[serde(default = "default_randomize_velocity")]
        velocity: u8,
        #[serde(default)]
        timing: f64,
        #[serde(default = "default_randomize_seed")]
        seed: u32,
    },
    LegatoNotes {
        track: EntityRef,
        clip: EntityRef,
        #[serde(default)]
        note_ids: Vec<u64>,
        #[serde(default)]
        mode: LegatoMode,
    },
    SplitNote {
        track: EntityRef,
        clip: EntityRef,
        note: u64,
        beat: f64,
    },
    DuplicateNote {
        track: EntityRef,
        clip: EntityRef,
        note: u64,
    },
    GlueNotes {
        track: EntityRef,
        clip: EntityRef,
        note_ids: Vec<u64>,
    },
    SetAudioGain {
        track: EntityRef,
        clip: EntityRef,
        gain: f32,
    },
    SetAudioFades {
        track: EntityRef,
        clip: EntityRef,
        fade_in: f64,
        fade_out: f64,
        in_curve: FadeCurve,
        out_curve: FadeCurve,
    },
    ReverseAudio {
        track: EntityRef,
        clip: EntityRef,
    },
    SetAudioProcessing {
        track: EntityRef,
        clip: EntityRef,
        processing: AudioProcessing,
    },
    SetCrossfade {
        track: EntityRef,
        clip: EntityRef,
        crossfade: Option<AudioCrossfade>,
    },
    SlipAudio {
        track: EntityRef,
        clip: EntityRef,
        delta: f64,
        #[serde(default)]
        source_beats: f64,
    },
    DeleteAudioRange {
        track: EntityRef,
        clip: EntityRef,
        start: f64,
        end: f64,
    },
    RippleDeleteAudio {
        track: EntityRef,
        start: f64,
        end: f64,
    },
    CopyAudioRange {
        track: EntityRef,
        clip: EntityRef,
        start: f64,
        end: f64,
        destination: EntityRef,
        at: f64,
    },
    SplitAudioTransients {
        track: EntityRef,
        clip: EntityRef,
        beats: Vec<f64>,
    },
    StripAudioSilence {
        track: EntityRef,
        clip: EntityRef,
        start: f64,
        end: f64,
        retained: Vec<(f64, f64)>,
    },
    RemoveInsert {
        track: EntityRef,
        effect: u64,
    },
    RemoveInstrument {
        track: EntityRef,
    },
    SetInsertEnabled {
        track: EntityRef,
        effect: u64,
        enabled: bool,
    },
    ReorderInsert {
        track: EntityRef,
        effect: u64,
        index: usize,
    },
    SetPluginParameters {
        track: EntityRef,
        effect: Option<u64>,
        parameters: Vec<PluginParameterValue>,
    },
    AddAutomation {
        name: String,
        target: AutomationTarget,
        value: f32,
        #[serde(default)]
        key: Option<String>,
    },
    SetAutomationInterpolation {
        track: EntityRef,
        interpolation: Interpolation,
    },
    SetAutomationPointCurve {
        track: EntityRef,
        point: u64,
        curve: f32,
    },
    SetAutomationOverride {
        track: EntityRef,
        value: Option<f32>,
    },
    UpsertAutomationPoints {
        track: EntityRef,
        points: Vec<(f64, f32)>,
    },
    MoveAutomationPoint {
        track: EntityRef,
        point: u64,
        beat: f64,
        value: f32,
    },
    RemoveAutomationPoints {
        track: EntityRef,
        point_ids: Vec<u64>,
    },
    GroupAudioTakes {
        track: EntityRef,
        clip_ids: Vec<u64>,
        #[serde(default)]
        key: Option<String>,
    },
    AddAudioTake {
        track: EntityRef,
        group: EntityRef,
        clip: EntityRef,
    },
    SelectAudioTake {
        track: EntityRef,
        group: EntityRef,
        source: EntityRef,
        range: Option<(f64, f64)>,
        crossfade_beats: f64,
    },
    DetachAudioTakes {
        track: EntityRef,
        group: EntityRef,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditBatch {
    pub label: String,
    pub edits: Vec<SongEdit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedEntity {
    pub kind: String,
    pub id: u64,
    pub key: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditResult {
    pub created: Vec<CreatedEntity>,
    pub changed: bool,
}

fn text(value: &str) -> Result<()> {
    ensure!(
        value.chars().count() <= 1024 && !value.contains('\0'),
        "Text must contain at most 1024 characters and no NUL"
    );
    Ok(())
}
fn resolve(value: &EntityRef, refs: &HashMap<String, u64>) -> Result<u64> {
    match value {
        EntityRef::Id(id) => {
            ensure!(*id > 0 && *id < u64::MAX, "Invalid object ID");
            Ok(*id)
        }
        EntityRef::Key(key) => refs
            .get(key)
            .copied()
            .context("Unknown batch-local object key"),
    }
}
fn record(
    result: &mut EditResult,
    refs: &mut HashMap<String, u64>,
    kind: &str,
    id: u64,
    key: &Option<String>,
) -> Result<()> {
    if let Some(key) = key {
        ensure!(
            !key.is_empty()
                && key.len() <= 64
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "Object keys must contain 1–64 letters, digits, underscores, or dashes"
        );
        ensure!(!refs.contains_key(key), "Duplicate batch-local key");
        refs.insert(key.clone(), id);
    }
    result.created.push(CreatedEntity {
        kind: kind.into(),
        id,
        key: key.clone(),
    });
    Ok(())
}
fn selected_notes(project: &Project, track: u64, clip: u64, ids: &[u64]) -> Result<HashSet<u64>> {
    ensure!(ids.len() <= MAX_BATCH_NOTES, "Too many selected notes");
    let notes = project
        .midi_notes(track, clip)
        .context("MIDI clip not found")?;
    if ids.is_empty() {
        return Ok(notes.iter().map(|note| note.id).collect());
    }
    let selected: HashSet<_> = ids.iter().copied().collect();
    ensure!(selected.len() == ids.len(), "Duplicate selected note ID");
    ensure!(
        selected
            .iter()
            .all(|id| notes.iter().any(|note| note.id == *id)),
        "Selected note not found"
    );
    Ok(selected)
}

fn default_randomize_velocity() -> u8 {
    RandomizeSettings::default().velocity
}

fn default_randomize_seed() -> u32 {
    RandomizeSettings::default().seed
}

impl EditBatch {
    /// Validate the complete candidate before publishing it; failures leave the
    /// original untouched. The host records the before-state once in history.
    /// Callers accepting untrusted slip bounds should use
    /// `apply_with_source_durations` with authoritative asset metadata instead.
    pub fn apply(&self, project: &mut Project) -> Result<EditResult> {
        self.apply_inner(project, None)
    }

    /// Resolve actual source durations in seconds from a trusted host cache.
    /// Source paths stay inside the host. This also validates batch-local clip
    /// references and accounts for tempo edits earlier in the same batch.
    pub fn apply_with_source_durations(
        &self,
        project: &mut Project,
        durations: &AudioSourceDurations<'_>,
    ) -> Result<EditResult> {
        self.apply_inner(project, Some(durations))
    }

    fn apply_inner(
        &self,
        project: &mut Project,
        durations: Option<&AudioSourceDurations<'_>>,
    ) -> Result<EditResult> {
        text(&self.label)?;
        ensure!(
            !self.edits.is_empty() && self.edits.len() <= MAX_BATCH_EDITS,
            "An edit batch needs 1–128 operations"
        );
        let note_count: usize = self
            .edits
            .iter()
            .map(|edit| match edit {
                SongEdit::AddMidiClip { notes, .. } | SongEdit::AddNotes { notes, .. } => {
                    notes.len()
                }
                SongEdit::UpdateNotes { notes, .. } => notes.len(),
                _ => 0,
            })
            .sum();
        ensure!(note_count <= MAX_BATCH_NOTES, "Too many notes in one batch");
        let mut candidate = project.clone();
        let mut refs = HashMap::new();
        let mut result = EditResult::default();
        for (index, edit) in self.edits.iter().enumerate() {
            apply_one(edit, &mut candidate, &mut refs, &mut result, durations)
                .with_context(|| format!("Edit {}", index + 1))?;
        }
        candidate.prune_automation();
        candidate.validate()?;
        result.changed = candidate != *project;
        if result.changed {
            *project = candidate;
        }
        Ok(result)
    }
}

fn apply_one(
    edit: &SongEdit,
    p: &mut Project,
    refs: &mut HashMap<String, u64>,
    result: &mut EditResult,
    durations: Option<&AudioSourceDurations<'_>>,
) -> Result<()> {
    let r = |value: &EntityRef| resolve(value, refs);
    match edit {
        SongEdit::RenameSong { name } => {
            text(name)?;
            p.name.clone_from(name);
        }
        SongEdit::SetTempo { bpm } => p.tempo = *bpm,
        SongEdit::SetMeter { beats_per_bar } => p.beats_per_bar = *beats_per_bar,
        SongEdit::SetLoop {
            enabled,
            start,
            end,
        } => {
            p.loop_enabled = *enabled;
            p.loop_start = *start;
            p.loop_end = *end;
        }
        SongEdit::SetMetronome {
            enabled,
            gain,
            precount_bars,
        } => {
            p.metronome_enabled = *enabled;
            p.metronome_gain = *gain;
            p.precount_bars = *precount_bars;
        }
        SongEdit::AddTrack { kind, name, key } => {
            text(name)?;
            let id = match kind {
                EditableTrackKind::Audio => p.add_audio_track(name),
                EditableTrackKind::Midi => p.add_midi_track(name),
                EditableTrackKind::Bus => p.add_bus(name),
                EditableTrackKind::Folder => p.add_folder(name),
            };
            record(result, refs, "track", id, key)?;
        }
        SongEdit::DeleteTrack { track } => {
            p.delete_track(r(track)?)?;
        }
        SongEdit::ReorderTrack { track, index } => {
            ensure!(*index < p.tracks.len(), "Track index out of range");
            let id = r(track)?;
            let from = p
                .tracks
                .iter()
                .position(|t| t.id == id)
                .context("Track not found")?;
            let moved = p.tracks.remove(from);
            p.tracks.insert(*index, moved);
        }
        SongEdit::MoveTrack {
            track,
            target,
            placement,
        } => {
            p.reorder_track(r(track)?, r(target)?, *placement)?;
        }
        SongEdit::RenameTrack { track, name } => {
            text(name)?;
            p.track_mut(r(track)?)
                .context("Track not found")?
                .name
                .clone_from(name);
        }
        SongEdit::SetAudioInput { track, input } => {
            let track = p.track_mut(r(track)?).context("Track not found")?;
            ensure!(track.kind == TrackKind::Audio, "Choose an audio track");
            track.audio_input = Some(*input);
        }
        SongEdit::SetMidiInput { track, input } => {
            let track = p.track_mut(r(track)?).context("Track not found")?;
            ensure!(track.kind == TrackKind::Midi, "Choose a MIDI track");
            track.midi_input = Some(*input);
        }
        SongEdit::ClearTrackInput { track } => {
            let track = p.track_mut(r(track)?).context("Track not found")?;
            ensure!(
                matches!(track.kind, TrackKind::Audio | TrackKind::Midi),
                "Choose an audio or MIDI track"
            );
            track.audio_input = None;
            track.midi_input = None;
        }
        SongEdit::SetTrackColor { track, color } => {
            p.track_mut(r(track)?).context("Track not found")?.color = *color
        }
        SongEdit::SetMix { channels } => {
            ensure!(channels.len() <= MAX_BATCH_EDITS, "Too many mix channels");
            let mut seen = HashSet::new();
            for channel in channels {
                ensure!(
                    channel.gain.is_none() || channel.gain_db.is_none(),
                    "Choose linear gain or gain_db, not both"
                );
                let id = r(&channel.track)?;
                ensure!(seen.insert(id), "Duplicate mix channel");
                let track = p.track_mut(id).context("Track not found")?;
                if let Some(gain) = channel.gain {
                    ensure!(
                        gain.is_finite() && (0.0..=16.0).contains(&gain),
                        "Linear gain must be between 0 and 16"
                    );
                    track.gain = gain;
                }
                if let Some(db) = channel.gain_db {
                    ensure!(
                        db.is_finite() && (-120.0..=24.0).contains(&db),
                        "Gain must be between -120 and 24 dB"
                    );
                    track.gain = 10.0_f64.powf(db / 20.0) as f32;
                }
                if let Some(pan) = channel.pan {
                    track.pan = pan;
                }
                if let Some(mute) = channel.mute {
                    track.mute = mute;
                }
                if let Some(solo) = channel.solo {
                    track.solo = solo;
                }
            }
        }
        SongEdit::SetOutput { track, bus } => {
            p.set_output(r(track)?, bus.as_ref().map(r).transpose()?)?
        }
        SongEdit::SetFolder { track, folder } => {
            p.set_parent_folder(r(track)?, folder.as_ref().map(r).transpose()?)?
        }
        SongEdit::AddSend {
            track,
            bus,
            gain,
            pre_fader,
        } => p.add_send(r(track)?, r(bus)?, *gain, *pre_fader)?,
        SongEdit::UpdateSend {
            track,
            bus,
            gain,
            pre_fader,
        } => {
            let bus = r(bus)?;
            let send = p
                .track_mut(r(track)?)
                .context("Track not found")?
                .sends
                .iter_mut()
                .find(|s| s.target_bus == bus)
                .context("Send not found")?;
            send.gain = *gain;
            send.pre_fader = *pre_fader;
        }
        SongEdit::RemoveSend { track, bus } => {
            p.remove_send(r(track)?, r(bus)?)?;
        }
        SongEdit::AddMidiClip {
            track,
            name,
            start,
            length,
            notes,
            key,
        } => {
            text(name)?;
            let track = p.track_mut(r(track)?).context("Track not found")?;
            ensure!(track.kind == TrackKind::Midi, "Choose a MIDI track");
            let notes = notes
                .iter()
                .map(NoteData::into_note)
                .collect::<Result<Vec<_>>>()?;
            let clip = Clip::midi(name, *start, *length, notes);
            record(result, refs, "clip", clip.id, key)?;
            track.clips.push(clip);
        }
        SongEdit::RenameClip { track, clip, name } => {
            text(name)?;
            let (track, clip) = (r(track)?, r(clip)?);
            p.clip_mut(track, clip)
                .context("Clip not found")?
                .name
                .clone_from(name);
            p.synchronize_linked_clip(track, clip)?;
        }
        SongEdit::MoveClip { track, clip, start } => p.move_clip(r(track)?, r(clip)?, *start)?,
        SongEdit::MoveClipToTrack {
            track,
            clip,
            destination,
            start,
        } => {
            let (track, clip, destination) = (r(track)?, r(clip)?, r(destination)?);
            ensure!(
                p.track(destination).is_some(),
                "Destination track not found"
            );
            p.move_clip(track, clip, *start)?;
            if track != destination {
                let mut clip = p.delete_clip(track, clip)?;
                // Takes belong to one track. Transfer the audible region
                // independently, retaining its sources on the original track.
                clip.audio_take = None;
                p.track_mut(destination)
                    .context("Destination track not found")?
                    .clips
                    .push(clip);
            }
        }
        SongEdit::TrimClip {
            track,
            clip,
            start,
            length,
        } => p.trim_clip(r(track)?, r(clip)?, *start, *length)?,
        SongEdit::StretchClip {
            track,
            clip,
            length,
        } => p.stretch_clip(r(track)?, r(clip)?, *length)?,
        SongEdit::SplitClip {
            track,
            clip,
            beat,
            key,
        } => {
            let id = p.split_clip(r(track)?, r(clip)?, *beat)?;
            record(result, refs, "clip", id, key)?;
        }
        SongEdit::DuplicateClip {
            track,
            clip,
            start,
            key,
        } => {
            let id = p.duplicate_clip_at(r(track)?, r(clip)?, *start)?;
            record(result, refs, "clip", id, key)?;
        }
        SongEdit::GhostClip { track, clip, key } => {
            let id = p.clone_clip(r(track)?, r(clip)?)?;
            record(result, refs, "clip", id, key)?;
        }
        SongEdit::MakeIndependent { track, clip } => {
            p.make_clip_independent(r(track)?, r(clip)?)?
        }
        SongEdit::DeleteClip { track, clip } => {
            p.delete_clip(r(track)?, r(clip)?)?;
        }
        SongEdit::JoinMidiClips { track, clips, key } => {
            ensure!(clips.len() <= MAX_BATCH_EDITS, "Too many clips to join");
            let clips = clips.iter().map(r).collect::<Result<Vec<_>>>()?;
            let id = p.join_midi_clips(r(track)?, &clips)?;
            record(result, refs, "clip", id, key)?;
        }
        SongEdit::AddNotes { track, clip, notes } => {
            let (track, clip) = (r(track)?, r(clip)?);
            let added = notes
                .iter()
                .map(NoteData::into_note)
                .collect::<Result<Vec<_>>>()?;
            p.midi_notes_mut(track, clip)
                .context("MIDI clip not found")?
                .extend(added);
            p.synchronize_linked_clip(track, clip)?;
        }
        SongEdit::UpdateNotes { track, clip, notes } => {
            let (track, clip) = (r(track)?, r(clip)?);
            let data = p
                .midi_notes_mut(track, clip)
                .context("MIDI clip not found")?;
            let mut seen = HashSet::new();
            for change in notes {
                ensure!(seen.insert(change.id), "Duplicate note change");
                let note = data
                    .iter_mut()
                    .find(|n| n.id == change.id)
                    .context("Note not found")?;
                if let Some(v) = change.pitch {
                    note.pitch = v;
                }
                if let Some(v) = change.start {
                    note.start = v;
                }
                if let Some(v) = change.length {
                    note.length = v;
                }
                if let Some(v) = change.velocity {
                    note.velocity = v;
                }
                validate_note(note)?;
            }
            p.synchronize_linked_clip(track, clip)?;
        }
        SongEdit::DeleteNotes {
            track,
            clip,
            note_ids,
        } => {
            let (track, clip) = (r(track)?, r(clip)?);
            ensure!(!note_ids.is_empty(), "Select notes to delete");
            let selected = selected_notes(p, track, clip, note_ids)?;
            p.midi_notes_mut(track, clip)
                .context("MIDI clip not found")?
                .retain(|n| !selected.contains(&n.id));
            p.synchronize_linked_clip(track, clip)?;
        }
        SongEdit::TransformNotes {
            track,
            clip,
            note_ids,
            beats,
            semitones,
        } => {
            let (track, clip) = (r(track)?, r(clip)?);
            let selected = selected_notes(p, track, clip, note_ids)?;
            let source_beats = *beats * p.clip(track, clip).context("Clip not found")?.source_rate;
            let notes = move_midi_notes(
                p.midi_notes(track, clip).context("MIDI clip not found")?,
                &selected,
                source_beats,
                *semitones,
            )?;
            *p.midi_notes_mut(track, clip)
                .context("MIDI clip not found")? = notes;
            p.synchronize_linked_clip(track, clip)?;
        }
        SongEdit::QuantizeNotes {
            track,
            clip,
            note_ids,
            grid,
            strength,
            swing,
            ends,
        } => {
            let (track, clip) = (r(track)?, r(clip)?);
            let selected = selected_notes(p, track, clip, note_ids)?;
            let c = p.clip(track, clip).context("Clip not found")?;
            // Quantize audible arrangement timing, then map back to source time.
            let source = p
                .midi_notes(track, clip)
                .context("MIDI clip not found")?
                .to_vec();
            let mut notes = source.clone();
            for note in &mut notes {
                note.start /= c.source_rate;
                note.length /= c.source_rate;
            }
            let mut notes = quantize_notes(
                &notes,
                &selected,
                c.start - c.offset / c.source_rate,
                QuantizeSettings {
                    grid: *grid,
                    strength: *strength,
                    swing: *swing,
                    ends: *ends,
                },
            )?;
            for (note, original) in notes.iter_mut().zip(&source) {
                if *strength == 0.0 || !selected.contains(&note.id) {
                    *note = original.clone();
                    continue;
                }
                note.start = if note.start == original.start / c.source_rate {
                    original.start
                } else {
                    note.start * c.source_rate
                };
                note.length = if note.length == original.length / c.source_rate {
                    original.length
                } else {
                    note.length * c.source_rate
                };
            }
            *p.midi_notes_mut(track, clip)
                .context("MIDI clip not found")? = notes;
            p.synchronize_linked_clip(track, clip)?;
        }
        SongEdit::RandomizeNotes {
            track,
            clip,
            note_ids,
            velocity,
            timing,
            seed,
        } => {
            let (track, clip) = (r(track)?, r(clip)?);
            let selected = selected_notes(p, track, clip, note_ids)?;
            let source_rate = p.clip(track, clip).context("Clip not found")?.source_rate;
            let source = p.midi_notes(track, clip).context("MIDI clip not found")?;
            let settings = RandomizeSettings {
                velocity: *velocity,
                timing: *timing,
                seed: *seed,
            };
            let notes = randomize_source_notes(source, &selected, settings, source_rate)?;
            *p.midi_notes_mut(track, clip)
                .context("MIDI clip not found")? = notes;
            p.synchronize_linked_clip(track, clip)?;
        }
        SongEdit::LegatoNotes {
            track,
            clip,
            note_ids,
            mode,
        } => {
            let (track, clip) = (r(track)?, r(clip)?);
            let selected = selected_notes(p, track, clip, note_ids)?;
            let notes = legato_notes_with_mode(
                p.midi_notes(track, clip).context("MIDI clip not found")?,
                &selected,
                *mode,
            )?;
            *p.midi_notes_mut(track, clip)
                .context("MIDI clip not found")? = notes;
            p.synchronize_linked_clip(track, clip)?;
        }
        SongEdit::SplitNote {
            track,
            clip,
            note,
            beat,
        } => {
            let id = p.split_note(r(track)?, r(clip)?, *note, *beat)?;
            record(result, refs, "note", id, &None)?;
        }
        SongEdit::DuplicateNote { track, clip, note } => {
            let id = p.duplicate_note(r(track)?, r(clip)?, *note)?;
            record(result, refs, "note", id, &None)?;
        }
        SongEdit::GlueNotes {
            track,
            clip,
            note_ids,
        } => {
            let id = p.glue_notes(r(track)?, r(clip)?, note_ids)?;
            record(result, refs, "note", id, &None)?;
        }
        SongEdit::SetAudioGain { track, clip, gain } => {
            p.set_audio_gain(r(track)?, r(clip)?, *gain)?
        }
        SongEdit::SetAudioFades {
            track,
            clip,
            fade_in,
            fade_out,
            in_curve,
            out_curve,
        } => p.set_audio_fades(
            r(track)?,
            r(clip)?,
            *fade_in,
            *fade_out,
            *in_curve,
            *out_curve,
        )?,
        SongEdit::ReverseAudio { track, clip } => p.reverse_audio_clip(r(track)?, r(clip)?)?,
        SongEdit::SetAudioProcessing {
            track,
            clip,
            processing,
        } => p.set_audio_processing(r(track)?, r(clip)?, *processing)?,
        SongEdit::SetCrossfade {
            track,
            clip,
            crossfade,
        } => p.set_audio_crossfade(r(track)?, r(clip)?, *crossfade)?,
        SongEdit::SlipAudio {
            track,
            clip,
            delta,
            source_beats,
        } => {
            let (track, clip) = (r(track)?, r(clip)?);
            let bounds = if let Some(durations) = durations {
                let ClipContent::Audio { path, .. } =
                    &p.clip(track, clip).context("Clip not found")?.content
                else {
                    anyhow::bail!("Choose an audio clip");
                };
                durations(path).context(
                    "Source duration is unavailable; prepare or analyze this audio source first",
                )? * p.tempo
                    / 60.0
            } else {
                *source_beats
            };
            p.slip_audio_clip(track, clip, *delta, bounds)?;
        }
        SongEdit::DeleteAudioRange {
            track,
            clip,
            start,
            end,
        } => {
            for id in p.delete_audio_range(r(track)?, r(clip)?, *start, *end)? {
                record(result, refs, "clip", id, &None)?;
            }
        }
        SongEdit::RippleDeleteAudio { track, start, end } => {
            for id in p.ripple_delete_audio_range(r(track)?, *start, *end)? {
                record(result, refs, "clip", id, &None)?;
            }
        }
        SongEdit::CopyAudioRange {
            track,
            clip,
            start,
            end,
            destination,
            at,
        } => {
            let copied = p.copy_audio_range(r(track)?, r(clip)?, *start, *end)?;
            let id = p.paste_audio_clip(r(destination)?, &copied, *at)?;
            record(result, refs, "clip", id, &None)?;
        }
        SongEdit::SplitAudioTransients { track, clip, beats } => {
            ensure!(beats.len() <= 2048, "Too many transients");
            for id in p.split_audio_transients(r(track)?, r(clip)?, beats)? {
                record(result, refs, "clip", id, &None)?;
            }
        }
        SongEdit::StripAudioSilence {
            track,
            clip,
            start,
            end,
            retained,
        } => {
            ensure!(retained.len() <= 2048, "Too many retained regions");
            for id in p.strip_audio_silence(r(track)?, r(clip)?, (*start, *end), retained)? {
                record(result, refs, "clip", id, &None)?;
            }
        }
        SongEdit::RemoveInsert { track, effect } => {
            p.remove_insert(r(track)?, *effect)?;
        }
        SongEdit::RemoveInstrument { track } => p.set_instrument(r(track)?, Instrument::None)?,
        SongEdit::SetInsertEnabled {
            track,
            effect,
            enabled,
        } => {
            p.track_mut(r(track)?)
                .context("Track not found")?
                .inserts
                .iter_mut()
                .find(|s| s.id == *effect)
                .context("Insert not found")?
                .enabled = *enabled;
        }
        SongEdit::ReorderInsert {
            track,
            effect,
            index,
        } => {
            let slots = &mut p.track_mut(r(track)?).context("Track not found")?.inserts;
            ensure!(*index < slots.len(), "Insert index out of range");
            let from = slots
                .iter()
                .position(|s| s.id == *effect)
                .context("Insert not found")?;
            let slot = slots.remove(from);
            slots.insert(*index, slot);
        }
        SongEdit::SetPluginParameters {
            track,
            effect,
            parameters,
        } => {
            ensure!(parameters.len() <= 4096, "Too many plug-in parameters");
            let mut seen = HashSet::new();
            for parameter in parameters {
                ensure!(
                    seen.insert(parameter.id)
                        && parameter.value.is_finite()
                        && (0.0..=1.0).contains(&parameter.value),
                    "Invalid or duplicate normalized parameter"
                );
            }
            let track = p.track_mut(r(track)?).context("Track not found")?;
            let values = if let Some(effect) = effect {
                &mut track
                    .inserts
                    .iter_mut()
                    .find(|s| s.id == *effect)
                    .context("Insert not found")?
                    .parameters
            } else {
                ensure!(
                    matches!(track.instrument, Instrument::Vst3 { .. }),
                    "Track has no plug-in instrument"
                );
                &mut track.instrument_parameters
            };
            for parameter in parameters {
                if let Some(existing) = values.iter_mut().find(|v| v.id == parameter.id) {
                    existing.value = parameter.value;
                } else {
                    values.push(parameter.clone());
                }
            }
            ensure!(
                values.len() <= 4096,
                "Too many stored plug-in parameter overrides"
            );
        }
        SongEdit::AddAutomation {
            name,
            target,
            value,
            key,
        } => {
            text(name)?;
            let id = p.add_automation_track_with_value(name, *target, *value)?;
            record(result, refs, "track", id, key)?;
        }
        SongEdit::SetAutomationInterpolation {
            track,
            interpolation,
        } => {
            p.automation_lane_mut(r(track)?)
                .context("Automation track not found")?
                .interpolation = *interpolation
        }
        SongEdit::UpsertAutomationPoints { track, points } => {
            ensure!(points.len() <= 4096, "Too many automation points");
            let track = r(track)?;
            for (beat, value) in points {
                let id = p.upsert_automation_point(track, *beat, *value)?;
                record(result, refs, "point", id, &None)?;
            }
        }
        SongEdit::SetAutomationPointCurve {
            track,
            point,
            curve,
        } => {
            p.set_automation_point_curve(r(track)?, *point, *curve)?;
        }
        SongEdit::SetAutomationOverride { track, value } => {
            p.set_automation_override(r(track)?, *value)?;
        }
        SongEdit::MoveAutomationPoint {
            track,
            point,
            beat,
            value,
        } => p.move_automation_point(r(track)?, *point, *beat, *value)?,
        SongEdit::RemoveAutomationPoints { track, point_ids } => {
            ensure!(point_ids.len() <= 4096, "Too many automation points");
            let track = r(track)?;
            for id in point_ids {
                p.remove_automation_point(track, *id)?;
            }
        }
        SongEdit::GroupAudioTakes {
            track,
            clip_ids,
            key,
        } => {
            ensure!(clip_ids.len() <= MAX_AUDIO_TAKES, "Too many takes");
            let id = p.group_audio_takes(r(track)?, clip_ids)?;
            record(result, refs, "take_group", id, key)?;
        }
        SongEdit::AddAudioTake { track, group, clip } => {
            p.add_audio_take(r(track)?, r(group)?, r(clip)?)?
        }
        SongEdit::SelectAudioTake {
            track,
            group,
            source,
            range,
            crossfade_beats,
        } => {
            let id =
                p.select_audio_take(r(track)?, r(group)?, r(source)?, *range, *crossfade_beats)?;
            record(result, refs, "clip", id, &None)?;
        }
        SongEdit::DetachAudioTakes { track, group } => {
            p.detach_audio_take_group(r(track)?, r(group)?)?
        }
    }
    Ok(())
}
