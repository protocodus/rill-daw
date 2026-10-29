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
        self.assistant.stop();
        self.assistant.messages.clear();
        self.assistant.known_parameters.clear();
        self.assistant.cancel_all_tools();
        self.tool_joins.cancel();
        self.song_session.reset();
    }

    pub(crate) fn tool_ready(&mut self) -> Result<(), ToolError> {
        if self.edit_dirty
            || self.recording_busy()
            || self.state_sync_pending()
            || self.document_guard.is_some()
            || self.project_io_pending()
            || self.drag.is_some()
            || self.arranger_selection.drag.is_some()
            || self.arranger_selection.audio_drag.is_some()
            || self.arranger_selection.gesture.is_some()
        {
            return Err(ToolError::new(
                "busy",
                "Finish the active recording, edit or document operation first",
            ));
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
            .is_some_and(|(t, c)| self.project.clip(t, c).is_none())
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
                self.waveforms.get(path).map(AudioData::duration_seconds)
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
                    (Some(t), Some(c)) => Some(self.project.clip(t, c).ok_or_else(|| {
                        ToolError::new("not_found", "Clip not found on this track")
                    })?),
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
                    "undo":self.history.undo_labels().collect::<Vec<_>>(),"redo":self.history.redo_labels().collect::<Vec<_>>()}),
                )
            }
            "list_plugins" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    #[serde(default)]
                    query: String,
                }
                let args: Args = parse_arguments(arguments)?;
                if args.query.len() > 256 {
                    return Err(ToolError::new("invalid_arguments", "Search is too long"));
                }
                let query = args.query.to_lowercase();
                if self.vst_catalog.is_empty() && self.vst_scan.is_none() {
                    self.scan_plugins();
                }
                let plugins: Vec<_> = self
                    .vst_catalog
                    .iter()
                    .filter(|entry| {
                        format!("{} {}", entry.plugin.name, entry.plugin.vendor)
                            .to_lowercase()
                            .contains(&query)
                    })
                    .take(256)
                    .map(|entry| {
                        json!({"id":catalog_id(entry),"name":entry.plugin.name,
                    "vendor":entry.plugin.vendor,"category":entry.plugin.category,
                    "is_instrument":entry.plugin.is_instrument()})
                    })
                    .collect();
                Ok(json!({"plugins":plugins,"scanning":self.vst_scan.is_some()}))
            }
            "assign_plugin" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    session_id: u64,
                    expected_revision: u64,
                    track_id: u64,
                    plugin_id: String,
                    #[serde(default)]
                    insert: bool,
                }
                let args: Args = parse_arguments(arguments)?;
                self.check_tool_revision(args.session_id, args.expected_revision)?;
                let entry = self
                    .vst_catalog
                    .iter()
                    .find(|entry| catalog_id(entry) == args.plugin_id)
                    .ok_or_else(|| {
                        ToolError::new("not_found", "Choose a plug-in ID returned by list_plugins")
                    })?;
                let classification = entry.plugin.is_instrument();
                if (!args.insert && classification == Some(false))
                    || (args.insert && classification == Some(true))
                {
                    return Err(ToolError::new(
                        "invalid_arguments",
                        "Choose an instrument for a MIDI track or an effect for an insert",
                    ));
                }
                let mut candidate = self.project.clone();
                let path = entry.plugin.path.to_string_lossy().into_owned();
                let class_id = entry.plugin.class_id.clone();
                let effect_id = if args.insert {
                    let slot = EffectSlot::new(&entry.plugin.name, Effect::Vst3 { path, class_id });
                    let id = slot.id;
                    candidate
                        .track_mut(args.track_id)
                        .ok_or_else(|| ToolError::new("not_found", "Track not found"))?
                        .inserts
                        .push(slot);
                    Some(id)
                } else {
                    candidate
                        .set_instrument(args.track_id, Instrument::Vst3 { path, class_id })
                        .map_err(failure)?;
                    None
                };
                self.commit_song_candidate(candidate, "Assign plug-in")?;
                Ok(
                    json!({"session_id":self.song_session.id,"revision":self.song_session.revision,"track_id":args.track_id,"insert_id":effect_id}),
                )
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

pub(crate) fn catalog_id(entry: &daw_plugins::catalog::CatalogEntry) -> String {
    format!(
        "{}:{}",
        entry.fingerprint,
        entry.plugin.class_id.as_deref().unwrap_or("default")
    )
}
