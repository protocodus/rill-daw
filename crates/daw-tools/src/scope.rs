//! Pinned musical capabilities shared by discovery and execution. A prompt is
//! explanatory; these checks are the authority boundary for contextual tools.
use crate::{ToolError, ToolSpec, parse_arguments, song_tool_specs};
use daw_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolContext {
    Song,
    MidiClip,
    AudioClip,
    Instrument,
    Effect,
    EffectChain,
    Channel,
    Routing,
    Automation,
}

impl ToolContext {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Song => "Song",
            Self::MidiClip => "Piano roll",
            Self::AudioClip => "Audio clip",
            Self::Instrument => "Instrument",
            Self::Effect => "Effect",
            Self::EffectChain => "Effect chain",
            Self::Channel => "Channel",
            Self::Routing => "Routing",
            Self::Automation => "Automation",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolScope {
    #[default]
    Song,
    MidiClip {
        track_id: u64,
        clip_id: u64,
    },
    AudioClip {
        track_id: u64,
        clip_id: u64,
    },
    Instrument {
        track_id: u64,
    },
    Effect {
        track_id: u64,
        effect_id: u64,
    },
    EffectChain {
        track_id: u64,
    },
    Channel {
        track_id: u64,
    },
    Routing {
        track_id: u64,
    },
    Automation {
        track_id: u64,
        lane_id: u64,
    },
}

fn denied(message: impl Into<String>) -> ToolError {
    ToolError::new("scope_violation", message)
}

impl ToolSpec {
    /// Canonical applicability, also used by the browsable tool library.
    pub fn contexts(&self) -> &'static [ToolContext] {
        use ToolContext::*;
        match self.name.as_str() {
            "get_song_context" | "get_track" | "apply_edits" => &[
                Song,
                MidiClip,
                AudioClip,
                Instrument,
                Effect,
                EffectChain,
                Channel,
                Routing,
                Automation,
            ],
            "get_clip" => &[Song, MidiClip, AudioClip],
            "transform_phrase"
            | "composer_catalog"
            | "composer_scale"
            | "composer_chord"
            | "composer_progression"
            | "composer_euclidean" => &[Song, MidiClip],
            "audio_analyze_clip" | "audio_process_clip" => &[Song, AudioClip],
            "plugin_describe_parameters" => &[Song, Instrument, Effect, EffectChain],
            "list_plugins" | "assign_plugin" => &[Song, EffectChain],
            "set_mix" => &[Song, Channel],
            _ => &[Song],
        }
    }
}

impl ToolScope {
    pub fn is_global(&self) -> bool {
        matches!(self, Self::Song)
    }

    pub fn context_kind(&self) -> ToolContext {
        match self {
            Self::Song => ToolContext::Song,
            Self::MidiClip { .. } => ToolContext::MidiClip,
            Self::AudioClip { .. } => ToolContext::AudioClip,
            Self::Instrument { .. } => ToolContext::Instrument,
            Self::Effect { .. } => ToolContext::Effect,
            Self::EffectChain { .. } => ToolContext::EffectChain,
            Self::Channel { .. } => ToolContext::Channel,
            Self::Routing { .. } => ToolContext::Routing,
            Self::Automation { .. } => ToolContext::Automation,
        }
    }

    pub fn context_key(&self) -> &'static str {
        match self {
            Self::Song => "song",
            Self::MidiClip { .. } => "midi_clip",
            Self::AudioClip { .. } => "audio_clip",
            Self::Instrument { .. } => "instrument",
            Self::Effect { .. } => "effect",
            Self::EffectChain { .. } => "effect_chain",
            Self::Channel { .. } => "channel",
            Self::Routing { .. } => "routing",
            Self::Automation { .. } => "automation",
        }
    }

    fn track_id(&self) -> Option<u64> {
        match *self {
            Self::Song => None,
            Self::MidiClip { track_id, .. }
            | Self::AudioClip { track_id, .. }
            | Self::Instrument { track_id }
            | Self::Effect { track_id, .. }
            | Self::EffectChain { track_id }
            | Self::Channel { track_id }
            | Self::Routing { track_id }
            | Self::Automation { track_id, .. } => Some(track_id),
        }
    }

    fn clip_id(&self) -> Option<u64> {
        match *self {
            Self::MidiClip { clip_id, .. } | Self::AudioClip { clip_id, .. } => Some(clip_id),
            _ => None,
        }
    }

    pub fn validate_target(&self, project: &Project) -> Result<(), ToolError> {
        if self.is_global() {
            return Ok(());
        }
        let track = project
            .track(self.track_id().unwrap())
            .ok_or_else(|| denied("The pinned track no longer exists"))?;
        let valid = match *self {
            Self::MidiClip { clip_id, .. } => track
                .clips
                .iter()
                .any(|c| c.id == clip_id && matches!(c.content, ClipContent::Midi { .. })),
            Self::AudioClip { clip_id, .. } => track
                .clips
                .iter()
                .any(|c| c.id == clip_id && matches!(c.content, ClipContent::Audio { .. })),
            Self::Instrument { .. } => {
                track.kind == TrackKind::Midi && !matches!(track.instrument, Instrument::None)
            }
            Self::Effect { effect_id, .. } => track.inserts.iter().any(|e| e.id == effect_id),
            Self::EffectChain { .. } | Self::Channel { .. } | Self::Routing { .. } => matches!(
                track.kind,
                TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
            ),
            Self::Automation { track_id, lane_id } => project
                .automation_lane(lane_id)
                .is_some_and(|l| l.target.track == track_id),
            Self::Song => true,
        };
        if valid {
            Ok(())
        } else {
            Err(denied(
                "The pinned musical object is unavailable or changed kind",
            ))
        }
    }

    fn allows_tool(&self, name: &str) -> bool {
        if self.is_global() {
            return true;
        }
        match name {
            "get_song_context" | "get_track" | "apply_edits" => true,
            "get_clip" => self.clip_id().is_some(),
            "transform_phrase" => matches!(self, Self::MidiClip { .. }),
            "audio_analyze_clip" | "audio_process_clip" => matches!(self, Self::AudioClip { .. }),
            "plugin_describe_parameters" => matches!(
                self,
                Self::Instrument { .. } | Self::Effect { .. } | Self::EffectChain { .. }
            ),
            "list_plugins" | "assign_plugin" => matches!(self, Self::EffectChain { .. }),
            "set_mix" => matches!(self, Self::Channel { .. }),
            _ => matches!(self, Self::MidiClip { .. }) && crate::composer::is_read(name),
        }
    }

    fn allows_op(&self, op: &str) -> bool {
        let common_clip = matches!(
            op,
            "rename_clip" | "move_clip" | "trim_clip" | "stretch_clip" | "make_independent"
        );
        match self {
            Self::Song => true,
            Self::MidiClip { .. } => {
                common_clip
                    || matches!(
                        op,
                        "add_notes"
                            | "update_notes"
                            | "delete_notes"
                            | "transform_notes"
                            | "quantize_notes"
                            | "randomize_notes"
                            | "legato_notes"
                            | "split_note"
                            | "duplicate_note"
                            | "glue_notes"
                    )
            }
            Self::AudioClip { .. } => {
                common_clip
                    || matches!(
                        op,
                        "set_audio_gain"
                            | "set_audio_fades"
                            | "reverse_audio"
                            | "set_audio_processing"
                            | "set_crossfade"
                            | "slip_audio"
                    )
            }
            Self::Instrument { .. } => op == "set_plugin_parameters" || automation_op(op),
            Self::Effect { .. } => {
                matches!(op, "set_plugin_parameters" | "set_insert_enabled") || automation_op(op)
            }
            Self::EffectChain { .. } => {
                matches!(
                    op,
                    "set_plugin_parameters"
                        | "set_insert_enabled"
                        | "remove_insert"
                        | "reorder_insert"
                ) || automation_op(op)
            }
            Self::Channel { .. } => op == "set_mix" || automation_op(op),
            Self::Routing { .. } => {
                matches!(
                    op,
                    "set_output" | "add_send" | "update_send" | "remove_send"
                ) || automation_op(op)
            }
            Self::Automation { .. } => automation_op(op) && op != "add_automation",
        }
    }

    fn allows_automation(
        &self,
        project: &Project,
        lane_id: Option<u64>,
        target: AutomationTarget,
    ) -> bool {
        if self.track_id() != Some(target.track) {
            return false;
        }
        match *self {
            Self::Instrument { .. } => matches!(
                target.parameter,
                AutomationParameter::VstParameter { effect: None, .. }
            ),
            Self::Effect { effect_id, .. } => {
                matches!(target.parameter, AutomationParameter::VstParameter { effect: Some(id), .. } if id == effect_id)
            }
            Self::EffectChain { track_id } => {
                matches!(target.parameter, AutomationParameter::VstParameter { effect: Some(id), .. } if project.track(track_id).is_some_and(|t| t.inserts.iter().any(|e|e.id == id)))
            }
            Self::Channel { .. } => matches!(
                target.parameter,
                AutomationParameter::TrackGain | AutomationParameter::TrackPan
            ),
            Self::Routing { .. } => {
                matches!(target.parameter, AutomationParameter::SendGain { .. })
            }
            Self::Automation {
                lane_id: pinned, ..
            } => lane_id == Some(pinned),
            _ => false,
        }
    }

    /// Reject even ineffective out-of-scope operations before they reach a host.
    pub fn validate_call(
        &self,
        project: &Project,
        name: &str,
        args: &Value,
    ) -> Result<(), ToolError> {
        self.validate_target(project)?;
        if self.is_global() {
            return Ok(());
        }
        if !self.allows_tool(name) {
            return Err(denied(format!("{name} is unavailable in this context")));
        }
        let check_track = || {
            if args.get("track_id").and_then(Value::as_u64) == self.track_id() {
                Ok(())
            } else {
                Err(denied("Use the pinned track ID"))
            }
        };
        match name {
            "get_song_context" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Empty {}
                let _: Empty = parse_arguments(args.clone())?;
            }
            "get_track" => {
                let q: crate::TrackQuery = parse_arguments(args.clone())?;
                crate::Page {
                    offset: q.offset,
                    limit: q.limit,
                }
                .validate()?;
                if Some(q.track_id) != self.track_id()
                    && !project.automation_lane(q.track_id).is_some_and(|l| {
                        self.allows_automation(project, Some(q.track_id), l.target)
                    })
                {
                    return Err(denied("Read only the pinned target and its automation"));
                }
            }
            "get_clip" | "transform_phrase" | "audio_analyze_clip" | "audio_process_clip" => {
                check_track()?;
                if args.get("clip_id").and_then(Value::as_u64) != self.clip_id() {
                    return Err(denied("Use the pinned clip ID"));
                }
                if name == "audio_process_clip"
                    && args.get("operation").and_then(Value::as_str) != Some("normalize")
                {
                    return Err(denied(
                        "This clip context supports normalization only; structural audio edits require song scope",
                    ));
                }
            }
            "plugin_describe_parameters" => {
                check_track()?;
                self.validate_effect(project, args.get("insert_id").and_then(Value::as_u64))?;
            }
            "assign_plugin" => {
                check_track()?;
                if args.get("insert").and_then(Value::as_bool) != Some(true) {
                    return Err(denied("Effect-chain tools can add inserts only"));
                }
            }
            "list_plugins" => {
                if args.get("refresh").and_then(Value::as_bool) == Some(true) {
                    return Err(denied("Refresh the plugin library from song scope"));
                }
            }
            "set_mix" => {
                let q: crate::MixArgs = parse_arguments(args.clone())?;
                if q.channels.iter().any(|c| !self.owns_track_ref(&c.track)) {
                    return Err(denied("Mix only the pinned channel"));
                }
            }
            "apply_edits" => {
                let q: crate::ApplyArgs = parse_arguments(args.clone())?;
                let mut lanes = HashMap::new();
                for edit in q.edits {
                    self.validate_edit(project, &edit, &mut lanes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn owns_track_ref(&self, track: &EntityRef) -> bool {
        matches!(track, EntityRef::Id(id) if Some(*id) == self.track_id())
    }

    fn validate_effect(&self, project: &Project, effect: Option<u64>) -> Result<(), ToolError> {
        let valid = match *self {
            Self::Instrument { .. } => effect.is_none(),
            Self::Effect { effect_id, .. } => effect == Some(effect_id),
            Self::EffectChain { track_id } => effect.is_some_and(|id| {
                project
                    .track(track_id)
                    .is_some_and(|t| t.inserts.iter().any(|e| e.id == id))
            }),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(denied("Use only the pinned instrument or effect"))
        }
    }

    fn validate_edit(
        &self,
        project: &Project,
        edit: &SongEdit,
        keys: &mut HashMap<String, AutomationTarget>,
    ) -> Result<(), ToolError> {
        let value = serde_json::to_value(edit).map_err(|e| denied(e.to_string()))?;
        let op = value["op"].as_str().unwrap_or("");
        if !self.allows_op(op) {
            return Err(denied(format!("{op} is unavailable in this context")));
        }
        if let SongEdit::AddAutomation { target, key, .. } = edit {
            if !self.allows_automation(project, None, *target) {
                return Err(denied("Automation must target this context"));
            }
            if let Some(key) = key {
                keys.insert(key.clone(), *target);
            }
            return Ok(());
        }
        if automation_op(op) {
            let reference: EntityRef = parse_arguments(value["track"].clone())?;
            let valid = match reference {
                EntityRef::Id(id) => project
                    .automation_lane(id)
                    .is_some_and(|l| self.allows_automation(project, Some(id), l.target)),
                EntityRef::Key(key) => keys
                    .get(&key)
                    .is_some_and(|t| self.allows_automation(project, None, *t)),
            };
            return if valid {
                Ok(())
            } else {
                Err(denied("Edit only this context's automation lanes"))
            };
        }
        if let SongEdit::SetMix { channels } = edit {
            return if channels.iter().all(|c| self.owns_track_ref(&c.track)) {
                Ok(())
            } else {
                Err(denied("Mix only the pinned channel"))
            };
        }
        let track: EntityRef = parse_arguments(value["track"].clone())?;
        if !self.owns_track_ref(&track) {
            return Err(denied("Use the pinned track ID"));
        }
        if let Some(clip) = self.clip_id()
            && value["clip"].as_u64() != Some(clip)
        {
            return Err(denied(
                "Use the pinned clip ID; batch aliases cannot broaden clip scope",
            ));
        }
        match edit {
            SongEdit::SetPluginParameters { effect, .. } => {
                self.validate_effect(project, *effect)?
            }
            SongEdit::RemoveInsert { effect, .. }
            | SongEdit::ReorderInsert { effect, .. }
            | SongEdit::SetInsertEnabled { effect, .. } => {
                self.validate_effect(project, Some(*effect))?
            }
            _ => {}
        }
        Ok(())
    }

    /// Inspect the final candidate, including ghost synchronization and manual
    /// automation overrides, before any history, engine or UI publication.
    pub fn validate_candidate(&self, before: &Project, after: &Project) -> Result<(), ToolError> {
        if self.is_global() {
            return Ok(());
        }
        self.validate_target(before)?;
        self.validate_target(after)?;
        let mut restored = after.clone();
        let mut new_lanes = Vec::new();
        for track in &restored.tracks {
            if before.track(track.id).is_none() {
                let lane = track
                    .automation
                    .as_ref()
                    .ok_or_else(|| denied("This context cannot create tracks"))?;
                if !self.allows_automation(after, Some(track.id), lane.target) {
                    return Err(denied("New automation must target this context"));
                }
                let mut expected = Track::new(&track.name, TrackKind::Automation);
                expected.id = track.id;
                expected.color = track.color;
                expected.automation = track.automation.clone();
                if expected != *track {
                    return Err(denied("A new automation lane contains unrelated changes"));
                }
                new_lanes.push(track.id);
            }
        }
        restored.tracks.retain(|t| !new_lanes.contains(&t.id));
        for (index, old) in before.tracks.iter().enumerate() {
            let Some(track) = restored.track_mut(old.id) else {
                let removable = old.automation.as_ref().is_some_and(|l| {
                    self.allows_automation(before, Some(old.id), l.target)
                        && l.target.validate(after).is_err()
                });
                if removable {
                    restored.tracks.insert(index, old.clone());
                    continue;
                }
                return Err(denied("This context cannot remove another object"));
            };
            if let Some(lane) = &old.automation
                && self.allows_automation(before, Some(old.id), lane.target)
            {
                if track
                    .automation
                    .as_ref()
                    .is_none_or(|l| l.target != lane.target)
                {
                    return Err(denied("Automation targets cannot be changed"));
                }
                track.automation = old.automation.clone();
            }
            if Some(old.id) != self.track_id() {
                continue;
            }
            match *self {
                Self::MidiClip { clip_id, .. } | Self::AudioClip { clip_id, .. } => {
                    let source = old.clips.iter().find(|c| c.id == clip_id).unwrap();
                    let clip = track
                        .clips
                        .iter_mut()
                        .find(|c| c.id == clip_id)
                        .ok_or_else(|| denied("The pinned clip cannot be replaced"))?;
                    if clip.audio_take != source.audio_take
                        || (clip.link_group != source.link_group && clip.link_group.is_some())
                    {
                        return Err(denied("Clip membership cannot be retargeted"));
                    }
                    if let (ClipContent::Audio { path: a, .. }, ClipContent::Audio { path: b, .. }) =
                        (&source.content, &clip.content)
                        && a != b
                    {
                        return Err(denied("Audio sources cannot be replaced from clip context"));
                    }
                    *clip = source.clone();
                }
                Self::Instrument { .. } => {
                    track.instrument_parameters = old.instrument_parameters.clone()
                }
                Self::Effect { effect_id, .. } => {
                    let source = old.inserts.iter().find(|e| e.id == effect_id).unwrap();
                    let effect = track
                        .inserts
                        .iter_mut()
                        .find(|e| e.id == effect_id)
                        .ok_or_else(|| denied("The pinned effect cannot be replaced"))?;
                    effect.parameters = source.parameters.clone();
                    effect.enabled = source.enabled;
                }
                Self::EffectChain { .. } => track.inserts = old.inserts.clone(),
                Self::Channel { .. } => {
                    track.gain = old.gain;
                    track.pan = old.pan;
                    track.mute = old.mute;
                    track.solo = old.solo;
                }
                Self::Routing { .. } => {
                    track.sends = old.sends.clone();
                    track.output_bus = old.output_bus;
                }
                _ => {}
            }
        }
        if restored == *before {
            Ok(())
        } else {
            Err(denied(
                "The edit would change objects outside this context. Linked clips must be made independent before source edits.",
            ))
        }
    }

    pub fn tool_specs(&self, project: &Project) -> Vec<ToolSpec> {
        if self.is_global() {
            return song_tool_specs();
        }
        if self.validate_target(project).is_err() {
            return Vec::new();
        }
        song_tool_specs().into_iter().filter(|s| self.allows_tool(&s.name)).map(|mut spec| {
            let properties = &mut spec.input_schema["properties"];
            if spec.name == "get_track" {
                properties["track_id"] = json!({"type":"integer","minimum":1,"description":"Pinned track ID, or an automation lane ID whose target belongs to this context."});
                spec.description = "Read this context, or page a matching automation lane by its lane track ID. offset/limit page automation points, instrument parameter overrides, or chain inserts. get_clip and plugin_describe_parameters page notes and plug-in controls.".into();
            }
            if properties.get("track_id").is_some() && spec.name != "get_track" { properties["track_id"] = json!({"type":"integer","const":self.track_id()}); }
            if properties.get("clip_id").is_some() { properties["clip_id"] = json!({"type":"integer","const":self.clip_id()}); }
            if spec.name == "audio_process_clip" { properties["operation"] = json!({"type":"string","const":"normalize"}); spec.description="Normalize only this audio clip in one undoable action. Analysis runs in the background; the unchanged song revision and pinned scope are checked before publishing. Structural audio edits require song scope.".into(); }
            if spec.name == "audio_analyze_clip" { spec.description="Analyze only this audio clip. Measurements and suggested edits are read-only; this context permits normalization and in-place settings, not splitting or stripping silence.".into(); }
            if spec.name == "assign_plugin" { properties["insert"] = json!({"type":"boolean","const":true}); spec.description="Add a scanned catalog plug-in as an insert in this effect chain. Instrument replacement is unavailable in this context.".into(); }
            if spec.name == "list_plugins" { properties["refresh"] = json!({"type":"boolean","const":false}); }
            if spec.name == "plugin_describe_parameters" {
                match self {
                    Self::Instrument { .. } => { properties.as_object_mut().unwrap().remove("insert_id"); }
                    Self::Effect { effect_id, .. } => { properties["insert_id"] = json!({"type":"integer","const":effect_id}); }
                    Self::EffectChain { .. } => { properties["insert_id"] = json!({"type":"integer","minimum":1,"description":"An insert currently owned by this effect chain."}); }
                    _ => {}
                }
            }
            if spec.name == "apply_edits"
                && let Some(edits) = properties["edits"]["items"]["oneOf"].as_array_mut() {
                    edits.retain(|e|self.allows_op(e["properties"]["op"]["const"].as_str().unwrap_or("")));
                    spec.description=format!("Apply atomic edits only within this pinned context. Available operations: {}. Numeric IDs must name the pinned object or its matching automation lanes; new automation keys may be referenced later in the same batch. Linked source propagation is rejected; make_independent requires fresh note IDs afterward.",edits.iter().filter_map(|e|e["properties"]["op"]["const"].as_str()).collect::<Vec<_>>().join(", "));
                    for edit in edits {
                        let op = edit["properties"]["op"]["const"].as_str().unwrap_or("").to_owned();
                        if !automation_op(&op) && edit["properties"].get("track").is_some() { edit["properties"]["track"] = json!({"type":"integer","const":self.track_id()}); }
                        if edit["properties"].get("clip").is_some() { edit["properties"]["clip"] = json!({"type":"integer","const":self.clip_id()}); }
                        if op == "set_mix" { edit["properties"]["channels"]["items"]["properties"]["track"] = json!({"type":"integer","const":self.track_id()}); }
                        if automation_op(&op) && op != "add_automation" {
                            edit["properties"]["track"] = if let Self::Automation { lane_id, .. } = self { json!({"type":"integer","const":lane_id}) } else { json!({"oneOf":[{"type":"integer","minimum":1,"description":"Existing automation lane targeting this context."},{"type":"string","minLength":1,"maxLength":64,"pattern":"^[a-zA-Z0-9_-]+$"}]}) };
                        }
                        if op == "add_automation" {
                            edit["properties"]["target"]["properties"]["track"] = json!({"type":"integer","const":self.track_id()});
                            edit["properties"]["target"]["properties"]["parameter"] = self.automation_parameter_schema(project);
                        }
                        if edit["properties"].get("effect").is_some() {
                            edit["properties"]["effect"] = self.effect_schema(project);
                        }
                    }
            }
            if spec.name == "set_mix" { properties["channels"]["items"]["properties"]["track"] = json!({"type":"integer","const":self.track_id()}); }
            if spec.name == "assign_plugin" { spec.input_schema["required"].as_array_mut().unwrap().push(json!("insert")); }
            if spec.name == "plugin_describe_parameters" && matches!(self,Self::Effect { .. } | Self::EffectChain { .. }) { spec.input_schema["required"].as_array_mut().unwrap().push(json!("insert_id")); }
            spec.description.push_str(" Restricted to the pinned context; edits outside it are rejected.");
            spec
        }).collect()
    }

    fn effect_schema(&self, _project: &Project) -> Value {
        match self {
            Self::Instrument { .. } => json!({"type":"null"}),
            Self::Effect { effect_id, .. } => json!({"type":"integer","const":effect_id}),
            Self::EffectChain { .. } => {
                json!({"type":"integer","minimum":1,"description":"An insert currently owned by this effect chain."})
            }
            _ => json!({"type":"integer"}),
        }
    }

    fn automation_parameter_schema(&self, project: &Project) -> Value {
        match self {
            Self::Channel { .. } => json!({"type":"string","enum":["TrackGain","TrackPan"]}),
            Self::Routing { .. } => {
                json!({"type":"object","properties":{"SendGain":{"type":"object","properties":{"bus":{"type":"integer","enum":project.tracks.iter().filter(|t|t.kind == TrackKind::Bus).map(|t|t.id).collect::<Vec<_>>()}},"required":["bus"],"additionalProperties":false}},"required":["SendGain"],"additionalProperties":false})
            }
            _ => {
                json!({"type":"object","properties":{"VstParameter":{"type":"object","properties":{"effect":self.effect_schema(project),"id":{"type":"integer","minimum":0}},"required":["effect","id"],"additionalProperties":false}},"required":["VstParameter"],"additionalProperties":false})
            }
        }
    }

    pub fn label(&self, project: &Project) -> String {
        if self.is_global() {
            return "Song".into();
        }
        let track = self.track_id().and_then(|id| project.track(id));
        let name = short_label(track.map(|t| t.name.as_str()).unwrap_or("Unavailable"));
        match *self {
            Self::MidiClip { clip_id, .. } | Self::AudioClip { clip_id, .. } => format!(
                "{} · {name} / {}",
                if matches!(self, Self::MidiClip { .. }) {
                    "Piano roll"
                } else {
                    "Audio clip"
                },
                short_label(
                    track
                        .and_then(|t| t.clips.iter().find(|c| c.id == clip_id))
                        .map(|c| c.name.as_str())
                        .unwrap_or("Unavailable")
                )
            ),
            Self::Effect { effect_id, .. } => format!(
                "Effect · {name} / {}",
                short_label(
                    track
                        .and_then(|t| t.inserts.iter().find(|e| e.id == effect_id))
                        .map(|e| e.name.as_str())
                        .unwrap_or("Unavailable")
                )
            ),
            Self::Automation { lane_id, .. } => format!(
                "Automation · {}",
                short_label(
                    project
                        .track(lane_id)
                        .map(|t| t.name.as_str())
                        .unwrap_or("Unavailable")
                )
            ),
            Self::Instrument { .. } => format!("Instrument · {name}"),
            Self::EffectChain { .. } => format!("Effects · {name}"),
            Self::Channel { .. } => format!("Channel · {name}"),
            Self::Routing { .. } => format!("Routing · {name}"),
            Self::Song => "Song".into(),
        }
    }

    /// Read through the same scope checks as execution, preserving useful
    /// paging without disclosing sibling clips or unrelated processor state.
    pub fn read_context(
        &self,
        project: &Project,
        name: &str,
        args: &Value,
    ) -> Result<Value, ToolError> {
        self.validate_call(project, name, args)?;
        if !matches!(name, "get_song_context" | "get_track") || self.is_global() {
            return Err(denied("Use the ordinary dispatcher for this read"));
        }
        let mut value = self.context(project);
        if name == "get_song_context" {
            return Ok(value);
        }
        let q: crate::TrackQuery = parse_arguments(args.clone())?;
        if project.automation_lane(q.track_id).is_none() {
            value["automation"] = json!(self.automation_summaries(project, q.offset, q.limit));
            value["automation_offset"] = json!(q.offset);
        }
        if let Some(lane) = project.automation_lane(q.track_id) {
            value["id"] = json!(q.track_id);
            value["name"] = json!(crate::bounded_text(
                &project.track(q.track_id).unwrap().name
            ));
            value["automation"] = json!({"target":lane.target,"interpolation":lane.interpolation,"override_value":lane.override_value,"total_points":lane.points.len(),"points_offset":q.offset,"points":lane.points.iter().skip(q.offset).take(q.limit).collect::<Vec<_>>()});
        } else if let Self::Instrument { track_id } = self {
            let parameters = &project.track(*track_id).unwrap().instrument_parameters;
            value["instrument"] = json!({"assigned":true,"total_parameters":parameters.len(),"parameters_offset":q.offset,"parameters":parameters.iter().skip(q.offset).take(q.limit).collect::<Vec<_>>()});
        } else if let Self::EffectChain { track_id } = self {
            let inserts = &project.track(*track_id).unwrap().inserts;
            value["total_inserts"] = json!(inserts.len());
            value["inserts_offset"] = json!(q.offset);
            value["inserts"] = json!(inserts.iter().skip(q.offset).take(q.limit).map(|e|json!({"id":e.id,"name":crate::bounded_text(&e.name),"enabled":e.enabled,"parameter_count":e.parameters.len()})).collect::<Vec<_>>());
        } else if let Self::Effect {
            track_id,
            effect_id,
        } = self
        {
            let effect = project
                .track(*track_id)
                .unwrap()
                .inserts
                .iter()
                .find(|e| e.id == *effect_id)
                .unwrap();
            value["inserts"] = json!([{"id":effect.id,"name":crate::bounded_text(&effect.name),"enabled":effect.enabled,"total_parameters":effect.parameters.len(),"parameters_offset":q.offset,"parameters":effect.parameters.iter().skip(q.offset).take(q.limit).collect::<Vec<_>>() }]);
        }
        Ok(value)
    }

    /// Bounded, path-free context. Hosts add the current session and revision.
    pub fn context(&self, project: &Project) -> Value {
        let mut result = json!({"scope":self,"scope_label":self.label(project),"available":self.validate_target(project).is_ok(),"tempo":project.tempo,"beats_per_bar":project.beats_per_bar});
        if self.is_global() {
            result["name"] = json!(crate::bounded_text(&project.name));
            return result;
        }
        let Some(track) = self.track_id().and_then(|id| project.track(id)) else {
            return result;
        };
        result["track_id"] = json!(track.id);
        result["track_name"] = json!(crate::bounded_text(&track.name));
        if let Some(clip) = self
            .clip_id()
            .and_then(|id| track.clips.iter().find(|c| c.id == id))
        {
            result["clip"] = crate::clip_summary(clip);
            result["linked_source"] = json!(clip.link_group.is_some());
            if clip.link_group.is_some() {
                result["warning"] = json!(
                    "Source edits to linked clips are rejected if they affect another clip. Use make_independent, then read fresh note IDs."
                );
            }
        }
        match *self {
            Self::Instrument { .. } => {
                result["instrument"] = json!({"assigned":!matches!(track.instrument,Instrument::None),"total_parameters":track.instrument_parameters.len(),"parameters":track.instrument_parameters.iter().take(64).collect::<Vec<_>>() })
            }
            Self::Effect { effect_id, .. } => {
                result["inserts"] = json!(track.inserts.iter().filter(|e|e.id == effect_id).map(|e|json!({"id":e.id,"name":crate::bounded_text(&e.name),"enabled":e.enabled,"total_parameters":e.parameters.len(),"parameters":e.parameters.iter().take(64).collect::<Vec<_>>() })).collect::<Vec<_>>());
            }
            Self::EffectChain { .. } => {
                result["total_inserts"] = json!(track.inserts.len());
                result["inserts"] = json!(track.inserts.iter().take(64).map(|e|json!({"id":e.id,"name":crate::bounded_text(&e.name),"enabled":e.enabled,"parameter_count":e.parameters.len()})).collect::<Vec<_>>());
            }
            Self::Channel { .. } => {
                result["mix"] =
                    json!({"gain":track.gain,"pan":track.pan,"mute":track.mute,"solo":track.solo})
            }
            Self::Routing { .. } => {
                result["output_bus"] = json!(track.output_bus);
                result["sends"] = json!(
                    track
                        .sends
                        .iter()
                        .take(crate::MAX_QUERY_ITEMS)
                        .collect::<Vec<_>>()
                );
                result["destinations"] = json!(
                    project
                        .tracks
                        .iter()
                        .filter(|t| t.kind == TrackKind::Bus)
                        .take(crate::MAX_QUERY_ITEMS)
                        .map(|t| json!({"id":t.id,"name":crate::bounded_text(&t.name)}))
                        .collect::<Vec<_>>()
                );
            }
            _ => {}
        }
        result["total_automation_lanes"] = json!(
            project
                .tracks
                .iter()
                .filter(|t| t
                    .automation
                    .as_ref()
                    .is_some_and(|l| self.allows_automation(project, Some(t.id), l.target)))
                .count()
        );
        result["automation"] = json!(self.automation_summaries(project, 0, 64));
        result
    }

    fn automation_summaries(&self, project: &Project, offset: usize, limit: usize) -> Vec<Value> {
        project.tracks.iter().filter_map(|t|t.automation.as_ref().filter(|l|self.allows_automation(project,Some(t.id),l.target)).map(|l|(t,l))).skip(offset).take(limit).map(|(t,l)|json!({"id":t.id,"name":crate::bounded_text(&t.name),"target":l.target,"interpolation":l.interpolation,"override_value":l.override_value,"total_points":l.points.len()})).collect()
    }
}

fn automation_op(op: &str) -> bool {
    matches!(
        op,
        "add_automation"
            | "set_automation_interpolation"
            | "set_automation_point_curve"
            | "set_automation_override"
            | "upsert_automation_points"
            | "move_automation_point"
            | "remove_automation_points"
    )
}

fn short_label(value: &str) -> String {
    let mut label: String = value.chars().take(48).collect();
    if value.chars().count() > 48 {
        label.push('…');
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Project, u64, u64, u64, u64) {
        let mut project = Project::new();
        let mut track = Track::new("Keys", TrackKind::Midi);
        track.instrument = Instrument::Vst3 {
            path: "trusted-instrument.vst3".into(),
            class_id: None,
        };
        track.inserts.push(EffectSlot::new(
            "Delay",
            Effect::Vst3 {
                path: "trusted-delay.vst3".into(),
                class_id: None,
            },
        ));
        track.inserts.push(EffectSlot::new(
            "EQ",
            Effect::Vst3 {
                path: "trusted-eq.vst3".into(),
                class_id: None,
            },
        ));
        let first = Clip::midi("Verse", 0.0, 4.0, vec![Note::new(60, 0.0, 1.0, 100)]);
        let second = Clip::midi("Chorus", 4.0, 4.0, vec![Note::new(64, 0.0, 1.0, 100)]);
        let ids = (track.id, first.id, second.id, track.inserts[0].id);
        track.clips.extend([first, second]);
        project.tracks.push(track);
        project.tracks.push(Track::new("Bus", TrackKind::Bus));
        (project, ids.0, ids.1, ids.2, ids.3)
    }

    fn call(edits: Vec<SongEdit>) -> Value {
        json!({"session_id":1,"expected_revision":0,"label":"Scoped edit","edits":edits})
    }

    fn apply(project: &Project, edits: Vec<SongEdit>) -> Project {
        let mut candidate = project.clone();
        EditBatch {
            label: "Scoped edit".into(),
            edits,
        }
        .apply(&mut candidate)
        .unwrap();
        candidate
    }

    #[test]
    fn typed_calls_reject_outside_ids_aliases_and_global_noops() {
        let (project, track, clip, other, _) = fixture();
        let scope = ToolScope::MidiClip {
            track_id: track,
            clip_id: clip,
        };
        let good = SongEdit::AddNotes {
            track: track.into(),
            clip: clip.into(),
            notes: vec![NoteData {
                pitch: 67,
                start: 1.0,
                length: 1.0,
                velocity: 100,
            }],
        };
        scope
            .validate_call(&project, "apply_edits", &call(vec![good]))
            .unwrap();
        for edit in [
            SongEdit::RenameClip {
                track: track.into(),
                clip: other.into(),
                name: "Elsewhere".into(),
            },
            SongEdit::RenameClip {
                track: track.into(),
                clip: EntityRef::Key("clip".into()),
                name: "Alias".into(),
            },
            SongEdit::SetTempo { bpm: project.tempo },
            SongEdit::SetMix { channels: vec![] },
        ] {
            assert_eq!(
                scope
                    .validate_call(&project, "apply_edits", &call(vec![edit]))
                    .unwrap_err()
                    .code,
                "scope_violation"
            );
        }
        for tool in [
            "history",
            "transport",
            "set_selection",
            "organize_plugin",
            "create_instrument_part",
            "join_clips",
        ] {
            assert!(scope.validate_call(&project, tool, &json!({})).is_err());
        }
        assert!(
            scope
                .validate_call(
                    &project,
                    "get_clip",
                    &json!({"track_id":track,"clip_id":other})
                )
                .is_err()
        );
    }

    #[test]
    fn final_candidate_rejects_ghost_propagation_and_allows_explicit_detach() {
        let (mut project, track, clip, _, _) = fixture();
        project.clone_clip(track, clip).unwrap();
        let scope = ToolScope::MidiClip {
            track_id: track,
            clip_id: clip,
        };
        let edit = SongEdit::TransformNotes {
            track: track.into(),
            clip: clip.into(),
            note_ids: vec![],
            beats: 0.0,
            semitones: 2,
        };
        let leaked = apply(&project, vec![edit.clone()]);
        assert!(scope.validate_candidate(&project, &leaked).is_err());
        let independent = apply(
            &project,
            vec![SongEdit::MakeIndependent {
                track: track.into(),
                clip: clip.into(),
            }],
        );
        scope.validate_candidate(&project, &independent).unwrap();
        let edited = apply(&independent, vec![edit]);
        scope.validate_candidate(&independent, &edited).unwrap();
        assert!(
            scope.context(&project)["warning"]
                .as_str()
                .unwrap()
                .contains("fresh note IDs")
        );
    }

    #[test]
    fn candidates_preserve_unrelated_fields_and_stable_identity() {
        let (project, track, clip, other, _) = fixture();
        let scope = ToolScope::MidiClip {
            track_id: track,
            clip_id: clip,
        };
        let good = apply(
            &project,
            vec![SongEdit::RenameClip {
                track: track.into(),
                clip: clip.into(),
                name: "New verse".into(),
            }],
        );
        scope.validate_candidate(&project, &good).unwrap();
        let mut outside = good.clone();
        outside.tempo += 1.0;
        assert!(scope.validate_candidate(&project, &outside).is_err());
        let mut outside = good.clone();
        outside.track_mut(track).unwrap().gain = 0.1;
        assert!(scope.validate_candidate(&project, &outside).is_err());
        let mut outside = good.clone();
        outside.clip_mut(track, other).unwrap().name = "Elsewhere".into();
        assert!(scope.validate_candidate(&project, &outside).is_err());
        let mut outside = good.clone();
        outside.tracks.reverse();
        assert!(scope.validate_candidate(&project, &outside).is_err());
        let mut outside = good;
        outside.clip_mut(track, clip).unwrap().id = new_id();
        assert!(scope.validate_candidate(&project, &outside).is_err());
    }

    #[test]
    fn matching_automation_creation_keys_overrides_and_paging_stay_local() {
        let (project, track, _, _, _) = fixture();
        let scope = ToolScope::Channel { track_id: track };
        let target = AutomationTarget {
            track,
            parameter: AutomationParameter::TrackGain,
        };
        let edits = vec![
            SongEdit::AddAutomation {
                name: "Gain".into(),
                target,
                value: 0.5,
                key: Some("gain".into()),
            },
            SongEdit::SetAutomationOverride {
                track: EntityRef::Key("gain".into()),
                value: Some(0.6),
            },
        ];
        scope
            .validate_call(&project, "apply_edits", &call(edits.clone()))
            .unwrap();
        let mut candidate = apply(&project, edits);
        scope.validate_candidate(&project, &candidate).unwrap();
        let lane = candidate
            .tracks
            .iter()
            .find(|t| t.automation.is_some())
            .unwrap()
            .id;
        candidate
            .track_mut(lane)
            .unwrap()
            .automation
            .as_mut()
            .unwrap()
            .points = (0..300)
            .map(|i| AutomationPoint::new(i as f64, 0.5))
            .collect();
        scope.validate_candidate(&project, &candidate).unwrap();
        let page = scope
            .read_context(
                &candidate,
                "get_track",
                &json!({"track_id":lane,"offset":256,"limit":20}),
            )
            .unwrap();
        assert_eq!(page["automation"]["total_points"], 300);
        assert_eq!(page["automation"]["points_offset"], 256);
        assert_eq!(page["automation"]["points"].as_array().unwrap().len(), 20);
        let pinned = ToolScope::Automation {
            track_id: track,
            lane_id: lane,
        };
        let mut retargeted = candidate.clone();
        retargeted
            .track_mut(lane)
            .unwrap()
            .automation
            .as_mut()
            .unwrap()
            .target
            .parameter = AutomationParameter::TrackPan;
        assert!(pinned.validate_candidate(&candidate, &retargeted).is_err());
        let wrong = SongEdit::AddAutomation {
            name: "Other gain".into(),
            target: AutomationTarget {
                track: project.tracks[1].id,
                parameter: AutomationParameter::TrackGain,
            },
            value: 0.5,
            key: None,
        };
        assert!(
            scope
                .validate_call(&project, "apply_edits", &call(vec![wrong.clone()]))
                .is_err()
        );
        assert!(
            scope
                .validate_candidate(&project, &apply(&project, vec![wrong]))
                .is_err()
        );
    }

    #[test]
    fn plugin_scopes_separate_instrument_effect_and_chain() {
        let (project, track, _, _, effect) = fixture();
        let instrument = ToolScope::Instrument { track_id: track };
        let scoped_effect = ToolScope::Effect {
            track_id: track,
            effect_id: effect,
        };
        let chain = ToolScope::EffectChain { track_id: track };
        let params = vec![PluginParameterValue { id: 7, value: 0.5 }];
        let edit = SongEdit::SetPluginParameters {
            track: track.into(),
            effect: None,
            parameters: params.clone(),
        };
        instrument
            .validate_call(&project, "apply_edits", &call(vec![edit.clone()]))
            .unwrap();
        instrument
            .validate_candidate(&project, &apply(&project, vec![edit.clone()]))
            .unwrap();
        assert!(
            scoped_effect
                .validate_call(&project, "apply_edits", &call(vec![edit.clone()]))
                .is_err()
        );
        assert!(
            chain
                .validate_call(&project, "apply_edits", &call(vec![edit]))
                .is_err()
        );
        let edit = SongEdit::SetPluginParameters {
            track: track.into(),
            effect: Some(effect),
            parameters: params,
        };
        scoped_effect
            .validate_call(&project, "apply_edits", &call(vec![edit.clone()]))
            .unwrap();
        scoped_effect
            .validate_candidate(&project, &apply(&project, vec![edit]))
            .unwrap();
        assert!(
            instrument
                .validate_call(
                    &project,
                    "assign_plugin",
                    &json!({"track_id":track,"insert":false})
                )
                .is_err()
        );
        assert!(
            chain
                .validate_call(
                    &project,
                    "assign_plugin",
                    &json!({"track_id":track,"insert":false})
                )
                .is_err()
        );
        chain
            .validate_call(
                &project,
                "assign_plugin",
                &json!({"track_id":track,"insert":true}),
            )
            .unwrap();
        let mut candidate = project.clone();
        candidate.track_mut(track).unwrap().inserts.reverse();
        chain.validate_candidate(&project, &candidate).unwrap();
        assert!(
            scoped_effect
                .validate_candidate(&project, &candidate)
                .is_err()
        );
        let mut candidate = project.clone();
        candidate.track_mut(track).unwrap().instrument_state.push(1);
        assert!(instrument.validate_candidate(&project, &candidate).is_err());
    }

    #[test]
    fn chain_and_routing_allow_only_own_pruned_automation() {
        let (mut project, track, _, _, effect) = fixture();
        let chain = ToolScope::EffectChain { track_id: track };
        project
            .add_automation_track_with_value(
                "Delay parameter",
                AutomationTarget {
                    track,
                    parameter: AutomationParameter::VstParameter {
                        effect: Some(effect),
                        id: 7,
                    },
                },
                0.5,
            )
            .unwrap();
        let removed = apply(
            &project,
            vec![SongEdit::RemoveInsert {
                track: track.into(),
                effect,
            }],
        );
        chain.validate_candidate(&project, &removed).unwrap();
        let bus = project.tracks[1].id;
        project.add_send(track, bus, 0.5, false).unwrap();
        project
            .add_automation_track_with_value(
                "Send",
                AutomationTarget {
                    track,
                    parameter: AutomationParameter::SendGain { bus },
                },
                0.5,
            )
            .unwrap();
        let routing = ToolScope::Routing { track_id: track };
        let removed = apply(
            &project,
            vec![SongEdit::RemoveSend {
                track: track.into(),
                bus: bus.into(),
            }],
        );
        routing.validate_candidate(&project, &removed).unwrap();
        let mut leak = removed;
        leak.track_mut(bus).unwrap().gain = 0.1;
        assert!(routing.validate_candidate(&project, &leak).is_err());
    }

    #[test]
    fn audio_scope_excludes_structural_work_and_source_replacement() {
        let mut project = Project::new();
        let mut track = Track::new("Vocal", TrackKind::Audio);
        let clip = Clip::audio("Verse", 0.0, 4.0, "private.wav");
        let scope = ToolScope::AudioClip {
            track_id: track.id,
            clip_id: clip.id,
        };
        track.clips.push(clip);
        project.tracks.push(track);
        let (track, clip) = (scope.track_id().unwrap(), scope.clip_id().unwrap());
        scope
            .validate_call(
                &project,
                "audio_process_clip",
                &json!({"track_id":track,"clip_id":clip,"operation":"normalize"}),
            )
            .unwrap();
        for operation in ["split_transients", "strip_silence"] {
            assert!(
                scope
                    .validate_call(
                        &project,
                        "audio_process_clip",
                        &json!({"track_id":track,"clip_id":clip,"operation":operation})
                    )
                    .is_err()
            );
        }
        assert!(
            scope
                .validate_call(
                    &project,
                    "apply_edits",
                    &call(vec![SongEdit::SplitAudioTransients {
                        track: track.into(),
                        clip: clip.into(),
                        beats: vec![1.0]
                    }])
                )
                .is_err()
        );
        let candidate = apply(
            &project,
            vec![SongEdit::SetAudioGain {
                track: track.into(),
                clip: clip.into(),
                gain: 0.5,
            }],
        );
        scope.validate_candidate(&project, &candidate).unwrap();
        let mut leak = candidate;
        if let ClipContent::Audio { path, .. } = &mut leak.clip_mut(track, clip).unwrap().content {
            *path = "another.wav".into();
        }
        assert!(scope.validate_candidate(&project, &leak).is_err());
        assert!(!scope.context(&project).to_string().contains("private.wav"));
    }

    #[test]
    fn schemas_and_metadata_match_context_capabilities() {
        let (project, track, clip, _, effect) = fixture();
        let scope = ToolScope::MidiClip {
            track_id: track,
            clip_id: clip,
        };
        let specs = scope.tool_specs(&project);
        assert!(
            specs
                .iter()
                .all(|s| s.contexts().contains(&ToolContext::MidiClip))
        );
        assert!(specs.iter().any(|s| s.name == "composer_progression"));
        assert!(!specs.iter().any(|s| s.name == "create_midi_part"));
        let edits = &specs
            .iter()
            .find(|s| s.name == "apply_edits")
            .unwrap()
            .input_schema["properties"]["edits"]["items"]["oneOf"];
        let ops: Vec<_> = edits
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["properties"]["op"]["const"].as_str().unwrap())
            .collect();
        assert!(ops.contains(&"add_notes"));
        assert!(ops.contains(&"make_independent"));
        assert!(!ops.contains(&"add_track"));
        assert!(!ops.contains(&"set_tempo"));
        let scope = ToolScope::Effect {
            track_id: track,
            effect_id: effect,
        };
        let specs = scope.tool_specs(&project);
        let parameters = specs
            .iter()
            .find(|s| s.name == "plugin_describe_parameters")
            .unwrap();
        assert!(
            parameters.input_schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("insert_id"))
        );
        assert_eq!(
            parameters.input_schema["properties"]["insert_id"]["const"],
            effect
        );
        assert_eq!(
            serde_json::to_value(scope).unwrap(),
            json!({"kind":"effect","track_id":track,"effect_id":effect})
        );
        let missing = ToolScope::MidiClip {
            track_id: track,
            clip_id: u64::MAX,
        };
        assert!(missing.tool_specs(&project).is_empty());
        assert!(missing.validate_target(&project).is_err());
        assert_eq!(
            serde_json::to_value(ToolScope::Song.tool_specs(&project)).unwrap(),
            serde_json::to_value(song_tool_specs()).unwrap()
        );
    }
}
