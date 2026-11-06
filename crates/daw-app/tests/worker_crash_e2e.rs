//! Real worker panics must release the running app and keep completed work.
mod common;

use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn enter_prompt(app: &mut Harness, state: &Value, text: &str) {
    let rect = &state["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"text","text":text}));
}

fn manual_work(app: &mut Harness) -> Value {
    app.click("Track");
    app.click("Add audio track")
}

#[test]
fn provider_panic_keeps_completed_edits_checkpoint_and_recoverable_prompt() {
    let mut app = Harness::new();
    let before = manual_work(&mut app);
    app.command(json!({"command":"assistant_script","crash_after":true,"turns":[
        {"calls":[{"id":"rename","name":"apply_edits","arguments":{
            "session_id":before["song_session"]["id"],
            "expected_revision":before["song_session"]["revision"],
            "label":"Assistant rename","edits":[{"op":"rename_song","name":"Kept after AI crash"}]
        }}]}
    ]}));
    app.click("Window");
    let opened = app.click("Assistant");
    let prompt = "Rename this song and keep my audio track.";
    enter_prompt(&mut app, &opened, prompt);
    app.click("Send");
    let failed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(failed["assistant"]["busy"], false);
    assert_eq!(failed["assistant"]["recover_prompt"], true);
    assert!(
        failed["assistant"]["failure"]
            .as_str()
            .unwrap()
            .contains("AI worker stopped")
    );
    assert_eq!(failed["project"]["name"], "Kept after AI crash");
    assert_eq!(failed["project"]["tracks"], before["project"]["tracks"]);
    assert_eq!(failed["history"]["undo"].as_array().unwrap().len(), 2);

    app.command(json!({"command":"wait_project"}));
    let checkpoint = Project::load(app.directory.path().join("scratch/recovery.daw.json")).unwrap();
    assert_eq!(checkpoint.name, "Kept after AI crash");
    assert_eq!(
        serde_json::to_value(checkpoint.tracks).unwrap(),
        failed["project"]["tracks"]
    );
    let recovered = app.click("Edit last prompt");
    assert_eq!(recovered["assistant"]["prompt"], prompt);
    assert_eq!(recovered["project"], failed["project"]);

    app.command(
        json!({"command":"assistant_script","turns":[{"text":"Your completed rename is intact."}]}),
    );
    app.click("Send");
    let resumed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(resumed["assistant"]["status"], "Done");
    assert_eq!(resumed["project"], failed["project"]);
    let undone = app.click("Undo");
    assert_eq!(undone["project"], before["project"]);
    app.quit();
}

#[test]
fn tool_worker_panic_preserves_history_and_releases_the_worker_slot() {
    let mut app = Harness::new();
    let before = manual_work(&mut app);
    app.command(
        json!({"command":"assistant_script","crash_tool":true,"turns":[
            {"calls":[{"id":"analysis","name":"audio_analyze","arguments":{"end_beat":1.0}}]},
            {"text":"The analysis failed; your existing work is intact."}
        ]}),
    );
    app.click("Window");
    let opened = app.click("Assistant");
    enter_prompt(&mut app, &opened, "Analyze the song without changing it.");
    app.click("Send");
    let failed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(failed["assistant"]["busy"], false);
    assert_eq!(
        failed["assistant"]["last_tool_error"]["code"],
        "tool_crashed"
    );
    assert_eq!(failed["project"], before["project"]);
    assert_eq!(failed["history"], before["history"]);
    let retried = app.command(
        json!({"command":"song_tool","name":"audio_analyze","arguments":{"end_beat":1.0}}),
    );
    assert_eq!(retried["tool_result"]["ok"], true, "{retried}");
    assert_eq!(retried["project"], before["project"]);
    app.command(json!({"command":"wait_project"}));
    let checkpoint = Project::load(app.directory.path().join("scratch/recovery.daw.json")).unwrap();
    assert_eq!(serde_json::to_value(checkpoint).unwrap(), before["project"]);
    app.quit();
}
