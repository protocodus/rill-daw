use crate::{ToolProfile, ToolSpec};
use daw_core::{Chord, Scale};
use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn number() -> Value {
    json!({"type":"number"})
}
fn integer() -> Value {
    json!({"type":"integer","minimum":0})
}
fn string() -> Value {
    json!({"type":"string","maxLength":1024})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}
fn randomize_velocity() -> Value {
    json!({"type":"integer","minimum":0,"maximum":127,"default":16,"description":"Symmetric velocity variation, plus or minus this amount, clamped to MIDI velocity 1–127. Zero keeps velocity unchanged."})
}
fn randomize_timing() -> Value {
    json!({"type":"number","minimum":0,"maximum":0.25,"default":0,"description":"Symmetric onset jitter, plus or minus this many audible quarter-note beats. Zero preserves timing; pitches and note lengths are preserved."})
}
fn randomize_seed() -> Value {
    json!({"type":"integer","minimum":0,"maximum":4294967295_u64,"default":1,"description":"Deterministic 32-bit seed; the same source notes, selection and settings produce the same result."})
}
fn id() -> Value {
    json!({"type":"integer","minimum":1})
}
fn reference() -> Value {
    json!({"oneOf":[id(),key()]})
}
fn key() -> Value {
    json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[a-zA-Z0-9_-]+$"})
}
fn nullable(value: Value) -> Value {
    json!({"anyOf":[value,{"type":"null"}]})
}
fn array(value: Value, max: usize) -> Value {
    json!({"type":"array","items":value,"maxItems":max})
}
fn choices(values: &[&str]) -> Value {
    json!({"type":"string","enum":values})
}
fn note() -> Value {
    object(
        json!({"pitch":{"type":"integer","minimum":0,"maximum":127},"start":number(),"length":number(),"velocity":{"type":"integer","minimum":1,"maximum":127}}),
        &["pitch", "start", "length", "velocity"],
    )
}
fn mix() -> Value {
    object(
        json!({"track":reference(),"gain":{"type":"number","minimum":0,"maximum":16,"description":"Linear gain, including exact zero; omit gain_db when using gain."},"gain_db":{"type":"number","minimum":-120,"maximum":24,"description":"Gain in dB; omit gain when using gain_db."},"pan":{"type":"number","minimum":-1,"maximum":1},"mute":boolean(),"solo":boolean()}),
        &["track"],
    )
}
fn edit(op: &str, fields: Value, required: &[&str]) -> Value {
    let mut properties = fields;
    properties["op"] = json!({"type":"string","const":op});
    let mut required = required.to_vec();
    required.push("op");
    object(properties, &required)
}

pub fn song_edit_schema() -> Value {
    let mut v = Vec::new();
    macro_rules! add {($op:literal,$fields:expr,[$($required:literal),*])=>{v.push(edit($op,$fields,&[$($required),*]));};}
    add!("rename_song", json!({"name":string()}), ["name"]);
    add!(
        "set_tempo",
        json!({"bpm":{"type":"number","minimum":20,"maximum":400}}),
        ["bpm"]
    );
    add!(
        "set_meter",
        json!({"beats_per_bar":{"type":"integer","minimum":1,"maximum":32}}),
        ["beats_per_bar"]
    );
    add!(
        "set_loop",
        json!({"enabled":boolean(),"start":number(),"end":number()}),
        ["enabled", "start", "end"]
    );
    add!(
        "set_metronome",
        json!({"enabled":boolean(),"gain":number(),"precount_bars":{"type":"integer","minimum":0,"maximum":2}}),
        ["enabled", "gain", "precount_bars"]
    );
    add!(
        "add_track",
        json!({"kind":choices(&["audio","midi","bus","folder"]),"name":string(),"key":key()}),
        ["kind", "name"]
    );
    add!("delete_track", json!({"track":reference()}), ["track"]);
    add!("clear_track_input", json!({"track":reference()}), ["track"]);
    add!(
        "join_midi_clips",
        json!({"track":reference(),"clips":array(reference(),128),"key":key()}),
        ["track", "clips"]
    );
    add!(
        "reorder_track",
        json!({"track":reference(),"index":integer()}),
        ["track", "index"]
    );
    add!(
        "set_audio_input",
        json!({"track":reference(),"input":{"oneOf":[object(json!({"Mono":object(json!({"channel":integer()}),&["channel"])}),&["Mono"]),object(json!({"Stereo":object(json!({"left":integer(),"right":integer()}),&["left","right"])}),&["Stereo"])]}}),
        ["track", "input"]
    );
    add!(
        "set_midi_input",
        json!({"track":reference(),"input":{"oneOf":[choices(&["All"]),object(json!({"Channel":object(json!({"channel":{"type":"integer","minimum":0,"maximum":15}}),&["channel"])}),&["Channel"])]}}),
        ["track", "input"]
    );
    add!(
        "rename_track",
        json!({"track":reference(),"name":string()}),
        ["track", "name"]
    );
    add!(
        "set_track_color",
        json!({"track":reference(),"color":{"type":"array","minItems":3,"maxItems":3,"items":{"type":"integer","minimum":0,"maximum":255}}}),
        ["track", "color"]
    );
    add!(
        "set_mix",
        json!({"channels":array(mix(),128)}),
        ["channels"]
    );
    add!(
        "set_output",
        json!({"track":reference(),"bus":nullable(reference())}),
        ["track", "bus"]
    );
    add!(
        "set_folder",
        json!({"track":reference(),"folder":nullable(reference())}),
        ["track", "folder"]
    );
    for op in ["add_send", "update_send"] {
        v.push(edit(
            op,
            json!({"track":reference(),"bus":reference(),"gain":number(),"pre_fader":boolean()}),
            &["track", "bus", "gain", "pre_fader"],
        ));
    }
    add!(
        "remove_send",
        json!({"track":reference(),"bus":reference()}),
        ["track", "bus"]
    );
    add!(
        "add_midi_clip",
        json!({"track":reference(),"name":string(),"start":number(),"length":number(),"notes":array(note(),8192),"key":key()}),
        ["track", "name", "start", "length", "notes"]
    );
    add!(
        "rename_clip",
        json!({"track":reference(),"clip":reference(),"name":string()}),
        ["track", "clip", "name"]
    );
    add!(
        "move_clip",
        json!({"track":reference(),"clip":reference(),"start":number()}),
        ["track", "clip", "start"]
    );
    add!(
        "move_clip_to_track",
        json!({"track":reference(),"clip":reference(),"destination":reference(),"start":number()}),
        ["track", "clip", "destination", "start"]
    );
    add!(
        "trim_clip",
        json!({"track":reference(),"clip":reference(),"start":number(),"length":number()}),
        ["track", "clip", "start", "length"]
    );
    add!(
        "stretch_clip",
        json!({"track":reference(),"clip":reference(),"length":number()}),
        ["track", "clip", "length"]
    );
    add!(
        "split_clip",
        json!({"track":reference(),"clip":reference(),"beat":number(),"key":key()}),
        ["track", "clip", "beat"]
    );
    add!(
        "duplicate_clip",
        json!({"track":reference(),"clip":reference(),"start":number(),"key":key()}),
        ["track", "clip", "start"]
    );
    add!(
        "ghost_clip",
        json!({"track":reference(),"clip":reference(),"key":key()}),
        ["track", "clip"]
    );
    for op in ["make_independent", "delete_clip", "reverse_audio"] {
        v.push(edit(
            op,
            json!({"track":reference(),"clip":reference()}),
            &["track", "clip"],
        ));
    }
    add!(
        "add_notes",
        json!({"track":reference(),"clip":reference(),"notes":array(note(),8192)}),
        ["track", "clip", "notes"]
    );
    add!(
        "update_notes",
        json!({"track":reference(),"clip":reference(),"notes":array(object(json!({"id":id(),"pitch":integer(),"start":number(),"length":number(),"velocity":integer()}),&["id"]),8192)}),
        ["track", "clip", "notes"]
    );
    for op in ["delete_notes", "glue_notes"] {
        v.push(edit(
            op,
            json!({"track":reference(),"clip":reference(),"note_ids":array(id(),8192)}),
            &["track", "clip", "note_ids"],
        ));
    }
    add!(
        "legato_notes",
        json!({"track":reference(),"clip":reference(),"note_ids":array(id(),8192),"mode":{"type":"string","enum":["next_onset","top_to_bottom"],"default":"next_onset","description":"next_onset connects each note to the next distinct onset. top_to_bottom ranks simultaneous notes from high to low and connects matching ranks across later onset groups."}}),
        ["track", "clip"]
    );
    add!(
        "transform_notes",
        json!({"track":reference(),"clip":reference(),"note_ids":array(id(),8192),"beats":number(),"semitones":{"type":"integer"}}),
        ["track", "clip", "beats", "semitones"]
    );
    add!(
        "quantize_notes",
        json!({"track":reference(),"clip":reference(),"note_ids":array(id(),8192),"grid":number(),"strength":number(),"swing":number(),"ends":boolean()}),
        ["track", "clip", "grid", "strength", "swing", "ends"]
    );
    add!(
        "randomize_notes",
        json!({"track":reference(),"clip":reference(),"note_ids":array(id(),8192),"velocity":randomize_velocity(),"timing":randomize_timing(),"seed":randomize_seed()}),
        ["track", "clip"]
    );
    add!(
        "split_note",
        json!({"track":reference(),"clip":reference(),"note":id(),"beat":number()}),
        ["track", "clip", "note", "beat"]
    );
    add!(
        "duplicate_note",
        json!({"track":reference(),"clip":reference(),"note":id()}),
        ["track", "clip", "note"]
    );
    add!(
        "set_audio_gain",
        json!({"track":reference(),"clip":reference(),"gain":number()}),
        ["track", "clip", "gain"]
    );
    let fade = choices(&["Linear", "EqualPower", "SCurve"]);
    add!(
        "set_audio_fades",
        json!({"track":reference(),"clip":reference(),"fade_in":{"type":"number","minimum":0,"description":"Fade duration in source quarter-note beats."},"fade_out":{"type":"number","minimum":0,"description":"Fade duration in source quarter-note beats."},"in_curve":fade,"out_curve":fade}),
        [
            "track",
            "clip",
            "fade_in",
            "fade_out",
            "in_curve",
            "out_curve"
        ]
    );
    add!(
        "set_audio_processing",
        json!({"track":reference(),"clip":reference(),"processing":object(json!({"algorithm":choices(&["Signalsmith","Bungee"]),"transpose_semitones":{"type":"number","minimum":-24,"maximum":24}}),&["algorithm","transpose_semitones"])}),
        ["track", "clip", "processing"]
    );
    let pair = json!({"type":"array","items":number(),"minItems":2,"maxItems":2});
    let curve = object(
        json!({"control1":pair.clone(),"control2":pair.clone()}),
        &["control1", "control2"],
    );
    add!(
        "set_crossfade",
        json!({"track":reference(),"clip":reference(),"crossfade":nullable(object(json!({"duration":number(),"curve":curve}),&["duration","curve"]))}),
        ["track", "clip", "crossfade"]
    );
    add!(
        "slip_audio",
        json!({"track":reference(),"clip":reference(),"delta":number(),"source_beats":{"type":"number","description":"Optional advisory bound for trusted offline callers; the live host resolves the actual source duration."}}),
        ["track", "clip", "delta"]
    );
    add!(
        "delete_audio_range",
        json!({"track":reference(),"clip":reference(),"start":number(),"end":number()}),
        ["track", "clip", "start", "end"]
    );
    add!(
        "ripple_delete_audio",
        json!({"track":reference(),"start":number(),"end":number()}),
        ["track", "start", "end"]
    );
    add!(
        "copy_audio_range",
        json!({"track":reference(),"clip":reference(),"start":number(),"end":number(),"destination":reference(),"at":number()}),
        ["track", "clip", "start", "end", "destination", "at"]
    );
    add!(
        "split_audio_transients",
        json!({"track":reference(),"clip":reference(),"beats":array(number(),2048)}),
        ["track", "clip", "beats"]
    );
    add!(
        "strip_audio_silence",
        json!({"track":reference(),"clip":reference(),"start":number(),"end":number(),"retained":array(pair.clone(),2048)}),
        ["track", "clip", "start", "end", "retained"]
    );
    add!("remove_instrument", json!({"track":reference()}), ["track"]);
    add!(
        "remove_insert",
        json!({"track":reference(),"effect":id()}),
        ["track", "effect"]
    );
    add!(
        "set_insert_enabled",
        json!({"track":reference(),"effect":id(),"enabled":boolean()}),
        ["track", "effect", "enabled"]
    );
    add!(
        "reorder_insert",
        json!({"track":reference(),"effect":id(),"index":integer()}),
        ["track", "effect", "index"]
    );
    add!(
        "set_plugin_parameters",
        json!({"track":reference(),"effect":nullable(id()),"parameters":array(object(json!({"id":integer(),"value":{"type":"number","minimum":0,"maximum":1}}),&["id","value"]),4096)}),
        ["track", "effect", "parameters"]
    );
    let parameter = json!({"oneOf":[choices(&["TrackGain","TrackPan"]),object(json!({"SendGain":object(json!({"bus":id()}),&["bus"])}),&["SendGain"]),object(json!({"VstParameter":object(json!({"effect":nullable(id()),"id":integer()}),&["effect","id"])}),&["VstParameter"])]});
    let normalized = json!({"type":"number","minimum":0,"maximum":1,"description":"Normalized automation value: TrackGain=linear_gain/1.5, TrackPan=(pan+1)/2, SendGain=linear_gain/16, VST already normalized."});
    add!(
        "add_automation",
        json!({"name":string(),"target":object(json!({"track":id(),"parameter":parameter}),&["track","parameter"]),"value":normalized,"key":key()}),
        ["name", "target", "value"]
    );
    add!(
        "set_automation_interpolation",
        json!({"track":reference(),"interpolation":choices(&["Step","Linear","Smooth"])}),
        ["track", "interpolation"]
    );
    let mut automation_points = array(pair.clone(), 4096);
    automation_points["description"] = json!(
        "Pairs [arrangement_beat, normalized_value]. Values must be 0..1 using the target's normalization documented in add_automation."
    );
    add!(
        "upsert_automation_points",
        json!({"track":reference(),"points":automation_points}),
        ["track", "points"]
    );
    add!(
        "move_automation_point",
        json!({"track":reference(),"point":id(),"beat":number(),"value":normalized}),
        ["track", "point", "beat", "value"]
    );
    add!(
        "remove_automation_points",
        json!({"track":reference(),"point_ids":array(id(),4096)}),
        ["track", "point_ids"]
    );
    add!(
        "group_audio_takes",
        json!({"track":reference(),"clip_ids":array(id(),64),"key":key()}),
        ["track", "clip_ids"]
    );
    add!(
        "add_audio_take",
        json!({"track":reference(),"group":reference(),"clip":reference()}),
        ["track", "group", "clip"]
    );
    add!(
        "select_audio_take",
        json!({"track":reference(),"group":reference(),"source":reference(),"range":nullable(pair),"crossfade_beats":number()}),
        ["track", "group", "source", "range", "crossfade_beats"]
    );
    add!(
        "detach_audio_takes",
        json!({"track":reference(),"group":reference()}),
        ["track", "group"]
    );
    json!({"oneOf":v})
}

pub fn song_tool_specs() -> Vec<ToolSpec> {
    let page = json!({"offset":integer(),"limit":{"type":"integer","minimum":1,"maximum":256}});
    let mut track = page.clone();
    track["track_id"] = id();
    let mut clip = track.clone();
    clip["clip_id"] = id();
    let mut part = json!({"session_id":id(),"expected_revision":integer(),"track_id":id(),"name":string(),"start":number(),"length":number(),"notes":array(note(),8192)});
    let mut generate = part.clone();
    generate.as_object_mut().expect("object").remove("notes");
    for (name, schema) in [
        ("kind", choices(&["chord", "arpeggio", "melody"])),
        (
            "chord",
            json!({"type":"string","enum":Chord::ALL.iter().map(|value|format!("{value:?}")).collect::<Vec<_>>()}),
        ),
        (
            "scale",
            json!({"type":"string","enum":Scale::ALL.iter().map(|value|format!("{value:?}")).collect::<Vec<_>>()}),
        ),
        ("root", integer()),
        ("start_octave", json!({"type":"integer"})),
        ("octaves", integer()),
        ("shift", json!({"type":"integer"})),
        (
            "pattern",
            choices(&[
                "Up",
                "Down",
                "UpDown",
                "DownUp",
                "OutsideIn",
                "InsideOut",
                "Random",
            ]),
        ),
        ("speed_multiplier", number()),
        ("melody", choices(&["RandomWalk", "Motif", "Euclidean"])),
        (
            "accompaniment",
            choices(&["None", "Counterpoint", "Tintinnabuli"]),
        ),
        ("additional_voices", integer()),
        ("seed", integer()),
        ("gate", number()),
        ("density", number()),
        ("step_size", integer()),
        ("motif_steps", integer()),
        ("velocity", integer()),
    ] {
        generate[name] = schema;
    }
    part["notes"]["description"] = json!("Clip-local source beats; all IDs are allocated by Rill.");
    let specs = [
        (
            "get_song_context",
            "Read current song identity, revision, tempo, meter and tool limits. Read before editing; IDs/revisions are required by mutations.",
            object(json!({}), &[]),
        ),
        (
            "list_tracks",
            "Read paginated track IDs, names, mixing and routing summaries.",
            object(page, &[]),
        ),
        (
            "get_track",
            "Read a track's clips, sends, inserts and automation. No filesystem paths or opaque states are disclosed.",
            object(track, &["track_id"]),
        ),
        (
            "get_clip",
            "Read clip timing and paginated MIDI notes or nondestructive audio settings. Note start/length use source beats.",
            object(clip, &["track_id", "clip_id"]),
        ),
        (
            "apply_edits",
            "Apply 1–128 typed operations atomically in one undoable edit. Numeric references address existing IDs; string references address an earlier creation's key in this batch. All positions are zero-based quarter-note beats. No load/save/file paths.",
            object(
                json!({"session_id":id(),"expected_revision":integer(),"label":string(),"edits":array(song_edit_schema(),128)}),
                &["session_id", "expected_revision", "label", "edits"],
            ),
        ),
        (
            "create_midi_part",
            "Create a complete MIDI part and optionally its track in one undoable call. Supply track_id to use an existing MIDI track. Notes are source-local to the new clip. Assign an instrument separately if needed.",
            object(
                part,
                &[
                    "session_id",
                    "expected_revision",
                    "start",
                    "length",
                    "notes",
                ],
            ),
        ),
        (
            "generate_part",
            "Generate a seeded chord, arpeggio or melody and create its clip/optional MIDI track in one call. Optional settings mirror the piano-roll generator. Chord/scale/pattern accept named values (e.g. MinorSeventh, Minor, UpDown). root is MIDI pitch class 0–11. start is arrangement beat; generated notes start at clip source beat zero.",
            object(
                generate,
                &["session_id", "expected_revision", "start", "length", "kind"],
            ),
        ),
        (
            "transform_phrase",
            "Transform all source notes (omit note_ids) or a chosen subset in a single undoable action: transpose/move, quantize, legato, set velocity, or randomize velocity/onsets. Legato defaults to next_onset; top_to_bottom connects matching high-to-low voice ranks across later onset groups. Randomize uses symmetric velocity and audible-beat timing intensities with a deterministic seed; pitch and note length stay unchanged.",
            object(
                json!({"session_id":id(),"expected_revision":integer(),"track_id":id(),"clip_id":id(),"operation":choices(&["transpose","move","quantize","legato","velocity","randomize"]),"note_ids":array(id(),8192),"semitones":json!({"type":"integer"}),"beats":number(),"grid":number(),"strength":number(),"swing":number(),"ends":boolean(),"velocity":{"type":"integer","minimum":0,"maximum":127,"description":"For velocity: the absolute target velocity (1–127), required. For randomize: the symmetric plus/minus intensity (0–127), default 16."},"mode":{"type":"string","enum":["next_onset","top_to_bottom"],"default":"next_onset","description":"Legato voice connection mode."},"timing":randomize_timing(),"seed":randomize_seed()}),
                &[
                    "session_id",
                    "expected_revision",
                    "track_id",
                    "clip_id",
                    "operation",
                ],
            ),
        ),
        (
            "set_mix",
            "Set gain in dB or linear gain (gain:0 is silence), pan (-1 left to +1 right), mute or solo on multiple channels in one undoable action. Use either gain or gain_db per channel. Unspecified values are preserved.",
            object(
                json!({"session_id":id(),"expected_revision":integer(),"channels":array(mix(),128)}),
                &["session_id", "expected_revision", "channels"],
            ),
        ),
    ];
    let mut specs: Vec<_> = specs
        .into_iter()
        .map(|(name, description, input_schema)| ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema,
        })
        .collect();
    let midi_part = specs
        .iter()
        .find(|spec| spec.name == "create_midi_part")
        .expect("MIDI part schema")
        .input_schema
        .clone();
    let generated_part = specs
        .iter()
        .find(|spec| spec.name == "generate_part")
        .expect("Generated part schema")
        .input_schema
        .clone();
    specs.push(ToolSpec{name:"create_instrument_part".into(),description:"Create an audible instrument part in one undoable call: resolve a trusted catalog plugin_id and create the MIDI part together. part uses create_midi_part arguments; set generate=true and use generate_part arguments for seeded generation. Optional track_id in part uses an existing MIDI track.".into(),input_schema:object(json!({"plugin_id":string(),"generate":boolean(),"part":{"oneOf":[midi_part,generated_part]}}),&["plugin_id","part"])});
    specs.extend(crate::composer::tool_specs());
    specs.extend(host_tool_specs());
    specs.extend(recording_tool_specs());
    specs
}

/// Canonical contracts for capabilities implemented by the desktop host. They
/// remain explicit; the embedded agent can receive a narrower capability set.
pub fn host_tool_specs() -> Vec<ToolSpec> {
    let specs = [
        (
            "get_selection",
            "Read the current selected track, clip, notes and cursor beat.",
            object(json!({}), &[]),
        ),
        (
            "set_selection",
            "Select song objects by stable IDs; omitted fields clear the corresponding selection. Clip must belong to track, and note IDs must belong to that clip. Does not change the active view.",
            object(
                json!({"track_id":id(),"clip_id":id(),"note_ids":array(id(),8192)}),
                &[],
            ),
        ),
        (
            "join_clips",
            "Join 2–128 distinct clips on one track in a single undoable action. MIDI joins retain audible note timing; audio joins bake the existing clip sources and edits in the background. Source assets and rendered paths remain host-owned.",
            object(
                json!({"session_id":id(),"expected_revision":integer(),"track_id":id(),"clip_ids":{"type":"array","items":id(),"minItems":2,"maxItems":128,"uniqueItems":true},"name":string()}),
                &["session_id", "expected_revision", "track_id", "clip_ids"],
            ),
        ),
        (
            "audio_analyze_clip",
            "Analyze an audio source once for normalization, transient splitting and stripping silence. Returns measurements and prepared typed SongEdit proposals; apply_edits commits chosen edits separately. start/end use arrangement beats. Does not edit audio.",
            object(
                json!({"track_id":id(),"clip_id":id(),"start_beat":number(),"end_beat":number(),"transient_algorithm":choices(&["energy_rise","amplitude_gate"]),"sensitivity":number(),"minimum_gap_ms":number(),"silence_threshold_db":number(),"minimum_silence_ms":number(),"padding_ms":number(),"normalize_target_db":number()}),
                &["track_id", "clip_id"],
            ),
        ),
        (
            "audio_process_clip",
            "Normalize, split at detected transients, or strip silence in one undoable action. The host analyzes the existing clip in a background worker and commits only against the requested unchanged song revision. No file paths are accepted.",
            object(
                json!({"session_id":id(),"expected_revision":integer(),"track_id":id(),"clip_id":id(),"operation":choices(&["normalize","split_transients","strip_silence"]),"start_beat":number(),"end_beat":number(),"transient_algorithm":choices(&["energy_rise","amplitude_gate"]),"sensitivity":number(),"minimum_gap_ms":number(),"silence_threshold_db":number(),"minimum_silence_ms":number(),"padding_ms":number(),"normalize_target_db":number()}),
                &[
                    "session_id",
                    "expected_revision",
                    "track_id",
                    "clip_id",
                    "operation",
                ],
            ),
        ),
        (
            "transport",
            "Inspect or control playback of the current song. seek uses zero-based quarter-note beat.",
            object(
                json!({"action":choices(&["state","play","pause","stop","seek"]),"beat":number()}),
                &["action"],
            ),
        ),
        (
            "history",
            "Read named undo/redo history. Undo/redo require session_id and expected_revision from fresh song context so stale requests cannot undo newer user edits; state does not need them.",
            object(
                json!({"action":choices(&["state","undo","redo"]),"session_id":id(),"expected_revision":integer()}),
                &["action"],
            ),
        ),
        (
            "list_plugins",
            "Search already scanned and trusted local VST3 instruments/effects; use returned catalog plugin_id for assignment.",
            object(json!({"query":{"type":"string","maxLength":256}}), &[]),
        ),
        (
            "assign_plugin",
            "Assign a scanned catalog instrument, or add an insert when insert is true, to the current song. The host resolves plugin_id; paths and opaque plugin states are not accepted.",
            object(
                json!({"session_id":id(),"expected_revision":integer(),"track_id":id(),"plugin_id":string(),"insert":boolean()}),
                &["session_id", "expected_revision", "track_id", "plugin_id"],
            ),
        ),
        (
            "audio_analyze",
            "Render and analyze a bounded current-song excerpt locally (up to 30 seconds, ending within the first 120 seconds). Returns peak/RMS/clipping/stereo and spectral measurements, not perceptual listening. Optional track_id measures its contribution through the song routing.",
            object(
                json!({"start_beat":number(),"end_beat":number(),"track_id":id()}),
                &[],
            ),
        ),
        (
            "plugin_describe_parameters",
            "Read paginated parameter names, IDs, normalized values, units and writeability from the assigned instrument or optional insert. Discover actual IDs before parameter editing.",
            object(
                json!({"track_id":id(),"insert_id":id(),"query":{"type":"string","maxLength":256},"offset":integer(),"limit":{"type":"integer","minimum":1,"maximum":200}}),
                &["track_id"],
            ),
        ),
    ];
    specs
        .into_iter()
        .map(|(name, description, input_schema)| ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema,
        })
        .collect()
}

pub fn recording_tool_specs() -> Vec<ToolSpec> {
    let input = object(
        json!({"kind":choices(&["none","audio_mono","audio_stereo","midi_all","midi_channel"]),"channel":integer(),"left":integer(),"right":integer()}),
        &["kind"],
    );
    let tracks = array(
        object(
            json!({"track_id":id(),"input":input,"armed":boolean(),"monitor":choices(&["off","auto","on"])}),
            &["track_id"],
        ),
        32,
    );
    let timing = object(
        json!({"precount_bars":{"type":"integer","minimum":0,"maximum":2},"click_enabled":boolean(),"click_gain":{"type":"number","minimum":0,"maximum":1}}),
        &["precount_bars", "click_enabled", "click_gain"],
    );
    let specs = [
        (
            "get_recording",
            "Read recording phase, current input assignments, arm/monitor state, placement and count-in/click settings.",
            object(json!({}), &[]),
        ),
        (
            "configure_recording",
            "Configure up to 32 track inputs, arming and monitoring, take/overdub placement, and pre-count/metronome timing. Persistent song changes form one undoable edit. Recording remains stopped until start_recording.",
            object(
                json!({"session_id":id(),"expected_revision":integer(),"tracks":tracks,"placement":choices(&["take","overdub"]),"timing":timing}),
                &["session_id", "expected_revision"],
            ),
        ),
        (
            "start_recording",
            "Begin recording the armed tracks of the current song through the normal application recording path.",
            object(
                json!({"session_id":id(),"expected_revision":integer()}),
                &["session_id", "expected_revision"],
            ),
        ),
        (
            "stop_recording",
            "Finalize the current recording, keeping playback running by default; optionally pause or stop transport.",
            object(json!({"after":choices(&["continue","pause","stop"])}), &[]),
        ),
        (
            "retry_recording_input",
            "Retry a failed recording/monitor input through the application's normal device recovery path.",
            object(json!({}), &[]),
        ),
    ];
    specs
        .into_iter()
        .map(|(name, description, input_schema)| ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema,
        })
        .collect()
}

/// Full application catalog is available only to explicitly privileged hosts.
pub fn application_tool_specs() -> Vec<ToolSpec> {
    let audio = object(
        json!({"device_name":nullable(string()),"input_device_name":nullable(string()),"midi_input_device":nullable(string()),"sample_rate":{"type":"integer","minimum":1},"buffer_frames":{"type":"integer","minimum":1},"resample_quality":choices(&["High","Fast"])}),
        &[],
    );
    let ai = object(
        json!({"provider":choices(&["open_ai","anthropic"]),"model":{"type":"string","minLength":1,"maxLength":200},"api_key":{"type":"string","maxLength":4096,"description":"Write-only provider credential; never returned by get_setup."},"remove_api_key":boolean()}),
        &[],
    );
    let specs = [
        (
            "document_status",
            "Inspect document path, dirty status and pending document operations. Available only to an application capability connection.",
            object(json!({}), &[]),
        ),
        (
            "document_action",
            "Use the application's guarded New/Open/Save/Save As/Import/Export/Recover pipeline. Paths are accepted only by this explicitly privileged application tool. Unsaved edits are preserved unless discard_unsaved is true. Import beat uses zero-based quarter-note beats.",
            object(
                json!({"action":choices(&["new","open","save","save_as","import","export","recover"]),"path":string(),"discard_unsaved":boolean(),"beat":number(),"track_id":id(),"include_tails":boolean()}),
                &["action"],
            ),
        ),
        (
            "get_setup",
            "Read application setup and refresh available audio outputs, inputs and MIDI ports, with provider credentials redacted. Discovery does not open audio devices or start capture. Available only to an application capability connection.",
            object(json!({}), &[]),
        ),
        (
            "configure_setup",
            "Configure application audio devices, storage, VST scan roots or assistant settings. Omitted properties are preserved; explicit null clears a device selection. A provided API key is encrypted and never echoed. api_key and remove_api_key=true are mutually exclusive. Available only to an application capability connection.",
            object(
                json!({"audio":audio,"scratch_dir":string(),"vst_dirs":array(string(),256),"ai":ai}),
                &[],
            ),
        ),
    ];
    specs
        .into_iter()
        .map(|(name, description, input_schema)| ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema,
        })
        .collect()
}

pub fn tool_specs_for(profile: ToolProfile) -> Vec<ToolSpec> {
    match profile {
        ToolProfile::CurrentSong => song_tool_specs(),
        ToolProfile::Application => tool_specs(),
    }
}

/// Canonical availability catalog; callers grant a profile separately.
pub fn tool_specs() -> Vec<ToolSpec> {
    let mut specs = song_tool_specs();
    specs.extend(application_tool_specs());
    specs
}
