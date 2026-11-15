//! The running application exposes the same bounded sample-tree model for
//! Sampler and Drums, with atomic edits and portable Project Sample references.
mod common;

use common::Harness;
use daw_core::{AudioData, Instrument, Project};
use serde_json::{Value, json};

fn call(app: &mut Harness, name: &str, arguments: Value) -> Value {
    let state = app.command(json!({"command":"song_tool","name":name,"arguments":arguments}));
    assert_eq!(
        state["tool_result"]["ok"], true,
        "{name}: {}",
        state["tool_result"]
    );
    state
}

fn mutation(app: &mut Harness, name: &str, mut arguments: Value) -> Value {
    let state = app.inspect();
    arguments["session_id"] = state["song_session"]["id"].clone();
    arguments["expected_revision"] = state["song_session"]["revision"].clone();
    call(app, name, arguments)
}

fn rejected(app: &mut Harness, name: &str, mut arguments: Value, code: &str) {
    let before = app.inspect();
    arguments["session_id"] = before["song_session"]["id"].clone();
    arguments["expected_revision"] = before["song_session"]["revision"].clone();
    let state = app.command(json!({"command":"song_tool","name":name,"arguments":arguments}));
    assert_eq!(
        state["tool_result"]["error"]["code"], code,
        "{}",
        state["tool_result"]
    );
    for key in ["project", "history", "song_session"] {
        assert_eq!(state[key], before[key], "Rejected {name} changed {key}");
    }
}

fn instrument(state: &Value, track: u64) -> &Value {
    &state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == track)
        .unwrap()["instrument"]["Builtin"]
}

fn sample(id: u64, slot: u32) -> Value {
    json!({"id":id,"kind":{"Sample":{"slot":slot,"start":0.0,"end":1.0,"reverse":false,"gain_db":0.0}}})
}

#[test]
fn project_samples_load_into_named_tree_leaves_in_both_instruments() {
    let mut app = Harness::new();
    let source = app.directory.path().join("short.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 480],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let sample_id = project
        .add_project_sample("Soft transient", source.to_string_lossy())
        .unwrap();
    let tracks: Vec<_> = ["r1-sampler", "r1-drums"]
        .iter()
        .map(|device| {
            let track = project.add_instrument_track(*device);
            project
                .set_instrument(track, Instrument::builtin(*device))
                .unwrap();
            track
        })
        .collect();
    let path = app.directory.path().join("players.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    std::fs::remove_file(source).unwrap();

    let listed = call(&mut app, "list_project_samples", json!({}));
    assert_eq!(
        listed["tool_result"]["result"]["samples"],
        json!([{"id":sample_id,"name":"Soft transient"}])
    );
    assert!(!listed["tool_result"]["result"].to_string().contains("path"));

    common::wait_until(&mut app, |state| {
        state["audio_sources"]["metadata_sources"] == 1
    });
    let removed = mutation(
        &mut app,
        "apply_edits",
        json!({"label":"Remove unused sample","edits":[
            {"op":"remove_project_sample","sample":sample_id}
        ]}),
    );
    assert_eq!(
        removed["audio_sources"]["metadata_sources"], 0,
        "Pool-only removal releases cached sources"
    );
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    common::wait_until(&mut app, |state| {
        state["audio_sources"]["metadata_sources"] == 1
    });

    for track in &tracks {
        let tree = json!({"id":1,"name":"Variation","kind":{"RoundRobin":{"children":[sample(2,42),sample(3,87)]}}});
        let state = mutation(
            &mut app,
            "set_sample_player",
            json!({"track_id":track,"slot":0,"player":tree}),
        );
        assert_eq!(instrument(&state, *track)["sample_players"]["0"], tree);
        let read = call(
            &mut app,
            "get_sample_player",
            json!({"track_id":track,"slot":0}),
        );
        assert_eq!(read["tool_result"]["result"]["player"], tree);
        rejected(
            &mut app,
            "set_sample_player",
            json!({"track_id":track,"slot":0}),
            "invalid_arguments",
        );
        rejected(
            &mut app,
            "apply_edits",
            json!({"label":"Invalid destination","edits":[
                {"op":"set_sample_player","track":track,"slot":16,"player":sample(1,0)}
            ]}),
            "invalid_arguments",
        );
        rejected(
            &mut app,
            "set_device_sample",
            json!({"track_id":track,"slot":0,"sample_id":sample_id}),
            "invalid_arguments",
        );
        rejected(
            &mut app,
            "set_device_sample",
            json!({"track_id":track,"slot":0,"node_id":99,"sample_id":sample_id}),
            "not_found",
        );
        rejected(
            &mut app,
            "set_device_sample",
            json!({"track_id":track,"slot":0,"node_id":2,"sample_id":u64::MAX}),
            "not_found",
        );
        let loaded = mutation(
            &mut app,
            "set_device_sample",
            json!({"track_id":track,"slot":0,"node_id":2,"sample_id":sample_id}),
        );
        assert_eq!(instrument(&loaded, *track)["samples"][0]["slot"], 42);
        assert_eq!(
            instrument(&loaded, *track)["samples"][0]["path"],
            loaded["project"]["samples"][0]["path"]
        );
        assert!(
            instrument(&loaded, *track)["samples"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["slot"] != 87)
        );
        mutation(
            &mut app,
            "rename_device_sample_slot",
            json!({"track_id":track,"slot":0,"name":"Soft hit"}),
        );
        let cleared = mutation(
            &mut app,
            "set_device_sample",
            json!({"track_id":track,"slot":0,"node_id":2,"clear":true}),
        );
        assert_eq!(
            instrument(&cleared, *track)["sample_names"]["0"],
            "Soft hit"
        );
        let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
        assert_eq!(instrument(&undone, *track)["samples"][0]["slot"], 42);
        assert_eq!(instrument(&undone, *track)["sample_names"]["0"], "Soft hit");
        let duplicate = json!({"id":1,"kind":{"All":{"children":[sample(2,42),sample(2,87)]}}});
        rejected(
            &mut app,
            "set_sample_player",
            json!({"track_id":track,"slot":0,"player":duplicate}),
            "invalid_arguments",
        );
        // The generic edit boundary also releases removed auxiliary bindings;
        // Undo restores both the tree and recording as one action.
        let removed = mutation(
            &mut app,
            "apply_edits",
            json!({"label":"Remove player","edits":[
                {"op":"set_sample_player","track":track,"slot":0,"player":null}
            ]}),
        );
        assert!(instrument(&removed, *track)["samples"].is_null());
        let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
        assert_eq!(instrument(&undone, *track)["samples"][0]["slot"], 42);
        assert_eq!(instrument(&undone, *track)["sample_players"]["0"], tree);
    }

    let saved = app.directory.path().join("retained.rill");
    let before = app.command(json!({"command":"save","path":saved}));
    let restored = app.command(json!({"command":"open","path":saved}));
    for track in tracks {
        let loaded = instrument(&restored, track);
        let expected = instrument(&before, track);
        assert_eq!(loaded["sample_players"], expected["sample_players"]);
        assert_eq!(loaded["sample_names"], expected["sample_names"]);
        assert_eq!(
            loaded["samples"][0]["path"],
            restored["project"]["samples"][0]["path"]
        );
    }
    app.quit();
}
