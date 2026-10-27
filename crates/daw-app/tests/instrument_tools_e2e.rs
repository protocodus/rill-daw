//! Complete instrument parts through the running assistant/tool dispatcher.
//! Metadata-only installed instruments avoid native code and audio hardware.
mod common;

use common::Harness;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[test]
fn explicit_assignment_uses_catalog_roles_and_never_opens_an_editor() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let instrument = instrument_id(&mut app);
    let listing = song_tool(&mut app, "list_plugins", json!({"query":"Rill Fixture FX"}));
    let effect = listing["tool_result"]["result"]["plugins"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let initial = app.inspect();
    let created = song_tool(&mut app, "create_midi_part", part(&initial));
    let track = created["project"]["tracks"][0]["id"].clone();
    for (plugin_id, insert) in [(&effect, false), (&instrument, true)] {
        let rejected = song_tool(
            &mut app,
            "assign_plugin",
            json!({"session_id":created["song_session"]["id"],
            "expected_revision":created["song_session"]["revision"],"track_id":track,"plugin_id":plugin_id,"insert":insert}),
        );
        assert_eq!(
            rejected["tool_result"]["error"]["code"],
            "invalid_arguments"
        );
        assert_eq!(rejected["project"], created["project"]);
        assert_eq!(rejected["history"], created["history"]);
    }
    let assigned = song_tool(
        &mut app,
        "assign_plugin",
        json!({"session_id":created["song_session"]["id"],
        "expected_revision":created["song_session"]["revision"],"track_id":track,"plugin_id":instrument}),
    );
    assert_eq!(assigned["tool_result"]["ok"], true);
    let inserted = song_tool(
        &mut app,
        "assign_plugin",
        json!({"session_id":assigned["song_session"]["id"],
        "expected_revision":assigned["song_session"]["revision"],"track_id":track,"plugin_id":effect,"insert":true}),
    );
    assert_eq!(inserted["tool_result"]["ok"], true);
    assert_eq!(
        inserted["project"]["tracks"][0]["inserts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(inserted["native_editor"]["open"], false);
    assert_eq!(inserted["popups"]["instrument"], false);
    app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(app.inspect()["project"], assigned["project"]);
    app.quit();
}

fn song_tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"song_tool","name":name,"arguments":arguments}))
}

fn instrument_id(app: &mut Harness) -> String {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let state = song_tool(app, "list_plugins", json!({"query":"Rill Fixture A"}));
        assert_eq!(state["tool_result"]["ok"], true, "{state}");
        if let Some(plugin) = state["tool_result"]["result"]["plugins"]
            .as_array()
            .and_then(|plugins| {
                plugins
                    .iter()
                    .find(|plugin| plugin["name"] == "Rill Fixture A")
            })
        {
            return plugin["id"].as_str().unwrap().to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "Fixture instrument did not scan: {state}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn part(state: &Value) -> Value {
    json!({
        "session_id":state["song_session"]["id"],
        "expected_revision":state["song_session"]["revision"],
        "name":"Instrument bass", "start":8.0, "length":4.0,
        "notes":[{"pitch":36,"start":0.0,"length":0.75,"velocity":100},
                 {"pitch":43,"start":1.0,"length":0.75,"velocity":90},
                 {"pitch":41,"start":2.0,"length":0.75,"velocity":100},
                 {"pitch":43,"start":3.0,"length":0.75,"velocity":95}]
    })
}

#[test]
fn one_assistant_tool_creates_notes_and_instrument_with_one_gui_undo() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let plugin_id = instrument_id(&mut app);
    let initial = app.inspect();
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":[{"id":"instrument_part","name":"create_instrument_part",
            "arguments":{"plugin_id":plugin_id,"part":part(&initial)}}]},
        {"text":"Created the bass phrase with Rill Fixture A."}
    ]}));
    app.click("Window");
    let opened = app.click("Assistant");
    let rect = &opened["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    app.command(
        json!({"command":"text","text":"Add a bass phrase at beat 8 using Rill Fixture A."}),
    );
    app.click("Send");
    let created = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(created["assistant"]["busy"], false, "{created}");
    let tracks = created["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0]["name"], "Instrument bass");
    assert_eq!(
        tracks[0]["instrument"]["Vst3"]["class_id"],
        "11111111111111111111111111111111"
    );
    assert_eq!(
        std::path::Path::new(tracks[0]["instrument"]["Vst3"]["path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        fixture
            .path()
            .join("RillFixture.vst3")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(tracks[0]["clips"][0]["start"], 8.0);
    assert_eq!(
        tracks[0]["clips"][0]["content"]["Midi"]["notes"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(created["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(created["popups"]["instrument"], false);
    assert!(
        created["assistant"]["messages"]
            .to_string()
            .contains("Created the bass phrase")
    );

    let undone = app.click("Undo");
    assert_eq!(undone["project"], initial["project"]);
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redone["project"], created["project"]);
    app.quit();
}

#[test]
fn instrument_generator_uses_shared_tool_and_unknown_catalog_ids_never_commit() {
    let fixture = common::plugin_fixture();
    let mut app = Harness::with_vst_path(fixture.path());
    let plugin_id = instrument_id(&mut app);
    let initial = app.inspect();
    let generated = song_tool(
        &mut app,
        "create_instrument_part",
        json!({
            "plugin_id":plugin_id,"generate":true,
            "part":{"session_id":initial["song_session"]["id"],"expected_revision":initial["song_session"]["revision"],
                "name":"Generated arp","start":0.0,"length":8.0,"kind":"arpeggio","seed":42}
        }),
    );
    assert_eq!(generated["tool_result"]["ok"], true, "{generated}");
    assert_eq!(generated["project"]["tracks"].as_array().unwrap().len(), 1);
    assert!(
        !generated["project"]["tracks"][0]["clips"][0]["content"]["Midi"]["notes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(generated["history"]["undo"].as_array().unwrap().len(), 1);
    assert_eq!(
        generated["tool_result"]["result"]["track_id"],
        generated["project"]["tracks"][0]["id"]
    );

    let failed = song_tool(
        &mut app,
        "create_instrument_part",
        json!({
            "plugin_id":"/an/arbitrary/plugin.vst3","part":part(&generated)
        }),
    );
    assert_eq!(failed["tool_result"]["ok"], false);
    assert_eq!(failed["tool_result"]["error"]["code"], "not_found");
    assert_eq!(failed["project"], generated["project"]);
    assert_eq!(failed["history"], generated["history"]);
    assert_eq!(failed["song_session"], generated["song_session"]);
    app.quit();
}
