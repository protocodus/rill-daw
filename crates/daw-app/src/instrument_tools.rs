//! Musical composition and trusted instrument assignment as one atomic edit.
//! The shared composition tools own note/generator validation; this layer adds
//! application catalog capabilities without exposing file paths to callers.

use crate::app::DawApp;
use daw_core::{EditBatch, EditResult, Instrument, Project, SongSession};
use daw_tools::{SongToolHost, ToolError, dispatch_song_tool, parse_arguments};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstrumentPartArgs {
    plugin_id: String,
    /// False selects create_midi_part; true selects generate_part. The nested
    /// arguments use the corresponding canonical tool schema unchanged.
    #[serde(default)]
    generate: bool,
    part: Value,
}

/// Stages the ordinary tool dispatcher against an isolated candidate. Its
/// temporary revision advances normally, but no application history, playback,
/// selection, or editor state changes until the final host commit.
struct CandidateHost {
    project: Project,
    session: SongSession,
}

impl SongToolHost for CandidateHost {
    fn project(&self) -> &Project {
        &self.project
    }

    fn song_session(&self) -> SongSession {
        self.session
    }

    fn apply_edits(&mut self, batch: EditBatch) -> Result<EditResult, ToolError> {
        let result = batch
            .apply(&mut self.project)
            .map_err(|error| ToolError::new("edit_failed", error.to_string()))?;
        if result.changed {
            self.session.bump();
        }
        Ok(result)
    }
}

impl DawApp {
    pub(crate) fn instrument_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Option<Result<Value, ToolError>> {
        if name != "create_instrument_part" {
            return None;
        }
        Some(self.create_instrument_part(arguments))
    }

    fn create_instrument_part(&mut self, arguments: Value) -> Result<Value, ToolError> {
        self.tool_ready()?;
        let args: InstrumentPartArgs = parse_arguments(arguments)?;
        if args.plugin_id.is_empty() || args.plugin_id.len() > 1024 {
            return Err(ToolError::new(
                "invalid_arguments",
                "Invalid plug-in catalog ID",
            ));
        }
        let entry = self
            .vst_catalog
            .iter()
            .find(|entry| crate::session_service::catalog_id(entry) == args.plugin_id)
            .ok_or_else(|| {
                ToolError::new("not_found", "Choose a plug-in ID returned by list_plugins")
            })?;
        if entry.plugin.is_instrument() == Some(false) {
            return Err(ToolError::new(
                "invalid_arguments",
                "Choose an instrument; this catalog entry is an effect",
            ));
        }
        let instrument = Instrument::Vst3 {
            path: entry.plugin.path.to_string_lossy().into_owned(),
            class_id: entry.plugin.class_id.clone(),
        };
        let instrument_name = entry.plugin.name.clone();
        let unverified = entry.plugin.is_instrument().is_none();
        let requested_track = args.part.get("track_id").and_then(Value::as_u64);
        let mut candidate = CandidateHost {
            project: self.project.clone(),
            session: self.song_session,
        };
        let mut result = dispatch_song_tool(
            &mut candidate,
            if args.generate {
                "generate_part"
            } else {
                "create_midi_part"
            },
            args.part,
        )?;
        let created = result["created"].as_array().ok_or_else(|| {
            ToolError::new(
                "internal_error",
                "Composition did not return its created objects",
            )
        })?;
        let track_id = requested_track
            .or_else(|| {
                created
                    .iter()
                    .find(|item| item["kind"] == "track")
                    .and_then(|item| item["id"].as_u64())
            })
            .ok_or_else(|| {
                ToolError::new("internal_error", "Composition did not return its track")
            })?;
        let clip_id = created
            .iter()
            .find(|item| item["kind"] == "clip")
            .and_then(|item| item["id"].as_u64())
            .ok_or_else(|| {
                ToolError::new("internal_error", "Composition did not return its clip")
            })?;
        candidate
            .project
            .set_instrument(track_id, instrument)
            .map_err(|error| ToolError::new("edit_failed", error.to_string()))?;
        let label = format!(
            "Create instrument part: {}",
            candidate
                .project
                .clip(track_id, clip_id)
                .ok_or_else(|| {
                    ToolError::new("internal_error", "Created instrument clip is missing")
                })?
                .name
        );
        let changed = self.commit_song_candidate(candidate.project, &label)?;
        result["session_id"] = json!(self.song_session.id);
        result["revision"] = json!(self.song_session.revision);
        result["changed"] = json!(changed);
        result["track_id"] = json!(track_id);
        result["clip_id"] = json!(clip_id);
        result["plugin_id"] = json!(args.plugin_id);
        result["instrument_name"] = json!(instrument_name);
        if unverified {
            result["warning"] = json!("Instrument support will be verified when the plug-in loads");
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_core::{ClipContent, Track, TrackKind};
    use daw_plugins::{CatalogEntry, PluginInfo, ThumbnailStatus};

    fn app() -> DawApp {
        let mut app = DawApp::with_context(&eframe::egui::Context::default(), Project::new(), None);
        app.audio_device_enabled = false;
        app.vst_catalog.push(CatalogEntry {
            plugin: PluginInfo {
                name: "Trusted test synth".into(),
                path: "/catalog/Trusted.vst3".into(),
                vendor: "Rill tests".into(),
                category: "Instrument|Synth".into(),
                class_id: Some("11111111111111111111111111111111".into()),
                ..PluginInfo::default()
            },
            fingerprint: "trusted".into(),
            thumbnail: None,
            thumbnail_size: None,
            status: ThumbnailStatus::Missing,
            added_unix_ms: 0,
        });
        app
    }

    fn args(app: &DawApp) -> Value {
        json!({
            "plugin_id":"trusted:11111111111111111111111111111111",
            "part":{
                "session_id":app.song_session.id,"expected_revision":app.song_session.revision,
                "name":"Bass", "start":8.0,"length":4.0,
                "notes":[{"pitch":36,"start":0.0,"length":1.0,"velocity":100},
                         {"pitch":43,"start":2.0,"length":1.0,"velocity":90}]
            }
        })
    }

    #[test]
    fn composition_and_instrument_are_one_commit_without_editor_side_effects() {
        let mut app = app();
        let before = app.project.clone();
        let revision = app.song_session.revision;
        let arguments = args(&app);
        let result = app
            .instrument_tool("create_instrument_part", arguments)
            .unwrap()
            .unwrap();
        assert_eq!(app.project.tracks.len(), 1);
        assert_eq!(app.song_session.revision, revision + 1);
        assert_eq!(app.history.undo_labels().count(), 1);
        let track = &app.project.tracks[0];
        // Composition arguments already provide an intentional musical name;
        // instrument assignment must preserve it, even for a new track.
        assert_eq!(track.name, "Bass");
        assert_eq!(result["track_id"], track.id);
        assert!(
            matches!(&track.instrument, Instrument::Vst3 { path, .. } if path == "/catalog/Trusted.vst3")
        );
        assert!(matches!(&track.clips[0].content, ClipContent::Midi { notes } if notes.len() == 2));
        assert!(!app.instrument_open);
        assert!(app.instrument_autolaunch.is_none());
        assert!(app.selected_track.is_none());
        app.undo_redo(false);
        assert_eq!(app.project, before);
        app.undo_redo(true);
        assert_eq!(app.project.tracks.len(), 1);
        assert!(matches!(
            app.project.tracks[0].instrument,
            Instrument::Vst3 { .. }
        ));
    }

    #[test]
    fn invalid_plugin_notes_or_revision_leave_no_partial_track() {
        let mut app = app();
        let before = app.project.clone();
        let session = app.song_session;
        for invalid in ["plugin", "notes", "revision", "unknown_field"] {
            let mut arguments = args(&app);
            match invalid {
                "plugin" => arguments["plugin_id"] = json!("/arbitrary/path.vst3"),
                "notes" => arguments["part"]["notes"][0]["length"] = json!(-1.0),
                "revision" => arguments["part"]["expected_revision"] = json!(session.revision + 1),
                _ => arguments["part"]["path"] = json!("/arbitrary/source.wav"),
            }
            assert!(
                app.instrument_tool("create_instrument_part", arguments)
                    .unwrap()
                    .is_err(),
                "{invalid}"
            );
            assert_eq!(app.project, before);
            assert_eq!(app.song_session, session);
            assert_eq!(app.history.undo_labels().count(), 0);
        }
    }

    #[test]
    fn generates_into_existing_track_and_preserves_same_instrument_state() {
        let mut app = app();
        let mut track = Track::new("Existing bass", TrackKind::Midi);
        track.instrument = Instrument::Vst3 {
            path: "/catalog/Trusted.vst3".into(),
            class_id: Some("11111111111111111111111111111111".into()),
        };
        track.instrument_state = vec![1, 2, 3];
        let track_id = track.id;
        app.project.tracks.push(track);
        let arguments = json!({
            "plugin_id":"trusted:11111111111111111111111111111111", "generate":true,
            "part":{"session_id":app.song_session.id,"expected_revision":app.song_session.revision,
                "track_id":track_id,"name":"Arpeggio","start":0.0,"length":4.0,"kind":"arpeggio","seed":42}
        });
        let result = app
            .instrument_tool("create_instrument_part", arguments)
            .unwrap()
            .unwrap();
        assert_eq!(result["track_id"], track_id);
        assert_eq!(app.project.tracks.len(), 1);
        assert_eq!(app.project.tracks[0].name, "Existing bass");
        assert_eq!(app.project.tracks[0].instrument_state, vec![1, 2, 3]);
        assert!(
            matches!(&app.project.tracks[0].clips[0].content, ClipContent::Midi { notes } if !notes.is_empty())
        );
        assert_eq!(app.history.undo_labels().count(), 1);
    }

    #[test]
    fn refuses_known_effect_instrument_assignment_atomically() {
        let mut app = app();
        app.vst_catalog[0].plugin.category = "Fx|Delay".into();
        let arguments = args(&app);
        assert!(
            app.instrument_tool("create_instrument_part", arguments)
                .unwrap()
                .is_err()
        );
        assert!(app.project.tracks.is_empty());
        assert_eq!(app.history.undo_labels().count(), 0);
    }

    #[test]
    fn an_explicit_default_looking_part_name_is_not_an_automatic_name() {
        let mut app = app();
        let mut arguments = args(&app);
        arguments["part"]["name"] = json!("New instrument");
        app.instrument_tool("create_instrument_part", arguments)
            .unwrap()
            .unwrap();
        assert_eq!(app.project.tracks[0].name, "New instrument");
    }
}
