//! User document actions preserve unsaved work until an explicit choice.
//! The driver supplies native picker results; menus, keys, modal choices and
//! close requests still travel through the running application's UI.
mod common;
use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn menu(app: &mut Harness, name: &str, item: &str) -> Value {
    app.click(name);
    app.click(item)
}
fn has(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}
fn add(app: &mut Harness) -> Value {
    menu(app, "Track", "Add audio track")
}
fn picker(app: &mut Harness, kind: &str, path: Option<&Path>) {
    app.command(json!({"command":"file_dialog", "kind":kind, "path":path}));
}
fn wait_for(app: &mut Harness, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = app.inspect();
        if ready(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Document action did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn confirm(app: &mut Harness) -> Value {
    wait_for(app, |state| state["document_guard"]["phase"] == "confirm")
}
fn done(app: &mut Harness) -> Value {
    wait_for(app, |state| {
        state["document_guard"].is_null() && state["project_pending"] == false
    })
}
fn cmd(app: &mut Harness, key: &str) -> Value {
    app.command(json!({"command":"key", "key":key, "cmd":true}))
}

#[test]
fn new_confirmation_is_bounded_blocks_background_edits_and_cancel_restores_context() {
    for dark in [false, true] {
        for scale in [1.0, 2.0] {
            let mut app = Harness::new();
            app.command(json!({"command":"system_theme", "dark":dark}));
            app.command(
                json!({"command":"viewport", "width":1100, "height":720, "pixels_per_point":scale}),
            );
            let initial = app.inspect();
            let title = initial["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == "Untitled session")
                .unwrap();
            assert!(
                title["max"][0].as_f64().unwrap() - title["min"][0].as_f64().unwrap() > 100.0,
                "Project title must be readable, not clipped to the viewport edge: {title}"
            );
            let mut project = Project::new();
            project.name =
                "夜のシンセ 🎹 Long project name with a meaningful description ".repeat(12);
            let seed = app.directory.path().join("long-name.json");
            project.save(&seed).unwrap();
            let loaded = app.command(json!({"command":"open", "path":seed}));
            let title = loaded["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| {
                    rect["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("夜のシンセ"))
                })
                .unwrap();
            let last_menu = loaded["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|rect| rect["text"] == "Window")
                .unwrap();
            assert!(
                title["min"][0].as_f64().unwrap() >= last_menu["max"][0].as_f64().unwrap() + 8.0,
                "Title covers the menus: {title}"
            );
            assert!(
                title["max"][0].as_f64().unwrap() - title["min"][0].as_f64().unwrap() > 100.0,
                "Long title must remain readable: {title}"
            );
            add(&mut app);
            let before = menu(&mut app, "Window", "Properties");
            menu(&mut app, "File", "New");
            let prompt = confirm(&mut app);
            assert_eq!(prompt["document_guard"]["intent"], "new");
            assert_eq!(prompt["project"], before["project"]);
            for label in ["Save changes?", "Save", "Don't save", "Cancel"] {
                let rect = prompt["label_rects"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|rect| rect["text"] == label)
                    .unwrap_or_else(|| panic!("Missing {label}: {}", prompt["visible_labels"]));
                assert!(rect["min"][0].as_f64().unwrap() >= 24.0, "{label}: {rect}");
                assert!(
                    rect["max"][0].as_f64().unwrap() <= 1076.0,
                    "{label}: {rect}"
                );
                assert!(rect["min"][1].as_f64().unwrap() >= 24.0, "{label}: {rect}");
                assert!(rect["max"][1].as_f64().unwrap() <= 696.0, "{label}: {rect}");
            }
            // Commands and backdrop clicks cannot undo edits or dismiss the choice.
            assert_eq!(cmd(&mut app, "Z")["project"], before["project"]);
            let outside =
                app.command(json!({"command":"pointer", "phase":"click", "x":10, "y":500}));
            assert_eq!(outside["document_guard"]["phase"], "confirm");
            assert_eq!(outside["project"], before["project"]);
            let cancelled = app.key("Escape");
            assert!(cancelled["document_guard"].is_null());
            assert_eq!(cancelled["project"], before["project"]);
            assert_eq!(cancelled["selected_track"], before["selected_track"]);
            assert_eq!(cancelled["popups"], before["popups"]);
            assert_eq!(cancelled["history"], before["history"]);

            menu(&mut app, "File", "New");
            confirm(&mut app);
            app.click("Don't save");
            let cleared = done(&mut app);
            assert!(cleared["project"]["tracks"].as_array().unwrap().is_empty());
            assert_eq!(cleared["modified"], false);
            assert!(cleared["history"]["undo"].as_array().unwrap().is_empty());
            assert_eq!(
                Project::load(&seed).unwrap(),
                project,
                "Don't save must not overwrite the original file"
            );
            app.quit();
        }
    }
}

#[test]
fn save_before_new_waits_for_the_real_archive_and_picker_cancel_retains_the_choice() {
    let mut app = Harness::new();
    let before = add(&mut app);
    menu(&mut app, "File", "New");
    confirm(&mut app);
    picker(&mut app, "save", None);
    let cancelled_picker = app.click("Save");
    assert_eq!(cancelled_picker["file_dialog_pending"], false);
    assert_eq!(cancelled_picker["project"], before["project"]);
    assert_eq!(cancelled_picker["document_guard"]["phase"], "confirm");
    let path = app.directory.path().join("Saved before new 🎹.rill");
    picker(&mut app, "save", Some(&path));
    app.click("Save");
    let cleared = done(&mut app);
    assert!(cleared["project"]["tracks"].as_array().unwrap().is_empty());
    let saved = Project::load(&path).unwrap();
    assert_eq!(json!(saved), before["project"]);
    assert!(std::fs::read(&path).unwrap().starts_with(b"PK"));
    app.quit();
}

#[test]
fn failed_save_keeps_the_document_and_can_be_cancelled_or_retried() {
    let mut app = Harness::new();
    let before = add(&mut app);
    let invalid = app.directory.path().join("directory.rill");
    std::fs::create_dir(&invalid).unwrap();
    menu(&mut app, "File", "New");
    confirm(&mut app);
    picker(&mut app, "save", Some(&invalid));
    app.click("Save");
    let failed = wait_for(&mut app, |state| {
        !state["document_guard"]["error"].is_null()
    });
    assert_eq!(failed["project"], before["project"]);
    assert_eq!(failed["modified"], true);
    assert!(invalid.is_dir());
    assert!(has(&failed, "Cancel"));
    let cancelled = app.click("Cancel");
    assert!(cancelled["document_guard"].is_null());
    assert_eq!(cancelled["project"], before["project"]);

    menu(&mut app, "File", "New");
    confirm(&mut app);
    let valid = app.directory.path().join("Retry.rill");
    picker(&mut app, "save", Some(&valid));
    app.click("Save");
    let finished = done(&mut app);
    assert!(finished["project"]["tracks"].as_array().unwrap().is_empty());
    assert_eq!(json!(Project::load(valid).unwrap()), before["project"]);
    app.quit();
}

#[test]
fn save_and_open_shortcuts_work_from_properties_without_losing_cancelled_context() {
    let mut app = Harness::new();
    add(&mut app);
    menu(&mut app, "Window", "Properties");
    let path = app.directory.path().join("Current.rill");
    picker(&mut app, "save", Some(&path));
    cmd(&mut app, "S");
    let saved = done(&mut app);
    assert_eq!(saved["modified"], false);
    assert_eq!(saved["project_path"], json!(path));
    assert_eq!(saved["popups"]["properties"], true);
    app.key("Escape");
    add(&mut app);
    let before = menu(&mut app, "Window", "Properties");

    let mut incoming = Project::new();
    incoming.name = "Another project".into();
    incoming.add_audio_track("Incoming audio");
    let incoming_path = app.directory.path().join("Incoming.rill");
    incoming.save(&incoming_path).unwrap();
    picker(&mut app, "open", Some(&incoming_path));
    cmd(&mut app, "O");
    assert_eq!(confirm(&mut app)["document_guard"]["intent"], "open");
    let cancelled = app.click("Cancel");
    assert_eq!(cancelled["project"], before["project"]);
    assert_eq!(cancelled["popups"], before["popups"]);
    picker(&mut app, "open", None);
    cmd(&mut app, "O");
    assert!(app.inspect()["document_guard"].is_null());
    picker(&mut app, "open", Some(&incoming_path));
    cmd(&mut app, "O");
    confirm(&mut app);
    app.click("Save");
    let opened = done(&mut app);
    assert_eq!(opened["project"], json!(incoming));
    assert_eq!(json!(Project::load(path).unwrap()), before["project"]);
    assert_eq!(opened["project_path"], json!(incoming_path));
    app.quit();
}

#[test]
fn completed_and_cancelled_pickers_preserve_text_shortcuts_and_unicode_names() {
    let mut app = Harness::new();
    add(&mut app);
    let path = app.directory.path().join("Neon 🎹 Circuit.rill");
    picker(&mut app, "save", Some(&path));
    menu(&mut app, "File", "Save as…");
    done(&mut app);
    let mut name = "New audio".to_owned();

    for (kind, item, replacement) in [
        ("open", "Open…", "夜のドラム 🎹"),
        ("save", "Save as…", "Neon bass 🎸"),
    ] {
        let before = app.inspect();
        picker(&mut app, kind, None);
        let cancelled = menu(&mut app, "File", item);
        assert_eq!(cancelled["project"], before["project"]);
        assert_eq!(cancelled["project_path"], before["project_path"]);
        assert_eq!(cancelled["file_dialog_pending"], false);
        let state = menu(&mut app, "Window", "Properties");
        let count = state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|label| label.as_str() == Some(&name))
            .count();
        assert!(count > 0);
        app.command(json!({"command":"click", "label":name, "occurrence":count - 1}));
        app.command(json!({"command":"key", "key":"A", "cmd":true, "release_modifiers":true}));
        let renamed = app.command(json!({"command":"text", "text":replacement}));
        assert_eq!(renamed["project"]["tracks"][0]["name"], replacement);
        app.key("Escape");
        cmd(&mut app, "S");
        done(&mut app);
        assert_eq!(Project::load(&path).unwrap().tracks[0].name, replacement);
        name = replacement.to_owned();
    }
    app.quit();
}

#[test]
fn native_close_and_menu_quit_wait_for_a_choice_and_save_before_exit() {
    for save in [false, true] {
        let mut app = Harness::new();
        add(&mut app);
        let path = app.directory.path().join("Before quit.rill");
        app.command(json!({"command":"save", "path":path}));
        let initial = Project::load(&path).unwrap();
        let before = add(&mut app);
        app.command(json!({"command":"close_window"}));
        assert_eq!(confirm(&mut app)["document_guard"]["intent"], "quit");
        let cancelled = app.click("Cancel");
        assert!(cancelled["document_guard"].is_null());
        assert_eq!(cancelled["project"], before["project"]);
        if save {
            app.command(json!({"command":"close_window"}));
        } else {
            menu(&mut app, "File", "Quit");
        }
        confirm(&mut app);
        let mut response = app
            .raw_command(json!({"command":"click", "label":if save {"Save"} else {"Don't save"}}));
        let deadline = Instant::now() + Duration::from_secs(20);
        while response["event"] != "exiting" {
            assert_eq!(response["ok"], true, "{response}");
            assert!(Instant::now() < deadline, "Quit did not finish: {response}");
            std::thread::sleep(Duration::from_millis(1));
            response = app.raw_command(json!({"command":"inspect"}));
        }
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(
            json!(Project::load(&path).unwrap()),
            if save {
                before["project"].clone()
            } else {
                json!(initial)
            }
        );
        app.wait_exit();
    }
}
