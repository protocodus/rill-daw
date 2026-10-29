//! Integrated assistant and MCP bridge. All tool execution enters the same
//! control-thread dispatcher; providers and stdio never borrow the live song.
use crate::app::{DawApp, PopupKind, SetupPage};
use daw_ai::{AgentEvent, AgentRun, ScriptedTurn};
use daw_tools::{SongToolHost, ToolError, ToolHost, ToolScope};
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
    scope: Option<ToolScope>,
}

#[derive(Default)]
pub(crate) struct Assistant {
    pub open: bool,
    pub prompt: String,
    pub messages: Vec<daw_ui::AssistantMessage>,
    pub status: String,
    ui: daw_ui::AssistantState,
    conversation: Vec<daw_ai::ConversationMessage>,
    last_prompt: String,
    request_prompt: String,
    scope: ToolScope,
    run_scope: ToolScope,
    scope_instrument: Option<daw_core::Instrument>,
    run_instrument: Option<daw_core::Instrument>,
    tools: Vec<daw_tools::ToolSpec>,
    last_tool_error: Option<Value>,
    /// Set only while an assistant call is executing on the control thread.
    /// An open contextual window never restricts ordinary UI or MCP commands.
    pub(crate) executing_scope: Option<ToolScope>,
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
    /// Activity rows have their own bound and must not evict the musical brief.
    fn remember(&mut self, message: daw_ai::ConversationMessage) {
        use daw_ai::ConversationMessage;
        let message = match message {
            ConversationMessage::User(text) => ConversationMessage::User(
                text.chars()
                    .take(daw_ai::MAX_CONVERSATION_MESSAGE_CHARS)
                    .collect(),
            ),
            ConversationMessage::Assistant(text) => ConversationMessage::Assistant(
                text.chars()
                    .take(daw_ai::MAX_CONVERSATION_MESSAGE_CHARS)
                    .collect(),
            ),
        };
        if match &message {
            ConversationMessage::User(text) | ConversationMessage::Assistant(text) => {
                text.trim().is_empty()
            }
        } {
            return;
        }
        self.conversation.push(message);
        let excess = self
            .conversation
            .len()
            .saturating_sub(daw_ai::MAX_CONVERSATION_MESSAGES);
        self.conversation.drain(..excess);
    }

    /// Starting a fresh conversation leaves an unsent draft available to edit.
    pub(crate) fn new_chat(&mut self) {
        self.messages.clear();
        self.conversation.clear();
        self.last_prompt.clear();
        self.request_prompt.clear();
        self.last_tool_error = None;
        self.status.clear();
        self.ui = Default::default();
    }

    pub(crate) fn reset_song(&mut self) {
        self.stop();
        self.new_chat();
        self.prompt.clear();
        self.next_script = None;
        self.scope = ToolScope::Song;
        self.run_scope = ToolScope::Song;
        self.scope_instrument = None;
        self.run_instrument = None;
        self.tools.clear();
        self.executing_scope = None;
    }

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
    pub(crate) fn open_global_assistant(&mut self) {
        self.open_context_assistant(ToolScope::Song);
    }

    pub(crate) fn open_assistant_examples(&mut self) {
        self.open_global_assistant();
        if !self.assistant.busy() {
            self.assistant.ui.page = daw_ui::AssistantPage::Examples;
            self.assistant.ui.category = "Compose".into();
        }
    }

    pub(crate) fn open_context_assistant(&mut self, scope: ToolScope) {
        let instrument = self.assistant_instrument_identity(&scope);
        if self.assistant.scope != scope || self.assistant.scope_instrument != instrument {
            self.assistant.stop();
            self.assistant.new_chat();
            self.assistant.prompt.clear();
            self.assistant.scope = scope;
            self.assistant.scope_instrument = instrument;
        }
        self.assistant.tools = self.assistant.scope.tool_specs(&self.project);
        if let Err(error) = self.assistant.scope.validate_target(&self.project) {
            self.assistant.status = error.message;
        }
        self.open_popup(PopupKind::Assistant);
    }

    fn assistant_instrument_identity(&self, scope: &ToolScope) -> Option<daw_core::Instrument> {
        match scope {
            ToolScope::Instrument { track_id } => self
                .project
                .track(*track_id)
                .map(|track| track.instrument.clone()),
            _ => None,
        }
    }

    pub(crate) fn assistant_diagnostics(&self, ctx: &egui::Context) -> Value {
        let rect = |id: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(id)))
                .map(|r| json!({"min":[r.min.x,r.min.y],"max":[r.max.x,r.max.y]}))
        };
        json!({"busy":self.assistant.busy(),"prompt":self.assistant.prompt,
            "context":self.assistant_context(),
            "scope":self.assistant.scope,
            "scope_label":self.assistant.scope.label(&self.project),
            "tools":self.assistant.tools.iter().map(|tool| &tool.name).collect::<Vec<_>>(),
            "request_prompt":self.assistant.request_prompt,
            "last_tool_error":self.assistant.last_tool_error,
            "provider":self.settings.ai.provider.name(),
            "configured":self.settings.ai.api_key.is_some(),
            "page":format!("{:?}",self.assistant.ui.page),"category":self.assistant.ui.category,
            "last_prompt":self.assistant.last_prompt,
            "conversation":self.assistant.conversation.iter().map(|message| match message {
                daw_ai::ConversationMessage::User(text) => json!({"role":"User","text":text}),
                daw_ai::ConversationMessage::Assistant(text) => json!({"role":"Assistant","text":text}),
            }).collect::<Vec<_>>(),
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
        let scope = self.assistant.scope;
        if self.assistant.scope_instrument != self.assistant_instrument_identity(&scope) {
            self.assistant.status =
                "The instrument changed. Open AI from the instrument again.".into();
            return;
        }
        if let Err(error) = scope.validate_target(&self.project) {
            self.assistant.status = error.message;
            return;
        }
        let context = if scope.is_global() {
            match self.call_tool("get_song_context", json!({})) {
                Ok(song) => json!({"song":song,
                "selection":{"track_id":self.selected_track,"clip_id":self.selected_clip,"note_ids":self.selected_notes,"cursor_beat":self.cursor,"clips":self.arranger_selection_ids(),"range":self.arranger_phrase_span()},
                "tracks":self.call_tool("list_tracks",json!({"limit":100})).ok()}),
                Err(error) => {
                    self.assistant.status = error.message;
                    return;
                }
            }
        } else {
            let mut context = scope.context(&self.project);
            context["session_id"] = json!(self.song_session.id);
            context["revision"] = json!(self.song_session.revision);
            if matches!(scope, ToolScope::MidiClip { track_id, clip_id }
                if self.selected_track == Some(track_id) && self.selected_clip == Some(clip_id))
            {
                context["selected_note_ids"] = json!(self.selected_notes);
            }
            context
        };
        let prompt = self.assistant.prompt.trim().to_owned();
        self.assistant.tools = scope.tool_specs(&self.project);
        let request = daw_ai::AgentRequest {
            prompt: prompt.clone(),
            context: json!({"scope":scope,"scope_label":scope.label(&self.project),
                "authority":if scope.is_global() { "current song only" } else { "only this pinned context; all other objects must remain unchanged" },
                "context":context}),
            conversation: self.assistant.conversation.clone(),
            tools: self.assistant.tools.clone(),
        };
        let request_prompt = daw_ai::request_message(&request);
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
                self.assistant.run_scope = scope;
                self.assistant.run_instrument = self.assistant.scope_instrument.clone();
                self.assistant.request_prompt = request_prompt;
                self.assistant.last_tool_error = None;
                self.assistant.run = Some(run);
                self.assistant.last_prompt.clone_from(&prompt);
                self.assistant
                    .remember(daw_ai::ConversationMessage::User(prompt.clone()));
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
        self.dispatch_scoped_tool(name, arguments, reply, cancel, None);
    }

    fn dispatch_scoped_tool(
        &mut self,
        name: String,
        arguments: Value,
        reply: Sender<Result<Value, ToolError>>,
        cancel: Arc<AtomicBool>,
        scope: Option<ToolScope>,
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
        if let Some(scope) = &scope
            && let Err(error) = scope.validate_call(&self.project, &name, &arguments)
        {
            self.record_assistant_tool_error(&error);
            let _ = reply.send(Err(error));
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
                scope,
            });
        } else {
            self.assistant.executing_scope = scope;
            let result = self.call_tool(&name, arguments);
            self.assistant.executing_scope = None;
            if scope.is_some()
                && let Err(error) = &result
            {
                self.record_assistant_tool_error(error);
            }
            let _ = reply.send(result);
        }
    }

    fn record_assistant_tool_error(&mut self, error: &ToolError) {
        self.assistant.last_tool_error = Some(json!({"code":error.code,"message":error.message}));
        self.assistant.messages.push(daw_ui::AssistantMessage {
            role: daw_ui::AssistantRole::Tool,
            text: format!("Not applied: {}", error.message),
        });
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
                if result.is_ok()
                    && let Some(scope) = &job.scope
                    && let Err(error) = scope.validate_target(&self.project)
                {
                    result = Err(error);
                }
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
                        self.assistant.executing_scope = job.scope;
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
                        self.assistant.executing_scope = None;
                    }
                }
                if let Ok(value) = &mut result {
                    value["session_id"] = json!(self.song_session.id);
                    value["revision"] = json!(self.song_session.revision);
                }
                job.cancel.store(true, Ordering::Release);
                if job.scope.is_some()
                    && !job.owner_cancel.load(Ordering::Acquire)
                    && let Err(error) = &result
                {
                    self.record_assistant_tool_error(error);
                }
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
                    if self.assistant.run_instrument
                        != self.assistant_instrument_identity(&self.assistant.run_scope)
                    {
                        let error = ToolError::new(
                            "scope_violation",
                            "The instrument changed. Open AI from the instrument again.",
                        );
                        self.record_assistant_tool_error(&error);
                        let _ = reply.send(Err(error.to_string()));
                        continue;
                    }
                    self.assistant.messages.push(daw_ui::AssistantMessage {
                        role: daw_ui::AssistantRole::Tool,
                        text: daw_ai::tool_activity(&name).into(),
                    });
                    let (host_reply, receiver) = mpsc::channel();
                    self.dispatch_scoped_tool(
                        name,
                        arguments,
                        host_reply,
                        cancel,
                        Some(self.assistant.run_scope),
                    );
                    std::thread::spawn(move || {
                        let result = receiver.recv().unwrap_or_else(|_| {
                            Err(ToolError::new("unavailable", "Tool dispatcher closed"))
                        });
                        let _ = reply.send(result.map_err(|e| e.to_string()));
                    });
                }
                AgentEvent::Finished(text) => {
                    self.assistant
                        .remember(daw_ai::ConversationMessage::Assistant(text.clone()));
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

    /// Describe exactly the current musical scope captured on the next Send.
    fn assistant_context(&self) -> String {
        let name = |value: &str| value.chars().take(80).collect::<String>();
        if !self.assistant.scope.is_global() {
            return format!(
                "{} BPM · {}/4 · Context stays fixed when selection changes",
                self.project.tempo, self.project.beats_per_bar,
            );
        }
        let mut parts = vec![format!(
            "{} · {} BPM · {}/4",
            name(&self.project.name),
            self.project.tempo,
            self.project.beats_per_bar
        )];
        if let Some(track) = self.selected_track.and_then(|id| self.project.track(id)) {
            parts.push(format!("Track: {}", name(&track.name)));
            if let Some(clip) = self
                .selected_clip
                .and_then(|id| self.project.clip(track.id, id))
            {
                parts.push(format!("Clip: {}", name(&clip.name)));
            }
        } else {
            parts.push("Current song".into());
        }
        if !self.selected_notes.is_empty() {
            parts.push(format!("{} selected notes", self.selected_notes.len()));
        }
        // Selection membership is maintained by the editor; do not walk every
        // clip in the song just to paint this count on each frame.
        if self
            .selected_clip
            .is_some_and(|id| self.arranger_selection.ids.contains(&id))
            && self.arranger_selection.ids.len() > 1
        {
            parts.push(format!(
                "{} selected clips",
                self.arranger_selection.ids.len()
            ));
        }
        if let Some((start, end)) = self.arranger_phrase_span() {
            parts.push(format!("Range: beats {start:.2}–{end:.2}"));
        }
        parts.push(format!("Cursor: beat {:.2}", self.cursor));
        parts.join(" · ")
    }

    pub(crate) fn assistant_popup(&mut self, ctx: &egui::Context) {
        if !self.assistant.open {
            return;
        }
        let context = self.assistant_context();
        let scope_label = self.assistant.scope.label(&self.project);
        let unavailable = self
            .assistant
            .scope
            .validate_target(&self.project)
            .err()
            .map(|error| error.message)
            .or_else(|| {
                (self.assistant.scope_instrument
                    != self.assistant_instrument_identity(&self.assistant.scope))
                .then(|| "The instrument changed. Open AI from the instrument again.".to_owned())
            });
        let examples: Vec<_> = daw_ai::scoped_prompt_examples(self.assistant.scope.context_key())
            .iter()
            .map(|example| daw_ui::AssistantExample {
                id: example.id,
                category: example.category.label(),
                title: example.title,
                description: example.description,
                prompt: example.prompt,
            })
            .collect();
        // Cache schemas on opening / starting a run. Painting only formats the
        // small published catalog; it never regenerates edit schemas or notes.
        let tool_details: Vec<_> = self.assistant.tools.iter().map(|tool| {
            let title = tool.name.replace('_', " ");
            let mut letters = title.chars();
            let title = letters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + letters.as_str()
            });
            let contexts = tool.contexts().iter().map(|context| context.label()).collect::<Vec<_>>().join(" · ");
            let prompt = if tool.effect() == daw_tools::ToolEffect::Read {
                format!("Use {} to inspect this context and explain the results. Do not change anything.", tool.name)
            } else {
                format!("Help me use {} within this scope. Explain the available changes and ask which settings I want before editing.", tool.name)
            };
            (title, contexts, prompt)
        }).collect();
        let tools: Vec<_> = self
            .assistant
            .tools
            .iter()
            .zip(&tool_details)
            .map(|(tool, (title, contexts, prompt))| daw_ui::AssistantTool {
                name: &tool.name,
                title,
                description: &tool.description,
                contexts,
                prompt,
            })
            .collect();
        let tips: Vec<_> = daw_ai::ASSISTANT_TIPS
            .iter()
            .map(|tip| daw_ui::AssistantTip {
                title: tip.title,
                body: tip.body,
            })
            .collect();
        let mut open = true;
        let mut action = None;
        let can_submit = !self.recording_busy()
            && unavailable.is_none()
            && (self.settings.ai.api_key.is_some() || self.assistant.next_script.is_some());
        // Undo is song-wide history; a contextual assistant must not undo an
        // unrelated edit merely because it is the most recent history item.
        let can_undo = self.assistant.scope.is_global() && self.history.can_undo();
        let can_reuse = self.assistant.prompt.is_empty() && !self.assistant.last_prompt.is_empty();
        daw_ui::popup("Assistant", ctx)
            .open(&mut open)
            .default_width(620.0)
            .default_height(520.0)
            .vscroll(false)
            .show(ctx, |ui| {
                let response = daw_ui::assistant(
                    ui,
                    &mut self.assistant.prompt,
                    &mut self.assistant.ui,
                    daw_ui::AssistantOptions {
                        messages: &self.assistant.messages,
                        status: Some(unavailable.as_deref().unwrap_or(&self.assistant.status)),
                        busy: self.assistant.run.is_some(),
                        can_submit,
                        can_undo,
                        context: &context,
                        scope: &scope_label,
                        provider: self.settings.ai.provider.name(),
                        configured: self.settings.ai.api_key.is_some(),
                        can_reuse,
                        examples: &examples,
                        tips: &tips,
                        tools: &tools,
                    },
                );
                ctx.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new("assistant_prompt_rect"),
                        response.prompt.interact_rect,
                    )
                });
                action = Some(response);
            });
        self.assistant.open = open;
        if let Some(response) = action {
            if response.stop {
                self.assistant.stop();
            }
            if response.undo && self.assistant.scope.is_global() {
                self.undo_redo(false);
            }
            if response.send {
                self.start_assistant();
            }
            if response.new_chat && !self.assistant.busy() {
                self.assistant.new_chat();
            }
            if response.reuse_last && !self.assistant.busy() && self.assistant.prompt.is_empty() {
                self.assistant
                    .prompt
                    .clone_from(&self.assistant.last_prompt);
                response.prompt.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ctx, response.prompt.id) {
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(
                            egui::text::CCursor::new(self.assistant.prompt.chars().count()),
                        )));
                    state.store(ctx, response.prompt.id);
                }
            }
            if response.settings && !self.assistant.busy() {
                self.open_popup(PopupKind::Setup);
                self.setup_page = SetupPage::Ai;
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
