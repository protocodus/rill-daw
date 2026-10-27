//! Explicit application capability covers I/O/setup without widening song AI.
mod common;

use common::Harness;
use daw_core::AudioData;
use serde_json::{Value, json};

fn application(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"application_tool","name":name,"arguments":arguments}))
}
fn song(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"song_tool","name":name,"arguments":arguments}))
}
fn assert_ok(state: &Value) {
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
}
fn import_tone(app: &mut Harness) -> Value {
    let source = app.directory.path().join("application-source.wav");
    AudioData {
        sample_rate: 48_000,
        channels: 2,
        samples: (0..48_000)
            .flat_map(|frame| {
                let value =
                    (std::f64::consts::TAU * 440.0 * frame as f64 / 48_000.0).sin() as f32 * 0.25;
                [value, value]
            })
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    application(
        app,
        "document_action",
        json!({"action":"import","path":source,"beat":0.0}),
    )
}

#[test]
fn privileged_documents_import_save_export_and_open_without_weakening_song_tools() {
    let mut app = Harness::new();
    let saved = app.directory.path().join("application-song.rill");
    app.command(json!({"command":"mcp_request","request":{
        "jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},
            "clientInfo":{"name":"Application E2E","version":"1"}}
    }}));
    app.command(json!({"command":"mcp_request","request":{
        "jsonrpc":"2.0","method":"notifications/initialized"
    }}));
    let current = app.command(json!({"command":"mcp_request","request":{
        "jsonrpc":"2.0","id":2,"method":"tools/list"
    }}));
    assert!(
        !current["mcp_result"]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "document_action")
    );
    let general = app.command(
        json!({"command":"mcp_request","application":true,"request":{
            "jsonrpc":"2.0","id":3,"method":"tools/list"
        }}),
    );
    assert!(
        general["mcp_result"]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "document_action")
    );
    let mcp_denied = app.command(json!({"command":"mcp_request","request":{
        "jsonrpc":"2.0","id":4,"method":"tools/call",
        "params":{"name":"document_action","arguments":{"action":"save_as","path":saved}}
    }}));
    assert!(
        !mcp_denied["mcp_result"]["error"].is_null()
            || mcp_denied["mcp_result"]["result"]["isError"] == true
    );
    assert!(!saved.exists());
    let denied = song(
        &mut app,
        "document_action",
        json!({"action":"save_as","path":saved}),
    );
    assert_eq!(denied["tool_result"]["ok"], false);
    assert!(!saved.exists());

    let imported = import_tone(&mut app);
    assert_ok(&imported);
    assert_eq!(imported["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(imported["history"]["undo"].as_array().unwrap().len(), 1);

    let protected = application(&mut app, "document_action", json!({"action":"new"}));
    assert_eq!(protected["tool_result"]["ok"], false);
    assert_eq!(protected["project"], imported["project"]);
    assert_eq!(protected["document_guard"], Value::Null);

    let stored = application(
        &mut app,
        "document_action",
        json!({"action":"save_as","path":saved}),
    );
    assert_ok(&stored);
    assert!(saved.exists());
    assert_eq!(stored["modified"], false);
    let original = stored["project"].clone();
    let output = app.directory.path().join("application-mix.wav");
    let exported = application(
        &mut app,
        "document_action",
        json!({"action":"export","path":output,"include_tails":false}),
    );
    assert_ok(&exported);
    let audio = AudioData::load_wav(&output).unwrap();
    assert_eq!(audio.sample_rate, 48_000);
    assert_eq!(audio.samples.len(), 96_000);
    assert!(audio.samples.iter().any(|sample| sample.abs() > 0.1));

    let empty = application(&mut app, "document_action", json!({"action":"new"}));
    assert_ok(&empty);
    assert!(empty["project"]["tracks"].as_array().unwrap().is_empty());
    assert_ne!(empty["song_session"]["id"], stored["song_session"]["id"]);
    let opened = application(
        &mut app,
        "document_action",
        json!({"action":"open","path":saved}),
    );
    assert_ok(&opened);
    // Archives may extract their source to a different scratch path.
    let recalled = &opened["project"]["tracks"][0];
    assert_eq!(recalled["name"], original["tracks"][0]["name"]);
    assert_eq!(
        recalled["clips"][0]["length"],
        original["tracks"][0]["clips"][0]["length"]
    );
    assert_eq!(opened["modified"], false);

    song(
        &mut app,
        "apply_edits",
        json!({
            "session_id":opened["song_session"]["id"],"expected_revision":opened["song_session"]["revision"],
            "label":"Unsaved name","edits":[{"op":"rename_song","name":"Unsaved tool edit"}]
        }),
    );
    let discarded = application(
        &mut app,
        "document_action",
        json!({"action":"new","discard_unsaved":true}),
    );
    assert_ok(&discarded);
    assert!(
        discarded["project"]["tracks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(discarded["modified"], false);
    let recovery = application(
        &mut app,
        "document_action",
        json!({"action":"recover","discard_unsaved":true}),
    );
    assert_ok(&recovery);
    assert_eq!(recovery["project"]["name"], "Unsaved tool edit");
    assert_eq!(recovery["modified"], true);
    let status = application(&mut app, "document_status", json!({}));
    assert_ok(&status);
    assert_eq!(status["tool_result"]["result"]["pending"], false);
    app.quit();
}

#[test]
fn privileged_setup_encrypts_keys_redacts_reads_and_revokes_keys_on_provider_switch() {
    let mut app = Harness::new();
    let isolated_scratch = app.directory.path().join("application-scratch");
    let denied = song(
        &mut app,
        "configure_setup",
        json!({"scratch_dir":isolated_scratch}),
    );
    assert_eq!(denied["tool_result"]["ok"], false);
    assert!(!isolated_scratch.exists());
    let original = app.inspect();
    let key = "application-tool-fake-openai-key";
    let configured = application(
        &mut app,
        "configure_setup",
        json!({
            "scratch_dir":isolated_scratch,
            "ai":{"provider":"open_ai","model":"e2e-model","api_key":key}
        }),
    );
    assert_ok(&configured);
    assert_eq!(configured["project"], original["project"]);
    assert_eq!(configured["history"], original["history"]);
    assert!(!configured.to_string().contains(key));
    let settings_path = app.directory.path().join("config/settings.json");
    let encoded = std::fs::read_to_string(&settings_path).unwrap();
    assert!(!encoded.contains(key));
    let settings: Value = serde_json::from_str(&encoded).unwrap();
    let encrypted: daw_ai::EncryptedSecret =
        serde_json::from_value(settings["ai"]["api_key"].clone()).unwrap();
    assert_eq!(
        daw_ai::decrypt_api_key(&encrypted).unwrap().expose_secret(),
        key
    );
    let read = application(&mut app, "get_setup", json!({}));
    assert_ok(&read);
    let setup = &read["tool_result"]["result"];
    assert_eq!(setup["ai"]["api_key_configured"], true);
    assert_eq!(setup["ai"]["model"], "e2e-model");
    for family in ["outputs", "inputs", "midi"] {
        assert!(setup["available_devices"][family].is_array());
    }
    assert!(setup["ai"].get("api_key").is_none());
    assert!(!setup.to_string().contains(&encrypted.ciphertext));

    let invalid = application(
        &mut app,
        "configure_setup",
        json!({"ai":{"api_key":"another-fake-key","remove_api_key":true}}),
    );
    assert_eq!(invalid["tool_result"]["ok"], false);
    assert_eq!(std::fs::read_to_string(&settings_path).unwrap(), encoded);
    let changed = application(
        &mut app,
        "configure_setup",
        json!({"ai":{"provider":"anthropic"}}),
    );
    assert_ok(&changed);
    let read = application(&mut app, "get_setup", json!({}));
    assert_eq!(read["tool_result"]["result"]["ai"]["provider"], "anthropic");
    assert_eq!(
        read["tool_result"]["result"]["ai"]["api_key_configured"],
        false
    );
    let settings: Value = serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
    assert_eq!(settings["ai"]["api_key"], Value::Null);
    let replacement = "application-tool-fake-replacement-key";
    let configured = application(
        &mut app,
        "configure_setup",
        json!({
            "ai":{"provider":"open_ai","api_key":replacement}
        }),
    );
    assert_ok(&configured);
    assert!(!configured.to_string().contains(replacement));
    let settings: Value = serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
    let encrypted: daw_ai::EncryptedSecret =
        serde_json::from_value(settings["ai"]["api_key"].clone()).unwrap();
    assert_eq!(
        daw_ai::decrypt_api_key(&encrypted).unwrap().expose_secret(),
        replacement
    );
    app.quit();
}

#[test]
fn setup_discovers_devices_without_opening_streams_or_editing_the_song() {
    let mut app = Harness::with_virtual_audio(None, None);
    let original = app.inspect();
    let read = application(&mut app, "get_setup", json!({}));
    assert_ok(&read);
    let devices = &read["tool_result"]["result"]["available_devices"];
    assert_eq!(devices["outputs"][0]["name"], "Rill virtual output");
    assert_eq!(devices["outputs"][0]["is_default"], true);
    assert_eq!(devices["outputs"][0]["channels"], 2);
    assert_eq!(devices["inputs"][0]["name"], "Rill virtual input");
    assert_eq!(devices["inputs"][0]["channels"], 2);
    assert_eq!(devices["midi"][0]["id"], "rill-virtual-midi");
    assert_eq!(read["device_open"], false);
    assert_eq!(read["recording"]["input_ready"], false);
    assert_eq!(read["recording"]["midi"]["input_ready"], false);
    assert_eq!(read["project"], original["project"]);
    assert_eq!(read["history"], original["history"]);
    assert_eq!(read["settings"], original["settings"]);
    app.quit();
}
