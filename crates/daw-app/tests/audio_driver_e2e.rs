//! The Driver choice in Setup → Audio and the setup tools: hosts come from the
//! running app, devices follow the chosen host, and the choice survives restart.
mod common;
use common::Harness;
use serde_json::{Value, json};

fn setup(app: &mut Harness) {
    app.click("Window");
    app.click("Setup");
    app.click("Audio");
}
fn application(app: &mut Harness, tool: &str, arguments: Value) -> Value {
    app.command(json!({"command":"application_tool","name":tool,"arguments":arguments}))
}

#[test]
fn driver_choice_lists_hosts_drives_devices_and_persists() {
    // JACK stands in for a server-dictated driver: 44.1 kHz and a 128 frame buffer.
    let mut app = Harness::with_virtual_drivers("ALSA,JACK:44100:128", None);
    assert_eq!(app.inspect()["settings"]["audio_host"], Value::Null);
    setup(&mut app);
    let state = app.click("ALSA");
    assert_eq!(state["setup_draft"]["audio_host"], Value::Null);
    let state = app.click("JACK");
    assert_eq!(state["setup_draft"]["audio_host"], "JACK");
    assert_eq!(state["setup_draft"]["sample_rate"], 44_100);
    assert_eq!(state["setup_draft"]["buffer_frames"], 128);
    assert_eq!(state["settings"]["audio_host"], Value::Null);
    let state = app.click("Cancel");
    assert_eq!(state["settings"]["audio_host"], Value::Null);
    assert_eq!(state["settings"]["sample_rate"], 48_000);

    setup(&mut app);
    app.click("ALSA");
    app.click("JACK");
    let state = app.click("Apply");
    assert_eq!(state["settings"]["audio_host"], "JACK");
    assert_eq!(state["settings"]["buffer_frames"], 128);
    assert_eq!(state["error"], false, "{}", state["status"]);

    let mut app = app.restart();
    let state = app.inspect();
    assert_eq!(state["settings"]["audio_host"], "JACK");
    assert_eq!(state["settings"]["sample_rate"], 44_100);
    let saved: Value = serde_json::from_slice(
        &std::fs::read(app.directory.path().join("config/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["audio_host"], "JACK");

    // Switching back restores the full choice of rates and buffers.
    setup(&mut app);
    app.click("JACK");
    let state = app.click("ALSA");
    assert_eq!(state["setup_draft"]["audio_host"], Value::Null);
    let state = app.click("Apply");
    assert_eq!(state["settings"]["audio_host"], Value::Null);
    app.quit();
}

#[test]
fn a_single_driver_hides_the_control() {
    let mut app = Harness::with_virtual_audio(None, None);
    setup(&mut app);
    let state = app.inspect();
    let labels = state["visible_labels"].as_array().unwrap();
    assert!(labels.iter().any(|label| label == "Sample rate"));
    assert!(!labels.iter().any(|label| label == "Driver"));
    app.quit();
}

#[test]
fn an_unavailable_saved_driver_falls_back_to_the_default_with_feedback() {
    let saved = r#"{"audio_host":"ASIO","device_name":"Old interface","sample_rate":44100}"#;
    let mut app = Harness::with_virtual_drivers("ALSA,JACK", Some(saved));
    let state = app.inspect();
    assert_eq!(state["settings"]["audio_host"], Value::Null);
    assert_eq!(state["settings"]["device_name"], Value::Null);
    assert_eq!(state["error"], true);
    assert!(
        state["status"]
            .as_str()
            .unwrap()
            .contains("Audio driver 'ASIO' is unavailable; using the default driver"),
        "{}",
        state["status"]
    );
    assert_eq!(state["settings"]["sample_rate"], 44_100);
    // The file is untouched until the user changes Setup.
    let saved: Value = serde_json::from_slice(
        &std::fs::read(app.directory.path().join("config/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["audio_host"], "ASIO");
    app.quit();
}

#[test]
fn setup_tools_report_and_change_the_driver() {
    let mut app = Harness::with_virtual_drivers("ALSA,JACK:44100:128", None);
    let read = application(&mut app, "get_setup", json!({}));
    let setup = &read["tool_result"]["result"];
    assert_eq!(
        setup["available_devices"]["hosts"],
        json!([
            {"name":"ALSA","is_default":true,"selected":true},
            {"name":"JACK","is_default":false,"selected":false}
        ])
    );
    assert_eq!(setup["audio"]["audio_host"], Value::Null);

    let denied = application(
        &mut app,
        "configure_setup",
        json!({"audio":{"audio_host":"ASIO"}}),
    );
    assert_eq!(denied["tool_result"]["ok"], false);
    assert!(
        denied["tool_result"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Available: ALSA, JACK")
    );

    let changed = application(
        &mut app,
        "configure_setup",
        json!({"audio":{"audio_host":"jack"}}),
    );
    assert_eq!(changed["tool_result"]["ok"], true, "{changed}");
    let audio = &changed["tool_result"]["result"]["setup"]["audio"];
    assert_eq!(audio["audio_host"], "JACK");
    assert_eq!(audio["sample_rate"], 44_100);
    assert_eq!(audio["buffer_frames"], 128);
    let read = application(&mut app, "get_setup", json!({}));
    let setup = &read["tool_result"]["result"];
    assert_eq!(setup["available_devices"]["hosts"][1]["selected"], true);
    assert_eq!(
        setup["available_devices"]["outputs"][0]["buffer_frames"],
        json!({"min":128,"max":128})
    );

    let back = application(
        &mut app,
        "configure_setup",
        json!({"audio":{"audio_host":null}}),
    );
    assert_eq!(
        back["tool_result"]["result"]["setup"]["audio"]["audio_host"],
        Value::Null
    );
    app.quit();
}
