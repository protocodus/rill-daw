//! Provider configuration uses real Setup controls and survives process restart.
mod common;

use common::Harness;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn ai_setup(app: &mut Harness) -> Value {
    app.click("Window");
    app.click("Setup");
    app.click("AI")
}

fn replace_field(app: &mut Harness, rect: &Value, text: &str) -> Value {
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}))
}

fn stored_settings(app: &Harness) -> (String, Value) {
    let raw = std::fs::read_to_string(app.directory.path().join("config/settings.json")).unwrap();
    let parsed = serde_json::from_str(&raw).unwrap();
    (raw, parsed)
}

fn assert_encrypted_key(settings: &Value, expected: &str) {
    let envelope = &settings["ai"]["api_key"];
    assert_eq!(envelope["version"], 1);
    assert_eq!(envelope["nonce"].as_str().unwrap().len(), 16);
    assert!(!envelope["ciphertext"].as_str().unwrap().is_empty());
    let encrypted: daw_ai::EncryptedSecret = serde_json::from_value(envelope.clone()).unwrap();
    let decrypted = daw_ai::decrypt_api_key(&encrypted).unwrap();
    assert_eq!(decrypted.expose_secret(), expected);
}

#[test]
fn setup_encrypts_credentials_restarts_and_discards_cancelled_replacements() {
    let mut app = Harness::new();
    let original = app.inspect();
    ai_setup(&mut app);
    app.click("OpenAI");
    app.click("Anthropic");
    let state = app.inspect();
    replace_field(
        &mut app,
        &state["assistant"]["model_rect"],
        "claude-test-model",
    );
    let state = app.inspect();
    let key = "sk-rill-e2e-placeholder-credential";
    let typed = replace_field(&mut app, &state["assistant"]["key_rect"], key);
    assert!(!typed.to_string().contains(key));
    let applied = app.click("Apply");
    assert_eq!(applied["settings"]["ai"]["provider"], "anthropic");
    assert_eq!(applied["settings"]["ai"]["model"], "claude-test-model");
    assert_eq!(applied["project"], original["project"]);
    assert_eq!(applied["history"], original["history"]);
    assert_eq!(applied["modified"], false);
    let (raw, saved) = stored_settings(&app);
    assert!(!raw.contains(key));
    assert_encrypted_key(&saved, key);

    app = app.restart();
    let restarted = app.inspect();
    assert_eq!(restarted["settings"]["ai"], saved["ai"]);
    assert!(!restarted.to_string().contains(key));
    let state = ai_setup(&mut app);
    let replacement = "sk-rill-e2e-cancelled-replacement";
    let typed = replace_field(&mut app, &state["assistant"]["key_rect"], replacement);
    assert!(!typed.to_string().contains(replacement));
    app.click("Cancel");
    let (cancelled_raw, cancelled) = stored_settings(&app);
    assert_eq!(cancelled, saved);
    assert!(!cancelled_raw.contains(replacement));

    // Re-encrypting the same credential must use a new authenticated nonce.
    let state = ai_setup(&mut app);
    replace_field(&mut app, &state["assistant"]["key_rect"], key);
    app.click("Apply");
    let (raw, reencrypted) = stored_settings(&app);
    assert!(!raw.contains(key));
    assert_encrypted_key(&reencrypted, key);
    assert_ne!(
        reencrypted["ai"]["api_key"]["nonce"],
        saved["ai"]["api_key"]["nonce"]
    );

    // A credential belongs to its provider. Changing provider must discard both
    // the saved credential and an unsubmitted replacement draft.
    ai_setup(&mut app);
    app.click("Anthropic");
    let state = app.click("Anthropic");
    assert_eq!(
        state["setup_draft"]["ai"]["api_key"],
        reencrypted["ai"]["api_key"]
    );
    replace_field(
        &mut app,
        &state["assistant"]["key_rect"],
        "sk-rill-e2e-old-provider-draft",
    );
    app.click("Anthropic");
    let switched = app.click("OpenAI");
    assert_eq!(switched["setup_draft"]["ai"]["api_key"], Value::Null);
    let applied = app.click("Apply");
    assert_eq!(applied["settings"]["ai"]["provider"], "open_ai");
    assert_eq!(applied["settings"]["ai"]["api_key"], Value::Null);
    let (raw, switched) = stored_settings(&app);
    assert_eq!(switched["ai"]["api_key"], Value::Null);
    assert!(!raw.contains("old-provider-draft"));
    app.quit();
}

#[test]
fn applying_ai_configuration_preserves_live_audio_project_and_history() {
    let mut app = Harness::with_virtual_audio(None, None);
    let source = app.directory.path().join("steady-audio.wav");
    daw_core::AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0.1; 48_000 * 20],
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop","paths":[source]}));
    app.click("Play");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = app.inspect();
        if state["engine"]["playing"] == true && state["playback_loading"] == false {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Playback did not prepare: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let before = app.command(json!({"command":"pump_audio","frames":4096}));
    let state = ai_setup(&mut app);
    replace_field(
        &mut app,
        &state["assistant"]["key_rect"],
        "sk-rill-e2e-live-placeholder",
    );
    let applied = app.click("Apply");
    assert_eq!(applied["project"], before["project"]);
    assert_eq!(applied["history"], before["history"]);
    assert_eq!(applied["modified"], before["modified"]);
    for field in [
        "engine_id",
        "generation",
        "requested_revision",
        "applied_revision",
    ] {
        assert_eq!(applied["engine"][field], before["engine"][field], "{field}");
    }
    assert_eq!(applied["engine"]["playing"], true);
    let after = app.command(json!({"command":"pump_audio","frames":4096}));
    assert_eq!(after["engine"]["engine_id"], before["engine"]["engine_id"]);
    assert_eq!(
        after["engine"]["generation"],
        before["engine"]["generation"]
    );
    assert!(
        after["engine"]["consumed_frames"].as_u64().unwrap()
            > before["engine"]["consumed_frames"].as_u64().unwrap()
    );
    let samples = after["audio_output"].as_array().unwrap();
    assert_eq!(samples.len(), 4096);
    for frame in &samples[samples.len() - 512..] {
        assert!(frame[0].as_f64().unwrap().abs() > 1e-5);
        assert!(frame[1].as_f64().unwrap().abs() > 1e-5);
    }
    app.quit();
}
