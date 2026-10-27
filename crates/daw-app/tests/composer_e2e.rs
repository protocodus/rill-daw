//! Native composition tools use the running app's assistant, MCP and history.
//! The deterministic provider fixture needs no credentials, plug-in code or device.
mod common;

use common::Harness;
use daw_core::{Clip, Instrument, Note, PluginParameterValue, Project, Track, TrackKind};
use serde_json::{Value, json};

const COMPOSER_TOOLS: [&str; 6] = [
    "composer_catalog",
    "composer_scale",
    "composer_chord",
    "composer_progression",
    "composer_euclidean",
    "compose_progression",
];

fn song_tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"song_tool","name":name,"arguments":arguments}))
}

fn result(state: &Value) -> &Value {
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    &state["tool_result"]["result"]
}

fn assert_song_unchanged(actual: &Value, expected: &Value) {
    assert_eq!(actual["project"], expected["project"]);
    assert_eq!(actual["history"], expected["history"]);
    assert_eq!(actual["song_session"], expected["song_session"]);
}

fn compose_args(state: &Value) -> Value {
    json!({
        "session_id":state["song_session"]["id"],
        "expected_revision":state["song_session"]["revision"],
        "name":"Four-chord arrangement", "start":8.0,
        "root":"C4", "scale":"major", "degrees":[1,5,6,4],
        "sevenths":false, "smooth":false, "beats_per_chord":4.0,
        "parts":[
            {"part":{"kind":"chords","name":"Pads","gate":0.5,"velocity":80}},
            {"part":{"kind":"arpeggio","name":"Arp","style":"up","step":0.5,"gate":0.5,"velocity":95}},
            {"part":{"kind":"bass","name":"Bass","style":"root","octave":2,"gate":0.5,"velocity":90}}
        ]
    })
}

fn track_named<'a>(state: &'a Value, name: &str) -> &'a Value {
    state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["name"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {state}"))
}

fn notes(clip: &Value) -> &[Value] {
    clip["content"]["Midi"]["notes"].as_array().unwrap()
}

fn pitches(notes: &[Value]) -> Vec<u64> {
    notes
        .iter()
        .map(|note| note["pitch"].as_u64().unwrap())
        .collect()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn enter_prompt(app: &mut Harness, opened: &Value, prompt: &str) {
    let rect = &opened["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"text","text":prompt}));
}

fn mcp(app: &mut Harness, request: Value) -> Value {
    app.command(json!({"command":"mcp_request","request":request}))
}

fn initialize_mcp(app: &mut Harness) {
    let initialized = mcp(
        app,
        json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize",
            "params":{"protocolVersion":"2025-11-25","capabilities":{},
                "clientInfo":{"name":"Composer E2E","version":"1"}}
        }),
    );
    assert_eq!(
        initialized["mcp_result"]["result"]["serverInfo"]["name"],
        "rill"
    );
    mcp(
        app,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );
}

#[test]
fn mcp_discovers_native_theory_and_reads_spelled_pitches_without_editing_the_song() {
    let mut app = Harness::new();
    let initial = app.inspect();
    initialize_mcp(&mut app);
    let listed = mcp(
        &mut app,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    let tools = listed["mcp_result"]["result"]["tools"].as_array().unwrap();
    for name in COMPOSER_TOOLS {
        assert!(
            tools.iter().any(|tool| tool["name"] == name),
            "Missing {name}"
        );
    }
    assert!(!tools.iter().any(|tool| tool["name"] == "notes_to_midi"));

    let catalog = song_tool(&mut app, "composer_catalog", json!({}));
    assert_eq!(result(&catalog)["scales"].as_array().unwrap().len(), 50);
    assert_eq!(result(&catalog)["chords"].as_array().unwrap().len(), 37);
    assert_song_unchanged(&catalog, &initial);

    let scale = mcp(
        &mut app,
        json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"composer_scale","arguments":{"root":"F4","scale":"major"}}
        }),
    );
    assert_eq!(scale["mcp_result"]["result"]["isError"], false, "{scale}");
    let value = &scale["mcp_result"]["result"]["structuredContent"];
    assert_eq!(
        value["notes"],
        json!(["F4", "G4", "A4", "Bb4", "C5", "D5", "E5", "F5"])
    );
    assert_eq!(value["midi"], json!([65, 67, 69, 70, 72, 74, 76, 77]));
    assert_song_unchanged(&scale, &initial);

    let chord = song_tool(&mut app, "composer_chord", json!({"root":"C4","chord":"9"}));
    assert_eq!(result(&chord)["midi"], json!([60, 64, 67, 70, 74]));
    assert_song_unchanged(&chord, &initial);
    let rhythm = song_tool(
        &mut app,
        "composer_euclidean",
        json!({"pulses":3,"steps":8}),
    );
    assert_eq!(result(&rhythm)["pattern"], "O..o..o.");
    assert_song_unchanged(&rhythm, &initial);

    let progression = song_tool(
        &mut app,
        "composer_progression",
        json!({
            "root":"C4","scale":"major","degrees":[1,5,6,4],"smooth":false
        }),
    );
    let chord_pitches: Vec<_> = result(&progression)["chords"]
        .as_array()
        .unwrap()
        .iter()
        .map(|chord| chord["midi"].clone())
        .collect();
    assert_eq!(
        chord_pitches,
        vec![
            json!([60, 64, 67]),
            json!([67, 71, 74]),
            json!([69, 72, 76]),
            json!([65, 69, 72])
        ]
    );
    assert_song_unchanged(&progression, &initial);
    app.quit();
}

#[test]
fn assistant_creates_three_complete_parts_in_one_mouse_undoable_operation() {
    let mut app = Harness::new();
    let initial = app.inspect();
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":[{"id":"compose","name":"compose_progression","arguments":compose_args(&initial)}]},
        {"text":"Created pads, arpeggio, and bass over C, G, Am, and F."}
    ]}));
    app.click("AI");
    let opened = app.inspect();
    let prompt = "Add four bars of C, G, Am, and F as pads, arpeggio, and bass at beat 8.";
    enter_prompt(&mut app, &opened, prompt);
    app.click("Send");
    let created = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(created["assistant"]["busy"], false, "{created}");
    assert!(
        created["assistant"]["messages"]
            .to_string()
            .contains("Created pads, arpeggio, and bass")
    );
    assert_eq!(created["project"]["tracks"].as_array().unwrap().len(), 3);
    assert_eq!(created["history"]["undo"].as_array().unwrap().len(), 1);

    let pads = &track_named(&created, "Pads")["clips"][0];
    assert_eq!(
        pitches(notes(pads)),
        vec![60, 64, 67, 67, 71, 74, 69, 72, 76, 65, 69, 72]
    );
    for (index, chord) in notes(pads).as_chunks::<3>().0.iter().enumerate() {
        for note in chord {
            near(note["start"].as_f64().unwrap(), index as f64 * 4.0);
            near(note["length"].as_f64().unwrap(), 2.0);
            assert_eq!(note["velocity"], 80);
        }
    }
    let arp = &track_named(&created, "Arp")["clips"][0];
    assert_eq!(notes(arp).len(), 32);
    assert_eq!(
        pitches(&notes(arp)[..8]),
        vec![60, 64, 67, 60, 64, 67, 60, 64]
    );
    for (index, note) in notes(arp).iter().enumerate() {
        near(note["start"].as_f64().unwrap(), index as f64 * 0.5);
        near(note["length"].as_f64().unwrap(), 0.25);
        assert_eq!(note["velocity"], 95);
    }
    let bass = &track_named(&created, "Bass")["clips"][0];
    assert_eq!(pitches(notes(bass)), vec![36, 43, 45, 41]);
    for (index, note) in notes(bass).iter().enumerate() {
        near(note["start"].as_f64().unwrap(), index as f64 * 4.0);
        near(note["length"].as_f64().unwrap(), 2.0);
        assert_eq!(note["velocity"], 90);
    }
    // The requested range includes each part's silent, gated tail.
    for clip in [pads, arp, bass] {
        assert_eq!(clip["start"], 8.0);
        assert_eq!(clip["length"], 16.0);
    }

    let undone = app.click("Undo");
    assert_eq!(undone["project"], initial["project"]);
    app.key("Escape");
    app.click("Edit");
    let redone = app.click("Redo");
    assert_eq!(redone["project"], created["project"]);
    app.click("Edit");
    let undone_again = app.click("Undo");
    assert_eq!(undone_again["project"], initial["project"]);
    app.quit();
}

#[test]
fn existing_instrument_target_retains_its_notes_mix_and_bus_when_adding_a_part() {
    let mut app = Harness::new();
    let fixture = common::plugin_fixture();
    let mut project = Project::new();
    let bus = Track::new("Music bus", TrackKind::Bus);
    let bus_id = bus.id;
    project.tracks.push(bus);
    let target_id = project.add_midi_track("Existing voice");
    let target = project.track_mut(target_id).unwrap();
    target.instrument = Instrument::Vst3 {
        path: fixture
            .path()
            .join("RillFixture.vst3")
            .to_string_lossy()
            .into_owned(),
        class_id: Some("11111111111111111111111111111111".into()),
    };
    target.instrument_state = vec![17, 19, 23];
    target.instrument_parameters = vec![PluginParameterValue { id: 7, value: 0.4 }];
    target.output_bus = Some(bus_id);
    target.gain = 0.42;
    target.pan = -0.25;
    target.clips.push(Clip::midi(
        "Existing phrase",
        0.0,
        4.0,
        vec![Note::new(55, 1.0, 1.0, 77)],
    ));
    let path = app.directory.path().join("existing-composition.rill");
    project.save(&path).unwrap();
    let loaded = app.command(json!({"command":"open","path":path}));
    let mut args = compose_args(&loaded);
    args["parts"] = json!([{"track_id":target_id,"part":{"kind":"chords","name":"Additional chords","gate":0.5}}]);
    let composed = song_tool(&mut app, "compose_progression", args);
    let report = result(&composed);
    assert_eq!(report["parts"][0]["track_id"], target_id);
    assert_eq!(report["parts"][0]["instrument_assigned"], true);
    assert_eq!(report["parts"][0]["note_count"], 12);
    assert_eq!(report["length"], 16.0);
    assert_eq!(composed["project"]["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(
        track_named(&composed, "Music bus"),
        track_named(&loaded, "Music bus")
    );
    let before = track_named(&loaded, "Existing voice");
    let after = track_named(&composed, "Existing voice");
    let mut expected = before.clone();
    expected["clips"]
        .as_array_mut()
        .unwrap()
        .push(after["clips"][1].clone());
    assert_eq!(
        after, &expected,
        "Appending a clip must preserve the entire instrument channel"
    );
    assert_eq!(after["clips"][1]["name"], "Additional chords");
    assert_eq!(after["clips"][1]["start"], 8.0);
    assert_eq!(after["clips"][1]["length"], 16.0);
    assert_eq!(composed["history"]["undo"].as_array().unwrap().len(), 1);
    app.click("Edit");
    let undone = app.click("Undo");
    assert_eq!(undone["project"], loaded["project"]);
    app.quit();
}

#[test]
fn stale_sessions_invalid_targets_and_oversized_drafts_never_partially_commit() {
    let mut app = Harness::new();
    let empty = app.inspect();
    initialize_mcp(&mut app);
    app.click("Track");
    let baseline = app.click("Add audio track");
    let audio_id = baseline["project"]["tracks"][0]["id"].clone();
    let stale = mcp(
        &mut app,
        json!({
            "jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"compose_progression","arguments":compose_args(&empty)}
        }),
    );
    assert_eq!(
        stale["mcp_result"]["result"]["structuredContent"]["error"]["code"],
        "conflict"
    );
    assert_song_unchanged(&stale, &baseline);

    let mut bad_target = compose_args(&baseline);
    // The first part could create a new track, but a later audio target rejects the whole operation.
    bad_target["parts"][1]["track_id"] = audio_id;
    let rejected = song_tool(&mut app, "compose_progression", bad_target);
    assert_eq!(rejected["tool_result"]["ok"], false);
    assert_song_unchanged(&rejected, &baseline);

    for (field, invalid) in [
        ("degrees", json!(vec![1; 33])),
        ("beats_per_chord", json!(2048.0)),
        ("parts", json!([])),
        ("root", json!("C20")),
    ] {
        let mut args = compose_args(&baseline);
        args[field] = invalid;
        let rejected = song_tool(&mut app, "compose_progression", args);
        assert_eq!(rejected["tool_result"]["ok"], false, "{field}: {rejected}");
        assert_song_unchanged(&rejected, &baseline);
    }
    app.click("File");
    app.click("New");
    let replacement = app.click("Don't save");
    assert_ne!(
        replacement["song_session"]["id"],
        baseline["song_session"]["id"]
    );
    let revoked = song_tool(&mut app, "compose_progression", compose_args(&baseline));
    assert_eq!(revoked["tool_result"]["error"]["code"], "conflict");
    assert_song_unchanged(&revoked, &replacement);
    app.quit();
}
