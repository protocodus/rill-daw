use daw_core::*;
use daw_tools::*;
use serde_json::{Value, json};

#[derive(Default)]
struct Host {
    p: Project,
    session: SongSession,
    history: History,
}
impl SongToolHost for Host {
    fn project(&self) -> &Project {
        &self.p
    }
    fn song_session(&self) -> SongSession {
        self.session
    }
    fn apply_edits(&mut self, batch: EditBatch) -> Result<EditResult, ToolError> {
        let before = self.p.clone();
        let result = batch
            .apply(&mut self.p)
            .map_err(|e| ToolError::new("edit_failed", format!("{e:#}")))?;
        if result.changed {
            self.history.commit_named(&before, batch.label);
            self.session.bump();
        }
        Ok(result)
    }
}
fn call(host: &mut Host, name: &str, args: Value) -> Value {
    dispatch_song_tool(host, name, args).unwrap()
}

#[test]
fn complete_part_is_one_call_and_one_undoable_operation() {
    let mut h = Host::default();
    let id = h.session.id;
    let result = call(
        &mut h,
        "create_midi_part",
        json!({"session_id":id,"expected_revision":0,"name":"Bass","start":16,"length":4,"notes":[{"pitch":36,"start":0,"length":1,"velocity":100},{"pitch":43,"start":2,"length":1,"velocity":90}]}),
    );
    assert_eq!(result["revision"], 1);
    assert_eq!(result["created"].as_array().unwrap().len(), 2);
    assert_eq!(h.p.tracks[0].clips[0].start, 16.0);
    assert_eq!(h.history.undo_labels().count(), 1);
    assert!(h.history.undo(&mut h.p));
    assert!(h.p.tracks.is_empty());
}

#[test]
fn stale_session_and_revision_reject_mutation() {
    let mut h = Host::default();
    let id = h.session.id;
    let before = h.p.clone();
    for (session, revision) in [(id + 1, 0), (id, 1)] {
        let err=dispatch_song_tool(&mut h,"apply_edits",json!({"session_id":session,"expected_revision":revision,"label":"Tempo","edits":[{"op":"set_tempo","bpm":90}]})).unwrap_err();
        assert_eq!(err.code, "conflict");
        assert_eq!(h.p, before);
    }
}

#[test]
fn core_queries_hide_paths_and_opaque_state_and_are_bounded() {
    let mut h = Host::default();
    let t = h.p.add_audio_track("Audio");
    let clip = Clip::audio("Sound", 0.0, 4.0, "/private/source.wav");
    let c = clip.id;
    h.p.track_mut(t).unwrap().clips.push(clip);
    h.p.smooth_audio_boundaries(&[(t, c)], 3.0, 5.0).unwrap();
    let result = call(&mut h, "get_clip", json!({"track_id":t,"clip_id":c}));
    assert!(!result.to_string().contains("/private"));
    assert!(
        result["audio"]["boundary_fades"]["fade_in"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert_eq!(
        dispatch_song_tool(&mut h, "list_tracks", json!({"limit":257}))
            .unwrap_err()
            .code,
        "invalid_arguments"
    );
    assert_eq!(
        dispatch_song_tool(&mut h, "get_song_context", json!({"path":"/tmp"}))
            .unwrap_err()
            .code,
        "invalid_arguments"
    );
    assert_eq!(
        dispatch_song_tool(&mut h, "save", json!({}))
            .unwrap_err()
            .code,
        "unknown_tool"
    );
}

#[test]
fn generator_is_reproducible_and_phrase_transform_uses_stable_ids() {
    let mut a = Host::default();
    let mut b = Host::default();
    for h in [&mut a, &mut b] {
        let id = h.session.id;
        call(
            h,
            "generate_part",
            json!({"session_id":id,"expected_revision":0,"name":"Arp","kind":"arpeggio","chord":"MinorSeventh","pattern":"up_down","seed":123,"start":0,"length":8}),
        );
    }
    let notes = |h: &Host| {
        let t = &h.p.tracks[0];
        h.p.midi_notes(t.id, t.clips[0].id)
            .unwrap()
            .iter()
            .map(|n| (n.pitch, n.start, n.length, n.velocity))
            .collect::<Vec<_>>()
    };
    assert_eq!(notes(&a), notes(&b));
    let t = a.p.tracks[0].id;
    let c = a.p.tracks[0].clips[0].id;
    let id = a.session.id;
    let original = a.p.midi_notes(t, c).unwrap()[0].clone();
    call(
        &mut a,
        "transform_phrase",
        json!({"session_id":id,"expected_revision":1,"track_id":t,"clip_id":c,"operation":"transpose","semitones":12}),
    );
    let changed = &a.p.midi_notes(t, c).unwrap()[0];
    assert_eq!(changed.id, original.id);
    assert_eq!(changed.pitch, original.pitch + 12);
}

fn phrase_host() -> (Host, u64, u64) {
    let mut host = Host::default();
    let notes: Vec<_> = (0..16).map(|index| json!({
        "pitch":60 + index % 5,"start":1.0 + f64::from(index) * 0.25,"length":0.125,"velocity":64
    })).collect();
    let session = host.session;
    call(
        &mut host,
        "create_midi_part",
        json!({
            "session_id":session.id,"expected_revision":session.revision,"name":"Phrase",
            "start":0.0,"length":8.0,"notes":notes
        }),
    );
    let track = host.p.tracks[0].id;
    let clip = host.p.tracks[0].clips[0].id;
    host.history = History::default();
    (host, track, clip)
}

fn clone_song_host(source: &Host) -> Host {
    Host {
        p: source.p.clone(),
        session: source.session,
        history: History::default(),
    }
}

fn transform_arguments(host: &Host, track: u64, clip: u64, operation: &str) -> Value {
    json!({
        "session_id":host.session.id,"expected_revision":host.session.revision,
        "track_id":track,"clip_id":clip,"operation":operation
    })
}

#[test]
fn randomize_tool_defaults_match_typed_edits_and_keep_one_undoable_commit() {
    let (mut phrase, track, clip) = phrase_host();
    let mut typed = clone_song_host(&phrase);
    let original = phrase.p.clone();
    let arguments = transform_arguments(&phrase, track, clip, "randomize");
    let session = typed.session;
    call(&mut phrase, "transform_phrase", arguments);
    call(
        &mut typed,
        "apply_edits",
        json!({
            "session_id":session.id,"expected_revision":session.revision,"label":"Randomize notes",
            "edits":[{"op":"randomize_notes","track":track,"clip":clip}]
        }),
    );
    assert_eq!(phrase.p, typed.p);
    let before = original.midi_notes(track, clip).unwrap();
    let after = phrase.p.midi_notes(track, clip).unwrap();
    assert!(
        after
            .iter()
            .zip(before)
            .any(|(a, b)| a.velocity != b.velocity)
    );
    for (a, b) in after.iter().zip(before) {
        assert_eq!(
            (a.id, a.pitch, a.start, a.length),
            (b.id, b.pitch, b.start, b.length)
        );
        assert!((i16::from(a.velocity) - i16::from(b.velocity)).abs() <= 16);
    }
    assert_eq!(phrase.history.undo_labels().count(), 1);
    assert!(phrase.history.undo(&mut phrase.p));
    assert_eq!(phrase.p, original);
}

#[test]
fn randomize_tool_respects_subset_noop_conflicts_and_atomic_failure() {
    let (mut host, track, clip) = phrase_host();
    let before = host.p.clone();
    let revision = host.session.revision;
    let mut arguments = transform_arguments(&host, track, clip, "randomize");
    arguments["velocity"] = json!(0);
    arguments["timing"] = json!(0);
    let unchanged = call(&mut host, "transform_phrase", arguments);
    assert_eq!(unchanged["changed"], false);
    assert_eq!(host.p, before);
    assert_eq!(host.session.revision, revision);
    assert_eq!(host.history.undo_labels().count(), 0);
    for extra in [
        json!({"expected_revision":revision+1}),
        json!({"session_id":host.session.id+1}),
        json!({"velocity":128}),
        json!({"timing":0.250001}),
        json!({"timing":-0.01}),
        json!({"seed":4294967296_u64}),
    ] {
        let mut arguments = transform_arguments(&host, track, clip, "randomize");
        arguments
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(dispatch_song_tool(&mut host, "transform_phrase", arguments).is_err());
        assert_eq!(host.p, before);
        assert_eq!(host.session.revision, revision);
        assert_eq!(host.history.undo_labels().count(), 0);
    }
    let session = host.session;
    assert!(dispatch_song_tool(&mut host,"apply_edits",json!({
        "session_id":session.id,"expected_revision":session.revision,"label":"Invalid randomize batch",
        "edits":[{"op":"set_tempo","bpm":95},{"op":"randomize_notes","track":track,"clip":clip,"timing":0.3}]
    })).is_err());
    assert_eq!(host.p, before);
    assert_eq!(host.history.undo_labels().count(), 0);
    let selected = host.p.midi_notes(track, clip).unwrap()[3].id;
    let mut arguments = transform_arguments(&host, track, clip, "randomize");
    arguments["note_ids"] = json!([selected]);
    arguments["velocity"] = json!(32);
    arguments["timing"] = json!(0.1);
    arguments["seed"] = json!(1234);
    call(&mut host, "transform_phrase", arguments);
    for note in before.midi_notes(track, clip).unwrap() {
        let changed = host
            .p
            .midi_notes(track, clip)
            .unwrap()
            .iter()
            .find(|n| n.id == note.id)
            .unwrap();
        assert_eq!(
            (changed.id, changed.pitch, changed.length),
            (note.id, note.pitch, note.length)
        );
        if note.id != selected {
            assert_eq!(changed, note);
        }
    }
    assert_eq!(host.history.undo_labels().count(), 1);
}

#[test]
fn phrase_legato_exposes_rank_mode_and_preserves_the_default_next_onset() {
    let mut ranked = Host::default();
    let session = ranked.session;
    call(
        &mut ranked,
        "create_midi_part",
        json!({
            "session_id":session.id,"expected_revision":session.revision,"name":"Voices","start":0,"length":4,
            "notes":[{"pitch":72,"start":0,"length":0.25,"velocity":90},
            {"pitch":60,"start":0,"length":0.25,"velocity":80},
            {"pitch":74,"start":1,"length":0.25,"velocity":90},
            {"pitch":76,"start":2,"length":0.25,"velocity":90},
            {"pitch":64,"start":2,"length":0.25,"velocity":80}]
        }),
    );
    ranked.history = History::default();
    let mut default = clone_song_host(&ranked);
    let mut typed = clone_song_host(&ranked);
    let track = ranked.p.tracks[0].id;
    let clip = ranked.p.tracks[0].clips[0].id;
    let mut args = transform_arguments(&ranked, track, clip, "legato");
    args["mode"] = json!("top_to_bottom");
    call(&mut ranked, "transform_phrase", args);
    let args = transform_arguments(&default, track, clip, "legato");
    call(&mut default, "transform_phrase", args);
    let session = typed.session;
    call(
        &mut typed,
        "apply_edits",
        json!({
            "session_id":session.id,"expected_revision":session.revision,"label":"Ranked legato",
            "edits":[{"op":"legato_notes","track":track,"clip":clip,"mode":"top_to_bottom"}]
        }),
    );
    assert_eq!(ranked.p, typed.p);
    let length = |host: &Host, pitch| {
        host.p
            .midi_notes(track, clip)
            .unwrap()
            .iter()
            .find(|n| n.pitch == pitch)
            .unwrap()
            .length
    };
    assert_eq!(length(&ranked, 72), 1.0);
    assert_eq!(length(&ranked, 60), 2.0);
    assert_eq!(length(&default, 60), 1.0);
    assert_eq!(ranked.history.undo_labels().count(), 1);
}

#[test]
fn published_edit_schema_only_contains_supported_operations() {
    let schema = song_edit_schema();
    let ops = schema["oneOf"].as_array().unwrap();
    assert!(ops.len() > 50);
    for op in ops {
        assert_eq!(op["additionalProperties"], false);
        let name = op["properties"]["op"]["const"].as_str().unwrap();
        assert!(!["save", "open", "execute_script", "set_project"].contains(&name));
    }
    let specs = tool_specs();
    let count = specs.len();
    let names = specs
        .into_iter()
        .map(|s| s.name)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(names.len(), count, "Tool names must be unique");
    for required in [
        "apply_edits",
        "create_midi_part",
        "generate_part",
        "transform_phrase",
        "set_mix",
        "get_selection",
        "set_selection",
        "audio_analyze_clip",
        "get_recording",
        "configure_recording",
        "start_recording",
        "stop_recording",
        "retry_recording_input",
    ] {
        assert!(names.contains(required), "Missing tool {required}");
    }
}

#[test]
fn application_capabilities_are_separate_from_canonical_availability() {
    let song = song_tool_specs();
    let all = tool_specs();
    for name in [
        "document_status",
        "document_action",
        "get_setup",
        "configure_setup",
    ] {
        assert!(!song.iter().any(|spec| spec.name == name));
        assert!(all.iter().any(|spec| spec.name == name));
    }
    assert_eq!(tool_specs_for(ToolProfile::CurrentSong).len(), song.len());
    assert_eq!(tool_specs_for(ToolProfile::Application).len(), all.len());
}

fn schema_example(schema: &Value) -> Value {
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some(values) = schema.get("enum") {
        return values[0].clone();
    }
    for variants in ["oneOf", "anyOf"] {
        if let Some(values) = schema.get(variants) {
            return schema_example(&values[0]);
        }
    }
    match schema["type"].as_str().unwrap() {
        "object" => Value::Object(
            schema["properties"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, schema)| (name.clone(), schema_example(schema)))
                .collect(),
        ),
        "array" => Value::Array(
            (0..schema["minItems"].as_u64().unwrap_or(1))
                .map(|_| schema_example(&schema["items"]))
                .collect(),
        ),
        "integer" => json!(schema["minimum"].as_u64().unwrap_or(1)),
        "number" => json!(schema["minimum"].as_f64().unwrap_or(1.0)),
        "string" => json!("example"),
        "boolean" => json!(true),
        "null" => Value::Null,
        other => panic!("Unhandled schema type: {other}"),
    }
}

#[test]
fn every_published_edit_shape_deserializes_to_the_real_command_contract() {
    let schema = song_edit_schema();
    let variants = schema["oneOf"].as_array().unwrap();
    assert_eq!(variants.len(), 66);
    for variant in variants {
        let example = schema_example(variant);
        serde_json::from_value::<SongEdit>(example.clone()).unwrap_or_else(|error| {
            panic!("Published command schema disagrees with serde: {example}: {error}")
        });
    }
}
