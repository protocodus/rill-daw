//! Integrated assistant and MCP bridge. All tool execution enters the same
//! control-thread dispatcher; providers and stdio never borrow the live song.
use crate::app::DawApp;
use daw_ai::{AgentEvent, AgentRun, ScriptedTurn};
use daw_tools::{SongToolHost, ToolError, ToolHost};
use eframe::egui;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};
use zeroize::Zeroize;

struct ToolRequest {
    profile: daw_tools::ToolProfile,
    cancel: Arc<AtomicBool>,
    name: String,
    arguments: Value,
    reply: Sender<Result<Value, ToolError>>,
}

struct PendingTool {
    receiver: Receiver<Result<Value, String>>,
    reply: Sender<Result<Value, ToolError>>,
    session: daw_core::SongSession,
    cancel: Arc<AtomicBool>,
    owner_cancel: Arc<AtomicBool>,
    process: Option<String>,
    metadata_target: Option<(u64, Option<u64>)>,
}

#[derive(Default)]
pub(crate) struct Assistant {
    pub open: bool,
    pub prompt: String,
    pub messages: Vec<daw_ui::AssistantMessage>,
    pub status: String,
    key_draft: String,
    pub(crate) known_parameters:
        std::collections::HashMap<(u64, Option<u64>), std::collections::HashMap<u32, bool>>,
    run: Option<AgentRun>,
    run_session: u64,
    next_script: Option<Vec<ScriptedTurn>>,
    mcp_requests: Option<Receiver<ToolRequest>>,
    pending: Vec<PendingTool>,
}

impl Drop for Assistant {
    fn drop(&mut self) {
        self.stop();
        self.cancel_all_tools();
        self.clear_key_draft();
    }
}

impl Assistant {
    pub fn busy(&self) -> bool {
        self.run.is_some()
    }
    pub fn stop(&mut self) {
        if let Some(run) = self.run.take() {
            run.cancel();
            self.status = "Stopped".into();
        }
    }
    pub fn cancel_all_tools(&self) {
        for job in &self.pending {
            job.cancel.store(true, Ordering::Release);
        }
    }
    pub fn clear_key_draft(&mut self) {
        self.key_draft.zeroize();
    }
    pub fn set_script(&mut self, turns: Vec<ScriptedTurn>) {
        self.next_script = Some(turns);
    }

    pub fn start_mcp_stdio(&mut self, profile: daw_tools::ToolProfile) {
        let (sender, receiver) = mpsc::channel();
        self.mcp_requests = Some(receiver);
        std::thread::spawn(move || {
            struct ChannelHost(Sender<ToolRequest>, daw_tools::ToolProfile);
            impl ToolHost for ChannelHost {
                fn tool_specs(&self) -> Vec<daw_tools::ToolSpec> {
                    daw_tools::tool_specs_for(self.1)
                }
                fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError> {
                    let (reply, receiver) = mpsc::channel();
                    let cancel = Arc::new(AtomicBool::new(false));
                    self.0
                        .send(ToolRequest {
                            profile: self.1,
                            cancel: cancel.clone(),
                            name: name.into(),
                            arguments,
                            reply,
                        })
                        .map_err(|_| ToolError::new("unavailable", "Rill closed"))?;
                    match receiver.recv_timeout(std::time::Duration::from_secs(90)) {
                        Ok(result) => result,
                        Err(_) => {
                            cancel.store(true, Ordering::Release);
                            Err(ToolError::new("timeout", "Tool dispatch timed out"))
                        }
                    }
                }
            }
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            if let Err(error) = daw_mcp::serve(
                stdin.lock(),
                stdout.lock(),
                &mut ChannelHost(sender, profile),
            ) {
                eprintln!("MCP transport ended: {error}");
            }
        });
    }
}

impl DawApp {
    pub(crate) fn assistant_diagnostics(&self, ctx: &egui::Context) -> Value {
        let rect = |id: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(id)))
                .map(|r| json!({"min":[r.min.x,r.min.y],"max":[r.max.x,r.max.y]}))
        };
        json!({"busy":self.assistant.busy(),"prompt":self.assistant.prompt,
            "status":self.assistant.status,"messages":self.assistant.messages.iter().map(|m|
                json!({"role":format!("{:?}",m.role),"text":m.text})).collect::<Vec<_>>(),
            "prompt_rect":rect("assistant_prompt_rect"),"key_rect":rect("ai_key_rect"),
            "model_rect":rect("ai_model_rect")})
    }

    fn start_assistant(&mut self) {
        if self.assistant.busy() {
            return;
        }
        if let Err(error) = self.tool_ready() {
            self.assistant.status = error.message;
            return;
        }
        let context = match self.call_tool("get_song_context", json!({})) {
            Ok(song) => json!({"song":song,
                "selection":{"track_id":self.selected_track,"clip_id":self.selected_clip,"note_ids":self.selected_notes,"cursor_beat":self.cursor,"clips":self.arranger_selection_ids(),"range":self.arranger_phrase_span()},
                "tracks":self.call_tool("list_tracks",json!({"limit":100})).ok(),
                "previous_messages":self.assistant.messages.iter().rev().take(8).collect::<Vec<_>>().into_iter().rev().map(|m|json!({"role":format!("{:?}",m.role),"text":m.text.chars().take(2000).collect::<String>()})).collect::<Vec<_>>()}),
            Err(error) => {
                self.assistant.status = error.message;
                return;
            }
        };
        let prompt = self.assistant.prompt.trim().to_owned();
        let request = daw_ai::AgentRequest {
            prompt: prompt.clone(),
            context,
            tools: daw_tools::song_tool_specs(),
        };
        let result = if let Some(script) = self.assistant.next_script.take() {
            daw_ai::spawn_scripted(request, script)
        } else {
            self.settings
                .ai
                .api_key
                .as_ref()
                .ok_or_else(|| "Add your API key in Setup → AI".to_string())
                .and_then(daw_ai::decrypt_api_key)
                .and_then(|api_key| {
                    daw_ai::spawn(
                        daw_ai::AgentConfig {
                            provider: self.settings.ai.provider,
                            model: self.settings.ai.model.clone(),
                            api_key,
                        },
                        request,
                    )
                })
        };
        match result {
            Ok(run) => {
                self.assistant.run_session = self.song_session.id;
                self.assistant.run = Some(run);
                self.assistant.messages.push(daw_ui::AssistantMessage {
                    role: daw_ui::AssistantRole::User,
                    text: prompt,
                });
                self.assistant.prompt.clear();
                self.assistant.status = "Working…".into();
            }
            Err(error) => self.assistant.status = error,
        }
    }

    /// A general catalog is available only to a deliberately privileged client.
    /// The embedded agent always uses CurrentSong at discovery and invocation.
    pub(crate) fn dispatch_profile_tool(
        &mut self,
        profile: daw_tools::ToolProfile,
        name: String,
        arguments: Value,
        reply: Sender<Result<Value, ToolError>>,
        cancel: Arc<AtomicBool>,
    ) {
        if cancel.load(Ordering::Acquire) {
            let _ = reply.send(Err(ToolError::new("cancelled", "Request abandoned")));
            return;
        }
        if !daw_tools::tool_specs_for(profile)
            .iter()
            .any(|spec| spec.name == name)
        {
            let _ = reply.send(Err(ToolError::new(
                "permission_denied",
                "Tool unavailable in this capability profile",
            )));
            return;
        }
        if profile == daw_tools::ToolProfile::Application
            && daw_tools::application_tool_specs()
                .iter()
                .any(|spec| spec.name == name)
        {
            let result = self
                .application_tool(&name, arguments)
                .unwrap_or_else(|| Err(ToolError::new("unknown_tool", "Unknown application tool")));
            let _ = reply.send(result);
            return;
        }
        self.dispatch_tool(name, arguments, reply, cancel);
    }

    /// Shared dispatch entry for assistant, stdio MCP and headless boundary tests.
    pub(crate) fn dispatch_tool(
        &mut self,
        name: String,
        arguments: Value,
        reply: Sender<Result<Value, ToolError>>,
        cancel: Arc<AtomicBool>,
    ) {
        if serde_json::to_vec(&arguments)
            .map_or(true, |bytes| bytes.len() > daw_tools::MAX_ARGUMENT_BYTES)
        {
            let _ = reply.send(Err(ToolError::new(
                "invalid_arguments",
                "Tool arguments exceed 1 MiB",
            )));
            return;
        }
        if cancel.load(Ordering::Acquire) {
            let _ = reply.send(Err(ToolError::new("cancelled", "Tool cancelled")));
            return;
        }
        if !daw_tools::song_tool_specs()
            .iter()
            .any(|spec| spec.name == name)
        {
            let _ = reply.send(Err(ToolError::new(
                "unknown_tool",
                "Tool unavailable in current-song profile",
            )));
            return;
        }
        if name == "join_clips" {
            self.start_join_tool(arguments, reply, cancel);
            return;
        }
        let mut arguments = arguments;
        let process = if name == "audio_process_clip" {
            if let Err(error) = self.tool_ready() {
                let _ = reply.send(Err(error));
                return;
            }
            let Some(object) = arguments.as_object_mut() else {
                let _ = reply.send(Err(ToolError::new(
                    "invalid_arguments",
                    "Expected an object",
                )));
                return;
            };
            let session = object.remove("session_id").and_then(|v| v.as_u64());
            let revision = object.remove("expected_revision").and_then(|v| v.as_u64());
            let op = object
                .remove("operation")
                .and_then(|v| v.as_str().map(str::to_owned));
            let (Some(session), Some(revision), Some(op)) = (session, revision, op) else {
                let _ = reply.send(Err(ToolError::new(
                    "invalid_arguments",
                    "Provide session_id, expected_revision and operation",
                )));
                return;
            };
            if !["normalize", "split_transients", "strip_silence"].contains(&op.as_str()) {
                let _ = reply.send(Err(ToolError::new(
                    "invalid_arguments",
                    "Unknown audio processing operation",
                )));
                return;
            }
            if let Err(error) = self.check_tool_revision(session, revision) {
                let _ = reply.send(Err(error));
                return;
            }
            Some(op)
        } else {
            None
        };
        if crate::tool_jobs::handles(&name) || process.is_some() {
            if let Err(error) = self.tool_ready() {
                let _ = reply.send(Err(error));
                return;
            }
            if self.assistant.pending.len() >= 2 {
                let _ = reply.send(Err(ToolError::new("busy", "Audio tool worker is busy")));
                return;
            }
            let metadata_target = if name == "plugin_describe_parameters" {
                arguments
                    .get("track_id")
                    .and_then(Value::as_u64)
                    .map(|track| (track, arguments.get("insert_id").and_then(Value::as_u64)))
            } else {
                None
            };
            let name = if process.is_some() {
                "audio_analyze_clip".into()
            } else {
                name
            };
            let owner_cancel = cancel;
            let cancel = Arc::new(AtomicBool::new(false));
            let project = self.project.clone();
            let rate = self.settings.sample_rate;
            let session = self.song_session;
            let worker_cancel = cancel.clone();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(crate::tool_jobs::execute(
                    project,
                    rate,
                    &name,
                    arguments,
                    &worker_cancel,
                ));
            });
            self.assistant.pending.push(PendingTool {
                receiver,
                reply,
                session,
                cancel,
                owner_cancel,
                process,
                metadata_target,
            });
        } else {
            let result = self.call_tool(&name, arguments);
            let _ = reply.send(result);
        }
    }

    pub(crate) fn poll_assistant(&mut self, ctx: &egui::Context) {
        self.poll_join_tools(ctx);
        let requests: Vec<_> = self
            .assistant
            .mcp_requests
            .as_ref()
            .map(|rx| rx.try_iter().take(8).collect())
            .unwrap_or_default();
        for request in requests {
            self.dispatch_profile_tool(
                request.profile,
                request.name,
                request.arguments,
                request.reply,
                request.cancel,
            );
        }
        let mut index = 0;
        while index < self.assistant.pending.len() {
            let job = &self.assistant.pending[index];
            let result =
                if job.cancel.load(Ordering::Acquire) || job.owner_cancel.load(Ordering::Acquire) {
                    Some(Err(ToolError::new("cancelled", "Tool cancelled")))
                } else if job.session != self.song_session {
                    Some(Err(ToolError::new(
                        "conflict",
                        "Song changed during analysis; request fresh results",
                    )))
                } else {
                    match job.receiver.try_recv() {
                        Ok(result) => {
                            Some(result.map_err(|message| ToolError::new("tool_failed", message)))
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            Some(Err(ToolError::new("unavailable", "Tool worker stopped")))
                        }
                        Err(mpsc::TryRecvError::Empty) => None,
                    }
                };
            if let Some(mut result) = result {
                let job = self.assistant.pending.swap_remove(index);
                if let Ok(value) = &result {
                    if let Some(target) = job.metadata_target {
                        let known = self.assistant.known_parameters.entry(target).or_default();
                        if let Some(parameters) = value.get("parameters").and_then(Value::as_array)
                        {
                            for parameter in parameters {
                                if let (Some(id), Some(read_only)) = (
                                    parameter["id"].as_u64().and_then(|v| u32::try_from(v).ok()),
                                    parameter["read_only"].as_bool(),
                                ) {
                                    known.insert(id, read_only);
                                }
                            }
                        }
                    }
                    if let Some(operation) = &job.process {
                        let edit = value["suggested_edits"][operation].clone();
                        result = if edit.is_null() {
                            if operation == "strip_silence" {
                                Err(ToolError::new(
                                    "limit_exceeded",
                                    "Too many silence regions; process a shorter range",
                                ))
                            } else {
                                Ok(
                                    json!({"changed":false,"operation":operation,"session_id":self.song_session.id,"revision":self.song_session.revision,
                                    "reason":if operation=="normalize"{"The source window is silent or below the normalization floor"}else{"No transients were detected"}}),
                                )
                            }
                        } else {
                            serde_json::from_value::<daw_core::SongEdit>(edit).map_err(|_|ToolError::new("tool_failed","Invalid prepared audio edit"))
                                .and_then(|edit|self.apply_edits(daw_core::EditBatch{label:format!("Audio: {}",operation.replace('_'," ")),edits:vec![edit]}))
                                .map(|commit|json!({"changed":commit.changed,"created":commit.created,"operation":operation,"session_id":self.song_session.id,"revision":self.song_session.revision}))
                        };
                    }
                }
                if let Ok(value) = &mut result {
                    value["session_id"] = json!(self.song_session.id);
                    value["revision"] = json!(self.song_session.revision);
                }
                job.cancel.store(true, Ordering::Release);
                let _ = job.reply.send(result);
            } else {
                index += 1;
            }
        }
        if self.assistant.run.is_some() && self.assistant.run_session != self.song_session.id {
            self.assistant.stop();
        }
        let events: Vec<_> = self
            .assistant
            .run
            .as_ref()
            .map(|run| run.events.try_iter().take(64).collect())
            .unwrap_or_default();
        for event in events {
            match event {
                AgentEvent::Progress(message) => self.assistant.status = message,
                AgentEvent::ToolCall {
                    name,
                    arguments,
                    reply,
                    ..
                } => {
                    let Some(run) = self.assistant.run.as_ref() else {
                        let _ = reply.send(Err("Run stopped".into()));
                        continue;
                    };
                    if run.is_cancelled() || self.assistant.run_session != self.song_session.id {
                        let _ = reply.send(Err("Run stopped".into()));
                        continue;
                    }
                    let cancel = run.cancellation_token();
                    self.assistant.messages.push(daw_ui::AssistantMessage {
                        role: daw_ui::AssistantRole::Tool,
                        text: name.replace('_', " "),
                    });
                    let (host_reply, receiver) = mpsc::channel();
                    self.dispatch_tool(name, arguments, host_reply, cancel);
                    std::thread::spawn(move || {
                        let result = receiver.recv().unwrap_or_else(|_| {
                            Err(ToolError::new("unavailable", "Tool dispatcher closed"))
                        });
                        let _ = reply.send(result.map_err(|e| e.to_string()));
                    });
                }
                AgentEvent::Finished(text) => {
                    self.assistant.messages.push(daw_ui::AssistantMessage {
                        role: daw_ui::AssistantRole::Assistant,
                        text,
                    });
                    self.assistant.status = "Done".into();
                    self.assistant.run = None;
                }
                AgentEvent::Failed(error) => {
                    self.assistant.status = error;
                    self.assistant.run = None;
                }
                AgentEvent::Cancelled => {
                    self.assistant.status = "Stopped".into();
                    self.assistant.run = None;
                }
            }
        }
        if self.assistant.messages.len() > 128 {
            let count = self.assistant.messages.len() - 128;
            self.assistant.messages.drain(..count);
        }
        if self.assistant.busy() || !self.assistant.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    pub(crate) fn assistant_popup(&mut self, ctx: &egui::Context) {
        if !self.assistant.open {
            return;
        }
        let context = self
            .selected_track
            .and_then(|id| self.project.track(id))
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "Current song".into());
        let mut open = true;
        let mut action = None;
        let can_submit = !self.recording_busy();
        let can_undo = self.history.can_undo();
        daw_ui::popup("Assistant", ctx)
            .open(&mut open)
            .default_width(620.0)
            .default_height(520.0)
            .vscroll(false)
            .show(ctx, |ui| {
                let response = daw_ui::assistant(
                    ui,
                    &mut self.assistant.prompt,
                    daw_ui::AssistantOptions {
                        messages: &self.assistant.messages,
                        status: Some(&self.assistant.status),
                        busy: self.assistant.run.is_some(),
                        can_submit,
                        can_undo,
                        context: &context,
                    },
                );
                ctx.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new("assistant_prompt_rect"),
                        response.prompt.interact_rect,
                    )
                });
                action = Some((response.send, response.stop, response.undo));
            });
        self.assistant.open = open;
        if let Some((send, stop, undo)) = action {
            if stop {
                self.assistant.stop();
            }
            if undo {
                self.undo_redo(false);
            }
            if send {
                self.start_assistant();
            }
        }
    }

    pub(crate) fn ai_setup(&mut self, ui: &mut egui::Ui) {
        daw_ui::form_grid(ui, "ai_settings", |ui| {
            let label = daw_ui::text::label(ui, "Provider");
            daw_ui::select("ai_provider", self.setup_draft.ai.provider.name())
                .show_ui(ui, |ui| {
                    for provider in [daw_ai::Provider::OpenAi, daw_ai::Provider::Anthropic] {
                        let previous_provider = self.setup_draft.ai.provider;
                        if daw_ui::selectable_value(
                            ui,
                            &mut self.setup_draft.ai.provider,
                            provider,
                            provider.name(),
                        )
                        .clicked()
                        {
                            if previous_provider != provider {
                                self.setup_draft.ai.model = provider.default_model().into();
                                self.setup_draft.ai.api_key = None;
                                self.assistant.clear_key_draft();
                            }
                            ui.close();
                        }
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();
            let label = daw_ui::text::label(ui, "Model");
            let response = daw_ui::text_field(ui, &mut self.setup_draft.ai.model, "Model", 280.0)
                .labelled_by(label.id);
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new("ai_model_rect"), response.rect));
            ui.end_row();
            let label = daw_ui::text::label(ui, "API key");
            let hint = if self.setup_draft.ai.api_key.is_some() {
                "Replace stored API key"
            } else {
                "API key"
            };
            let response = daw_ui::secret_field(ui, &mut self.assistant.key_draft, hint, 280.0)
                .labelled_by(label.id);
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new("ai_key_rect"), response.rect));
            ui.end_row();
        });
        if self.setup_draft.ai.api_key.is_some() && daw_ui::button(ui, "Remove API key").clicked() {
            self.setup_draft.ai.api_key = None;
            self.assistant.clear_key_draft();
        }
        daw_ui::text::note(
            ui,
            "The selected provider receives your prompts and requested song context.",
        );
        daw_ui::text::note(
            ui,
            "Keys are encrypted locally. Built-in encryption protects against casual inspection.",
        );
    }

    pub(crate) fn prepare_ai_settings(&mut self) -> bool {
        if self.setup_draft.ai.model.trim().is_empty() || self.setup_draft.ai.model.len() > 200 {
            self.message(
                Err(anyhow::anyhow!("Choose a model of at most 200 bytes")),
                "",
            );
            return false;
        }
        if !self.assistant.key_draft.is_empty() {
            match daw_ai::encrypt_api_key(self.assistant.key_draft.trim()) {
                Ok(key) => self.setup_draft.ai.api_key = Some(key),
                Err(error) => {
                    self.message(Err(anyhow::anyhow!(error)), "");
                    return false;
                }
            }
        }
        self.assistant.clear_key_draft();
        true
    }
}
