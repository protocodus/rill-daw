use daw_core::{EditBatch, EditResult, History, Project, SongSession};
use daw_tools::{
    SongToolHost, ToolEffect, ToolError, dispatch_song_tool, song_tool_specs, tool_effect,
};
use serde_json::{Value, json};

#[derive(Default)]
struct Host {
    project: Project,
    session: SongSession,
    history: History,
}
impl SongToolHost for Host {
    fn project(&self) -> &Project {
        &self.project
    }
    fn song_session(&self) -> SongSession {
        self.session
    }
    fn apply_edits(&mut self, batch: EditBatch) -> Result<EditResult, ToolError> {
        let before = self.project.clone();
        let result = batch
            .apply(&mut self.project)
            .map_err(|e| ToolError::new("edit_failed", e.to_string()))?;
        if result.changed {
            self.history.commit_named(&before, batch.label);
            self.session.bump();
        }
        Ok(result)
    }
}

fn args(host: &Host) -> Value {
    json!({"session_id":host.session.id,"expected_revision":host.session.revision,
        "start":0,"root":"C4","degrees":[1],"parts":[{"part":{"kind":"chords"}}]})
}

#[test]
fn theory_tools_are_discovered_read_only_and_reject_unrelated_authority() {
    let mut host = Host::default();
    let before = host.project.clone();
    for (name, arguments) in [
        ("composer_catalog", json!({})),
        ("composer_scale", json!({"root":"F4","scale":"major"})),
        ("composer_chord", json!({"root":"C4","chord":"9"})),
        (
            "composer_progression",
            json!({"root":"C4","degrees":[1,5,6,4]}),
        ),
        ("composer_euclidean", json!({"pulses":3,"steps":8})),
    ] {
        assert_eq!(tool_effect(name), ToolEffect::Read);
        assert!(song_tool_specs().iter().any(|s| s.name == name));
        dispatch_song_tool(&mut host, name, arguments.clone()).unwrap();
        let mut forbidden = arguments;
        forbidden["path"] = json!("/private/anything");
        assert_eq!(
            dispatch_song_tool(&mut host, name, forbidden)
                .unwrap_err()
                .code,
            "invalid_arguments"
        );
    }
    assert_eq!(tool_effect("compose_progression"), ToolEffect::Mutation);
    assert_eq!(tool_effect("composer_execute"), ToolEffect::Mutation);
    assert_eq!(
        dispatch_song_tool(&mut host, "composer_execute", json!({}))
            .unwrap_err()
            .code,
        "unknown_tool"
    );
    assert_eq!(host.project, before);
    assert_eq!(host.session.revision, 0);
}

#[test]
fn exact_note_budget_succeeds_and_an_extra_part_fails_atomically() {
    let mut host = Host::default();
    let mut request = args(&host);
    request["degrees"] = json!(vec![1; 32]);
    request["parts"] = json!([{"part":{"kind":"arpeggio","step":1.0/64.0}}]);
    let result = dispatch_song_tool(&mut host, "compose_progression", request.clone()).unwrap();
    assert_eq!(result["notes_total"], 8192);
    assert_eq!(result["length"], 128.0);
    let clip = &host.project.tracks[0].clips[0];
    assert_eq!(clip.length, 128.0);
    assert_eq!(
        host.project
            .midi_notes(host.project.tracks[0].id, clip.id)
            .unwrap()
            .len(),
        8192
    );
    let before = host.project.clone();
    let session = host.session;
    request["session_id"] = json!(session.id);
    request["expected_revision"] = json!(session.revision);
    request["parts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"part":{"kind":"bass"}}));
    assert_eq!(
        dispatch_song_tool(&mut host, "compose_progression", request)
            .unwrap_err()
            .code,
        "invalid_arguments"
    );
    assert_eq!(host.project, before);
    assert_eq!(host.session, session);
}

#[test]
fn published_parameter_bounds_execute_with_matching_part_grids() {
    let schema = song_tool_specs()
        .into_iter()
        .find(|s| s.name == "compose_progression")
        .unwrap()
        .input_schema;
    let fields = &schema["properties"];
    let variants = fields["parts"]["items"]["properties"]["part"]["oneOf"]
        .as_array()
        .unwrap();
    let arp = &variants[1]["properties"];
    let bass = &variants[2]["properties"];
    for (beats, part) in [
        (
            fields["beats_per_chord"]["minimum"].clone(),
            json!({"kind":"bass","octave":bass["octave"]["minimum"]}),
        ),
        (
            fields["beats_per_chord"]["minimum"].clone(),
            json!({"kind":"arpeggio","step":arp["step"]["minimum"]}),
        ),
        (
            fields["beats_per_chord"]["maximum"].clone(),
            json!({"kind":"arpeggio","step":arp["step"]["maximum"]}),
        ),
        (
            fields["beats_per_chord"]["maximum"].clone(),
            json!({"kind":"bass","style":"root_octave","step":bass["step"]["maximum"],"octave":bass["octave"]["maximum"]}),
        ),
    ] {
        let mut host = Host::default();
        let mut request = args(&host);
        request["beats_per_chord"] = beats;
        request["parts"] = json!([{"part":part}]);
        dispatch_song_tool(&mut host, "compose_progression", request).unwrap();
    }
}

#[test]
fn invalid_ranges_parameters_and_stale_requests_leave_the_song_intact() {
    let mut host = Host::default();
    let before = host.project.clone();
    let session = host.session;
    for patch in [
        json!({"start":999999.0}),
        json!({"start":-1}),
        json!({"root":"C"}),
        json!({"root":"G9"}),
        json!({"degrees":[0]}),
        json!({"scale":"whole tone"}),
        json!({"parts":[{"part":{"kind":"chords","gate":0}}]}),
        json!({"parts":[{"part":{"kind":"bass","style":"execute_script"}}]}),
        json!({"parts":[{"part":{"kind":"arpeggio","step":0.000001}}]}),
        json!({"parts":[{"part":{"kind":"chords","path":"/private"}}]}),
    ] {
        let mut request = args(&host);
        for (key, value) in patch.as_object().unwrap() {
            request[key] = value.clone();
        }
        assert!(
            dispatch_song_tool(&mut host, "compose_progression", request).is_err(),
            "{patch}"
        );
        assert_eq!(host.project, before);
        assert_eq!(host.session, session);
    }
    let mut stale = args(&host);
    stale["expected_revision"] = json!(1);
    assert_eq!(
        dispatch_song_tool(&mut host, "compose_progression", stale)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(host.project, before);
}
