//! Real app tools discover controls through the isolated plug-in protocol before
//! editing parameters or automation. This fixture models metadata/state, not DSP.
mod common;

use common::Harness;
use daw_core::{Clip, Effect, EffectSlot, Instrument, PluginParameterValue, Project};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Command};

struct Fixture {
    worker: tempfile::TempDir,
    plugins: tempfile::TempDir,
    host: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let worker = tempfile::tempdir().unwrap();
        let source = worker.path().join("plugin_tool_host.rs");
        std::fs::write(&source, include_str!("common/parameter_host.rs")).unwrap();
        let host = worker.path().join(if cfg!(windows) {
            "plugin-tool-host.exe"
        } else {
            "plugin-tool-host"
        });
        let result = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&host)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "fixture compilation: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Self {
            worker,
            plugins: common::plugin_fixture(),
            host,
        }
    }

    fn app(&self) -> (Harness, u64, u64) {
        let mut app = Harness::with_plugin_host(self.plugins.path(), &self.host);
        let mut project = Project::new();
        let track = project.add_midi_track("Tool-controlled synth");
        let path = self
            .plugins
            .path()
            .join("RillFixture.vst3")
            .to_string_lossy()
            .into_owned();
        project
            .set_instrument(
                track,
                Instrument::Vst3 {
                    path: path.clone(),
                    class_id: Some("11111111111111111111111111111111".into()),
                },
            )
            .unwrap();
        let channel = project.track_mut(track).unwrap();
        channel.instrument_state = vec![40];
        channel
            .instrument_parameters
            .push(PluginParameterValue { id: 7, value: 0.62 });
        channel
            .clips
            .push(Clip::midi("Unchanged phrase", 0.0, 8.0, vec![]));
        let mut effect = EffectSlot::new(
            "Tool-controlled delay",
            Effect::Vst3 {
                path,
                class_id: Some("33333333333333333333333333333333".into()),
            },
        );
        effect.state = vec![30];
        effect.parameters.push(PluginParameterValue {
            id: 42,
            value: 0.31,
        });
        let insert = effect.id;
        channel.inserts.push(effect);
        let project_path = app.directory.path().join("plugin-tools.rill");
        project.save(&project_path).unwrap();
        app.command(json!({"command":"open","path":project_path}));
        (app, track, insert)
    }

    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(self.worker.path().join("requests.log"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    app.command(json!({"command":"song_tool","name":name,"arguments":arguments}))
}

fn apply(app: &mut Harness, state: &Value, edits: Value) -> Value {
    tool(
        app,
        "apply_edits",
        json!({
            "session_id":state["song_session"]["id"],
            "expected_revision":state["song_session"]["revision"],
            "label":"Tool: Shape sound", "edits":edits
        }),
    )
}

fn parameter_edit(track: u64, effect: Option<u64>, id: u32, value: f64) -> Value {
    json!({"op":"set_plugin_parameters","track":track,"effect":effect,"parameters":[{"id":id,"value":value}]})
}

fn automation_edit(track: u64, effect: Option<u64>, id: u32, value: f64) -> Value {
    json!({"op":"add_automation","name":"Filter movement",
        "target":{"track":track,"parameter":{"VstParameter":{"effect":effect,"id":id}}},"value":value})
}

fn assert_rejected(state: &Value, before: &Value, code: &str) {
    assert_eq!(
        state["tool_result"]["ok"], false,
        "{}",
        state["tool_result"]
    );
    assert_eq!(
        state["tool_result"]["error"]["code"], code,
        "{}",
        state["tool_result"]
    );
    assert_eq!(state["project"], before["project"]);
    assert_eq!(state["history"], before["history"]);
    assert_eq!(state["song_session"], before["song_session"]);
    assert_eq!(state["native_editor"]["open"], false);
}

fn assert_private_metadata(result: &Value, fixture: &Fixture) {
    let serialized = result.to_string();
    assert!(!serialized.contains(&fixture.plugins.path().to_string_lossy().to_string()));
    assert!(!serialized.contains("instrument_state"));
    assert!(!serialized.contains("ciphertext"));
    assert!(!serialized.contains("\"path\""));
}

#[test]
fn instrument_controls_and_automation_require_discovery_and_refresh_after_gui_undo() {
    let fixture = Fixture::new();
    let (mut app, track, _) = fixture.app();
    let original = app.inspect();
    let denied = apply(
        &mut app,
        &original,
        json!([parameter_edit(track, None, 7, 0.75)]),
    );
    assert_rejected(&denied, &original, "discovery_required");
    let denied = apply(
        &mut app,
        &original,
        json!([automation_edit(track, None, 7, 0.75)]),
    );
    assert_rejected(&denied, &original, "discovery_required");

    app.command(json!({"command":"mcp_request","request":{
        "jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion":"2025-11-25","capabilities":{},
            "clientInfo":{"name":"Plugin tool E2E","version":"1"}}
    }}));
    app.command(json!({"command":"mcp_request","request":{
        "jsonrpc":"2.0","method":"notifications/initialized"
    }}));
    let catalog = app.command(json!({"command":"mcp_request","request":{
        "jsonrpc":"2.0","id":2,"method":"tools/list"
    }}));
    let spec = catalog["mcp_result"]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|spec| spec["name"] == "plugin_describe_parameters")
        .unwrap();
    assert_eq!(spec["inputSchema"]["additionalProperties"], false);
    assert_eq!(spec["inputSchema"]["required"], json!(["track_id"]));
    assert_eq!(spec["inputSchema"]["properties"]["limit"]["maximum"], 200);
    assert!(spec["inputSchema"]["properties"].get("path").is_none());

    let first = tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"limit":1}),
    );
    assert_eq!(first["tool_result"]["ok"], true, "{}", first["tool_result"]);
    let metadata = &first["tool_result"]["result"];
    assert_eq!(metadata["total_parameters"], 208);
    assert_eq!(metadata["next_offset"], 1);
    assert_eq!(metadata["parameters"][0]["id"], 7);
    assert_eq!(metadata["parameters"][0]["normalized_value"], 0.62);
    assert_eq!(metadata["parameters"][0]["unit"], "Hz");
    assert_eq!(metadata["parameters"][0]["read_only"], false);
    assert_private_metadata(metadata, &fixture);
    assert_eq!(first["history"], original["history"]);
    assert_eq!(first["song_session"], original["song_session"]);
    assert_eq!(first["native_editor"]["open"], false);

    // Only returned pages authorize IDs: ID8 exists but has not been returned.
    let unknown = apply(
        &mut app,
        &original,
        json!([parameter_edit(track, None, 8, 0.12)]),
    );
    assert_rejected(&unknown, &original, "invalid_arguments");
    let second = tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"query":"Filter","offset":1,"limit":1}),
    );
    assert_eq!(second["tool_result"]["result"]["matched_parameters"], 2);
    assert_eq!(second["tool_result"]["result"]["parameters"][0]["id"], 8);
    assert!(second["tool_result"]["result"]["next_offset"].is_null());
    let meter = tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"query":"Output Meter"}),
    );
    assert_eq!(meter["tool_result"]["result"]["parameters"][0]["id"], 43);
    assert_eq!(
        meter["tool_result"]["result"]["parameters"][0]["read_only"],
        true
    );
    for edit in [
        parameter_edit(track, None, 43, 0.5),
        automation_edit(track, None, 43, 0.5),
        parameter_edit(track, None, 99999, 0.5),
    ] {
        let denied = apply(&mut app, &original, json!([edit]));
        assert_rejected(&denied, &original, "invalid_arguments");
    }

    let shaped = apply(
        &mut app,
        &original,
        json!([
            {"op":"set_plugin_parameters","track":track,"effect":null,"parameters":[{"id":7,"value":0.75},{"id":8,"value":0.12}]}
        ]),
    );
    assert_eq!(
        shaped["tool_result"]["ok"], true,
        "{}",
        shaped["tool_result"]
    );
    assert_eq!(
        shaped["project"]["tracks"][0]["instrument_parameters"],
        json!([{"id":7,"value":0.75},{"id":8,"value":0.12}])
    );
    assert_eq!(
        shaped["project"]["tracks"][0]["instrument_state"],
        original["project"]["tracks"][0]["instrument_state"]
    );
    assert_eq!(shaped["history"]["undo"].as_array().unwrap().len(), 1);
    let stale = apply(
        &mut app,
        &original,
        json!([parameter_edit(track, None, 7, 0.8)]),
    );
    assert_rejected(&stale, &shaped, "conflict");

    let automated = apply(
        &mut app,
        &shaped,
        json!([automation_edit(track, None, 7, 0.75)]),
    );
    assert_eq!(
        automated["tool_result"]["ok"], true,
        "{}",
        automated["tool_result"]
    );
    let lane = &automated["project"]["tracks"][1]["automation"];
    assert_eq!(
        lane["target"],
        json!({"track":track,"parameter":{"VstParameter":{"effect":null,"id":7}}})
    );
    assert_eq!(lane["points"][0]["value"], 0.75);
    assert_eq!(automated["history"]["undo"].as_array().unwrap().len(), 2);
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], shaped["project"]);
    let invalidated = apply(
        &mut app,
        &undone,
        json!([parameter_edit(track, None, 7, 0.55)]),
    );
    assert_rejected(&invalidated, &undone, "discovery_required");
    let refreshed = tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"query":"Filter"}),
    );
    assert_eq!(
        refreshed["tool_result"]["result"]["parameters"][0]["normalized_value"],
        0.75
    );
    let reshaped = apply(
        &mut app,
        &undone,
        json!([parameter_edit(track, None, 7, 0.55)]),
    );
    assert_eq!(
        reshaped["tool_result"]["ok"], true,
        "{}",
        reshaped["tool_result"]
    );
    assert_eq!(
        reshaped["project"]["tracks"][0]["instrument_parameters"][0]["value"],
        0.55
    );
    assert!(
        fixture
            .requests()
            .iter()
            .any(|request| request == "GetAllParameters")
    );
    app.quit();
}

#[test]
fn insert_discovery_is_scoped_and_bulk_parameter_automation_edits_commit_once() {
    let fixture = Fixture::new();
    let (mut app, track, insert) = fixture.app();
    let original = app.inspect();
    let denied = apply(
        &mut app,
        &original,
        json!([parameter_edit(track, Some(insert), 42, 0.6)]),
    );
    assert_rejected(&denied, &original, "discovery_required");
    let discovered = tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"insert_id":insert,"query":"Delay"}),
    );
    assert_eq!(
        discovered["tool_result"]["ok"], true,
        "{}",
        discovered["tool_result"]
    );
    let metadata = &discovered["tool_result"]["result"];
    assert_eq!(metadata["insert_id"], insert);
    assert_eq!(metadata["parameters"][0]["id"], 42);
    assert_eq!(metadata["parameters"][0]["normalized_value"], 0.31);
    assert_private_metadata(metadata, &fixture);
    let wrong_target = apply(
        &mut app,
        &original,
        json!([parameter_edit(track, None, 42, 0.6)]),
    );
    assert_rejected(&wrong_target, &original, "discovery_required");

    let applied = apply(
        &mut app,
        &original,
        json!([
            parameter_edit(track, Some(insert), 42, 0.6),
            automation_edit(track, Some(insert), 42, 0.6)
        ]),
    );
    assert_eq!(
        applied["tool_result"]["ok"], true,
        "{}",
        applied["tool_result"]
    );
    assert_eq!(
        applied["project"]["tracks"][0]["inserts"][0]["parameters"],
        json!([{"id":42,"value":0.6}])
    );
    assert_eq!(applied["history"]["undo"].as_array().unwrap().len(), 1);
    let lane = &applied["project"]["tracks"][1]["automation"];
    assert_eq!(
        lane["target"],
        json!({"track":track,"parameter":{"VstParameter":{"effect":insert,"id":42}}})
    );
    assert!((lane["points"][0]["value"].as_f64().unwrap() - 0.6).abs() < 1e-6);
    let refreshed = tool(
        &mut app,
        "plugin_describe_parameters",
        json!({"track_id":track,"insert_id":insert,"query":"42"}),
    );
    assert_eq!(
        refreshed["tool_result"]["result"]["parameters"][0]["normalized_value"],
        0.6
    );
    assert_eq!(refreshed["project"], applied["project"]);
    assert_eq!(refreshed["history"], applied["history"]);
    assert_eq!(refreshed["native_editor"]["open"], false);
    assert!(
        fixture
            .requests()
            .iter()
            .any(|request| request["SetParameter"]["id"] == 42
                && request["SetParameter"]["value"] == 0.6)
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], original["project"]);
    app.quit();
}
