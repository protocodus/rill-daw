//! MCP is a transport adapter over Rill's canonical tools, not another editor.
//!
//! Supply a [`ToolHost`] attached to the running application session. The host
//! remains responsible for session capabilities, document revisions, atomic
//! commits, history, and cancellation. This crate never loads a project, opens
//! an audio device, or obtains provider credentials.

use daw_tools::ToolHost;
use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};

/// Current protocol plus older versions with the tools features implemented here.
pub const PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-06-18", "2025-03-26", "2024-11-05"];
/// Bound a single stdio frame before parsing or allocating its full contents.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct McpServer {
    initialize_received: bool,
    ready: bool,
}

impl McpServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one JSON-RPC message. Notifications deliberately produce no reply.
    ///
    /// MCP clients must initialize before calling tools. The caller controls the
    /// session bound to `host`; JSON arguments cannot replace that capability.
    pub fn handle_request<H: ToolHost + ?Sized>(
        &mut self,
        host: &mut H,
        request: Value,
    ) -> Option<Value> {
        let Some(object) = request.as_object() else {
            return Some(rpc_error(Value::Null, -32600, "Invalid request"));
        };
        let id = object.get("id").cloned();
        let valid_id = id
            .as_ref()
            .is_none_or(|id| id.is_null() || id.is_string() || id.is_i64() || id.is_u64());
        let method = object.get("method").and_then(Value::as_str);
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || method.is_none_or(str::is_empty)
            || !valid_id
        {
            return Some(rpc_error(Value::Null, -32600, "Invalid request"));
        }
        let method = method.unwrap();
        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        let Some(id) = id else {
            if method == "notifications/initialized"
                && self.initialize_received
                && params.is_object()
            {
                self.ready = true;
            }
            return None;
        };
        if !params.is_object() {
            return Some(rpc_error(id, -32602, "Parameters must be an object"));
        }
        let result = match method {
            "initialize" => {
                if self.initialize_received {
                    return Some(rpc_error(id, -32600, "Already initialized"));
                }
                let Some(version) = params.get("protocolVersion").and_then(Value::as_str) else {
                    return Some(rpc_error(id, -32602, "Missing protocolVersion"));
                };
                if version.is_empty()
                    || !params.get("capabilities").is_some_and(Value::is_object)
                    || !params.get("clientInfo").is_some_and(|info| {
                        info.get("name").is_some_and(Value::is_string)
                            && info.get("version").is_some_and(Value::is_string)
                    })
                {
                    return Some(rpc_error(id, -32602, "Invalid initialize parameters"));
                }
                self.initialize_received = true;
                json!({
                    "protocolVersion": if SUPPORTED_VERSIONS.contains(&version) { version } else { PROTOCOL_VERSION },
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "rill", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Use the structured tools published by this connection and bounded edit batches instead of driving the graphical interface. The host's published catalog defines the connection's capabilities. Query actual object IDs and parameter descriptors before editing."
                })
            }
            "ping" => json!({}),
            "tools/list" | "tools/call" if !self.ready => {
                return Some(rpc_error(id, -32002, "Server not initialized"));
            }
            "tools/list" => {
                if params.get("cursor").is_some_and(|cursor| !cursor.is_null()) {
                    return Some(rpc_error(id, -32602, "Invalid pagination cursor"));
                }
                let specs = host.tool_specs();
                let tools: Vec<Value> = specs
                    .into_iter()
                    .map(|spec| {
                        json!({
                            "name": spec.name,
                            "description": spec.description,
                            "inputSchema": spec.input_schema
                        })
                    })
                    .collect();
                json!({ "tools": tools })
            }
            "tools/call" => {
                let Some(name) = params.get("name").and_then(Value::as_str) else {
                    return Some(rpc_error(id, -32602, "Missing tool name"));
                };
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if name.is_empty() || !arguments.is_object() {
                    return Some(rpc_error(id, -32602, "Invalid tool arguments"));
                }
                if !host.tool_specs().iter().any(|spec| spec.name == name) {
                    return Some(rpc_error(
                        id,
                        -32602,
                        "Tool is unavailable in this connection",
                    ));
                }
                match host.call_tool(name, arguments) {
                    Ok(value) => {
                        let structured = if value.is_object() {
                            value
                        } else {
                            json!({ "result": value })
                        };
                        json!({
                            "content": [{ "type": "text", "text": structured.to_string() }],
                            "structuredContent": structured,
                            "isError": false
                        })
                    }
                    Err(error) => {
                        if matches!(error.code.as_str(), "unknown_tool" | "invalid_arguments") {
                            return Some(rpc_error(id, -32602, &error.message));
                        }
                        let details =
                            json!({ "error": { "code": error.code, "message": error.message } });
                        json!({
                            "content": [{ "type": "text", "text": details.to_string() }],
                            "structuredContent": details,
                            "isError": true
                        })
                    }
                }
            }
            _ => return Some(rpc_error(id, -32601, "Method not found")),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Serve newline-delimited MCP stdio against a host bound to a live session.
/// Only protocol responses are written to `writer`; diagnostics belong on stderr.
/// Oversized frames terminate the connection without allocating an unbounded line.
pub fn serve<R: BufRead, W: Write, H: ToolHost + ?Sized>(
    mut reader: R,
    mut writer: W,
    host: &mut H,
) -> io::Result<()> {
    let mut server = McpServer::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = (&mut reader)
            .take((MAX_MESSAGE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            return Ok(());
        }
        if count > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP message too large",
            ));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let response = match serde_json::from_slice(&line) {
            Ok(request) => server.handle_request(host, request),
            Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            let bytes = serde_json::to_vec(&response)?;
            if bytes.len() > MAX_MESSAGE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "MCP response too large",
                ));
            }
            writer.write_all(&bytes)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_tools::{ToolError, ToolProfile, song_tool_specs, tool_specs_for};
    use std::io::Cursor;

    #[derive(Default)]
    struct Host {
        calls: Vec<(String, Value)>,
        failure: Option<&'static str>,
        profile: ToolProfile,
    }

    impl ToolHost for Host {
        fn tool_specs(&self) -> Vec<daw_tools::ToolSpec> {
            tool_specs_for(self.profile)
        }
        fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError> {
            self.calls.push((name.to_owned(), arguments.clone()));
            if let Some(code) = self.failure {
                Err(ToolError::new(code, "Tool request failed"))
            } else {
                Ok(json!({ "arguments": arguments, "revision": 7 }))
            }
        }
    }

    fn request(id: Value, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    fn initialize<H: ToolHost>(server: &mut McpServer, host: &mut H, version: &str) -> Value {
        let result = server
            .handle_request(
                host,
                request(
                    json!(1),
                    "initialize",
                    json!({
                        "protocolVersion": version,
                        "capabilities": {},
                        "clientInfo": { "name": "test", "version": "1" }
                    }),
                ),
            )
            .unwrap();
        assert!(
            server
                .handle_request(
                    host,
                    json!({
                        "jsonrpc": "2.0", "method": "notifications/initialized"
                    })
                )
                .is_none()
        );
        result
    }

    #[test]
    fn negotiates_protocol_and_publishes_exact_canonical_schemas() {
        let mut server = McpServer::new();
        let mut host = Host::default();
        let init = initialize(&mut server, &mut host, "2025-06-18");
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        let listed = server
            .handle_request(&mut host, request(json!(2), "tools/list", json!({})))
            .unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        let specs = song_tool_specs();
        assert_eq!(tools.len(), specs.len());
        for (tool, spec) in tools.iter().zip(specs) {
            assert_eq!(tool["name"], spec.name);
            assert_eq!(tool["description"], spec.description);
            assert_eq!(tool["inputSchema"], spec.input_schema);
        }
        assert!(host.calls.is_empty());
    }

    #[test]
    fn application_tools_require_host_capability_before_execution() {
        let mut server = McpServer::new();
        let mut host = Host::default();
        initialize(&mut server, &mut host, PROTOCOL_VERSION);
        let denied=server.handle_request(&mut host,request(json!(2),"tools/call",json!({"name":"document_action","arguments":{"action":"new","discard_unsaved":true}}))).unwrap();
        assert_eq!(denied["error"]["code"], -32602);
        assert!(host.calls.is_empty());
        let song_list = server
            .handle_request(&mut host, request(json!(3), "tools/list", json!({})))
            .unwrap();
        assert!(
            !song_list["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|spec| spec["name"] == "document_action")
        );
        host.profile = ToolProfile::Application;
        let list = server
            .handle_request(&mut host, request(json!(4), "tools/list", json!({})))
            .unwrap();
        assert!(
            list["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|spec| spec["name"] == "document_action")
        );
        let allowed = server
            .handle_request(
                &mut host,
                request(
                    json!(5),
                    "tools/call",
                    json!({"name":"document_action","arguments":{"action":"new"}}),
                ),
            )
            .unwrap();
        assert_eq!(allowed["result"]["isError"], false);
        assert_eq!(host.calls[0].0, "document_action");
    }

    #[test]
    fn delegates_arguments_and_returns_structured_success_and_execution_errors() {
        let mut server = McpServer::new();
        let mut host = Host::default();
        initialize(&mut server, &mut host, PROTOCOL_VERSION);
        let args = json!({"track_id": 42, "gain_db": -3.0});
        let success = server
            .handle_request(
                &mut host,
                request(
                    json!("edit"),
                    "tools/call",
                    json!({
                        "name": "set_mix", "arguments": args
                    }),
                ),
            )
            .unwrap();
        assert_eq!(success["id"], "edit");
        assert_eq!(host.calls, vec![("set_mix".into(), args.clone())]);
        assert_eq!(success["result"]["structuredContent"]["arguments"], args);
        assert_eq!(success["result"]["isError"], false);
        host.failure = Some("conflict");
        let failed = server
            .handle_request(
                &mut host,
                request(
                    json!(3),
                    "tools/call",
                    json!({
                        "name": "set_mix", "arguments": args
                    }),
                ),
            )
            .unwrap();
        assert_eq!(failed["result"]["isError"], true);
        assert_eq!(
            failed["result"]["structuredContent"]["error"]["code"],
            "conflict"
        );
        assert!(failed.get("error").is_none());
        for code in ["unknown_tool", "invalid_arguments"] {
            host.failure = Some(code);
            let invalid = server
                .handle_request(
                    &mut host,
                    request(
                        json!(4),
                        "tools/call",
                        json!({
                            "name": "get_song_context", "arguments": {}
                        }),
                    ),
                )
                .unwrap();
            assert_eq!(invalid["error"]["code"], -32602);
            assert!(invalid.get("result").is_none());
        }
    }

    #[test]
    fn validates_handshake_and_rejects_malformed_requests_without_execution() {
        let mut server = McpServer::new();
        let mut host = Host::default();
        let before = server
            .handle_request(&mut host, request(json!(1), "tools/list", json!({})))
            .unwrap();
        assert_eq!(before["error"]["code"], -32002);
        let malformed = server
            .handle_request(&mut host, request(json!(2), "initialize", json!({})))
            .unwrap();
        assert_eq!(malformed["error"]["code"], -32602);
        let init = initialize(&mut server, &mut host, "future-protocol");
        assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSION);
        for malformed in [
            json!([]),
            json!({"jsonrpc":"1.0","id":3,"method":"ping"}),
            json!({"jsonrpc":"2.0","id":{},"method":"ping"}),
        ] {
            assert_eq!(
                server.handle_request(&mut host, malformed).unwrap()["error"]["code"],
                -32600
            );
        }
        let args = server
            .handle_request(
                &mut host,
                request(
                    json!(4),
                    "tools/call",
                    json!({"name":"anything","arguments":[]}),
                ),
            )
            .unwrap();
        assert_eq!(args["error"]["code"], -32602);
        let unknown = server
            .handle_request(&mut host, request(json!(5), "missing", json!({})))
            .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        assert!(host.calls.is_empty());
    }

    #[test]
    fn stdio_recovers_from_parse_errors_and_never_replies_to_notifications() {
        let input = concat!(
            "not-json\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"unknown/notification\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n"
        );
        let mut output = Vec::new();
        serve(Cursor::new(input), &mut output, &mut Host::default()).unwrap();
        let messages: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["error"]["code"], -32700);
        assert_eq!(messages[1]["id"], 9);
        assert_eq!(messages[1]["result"], json!({}));
    }

    #[test]
    fn stdio_rejects_oversized_frames() {
        let input = vec![b' '; MAX_MESSAGE_BYTES + 1];
        let error = serve(Cursor::new(input), Vec::new(), &mut Host::default()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn real_song_dispatch_creates_a_phrase_in_one_undoable_call() {
        use daw_core::{EditBatch, EditResult, History, Project, SongSession};
        use daw_tools::{SongToolHost, dispatch_song_tool};

        struct SongHost {
            project: Project,
            history: History,
            session: SongSession,
        }
        impl SongToolHost for SongHost {
            fn project(&self) -> &Project {
                &self.project
            }
            fn song_session(&self) -> SongSession {
                self.session
            }
            fn apply_edits(&mut self, batch: EditBatch) -> Result<EditResult, ToolError> {
                let before = self.project.clone();
                let result = batch
                    .apply(&mut self.project)
                    .map_err(|error| ToolError::new("edit_failed", error.to_string()))?;
                if result.changed {
                    self.history.commit_named(&before, batch.label);
                    self.session.bump();
                }
                Ok(result)
            }
        }
        impl ToolHost for SongHost {
            fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError> {
                dispatch_song_tool(self, name, arguments)
            }
        }

        let mut host = SongHost {
            project: Project::new(),
            history: History::new(),
            session: SongSession::default(),
        };
        let mut server = McpServer::new();
        initialize(&mut server, &mut host, PROTOCOL_VERSION);
        let args = json!({
            "session_id": host.session.id, "expected_revision": host.session.revision,
            "name": "Bass", "start": 0.0, "length": 4.0,
            "notes": [
                {"pitch":36,"start":0.0,"length":0.5,"velocity":100},
                {"pitch":43,"start":1.0,"length":0.5,"velocity":90}
            ]
        });
        let result = server
            .handle_request(
                &mut host,
                request(
                    json!(2),
                    "tools/call",
                    json!({
                        "name":"create_midi_part", "arguments":args
                    }),
                ),
            )
            .unwrap();
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert_eq!(host.project.tracks.len(), 1);
        assert_eq!(host.project.tracks[0].name, "Bass");
        assert_eq!(host.project.tracks[0].clips.len(), 1);
        let track_id = host.project.tracks[0].id;
        let clip_id = host.project.tracks[0].clips[0].id;
        assert_eq!(host.project.midi_notes(track_id, clip_id).unwrap().len(), 2);
        let direct = host.call_tool("get_song_context", json!({})).unwrap();
        let via_mcp = server
            .handle_request(
                &mut host,
                request(
                    json!(3),
                    "tools/call",
                    json!({
                        "name":"get_song_context", "arguments":{}
                    }),
                ),
            )
            .unwrap();
        assert_eq!(via_mcp["result"]["structuredContent"], direct);
        assert!(host.history.undo(&mut host.project));
        assert!(host.project.tracks.is_empty());
        assert!(!host.history.undo(&mut host.project));
    }
}
