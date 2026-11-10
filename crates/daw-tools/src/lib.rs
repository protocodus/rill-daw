//! The provider-neutral song tool contract. The desktop app, embedded assistant,
//! and MCP adapter dispatch through the same typed commands and schemas.
use daw_core::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::HashSet, error::Error, fmt};

pub mod bounded_json;
mod composer;
mod query;
#[cfg(test)]
use query::track_summary;
use query::{Page, TrackQuery, bounded_text, clip_summary};
mod scope;
pub use scope::{ToolContext, ToolScope};

pub const MAX_ARGUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_QUERY_ITEMS: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolEffect {
    Read,
    Mutation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolProfile {
    #[default]
    CurrentSong,
    Application,
}
impl ToolSpec {
    pub fn effect(&self) -> ToolEffect {
        tool_effect(&self.name)
    }
}
/// Conservative effect classification for scheduling and revision fencing.
pub fn tool_effect(name: &str) -> ToolEffect {
    if composer::is_read(name) {
        return ToolEffect::Read;
    }
    match name {
        "get_song_context"
        | "list_tracks"
        | "get_track"
        | "get_clip"
        | "get_note_expressions"
        | "get_selection"
        | "list_plugins"
        | "audio_analyze"
        | "audio_analyze_clip"
        | "plugin_describe_parameters"
        | "list_device_presets"
        | "get_recording"
        | "document_status"
        | "get_setup" => ToolEffect::Read,
        _ => ToolEffect::Mutation,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolError {
    pub code: String,
    pub message: String,
}
impl ToolError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl Error for ToolError {}

pub trait ToolHost {
    /// A transport may only invoke tools published by its host. Application
    /// privileges are opt-in; a plain host can only edit its current song.
    fn tool_specs(&self) -> Vec<ToolSpec> {
        song_tool_specs()
    }
    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError>;
}

/// App-owned history, editor synchronization, session fencing and playback are
/// deliberately outside the pure command layer. A call is serialized on its
/// control thread; no network work occurs while borrowing this host.
pub trait SongToolHost {
    fn project(&self) -> &Project;
    fn song_session(&self) -> SongSession;
    fn apply_edits(&mut self, batch: EditBatch) -> Result<EditResult, ToolError>;
}

pub fn parse_arguments<T: DeserializeOwned>(arguments: Value) -> Result<T, ToolError> {
    if bounded_json::encoded_len(&arguments, MAX_ARGUMENT_BYTES).is_err() {
        return Err(ToolError::new(
            "invalid_arguments",
            "Tool arguments exceed 1 MiB",
        ));
    }
    serde_json::from_value(arguments)
        .map_err(|e| ToolError::new("invalid_arguments", e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyArgs {
    session_id: u64,
    expected_revision: u64,
    label: String,
    edits: Vec<SongEdit>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MidiPartArgs {
    session_id: u64,
    expected_revision: u64,
    #[serde(default)]
    track_id: Option<u64>,
    #[serde(default = "part_name")]
    name: String,
    start: f64,
    length: f64,
    notes: Vec<NoteData>,
}
fn part_name() -> String {
    "AI part".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MixArgs {
    session_id: u64,
    expected_revision: u64,
    channels: Vec<MixChange>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformArgs {
    session_id: u64,
    expected_revision: u64,
    track_id: u64,
    clip_id: u64,
    operation: String,
    #[serde(default)]
    note_ids: Vec<u64>,
    #[serde(default)]
    semitones: i16,
    #[serde(default)]
    beats: f64,
    #[serde(default = "quarter")]
    grid: f64,
    #[serde(default = "one")]
    strength: f32,
    #[serde(default = "half")]
    swing: f32,
    #[serde(default)]
    ends: bool,
    #[serde(default)]
    velocity: Option<u8>,
    #[serde(default)]
    mode: LegatoMode,
    #[serde(default)]
    timing: f64,
    #[serde(default = "randomize_seed")]
    seed: u32,
}
fn randomize_seed() -> u32 {
    1
}
fn quarter() -> f64 {
    0.25
}
fn one() -> f32 {
    1.0
}
fn half() -> f32 {
    0.5
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateArgs {
    session_id: u64,
    expected_revision: u64,
    #[serde(default)]
    track_id: Option<u64>,
    #[serde(default = "part_name")]
    name: String,
    start: f64,
    length: f64,
    kind: String,
    #[serde(default)]
    chord: Option<String>,
    #[serde(default)]
    scale: Option<String>,
    #[serde(default)]
    root: u8,
    #[serde(default)]
    start_octave: Option<i8>,
    #[serde(default)]
    octaves: Option<u8>,
    #[serde(default)]
    shift: Option<i8>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    speed_multiplier: Option<f64>,
    #[serde(default)]
    melody: Option<String>,
    #[serde(default)]
    accompaniment: Option<String>,
    #[serde(default)]
    additional_voices: Option<u8>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    gate: Option<f64>,
    #[serde(default)]
    density: Option<f64>,
    #[serde(default)]
    step_size: Option<u8>,
    #[serde(default)]
    motif_steps: Option<u8>,
    #[serde(default)]
    velocity: Option<u8>,
}

fn conflict<H: SongToolHost + ?Sized>(host: &H, id: u64, revision: u64) -> Result<(), ToolError> {
    let current = host.song_session();
    if current.id != id || current.revision != revision {
        return Err(ToolError::new(
            "conflict",
            format!(
                "Song changed; read fresh context. Current session_id={}, revision={}",
                current.id, current.revision
            ),
        ));
    }
    Ok(())
}

fn apply<H: SongToolHost + ?Sized>(
    host: &mut H,
    id: u64,
    revision: u64,
    batch: EditBatch,
) -> Result<Value, ToolError> {
    conflict(host, id, revision)?;
    let result = host.apply_edits(batch)?;
    let session = host.song_session();
    Ok(
        json!({"session_id":session.id,"revision":session.revision,"changed":result.changed,"created":result.created}),
    )
}

/// The only dispatcher for core song tools. Unknown names are returned to the
/// app so it can dispatch its explicit transport/device/render capabilities.
pub fn dispatch_song_tool<H: SongToolHost + ?Sized>(
    host: &mut H,
    name: &str,
    arguments: Value,
) -> Result<Value, ToolError> {
    if query::handles(name) {
        return query::dispatch(host, name, arguments);
    }
    if composer::handles(name) {
        return composer::dispatch(host, name, arguments);
    }
    match name {
        "apply_edits" => {
            let q: ApplyArgs = parse_arguments(arguments)?;
            apply(
                host,
                q.session_id,
                q.expected_revision,
                EditBatch {
                    label: q.label,
                    edits: q.edits,
                },
            )
        }
        "create_midi_part" => {
            let q: MidiPartArgs = parse_arguments(arguments)?;
            let edits = part_edits(q.track_id, q.name.clone(), q.start, q.length, q.notes);
            apply(
                host,
                q.session_id,
                q.expected_revision,
                EditBatch {
                    label: format!("Create {}", q.name),
                    edits,
                },
            )
        }
        "set_mix" => {
            let q: MixArgs = parse_arguments(arguments)?;
            apply(
                host,
                q.session_id,
                q.expected_revision,
                EditBatch {
                    label: "Set mix".into(),
                    edits: vec![SongEdit::SetMix {
                        channels: q.channels,
                    }],
                },
            )
        }
        "transform_phrase" => {
            let q: TransformArgs = parse_arguments(arguments)?;
            conflict(host, q.session_id, q.expected_revision)?;
            let track = q.track_id.into();
            let clip = q.clip_id.into();
            let edit = match q.operation.as_str() {
                "transpose" | "move" => SongEdit::TransformNotes {
                    track,
                    clip,
                    note_ids: q.note_ids,
                    beats: q.beats,
                    semitones: q.semitones,
                },
                "quantize" => SongEdit::QuantizeNotes {
                    track,
                    clip,
                    note_ids: q.note_ids,
                    grid: q.grid,
                    strength: q.strength,
                    swing: q.swing,
                    ends: q.ends,
                },
                "legato" => SongEdit::LegatoNotes {
                    track,
                    clip,
                    note_ids: q.note_ids,
                    mode: q.mode,
                },
                "randomize" => SongEdit::RandomizeNotes {
                    track,
                    clip,
                    note_ids: q.note_ids,
                    velocity: q.velocity.unwrap_or(16),
                    timing: q.timing,
                    seed: q.seed,
                },
                "velocity" => {
                    let velocity = q.velocity.ok_or_else(|| {
                        ToolError::new("invalid_arguments", "velocity is required")
                    })?;
                    let notes = host
                        .project()
                        .midi_notes(q.track_id, q.clip_id)
                        .ok_or_else(|| ToolError::new("not_found", "MIDI clip not found"))?;
                    let selected: HashSet<_> = q.note_ids.into_iter().collect();
                    let changes: Vec<_> = notes
                        .iter()
                        .filter(|note| selected.is_empty() || selected.contains(&note.id))
                        .map(|note| NoteChange {
                            id: note.id,
                            pitch: None,
                            start: None,
                            length: None,
                            velocity: Some(velocity),
                        })
                        .collect();
                    if !selected.is_empty() && changes.len() != selected.len() {
                        return Err(ToolError::new("not_found", "Selected note not found"));
                    }
                    SongEdit::UpdateNotes {
                        track,
                        clip,
                        notes: changes,
                    }
                }
                _ => {
                    return Err(ToolError::new(
                        "invalid_arguments",
                        "Unknown phrase transformation",
                    ));
                }
            };
            apply(
                host,
                q.session_id,
                q.expected_revision,
                EditBatch {
                    label: format!("Transform phrase: {}", q.operation),
                    edits: vec![edit],
                },
            )
        }
        "generate_part" => {
            let q: GenerateArgs = parse_arguments(arguments)?;
            conflict(host, q.session_id, q.expected_revision)?;
            let settings = generator_settings(&q)?;
            let notes = generate_notes(&settings)
                .map_err(|e| ToolError::new("invalid_arguments", e.to_string()))?
                .iter()
                .map(|n| NoteData {
                    pitch: n.pitch,
                    start: n.start,
                    length: n.length,
                    velocity: n.velocity,
                })
                .collect();
            let edits = part_edits(q.track_id, q.name.clone(), q.start, q.length, notes);
            apply(
                host,
                q.session_id,
                q.expected_revision,
                EditBatch {
                    label: format!("Generate {}", q.name),
                    edits,
                },
            )
        }
        _ => Err(ToolError::new(
            "unknown_tool",
            format!("Unknown song tool: {name}"),
        )),
    }
}

fn part_edits(
    track: Option<u64>,
    name: String,
    start: f64,
    length: f64,
    notes: Vec<NoteData>,
) -> Vec<SongEdit> {
    let mut edits = Vec::new();
    let track = if let Some(id) = track {
        EntityRef::Id(id)
    } else {
        edits.push(SongEdit::AddTrack {
            kind: EditableTrackKind::Midi,
            name: name.clone(),
            key: Some("part_track".into()),
        });
        EntityRef::Key("part_track".into())
    };
    edits.push(SongEdit::AddMidiClip {
        track,
        name,
        start,
        length,
        notes,
        key: Some("part_clip".into()),
    });
    edits
}

fn enum_value<T: Copy + fmt::Debug>(choices: &[T], value: &str) -> Result<T, ToolError> {
    let normalized = value.replace(['_', '-', ' '], "").to_ascii_lowercase();
    choices
        .iter()
        .copied()
        .find(|v| format!("{v:?}").to_ascii_lowercase() == normalized)
        .ok_or_else(|| ToolError::new("invalid_arguments", format!("Unsupported value: {value}")))
}
fn generator_settings(q: &GenerateArgs) -> Result<GenerateSettings, ToolError> {
    let mut s = GenerateSettings {
        kind: enum_value(&GenerateKind::ALL, &q.kind)?,
        start: 0.0,
        length: q.length,
        root: q.root,
        ..Default::default()
    };
    if q.chord.is_some() && q.scale.is_some() {
        return Err(ToolError::new(
            "invalid_arguments",
            "Choose chord or scale, not both",
        ));
    }
    if let Some(v) = &q.chord {
        s.harmony = GenerateHarmony::Chord(
            Chord::from_name(v).map_err(|e| ToolError::new("invalid_arguments", e.to_string()))?,
        );
    }
    if let Some(v) = &q.scale {
        s.harmony = GenerateHarmony::Scale(
            Scale::from_name(v).map_err(|e| ToolError::new("invalid_arguments", e.to_string()))?,
        );
    }
    if let Some(v) = q.start_octave {
        s.start_octave = v;
    }
    if let Some(v) = q.octaves {
        s.octaves = v;
    }
    if let Some(v) = q.shift {
        s.shift = v;
    }
    if let Some(v) = &q.pattern {
        s.arp = enum_value(&ArpeggioPattern::ALL, v)?;
    }
    if let Some(v) = q.speed_multiplier {
        s.speed_multiplier = v;
    }
    if let Some(v) = &q.melody {
        s.melody = enum_value(&MelodyAlgorithm::ALL, v)?;
    }
    if let Some(v) = &q.accompaniment {
        s.accompaniment = enum_value(&Accompaniment::ALL, v)?;
    }
    if let Some(v) = q.additional_voices {
        s.additional_voices = v;
    }
    if let Some(v) = q.seed {
        s.seed = v;
    }
    if let Some(v) = q.gate {
        s.gate = v;
    }
    if let Some(v) = q.density {
        s.density = v;
    }
    if let Some(v) = q.step_size {
        s.step_size = v;
    }
    if let Some(v) = q.motif_steps {
        s.motif_steps = v;
    }
    if let Some(v) = q.velocity {
        s.velocity = v;
    }
    Ok(s)
}

mod schema;
pub use schema::{
    application_tool_specs, host_tool_specs, recording_tool_specs, song_edit_schema,
    song_tool_specs, tool_specs, tool_specs_for,
};
