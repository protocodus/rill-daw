//! Custom history budgets use ordinary Setup Apply/Cancel and document history.
mod common;
use common::Harness;
use serde_json::{Value, json};

fn setup_storage(app: &mut Harness) -> Value {
    app.click("Window");
    app.click("Setup");
    app.click("Storage")
}

fn enter_limit(app: &mut Harness, current: usize, value: usize) -> Value {
    let state = app.inspect();
    let caption = current.to_string();
    let field = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"].as_str() == Some(caption.as_str()))
        .unwrap_or_else(|| panic!("Missing history field: {}", state["visible_labels"]));
    let x = (field["min"][0].as_f64().unwrap() + field["max"][0].as_f64().unwrap()) * 0.5;
    let y = (field["min"][1].as_f64().unwrap() + field["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"double_click","x":x,"y":y}));
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":value.to_string()}));
    app.key("Enter")
}

fn add_track(app: &mut Harness) -> Value {
    app.click("Track");
    app.click("Add audio track")
}

fn history_action(app: &mut Harness, label: &str) -> Value {
    app.click("Edit");
    app.click(label)
}

#[test]
fn compact_history_settings_keep_field_and_apply_cancel_reachable_in_both_themes() {
    let mut app = Harness::new();
    for (density, dark) in [(1.0, false), (2.0, true)] {
        app.command(
            json!({"command":"viewport","width":800,"height":600,"pixels_per_point":density}),
        );
        app.command(json!({"command":"system_theme","dark":dark}));
        let state = setup_storage(&mut app);
        for label in ["Undo actions", "128", "Apply", "Cancel"] {
            let rect = state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["text"] == label)
                .unwrap_or_else(|| {
                    panic!("Missing {label} at {density}x: {}", state["visible_labels"])
                });
            assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{label}: {rect}");
            assert!(rect["max"][0].as_f64().unwrap() <= 800.0, "{label}: {rect}");
            assert!(rect["min"][1].as_f64().unwrap() >= 100.0, "{label}: {rect}");
            assert!(rect["max"][1].as_f64().unwrap() <= 600.0, "{label}: {rect}");
        }
        assert_eq!(
            enter_limit(&mut app, 128, 77)["setup_draft"]["history_limit"],
            77
        );
        assert_eq!(app.click("Cancel")["settings"]["history_limit"], 128);
    }
    app.quit();
}

#[test]
fn history_limit_apply_cancel_persistence_and_saved_document_identity() {
    let mut app = Harness::new();
    assert_eq!(app.inspect()["settings"]["history_limit"], 128);
    for _ in 0..5 {
        add_track(&mut app);
    }
    let path = app.directory.path().join("saved-history.rill");
    let saved = app.command(json!({"command":"save","path":path}));
    assert_eq!(saved["history"]["undo"].as_array().unwrap().len(), 5);
    let opened = setup_storage(&mut app);
    assert!(
        opened["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "128 MiB memory cap may retain fewer actions")
    );
    let draft = enter_limit(&mut app, 128, 2);
    assert_eq!(draft["setup_draft"]["history_limit"], 2);
    assert_eq!(draft["settings"]["history_limit"], 128);
    assert_eq!(draft["history"], saved["history"]);
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["history"], saved["history"]);
    assert_eq!(cancelled["settings"]["history_limit"], 128);

    setup_storage(&mut app);
    enter_limit(&mut app, 128, 2);
    let applied = app.click("Apply");
    assert_eq!(applied["settings"]["history_limit"], 2);
    assert_eq!(applied["history"]["undo"].as_array().unwrap().len(), 2);
    assert_eq!(applied["project"], saved["project"]);
    assert_eq!(applied["song_session"], saved["song_session"]);
    assert_eq!(applied["modified"], false);
    assert_eq!(applied["history"]["saved"], true);
    for count in [4, 3] {
        assert_eq!(
            history_action(&mut app, "Undo")["project"]["tracks"]
                .as_array()
                .unwrap()
                .len(),
            count
        );
    }
    assert!(
        app.inspect()["history"]["undo"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    history_action(&mut app, "Redo");
    let restored = history_action(&mut app, "Redo");
    assert_eq!(restored["project"], saved["project"]);
    assert_eq!(restored["modified"], false);
    assert_eq!(restored["history"]["saved"], true);

    setup_storage(&mut app);
    enter_limit(&mut app, 2, 4);
    let grown = app.click("Apply");
    assert_eq!(grown["history"]["undo"].as_array().unwrap().len(), 2);
    assert_eq!(grown["modified"], false);
    let mut app = app.restart();
    assert_eq!(app.inspect()["settings"]["history_limit"], 4);
    for _ in 0..6 {
        add_track(&mut app);
    }
    assert_eq!(
        app.inspect()["history"]["undo"].as_array().unwrap().len(),
        4
    );
    app.quit();
}
