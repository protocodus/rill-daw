//! AI batches and mouse edits share one bounded, reversible application history.
mod common;

use common::Harness;
use serde_json::{Value, json};

fn limited_app() -> Harness {
    let mut app = Harness::new();
    let mut settings = app.inspect()["settings"].clone();
    assert!(settings["history_limit"].as_u64().unwrap() >= 50);
    settings["history_limit"] = json!(3);
    std::fs::write(
        app.directory.path().join("config/settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
    app = app.restart();
    assert_eq!(app.inspect()["settings"]["history_limit"], 3);
    app
}

fn add_track(app: &mut Harness) -> Value {
    app.click("Track");
    app.click("Add audio track")
}

fn history_action(app: &mut Harness, action: &str) -> Value {
    app.click("Edit");
    app.click(action)
}

fn enter_prompt(app: &mut Harness, state: &Value, text: &str) {
    let rect = &state["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(json!({"command":"text","text":text}));
}

fn ai_batch(app: &mut Harness, arguments: Value) -> Value {
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":[{"id":"edit","name":"apply_edits","arguments":arguments}]},
        {"text":"Finished the requested batch."}
    ]}));
    let state = app.inspect();
    enter_prompt(app, &state, "Apply this requested song edit.");
    app.click("Send");
    let state = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(state["assistant"]["busy"], false);
    state
}

fn batch(state: &Value, label: &str, name: &str, tempo: f64) -> Value {
    json!({
        "session_id":state["song_session"]["id"],
        "expected_revision":state["song_session"]["revision"],
        "label":label,
        "edits":[{"op":"rename_song","name":name},{"op":"set_tempo","bpm":tempo}]
    })
}

fn open_assistant(app: &mut Harness) {
    app.click("Window");
    app.click("Assistant");
}

#[test]
fn successful_ai_batches_undo_and_redo_through_the_shared_action_limit() {
    let mut app = limited_app();
    let manual = add_track(&mut app);
    open_assistant(&mut app);
    let mut states = vec![manual];
    for index in 1..=4 {
        let label = format!("AI batch {index}");
        let name = format!("AI version {index}");
        let args = batch(states.last().unwrap(), &label, &name, 90.0 + index as f64);
        let state = ai_batch(&mut app, args);
        assert_eq!(state["assistant"]["last_tool_error"], Value::Null);
        assert_eq!(state["project"]["name"], name);
        assert_eq!(state["project"]["tempo"], 90.0 + index as f64);
        assert_eq!(state["project"]["tracks"], states[0]["project"]["tracks"]);
        assert_eq!(
            state["history"]["undo"].as_array().unwrap().len(),
            (index + 1).min(3)
        );
        states.push(state);
    }
    assert_eq!(
        states[4]["history"]["undo"],
        json!(["AI batch 4", "AI batch 3", "AI batch 2"])
    );

    // Each two-operation AI batch is one ordinary Undo entry. The manual edit
    // and oldest AI batch have fallen outside the same three-action budget.
    for index in (1..=3).rev() {
        let undone = history_action(&mut app, "Undo");
        assert_eq!(undone["project"], states[index]["project"]);
        assert_eq!(
            undone["history"]["undo"].as_array().unwrap().len(),
            index - 1
        );
        assert_eq!(
            undone["history"]["redo"].as_array().unwrap().len(),
            4 - index
        );
    }
    let exhausted = app.inspect();
    assert!(exhausted["history"]["undo"].as_array().unwrap().is_empty());
    app.key("Escape");
    let unchanged = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(unchanged["project"], states[1]["project"]);

    for (index, expected) in states.iter().enumerate().take(5).skip(2) {
        let redone = history_action(&mut app, "Redo");
        assert_eq!(redone["project"], expected["project"]);
        assert_eq!(
            redone["history"]["undo"].as_array().unwrap().len(),
            index - 1
        );
        assert_eq!(
            redone["history"]["redo"].as_array().unwrap().len(),
            4 - index
        );
    }
    let mixed = add_track(&mut app);
    assert_eq!(
        mixed["history"]["undo"],
        json!(["Add track", "AI batch 4", "AI batch 3"])
    );
    assert_eq!(
        history_action(&mut app, "Undo")["project"],
        states[4]["project"]
    );
    app.quit();
}

#[test]
fn failed_rejected_and_unchanged_ai_batches_preserve_existing_undo_and_redo() {
    let mut app = limited_app();
    let manual = add_track(&mut app);
    open_assistant(&mut app);
    let first = ai_batch(&mut app, batch(&manual, "AI first", "First version", 97.0));
    let second = ai_batch(
        &mut app,
        batch(&first, "AI second", "Second version", 103.0),
    );
    let before = history_action(&mut app, "Undo");
    assert_eq!(before["project"], first["project"]);
    assert_eq!(before["history"]["redo"], json!(["AI second"]));

    // The rename would succeed alone. The invalid second operation rejects the
    // complete candidate before history commit, including its pending Redo.
    let failed = ai_batch(
        &mut app,
        batch(&before, "Invalid AI batch", "Must not appear", 1000.0),
    );
    assert_eq!(
        failed["assistant"]["last_tool_error"]["code"],
        "edit_failed"
    );
    for field in ["project", "history", "song_session", "modified"] {
        assert_eq!(failed[field], before[field], "Failed batch changed {field}");
    }

    let mut stale = batch(&failed, "Stale AI batch", "Must not appear either", 105.0);
    stale["expected_revision"] = manual["song_session"]["revision"].clone();
    let rejected = ai_batch(&mut app, stale);
    assert_eq!(rejected["assistant"]["last_tool_error"]["code"], "conflict");
    for field in ["project", "history", "song_session", "modified"] {
        assert_eq!(
            rejected[field], before[field],
            "Rejected batch changed {field}"
        );
    }

    let unchanged = ai_batch(
        &mut app,
        batch(&rejected, "No change", "First version", 97.0),
    );
    assert_eq!(unchanged["assistant"]["last_tool_error"], Value::Null);
    for field in ["project", "history", "song_session", "modified"] {
        assert_eq!(
            unchanged[field], before[field],
            "Unchanged batch changed {field}"
        );
    }
    assert_eq!(
        history_action(&mut app, "Redo")["project"],
        second["project"]
    );
    assert_eq!(
        history_action(&mut app, "Undo")["project"],
        first["project"]
    );
    app.quit();
}
