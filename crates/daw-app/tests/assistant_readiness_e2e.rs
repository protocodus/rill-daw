//! Assistant readiness and recovery stay attached to the real song and draft.
//! Provider scripts exercise production dispatch without credentials or network.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn footer_visible(state: &Value, text: &str) -> bool {
    let top = state["assistant"]["prompt_rect"]["max"][1]
        .as_f64()
        .unwrap();
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rect| rect["text"] == text && rect["min"][1].as_f64().is_some_and(|y| y >= top))
}

fn unchanged(actual: &Value, expected: &Value) {
    for field in ["project", "history", "song_session", "modified"] {
        assert_eq!(actual[field], expected[field], "Changed {field}");
    }
}

fn replace_field(app: &mut Harness, rect: &Value, text: &str) -> Value {
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}))
}

fn prompt(app: &mut Harness, text: &str) -> Value {
    let state = app.inspect();
    replace_field(app, &state["assistant"]["prompt_rect"], text)
}

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn midi_context() -> (Harness, u64, u64) {
    let mut app = Harness::new();
    let mut project = Project::new();
    let track = project.add_midi_track("Idea keys");
    let clip = Clip::midi("My phrase", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 90)]);
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("readiness.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("My phrase");
    menu(&mut app, "Window", "Clip Editor");
    app.click("AI for this context");
    (app, track, clip_id)
}

fn assert_action_fits(state: &Value, name: &str, width: f64, height: f64) {
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|rect| rect["text"] == name)
        .unwrap_or_else(|| panic!("Missing {name}: {}", state["visible_labels"]));
    assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{rect}");
    assert!(rect["min"][1].as_f64().unwrap() >= 0.0, "{rect}");
    assert!(rect["max"][0].as_f64().unwrap() <= width, "{rect}");
    assert!(rect["max"][1].as_f64().unwrap() <= height, "{rect}");
}

#[test]
fn connect_ai_returns_to_the_same_scoped_draft_without_sending_or_editing() {
    for dark in [false, true] {
        let (mut app, _, _) = midi_context();
        app.command(json!({"command":"viewport","width":800,"height":600}));
        app.command(json!({"command":"system_theme","dark":dark}));
        let original = app.inspect();
        assert_eq!(original["assistant"]["submit_state"], "SetupRequired");
        assert!(visible(&original, "Connect AI"));
        assert_action_fits(&original, "Connect AI", 800.0, 600.0);
        let starter = daw_ai::scoped_prompt_examples("midi_clip").first().unwrap();
        assert!(visible(&original, starter.title));
        assert!(!visible(&original, "Start a song"));
        let inserted = app.click(starter.title);
        assert_eq!(inserted["assistant"]["prompt"], starter.prompt);
        assert_eq!(
            inserted["assistant"]["scope"],
            original["assistant"]["scope"]
        );
        assert_eq!(inserted["assistant"]["messages"], json!([]));
        assert_eq!(inserted["assistant"]["busy"], false);
        unchanged(&inserted, &original);
        let draft = "Keep the opening motif; suggest a gentler ending.";
        prompt(&mut app, draft);

        // The send shortcut does not connect or submit behind a disabled state.
        let shortcut = app.command(json!({"command":"key","key":"Enter","cmd":true}));
        assert_eq!(shortcut["assistant"]["busy"], false);
        assert_eq!(shortcut["assistant"]["messages"], json!([]));
        assert_eq!(shortcut["assistant"]["prompt"], draft);
        assert_eq!(shortcut["popups"]["setup"], false);
        unchanged(&shortcut, &original);

        let setup = app.click("Connect AI");
        assert_eq!(setup["popups"]["setup"], true);
        assert!(visible(&setup, "Provider"));
        assert!(visible(&setup, "API key"));
        assert_eq!(setup["assistant"]["scope"], original["assistant"]["scope"]);
        assert_eq!(setup["assistant"]["prompt"], draft);
        let cancelled = app.click("Cancel");
        assert!(visible(&cancelled, "Connect AI"));
        assert_eq!(cancelled["assistant"]["prompt"], draft);
        unchanged(&cancelled, &original);

        let setup = app.click("Connect AI");
        let placeholder = "sk-readiness-e2e-placeholder-not-a-real-key";
        let entered = replace_field(&mut app, &setup["assistant"]["key_rect"], placeholder);
        assert!(!entered.to_string().contains(placeholder));
        let applied = app.click("Apply");
        assert!(visible(&applied, "Send"));
        assert!(!visible(&applied, "Connect AI"));
        assert_eq!(applied["assistant"]["submit_state"], "Ready");
        assert_eq!(
            applied["assistant"]["scope"],
            original["assistant"]["scope"]
        );
        assert_eq!(applied["assistant"]["prompt"], draft);
        assert_eq!(applied["assistant"]["messages"], json!([]));
        assert_eq!(applied["assistant"]["busy"], false);
        unchanged(&applied, &original);
        app.quit();
    }
}

fn part_call(state: &Value) -> Value {
    json!({"id":"create-idea","name":"create_midi_part","arguments":{
        "session_id":state["song_session"]["id"],
        "expected_revision":state["song_session"]["revision"],
        "name":"Retained idea","start":0.0,"length":4.0,
        "notes":[{"pitch":60,"start":0.0,"length":1.0,"velocity":90}]
    }})
}

#[test]
fn failed_requests_offer_editable_recovery_and_keep_any_committed_music() {
    for commit_first in [false, true] {
        let mut app = Harness::new();
        let original = app.inspect();
        app.command(json!({"command":"viewport","width":800,"height":600}));
        app.click("AI");
        let draft = "Make one four-beat idea, then explain its harmony.";
        prompt(&mut app, draft);
        app.click("Tips");
        let turns = if commit_first {
            json!([{"calls":[part_call(&original)]}])
        } else {
            json!([])
        };
        let ready = app.command(json!({"command":"assistant_script","turns":turns}));
        assert_eq!(ready["assistant"]["configured"], false);
        assert_eq!(ready["assistant"]["submit_state"], "Ready");
        app.click("Send");
        let completed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
        // Worker completion and the automatic scroll-to-error animation have
        // separate lifetimes. Wait for the actual detail to enter the viewport.
        common::wait_visible(&mut app, "Assistant fixture ran out of responses");
        let failed = app.inspect();
        unchanged(&failed, &completed);
        assert_eq!(failed["assistant"]["busy"], false);
        assert_eq!(failed["assistant"]["page"], "Conversation");
        assert_eq!(
            failed["assistant"]["failure"],
            "Assistant fixture ran out of responses"
        );
        assert_eq!(failed["assistant"]["recover_prompt"], true);
        assert_eq!(failed["assistant"]["prompt"], "");
        assert!(
            visible(&failed, "Assistant fixture ran out of responses"),
            "Failure text missing: {}",
            failed["visible_labels"]
        );
        assert!(visible(&failed, "Edit last prompt"));
        assert!(!footer_visible(&failed, "Stop"));
        assert_eq!(footer_visible(&failed, "Undo"), commit_first);
        assert_action_fits(&failed, "Edit last prompt", 800.0, 600.0);
        if commit_first {
            assert_eq!(failed["project"]["tracks"].as_array().unwrap().len(), 1);
            assert_eq!(failed["project"]["tracks"][0]["name"], "Retained idea");
            assert_eq!(failed["history"]["undo"].as_array().unwrap().len(), 1);
        } else {
            unchanged(&failed, &original);
        }

        let recovered = app.click("Edit last prompt");
        assert_eq!(recovered["assistant"]["prompt"], draft);
        assert_eq!(recovered["assistant"]["busy"], false);
        assert_eq!(
            recovered["assistant"]["messages"],
            failed["assistant"]["messages"]
        );
        unchanged(&recovered, &failed);

        let replacement = "Explain the existing music without making further edits.";
        prompt(&mut app, replacement);
        let typing = app.inspect();
        assert!(!visible(&typing, "Edit last prompt"));
        let protected = menu(&mut app, "More", "Edit last prompt");
        assert_eq!(protected["assistant"]["prompt"], replacement);
        unchanged(&protected, &failed);
        if protected["native_menu"]["open"] == true {
            app.key("Escape");
        }
        app.command(json!({"command":"assistant_script","turns":[
            {"text":"The existing idea is unchanged."}
        ]}));
        app.click("Send");
        let finished = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
        assert_eq!(finished["assistant"]["failure"], Value::Null);
        assert_eq!(finished["assistant"]["recover_prompt"], false);
        assert_eq!(finished["assistant"]["status"], "Done");
        assert!(!visible(&finished, "Edit last prompt"));
        unchanged(&finished, &failed);

        app.command(json!({"command":"assistant_script","turns":[]}));
        prompt(&mut app, "Explain the existing idea.");
        app.click("Send");
        let failed_again = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
        assert!(!failed_again["assistant"]["failure"].is_null());
        let unsent = "Keep this draft when starting a fresh conversation.";
        prompt(&mut app, unsent);
        let cleared = menu(&mut app, "More", "New chat");
        assert_eq!(cleared["assistant"]["failure"], Value::Null);
        assert_eq!(cleared["assistant"]["recover_prompt"], false);
        assert_eq!(cleared["assistant"]["prompt"], unsent);
        assert_eq!(cleared["assistant"]["messages"], json!([]));
        unchanged(&cleared, &failed);
        app.quit();
    }
}

#[test]
fn a_missing_pinned_clip_explains_why_send_is_blocked_without_broadening_scope() {
    let (mut app, track, clip) = midi_context();
    let pinned = app.inspect();
    prompt(&mut app, "Explain only this phrase.");
    let deleted = app.command(
        json!({"command":"song_tool","name":"apply_edits","arguments":{
            "session_id":pinned["song_session"]["id"],
            "expected_revision":pinned["song_session"]["revision"],
            "label":"Remove pinned phrase","edits":[{"op":"delete_clip","track":track,"clip":clip}]
        }}),
    );
    assert_eq!(deleted["tool_result"]["ok"], true, "{deleted}");
    assert_eq!(deleted["assistant"]["submit_state"], "Blocked");
    let reason = deleted["assistant"]["submit_reason"].as_str().unwrap();
    assert!(!reason.is_empty());
    assert!(visible(&deleted, reason), "{}", deleted["visible_labels"]);
    assert!(visible(&deleted, "Send"));
    assert!(!visible(&deleted, "Connect AI"));
    assert_eq!(deleted["assistant"]["scope"], pinned["assistant"]["scope"]);
    let click = app.click("Send");
    assert_eq!(click["assistant"]["busy"], false);
    assert_eq!(click["assistant"]["messages"], json!([]));
    assert_eq!(click["assistant"]["prompt"], "Explain only this phrase.");
    unchanged(&click, &deleted);
    app.quit();
}

#[test]
fn changing_scope_or_song_clears_stale_failure_and_recovery() {
    let (mut app, _, _) = midi_context();
    let original = app.inspect();
    prompt(&mut app, "Explain only this phrase.");
    app.command(json!({"command":"assistant_script","turns":[]}));
    app.click("Send");
    let failed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert!(!failed["assistant"]["failure"].is_null());
    assert_eq!(failed["assistant"]["recover_prompt"], true);

    app.key("Escape");
    let global = app.click("AI");
    assert_eq!(global["assistant"]["scope"]["kind"], "song");
    assert_eq!(global["assistant"]["failure"], Value::Null);
    assert_eq!(global["assistant"]["recover_prompt"], false);
    assert_eq!(global["assistant"]["messages"], json!([]));
    unchanged(&global, &original);

    prompt(&mut app, "Explain this song.");
    app.command(json!({"command":"assistant_script","turns":[]}));
    app.click("Send");
    let failed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert!(!failed["assistant"]["failure"].is_null());
    let replacement = menu(&mut app, "File", "New");
    assert_ne!(
        replacement["song_session"]["id"],
        original["song_session"]["id"]
    );
    assert_eq!(replacement["assistant"]["failure"], Value::Null);
    assert_eq!(replacement["assistant"]["recover_prompt"], false);
    assert_eq!(replacement["assistant"]["prompt"], "");
    assert_eq!(replacement["assistant"]["messages"], json!([]));
    let reopened = app.click("AI");
    assert!(!visible(&reopened, "Edit last prompt"));
    assert!(visible(&reopened, "Connect AI"));
    app.quit();
}

fn wait_recording(app: &mut Harness, phase: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = app.inspect();
        if state["recording"]["phase"] == phase {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Recording did not reach {phase}: {state}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn recording_explains_the_block_and_keeps_the_draft_ready_for_afterward() {
    let mut app = Harness::with_virtual_audio(None, None);
    let mut project = Project::new();
    project.add_audio_track("Capture");
    let path = app.directory.path().join("recording-readiness.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.click("Capture");
    menu(&mut app, "Window", "Recording");
    app.click("Arm recording");
    app.key("Escape");
    app.click("Record");
    wait_recording(&mut app, "recording");
    app.click("AI");
    let draft = "Explain the current song after recording finishes.";
    let blocked = prompt(&mut app, draft);
    assert_eq!(blocked["assistant"]["submit_state"], "Blocked");
    let reason = blocked["assistant"]["submit_reason"].as_str().unwrap();
    assert!(reason.to_lowercase().contains("recording"));
    assert!(visible(&blocked, reason));
    assert!(!visible(&blocked, "Connect AI"));
    let clicked = app.click("Send");
    assert_eq!(clicked["assistant"]["busy"], false);
    assert_eq!(clicked["assistant"]["messages"], json!([]));
    assert_eq!(clicked["assistant"]["prompt"], draft);
    unchanged(&clicked, &blocked);
    app.key("Escape");
    app.click("Stop");
    wait_recording(&mut app, "idle");
    let reopened = app.click("AI");
    assert_eq!(reopened["assistant"]["prompt"], draft);
    assert_eq!(reopened["assistant"]["submit_state"], "SetupRequired");
    assert!(visible(&reopened, "Connect AI"));
    app.quit();
}

#[test]
fn stopping_a_run_offers_editing_without_repeating_already_committed_work() {
    let mut app = Harness::new();
    let original = app.inspect();
    let mut calls = vec![part_call(&original)];
    calls.extend((0..32).map(|index| {
        json!({
            "id":format!("context-{index}"),"name":"get_song_context","arguments":{}
        })
    }));
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":calls},{"text":"Completed the request."}
    ]}));
    app.click("AI");
    let draft = "Create one idea and inspect it before suggesting a next step.";
    prompt(&mut app, draft);
    app.click("Send");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = app.inspect();
        assert_eq!(
            state["assistant"]["busy"], true,
            "Run finished before Stop: {state}"
        );
        if state["project"]["tracks"].as_array().unwrap().len() == 1 {
            assert!(footer_visible(&state, "Stop"));
            assert!(!footer_visible(&state, "Undo"));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "First edit did not arrive: {state}"
        );
    }
    let stopped = app.click("Stop");
    assert_eq!(stopped["assistant"]["busy"], false);
    assert_eq!(stopped["assistant"]["status"], "Stopped");
    assert_eq!(stopped["assistant"]["recover_prompt"], true);
    assert_eq!(stopped["assistant"]["failure"], Value::Null);
    assert!(visible(&stopped, "Edit last prompt"));
    assert!(!footer_visible(&stopped, "Stop"));
    assert!(footer_visible(&stopped, "Undo"));
    assert_eq!(stopped["history"]["undo"].as_array().unwrap().len(), 1);
    let restored = app.click("Edit last prompt");
    assert_eq!(restored["assistant"]["prompt"], draft);
    assert_eq!(restored["assistant"]["busy"], false);
    unchanged(&restored, &stopped);
    app.quit();
}
