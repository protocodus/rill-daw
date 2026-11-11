//! Current-song command boundary shared by the UI, tools, MCP and assistant.
//! The app retains ownership of document I/O and device lifecycle.
use crate::app::DawApp;
use daw_core::*;
use daw_tools::{SongToolHost, ToolError, ToolHost, parse_arguments};
use serde::Deserialize;
use serde_json::{Value, json};

fn failure(error: impl std::fmt::Display) -> ToolError {
    ToolError::new("edit_failed", error.to_string())
}

impl DawApp {
    pub(crate) fn invalidate_assistant_session(&mut self) {
        self.assistant.reset_song();
        self.assistant.known_parameters.clear();
        self.assistant.cancel_all_tools();
        self.tool_joins.cancel();
        self.song_session.reset();
    }

    /// Read-only readiness shared by tool execution and the assistant composer.
    /// Painting must never flush a pending gesture or advance song history.
    pub(crate) fn tool_block_reason(&self) -> Option<&'static str> {
        if self.recording_busy() {
            Some("Finish recording before continuing")
        } else if self.document_guard.is_some() || self.project_io_pending() {
            Some("Finish the current document operation first")
        } else if self.state_sync_pending() {
            Some("Wait for the instrument or effect to finish updating")
        } else if self.edit_dirty
            || self.drag.is_some()
            || self.arranger_selection.drag.is_some()
            || self.arranger_selection.audio_drag.is_some()
            || self.arranger_selection.gesture.is_some()
        {
            Some("Finish the active edit first")
        } else {
            None
        }
    }

    pub(crate) fn tool_ready(&mut self) -> Result<(), ToolError> {
        if let Some(reason) = self.tool_block_reason() {
            return Err(ToolError::new("busy", reason));
        }
        self.flush_pending_edit();
        Ok(())
    }

    pub(crate) fn commit_song_candidate(
        &mut self,
        candidate: Project,
        label: &str,
    ) -> Result<bool, ToolError> {
        self.commit_song_candidate_with_automation_intent(candidate, label, &[])
    }

    fn commit_song_candidate_with_automation_intent(
        &mut self,
        mut candidate: Project,
        label: &str,
        explicit_lanes: &[u64],
    ) -> Result<bool, ToolError> {
        candidate.validate().map_err(failure)?;
        crate::automation_state::capture_manual_parameter_changes(
            &self.project,
            &mut candidate,
            explicit_lanes,
        );
        if let Some(scope) = &self.assistant.executing_scope {
            scope.validate_candidate(&self.project, &candidate)?;
        }
        if candidate == self.project {
            return Ok(false);
        }
        let audio_changed = !crate::app::audio_equivalent(&self.project, &candidate);
        let editor_state = self.editor_state_signature();
        self.assistant
            .known_parameters
            .retain(|(track, effect), _| {
                let old = self.project.track(*track);
                let new = candidate.track(*track);
                match (old, new, effect) {
                    (Some(old), Some(new), None) => {
                        old.instrument == new.instrument
                            && old.instrument_state == new.instrument_state
                    }
                    (Some(old), Some(new), Some(id)) => match (
                        old.inserts.iter().find(|s| s.id == *id),
                        new.inserts.iter().find(|s| s.id == *id),
                    ) {
                        (Some(old), Some(new)) => {
                            old.effect == new.effect && old.state == new.state
                        }
                        _ => false,
                    },
                    _ => false,
                }
            });
        let before_state = self.history.state_id();
        let before_selection = self.capture_selection();
        self.history.commit_named(&self.project, label);
        self.project = candidate;
        self.reconcile_editor_history(editor_state);
        self.song_session.bump();
        self.modified = !self.history.is_saved();
        if self
            .selected_track
            .is_some_and(|id| self.project.track(id).is_none())
        {
            self.selected_track = None;
            self.selected_clip = None;
        }
        if self
            .selected_track
            .zip(self.selected_clip)
            .is_some_and(|(t, c)| !self.project.has_clip_or_automation_clip(t, c))
        {
            self.selected_clip = None;
        }
        let valid: std::collections::HashSet<_> = self
            .selected()
            .and_then(|c| match &c.content {
                ClipContent::Midi { notes } => Some(notes.iter().map(|n| n.id).collect()),
                _ => None,
            })
            .unwrap_or_default();
        self.selected_notes.retain(|id| valid.contains(id));
        self.sync_arranger_selection();
        let after_selection = self.capture_selection();
        self.selection_history.record(
            (before_state, before_selection),
            (self.history.state_id(), after_selection),
            self.history.retained_state_ids(),
        );
        self.checkpoint_committed_work();
        if audio_changed {
            self.revision = self.revision.wrapping_add(1);
            self.update_playback();
        }
        Ok(true)
    }

    pub(crate) fn check_tool_revision(
        &self,
        session_id: u64,
        expected_revision: u64,
    ) -> Result<(), ToolError> {
        if session_id != self.song_session.id || expected_revision != self.song_session.revision {
            return Err(ToolError::new(
                "conflict",
                "Song changed; read fresh song context before editing",
            ));
        }
        Ok(())
    }
}

impl SongToolHost for DawApp {
    fn project(&self) -> &Project {
        &self.project
    }
    fn song_session(&self) -> SongSession {
        self.song_session
    }
    fn apply_edits(&mut self, batch: EditBatch) -> Result<EditResult, ToolError> {
        self.tool_ready()?;
        self.validate_tool_plugin_parameters(&batch)?;
        let mut candidate = self.project.clone();
        let result = batch
            .apply_with_source_durations(&mut candidate, &|path| {
                self.audio_source_info(path)
                    .map(|info| info.frames as f64 / f64::from(info.sample_rate))
            })
            .map_err(failure)?;
        // An explicit override/resume is authoritative for its lane within
        // this atomic batch, regardless of operation order or unchanged value.
        let explicit_lanes: Vec<_> = batch
            .edits
            .iter()
            .filter_map(|edit| {
                let SongEdit::SetAutomationOverride { track, .. } = edit else {
                    return None;
                };
                match track {
                    EntityRef::Id(id) => Some(*id),
                    EntityRef::Key(key) => result
                        .created
                        .iter()
                        .find(|entity| entity.key.as_ref() == Some(key))
                        .map(|entity| entity.id),
                }
            })
            .collect();
        self.commit_song_candidate_with_automation_intent(
            candidate,
            &batch.label,
            &explicit_lanes,
        )?;
        Ok(result)
    }
}

impl ToolHost for DawApp {
    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        if let Some(scope) = self.assistant.executing_scope {
            scope.validate_call(&self.project, name, &arguments)?;
            if !scope.is_global() && matches!(name, "get_song_context" | "get_track") {
                self.tool_ready()?;
                let mut context = scope.read_context(&self.project, name, &arguments)?;
                context["session_id"] = json!(self.song_session.id);
                context["revision"] = json!(self.song_session.revision);
                return Ok(context);
            }
        }
        // Flush before the registry checks optimistic revisions. Otherwise a
        // pending native gesture could commit between the check and mutation.
        if let Some(result) = self.recording_tool(name, arguments.clone()) {
            return result;
        }
        self.tool_ready()?;
        if let Some(result) = self.instrument_tool(name, arguments.clone()) {
            return result;
        }
        match name {
            "get_selection" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Empty {}
                let _: Empty = parse_arguments(arguments)?;
                Ok(
                    json!({"session_id":self.song_session.id,"revision":self.song_session.revision,
                    "track_id":self.selected_track,"clip_id":self.selected_clip,
                    "note_ids":self.selected_notes,"cursor_beat":self.cursor,
                    "clips":self.arranger_selection_ids(),"range":self.arranger_phrase_span()}),
                )
            }
            "set_selection" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    track_id: Option<u64>,
                    clip_id: Option<u64>,
                    #[serde(default)]
                    note_ids: Vec<u64>,
                }
                let args: Args = parse_arguments(arguments)?;
                if args.note_ids.len() > MAX_BATCH_NOTES {
                    return Err(ToolError::new(
                        "invalid_arguments",
                        "Too many selected notes",
                    ));
                }
                if args
                    .track_id
                    .is_some_and(|id| self.project.track(id).is_none())
                {
                    return Err(ToolError::new("not_found", "Track not found"));
                }
                let clip = match (args.track_id, args.clip_id) {
                    (Some(t), Some(c)) => {
                        if !self.project.has_clip_or_automation_clip(t, c) {
                            return Err(ToolError::new(
                                "not_found",
                                "Clip not found on this track",
                            ));
                        }
                        self.project.clip(t, c)
                    }
                    (_, Some(_)) => {
                        return Err(ToolError::new(
                            "invalid_arguments",
                            "A clip requires its track_id",
                        ));
                    }
                    _ => None,
                };
                let valid: std::collections::HashSet<_> = clip
                    .and_then(|c| match &c.content {
                        ClipContent::Midi { notes } => Some(notes.iter().map(|n| n.id).collect()),
                        _ => None,
                    })
                    .unwrap_or_default();
                if args.note_ids.iter().any(|id| !valid.contains(id)) {
                    return Err(ToolError::new(
                        "not_found",
                        "Selected note not found in this MIDI clip",
                    ));
                }
                self.arranger_select_clips(args.track_id.zip(args.clip_id), None);
                self.selected_track = args.track_id;
                self.selected_clip = args.clip_id;
                self.selected_notes = args.note_ids.into_iter().collect();
                self.call_tool("get_selection", json!({}))
            }
            "transport" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    action: String,
                    #[serde(default)]
                    beat: Option<f64>,
                }
                let args: Args = parse_arguments(arguments)?;
                match args.action.as_str() {
                    "state" => {}
                    "play" => {
                        if !self.playing() {
                            self.play();
                        }
                    }
                    "pause" => {
                        if self.playing() {
                            self.play();
                        }
                    }
                    "stop" => self.stop(),
                    "seek" => {
                        let beat = args
                            .beat
                            .filter(|v| v.is_finite() && (0.0..=MAX_BEAT).contains(v))
                            .ok_or_else(|| {
                                ToolError::new(
                                    "invalid_arguments",
                                    "seek requires a finite beat between 0 and 1000000",
                                )
                            })?;
                        self.cursor = beat;
                        if let Some(engine) = &self.engine {
                            engine.seek(beat);
                        }
                    }
                    _ => {
                        return Err(ToolError::new(
                            "invalid_arguments",
                            "Unknown transport action",
                        ));
                    }
                }
                Ok(json!({"playing":self.playing(),"cursor_beat":self.cursor}))
            }
            "history" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    action: String,
                    session_id: Option<u64>,
                    expected_revision: Option<u64>,
                }
                let args: Args = parse_arguments(arguments)?;
                if matches!(args.action.as_str(), "undo" | "redo") {
                    let (Some(id), Some(revision)) = (args.session_id, args.expected_revision)
                    else {
                        return Err(ToolError::new(
                            "invalid_arguments",
                            "History mutations require session_id and expected_revision",
                        ));
                    };
                    self.check_tool_revision(id, revision)?;
                }
                match args.action.as_str() {
                    "state" => {}
                    "undo" => self.undo_redo(false),
                    "redo" => self.undo_redo(true),
                    _ => {
                        return Err(ToolError::new(
                            "invalid_arguments",
                            "Unknown history action",
                        ));
                    }
                }
                Ok(
                    json!({"session_id":self.song_session.id,"revision":self.song_session.revision,
                    "undo":self.history.undo_labels().collect::<Vec<_>>(),"redo":self.history.redo_labels().collect::<Vec<_>>(),
                    "action_limit":self.history.step_limit(),"memory_bytes":self.history.memory_bytes(),"memory_limit_bytes":HISTORY_BYTES}),
                )
            }
            "list_plugins" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    #[serde(default)]
                    query: String,
                    format: Option<String>,
                    #[serde(default)]
                    refresh: bool,
                }
                let args: Args = parse_arguments(arguments)?;
                let format = args.format.as_deref().map(plugin_format).transpose()?;
                if args.query.len() > 256 {
                    return Err(ToolError::new("invalid_arguments", "Search is too long"));
                }
                let query = args.query.to_lowercase();
                if (args.refresh || self.vst_catalog.is_empty()) && !self.plugin_sync.scanning() {
                    self.refresh_plugins();
                }
                // Built-ins have no plug-in format; a format filter leaves them out.
                let builtins = crate::builtin_devices::catalog()
                    .iter()
                    .filter(|_| format.is_none());
                let plugins: Vec<_> =
                    builtins
                        .chain(self.vst_catalog.iter().filter(|entry| {
                            format.is_none_or(|format| entry.plugin.format == format)
                        }))
                        .filter(|entry| {
                            format!("{} {}", entry.plugin.name, entry.plugin.vendor)
                                .to_lowercase()
                                .contains(&query)
                        })
                        .take(256)
                        .map(|entry| {
                            json!({"id":catalog_id(entry),"name":entry.plugin.name,
                    "vendor":entry.plugin.vendor,"category":entry.plugin.category,
                    "format":crate::builtin_devices::format_label(&entry.plugin),
                    "is_instrument":entry.plugin.is_instrument(),
                    "favorite":self.plugin_favorite(&entry.plugin),
                    "categories":self.plugin_categories(&entry.plugin)})
                        })
                        .collect();
                Ok(json!({"plugins":plugins,"scanning":self.plugin_sync.scanning()}))
            }
            "organize_plugin" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    plugin_id: String,
                    favorite: Option<bool>,
                    categories: Option<Vec<String>>,
                }
                let args: Args = parse_arguments(arguments)?;
                if args.categories.as_ref().is_some_and(|categories| {
                    categories.len() > 32
                        || categories.iter().any(|category| {
                            category.trim().is_empty()
                                || category.len() > 64
                                || category.contains(',')
                        })
                }) {
                    return Err(ToolError::new(
                        "invalid_arguments",
                        "Use up to 32 non-empty categories of at most 64 characters, without commas",
                    ));
                }
                let plugin = crate::builtin_devices::catalog()
                    .iter()
                    .chain(&self.vst_catalog)
                    .find(|entry| catalog_id(entry) == args.plugin_id)
                    .ok_or_else(|| {
                        ToolError::new("not_found", "Choose a plug-in ID returned by list_plugins")
                    })?
                    .plugin
                    .clone();
                let saved = |result: anyhow::Result<()>| {
                    result.map_err(|error| ToolError::new("settings_failed", format!("{error:#}")))
                };
                if let Some(favorite) = args.favorite {
                    saved(
                        self.edit_plugin_library(crate::library::LibraryEdit::Favorite(
                            plugin.clone(),
                            favorite,
                        )),
                    )?;
                }
                if let Some(categories) = args.categories {
                    saved(
                        self.edit_plugin_library(crate::library::LibraryEdit::Categories(
                            plugin.clone(),
                            categories.join(","),
                        )),
                    )?;
                }
                Ok(json!({"plugin_id":args.plugin_id,
                    "favorite":self.plugin_favorite(&plugin),
                    "categories":self.plugin_categories(&plugin)}))
            }
            "assign_plugin" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    session_id: u64,
                    expected_revision: u64,
                    track_id: u64,
                    plugin_id: String,
                    format: Option<String>,
                    #[serde(default)]
                    insert: bool,
                }
                let args: Args = parse_arguments(arguments)?;
                let format = args.format.as_deref().map(plugin_format).transpose()?;
                self.check_tool_revision(args.session_id, args.expected_revision)?;
                let entry = crate::builtin_devices::catalog()
                    .iter()
                    .chain(&self.vst_catalog)
                    .find(|entry| catalog_id(entry) == args.plugin_id)
                    .ok_or_else(|| {
                        ToolError::new("not_found", "Choose a plug-in ID returned by list_plugins")
                    })?;
                let builtin = crate::builtin_devices::is_builtin(&entry.plugin);
                if format.is_some_and(|format| builtin || format != entry.plugin.format) {
                    return Err(ToolError::new(
                        "invalid_arguments",
                        format!(
                            "This catalog plug-in is {}",
                            crate::builtin_devices::format_label(&entry.plugin)
                        ),
                    ));
                }
                let classification = entry.plugin.is_instrument();
                let used_plugin = entry.plugin.clone();
                if (!args.insert && classification == Some(false))
                    || (args.insert && classification == Some(true))
                {
                    return Err(ToolError::new(
                        "invalid_arguments",
                        "Choose an instrument for a MIDI track or an effect for an insert",
                    ));
                }
                let mut candidate = self.project.clone();
                let effect_id = if args.insert {
                    let slot = EffectSlot::new(
                        &entry.plugin.name,
                        crate::builtin_devices::effect(&entry.plugin),
                    );
                    let id = slot.id;
                    candidate
                        .track_mut(args.track_id)
                        .ok_or_else(|| ToolError::new("not_found", "Track not found"))?
                        .inserts
                        .push(slot);
                    Some(id)
                } else {
                    candidate
                        .set_instrument(
                            args.track_id,
                            crate::builtin_devices::instrument(&entry.plugin),
                        )
                        .map_err(failure)?;
                    None
                };
                self.commit_song_candidate(candidate, "Assign plug-in")?;
                self.record_plugin_use(&used_plugin);
                Ok(
                    json!({"session_id":self.song_session.id,"revision":self.song_session.revision,"track_id":args.track_id,"insert_id":effect_id,"format":crate::builtin_devices::format_label(&used_plugin)}),
                )
            }
            "set_device_parameters" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Parameter {
                    parameter: Value,
                    value: Value,
                }
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    session_id: u64,
                    expected_revision: u64,
                    track_id: u64,
                    insert_id: Option<u64>,
                    parameters: Vec<Parameter>,
                }
                let args: Args = parse_arguments(arguments)?;
                self.check_tool_revision(args.session_id, args.expected_revision)?;
                let descriptor = self.tool_device(args.track_id, args.insert_id)?;
                if args.parameters.is_empty() || args.parameters.len() > descriptor.parameters.len()
                {
                    return Err(ToolError::new(
                        "invalid_arguments",
                        format!(
                            "Set 1 to {} parameters per call",
                            descriptor.parameters.len()
                        ),
                    ));
                }
                let mut values = Vec::with_capacity(args.parameters.len());
                for parameter in &args.parameters {
                    let spec = match &parameter.parameter {
                        Value::String(key) => descriptor.parameter_by_key(key),
                        Value::Number(id) => id
                            .as_u64()
                            .and_then(|id| u32::try_from(id).ok())
                            .and_then(|id| descriptor.parameter(id)),
                        _ => None,
                    }
                    .ok_or_else(|| {
                        ToolError::new(
                            "invalid_arguments",
                            format!(
                                "{} has no parameter {}; use a key or ID from plugin_describe_parameters",
                                descriptor.name, parameter.parameter
                            ),
                        )
                    })?;
                    let normalized =
                        crate::builtin_devices::normalized_from_tool(spec, &parameter.value)
                            .map_err(|message| ToolError::new("invalid_arguments", message))?;
                    values.push((spec, normalized));
                }
                let mut candidate = self.project.clone();
                store_values(
                    device_parameters_mut(&mut candidate, args.track_id, args.insert_id)?,
                    values
                        .iter()
                        .map(|(spec, normalized)| (spec.id, *normalized)),
                );
                self.commit_song_candidate(candidate, "Device parameters")?;
                let parameters: Vec<_> = values
                    .iter()
                    .map(|(spec, normalized)| {
                        crate::builtin_devices::describe_parameter(spec, *normalized)
                    })
                    .collect();
                Ok(
                    json!({"session_id":self.song_session.id,"revision":self.song_session.revision,
                    "track_id":args.track_id,"insert_id":args.insert_id,"parameters":parameters}),
                )
            }
            "list_device_presets" | "load_device_preset" | "store_device_preset" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    track_id: u64,
                    insert_id: Option<u64>,
                    name: Option<String>,
                    #[serde(default)]
                    query: String,
                    session_id: Option<u64>,
                    expected_revision: Option<u64>,
                }
                let args: Args = parse_arguments(arguments)?;
                let descriptor = self.tool_device(args.track_id, args.insert_id)?;
                let preset_name = || {
                    args.name
                        .as_deref()
                        .map(str::trim)
                        .filter(|name| !name.is_empty() && name.len() <= 128)
                        .ok_or_else(|| {
                            ToolError::new(
                                "invalid_arguments",
                                "Give a preset name of 1 to 128 bytes",
                            )
                        })
                };
                match name {
                    "list_device_presets" => {
                        let query = args.query.to_lowercase();
                        let presets: Vec<_> = crate::builtin_devices::all_presets(descriptor)
                            .into_iter()
                            .filter(|(_, preset)| {
                                std::iter::once(&preset.name)
                                    .chain([&preset.category, &preset.description])
                                    .chain(&preset.tags)
                                    .any(|text| text.to_lowercase().contains(&query))
                            })
                            .map(|(factory, preset)| json!({"name":preset.name,"category":preset.category,
                                "tags":preset.tags,"description":preset.description,"source":if factory {"factory"} else {"user"}}))
                            .collect();
                        Ok(json!({"device":descriptor.id,"presets":presets}))
                    }
                    "load_device_preset" => {
                        let (Some(session_id), Some(expected_revision)) =
                            (args.session_id, args.expected_revision)
                        else {
                            return Err(ToolError::new(
                                "invalid_arguments",
                                "Loading a preset needs session_id and expected_revision",
                            ));
                        };
                        self.check_tool_revision(session_id, expected_revision)?;
                        let name = preset_name()?;
                        let preset = crate::builtin_devices::all_presets(descriptor)
                            .into_iter()
                            .map(|(_, preset)| preset)
                            .find(|preset| preset.name.eq_ignore_ascii_case(name))
                            .ok_or_else(|| {
                                ToolError::new(
                                    "not_found",
                                    "No such preset; list the device's presets first",
                                )
                            })?;
                        let resolved = preset.resolve(descriptor).map_err(failure)?;
                        let mut candidate = self.project.clone();
                        store_values(
                            device_parameters_mut(&mut candidate, args.track_id, args.insert_id)?,
                            resolved
                                .values
                                .iter()
                                .map(|(id, value)| (*id, f64::from(*value))),
                        );
                        self.commit_song_candidate(candidate, "Device preset")?;
                        Ok(
                            json!({"session_id":self.song_session.id,"revision":self.song_session.revision,
                            "preset":preset.name,"skipped":resolved.skipped}),
                        )
                    }
                    _ => {
                        let name = preset_name()?;
                        let overrides =
                            device_parameters(&self.project, args.track_id, args.insert_id)
                                .unwrap_or_default();
                        let preset = daw_devices::Preset::capture(descriptor, name, |id| {
                            crate::builtin_devices::value(descriptor, overrides, id) as f32
                        });
                        crate::builtin_devices::save_user_preset(&preset).map_err(|error| {
                            ToolError::new("settings_failed", format!("{error:#}"))
                        })?;
                        Ok(json!({"device":descriptor.id,"saved":preset.name}))
                    }
                }
            }
            _ if crate::tool_jobs::handles(name) => Err(ToolError::new(
                "worker_required",
                "This tool must run on the background tool dispatcher",
            )),
            _ => daw_tools::dispatch_song_tool(self, name, arguments),
        }
    }
}

impl DawApp {
    fn validate_tool_plugin_parameters(&self, batch: &EditBatch) -> Result<(), ToolError> {
        for edit in &batch.edits {
            let (track, effect, ids): (u64, Option<u64>, Vec<u32>) = match edit {
                SongEdit::SetPluginParameters {
                    track: EntityRef::Id(track),
                    effect,
                    parameters,
                } => (*track, *effect, parameters.iter().map(|p| p.id).collect()),
                SongEdit::SetPluginParameters { .. } => {
                    return Err(ToolError::new(
                        "discovery_required",
                        "Discover the assigned plug-in controls before editing parameters",
                    ));
                }
                SongEdit::AddAutomation {
                    target:
                        AutomationTarget {
                            track,
                            parameter: AutomationParameter::VstParameter { effect, id },
                        },
                    ..
                } => (*track, *effect, vec![*id]),
                _ => continue,
            };
            let known = self
                .assistant
                .known_parameters
                .get(&(track, effect))
                .ok_or_else(|| {
                    ToolError::new(
                        "discovery_required",
                        "Call plugin_describe_parameters for this assigned plug-in first",
                    )
                })?;
            if ids.iter().any(|id| known.get(id) != Some(&false)) {
                return Err(ToolError::new(
                    "invalid_arguments",
                    "Use writable parameter IDs returned by plugin_describe_parameters",
                ));
            }
        }
        Ok(())
    }
}

impl DawApp {
    /// The built-in device assigned as this track's instrument or insert.
    fn tool_device(
        &self,
        track_id: u64,
        insert_id: Option<u64>,
    ) -> Result<&'static daw_devices::DeviceDescriptor, ToolError> {
        let track = self
            .project
            .track(track_id)
            .ok_or_else(|| ToolError::new("not_found", "Track not found"))?;
        let device = match insert_id {
            Some(id) => track
                .inserts
                .iter()
                .find(|slot| slot.id == id)
                .ok_or_else(|| ToolError::new("not_found", "Insert not found on this track"))?
                .effect
                .builtin_device(),
            None => track.instrument.builtin_device(),
        }
        .ok_or_else(|| {
            ToolError::new(
                "invalid_arguments",
                "This is not a Rill built-in device; use set_plugin_parameters for hosted plug-ins",
            )
        })?;
        daw_devices::descriptor(device).ok_or_else(|| {
            ToolError::new(
                "unavailable",
                format!("Built-in device \"{device}\" is not available in this version"),
            )
        })
    }
}

fn device_parameters(
    project: &Project,
    track_id: u64,
    insert_id: Option<u64>,
) -> Option<&[PluginParameterValue]> {
    let track = project.track(track_id)?;
    Some(match insert_id {
        Some(id) => &track.inserts.iter().find(|slot| slot.id == id)?.parameters,
        None => &track.instrument_parameters,
    })
}

fn device_parameters_mut(
    project: &mut Project,
    track_id: u64,
    insert_id: Option<u64>,
) -> Result<&mut Vec<PluginParameterValue>, ToolError> {
    let track = project
        .track_mut(track_id)
        .ok_or_else(|| ToolError::new("not_found", "Track not found"))?;
    Ok(match insert_id {
        Some(id) => {
            &mut track
                .inserts
                .iter_mut()
                .find(|slot| slot.id == id)
                .ok_or_else(|| ToolError::new("not_found", "Insert not found on this track"))?
                .parameters
        }
        None => &mut track.instrument_parameters,
    })
}

/// Replaces stored values by ID and appends new ones.
fn store_values(stored: &mut Vec<PluginParameterValue>, values: impl Iterator<Item = (u32, f64)>) {
    for (id, value) in values {
        match stored.iter_mut().find(|parameter| parameter.id == id) {
            Some(parameter) => parameter.value = value,
            None => stored.push(PluginParameterValue { id, value }),
        }
    }
}

fn plugin_format(label: &str) -> Result<daw_plugins::PluginFormat, ToolError> {
    daw_plugins::PluginFormat::from_label(label)
        .ok_or_else(|| ToolError::new("invalid_arguments", "Format must be VST3, CLAP, LV2 or AU"))
}

pub(crate) fn catalog_id(entry: &daw_plugins::catalog::CatalogEntry) -> String {
    // A built-in's fingerprint is already its full `rill:<id>` catalog ID.
    if crate::builtin_devices::is_builtin(&entry.plugin) {
        return entry.fingerprint.clone();
    }
    format!(
        "{}:{}",
        entry.fingerprint,
        entry.plugin.class_id.as_deref().unwrap_or("default")
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn tool_formats_accept_every_label_including_au_on_every_platform() {
        for format in daw_plugins::PluginFormat::ALL {
            assert_eq!(
                super::plugin_format(&format.label().to_lowercase()).unwrap(),
                format
            );
        }
        let error = super::plugin_format("AAX").unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
        assert!(error.message.contains("AU"), "{}", error.message);
    }
}
