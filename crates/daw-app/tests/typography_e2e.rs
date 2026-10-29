//! The production UI keeps Unicode names and readable text at desktop densities.
mod common;
use common::Harness;
use serde_json::{Value, json};

fn menu(app: &mut Harness, menu: &str, item: &str) -> Value {
    app.click(menu);
    app.click(item)
}
fn front(app: &mut Harness, label: &str) -> Value {
    let state = app.inspect();
    let count = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|text| *text == label)
        .count();
    assert!(count > 0, "Missing {label}: {state}");
    app.command(json!({"command":"click","label":label,"occurrence":count-1}))
}

#[test]
fn unicode_emoji_names_edit_display_and_recall_at_both_densities_and_appearances() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":1100,"height":760}));
    menu(&mut app, "Track", "Add audio track");
    menu(&mut app, "Window", "Properties");
    front(&mut app, "New audio");
    app.command(json!({"command":"key","key":"A","cmd":true}));
    let name = "Žluťoučký 🎹 東京";
    app.command(json!({"command":"text","text":name}));
    app.key("Escape");
    let path = app.directory.path().join("Unicode 🎹.rill");
    app.command(json!({"command":"save","path":path}));
    for density in [1.0, 2.0] {
        app.command(
            json!({"command":"viewport","width":1100,"height":760,"pixels_per_point":density}),
        );
        for dark in [true, false] {
            let state = app.command(json!({"command":"system_theme","dark":dark}));
            assert_eq!(state["project"]["tracks"][0]["name"], name);
            assert_eq!(state["text_sizes"]["body"], 15.0);
            assert!(state["text_sizes"]["note"].as_f64().unwrap() >= 14.0);
            let row = state["label_rects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["text"] == name)
                .unwrap();
            assert!(row["max"][1].as_f64().unwrap() - row["min"][1].as_f64().unwrap() >= 14.0);
            menu(&mut app, "Window", "Properties");
            let props = app.click("Channel");
            assert!(
                props["visible_labels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|label| label == "Gain")
            );
            app.key("Escape");
        }
    }
    app = app.restart();
    let loaded = app.command(json!({"command":"open","path":path}));
    assert_eq!(loaded["project"]["tracks"][0]["name"], name);
    app.quit();
}

#[test]
fn long_device_name_wraps_on_card_and_actions_stay_accessible_in_small_window() {
    let fixture = common::plugin_fixture();
    let path = fixture
        .path()
        .join("RillFixture.vst3/Contents/Resources/moduleinfo.json");
    let mut metadata: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let name = "Žluťoučký 🎹 東京 — Spacious instrument with an unusually long descriptive name";
    metadata["Classes"][0]["Name"] = json!(name);
    std::fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    app.command(json!({"command":"viewport","width":1100,"height":760}));
    menu(&mut app, "Window", "Library");
    app.click("Search instruments");
    app.command(json!({"command":"text","text":"Žluťoučký"}));
    common::wait_library_device(&mut app, name);
    let state = app.inspect();
    let name_rows: Vec<_> = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["text"] == name)
        .collect();
    assert_eq!(
        name_rows.len(),
        3,
        "card, detail pane and selection footer must retain full source name"
    );
    let details = &state["library"]["details"];
    assert!(
        name_rows.iter().any(|row| {
            row["min"][0].as_f64() >= details["min"][0].as_f64()
                && row["max"][0].as_f64().unwrap() <= details["max"][0].as_f64().unwrap() + 1.0
        }),
        "the detail title wraps within its pane: {details}"
    );
    assert!(
        name_rows
            .iter()
            .any(|row| row["max"][1].as_f64().unwrap() - row["min"][1].as_f64().unwrap() > 30.0),
        "card name should wrap"
    );
    let action = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["text"] == "Add instrument track")
        .unwrap();
    assert!(
        action["max"][0].as_f64().unwrap() < 1100.0 && action["max"][1].as_f64().unwrap() < 760.0
    );
    let inserted = app.click("Add instrument track");
    assert_eq!(inserted["project"]["tracks"][0]["name"], name);
    assert_eq!(inserted["project"]["tracks"][0]["kind"], "Midi");
    app.quit();
}
