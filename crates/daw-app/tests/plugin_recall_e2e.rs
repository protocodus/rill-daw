//! Production app flows for native effect editing, automatic recall and automation.
mod common;
use common::Harness;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn menu(app: &mut Harness, name: &str, item: &str) -> Value {
    app.click(name);
    app.click(item)
}

fn wait(app: &mut Harness, ready: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let state = app.inspect();
        if ready(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Plugin transition timed out: {state}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn insert(app: &mut Harness, name: &str) -> Value {
    menu(app, "Window", "Effects");
    app.click("+ Insert");
    common::wait_visible(app, "Search effects");
    app.click("Search effects");
    app.command(json!({"command":"text","text":name}));
    common::wait_visible(app, "Add to effect chain");
    app.command(json!({"command":"click","label":name,"occurrence":1}));
    let state = app.click("Add to effect chain");
    assert!(
        !state["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    state
}

#[test]
fn effect_editor_failure_preserves_insert_and_returns_to_its_chain() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    menu(&mut app, "Track", "Add audio track");
    let inserted = insert(&mut app, "Rill Fixture FX");
    let before = inserted["project"].clone();
    let slot = before["tracks"][0]["inserts"][0]["id"].clone();
    let state = app.click("Open editor");
    assert_eq!(state["popups"]["instrument"], true);
    let state = wait(&mut app, |state| !state["native_editor"]["error"].is_null());
    assert_eq!(state["native_editor"]["effect"], slot);
    assert_eq!(state["project"], before);
    assert!(
        !state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "Change instrument" || label == "Save state")
    );
    let state = app.key("Escape");
    assert_eq!(state["popups"]["effects"], true);
    assert_eq!(state["project"], before);
    let saved = app.directory.path().join("effect.daw.json");
    app.command(json!({"command":"save","path":saved}));
    let state = app.command(json!({"command":"open","path":saved}));
    assert_eq!(state["project"], before);
    app.quit();
}

fn rms(path: &std::path::Path) -> f64 {
    let audio = daw_core::AudioData::load_wav(path).unwrap();
    assert!(audio.samples.iter().all(|x| x.is_finite()));
    (audio
        .samples
        .iter()
        .map(|x| f64::from(*x).powi(2))
        .sum::<f64>()
        / audio.samples.len() as f64)
        .sqrt()
}

fn export(app: &mut Harness, path: &std::path::Path) -> f64 {
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    rms(path)
}

#[test]
#[ignore = "requires installed CHOWTapeModel and a desktop via RILL_TEST_VST_EFFECT"]
fn native_effect_edit_autosnapshot_automation_archive_recall_and_crash_recovery() {
    let plugin = std::path::PathBuf::from(
        std::env::var_os("RILL_TEST_VST_EFFECT").expect("Set RILL_TEST_VST_EFFECT"),
    );
    let name = plugin.file_stem().unwrap().to_string_lossy().into_owned();
    let mut app = Harness::with_vst_path(plugin.parent().unwrap());
    menu(&mut app, "Track", "Add audio track");
    let source = app.directory.path().join("tone.wav");
    daw_core::AudioData {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..48_000)
            .map(|frame| 0.2 * (std::f32::consts::TAU * 220.0 * frame as f32 / 48_000.0).sin())
            .collect(),
    }
    .write_wav(&source)
    .unwrap();
    app.command(json!({"command":"drop","paths":[source],"track":0}));
    let inserted = insert(&mut app, &name);
    let track = inserted["project"]["tracks"][0]["id"].clone();
    let slot = inserted["project"]["tracks"][0]["inserts"][0]["id"].clone();
    let original_path = app.directory.path().join("original.wav");
    let baseline = export(&mut app, &original_path);
    assert!(baseline > 0.001);
    app.click("Open editor");
    let state = wait(&mut app, |state| {
        state["native_editor"]["open"] == true
            && state["native_editor"]["parameter_count"]
                .as_u64()
                .unwrap_or(0)
                > 0
    });
    assert_eq!(state["native_editor"]["effect"], slot);
    app.click("Search parameters");
    app.command(json!({"command":"text","text":"Output Gain"}));
    common::wait_visible(&mut app, "Output Gain");
    app.command(json!({"command":"click","label":"Output Gain","occurrence":1}));
    let state = app.inspect();
    let parameter = state["native_editor"]["selected"].clone();
    let rect = &state["instrument_parameter_rect"];
    // Slider rail starts on the left; choose a strongly attenuated value.
    app.command(json!({"command":"pointer","phase":"click","x":rect["min"][0].as_f64().unwrap()+15.0,"y":(rect["min"][1].as_f64().unwrap()+rect["max"][1].as_f64().unwrap())*0.5}));
    common::wait_visible(&mut app, "Automate last touched · Output Gain");
    let value = app.inspect()["native_editor"]["selected_value"]
        .as_f64()
        .unwrap();
    let state = wait(&mut app, |state| {
        state["project"]["tracks"][0]["inserts"][0]["state"]
            .as_array()
            .is_some_and(|bytes| !bytes.is_empty())
            && state["project"]["tracks"][0]["inserts"][0]["parameters"] == json!([])
    });
    assert_eq!(state["native_editor"]["last_touched"], parameter);
    let state = app.click("Automate last touched · Output Gain");
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["target"]["track"],
        track
    );
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["target"]["parameter"]["VstParameter"]["effect"],
        slot
    );
    assert_eq!(
        state["project"]["tracks"][1]["automation"]["target"]["parameter"]["VstParameter"]["id"],
        parameter
    );
    let state = app.key("Escape");
    assert_eq!(state["popups"]["instrument"], true);
    assert_eq!(state["native_editor"]["effect"], slot);
    let saved = app.directory.path().join("effect-session.rill");
    app.command(json!({"command":"save","path":saved}));
    let changed_path = app.directory.path().join("changed.wav");
    let changed = export(&mut app, &changed_path);
    assert!(
        changed < baseline * 0.8,
        "Edited output gain must affect rendered audio: {baseline} -> {changed}"
    );
    // Terminate only the helper owned by this isolated test app.
    let pid = app.inspect()["native_editor"]["pid"]
        .as_u64()
        .unwrap()
        .to_string();
    #[cfg(unix)]
    assert!(
        std::process::Command::new("kill")
            .args(["-KILL", &pid])
            .status()
            .unwrap()
            .success()
    );
    #[cfg(windows)]
    assert!(
        std::process::Command::new("taskkill")
            .args(["/PID", &pid, "/F"])
            .status()
            .unwrap()
            .success()
    );
    wait(&mut app, |state| {
        state["native_editor"]["open"] == false && !state["native_editor"]["error"].is_null()
    });
    app.click("Open editor");
    wait(&mut app, |state| state["native_editor"]["open"] == true);
    app.command(json!({"command":"click","label":"Output Gain","occurrence":1}));
    assert!(
        (app.inspect()["native_editor"]["selected_value"]
            .as_f64()
            .unwrap()
            - value)
            .abs()
            < 1e-5
    );
    app.command(json!({"command":"open","path":saved}));
    insert_editor_after_reopen(&mut app, &slot, value);
    let recalled_path = app.directory.path().join("recalled.wav");
    let recalled = export(&mut app, &recalled_path);
    assert!(
        (recalled - changed).abs() < changed.max(0.0001) * 0.05,
        "Saved effect/automation must recall audibly: {changed} vs {recalled}"
    );
    app.quit();
}

fn insert_editor_after_reopen(app: &mut Harness, slot: &Value, value: f64) {
    menu(app, "Window", "Effects");
    app.click("Open editor");
    let state = wait(app, |state| {
        state["native_editor"]["open"] == true
            && state["native_editor"]["parameter_count"]
                .as_u64()
                .unwrap_or(0)
                > 0
    });
    assert_eq!(&state["native_editor"]["effect"], slot);
    app.click("Search parameters");
    app.command(json!({"command":"text","text":"Output Gain"}));
    common::wait_visible(app, "Output Gain");
    app.command(json!({"command":"click","label":"Output Gain","occurrence":1}));
    assert!(
        (app.inspect()["native_editor"]["selected_value"]
            .as_f64()
            .unwrap()
            - value)
            .abs()
            < 1e-5
    );
}
