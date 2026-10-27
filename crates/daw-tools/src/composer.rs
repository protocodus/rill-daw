//! The composition module's canonical assistant/MCP adapter. Musical drafts
//! remain independent of the song; only this boundary turns them into edits.
use crate::{SongToolHost, ToolError, ToolSpec, apply, conflict, parse_arguments};
use daw_core::{
    EditBatch, EditableTrackKind, EntityRef, Instrument, MAX_BEAT, NoteData, SongEdit, TrackKind,
};
use serde::Deserialize;
use serde_json::{Value, json};

const COMPOSE: &str = "compose_progression";

pub(crate) fn is_read(name: &str) -> bool {
    matches!(
        name,
        "composer_catalog"
            | "composer_scale"
            | "composer_chord"
            | "composer_progression"
            | "composer_euclidean"
    )
}

pub(crate) fn handles(name: &str) -> bool {
    is_read(name) || name == COMPOSE
}

fn invalid(error: impl std::fmt::Display) -> ToolError {
    ToolError::new("invalid_arguments", error.to_string())
}

fn text(value: &str, field: &str, limit: usize) -> Result<(), ToolError> {
    if value.trim().is_empty() || value.chars().count() > limit || value.contains('\0') {
        return Err(invalid(format!(
            "{field} must contain 1–{limit} characters and no NUL"
        )));
    }
    Ok(())
}

fn output(value: impl serde::Serialize) -> Result<Value, ToolError> {
    serde_json::to_value(value).map_err(invalid)
}

fn major() -> String {
    "major".into()
}
fn smooth() -> bool {
    true
}
fn beats_per_chord() -> f64 {
    4.0
}
fn composition_name() -> String {
    "Composition".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleArgs {
    root: String,
    scale: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChordArgs {
    root: String,
    chord: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressionArgs {
    root: String,
    #[serde(default = "major")]
    scale: String,
    degrees: Vec<u8>,
    #[serde(default)]
    sevenths: bool,
    #[serde(default = "smooth")]
    smooth: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RhythmArgs {
    pulses: u16,
    steps: u16,
    #[serde(default)]
    rotation: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartTarget {
    #[serde(default)]
    track_id: Option<u64>,
    part: daw_composer::PartSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComposeArgs {
    session_id: u64,
    expected_revision: u64,
    #[serde(default = "composition_name")]
    name: String,
    start: f64,
    root: String,
    #[serde(default = "major")]
    scale: String,
    degrees: Vec<u8>,
    #[serde(default)]
    sevenths: bool,
    #[serde(default = "smooth")]
    smooth: bool,
    #[serde(default = "beats_per_chord")]
    beats_per_chord: f64,
    parts: Vec<PartTarget>,
}

fn progression(args: &ProgressionArgs) -> Result<daw_composer::Progression, ToolError> {
    text(&args.root, "root", 32)?;
    text(&args.scale, "scale", 64)?;
    daw_composer::progression(
        &args.root,
        &args.scale,
        &args.degrees,
        args.sevenths,
        args.smooth,
    )
    .map_err(invalid)
}

pub(crate) fn dispatch<H: SongToolHost + ?Sized>(
    host: &mut H,
    name: &str,
    arguments: Value,
) -> Result<Value, ToolError> {
    match name {
        "composer_catalog" => {
            let _: Empty = parse_arguments(arguments)?;
            let mut value = output(daw_composer::catalog())?;
            value["limits"] = json!({"chords":daw_composer::MAX_CHORDS,"parts":daw_composer::MAX_PARTS,"notes":daw_composer::MAX_NOTES,"duration_beats":daw_composer::MAX_DURATION});
            Ok(value)
        }
        "composer_scale" => {
            let q: ScaleArgs = parse_arguments(arguments)?;
            text(&q.root, "root", 32)?;
            text(&q.scale, "scale", 64)?;
            output(daw_composer::get_scale(&q.root, &q.scale).map_err(invalid)?)
        }
        "composer_chord" => {
            let q: ChordArgs = parse_arguments(arguments)?;
            text(&q.root, "root", 32)?;
            text(&q.chord, "chord", 64)?;
            output(daw_composer::get_chord(&q.root, &q.chord).map_err(invalid)?)
        }
        "composer_progression" => {
            let q: ProgressionArgs = parse_arguments(arguments)?;
            output(progression(&q)?)
        }
        "composer_euclidean" => {
            let q: RhythmArgs = parse_arguments(arguments)?;
            output(daw_composer::euclidean(q.pulses, q.steps, q.rotation).map_err(invalid)?)
        }
        COMPOSE => compose(host, parse_arguments(arguments)?),
        _ => Err(ToolError::new("unknown_tool", "Unknown composition tool")),
    }
}

fn compose<H: SongToolHost + ?Sized>(host: &mut H, q: ComposeArgs) -> Result<Value, ToolError> {
    conflict(host, q.session_id, q.expected_revision)?;
    text(&q.name, "name", 128)?;
    if !q.start.is_finite() || !(0.0..MAX_BEAT).contains(&q.start) {
        return Err(invalid(
            "start must be a nonnegative arrangement beat below the song limit",
        ));
    }
    if q.parts.is_empty() || q.parts.len() > daw_composer::MAX_PARTS {
        return Err(invalid("Supply 1–8 composition parts"));
    }
    for target in &q.parts {
        if let Some(id) = target.track_id {
            let track = host
                .project()
                .track(id)
                .ok_or_else(|| ToolError::new("not_found", "Target track not found"))?;
            if track.kind != TrackKind::Midi {
                return Err(invalid("Composition targets must be MIDI tracks"));
            }
        }
    }
    let harmony = progression(&ProgressionArgs {
        root: q.root,
        scale: q.scale,
        degrees: q.degrees,
        sevenths: q.sevenths,
        smooth: q.smooth,
    })?;
    let specs: Vec<_> = q.parts.iter().map(|p| p.part.clone()).collect();
    let drafts = daw_composer::compose(&harmony, q.beats_per_chord, &specs).map_err(invalid)?;
    let length = drafts[0].length;
    if q.start + length > MAX_BEAT || q.start + length <= q.start {
        return Err(invalid("Composition extends beyond the song limit"));
    }
    let mut edits = Vec::new();
    let mut summaries = Vec::new();
    let mut notes_total = 0;
    for (index, (target, draft)) in q.parts.iter().zip(drafts).enumerate() {
        let track_key = format!("composer_track_{index}");
        let clip_key = format!("composer_clip_{index}");
        let track = if let Some(id) = target.track_id {
            EntityRef::Id(id)
        } else {
            edits.push(SongEdit::AddTrack {
                kind: EditableTrackKind::Midi,
                name: draft.name.clone(),
                key: Some(track_key.clone()),
            });
            EntityRef::Key(track_key.clone())
        };
        let count = draft.notes.len();
        notes_total += count;
        summaries.push((
            target.track_id,
            track_key,
            clip_key.clone(),
            draft.name.clone(),
            draft.length,
            count,
        ));
        edits.push(SongEdit::AddMidiClip {
            track,
            name: draft.name,
            start: q.start,
            length: draft.length,
            notes: draft
                .notes
                .into_iter()
                .map(|n| NoteData {
                    pitch: n.pitch,
                    start: n.start,
                    length: n.length,
                    velocity: n.velocity,
                })
                .collect(),
            key: Some(clip_key),
        });
    }
    let mut result = apply(
        host,
        q.session_id,
        q.expected_revision,
        EditBatch {
            label: format!("Compose {}", q.name),
            edits,
        },
    )?;
    // Resolve batch-local creation keys after the single atomic commit. Never
    // return the entire draft, which could exceed an assistant's result budget.
    let created = result["created"]
        .as_array()
        .expect("edit result created array");
    let find = |key: &str| {
        created
            .iter()
            .find(|e| e["key"] == key)
            .and_then(|e| e["id"].as_u64())
    };
    let parts: Vec<_> = summaries.into_iter().map(|(existing, track_key, clip_key, name, length, note_count)| {
        let track_id = existing.or_else(|| find(&track_key)).expect("committed track ID");
        let clip_id = find(&clip_key).expect("committed clip ID");
        let instrument_assigned = host.project().track(track_id).is_some_and(|t| !matches!(t.instrument, Instrument::None));
        json!({"track_id":track_id,"clip_id":clip_id,"name":name,"length":length,"note_count":note_count,"instrument_assigned":instrument_assigned})
    }).collect();
    let needs_instrument = parts.iter().any(|p| p["instrument_assigned"] == false);
    result["parts"] = json!(parts);
    result["length"] = json!(length);
    result["notes_total"] = json!(notes_total);
    if needs_instrument {
        result["instrument_note"] = json!(
            "Assign an installed VST3 instrument to unassigned MIDI tracks before auditioning, or target existing instrument tracks."
        );
    }
    Ok(result)
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn string(max: usize) -> Value {
    json!({"type":"string","minLength":1,"maxLength":max})
}
fn choices(values: &[&str]) -> Value {
    json!({"type":"string","enum":values})
}
fn number(min: f64, max: f64) -> Value {
    json!({"type":"number","minimum":min,"maximum":max})
}
fn integer(min: i64, max: i64) -> Value {
    json!({"type":"integer","minimum":min,"maximum":max})
}
fn base_properties() -> Value {
    json!({"root":string(32),"scale":string(64),"degrees":{"type":"array","minItems":1,"maxItems":32,"items":integer(1,7)},"sevenths":{"type":"boolean"},"smooth":{"type":"boolean"}})
}
fn part_schema() -> Value {
    let common = json!({"name":string(128),"velocity":integer(1,127),"gate":{"type":"number","exclusiveMinimum":0,"maximum":1}});
    let mut chords = common.clone();
    chords["kind"] = json!({"type":"string","const":"chords"});
    chords["octave_shift"] = integer(-10, 10);
    let mut arp = chords.clone();
    arp["kind"] = json!({"type":"string","const":"arpeggio"});
    arp["style"] = choices(&["up", "down", "up_down", "alberti"]);
    arp["step"] = number(daw_composer::MIN_STEP, daw_composer::MAX_STEP);
    arp["step"]["description"] =
        json!("Quarter-note beats per note; must evenly divide beats_per_chord.");
    let mut bass = common;
    bass["kind"] = json!({"type":"string","const":"bass"});
    bass["style"] = choices(&["root", "root_fifth", "root_octave"]);
    bass["step"] = number(daw_composer::MIN_STEP, daw_composer::MAX_STEP);
    bass["step"]["description"] = json!(
        "Grid for root_fifth/root_octave; must evenly divide beats_per_chord. Root style holds each chord independently of this grid."
    );
    bass["octave"] = integer(
        daw_composer::MIN_BASS_OCTAVE.into(),
        daw_composer::MAX_BASS_OCTAVE.into(),
    );
    json!({"oneOf":[object(chords,&["kind"]),object(arp,&["kind"]),object(bass,&["kind"])]})
}

pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    let mut compose = base_properties();
    compose["session_id"] = integer(1, i64::MAX);
    compose["expected_revision"] = json!({"type":"integer","minimum":0});
    compose["name"] = string(128);
    compose["start"] = number(0.0, MAX_BEAT);
    compose["beats_per_chord"] = number(
        daw_composer::MIN_BEATS_PER_CHORD,
        daw_composer::MAX_BEATS_PER_CHORD,
    );
    compose["parts"] = json!({"type":"array","minItems":1,"maxItems":8,"items":object(json!({"track_id":{"anyOf":[integer(1,i64::MAX),{"type":"null"}]},"part":part_schema()}),&["part"])});
    [
        ("composer_catalog","List the native composition module's 50 scales and 37 chord types, aliases, descriptions and limits. Pure music theory, no song edits or Python runtime.",object(json!({}),&[])),
        ("composer_scale","Spell a scale from a root, using composer_catalog names/aliases. An abstract root (F) returns note names; a concrete root (F4) also returns MIDI pitches. C4 = MIDI 60. Does not edit the song.",object(json!({"root":string(32),"scale":string(64)}),&["root","scale"])),
        ("composer_chord","Spell a chord from a root and a composer_catalog chord type/alias (e.g. root C4, chord 9). Concrete roots also return MIDI pitches; abstract roots return pitch classes. Does not edit the song.",object(json!({"root":string(32),"chord":string(64)}),&["root","chord"])),
        ("composer_progression","Resolve 1–32 diatonic scale degrees into triads or sevenths in a seven-note scale; smooth=true chooses close voice-led inversions. degrees are integer positions 1–7, not literal Roman/borrowed/applied chords. root must include an octave, e.g. D4. Defaults: major, sevenths=false, smooth=true. Read-only.",object(base_properties(),&["root","degrees"])),
        ("composer_euclidean","Spread pulses across steps with the upstream Euclidean rhythm convention: O is the first accent, o another onset, . a rest. Positive rotation shifts the pattern left. Read-only; max 256 steps.",object(json!({"pulses":integer(0,256),"steps":integer(1,256),"rotation":integer(i32::MIN.into(),i32::MAX.into())}),&["pulses","steps"])),
        (COMPOSE,"Compose chord, arpeggio and bass parts over 1–32 diatonic degree positions in one atomic undoable edit. Read fresh song identity/revision first. Root includes octave (D4), start is a zero-based arrangement beat; notes use source-local quarter-note beats. Each parts item wraps a musical part spec and optional existing MIDI track_id, preserving that track's instrument and notes. Omit track_id to create a MIDI track, then assign an installed instrument if needed. Up to 8 parts and 8192 notes total. Defaults: major, smooth=true, sevenths=false, 4 beats/chord. Entire requested range is retained even after the last gated note.",object(compose,&["session_id","expected_revision","start","root","degrees","parts"])),
    ].into_iter().map(|(name,description,input_schema)|ToolSpec{name:name.into(),description:description.into(),input_schema}).collect()
}
