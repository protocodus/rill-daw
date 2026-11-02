//! Category discovery and filtering through the running VST browser.
mod common;
use common::Harness;
use serde_json::{Value, json};

fn visible(state: &Value, label: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|text| text == label)
}

#[test]
fn searchable_categories_filter_instruments_and_effects_without_instantiating_them() {
    let fixture = common::plugin_fixture();
    let metadata = fixture
        .path()
        .join("RillFixture.vst3/Contents/Resources/moduleinfo.json");
    let mut info: Value = serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
    info["Classes"][0]["Sub Categories"] = json!(["Instrument", "Synth", "Atmospheric"]);
    info["Classes"][1]["Sub Categories"] = json!(["Instrument", "Piano", "Acoustic"]);
    info["Classes"][2]["Sub Categories"] = json!(["Fx", "Delay"]);
    std::fs::write(metadata, serde_json::to_vec(&info).unwrap()).unwrap();
    let mut app = Harness::with_vst_path(fixture.path());
    common::fixture_library(&mut app);
    app.click("All categories");
    app.click("Search categories");
    let filtered = app.command(json!({"command":"text","text":"sYn"}));
    assert!(visible(&filtered, "Synth"));
    assert!(!visible(&filtered, "Piano"));
    let enter = app.key("Enter");
    assert!(enter["project"]["tracks"].as_array().unwrap().is_empty());
    assert_eq!(enter["popups"]["library"], true);
    let selected = app.click("Synth");
    assert_eq!(selected["library"]["category"], "Synth");
    assert_eq!(selected["library"]["total"], 1);
    assert!(visible(&selected, "Rill Fixture A"));
    assert!(!visible(&selected, "Rill Fixture B"));
    app.click("Clear filters");
    app.click("All categories");
    app.click("Search categories");
    let no_matches = app.command(json!({"command":"text","text":"no-such-category"}));
    assert!(visible(&no_matches, "No matches"));
    app.click("Clear");
    let cleared = app.inspect();
    assert!(visible(&cleared, "Piano"));
    app.click("Piano");
    assert_eq!(app.inspect()["library"]["total"], 1);
    app.click("Effects");
    common::wait_visible(&mut app, "Rill Fixture FX");
    app.click("All categories");
    app.click("Search categories");
    let effects = app.command(json!({"command":"text","text":"del"}));
    assert!(visible(&effects, "Delay"));
    assert!(!visible(&effects, "Synth"));
    let selected = app.click("Delay");
    assert_eq!(selected["library"]["category"], "Delay");
    assert_eq!(selected["library"]["total"], 1);
    assert!(selected["project"]["tracks"].as_array().unwrap().is_empty());
    app.quit();
}
