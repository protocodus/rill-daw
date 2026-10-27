//! Explicitly authorized application tools. Never expose this host to the
//! embedded current-song agent; the application MCP profile opts into it.
use crate::{app::DawApp, document_ui::DocumentTransition, settings::Settings};
use daw_core::{AudioData, Clip, MAX_BEAT, TrackKind};
use daw_tools::{ToolError, parse_arguments};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

#[derive(Default)]
enum Patch<T> {
    #[default]
    Keep,
    Set(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::Set)
    }
}

struct SecretInput(String);
impl<'de> Deserialize<'de> for SecretInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self)
    }
}
impl Drop for SecretInput {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentArgs {
    action: String,
    path: Option<String>,
    #[serde(default)]
    discard_unsaved: bool,
    beat: Option<f64>,
    track_id: Option<u64>,
    #[serde(default = "include_tails")]
    include_tails: bool,
}
fn include_tails() -> bool {
    true
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct AudioPatch {
    device_name: Patch<Option<String>>,
    input_device_name: Patch<Option<String>>,
    midi_input_device: Patch<Option<String>>,
    sample_rate: Option<u32>,
    buffer_frames: Option<u32>,
    resample_quality: Option<daw_engine::ResampleQuality>,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct AiPatch {
    provider: Option<daw_ai::Provider>,
    model: Option<String>,
    api_key: Option<SecretInput>,
    remove_api_key: bool,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct SetupArgs {
    audio: Option<AudioPatch>,
    scratch_dir: Option<String>,
    vst_dirs: Option<Vec<String>>,
    ai: Option<AiPatch>,
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::new("invalid_arguments", message)
}
fn path(value: &str) -> Result<PathBuf, ToolError> {
    if value.trim().is_empty() || value.len() > 4096 || value.contains('\0') {
        return Err(invalid("Provide a nonempty path of at most 4096 bytes"));
    }
    Ok(PathBuf::from(value))
}
fn required_path(value: Option<&str>) -> Result<PathBuf, ToolError> {
    path(value.ok_or_else(|| invalid("This action requires an explicit path"))?)
}
fn redacted_setup(settings: &Settings) -> Value {
    json!({
        "audio":{"device_name":settings.device_name,"input_device_name":settings.input_device_name,
            "midi_input_device":settings.midi_input_device,"sample_rate":settings.sample_rate,
            "buffer_frames":settings.buffer_frames,"resample_quality":settings.resample_quality},
        "scratch_dir":settings.scratch_dir,"vst_dirs":settings.vst_dirs,
        "ai":{"provider":settings.ai.provider,"model":settings.ai.model,
            "api_key_configured":settings.ai.api_key.is_some()}
    })
}

impl DawApp {
    pub(crate) fn application_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Option<Result<Value, ToolError>> {
        match name {
            "document_status" => Some((|| {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Empty {}
                let _: Empty = parse_arguments(arguments)?;
                Ok(self.application_document_status())
            })()),
            "document_action" => Some((|| {
                let args: DocumentArgs = parse_arguments(arguments)?;
                self.application_document_action(args)
            })()),
            "get_setup" => Some((|| {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Empty {}
                let _: Empty = parse_arguments(arguments)?;
                // Refresh only enumerates metadata; it never opens a stream.
                // The shared helper also refreshes audio inputs and MIDI ports.
                self.refresh_devices();
                let mut result = redacted_setup(&self.settings);
                let audio_device = |device: &daw_engine::DeviceInfo| {
                    json!({
                        "name":device.name,"is_default":device.is_default,
                        "channels":device.channels,"supported_sample_rates":device.supported_sample_rates
                    })
                };
                result["available_devices"] = json!({
                    "outputs":self.devices.iter().map(audio_device).collect::<Vec<_>>(),
                    "inputs":self.input_devices.iter().map(audio_device).collect::<Vec<_>>(),
                    "midi":self.recording.midi_ports.iter().map(|port| json!({
                        "id":port.id,"name":port.name,
                        "selected":self.settings.midi_input_device.as_deref()==Some(port.id.as_str())
                    })).collect::<Vec<_>>()
                });
                Ok(result)
            })()),
            "configure_setup" => Some((|| {
                // Bound parsing without leaving a serialized plaintext key copy.
                let bytes = Zeroizing::new(
                    serde_json::to_vec(&arguments)
                        .map_err(|_| invalid("Invalid setup arguments"))?,
                );
                if bytes.len() > daw_tools::MAX_ARGUMENT_BYTES {
                    return Err(invalid("Tool arguments exceed 1 MiB"));
                }
                let args: SetupArgs = serde_json::from_value(arguments)
                    .map_err(|_| invalid("Invalid setup arguments"))?;
                self.application_configure_setup(args)
            })()),
            _ => None,
        }
    }

    pub(crate) fn application_document_status(&self) -> Value {
        json!({
            "name":self.project.name,"path":self.project_path,"modified":self.modified,
            "session_id":self.song_session.id,"revision":self.song_session.revision,
            "pending":self.project_io_pending() || self.state_sync_pending()
                || self.export_job.is_some() || self.loading.is_some() || self.document_guard.is_some(),
            "recording":self.recording_busy(),"status":self.status,"error":self.error
        })
    }

    fn application_document_action(&mut self, args: DocumentArgs) -> Result<Value, ToolError> {
        if self.edit_dirty || self.drag.is_some() {
            return Err(ToolError::new(
                "busy",
                "Finish the active edit gesture first",
            ));
        }
        if self.document_guard.is_some() {
            return Err(ToolError::new(
                "busy",
                "Finish the active document operation first",
            ));
        }
        self.flush_pending_edit();
        self.error = false;
        match args.action.as_str() {
            "new" => self
                .request_tool_document_transition(DocumentTransition::New, args.discard_unsaved)?,
            "open" | "recover" => {
                let recovery = args.action == "recover";
                let path = match args.path.as_deref() {
                    Some(value) => path(value)?,
                    None if recovery => self.settings.scratch_dir.join("recovery.daw.json"),
                    None => return Err(invalid("Opening a project requires an explicit path")),
                };
                self.request_tool_document_transition(
                    DocumentTransition::Open { path, recovery },
                    args.discard_unsaved,
                )?;
            }
            "save" | "save_as" => {
                let path = if args.action == "save" {
                    args.path
                        .as_deref()
                        .map(path)
                        .transpose()?
                        .or_else(|| self.project_path.clone())
                        .ok_or_else(|| invalid("The current project has no path; provide one"))?
                } else {
                    required_path(args.path.as_deref())?
                };
                self.save_to(path);
            }
            "export" => {
                self.export_to_with_tails(required_path(args.path.as_deref())?, args.include_tails)
            }
            "import" => self.application_import(
                &required_path(args.path.as_deref())?,
                args.track_id,
                args.beat.unwrap_or(self.cursor),
            )?,
            _ => return Err(invalid("Unknown document action")),
        }
        if self.error {
            return Err(ToolError::new("application_failed", self.status.clone()));
        }
        Ok(json!({"accepted":true,"document":self.application_document_status()}))
    }

    fn application_import(
        &mut self,
        source: &Path,
        track_id: Option<u64>,
        beat: f64,
    ) -> Result<(), ToolError> {
        self.tool_ready()?;
        if !beat.is_finite() || !(0.0..=MAX_BEAT).contains(&beat) {
            return Err(invalid(
                "Import beat must be finite and between 0 and 1000000",
            ));
        }
        let mut candidate = self.project.clone();
        let result: anyhow::Result<Option<(u64, u64)>> = (|| {
            let extension = source
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if extension.eq_ignore_ascii_case("mid") || extension.eq_ignore_ascii_case("midi") {
                anyhow::ensure!(
                    track_id.is_none(),
                    "MIDI import creates tracks; omit track_id"
                );
                candidate.import_midi(source, beat)?;
                Ok(None)
            } else {
                let audio = AudioData::load_wav(source)?;
                let length = audio.duration_seconds() * candidate.tempo / 60.0;
                anyhow::ensure!(length > 0.0, "Audio file is empty");
                let name = source.file_stem().unwrap_or_default().to_string_lossy();
                let track = match track_id {
                    Some(id) => {
                        anyhow::ensure!(
                            candidate
                                .track(id)
                                .is_some_and(|track| track.kind == TrackKind::Audio),
                            "Choose an existing audio track"
                        );
                        id
                    }
                    None => candidate.add_audio_track(name.as_ref()),
                };
                let canonical = std::fs::canonicalize(source)?;
                let clip = Clip::audio(name.as_ref(), beat, length, canonical.to_string_lossy());
                let clip_id = clip.id;
                candidate.track_mut(track).unwrap().clips.push(clip);
                Ok(Some((track, clip_id)))
            }
        })();
        let selection =
            result.map_err(|error| ToolError::new("import_failed", error.to_string()))?;
        self.commit_song_candidate(candidate, "Import media")?;
        if let Some((track, clip)) = selection {
            self.selected_track = Some(track);
            self.selected_clip = Some(clip);
            self.selected_notes.clear();
        }
        self.cache_audio();
        self.error = false;
        self.status = "Media imported".into();
        Ok(())
    }

    fn application_configure_setup(&mut self, args: SetupArgs) -> Result<Value, ToolError> {
        if self.edit_dirty || self.drag.is_some() {
            return Err(ToolError::new(
                "busy",
                "Finish the active edit gesture first",
            ));
        }
        if self.document_guard.is_some() {
            return Err(ToolError::new(
                "busy",
                "Finish the active document operation first",
            ));
        }
        let mut settings = self.settings.clone();
        if let Some(audio) = args.audio {
            if let Patch::Set(value) = audio.device_name {
                settings.device_name = value;
            }
            if let Patch::Set(value) = audio.input_device_name {
                settings.input_device_name = value;
            }
            if let Patch::Set(value) = audio.midi_input_device {
                settings.midi_input_device = value;
            }
            if let Some(value) = audio.sample_rate {
                settings.sample_rate = value;
            }
            if let Some(value) = audio.buffer_frames {
                settings.buffer_frames = value;
            }
            if let Some(value) = audio.resample_quality {
                settings.resample_quality = value;
            }
        }
        if let Some(value) = args.scratch_dir {
            settings.scratch_dir = path(&value)?;
        }
        if let Some(values) = args.vst_dirs {
            if values.len() > 128 {
                return Err(invalid("At most 128 VST directories are supported"));
            }
            settings.vst_dirs = values
                .iter()
                .map(|value| path(value))
                .collect::<Result<_, _>>()?;
        }
        if let Some(ai) = args.ai {
            if ai.api_key.is_some() && ai.remove_api_key {
                return Err(invalid("Provide api_key or remove_api_key, not both"));
            }
            if let Some(provider) = ai.provider {
                if provider != settings.ai.provider {
                    settings.ai.api_key = None;
                    if ai.model.is_none() {
                        settings.ai.model = provider.default_model().into();
                    }
                }
                settings.ai.provider = provider;
            }
            if let Some(model) = ai.model {
                settings.ai.model = model;
            }
            if let Some(key) = &ai.api_key {
                let value = key.0.trim();
                if value.is_empty() || value.len() > 4096 || value.contains(['\n', '\r']) {
                    return Err(invalid(
                        "Provide a nonempty API key of at most 4096 bytes without line breaks",
                    ));
                }
                settings.ai.api_key =
                    Some(daw_ai::encrypt_api_key(value).map_err(|_| {
                        ToolError::new("settings_failed", "Could not encrypt API key")
                    })?);
            }
            if ai.remove_api_key {
                settings.ai.api_key = None;
            }
        }
        if settings.ai.model.trim().is_empty() || settings.ai.model.len() > 200 {
            return Err(invalid("Choose a model of at most 200 bytes"));
        }
        settings
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        let desired = redacted_setup(&settings);
        self.apply_settings_snapshot(settings);
        if self.error {
            return Err(ToolError::new("settings_failed", self.status.clone()));
        }
        Ok(json!({"accepted":true,"setup":desired,"document":self.application_document_status()}))
    }
}
