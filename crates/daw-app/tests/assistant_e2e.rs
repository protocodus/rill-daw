//! The integrated assistant, MCP, and manual editor share the real app session.
mod common;

use common::Harness;
use daw_core::{AudioData, Clip, Project};
use serde_json::{Value, json};

#[test]
fn delayed_history_mutations_cannot_undo_a_newer_manual_edit() {
    let mut app = common::Harness::new();
    let initial = app.inspect();
    let created = app.command(json!({"command":"song_tool","name":"create_midi_part","arguments":{
        "session_id":initial["song_session"]["id"],"expected_revision":initial["song_session"]["revision"],
        "name":"First phrase","start":0.0,"length":4.0,"notes":[{"pitch":60,"start":0.0,"length":1.0,"velocity":90}]
    }}));
    assert_eq!(created["tool_result"]["ok"], true);
    app.click("Track");
    let manual = app.click("Add audio track");
    let stale = app.command(json!({"command":"song_tool","name":"history","arguments":{
        "action":"undo","session_id":created["song_session"]["id"],"expected_revision":created["song_session"]["revision"]
    }}));
    assert_eq!(stale["tool_result"]["error"]["code"], "conflict");
    assert_eq!(stale["project"], manual["project"]);
    assert_eq!(stale["history"], manual["history"]);
    let unfenced =
        app.command(json!({"command":"song_tool","name":"history","arguments":{"action":"undo"}}));
    assert_eq!(
        unfenced["tool_result"]["error"]["code"],
        "invalid_arguments"
    );
    assert_eq!(unfenced["project"], manual["project"]);
    let undone = app.command(json!({"command":"song_tool","name":"history","arguments":{
        "action":"undo","session_id":manual["song_session"]["id"],"expected_revision":manual["song_session"]["revision"]
    }}));
    assert_eq!(undone["tool_result"]["ok"], true);
    assert_eq!(undone["project"], created["project"]);
    app.quit();
}

fn song_tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"song_tool", "name":name, "arguments":arguments}))
}

fn mcp(app: &mut Harness, request: Value) -> Value {
    app.command(json!({"command":"mcp_request", "request":request}))
}

fn part_arguments(state: &Value, name: &str) -> Value {
    json!({
        "session_id":state["song_session"]["id"],
        "expected_revision":state["song_session"]["revision"],
        "name":name, "start":8.0, "length":8.0,
        "notes":[
            {"pitch":36,"start":0.0,"length":0.75,"velocity":110},
            {"pitch":43,"start":1.0,"length":0.75,"velocity":95},
            {"pitch":36,"start":2.0,"length":0.75,"velocity":105},
            {"pitch":46,"start":3.0,"length":0.75,"velocity":90},
            {"pitch":41,"start":4.0,"length":0.75,"velocity":110},
            {"pitch":48,"start":5.0,"length":0.75,"velocity":95},
            {"pitch":41,"start":6.0,"length":0.75,"velocity":105},
            {"pitch":43,"start":7.0,"length":0.75,"velocity":90}
        ]
    })
}

fn enter_prompt(app: &mut Harness, state: &Value, prompt: &str) {
    let rect = &state["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().expect("prompt left")
        + rect["max"][0].as_f64().expect("prompt right"))
        * 0.5;
    let y = (rect["min"][1].as_f64().expect("prompt top")
        + rect["max"][1].as_f64().expect("prompt bottom"))
        * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"text","text":prompt}));
}

#[test]
fn prompt_creates_a_complete_phrase_and_gui_undo_redo_reverses_one_edit() {
    let mut app = Harness::new();
    let original = app.inspect();
    let args = part_arguments(&original, "Assistant bass");
    app.command(json!({
        "command":"assistant_script",
        "turns":[
            {"calls":[{"id":"context","name":"get_song_context","arguments":{}}]},
            {"calls":[{"id":"phrase","name":"create_midi_part","arguments":args}]},
            {"text":"Created the eight-note bass phrase at beat 8."}
        ]
    }));

    app.click("Window");
    let opened = app.click("Assistant");
    assert_eq!(opened["project"], original["project"]);
    let prompt = "Write a two-bar bass phrase starting at beat 8.";
    enter_prompt(&mut app, &opened, prompt);
    assert_eq!(app.inspect()["assistant"]["prompt"], prompt);
    app.click("Send");
    let created = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(created["assistant"]["busy"], false, "{created}");
    assert_eq!(created["assistant"]["prompt"], "");
    let transcript = created["assistant"]["messages"].to_string();
    assert!(transcript.contains(prompt), "{transcript}");
    assert!(
        transcript.contains("Created the eight-note bass phrase"),
        "{transcript}"
    );

    let tracks = created["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0]["name"], "Assistant bass");
    let clip = &tracks[0]["clips"][0];
    assert_eq!(clip["start"], 8.0);
    assert_eq!(clip["length"], 8.0);
    let notes = clip["content"]["Midi"]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 8);
    let pitches: Vec<_> = notes
        .iter()
        .map(|note| note["pitch"].as_u64().unwrap())
        .collect();
    assert_eq!(pitches, vec![36, 43, 36, 46, 41, 48, 41, 43]);
    assert_eq!(notes[7]["start"], 7.0);
    assert_eq!(notes[7]["length"], 0.75);
    assert_eq!(created["history"]["undo"].as_array().unwrap().len(), 1);
    assert!(
        created["song_session"]["revision"].as_u64().unwrap()
            > original["song_session"]["revision"].as_u64().unwrap()
    );

    // This is the assistant's actual shared Undo control, not a test-only edit.
    let undone = app.click("Undo");
    assert_eq!(undone["project"], original["project"]);
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redone["project"], created["project"]);
    assert!(
        redone["song_session"]["revision"].as_u64().unwrap()
            > created["song_session"]["revision"].as_u64().unwrap()
    );
    app.quit();
}

#[test]
fn mcp_and_manual_edits_share_state_and_reject_failed_stale_or_file_operations() {
    let mut app = Harness::new();
    let initial = app.inspect();
    let initialized = mcp(
        &mut app,
        json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-11-25","capabilities":{},
                "clientInfo":{"name":"Rill E2E","version":"1"}}
        }),
    );
    assert_eq!(
        initialized["mcp_result"]["result"]["serverInfo"]["name"],
        "rill"
    );
    mcp(
        &mut app,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );
    let catalog = mcp(
        &mut app,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    let names: Vec<_> = catalog["mcp_result"]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"create_midi_part"));
    assert!(names.contains(&"apply_edits"));
    assert!(
        !names
            .iter()
            .any(|name| name.contains("save") || name.contains("open_project"))
    );

    let created = mcp(
        &mut app,
        json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"create_midi_part","arguments":part_arguments(&initial,"MCP bass")}
        }),
    );
    assert_eq!(
        created["mcp_result"]["result"]["isError"], false,
        "{created}"
    );
    assert_eq!(created["project"]["tracks"][0]["name"], "MCP bass");
    let listed = song_tool(&mut app, "list_tracks", json!({}));
    assert_eq!(
        listed["tool_result"]["result"]["tracks"][0]["name"],
        "MCP bass"
    );
    assert_eq!(listed["song_session"], created["song_session"]);

    // A normal menu edit invalidates the revision previously returned to MCP.
    app.click("Track");
    let manual = app.click("Add audio track");
    assert_eq!(manual["project"]["tracks"].as_array().unwrap().len(), 2);
    assert!(
        manual["song_session"]["revision"].as_u64().unwrap()
            > created["song_session"]["revision"].as_u64().unwrap()
    );
    let stale = song_tool(
        &mut app,
        "create_midi_part",
        part_arguments(&created, "Stale part"),
    );
    assert_eq!(stale["tool_result"]["ok"], false);
    assert_eq!(stale["tool_result"]["error"]["code"], "conflict");
    assert_eq!(stale["project"], manual["project"]);
    assert_eq!(stale["history"], manual["history"]);

    // The first operation would succeed alone; invalid tempo rolls both back.
    let failed = song_tool(
        &mut app,
        "apply_edits",
        json!({
            "session_id":manual["song_session"]["id"],
            "expected_revision":manual["song_session"]["revision"],
            "label":"Must be atomic",
            "edits":[{"op":"rename_song","name":"Must not commit"},
                     {"op":"set_tempo","bpm":1000.0}]
        }),
    );
    assert_eq!(failed["tool_result"]["ok"], false);
    assert_eq!(failed["project"], manual["project"]);
    assert_eq!(failed["history"], manual["history"]);
    assert_eq!(failed["song_session"], manual["song_session"]);

    let forbidden_path = app.directory.path().join("forbidden-save.rill");
    let forbidden = song_tool(&mut app, "save_project", json!({"path":forbidden_path}));
    assert_eq!(forbidden["tool_result"]["ok"], false);
    assert!(!forbidden_path.exists());
    assert_eq!(forbidden["project"], manual["project"]);

    app.click("File");
    app.click("New");
    let replacement = app.click("Don't save");
    assert_ne!(
        replacement["song_session"]["id"],
        manual["song_session"]["id"]
    );
    let revoked = mcp(
        &mut app,
        json!({
            "jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"create_midi_part","arguments":part_arguments(&manual,"Wrong song")}
        }),
    );
    assert_eq!(revoked["mcp_result"]["result"]["isError"], true);
    assert_eq!(
        revoked["mcp_result"]["result"]["structuredContent"]["error"]["code"],
        "conflict"
    );
    assert_eq!(revoked["project"], replacement["project"]);
    assert!(revoked["project"]["tracks"].as_array().unwrap().is_empty());
    app.quit();
}

#[test]
fn audio_tool_measures_the_rendered_song_and_observes_shared_mix_edits() {
    let mut app = Harness::new();
    let source = app.directory.path().join("assistant-analysis.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000)
            .flat_map(|frame| {
                let value =
                    (std::f64::consts::TAU * 1000.0 * frame as f64 / 48_000.0).sin() as f32 * 0.5;
                [value, -value * 0.5]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let track = project.add_audio_track("Measured audio");
    let channel = project.track_mut(track).unwrap();
    channel.gain = 1.0;
    channel.clips.push(Clip::audio(
        "Known stereo tone",
        0.0,
        2.0,
        source.to_string_lossy(),
    ));
    let path = app.directory.path().join("analysis.rill");
    project.save(&path).unwrap();
    let loaded = app.command(json!({"command":"open","path":path}));
    let range = json!({"start_beat":0.0,"end_beat":2.0});
    let measured = song_tool(&mut app, "audio_analyze", range.clone());
    assert_eq!(measured["tool_result"]["ok"], true, "{measured}");
    assert_eq!(measured["project"], loaded["project"]);
    assert_eq!(measured["history"], loaded["history"]);
    assert_eq!(measured["song_session"], loaded["song_session"]);
    let audio = &measured["tool_result"]["result"];
    assert_eq!(audio["frames"], 48_000);
    assert_eq!(audio["duration_seconds"], 1.0);
    assert_eq!(audio["silent"], false);
    assert_eq!(audio["frames_at_or_above_full_scale"], 0);
    let correlation = audio["stereo_correlation"].as_f64().unwrap();
    assert!((correlation + 1.0).abs() < 1e-6, "{audio}");
    let left = audio["sample_peak_linear"]["left"].as_f64().unwrap();
    let right = audio["sample_peak_linear"]["right"].as_f64().unwrap();
    assert!((left / right - 2.0).abs() < 1e-5, "{audio}");
    assert!(
        audio["listening"]
            .as_str()
            .unwrap()
            .contains("no audio was sent")
    );
    let rms = audio["rms_linear"].as_f64().unwrap();
    assert!(rms > 0.1);

    let mixed = song_tool(
        &mut app,
        "set_mix",
        json!({
            "session_id":loaded["song_session"]["id"],
            "expected_revision":loaded["song_session"]["revision"],
            "channels":[{"track":track,"gain_db":-6.020599913279624}]
        }),
    );
    assert_eq!(mixed["tool_result"]["ok"], true, "{mixed}");
    let quieter = song_tool(&mut app, "audio_analyze", range);
    assert_eq!(quieter["tool_result"]["ok"], true, "{quieter}");
    let quiet_rms = quieter["tool_result"]["result"]["rms_linear"]
        .as_f64()
        .unwrap();
    assert!((quiet_rms / rms - 0.5).abs() < 1e-5, "{quieter}");
    assert_eq!(quieter["history"]["undo"].as_array().unwrap().len(), 1);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], loaded["project"]);
    app.quit();
}

#[test]
fn assistant_blocks_background_edit_shortcuts_with_and_without_prompt_focus() {
    let mut app = Harness::new();
    let initial = app.inspect();
    let created = song_tool(
        &mut app,
        "create_midi_part",
        part_arguments(&initial, "Protected part"),
    );
    let track = &created["project"]["tracks"][0];
    song_tool(
        &mut app,
        "set_selection",
        json!({
            "track_id":track["id"], "clip_id":track["clips"][0]["id"]
        }),
    );
    app.click("Window");
    let opened = app.click("Assistant");
    assert!(
        opened["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "AI prompt")
    );

    // The prompt has not received focus: the utility itself must gate shortcuts.
    for command in [
        json!({"command":"key","key":"Delete"}),
        json!({"command":"key","key":"Backspace"}),
        json!({"command":"key","key":"D","cmd":true}),
    ] {
        let state = app.command(command);
        assert_eq!(state["project"], opened["project"]);
        assert_eq!(state["history"], opened["history"]);
        assert_eq!(state["song_session"], opened["song_session"]);
    }

    enter_prompt(&mut app, &opened, "Keep this draft");
    app.command(json!({"command":"key","key":"D","cmd":true}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let typing = app.key("Backspace");
    assert_eq!(typing["assistant"]["prompt"], "");
    assert_eq!(typing["project"], opened["project"]);
    assert_eq!(typing["history"], opened["history"]);
    assert_eq!(typing["song_session"], opened["song_session"]);
    app.quit();
}

#[test]
fn closing_assistant_utility_keeps_its_current_run_working() {
    let mut app = Harness::new();
    let initial = app.inspect();
    // Sequential host requests guarantee that work is pending when Escape is
    // processed, without adding sleeps or timing hooks to the production agent.
    let calls: Vec<_> = (0..32)
        .map(|index| {
            json!({
                "id":format!("context-{index}"), "name":"get_song_context", "arguments":{}
            })
        })
        .collect();
    app.command(json!({"command":"assistant_script", "turns":[
        {"calls":calls}, {"text":"Finished while the song remained visible."}
    ]}));
    app.click("Window");
    let opened = app.click("Assistant");
    enter_prompt(&mut app, &opened, "Inspect the song while I keep editing.");
    let sent = app.click("Send");
    assert_eq!(sent["assistant"]["busy"], true);
    let closed = app.key("Escape");
    assert_eq!(closed["assistant"]["busy"], true);
    assert!(
        !closed["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "AI prompt")
    );
    let finished = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(finished["assistant"]["busy"], false);
    assert_eq!(finished["assistant"]["status"], "Done");
    assert!(
        finished["assistant"]["messages"]
            .to_string()
            .contains("Finished while the song remained visible.")
    );
    assert_eq!(finished["project"], initial["project"]);
    assert_eq!(finished["history"], initial["history"]);
    app.quit();
}

#[test]
fn song_query_during_a_manual_fader_drag_does_not_split_its_undo_step() {
    let mut app = Harness::new();
    let initial = app.inspect();
    let before = song_tool(
        &mut app,
        "create_midi_part",
        part_arguments(&initial, "Live fader"),
    );
    let track = &before["project"]["tracks"][0]["id"];
    let rect = &before["timeline"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == *track)
        .expect("visible channel")["gain"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"down","x":x,"y":y}));
    let dragging = app.command(json!({"command":"pointer","phase":"move","x":x-25.0,"y":y}));
    assert_ne!(
        dragging["project"]["tracks"][0]["gain"],
        before["project"]["tracks"][0]["gain"]
    );
    assert_eq!(dragging["history"], before["history"]);

    let queried = song_tool(&mut app, "get_song_context", json!({}));
    assert_eq!(queried["tool_result"]["ok"], false);
    assert_eq!(queried["tool_result"]["error"]["code"], "busy");
    assert_eq!(queried["history"], before["history"]);
    assert_eq!(queried["song_session"], before["song_session"]);
    app.command(json!({"command":"pointer","phase":"move","x":x-40.0,"y":y}));
    let released = app.command(json!({"command":"pointer","phase":"up","x":x-40.0,"y":y}));
    assert_eq!(
        released["history"]["undo"].as_array().unwrap().len(),
        before["history"]["undo"].as_array().unwrap().len() + 1
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], before["project"]);
    app.quit();
}
