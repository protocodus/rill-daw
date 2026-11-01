//! Window manipulation, discovery diagnostics and direct mixer device access.
mod common;
use common::Harness;
use daw_core::{Clip, Effect, EffectSlot, Instrument, Project};
use serde_json::{Value, json};

fn menu(app: &mut Harness, name: &str, item: &str) -> Value {
    app.click(name);
    app.click(item)
}
fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == label)
}
fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}
fn drag(app: &mut Harness, at: (f64, f64), delta: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":at.0+delta.0,"to_y":at.1+delta.1}))
}
fn front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| *value == label)
        .count();
    assert!(count > 0, "Missing {label}: {state}");
    app.command(json!({"command":"click","label":label,"occurrence":count-1}))
}
fn rename(app: &mut Harness, from: &str, to: &str) -> Value {
    front(app, from);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":to}));
    app.key("Enter")
}

fn editor_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("device_editor.rs");
    let metadata =
        include_str!("common/parameter_host.rs").replace("fn main() {", "fn metadata_main() {");
    std::fs::write(
        &source,
        format!(
            "{metadata}\n{}",
            include_str!("common/native_parameter_editor.rs")
        ),
    )
    .unwrap();
    let executable = directory.path().join(if cfg!(windows) {
        "device-editor.exe"
    } else {
        "device-editor"
    });
    let output =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (directory, executable)
}

#[test]
fn device_library_can_move_and_resize_without_changing_the_project() {
    let mut app = Harness::new();
    let before = menu(&mut app, "Track", "Add instrument…");
    let window = &before["library"]["window"];
    let title = before["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == "Device library")
        .unwrap();
    let moved = drag(&mut app, center(title), (70.0, 35.0));
    let after = &moved["library"]["window"];
    assert!(
        after["min"][0].as_f64().unwrap() - window["min"][0].as_f64().unwrap() > 50.0,
        "{window} -> {after}"
    );
    assert!(after["min"][1].as_f64().unwrap() - window["min"][1].as_f64().unwrap() > 25.0);
    let at = (
        after["max"][0].as_f64().unwrap() - 3.0,
        after["max"][1].as_f64().unwrap() - 3.0,
    );
    let resized = drag(&mut app, at, (-120.0, -90.0));
    let resized_window = &resized["library"]["window"];
    assert!(
        resized_window["max"][0].as_f64().unwrap() < after["max"][0].as_f64().unwrap() - 80.0,
        "{after} -> {resized_window}"
    );
    assert!(resized_window["max"][1].as_f64().unwrap() < after["max"][1].as_f64().unwrap() - 50.0);
    assert!(visible(&resized, "Search instruments"));
    assert_eq!(resized["project"], before["project"]);
    assert_eq!(resized["history"], before["history"]);
    app.key("Escape");
    let recalled = menu(&mut app, "Window", "Library");
    assert_eq!(recalled["library"]["window"], resized["library"]["window"]);
    app.quit();
}

#[test]
fn unknown_plugins_are_separate_and_explain_missing_or_malformed_metadata() {
    let fixture = common::plugin_fixture();
    std::fs::create_dir_all(fixture.path().join("Rill Mystery.vst3/Contents/Resources")).unwrap();
    let broken = fixture.path().join("Rill Broken.vst3/Contents/Resources");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("moduleinfo.json"), "{not valid JSON").unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Window", "Library");
    app.click("Search instruments");
    app.command(json!({"command":"text","text":"Rill"}));
    common::wait_library_device(&mut app, "Rill Fixture A");
    let instruments = app.inspect();
    assert!(!visible(&instruments, "Rill Fixture FX"));
    assert!(!visible(&instruments, "Rill Mystery"));
    assert!(!visible(&instruments, "Rill Broken"));
    app.click("Effects");
    app.click("Search effects");
    let effects = app.command(json!({"command":"text","text":"Rill"}));
    assert!(
        visible(&effects, "Rill Fixture FX"),
        "{} {}",
        effects["visible_labels"],
        effects["library"]
    );
    assert!(!visible(&effects, "Rill Fixture A"));
    app.click("Unverified");
    app.click("Search unverified devices");
    let unknown = app.command(json!({"command":"text","text":"Rill"}));
    assert!(visible(&unknown, "Rill Mystery"));
    assert!(visible(&unknown, "Rill Broken"));
    assert!(!visible(&unknown, "Rill Fixture A"));
    front(&mut app, "Rill Mystery");
    let details = app.inspect();
    let labels = details["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(labels.contains("No moduleinfo.json found"), "{labels}");
    assert!(labels.contains("Not declared"));
    assert!(labels.contains("Capture preview"));
    assert!(details["project"]["tracks"].as_array().unwrap().is_empty());
    app.click("Close");
    front(&mut app, "Rill Broken");
    let details = app.inspect();
    let labels = details["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(labels.contains("Invalid JSON"), "{labels}");
    app.click("Copy details");
    app.click("Close");
    assert_eq!(app.inspect()["modified"], false);
    app.quit();
}

#[test]
fn mixer_opens_each_insert_and_device_names_survive_undo_and_save() {
    let fixture = common::plugin_fixture();
    let bundle = fixture.path().join("RillFixture.vst3");
    let (_worker, host) = editor_fixture();
    let mut app = Harness::with_plugin_host(fixture.path(), &host);
    let mut project = Project::new();
    let track = project.add_midi_track("Keys");
    let channel = project.track_mut(track).unwrap();
    channel.instrument = Instrument::Vst3 {
        path: bundle.to_string_lossy().into_owned(),
        class_id: Some("11111111111111111111111111111111".into()),
    };
    channel.instrument_name = "Soft keys".into();
    channel.instrument_state = vec![40];
    channel
        .clips
        .push(Clip::midi("Pattern", 0.0, 4.0, Vec::new()));
    channel.inserts.push(EffectSlot::new(
        "Space",
        Effect::Vst3 {
            path: bundle.to_string_lossy().into_owned(),
            class_id: Some("33333333333333333333333333333333".into()),
        },
    ));
    channel.inserts.push(EffectSlot::new(
        "Echo",
        Effect::Vst3 {
            path: bundle.to_string_lossy().into_owned(),
            class_id: Some("33333333333333333333333333333333".into()),
        },
    ));
    for slot in &mut channel.inserts {
        slot.state = vec![40];
    }
    let path = app.directory.path().join("device-names.daw.json");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let mixer = menu(&mut app, "Window", "Mixer");
    assert!(visible(&mixer, "Soft keys"));
    assert!(visible(&mixer, "Space"));
    assert!(visible(&mixer, "Echo"));
    assert_eq!(mixer["mixer_devices"].as_array().unwrap().len(), 3);
    app.click("Space");
    assert!(visible(&app.inspect(), "Space of Keys"));
    let renamed = rename(&mut app, "Space", "Long reverb");
    assert_eq!(
        renamed["project"]["tracks"][0]["inserts"][0]["name"],
        "Long reverb"
    );
    app.key("Escape");
    let mixer = app.inspect();
    assert!(visible(&mixer, "Long reverb"));
    app.click("Soft keys");
    common::wait_visible(&mut app, "Soft keys");
    let named = rename(&mut app, "Soft keys", "Warm piano");
    assert_eq!(
        named["project"]["tracks"][0]["instrument_name"],
        "Warm piano"
    );
    app.key("Escape");
    let undone = menu(&mut app, "Edit", "Undo");
    assert_eq!(
        undone["project"]["tracks"][0]["instrument_name"],
        "Soft keys"
    );
    menu(&mut app, "Edit", "Redo");
    app.click("Echo");
    assert!(visible(&app.inspect(), "Echo of Keys"));
    app.key("Escape");
    let archive = app.directory.path().join("named.rill");
    app.command(json!({"command":"save","path":archive}));
    let recalled = app.command(json!({"command":"open","path":archive}));
    assert_eq!(
        recalled["project"]["tracks"][0]["instrument_name"],
        "Warm piano"
    );
    assert_eq!(
        recalled["project"]["tracks"][0]["inserts"][0]["name"],
        "Long reverb"
    );
    menu(&mut app, "Window", "Mixer");
    app.click("Warm piano");
    front(&mut app, "Keyboard…");
    app.click("Clear");
    for letter in ["p", "a", "d"] {
        front(&mut app, letter);
    }
    let mouse_named = app.inspect();
    assert_eq!(
        mouse_named["project"]["tracks"][0]["instrument_name"],
        "pad"
    );
    app.quit();
}
